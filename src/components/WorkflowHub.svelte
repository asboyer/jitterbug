<script lang="ts">
    import { invoke } from '@tauri-apps/api/core';
    import HarnessBadge from './HarnessBadge.svelte';
    import HeldAgents from './HeldAgents.svelte';
    import Icon from './Icon.svelte';
    import InfoTip from './InfoTip.svelte';
    import TaskDetail from './TaskDetail.svelte';
    import { workflowRows } from '../lib/workflow-hub';
    import { memory, percent, errorMessage } from '../lib/format';
    import type { ValidationQueue } from '../lib/queue';
    import type { Snapshot, Workload } from '../lib/types';
    import type { ManagementSelection } from '../lib/manage';

    let {
        snapshot,
        queue,
        busy,
        error,
        oncontrol,
        onmanage,
        onselect,
        onusage,
    }: {
        snapshot: Snapshot;
        queue: ValidationQueue | null;
        busy: boolean;
        error: string;
        oncontrol: (action: string, id?: string) => Promise<boolean>;
        onmanage: (selection: ManagementSelection) => void;
        onselect: (workload: Workload) => void;
        onusage: () => void;
    } = $props();
    let expanded = $state<string | null>(null);
    let showRecent = $state(false);
    let planBusy = $state(false);
    let planError = $state('');
    let plan = $state<{
        action: string;
        job_id: string | null;
        reason: string;
        provider: string;
        model: string;
    } | null>(null);
    const demo = $derived(snapshot.source === 'demo');
    const rows = $derived(queue ? workflowRows(queue, snapshot) : []);
    const cpu = $derived(demo && queue ? queue.resources.cpu_percent : snapshot.cpu_percent);
    const current = $derived(rows.filter((row) => !row.complete));
    const recent = $derived(rows.filter((row) => row.complete));
    const protectedUntil = $derived(queue?.control?.protected_until ?? 0);
    const protectedNow = $derived(protectedUntil > snapshot.captured_at);
    const activity = $derived(queue?.control?.events.at(-1));
    const otherApps = $derived(
        snapshot.workloads
            .filter((workload) => ['application', 'browser', 'model'].includes(workload.category))
            .sort((a, b) => b.cpu_percent - a.cpu_percent)
            .slice(0, 3)
    );

    $effect(() => {
        demo;
        plan = null;
        planError = '';
    });

    async function makePlan() {
        if (planBusy || !queue) return;
        const source = snapshot.source;
        planBusy = true;
        planError = '';
        try {
            const proposal: NonNullable<typeof plan> = demo
                ? {
                      action: 'protect',
                      job_id: null,
                      reason: 'Give your foreground apps room by holding upcoming checks for 20 minutes.',
                      provider: 'Demo',
                      model: 'No model called',
                  }
                : await invoke('plan_workflow_queue');
            if (snapshot.source === source) plan = proposal;
            if (source === 'live') onusage();
        } catch (cause) {
            if (snapshot.source === source) planError = errorMessage(cause);
        } finally {
            planBusy = false;
        }
    }
</script>

