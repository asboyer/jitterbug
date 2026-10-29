<script lang="ts">
    import { memory, percent } from '../lib/format';
    import type { Snapshot, Workload } from '../lib/types';
    import ActivityGraph from './ActivityGraph.svelte';
    import ResourceRing from './ResourceRing.svelte';
    import AppIcon from './AppIcon.svelte';
    import Icon from './Icon.svelte';

    let {
        snapshot,
        onselect,
        onask,
        onworkloads,
    }: {
        snapshot: Snapshot;
        onselect: (workload: Workload) => void;
        onask: () => void;
        onworkloads: () => void;
    } = $props();
    const memoryTight = $derived(
        snapshot.memory_available_bytes / snapshot.memory_total_bytes < 0.15
    );
    const busy = $derived(snapshot.pressure.level !== 'normal');
    const leaders = $derived(
        [...snapshot.workloads]
            .sort((a, b) =>
                memoryTight
                    ? b.memory_bytes - a.memory_bytes
                    : b.cpu_percent - a.cpu_percent || b.memory_bytes - a.memory_bytes
            )
            .slice(0, 5)
    );
    const categories: Record<string, string> = {
        app: 'Application',
        browser: 'Browser processes',
        project: 'Development work',
        agent: 'Coding tool',
        editor: 'Editor or coding tool',
        build: 'Build tooling',
        python: 'Python process',
        javascript: 'JavaScript process',
        jitterbug: 'Resource assistant',
        test: 'Test process',
        model: 'Local model',
        system: 'System work',
        other: 'App or system process',
    };
    const usedPercent = $derived(
        100 * (1 - snapshot.memory_available_bytes / snapshot.memory_total_bytes)
    );
    const breakdown = $derived(snapshot.platform.memory);
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
    <section class="health-summary" class:busy aria-label="Computer health" data-tour="health">
        <span class="health-symbol"><Icon name="activity" size={24} /></span>
        <div>
            <h2>
                {!snapshot.sample_ready
                    ? 'Getting a reading…'
                    : busy
                      ? 'Your computer is working hard'
                      : 'Your computer has room to work'}
            </h2>
            <p>
                {!snapshot.sample_ready
                    ? 'CPU needs a second sample before we can assess activity.'
                    : memoryTight
                      ? 'Memory is tight. Start with the largest apps below.'
                      : busy
                        ? 'Review the busiest workloads or ask Jitterbug to investigate.'
                        : 'No system pressure detected. Here’s where resources are going.'}
            </p>
        </div>
    </section>
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
    <section class="panel contributors" aria-label="Top resource users">
        <div class="section-title">
            <div>
                <h2>{memoryTight ? 'Using the most memory' : 'Using the most CPU'}</h2>
                <p class="muted">Select an app to see what it’s doing and what you can change.</p>
            </div>
            <button class="text-button" onclick={onworkloads}>All workloads ↗</button>
        </div>
        {#each leaders as workload (workload.id)}
            <button class="contributor" onclick={() => onselect(workload)}>
                <AppIcon
                    workloadId={workload.id}
                    category={workload.category}
                    demo={snapshot.source === 'demo'}
                />
                <span class="contributor-copy"
                    ><strong>{workload.label}</strong><small
                        >{categories[workload.category] ?? 'Workload'} · {memoryTight
                            ? `${workload.process_count} processes`
                            : `${memory(workload.memory_bytes)} memory`}</small
                    ></span
                >
                <span class="contributor-value"
                    >{memoryTight
                        ? memory(workload.memory_bytes)
                        : snapshot.sample_ready
                          ? percent(workload.cpu_percent)
                          : '—'}<small>{memoryTight ? 'memory' : 'CPU'}</small></span
                >
                <span class="muted" aria-hidden="true">›</span>
            </button>
        {:else}<p class="empty">Waiting for workload samples.</p>{/each}
        <div class="activity-footer">
            <Icon name="bulb" size={16} /><span>Something feels slow?</span><button
                class="text-button"
                onclick={onask}>Help me find out why ↗</button
            >
        </div>
    </section>
</div>

<style>
    .dashboard {
        container-type: inline-size;
        container-name: dashboard;
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
    .health-summary {
        display: flex;
        align-items: center;
        gap: 14px;
        padding: 0 0 22px;
    }
    .health-symbol {
        display: flex;
        color: var(--success);
        background: var(--success-soft);
        padding: 12px;
        border-radius: 14px;
    }
    .busy .health-symbol {
        color: var(--warning);
        background: var(--warning-soft);
    }
    .health-summary h2 {
        font-size: 17px;
        margin: 0 0 5px;
    }
    .health-summary p {
        color: var(--muted);
        margin: 0;
        line-height: 1.5;
    }
    .contributors {
        overflow: hidden;
    }
    .contributors .section-title {
        padding: 16px 18px;
        margin: 0;
        flex-wrap: wrap;
        gap: 8px;
    }
    .contributors .section-title p {
        margin: 5px 0 0;
        font-size: 11px;
    }
    .contributor {
        display: flex;
        align-items: center;
        gap: 14px;
        text-align: left;
        width: 100%;
        border: 0;
        border-top: 1px solid var(--border);
        border-radius: 0;
        padding: 14px 18px;
        background: transparent;
    }
    .contributor-copy {
        flex: 1;
        min-width: 0;
        display: grid;
        gap: 5px;
    }
    .contributor-copy small {
        color: var(--muted);
        overflow: hidden;
        white-space: nowrap;
        text-overflow: ellipsis;
        font-size: 11px;
    }
    .contributor-value {
        display: grid;
        gap: 4px;
        text-align: right;
        font-variant-numeric: tabular-nums;
    }
    .contributor-value small {
        color: var(--muted);
        font-size: 10px;
    }
    .activity-footer {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 8px;
        padding: 10px 18px;
        border-top: 1px solid var(--border);
        color: var(--muted);
        font-size: 11px;
    }
</style>
