<script lang="ts">
    import { memory, percent } from '../lib/format';
    import type { Snapshot, Workload } from '../lib/types';
    import ActivityGraph from './ActivityGraph.svelte';
    import Icon from './Icon.svelte';

    let {
        snapshot,
        selectedId,
        onselect,
        onask,
    }: {
        snapshot: Snapshot;
        selectedId: string | null;
        onselect: (workload: Workload) => void;
        onask: () => void;
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

<div class="page-heading">
    <div>
        <h1>Activity</h1>
        <p class="muted">Select a workload to investigate its resource use.</p>
    </div>
    <button class="primary" onclick={onask}>Ask Jitterbug <span aria-hidden="true">↗</span></button>
</div>
<div class="metrics">
    <article class="metric">
        <div class="metric-label">CPU <span>Total capacity</span></div>
        <strong>{snapshot.sample_ready ? percent(snapshot.cpu_percent) : 'Sampling…'}</strong>
        <div class="meter"><i style={`width:${snapshot.cpu_percent}%`}></i></div>
        <small>{snapshot.logical_cpus} logical cores</small>
    </article>
    <article class="metric">
        <div class="metric-label">Memory <span>Available</span></div>
        <strong>{memory(snapshot.memory_available_bytes)}</strong>
        <div class="meter blue"><i style={`width:${100 - usedPercent}%`}></i></div>
        <small
            >of {memory(snapshot.memory_total_bytes)} · {memory(snapshot.swap_used_bytes)} swap</small
        >
    </article>
    <article class="metric">
        <div class="metric-label">
            Pressure <span class:warning={snapshot.pressure.level !== 'normal'} class="status-dot"
            ></span>
        </div>
        <strong
            >{snapshot.pressure.level === 'normal'
                ? 'Normal'
                : snapshot.pressure.level === 'watch'
                  ? 'Elevated'
                  : 'High'}</strong
        >
        <small
            >Thermal: {snapshot.platform.thermal ?? 'unavailable'}<br />Memory: {snapshot.platform
                .memory_pressure ?? 'estimated from available RAM'}</small
        >
    </article>
</div>
<article class="timeline panel">
    <div class="section-title">
        <h2>CPU history</h2>
        <span class="muted">Total capacity · up to 10 minutes</span>
    </div>
    <ActivityGraph {snapshot} />
    <details class="resource-details">
        <summary>Core activity & memory details</summary>
        <div class="core-label">
            <span>Logical cores</span><span>Each bar: 0–100% of one core</span>
        </div>
        <div class="core-bars" role="img" aria-label="Individual logical core activity">
            {#each snapshot.cpu_cores as cpu, index}<div
                    title={`Core ${index + 1}: ${percent(cpu)}`}
                >
                    <i style={`height:${snapshot.sample_ready ? Math.max(cpu, 1) : 0}%`}></i>
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
                {snapshot.source === 'demo' ? 'Synthetic example.' : 'Native macOS VM counters.'} App
                + wired + compressed = used memory. Cached files are reclaimable. Swap activity alone
                does not identify a cause.
            </p>
        {:else}<p class="muted">
                Native memory breakdown and swap activity are unavailable on this platform.
                Available memory and swap usage above come from the shared system collector.
            </p>{/if}
    </details>
</article>
<section class="panel workload-panel">
    <div class="section-title">
        <div>
            <h2>Workloads</h2>
            <span class="muted">{snapshot.process_count} processes · up to 40 groups</span>
        </div>
        <div class="filters">
            <input
                aria-label="Filter workloads"
                placeholder="Search workloads"
                bind:value={search}
            /><select aria-label="Sort workloads" bind:value={sort}
                ><option value="impact">Impact</option><option value="cpu">CPU</option><option
                    value="memory">Memory</option
                ></select
            >
        </div>
    </div>
    <div class="workload-header">
        <span>Name / inferred purpose</span><span>CPU</span><span>Memory</span>
    </div>
    {#each rows as workload (workload.id)}
        <button
            class:chosen={selectedId === workload.id}
            class="workload-row"
            onclick={() => onselect(workload)}
        >
            <div class="workload-main">
                <span class="workload-icon"
                    ><Icon
                        name={workload.category === 'project'
                            ? 'code'
                            : workload.category === 'model'
                              ? 'model'
                              : 'app'}
                        size={18}
                    /></span
                >
                <div>
                    <div class="workload-name">
                        <strong>{workload.label}</strong><span class="row-count"
                            >{workload.process_count}
                            {workload.process_count === 1 ? 'process' : 'processes'}</span
                        >
                    </div>
                    <p title={`${workload.description} (${workload.attribution})`}>
                        {workload.description}
                    </p>
                </div>
            </div>
            <span class="numeric">{percent(workload.cpu_percent)}</span><span class="numeric"
                >{memory(workload.memory_bytes)}</span
            >
        </button>
    {:else}<p class="empty">No matching workloads in the current sample.</p>{/each}
    <p class="footnote">
        Purpose is inferred by rules. CPU is a share of total capacity. Memory is summed process
        RSS; shared pages may be counted more than once.
    </p>
</section>
