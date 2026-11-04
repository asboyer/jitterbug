<script lang="ts">
    import type { Snapshot } from '../lib/types';
    import { memory, percent } from '../lib/format';
    let { snapshot }: { snapshot: Snapshot } = $props();
    let expanded = $state(false);
    const used = $derived(snapshot.memory_total_bytes - snapshot.memory_available_bytes);
    const pressure = $derived(snapshot.pressure.level);
    const points = $derived(
        snapshot.history
            .map(
                (p, i, a) =>
                    `${(i / Math.max(a.length - 1, 1)) * 110},${28 - Math.min(p.cpu_percent, 100) * 0.26}`
            )
            .join(' ')
    );
</script>

<section class="monitor-stats" aria-label="System resources" data-tour="health">
    <div class="stat">
        <div>
            <span>CPU</span><strong
                >{snapshot.sample_ready ? percent(snapshot.cpu_percent) : '—'}</strong
            >
        </div>
        <svg viewBox="0 0 110 30" aria-label="CPU history" role="img"><polyline {points} /></svg>
    </div>
    <div class="stat memory">
        <div>
            <span>Memory</span><strong
                >{memory(used)}<small> / {memory(snapshot.memory_total_bytes)}</small></strong
            >
        </div>
        <div class="memory-track">
            <i style:width={`${(100 * used) / Math.max(snapshot.memory_total_bytes, 1)}%`}></i>
        </div>
    </div>
    <div class="stat health">
        <div>
            <span>System pressure</span><strong class:warning={pressure !== 'normal'}
                ><i></i>{!snapshot.sample_ready
                    ? 'Sampling'
                    : pressure === 'normal'
                      ? 'Normal'
                      : pressure === 'watch'
                        ? 'Elevated'
                        : 'High'}</strong
            >
        </div>
        <button
            class="text-button"
            aria-label="System details"
            aria-expanded={expanded}
            onclick={() => (expanded = !expanded)}>···</button
        >
    </div>
    {#if expanded}<div class="stats-detail">
            <span>{snapshot.logical_cpus} CPU cores</span><span
                >{memory(snapshot.memory_available_bytes)} available</span
            ><span>{memory(snapshot.swap_used_bytes)} swap</span><span
                >Thermal: {snapshot.platform.thermal ?? 'unavailable'}</span
            ><span>CPU is measured across all cores.</span>
        </div>{/if}
</section>

<style>
    .monitor-stats {
        display: grid;
        grid-template-columns: 1fr 1.15fr 1fr;
        border: 1px solid var(--border);
        border-radius: 12px;
        background: var(--panel);
        flex-shrink: 0;
    }
    .stat {
        min-width: 0;
        display: flex;
        gap: 16px;
        align-items: center;
        justify-content: space-between;
        padding: 17px 20px;
    }
    .stat + .stat {
        border-left: 1px solid var(--border);
    }
    .stat span {
        display: block;
        color: var(--muted);
        font-size: 11px;
        margin-bottom: 5px;
    }
    strong {
        display: block;
        font-size: 23px;
        line-height: 1.2;
        font-weight: 600;
        font-variant-numeric: tabular-nums;
        letter-spacing: -0.6px;
    }
    small {
        font-size: 12px;
        color: var(--muted);
        font-weight: 400;
        letter-spacing: 0;
    }
    svg {
        width: 100px;
        height: 30px;
        overflow: visible;
    }
    polyline {
        fill: none;
        stroke: var(--accent);
        stroke-width: 1.8;
    }
    .memory {
        display: block;
    }
    .memory-track {
        height: 3px;
        background: var(--track);
        margin-top: 10px;
        border-radius: 4px;
        overflow: hidden;
    }
    .memory-track i {
        display: block;
        height: 100%;
        background: var(--accent);
        opacity: 0.7;
    }
    .health strong {
        font-size: 19px;
        display: flex;
        gap: 8px;
        align-items: center;
    }
    .health i {
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: #39ad79;
    }
    .health strong.warning i {
        background: #e4a239;
    }
    .health button {
        font-size: 20px;
        padding: 0 5px;
    }
    .stats-detail {
        grid-column: 1 / -1;
        display: flex;
        gap: 12px 24px;
        flex-wrap: wrap;
        padding: 12px 20px;
        border-top: 1px solid var(--border);
        color: var(--muted);
        font-size: 11px;
    }
    @media (max-width: 1050px) {
        svg {
            display: none;
        }
        .stat {
            padding: 14px;
        }
        strong {
            font-size: 20px;
        }
    }
</style>
