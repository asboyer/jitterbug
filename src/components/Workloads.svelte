<script lang="ts">
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import { untrack } from 'svelte';
    import { memory, percent, errorMessage } from '../lib/format';
    import type { Snapshot, Workload } from '../lib/types';
    import AppIcon from './AppIcon.svelte';
    let { snapshot, onselect }: { snapshot: Snapshot; onselect: (workload: Workload) => void } =
        $props();
    let search = $state('');
    let sort = $state('impact');
    let results: Workload[] | null = $state(null);
    let searchError = $state('');
    let searching = $state(false);
    let previousSearch = '';
    $effect(() => {
        const query = search.trim();
        const sample = snapshot;
        let cancelled = false;
        untrack(() => {
            if (query !== previousSearch || sample.source !== 'live') results = null;
            previousSearch = query;
            searchError = '';
            searching = false;
        });
        if (!query || !isTauri() || sample.source !== 'live') return;
        searching = true;
        const timer = setTimeout(async () => {
            try {
                const matches = await invoke<Workload[]>('search_workloads', { query });
                if (!cancelled) results = matches;
            } catch (cause) {
                if (!cancelled) searchError = errorMessage(cause);
            } finally {
                if (!cancelled) searching = false;
            }
        }, 180);
        return () => {
            cancelled = true;
            clearTimeout(timer);
        };
    });
    const rows = $derived.by(() => {
        const filtered =
            results ??
            snapshot.workloads.filter((w) =>
                `${w.label} ${w.category}`.toLowerCase().includes(search.toLowerCase())
            );
        if (sort === 'cpu') return [...filtered].sort((a, b) => b.cpu_percent - a.cpu_percent);
        if (sort === 'memory') return [...filtered].sort((a, b) => b.memory_bytes - a.memory_bytes);
        return filtered;
    });
</script>

<div class="page-heading">
    <div>
        <h1>Workloads</h1>
        <p class="muted">Apps and background work. Select one to understand its activity.</p>
    </div>
</div>
<section class="panel workload-panel" data-tour="workloads">
    <div class="section-title">
        <div>
            <h2>All activity</h2>
            <span class="muted"
                >{search.trim()
                    ? 'Search all observed processes'
                    : `${snapshot.process_count} processes · busiest workloads`}</span
            >
        </div>
        <div class="filters">
            <input
                aria-label="Filter workloads"
                placeholder="Name or PID"
                maxlength="128"
                bind:value={search}
            /><select aria-label="Sort workloads" bind:value={sort}
                ><option value="impact">Impact</option><option value="cpu">CPU</option><option
                    value="memory">Memory</option
                ></select
            >
        </div>
    </div>
    {#if searchError}<p class="error" role="alert">{searchError}</p>{/if}
    <div class="workload-header">
        <span>Application / workload</span><span>CPU</span><span>Memory</span>
    </div>
    {#each rows as workload (workload.id)}
        <button class="workload-row" onclick={() => onselect(workload)}>
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
                ><span>{snapshot.sample_ready ? percent(workload.cpu_percent) : '—'}</span><span
                    class="usage-bar"
                    aria-hidden="true"
                    ><i
                        style={`width:${snapshot.sample_ready ? Math.min(workload.cpu_percent, 100) : 0}%`}
                    ></i></span
                ></span
            ><span class="numeric">{memory(workload.memory_bytes)}</span>
        </button>
    {:else}<p class="empty">
            {searching ? 'Searching processes…' : 'No matching workloads in the current sample.'}
        </p>{/each}
    <p class="footnote">
        CPU: share of all cores · Memory: summed RSS, including shared pages. Select a row to
        investigate.
    </p>
</section>

<style>
    .section-title {
        gap: 12px;
        flex-wrap: wrap;
    }
    .filters {
        gap: 8px;
    }
    .filters input {
        width: 160px;
    }
    .workload-name {
        display: grid;
        gap: 4px;
    }
    .row-count {
        color: var(--muted);
        font-size: 11px;
    }
    .workload-row {
        padding-top: 14px;
        padding-bottom: 14px;
    }
    .usage-cell {
        display: grid;
        gap: 6px;
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
    }
</style>