<section class="workflow-hub" aria-label="Workflow hub">
    <header class="hub-heading">
        <div>
            <span class="eyebrow">WORKFLOWS {demo ? '· DEMO' : ''}</span>
            <h1>Room to work.</h1>
        </div>
        <div class="hub-settings">
            <select
                aria-label="Scheduling goal"
                disabled={!queue || busy}
                value={queue?.control?.goal ?? 'responsive'}
                onchange={(event) => oncontrol(event.currentTarget.value)}
                ><option value="responsive">Keep responsive</option><option value="throughput"
                    >Finish sooner</option
                ></select
            >
            <button
                class="auto-toggle"
                aria-pressed={!queue?.paused}
                disabled={!queue || busy}
                onclick={() => oncontrol(queue?.paused ? 'resume' : 'pause')}
                ><span class:off={queue?.paused}></span>Auto-balance {queue?.paused
                    ? 'paused'
                    : 'on'}</button
            >
            <InfoTip label="About auto-balance"
                >Admits approved checks one at a time when there is room. Pausing holds new checks;
                running work finishes. Keep responsive reserves more CPU headroom. No model runs
                continuously.</InfoTip
            >
        </div>
    </header>
    <div class="hub-capacity">
        <div>
            <span
                ><Icon name="cpu" size={14} /> CPU
                <strong>{percent(cpu)}</strong></span
            ><span>Available memory <strong>{memory(snapshot.memory_available_bytes)}</strong></span
            >
        </div>
        <div
            class="capacity-track"
            role="meter"
            aria-label="Total CPU usage"
            aria-valuemin="0"
            aria-valuemax="100"
            aria-valuenow={cpu}
        >
            <span style:width={`${Math.min(100, Math.max(0, cpu))}%`}></span>
        </div>
    </div>
    {#if snapshot.governor}<HeldAgents governor={snapshot.governor} {demo} />{/if}
    {#if protectedNow}<div class="protection-strip">
            <Icon name="app" size={16} /><strong>Making room for your apps</strong><span
                >Until {new Date(protectedUntil * 1000).toLocaleTimeString([], {
                    hour: 'numeric',
                    minute: '2-digit',
                })}</span
            ><button class="text-button" disabled={busy} onclick={() => oncontrol('unprotect')}
                >End protection</button
            >
        </div>{/if}
    <div class="hub-toolbar">
        <button disabled={!queue || busy || protectedNow} onclick={() => oncontrol('protect')}
            >Make room · 20 min</button
        ><button
            disabled={planBusy || !queue || !queue.jobs.some((job) => job.stage === 'queued')}
            onclick={makePlan}>{planBusy ? 'Planning…' : 'Find a plan'}</button
        ><InfoTip label="About AI plans"
            >Sends task titles, scheduling goal, and resource measurements to your configured
            intelligent model. It can propose a hold or a next check. You apply the plan; resource
            limits still apply.</InfoTip
        ><button class="text-button connect" disabled={demo} onclick={() => onmanage({})}
            >Connect workloads</button
        >
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if planError}<p class="error" role="alert">{planError}</p>{/if}
    {#if plan}<div class="hub-plan" aria-label="Scheduling plan">
            <Icon name="companion" />
            <div><strong>{plan.reason}</strong><small>{plan.provider} · {plan.model}</small></div>
            {#if plan.action !== 'none'}<button
                    class="primary"
                    disabled={busy}
                    onclick={async () => {
                        if (await oncontrol(plan!.action, plan!.job_id ?? undefined)) plan = null;
                    }}>Apply plan</button
                >{/if}<button
                class="icon-button"
                aria-label="Dismiss plan"
                onclick={() => {
                    plan = null;
                }}>×</button
            >
        </div>{/if}
    {#if !queue}<p class="muted">Reading workflow controls…</p>{/if}
    {#if queue && current.length === 0}<div class="hub-empty">
            <Icon name="queue" size={30} />
            <h2>Ready for your next task.</h2>
            <p>Connect a test or build to let Jitterbug manage when it runs.</p>
            <button class="primary" disabled={demo} onclick={() => onmanage({})}
                >Connect a workload</button
            >
        </div>{/if}
    {#each [{ label: 'Working now', tasks: current }, { label: 'Recent', tasks: recent.slice(0, 6) }] as group}
        {#if group.tasks.length}
            {#if group.label === 'Recent'}<button
                    class="text-button recent-toggle"
                    aria-expanded={showRecent}
                    onclick={() => {
                        showRecent = !showRecent;
                    }}
                    ><span aria-hidden="true">{showRecent ? '▾' : '▸'}</span> Recent · {recent.length}</button
                >{/if}
            <div hidden={group.label === 'Recent' && !showRecent}>
                <div class="hub-group">
                    <h2>{group.label}</h2>
                    <span>Step</span><span class="metrics-label">CPU / Memory</span>
                </div>
                {#each group.tasks as row (row.id)}
                    <article class="hub-task" aria-label={row.title}>
                        <div class="hub-row">
                            <button
                                class="task-toggle"
                                aria-expanded={expanded === row.id}
                                onclick={() => {
                                    expanded = expanded === row.id ? null : row.id;
                                }}
                            >
                                <HarnessBadge harness={row.harnesses[0] ?? 'Other'} compact />
                                <span
                                    ><strong>{row.title}</strong><span class="task-harness"
                                        >{row.harnesses.join(' · ')}{row.kind === 'job'
                                            ? ' · Check'
                                            : row.kind === 'session'
                                              ? ' · Checkpoint'
                                              : ''}</span
                                    ></span
                                >
                            </button>
                            <div class="step-cell">
                                <div>
                                    <span
                                        >{row.phase === 'observed'
                                            ? 'Working'
                                            : row.phase[0].toUpperCase() + row.phase.slice(1)}</span
                                    ><span
                                        class:completed={row.complete &&
                                            ![
                                                'Failed',
                                                'Stale',
                                                'Cancelled',
                                                'Interrupted',
                                            ].includes(row.state)}
                                        class="step-state">{row.state}</span
                                    >
                                </div>
                                <div class="step-track" aria-hidden="true">
                                    {#each row.kind === 'job' ? ['check'] : ['planning', 'editing', 'tests', 'review'] as phase}<span
                                            class:filled={phase === row.phase}
                                            class:held={[
                                                'Held',
                                                'Waiting',
                                                'Paused at checkpoint',
                                                'Pause requested',
                                            ].includes(row.state)}
                                        ></span>{/each}
                                </div>
                            </div>
                            <div class="row-metrics">
                                {#if row.cpu !== null}<strong>{percent(row.cpu)}</strong><small
                                        >{memory(row.memory ?? 0)}{row.peak ? ' · peak' : ''}</small
                                    >{:else}<span>—</span><small
                                        >{row.kind === 'session'
                                            ? 'Not attributed'
                                            : 'Not sampled'}</small
                                    >{/if}
                            </div>
                            <div class="row-controls">
                                {#if row.canPrioritize}<button
                                        class="icon-button"
                                        aria-label={`Run ${row.title} next`}
                                        aria-pressed={row.next}
                                        title="Run next"
                                        disabled={busy}
                                        onclick={() => oncontrol('run_next', row.id)}>↑</button
                                    >{/if}{#if row.action}<button
                                        class="icon-button"
                                        aria-label={`${row.actionLabel}: ${row.title}`}
                                        title={row.actionLabel}
                                        disabled={busy}
                                        onclick={() => oncontrol(row.action!, row.id)}
                                        ><Icon
                                            name={row.action.startsWith('resume')
                                                ? 'play'
                                                : 'pause'}
                                            size={16}
                                        /></button
                                    >{:else if row.kind === 'observed' && !row.complete}<button
                                        class="text-button"
                                        disabled={demo}
                                        onclick={() => onmanage({ task_id: row.id })}
                                        >Connect</button
                                    >{/if}
                            </div>
                        </div>
                        {#if expanded === row.id}<div class="hub-task-detail">
                                <p>{row.detail}</p>
                                {#if row.workflow}<TaskDetail
                                        workflow={row.workflow}
                                        {snapshot}
                                        {onselect}
                                        {onusage}
                                        {onmanage}
                                    />{/if}{#if row.kind === 'job' && row.action}<button
                                        class="text-button"
                                        disabled={busy}
                                        onclick={() => oncontrol('cancel', row.id)}
                                        >Cancel waiting check</button
                                    >{/if}
                            </div>{/if}
                    </article>
                {/each}
            </div>
        {/if}
    {/each}
    {#if otherApps.length}<div class="hub-other">
            <span>Other apps</span>{#each otherApps as workload}<button
                    class="text-button"
                    onclick={() => onselect(workload)}
                    >{workload.label} <small>{percent(workload.cpu_percent)}</small></button
                >{/each}
        </div>{/if}
    <footer class="hub-activity">
        <Icon name="activity" size={16} /><span aria-live="polite"
            >{activity?.message ?? queue?.reason ?? 'Waiting for workflow data.'}</span
        >
        <details>
            <summary aria-label="Recent scheduling activity">History</summary>
            <div>
                {#each [...(queue?.control?.events ?? [])].reverse().slice(0, 8) as event}<p>
                        <time
                            >{new Date(event.at * 1000).toLocaleTimeString([], {
                                hour: 'numeric',
                                minute: '2-digit',
                            })}</time
                        >{event.message}
                    </p>{/each}
            </div>
        </details>
    </footer>
    {#if demo}<div class="hub-demo">
            <span>SIMULATED</span><button onclick={() => oncontrol('demo_pressure')}
                >{(queue?.resources.cpu_percent ?? 0) >= 85
                    ? 'Clear spike'
                    : 'Resource spike'}</button
            ><button onclick={() => oncontrol('demo_checkpoint')}>Next checkpoint</button>
        </div>{/if}
</section>

<style>
    .recent-toggle {
        margin: 14px 0;
        display: flex;
        gap: 7px;
    }
    .workflow-hub {
        max-width: 1100px;
        margin: 0 auto;
    }
    .hub-heading,
    .hub-settings,
    .hub-toolbar {
        display: flex;
        align-items: center;
        gap: 10px;
        flex-wrap: wrap;
    }
    .hub-heading {
        justify-content: space-between;
        margin-bottom: 25px;
    }
    h1 {
        font-size: 29px;
        letter-spacing: -0.8px;
        margin: 6px 0 0;
    }
    .hub-settings select {
        padding: 7px;
        border-radius: 7px;
        color: var(--text);
        background: var(--control);
        border: 1px solid var(--border);
        max-width: 170px;
    }
    .auto-toggle {
        display: flex;
        align-items: center;
        gap: 7px;
        padding: 7px 10px;
    }
    .auto-toggle span {
        width: 6px;
        height: 6px;
        border-radius: 50%;
        background: var(--success);
    }
    .auto-toggle span.off {
        background: var(--muted);
    }
    .hub-capacity {
        margin-bottom: 20px;
    }
    .hub-capacity > div:first-child {
        display: flex;
        justify-content: space-between;
        gap: 10px;
        font-size: 12px;
        color: var(--muted);
    }
    .hub-capacity span {
        display: flex;
        align-items: center;
        gap: 7px;
    }
    .hub-capacity strong {
        color: var(--text);
        font-weight: 500;
        font-variant-numeric: tabular-nums;
    }
    .capacity-track {
        height: 4px;
        border-radius: 4px;
        margin-top: 10px;
        background: var(--track);
        overflow: hidden;
    }
    .capacity-track span {
        height: 100%;
        background: var(--accent);
        transition: width 220ms;
    }
    .hub-toolbar {
        margin: 18px 0 24px;
    }
    .hub-toolbar .connect {
        margin-left: auto;
    }
    .protection-strip {
        display: flex;
        gap: 10px;
        align-items: center;
        flex-wrap: wrap;
        padding: 12px;
        border-radius: 9px;
        background: var(--accent-soft);
        font-size: 12px;
    }
    .protection-strip span {
        color: var(--muted);
    }
    .protection-strip button {
        margin-left: auto;
    }
    .hub-group,
    .hub-row {
        display: grid;
        grid-template-columns: minmax(180px, 1.3fr) minmax(145px, 1fr) 95px 78px;
        gap: 16px;
        align-items: center;
    }
    .hub-group {
        color: var(--muted);
        font-size: 11px;
        margin-top: 24px;
        padding-bottom: 9px;
    }
    .hub-group h2 {
        font-size: 12px;
        font-weight: 500;
        margin: 0;
        color: var(--muted);
    }
    .metrics-label {
        text-align: right;
    }
    .hub-task {
        border-top: 1px solid var(--border);
    }
    .hub-row {
        padding: 17px 0;
    }
    .task-toggle {
        display: flex;
        gap: 10px;
        align-items: center;
        min-width: 0;
        text-align: left;
        border: 0;
        padding: 0;
        background: transparent;
    }
    .task-toggle > span {
        min-width: 0;
    }
    .task-toggle strong {
        display: block;
        font-size: 13px;
        font-weight: 600;
        overflow-wrap: anywhere;
    }
    .task-harness {
        display: block;
        font-size: 11px;
        color: var(--muted);
        margin-top: 4px;
    }
    .step-cell > div:first-child {
        display: flex;
        justify-content: space-between;
        gap: 9px;
        font-size: 11px;
    }
    .step-state {
        color: var(--muted);
        text-align: right;
    }
    .step-state.completed {
        color: var(--success);
    }
    .step-track {
        display: flex;
        gap: 4px;
        margin-top: 8px;
    }
    .step-track span {
        flex: 1;
        background: var(--track);
        border-radius: 2px;
        height: 3px;
    }
    .step-track .filled {
        background: var(--accent);
    }
    .step-track .filled.held {
        background: var(--warning);
    }
    .row-metrics {
        text-align: right;
        font-size: 12px;
        font-variant-numeric: tabular-nums;
    }
    .row-metrics strong {
        font-weight: 500;
    }
    .row-metrics small {
        display: block;
        color: var(--muted);
        font-size: 11px;
        margin-top: 3px;
    }
    .row-controls {
        display: flex;
        justify-content: flex-end;
        gap: 6px;
    }
    .row-controls button {
        min-width: 30px;
        height: 32px;
    }
    .row-controls [aria-pressed='true'] {
        background: var(--accent-soft);
        color: var(--accent);
    }
    .hub-task-detail {
        padding: 0 0 18px 44px;
    }
    .hub-task-detail > p {
        font-size: 12px;
        color: var(--muted);
        margin: 0 0 14px;
    }
    .hub-other {
        display: flex;
        align-items: center;
        gap: 16px;
        flex-wrap: wrap;
        padding: 18px 0;
        border-top: 1px solid var(--border);
        color: var(--muted);
        font-size: 12px;
    }
    .hub-other small {
        color: var(--muted);
        margin-left: 5px;
    }
    .hub-activity {
        display: flex;
        gap: 9px;
        align-items: flex-start;
        padding: 16px 0;
        font-size: 12px;
        border-top: 1px solid var(--border);
    }
    .hub-activity > span {
        flex: 1;
    }
    .hub-activity > :global(svg) {
        color: var(--accent);
        flex-shrink: 0;
    }
    .hub-activity details {
        max-width: 50%;
        color: var(--muted);
    }
    .hub-activity summary {
        cursor: pointer;
    }
    .hub-activity p {
        display: flex;
        gap: 10px;
        font-size: 11px;
    }
    .hub-activity time {
        white-space: nowrap;
    }
    .hub-empty {
        padding: 36px 0;
        text-align: center;
    }
    .hub-empty p {
        color: var(--muted);
    }
    .hub-demo {
        display: flex;
        justify-content: center;
        gap: 10px;
        align-items: center;
        margin-top: 20px;
    }
    .hub-demo > span {
        color: var(--muted);
        font-size: 10px;
        letter-spacing: 1px;
    }
    .hub-plan {
        display: flex;
        gap: 12px;
        align-items: center;
        background: var(--accent-soft);
        border-radius: 10px;
        padding: 14px;
        margin-bottom: 16px;
    }
    .hub-plan > div {
        flex: 1;
    }
    .hub-plan strong {
        font-weight: 500;
        font-size: 13px;
    }
    .hub-plan small {
        display: block;
        color: var(--muted);
        font-size: 11px;
        margin-top: 4px;
    }
    @media (max-width: 1000px) {
        .hub-group,
        .hub-row {
            grid-template-columns: minmax(150px, 1fr) minmax(125px, 1fr) 70px;
            gap: 12px;
        }
        .row-metrics,
        .metrics-label {
            display: none;
        }
    }
    @media (max-width: 560px) {
        .hub-group {
            display: none;
        }
        .hub-row {
            grid-template-columns: 1fr 70px;
        }
        .step-cell {
            grid-column: 1 / -1;
            grid-row: 2;
            padding-left: 44px;
        }
        .row-controls {
            grid-column: 2;
            grid-row: 1;
        }
        .hub-task-detail {
            padding-left: 0;
        }
        .hub-plan {
            flex-wrap: wrap;
        }
    }
</style>
