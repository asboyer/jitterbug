<script lang="ts">
    import { tick } from 'svelte';
    import { memory, percent } from '../lib/format';
    import { currentTip, taskImpact } from '../lib/task-view';
    import HarnessBadge from './HarnessBadge.svelte';
    import TaskDetail from './TaskDetail.svelte';
    import QueuePreview from './QueuePreview.svelte';
    import InfoTip from './InfoTip.svelte';
    import Icon from './Icon.svelte';
    import type { Snapshot, Workload, AgentWorkflow } from '../lib/types';
    import type { ManagementSelection } from '../lib/manage';

    let {
        snapshot,
        onselect,
        onusage,
        onmanage,
        selectedId = $bindable(null),
    }: {
        snapshot: Snapshot;
        selectedId?: string | null;
        onselect: (workload: Workload) => void;
        onusage: () => void;
        onmanage: (selection: ManagementSelection) => void;
    } = $props();
    let preview = $state<AgentWorkflow | null>(null);
    let opener: HTMLButtonElement | null = null;
    let detailHeading: HTMLHeadingElement | undefined = $state();
    const selected = $derived(snapshot.agent_workflows.find((task) => task.task_id === selectedId));
    const active = $derived(snapshot.agent_workflows.filter((task) => task.status === 'active'));
    const recent = $derived(snapshot.agent_workflows.filter((task) => task.status !== 'active'));
    const status = (value: string) => value.charAt(0).toUpperCase() + value.slice(1);
    $effect(() => {
        if (selectedId) void tick().then(() => detailHeading?.focus());
    });
    function manage(workflow: AgentWorkflow) {
        if (snapshot.source === 'demo') preview = workflow;
        else onmanage({ task_id: workflow.task_id });
    }
    function view(workflow: AgentWorkflow, event: MouseEvent) {
        opener = event.currentTarget as HTMLButtonElement;
        selectedId = workflow.task_id;
    }
    function back() {
        const id = selectedId;
        selectedId = null;
        void tick().then(() => {
            [...document.querySelectorAll<HTMLButtonElement>('[data-task-view]')]
                .find((button) => button.dataset.taskView === id)
                ?.focus();
        });
    }
</script>

