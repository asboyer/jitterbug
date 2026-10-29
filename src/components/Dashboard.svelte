<script lang="ts">
    import { memory, percent } from '../lib/format';
    import type { Snapshot, Workload } from '../lib/types';
    import ActivityGraph from './ActivityGraph.svelte';
    import ResourceRing from './ResourceRing.svelte';
    import AppIcon from './AppIcon.svelte';
    import AgentWorkflows from './AgentWorkflows.svelte';
    import ValidationQueue from './ValidationQueue.svelte';
    import type { ManagementSelection } from '../lib/manage';

    let {
        snapshot,
        selectedId,
        onselect,
        onask,
        onusage,
        onmanage,
    }: {
        snapshot: Snapshot;
        selectedId: string | null;
        onselect: (workload: Workload) => void;
        onask: () => void;
        onusage: () => void;
        onmanage: (selection: ManagementSelection) => void;
    } = $props();
    let search = $state('');
    let sort = $state('impact');
    const usedPercent = $derived(
        100 * (1 - snapshot.memory_available_bytes / snapshot.memory_total_bytes)
    );
    const breakdown = $derived(snapshot.platform.memory);
    const rows = $derived.by(() => {
        const filtered = snapshot.workloads.filter((w) =>
            `${w.label} ${w.category}`.toLowerCase().includes(search.toLowerCase())
        );
        if (sort === 'cpu') return filtered.sort((a, b) => b.cpu_percent - a.cpu_percent);
        if (sort === 'memory') return filtered.sort((a, b) => b.memory_bytes - a.memory_bytes);
        return filtered;
    });
</script>

