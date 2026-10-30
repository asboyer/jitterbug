<script lang="ts">
    import { tick, untrack } from 'svelte';
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import { errorMessage, memory, percent } from '../lib/format';
    import { demoDetails } from '../lib/demo';
    import type {
        AppProfile,
        ProcessRecord,
        Reply,
        Snapshot,
        Workload,
        WorkloadDetails,
    } from '../lib/types';
    import AppIcon from './AppIcon.svelte';
    import InfoTip from './InfoTip.svelte';
    import ProcessInsight from './ProcessInsight.svelte';
    import Icon from './Icon.svelte';
    import InsidePanel from './InsidePanel.svelte';
    import type { ManagementSelection } from '../lib/manage';

    let {
        snapshot,
        workload,
        actionBusy,
        onback,
        onask,
        onaction,
        onusage,
        onmanage,
    }: {
        snapshot: Snapshot;
        workload: Workload;
        actionBusy: boolean;
        onback: () => void;
        onask: (process: ProcessRecord | null, reply?: Reply | null) => void;
        onaction: (process: ProcessRecord, action: string) => void;
        onusage: () => void;
        onmanage: (selection: ManagementSelection) => void;
    } = $props();
    let detail: WorkloadDetails | null = $state(null);
    let selected: { pid: number; start_time: number } | null = $state(null);
    let error = $state('');
    let profileError = $state('');
    let generating = $state(false);
    let profileErrorKey = $state('');
    const attemptedProfiles = new Set<string>();
    let search = $state('');
    let sequence = 0;
    let revealOrigin = false;
    let originOpen = $state(false);
    let originPanel: HTMLElement | undefined = $state();
    const demo = $derived(snapshot.source === 'demo' || !isTauri());
    const current = $derived.by(() => detail?.workload ?? workload);
    const members = $derived.by(
        () =>
            detail?.members.filter((p) =>
                `${p.name} ${p.pid}`.toLowerCase().includes(search.toLowerCase())
            ) ?? []
    );
    const process = $derived.by(() => detail?.process ?? null);
    const leaf = $derived.by(() => detail?.ancestry.at(-1));
    const parent = $derived.by(() => detail?.ancestry.at(-2));
    const graph = $derived(
        current.history
            .map(
                (point, index) =>
                    `${(index / Math.max(current.history.length - 1, 1)) * 200},${36 - Math.min(point.cpu_percent, 100) * 0.34}`
            )
            .join(' ')
    );
    const date = (time: number) => (time ? new Date(time * 1000).toLocaleString() : 'Unavailable');

    async function load(id: string, identity: typeof selected, sample: Snapshot) {
        const request = ++sequence;
        try {
            const next =
                sample.source === 'demo' || !isTauri()
                    ? demoDetails(sample, id)
                    : await invoke<WorkloadDetails>('get_workload_details', {
                          workloadId: id,
                          pid: identity?.pid ?? null,
                          startTime: identity?.start_time ?? null,
                      });
            if (request !== sequence) return;
            if (
                detail?.profile_key === next.profile_key &&
                detail?.profile.source === 'model' &&
                next.profile.source === 'inference'
            )
                next.profile = detail.profile;
            detail = next;
            error = '';
            if (revealOrigin) {
                revealOrigin = false;
                originOpen = true;
                await tick();
                originPanel?.scrollIntoView({ block: 'start', behavior: 'smooth' });
            }
        } catch (cause) {
            if (request === sequence) error = errorMessage(cause);
        }
    }

    $effect(() => {
        // Telemetry events stop when the window is hidden. No extra polling or model calls.
        const sample = snapshot;
        const id = workload.id;
        const identity = selected;
        sample.captured_at;
        untrack(() => void load(id, identity, sample));
        return () => {
            sequence++;
        };
    });

    $effect(() => {
        if (
            detail &&
            !demo &&
            !error &&
            !generating &&
            detail.can_generate_profile &&
            detail.profile.source === 'inference' &&
            detail.profile_key &&
            !attemptedProfiles.has(detail.profile_key)
        ) {
            void explain();
        }
    });

    async function explain() {
        if (!detail || demo || generating || !detail.profile_key) return;
        const key = detail.profile_key;
        attemptedProfiles.add(key);
        generating = true;
        profileError = '';
        profileErrorKey = key;
        try {
            const profile = await invoke<AppProfile>('explain_application', {
                workloadId: workload.id,
                pid: detail.process.pid,
                startTime: detail.process.start_time,
            });
            // Telemetry may refresh or the selected process may change during inference.
            // A general explanation belongs to the executable identity, not a stale PID.
            if (detail?.profile_key === key) detail = { ...detail, profile };
        } catch (cause) {
            if (detail?.profile_key === key) profileError = errorMessage(cause);
        } finally {
            generating = false;
            onusage();
        }
    }
    async function showOrigin() {
        originOpen = true;
        await tick();
        originPanel?.scrollIntoView({ block: 'start', behavior: 'smooth' });
        originPanel?.focus({ preventScroll: true });
    }