<section aria-label="Agent workflows" data-tour="agent-tasks">
    {#if selectedId}
        <button class="text-button back" onclick={back}>← All agent tasks</button>
        {#if selected}
            <div class="task-detail-heading">
                <div>
                    <div class="harnesses">
                        {#each [...new Set(selected.agents.map((agent) => agent.harness))] as harness}<HarnessBadge
                                {harness}
                            />{/each}
                    </div>
                    <h2 bind:this={detailHeading} tabindex="-1">{selected.task_title}</h2>
                </div>
                <span
                    class="run-status"
                    class:active={selected.status === 'active'}
                    class:completed={selected.status === 'completed'}
                    >{status(selected.status)}</span
                >
            </div>
            <section class="panel" aria-label="Task detail">
                {#key selected.task_id}<TaskDetail
                        workflow={selected}
                        {snapshot}
                        {onselect}
                        {onusage}
                        onmanage={() => manage(selected)}
                    />{/key}
            </section>
        {:else}<p>
                This task is no longer in the recent inventory. Return to the task list for current
                activity.
            </p>{/if}
    {:else}
        {#each [{ label: 'Working now', tasks: active }, { label: 'Recent activity', tasks: recent }] as group}
            {#if group.tasks.length}
                <div class="group-heading">
                    <h2>{group.label}</h2>
                    <span>{group.tasks.length}</span>
                </div>
                <div class="task-cards">
                    {#each group.tasks as workflow (workflow.task_id)}
                        {@const tip = currentTip(workflow)}
                        {@const running = workflow.status === 'active'}
                        {#if running}<article
                                class="panel task-card"
                                class:finished={!running}
                                aria-label={workflow.task_title}
                            >
                                <div class="task-topline">
                                    <div class="harnesses">
                                        {#each [...new Set(workflow.agents.map((agent) => agent.harness))] as harness}<HarnessBadge
                                                {harness}
                                            />{/each}
                                    </div>
                                    <span
                                        class="run-status"
                                        class:active={running}
                                        class:completed={workflow.status === 'completed'}
                                        class:failed={['failed', 'interrupted'].includes(
                                            workflow.status
                                        )}>{status(workflow.status)}</span
                                    >
                                </div>
                                <h3>{workflow.task_title}</h3>
                                {#if running || workflow.observed_samples}<div class="impact-row">
                                        <Icon name="activity" size={17} />
                                        <p>{taskImpact(workflow)}</p>
                                        <InfoTip
                                            label={`How resource use was attributed for ${workflow.task_title}`}
                                            ><p>
                                                Process roles are inferred from names and commands.
                                                Ownership uses the observed parent chain. This
                                                describes measured work, not a proven cause of
                                                slowness.
                                            </p></InfoTip
                                        >
                                    </div>
                                    {#if workflow.observed_samples}<div
                                            class="task-readings"
                                            aria-label={running
                                                ? 'Current task resources'
                                                : 'Retained task measurements'}
                                        >
                                            <span
                                                ><strong
                                                    >{workflow.observed_samples
                                                        ? percent(
                                                              running
                                                                  ? workflow.cpu_percent
                                                                  : workflow.peak_cpu_percent
                                                          )
                                                        : '—'}</strong
                                                >
                                                {running ? 'CPU now' : 'sampled CPU peak'}</span
                                            >
                                            <span
                                                ><strong
                                                    >{workflow.peak_process_count
                                                        ? memory(
                                                              running
                                                                  ? workflow.memory_bytes
                                                                  : workflow.peak_memory_bytes
                                                          )
                                                        : '—'}</strong
                                                >
                                                {running ? 'memory' : 'peak memory'}</span
                                            >
                                            <span
                                                >{workflow.agents.length}
                                                {workflow.agents.length === 1
                                                    ? 'session'
                                                    : 'sessions'}</span
                                            >
                                        </div>
                                    {/if}{:else}<p class="no-measurements">
                                        No resource samples available.
                                    </p>{/if}
                                {#if tip}
                                    <div class="task-tip">
                                        <Icon name="bulb" size={18} />
                                        <div>
                                            <strong
                                                >{tip.id.startsWith('overlapping-')
                                                    ? 'Let heavy checks take turns'
                                                    : tip.title}</strong
                                            >
                                            <p>
                                                {tip.id.startsWith('overlapping-')
                                                    ? 'Set up a shared queue for future test and build runs.'
                                                    : tip.recommendation}
                                            </p>
                                        </div>
                                    </div>
                                {/if}
                                <div class="task-actions">
                                    {#if running}<button
                                            class:primary={!!tip}
                                            onclick={(event) => {
                                                opener = event.currentTarget;
                                                manage(workflow);
                                            }}
                                            >{snapshot.source === 'demo'
                                                ? 'Preview managed checks'
                                                : 'Set up managed checks'}</button
                                        >{/if}
                                    <button
                                        class="text-button"
                                        data-task-view={workflow.task_id}
                                        onclick={(event) => view(workflow, event)}
                                        >View task <span aria-hidden="true">↗</span></button
                                    >
                                </div>
                            </article>
                        {:else}
                            <article class="panel recent-task" aria-label={workflow.task_title}>
                                <div class="recent-copy">
                                    <h3>{workflow.task_title}</h3>
                                    <div class="recent-meta">
                                        <div class="harnesses">
                                            {#each [...new Set(workflow.agents.map((agent) => agent.harness))] as harness}<HarnessBadge
                                                    {harness}
                                                />{/each}
                                        </div>
                                        <p class="retained">
                                            {workflow.observed_samples
                                                ? `Sampled peak · ${percent(workflow.peak_cpu_percent)} CPU · ${memory(workflow.peak_memory_bytes)}`
                                                : 'No resource samples available.'}
                                        </p>
                                    </div>
                                </div>
                                <span
                                    class="run-status"
                                    class:completed={workflow.status === 'completed'}
                                    class:failed={['failed', 'interrupted'].includes(
                                        workflow.status
                                    )}>{status(workflow.status)}</span
                                >
                                <button
                                    class="text-button"
                                    data-task-view={workflow.task_id}
                                    onclick={(event) => view(workflow, event)}
                                    >View task <span aria-hidden="true">↗</span></button
                                >
                            </article>
                        {/if}
                    {/each}
                </div>
            {/if}
        {/each}
        {#if !snapshot.agent_workflows.length}
            <div class="panel empty-tasks">
                <Icon name="code" size={28} />
                <h2>See the work behind your coding tools</h2>
                <p>
                    Connect a harness to see each task, what uses resources, and which checks could
                    take turns. Your other apps are already in Workloads.
                </p>
                <button onclick={() => onmanage({})}>Connect a workload</button>
            </div>
        {/if}
    {/if}
</section>
{#if preview}<QueuePreview
        workflow={preview}
        onclose={() => {
            preview = null;
            void tick().then(() => {
                if (opener?.isConnected) opener.focus();
                else document.querySelector<HTMLButtonElement>('.workflow-actions button')?.focus();
            });
        }}
    />{/if}

<style>
    .recent-task {
        display: flex;
        align-items: center;
        gap: 16px;
        padding: 16px 20px;
    }
    .recent-copy {
        flex: 1;
        min-width: 0;
    }
    .recent-copy h3 {
        font-size: 14px;
        margin: 0 0 9px;
    }
    .retained {
        color: var(--muted);
        font-size: 10px;
        margin: 0;
    }
    .recent-meta {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 8px 16px;
    }
    .recent-task > button {
        white-space: nowrap;
    }
    @media (max-width: 850px) {
        .recent-task {
            flex-wrap: wrap;
        }
        .recent-copy {
            flex-basis: 65%;
        }
    }
    .finished {
        padding-top: 16px;
        padding-bottom: 16px;
    }
    .no-measurements {
        font-size: 12px;
        color: var(--muted);
        margin: 0;
    }
    .group-heading {
        display: flex;
        gap: 10px;
        align-items: center;
        margin: 6px 0 12px;
    }
    .group-heading h2 {
        font-size: 13px;
        margin: 0;
    }
    .group-heading > span {
        font-size: 11px;
        color: var(--muted);
    }
    .task-cards {
        display: grid;
        gap: 14px;
        margin-bottom: 26px;
    }
    .task-card {
        padding: 20px;
    }
    .task-topline,
    .task-detail-heading {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
    }
    .harnesses {
        display: flex;
        gap: 14px;
        flex-wrap: wrap;
    }
    h3 {
        font-size: 17px;
        line-height: 1.4;
        margin: 14px 0 12px;
    }
    .impact-row {
        display: flex;
        gap: 9px;
        align-items: center;
        color: var(--muted);
    }
    .impact-row p {
        flex: 1;
        font-size: 13px;
        margin: 0;
        line-height: 1.5;
    }
    .task-readings {
        display: flex;
        gap: 12px 22px;
        flex-wrap: wrap;
        margin-top: 14px;
        font-size: 11px;
        color: var(--muted);
    }
    .task-readings strong {
        font-size: 13px;
        color: var(--text);
        font-weight: 550;
        font-variant-numeric: tabular-nums;
    }
    .task-tip {
        margin-top: 18px;
        padding: 14px;
        display: flex;
        align-items: start;
        gap: 10px;
        background: var(--accent-soft);
        border-radius: 9px;
    }
    .task-tip > :global(svg) {
        color: var(--accent);
        flex-shrink: 0;
        margin-top: 1px;
    }
    .task-tip strong {
        font-size: 13px;
        font-weight: 550;
    }
    .task-tip p {
        margin: 5px 0 0;
        font-size: 12px;
        line-height: 1.5;
        color: var(--muted);
    }
    .task-actions {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 14px;
        margin-top: 18px;
    }
    .run-status {
        font-size: 10px;
        padding: 4px 7px;
        border-radius: 5px;
        background: var(--hover);
        color: var(--muted);
    }
    .run-status.active {
        color: var(--accent);
        background: var(--accent-soft);
    }
    .run-status.completed {
        color: var(--success);
        background: var(--success-soft);
    }
    .run-status.failed {
        color: var(--warning);
        background: var(--warning-soft);
    }
    .back {
        margin-bottom: 20px;
    }
    .task-detail-heading {
        margin-bottom: 20px;
        align-items: start;
    }
    .task-detail-heading h2 {
        font-size: 20px;
        margin: 12px 0 0;
    }
    .empty-tasks {
        padding: 28px;
    }
    .empty-tasks h2 {
        font-size: 18px;
    }
    .empty-tasks p {
        max-width: 440px;
        color: var(--muted);
        line-height: 1.6;
        margin-bottom: 20px;
    }
</style>