<div class="dashboard">
    <div class="page-heading">
        <div>
            <h1>Activity</h1>
        </div>
        <button class="primary" onclick={onask}
            >Ask Jitterbug <span aria-hidden="true">↗</span></button
        >
    </div>
    <section class="system-panel panel" aria-label="System resources">
        <div class="metrics">
            <article class="metric">
                <ResourceRing value={snapshot.cpu_percent} available={snapshot.sample_ready} />
                <div>
                    <div class="metric-label">CPU</div>
                    <strong>{snapshot.sample_ready ? percent(snapshot.cpu_percent) : '—'}</strong>
                    <small>{snapshot.sample_ready ? 'of total capacity' : 'Sampling…'}</small>
                </div>
            </article>
            <article class="metric">
                <ResourceRing value={100 - usedPercent} />
                <div>
                    <div class="metric-label">Memory available</div>
                    <strong>{memory(snapshot.memory_available_bytes)}</strong>
                    <small>of {memory(snapshot.memory_total_bytes)}</small>
                </div>
            </article>
            <article class="metric pressure-metric">
                <div
                    class="pressure-symbol"
                    class:elevated={snapshot.pressure.level !== 'normal'}
                    aria-hidden="true"
                >
                    <svg viewBox="0 0 48 48"><path d="M5 25h9l5-12 9 24 6-12h9" /></svg>
                </div>
                <div>
                    <div class="metric-label">Pressure</div>
                    <strong
                        >{snapshot.pressure.level === 'normal'
                            ? 'Normal'
                            : snapshot.pressure.level === 'watch'
                              ? 'Elevated'
                              : 'High'}</strong
                    >
                    <small>Thermal: {snapshot.platform.thermal ?? 'unavailable'}</small>
                </div>
            </article>
        </div>
        <article class="timeline">
            <div class="section-title">
                <h2>CPU history</h2>
                <span class="muted">0–100% · up to 10 min</span>
            </div>
            <ActivityGraph {snapshot} />
            <details class="resource-details">
                <summary
                    >{snapshot.logical_cpus} cores · {memory(snapshot.swap_used_bytes)} swap
                    <span class="detail-label">Details</span></summary
                >
                <p class="muted">
                    Memory pressure: {snapshot.platform.memory_pressure ??
                        'estimated from available RAM'}
                </p>
                <div class="core-label">
                    <span>Logical cores</span><span>Each bar: 0–100% of one core</span>
                </div>
                <div class="core-bars" role="img" aria-label="Individual logical core activity">
                    {#each snapshot.cpu_cores as cpu, index}<div
                            title={`Core ${index + 1}: ${percent(cpu)}`}
                        >
                            <i style={`height:${snapshot.sample_ready ? Math.max(cpu, 1) : 0}%`}
                            ></i>
                        </div>{/each}
                </div>
                {#if breakdown}
                    <dl class="memory-breakdown">
                        <div>
                            <dt>App memory</dt>
                            <dd>{memory(breakdown.app_bytes)}</dd>
                        </div>
                        <div>
                            <dt>Wired</dt>
                            <dd>{memory(breakdown.wired_bytes)}</dd>
                        </div>
                        <div>
                            <dt>Compressed</dt>
                            <dd>{memory(breakdown.compressed_bytes)}</dd>
                        </div>
                        <div>
                            <dt>Cached files</dt>
                            <dd>{memory(breakdown.cached_bytes)}</dd>
                        </div>
                        <div>
                            <dt>Swap in · last {snapshot.interval_ms / 1000}s</dt>
                            <dd>
                                {snapshot.swap_in_bytes === null
                                    ? 'Sampling…'
                                    : memory(snapshot.swap_in_bytes)}
                            </dd>
                        </div>
                        <div>
                            <dt>Swap out · last {snapshot.interval_ms / 1000}s</dt>
                            <dd>
                                {snapshot.swap_out_bytes === null
                                    ? 'Sampling…'
                                    : memory(snapshot.swap_out_bytes)}
                            </dd>
                        </div>
                    </dl>
                    <p class="muted">
                        {snapshot.source === 'demo'
                            ? 'Synthetic example.'
                            : 'Native macOS VM counters.'} App + wired + compressed = used memory. Cached
                        files are reclaimable. Swap activity alone does not identify a cause.
                    </p>
                {:else}<p class="muted">
                        Native memory breakdown and swap activity are unavailable on this platform.
                        Available memory and swap usage above come from the shared system collector.
                    </p>{/if}
            </details>
        </article>
    </section>
    <ValidationQueue {snapshot} {onselect} {onmanage} />
    <div class="activity-columns">
        <AgentWorkflows {snapshot} {onselect} {onusage} {onmanage} />
        <section class="panel workload-panel">
            <div class="section-title">
                <div>
                    <h2>Workloads</h2>
                    <span class="muted">{snapshot.process_count} processes · up to 40 groups</span>
                </div>
                <div class="filters">
                    <input
                        aria-label="Filter workloads"
                        placeholder="Search"
                        bind:value={search}
                    /><select aria-label="Sort workloads" bind:value={sort}
                        ><option value="impact">Impact</option><option value="cpu">CPU</option
                        ><option value="memory">Memory</option></select
                    >
                </div>
            </div>
            <div class="workload-header">
                <span>Application / workload</span><span>CPU</span><span>Memory</span>
            </div>
            {#each rows as workload (workload.id)}
                <button
                    class:chosen={selectedId === workload.id}
                    class="workload-row"
                    onclick={() => onselect(workload)}
                    title={`${workload.description} · Click to investigate`}
                >
                    <div class="workload-main">
                        <AppIcon
                            workloadId={workload.id}
                            category={workload.category}
                            demo={snapshot.source === 'demo'}
                        />
                        <div>
                            <div class="workload-name">
                                <strong>{workload.label}</strong><span class="row-count"
                                    >{workload.process_count}
                                    {workload.process_count === 1 ? 'process' : 'processes'}</span
                                >
                            </div>
                        </div>
                    </div>
                    <span class="numeric usage-cell"
                        ><span>{snapshot.sample_ready ? percent(workload.cpu_percent) : '—'}</span
                        ><span class="usage-bar" aria-hidden="true"
                            ><i
                                style={`width:${snapshot.sample_ready ? Math.min(workload.cpu_percent, 100) : 0}%`}
                            ></i></span
                        ></span
                    ><span class="numeric">{memory(workload.memory_bytes)}</span>
                </button>
            {:else}<p class="empty">No matching workloads in the current sample.</p>{/each}
            <p class="footnote">
                CPU: share of all cores · Memory: summed RSS, including shared pages. Select a row
                to investigate.
            </p>
        </section>
    </div>
</div>

<style>
    .dashboard {
        container-type: inline-size;
        container-name: dashboard;
    }
    .activity-columns {
        display: grid;
        grid-template-columns: minmax(0, 1fr);
        gap: 16px;
        align-items: start;
    }
    .workload-panel .section-title {
        gap: 10px;
    }
    .filters {
        flex: 1;
        justify-content: flex-end;
    }
    .filters input {
        width: 120px;
    }
    .workload-name {
        display: block;
    }
    .workload-name strong {
        display: block;
    }
    .workload-header,
    .workload-row {
        grid-template-columns: minmax(0, 1fr) 54px 72px;
        column-gap: 14px;
    }
    .workload-name strong {
        font-size: 12px;
    }
    .row-count {
        display: block;
        margin: 3px 0 0;
    }
    @container dashboard (min-width: 860px) {
        .activity-columns {
            grid-template-columns: minmax(0, 1.08fr) minmax(0, 1fr);
        }
    }
    @container dashboard (max-width: 620px) {
        .metric :global(.resource-ring),
        .pressure-symbol {
            display: none;
        }
        .metric {
            padding: 0 12px;
        }
        .metric strong {
            font-size: 19px;
        }
    }
    .system-panel {
        margin-bottom: 16px;
        overflow: hidden;
    }
    .metrics {
        gap: 0;
        margin: 0;
        padding: 16px 0;
    }
    .metric {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 0 18px;
        min-height: 68px;
        border: none;
        border-radius: 0;
        background: transparent;
    }
    .metric + .metric {
        border-left: 1px solid var(--border);
    }
    .metric-label {
        color: var(--muted);
        margin-bottom: 4px;
        font-size: 11px;
    }
    .metric strong {
        font-size: 23px;
        margin: 0 0 4px;
        font-variant-numeric: tabular-nums;
    }
    .metric small {
        color: var(--muted);
        line-height: 1.4;
    }
    .pressure-symbol {
        width: 48px;
        height: 48px;
        color: var(--success);
        background: var(--success-soft);
        border-radius: 50%;
        flex-shrink: 0;
        padding: 7px;
    }
    .pressure-symbol.elevated {
        color: var(--warning);
        background: var(--warning-soft);
    }
    .pressure-symbol svg {
        width: 100%;
        height: 100%;
        fill: none;
        stroke: currentColor;
        stroke-width: 2;
        stroke-linejoin: round;
        stroke-linecap: round;
    }
    .timeline {
        border-top: 1px solid var(--border);
        border-radius: 0;
        padding: 10px 16px 0;
        margin: 0;
    }
    .timeline .section-title {
        margin-bottom: 5px;
    }
    .timeline h2 {
        font-size: 10px;
        color: var(--muted);
        font-weight: 500;
        margin: 0;
    }
    .timeline .section-title > span {
        font-size: 10px;
    }
    .resource-details > summary {
        font-size: 10px;
        padding: 9px 0;
    }
    .detail-label {
        float: right;
    }
    .usage-cell {
        display: grid;
        gap: 5px;
    }
    .usage-bar {
        height: 3px;
        border-radius: 2px;
        background: var(--track);
        overflow: hidden;
    }
    .usage-bar i {
        display: block;
        height: 100%;
        background: var(--accent);
        opacity: 0.7;
    }
    .workload-row {
        padding-top: 10px;
        padding-bottom: 10px;
    }
    @media (max-width: 900px) {
        .metric {
            gap: 8px;
            padding: 0 12px;
        }
        .metric strong {
            font-size: 19px;
        }
        .pressure-symbol {
            display: none;
        }
    }
</style>