</script>

<div class="workload-detail">
    <button class="text-button back" onclick={onback}>‹ Back to workloads</button>
    <div class="detail-heading">
        <AppIcon workloadId={workload.id} category={workload.category} size={44} {demo} />
        <div class="heading-copy">
            <h1>{workload.label}</h1>
            <p>
                {detail?.launch
                    ? `${detail.launch.program} · ${detail.launch.task}`
                    : (detail?.application?.installation ?? workload.category)}
            </p>
        </div>
        <button disabled={demo} onclick={() => onmanage({ workload_id: workload.id })}
            >Manage workload</button
        >
    </div>
    <div class="detail-stats" aria-label="Workload resource use">
        <div>
            <span>CPU</span><strong
                >{snapshot.sample_ready ? percent(current.cpu_percent) : '—'}</strong
            >
        </div>
        <div><span>Memory</span><strong>{memory(current.memory_bytes)}</strong></div>
        <div><span>Processes</span><strong>{current.process_count}</strong></div>
        <div class="workload-history">
            <span>Recent CPU · 0–100%</span><svg
                viewBox="0 0 200 38"
                preserveAspectRatio="none"
                role="img"
                aria-label="Workload CPU history, zero to 100 percent"
                ><path d="M0 36H200" stroke="var(--border)" />{#if graph}<polyline
                        points={graph}
                        fill="none"
                        stroke="var(--accent)"
                        stroke-width="1.5"
                        vector-effect="non-scaling-stroke"
                    />{/if}</svg
            >
        </div>
    </div>
    {#if error}<div class="banner error" role="alert">
            {error}<button
                onclick={() => {
                    selected = null;
                    void load(workload.id, null, snapshot);
                }}>Refresh selection</button
            >
        </div>{/if}
    {#if detail}
        <div class="process-overview">
            <section
                class="panel app-explanation"
                aria-label="Process explanation"
                aria-busy={generating}
            >
                <div class="section-title">
                    <h2>{detail.launch ? 'Why it’s running' : 'What it does'}</h2>
                    <div class="explanation-source">
                        <span
                            >{detail.launch
                                ? 'Command verified locally'
                                : detail.profile.source === 'model'
                                  ? detail.profile.cached
                                      ? 'AI · saved on this device'
                                      : 'AI explanation'
                                  : detail.profile.source === 'reference'
                                    ? 'Built-in reference'
                                    : demo
                                      ? 'Synthetic example'
                                      : generating
                                        ? 'Identifying…'
                                        : 'Local evidence'}</span
                        >
                        <InfoTip label="About this explanation">
                            {#if detail.launch}
                                <p>{detail.launch.limitation}</p>
                                <p>
                                    Commands are inspected on this device. Only a recognized job
                                    category can be sent for an AI investigation.
                                </p>
                            {:else}
                                <p>
                                    General purpose, not a diagnosis of current activity. Cached for
                                    up to 30 days; changes to the executable or app version
                                    invalidate the saved explanation.
                                </p>
                                <p>
                                    Models receive the executable name, component and bundle
                                    identifier. Paths, arguments, working directories and files stay
                                    on this device.
                                </p>
                                {#if detail.profile.generated_at}<p>
                                        {detail.profile.provider} · {detail.profile.model} · {date(
                                            detail.profile.generated_at
                                        )}
                                    </p>{/if}
                                {#if detail.application}<p>
                                        {detail.application.identity_source}{detail.application
                                            .identifier
                                            ? ` · ${detail.application.identifier}`
                                            : ''}. This does not verify a publisher’s signature.
                                    </p>{/if}
                            {/if}
                        </InfoTip>
                    </div>
                </div>
                {#if detail.launch}
                    <h3 class="launch-task">{detail.launch.program} · {detail.launch.task}</h3>
                    <p class="explanation-text">{detail.launch.summary}</p>
                {:else if generating && detail.profile.source === 'inference'}<p
                        class="explanation-text identifying"
                        role="status"
                    >
                        Identifying {detail.process.name} and what it does…
                    </p>
                {:else if profileError && profileErrorKey === detail.profile_key}<div
                        class="profile-generation"
                        role="status"
                    >
                        <p>The explanation couldn’t be completed.</p>
                        <button disabled={generating || !!error} onclick={explain}
                            >Retry explanation</button
                        ><InfoTip label="Why the explanation is unavailable"
                            ><p>{profileError}</p></InfoTip
                        >
                    </div>
                {:else}<p class="explanation-text">{detail.profile.text}</p>{/if}
                <div class="origin-preview" aria-label="Origin overview">
                    <Icon name="app" size={16} />
                    <div>
                        <small>Part of</small><strong
                            >{detail.launch?.program ??
                                detail.application?.component ??
                                detail.application?.executable_name ??
                                detail.process.name}</strong
                        >
                    </div>
                    <div>
                        <small>Launched by</small><strong
                            >{detail.launch?.launcher ??
                                (parent?.name === 'launchd'
                                    ? 'macOS · launchd'
                                    : (parent?.name ?? 'Parent not observed'))}</strong
                        >
                    </div>
                    <button class="text-button" onclick={showOrigin}
                        >Show origin evidence <span aria-hidden="true">↓</span></button
                    >
                </div>
            </section>
            <ProcessInsight
                {snapshot}
                workload={current}
                {demo}
                disabled={!!error || generating || actionBusy}
                {onusage}
                onask={(reply) => onask(null, reply)}
                {onaction}
            />
        </div>
        <InsidePanel {snapshot} workload={current} {demo} {actionBusy} {onaction} />
        <details class="panel process-section">
            <summary
                ><span>Processes</span><span class="summary-count">{detail.member_count}</span
                ></summary
            >
            <div class="section-title process-toolbar">
                <input
                    aria-label="Filter processes"
                    placeholder="Name or PID"
                    bind:value={search}
                /><InfoTip label="About process measurements"
                    ><p>
                        CPU is a share of all cores. Memory is summed resident memory and may count
                        shared pages more than once. Showing {detail.members.length} of {detail.member_count}
                        observed members.
                    </p>
                    <p>
                        {detail.independent_roots > 1
                            ? `${detail.independent_roots} independent roots; this group is not a single launch.`
                            : 'Select a process to see its observed parent chain.'}
                    </p></InfoTip
                >
            </div>
            <div class="members" role="group" aria-label="Workload processes">
                {#each members as member (`${member.pid}:${member.start_time}`)}<button
                        class="member"
                        class:chosen={member.pid === process?.pid &&
                            member.start_time === process?.start_time}
                        aria-pressed={member.pid === process?.pid &&
                            member.start_time === process?.start_time}
                        onclick={() => {
                            revealOrigin = true;
                            selected = { pid: member.pid, start_time: member.start_time };
                        }}
                        aria-label={`Inspect ${member.name}, PID ${member.pid}`}
                    >
                        <span class="member-name"
                            ><strong>{member.name}</strong><small>PID {member.pid}</small></span
                        ><span class="numeric">{percent(member.cpu_percent)}<small>CPU</small></span
                        ><span class="numeric"
                            >{memory(member.memory_bytes)}<small>Memory</small></span
                        ><span aria-hidden="true">›</span>
                    </button>{:else}<p class="muted empty">No matching processes.</p>{/each}
            </div>
        </details>
        {#if process}<section
                class="panel origin-panel"
                aria-label="Process origins"
                bind:this={originPanel}
                tabindex="-1"
            >
                <details bind:open={originOpen}>
                    <summary
                        ><span>Origin & technical details</span><span class="summary-count"
                            >PID {process.pid}</span
                        ></summary
                    >
                    <div class="origin-content">
                        {#if detail.launch}
                            <div class="launch-evidence">
                                {#each detail.launch.evidence as evidence}<p>{evidence}</p>{/each}
                                {#if detail.launch.reference_url}<a
                                        href={detail.launch.reference_url}
                                        target="_blank"
                                        rel="noreferrer">About this background service ↗</a
                                    >{/if}
                            </div>
                        {/if}
                        <div class="origin-title">
                            <div>
                                <h2>{process.name}</h2>
                                <span class="muted"
                                    >Observed {new Date(
                                        detail.captured_at * 1000
                                    ).toLocaleTimeString()}</span
                                >
                            </div>
                            <InfoTip label="About process origins"
                                ><p>{detail.origin_summary}</p>
                                <p>{detail.origin_limit}</p>
                                <p>
                                    Paths stay on this device. These are current OS parents, not a
                                    historical launch log.
                                </p></InfoTip
                            >
                        </div>
                        <ol class="origin-chain" aria-label="Observed parent chain">
                            {#each detail.ancestry as ancestor, index}<li
                                    class:current={index === detail.ancestry.length - 1}
                                >
                                    <span class="origin-dot" aria-hidden="true"></span>
                                    <div class="ancestor-copy">
                                        <div class="ancestor-heading">
                                            <strong
                                                >{detail.launch?.chain.find(
                                                    (step) => step.pid === ancestor.pid
                                                )?.role ?? ancestor.name}</strong
                                            ><span class="pid">PID {ancestor.pid}</span
                                            >{#if index === detail.ancestry.length - 1}<span
                                                    class="selected-label">Selected</span
                                                >{/if}
                                        </div>
                                        {#if ancestor.start_time}<small
                                                >Started {date(ancestor.start_time)}</small
                                            >{/if}
                                    </div>
                                </li>{/each}
                        </ol>
                        <dl class="process-facts">
                            <div>
                                <dt>Executable</dt>
                                <dd>{leaf?.executable ?? 'Not exposed by the OS'}</dd>
                            </div>
                            <div>
                                <dt>Working folder</dt>
                                <dd>{leaf?.working_directory ?? 'Not exposed by the OS'}</dd>
                            </div>
                            <div>
                                <dt>Disk · last {snapshot.interval_ms / 1000}s</dt>
                                <dd>
                                    {memory(process.disk_read_bytes)} read · {memory(
                                        process.disk_written_bytes
                                    )} written
                                </dd>
                            </div>
                        </dl>
                        <div class="process-actions">
                            <button disabled={!!error} onclick={() => onask(process)}
                                >Investigate process</button
                            ><button
                                disabled={demo || !!error}
                                onclick={() =>
                                    onmanage({
                                        workload_id: workload.id,
                                        pid: process.pid,
                                        start_time: process.start_time,
                                    })}>Manage command</button
                            >{#if process.actionable && !demo}{#each process.actions as action}<button
                                        disabled={actionBusy || !!error}
                                        class:danger-text={action === 'terminate'}
                                        onclick={() => onaction(process, action)}
                                        >{action === 'terminate'
                                            ? 'Stop process…'
                                            : 'Lower priority…'}</button
                                    >{/each}{/if}
                        </div>
                    </div>
                </details>
            </section>{/if}
    {:else if !error}<p class="muted">Reading process details…</p>{/if}
</div>

<style>
    .launch-task {
        font-size: 17px;
        margin: 18px 0 8px;
        letter-spacing: -0.2px;
    }
    .launch-evidence {
        padding: 0 0 18px;
        margin-bottom: 18px;
        border-bottom: 1px solid var(--border);
        font-size: 12px;
        line-height: 1.6;
    }
    .launch-evidence p {
        margin: 4px 0;
    }
    .workload-detail {
        container-type: inline-size;
        container-name: process-detail;
        max-width: 920px;
        margin: 0 auto;
    }
    .back {
        margin: 0 0 20px -8px;
    }
    .detail-heading {
        display: flex;
        align-items: center;
        gap: 14px;
        margin-bottom: 26px;
        flex-wrap: wrap;
    }
    .heading-copy {
        flex: 1;
        min-width: 120px;
    }
    h1 {
        margin: 0;
        font-size: 25px;
        letter-spacing: -0.6px;
        overflow-wrap: anywhere;
    }
    .heading-copy p {
        margin: 5px 0 0;
        color: var(--muted);
        font-size: 12px;
    }
    .detail-stats {
        display: grid;
        grid-template-columns: repeat(3, 1fr) 1.5fr;
        gap: 22px;
        padding: 0 0 24px;
    }
    .detail-stats span {
        display: block;
        color: var(--muted);
        font-size: 11px;
        margin-bottom: 8px;
    }
    .detail-stats strong {
        font-size: 23px;
        font-weight: 500;
        letter-spacing: -0.5px;
        font-variant-numeric: tabular-nums;
    }
    .workload-history svg {
        height: 32px;
        width: 100%;
        display: block;
    }
    .process-overview {
        display: grid;
        gap: 16px;
        margin-bottom: 20px;
    }
    .process-overview :global(.activity-insight) {
        margin: 0;
        min-width: 0;
    }
    .app-explanation {
        display: flex;
        flex-direction: column;
        min-width: 0;
        padding: 18px 20px;
    }
    .app-explanation .explanation-text {
        flex: 1;
    }
    @container process-detail (min-width: 800px) {
        .process-overview {
            grid-template-columns: repeat(2, minmax(0, 1fr));
        }
    }
    .section-title {
        flex-wrap: wrap;
        gap: 10px;
        margin-bottom: 0;
    }
    h2 {
        font-size: 13px;
        margin: 0;
        font-weight: 600;
    }
    .explanation-text {
        font-size: 14px;
        line-height: 1.65;
        margin: 12px 0 18px;
        max-width: 72ch;
    }
    .explanation-source {
        display: flex;
        align-items: center;
        gap: 6px;
        color: var(--muted);
        font-size: 10px;
    }
    .identifying {
        color: var(--muted);
    }
    .origin-preview {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 14px 22px;
        padding-top: 16px;
        border-top: 1px solid var(--border);
    }
    .origin-preview > div {
        display: grid;
        gap: 5px;
    }
    .origin-preview small {
        color: var(--muted);
        font-size: 10px;
    }
    .origin-preview strong {
        font-size: 11px;
        font-weight: 500;
        overflow-wrap: anywhere;
    }
    .origin-preview button {
        margin-left: auto;
        font-size: 11px;
    }
    .profile-generation {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 12px;
    }
    .profile-generation p {
        flex: 1;
        color: var(--muted);
    }
    .process-section,
    .origin-panel {
        margin-bottom: 12px;
        padding: 0 18px;
    }
    summary {
        padding: 16px 0;
        font-size: 12px;
        font-weight: 500;
    }
    .summary-count {
        float: right;
        color: var(--muted);
        font-size: 11px;
        font-weight: 400;
    }
    .process-toolbar {
        padding: 0 0 8px;
    }
    .process-toolbar input {
        max-width: 220px;
    }
    .members {
        max-height: 280px;
        overflow: auto;
        margin-bottom: 12px;
    }
    .member {
        display: grid;
        grid-template-columns: minmax(100px, 1fr) 60px 80px 8px;
        align-items: center;
        gap: 14px;
        width: 100%;
        padding: 12px 8px;
        border: 0;
        box-shadow: none;
        background: transparent;
        text-align: left;
        border-radius: 6px;
    }
    .member:hover {
        background: var(--hover);
    }
    .member.chosen {
        background: var(--accent-soft);
    }
    .member-name {
        min-width: 0;
    }
    .member strong {
        display: block;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-weight: 500;
    }
    .member small {
        display: block;
        color: var(--muted);
        font-size: 10px;
        margin-top: 4px;
    }
    .origin-title {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 12px;
    }
    .origin-title .muted {
        display: block;
        margin-top: 6px;
        font-size: 11px;
    }
    .origin-content {
        padding: 4px 0 20px;
    }
    .origin-chain {
        padding: 0;
        margin: 24px 0;
        list-style: none;
    }
    .origin-chain li {
        display: flex;
        position: relative;
        gap: 16px;
        padding-bottom: 24px;
    }
    .origin-chain li:last-child {
        padding-bottom: 0;
    }
    .origin-chain li:not(:last-child)::before {
        content: '';
        position: absolute;
        left: 4px;
        top: 10px;
        height: 100%;
        border-left: 1px solid var(--border);
    }
    .origin-dot {
        z-index: 1;
        border: 2px solid var(--muted);
        border-radius: 50%;
        width: 9px;
        height: 9px;
        flex: 0 0 9px;
        margin-top: 4px;
        background: var(--panel);
    }
    .current .origin-dot {
        border-color: var(--accent);
        background: var(--accent);
    }
    .ancestor-copy {
        min-width: 0;
    }
    .ancestor-heading {
        display: flex;
        align-items: baseline;
        gap: 10px 14px;
        flex-wrap: wrap;
    }
    .ancestor-heading strong {
        font-size: 12px;
        font-weight: 500;
        overflow-wrap: anywhere;
    }
    .pid,
    .selected-label {
        color: var(--muted);
        font-size: 10px;
        white-space: nowrap;
    }
    .selected-label {
        color: var(--accent);
    }
    .origin-chain small {
        display: block;
        color: var(--muted);
        font-size: 11px;
        margin-top: 6px;
    }
    .process-facts {
        border-top: 1px solid var(--border);
        padding-top: 14px;
        font-size: 11px;
    }
    .process-facts > div {
        display: grid;
        grid-template-columns: 110px minmax(0, 1fr);
        gap: 16px;
        padding: 7px 0;
    }
    dt {
        color: var(--muted);
    }
    dd {
        margin: 0;
        overflow-wrap: anywhere;
        line-height: 1.5;
    }
    .process-actions {
        display: flex;
        gap: 10px;
        flex-wrap: wrap;
        margin-top: 20px;
    }
    @container process-detail (max-width: 700px) {
        .detail-stats {
            grid-template-columns: repeat(3, 1fr);
        }
        .workload-history {
            display: none;
        }
        .process-facts > div {
            grid-template-columns: 1fr;
            gap: 5px;
        }
    }
</style>
