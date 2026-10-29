<script lang="ts">
    import { memory, percent } from '../lib/format';
    import type { Snapshot, Workload } from '../lib/types';

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
    const rows = $derived.by(() => {
        const filtered = snapshot.workloads.filter((w) =>
            `${w.label} ${w.category}`.toLowerCase().includes(search.toLowerCase())
        );
        if (sort === 'cpu') return filtered.sort((a, b) => b.cpu_percent - a.cpu_percent);
        if (sort === 'memory') return filtered.sort((a, b) => b.memory_bytes - a.memory_bytes);
        return filtered;
    });
    const points = $derived(
        snapshot.history
            .map(
                (p, i) =>
                    `${(i / Math.max(snapshot.history.length - 1, 1)) * 600},${80 - p.cpu_percent * 0.7}`
            )
            .join(' ')
    );
</script>

<div class="page-heading">
    <div>
        <p class="eyebrow">YOUR COMPUTER, EXPLAINED</p>
        <h1>Workloads</h1>
        <p class="muted">A little context for everything running.</p>
    </div>
    <button class="primary" onclick={onask}
        >Why is my computer struggling? <span aria-hidden="true">↗</span></button
    >
</div>

<div class="metrics">
    <article class="metric">
        <div class="metric-label">CPU <span>Whole machine</span></div>
        <strong>{snapshot.sample_ready ? percent(snapshot.cpu_percent) : 'Warming up'}</strong>
        <div class="meter"><i style={`width:${snapshot.cpu_percent}%`}></i></div>
        <small>{snapshot.logical_cpus} logical cores · all CPU values use total capacity</small>
    </article>
    <article class="metric">
        <div class="metric-label">Memory <span>Available</span></div>
        <strong>{memory(snapshot.memory_available_bytes)}</strong>
        <div class="meter blue"><i style={`width:${100 - usedPercent}%`}></i></div>
        <small
            >{memory(snapshot.memory_total_bytes)} total · {memory(snapshot.swap_used_bytes)} swap</small
        >
    </article>
    <article class="metric">
        <div class="metric-label">
            Resource pressure <span
                class:warning={snapshot.pressure.level !== 'normal'}
                class="status-dot"
            ></span>
        </div>
        <strong class="capitalize"
            >{snapshot.pressure.level === 'normal'
                ? 'Looking good'
                : snapshot.pressure.level === 'watch'
                  ? 'Watching a spike'
                  : 'Needs attention'}</strong
        ><small
            >{snapshot.platform.thermal
                ? `Thermal state: ${snapshot.platform.thermal}`
                : 'Thermal sensor unavailable'}<br />{snapshot.platform.memory_pressure
                ? `OS memory pressure: ${snapshot.platform.memory_pressure}`
                : 'Using available-memory heuristic'}</small
        >
    </article>
</div>

<article class="timeline panel">
    <div class="section-title">
        <h2>Recent activity</h2>
        <span class="muted">CPU · up to 10 minutes</span>
    </div>
    <svg
        viewBox="0 0 600 90"
        preserveAspectRatio="none"
        role="img"
        aria-label="Recent total CPU use"
    >
        <path d="M0 15H600 M0 45H600 M0 75H600" stroke="var(--border)" stroke-dasharray="3 5" />
        {#if points}<polyline
                {points}
                fill="none"
                stroke="var(--accent)"
                stroke-width="2"
                vector-effect="non-scaling-stroke"
            />{/if}
    </svg>
</article>

<section class="panel workload-panel">
    <div class="section-title">
        <div>
            <h2>What’s running</h2>
            <span class="muted"
                >{snapshot.process_count} processes · showing up to 40 workload groups</span
            >
        </div>
        <div class="filters">
            <input
                aria-label="Filter workloads"
                placeholder="Find a workload…"
                bind:value={search}
            /><select aria-label="Sort workloads" bind:value={sort}
                ><option value="impact">Overall impact</option><option value="cpu">CPU use</option
                ><option value="memory">Memory use</option></select
            >
        </div>
    </div>
    <div class="workload-header">
        <span>Workload / inferred purpose</span><span>CPU</span><span>Memory</span>
    </div>
    {#each rows as workload (workload.id)}
        <button
            class:chosen={selectedId === workload.id}
            class="workload-row"
            onclick={() => onselect(workload)}
        >
            <div class="workload-main">
                <span class="workload-icon" aria-hidden="true"
                    >{workload.category === 'project'
                        ? '⌘'
                        : workload.category === 'model'
                          ? '◈'
                          : '◌'}</span
                >
                <div>
                    <strong>{workload.label}</strong>
                    <p>{workload.description}</p>
                    <span class="tag">{workload.attribution}</span><span class="row-count"
                        >{workload.process_count}
                        {workload.process_count === 1 ? 'process' : 'processes'}</span
                    >
                </div>
            </div>
            <span class="numeric">{percent(workload.cpu_percent)}</span><span class="numeric"
                >{memory(workload.memory_bytes)}</span
            >
        </button>
    {:else}<p class="empty">No matching workloads in the current sample.</p>{/each}
    <p class="footnote">
        Descriptions are rule-based inferences. Process memory is summed RSS and may count shared
        pages more than once. Hidden or protected process metadata may be unavailable.
    </p>
</section>
