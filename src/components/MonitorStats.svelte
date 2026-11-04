<script lang="ts">
    import type { Snapshot } from '../lib/types';
    import { memory, percent } from '../lib/format';
    let { snapshot }: { snapshot: Snapshot } = $props();
    let expanded = $state(false);
    const used = $derived(snapshot.memory_total_bytes - snapshot.memory_available_bytes);
    const pressure = $derived(snapshot.pressure.level);
</script>

<section class="monitor-stats" aria-label="System resources">
    <div class="stat">
        <span>CPU</span><strong
            >{snapshot.sample_ready ? percent(snapshot.cpu_percent) : '—'}</strong
        ><small>of total capacity</small>
    </div>
    <div class="stat">
        <span>Memory</span><strong
            >{memory(used)}<small> / {memory(snapshot.memory_total_bytes)}</small></strong
        ><small>{memory(snapshot.memory_available_bytes)} available</small>
    </div>
    <div class="stat health">
        <span>System pressure</span><strong class:strained={pressure !== 'normal'}
            ><i></i>{!snapshot.sample_ready
                ? 'Sampling'
                : pressure === 'normal'
                  ? 'Normal'
                  : pressure === 'watch'
                    ? 'Elevated'
                    : 'High'}</strong
        ><small
            >{snapshot.platform.thermal === 'nominal'
                ? 'Thermal state normal'
                : `Thermal: ${snapshot.platform.thermal ?? 'unavailable'}`}</small
        >
    </div>
    <button
        class="text-button details-toggle"
        aria-label="System details"
        aria-expanded={expanded}
        onclick={() => (expanded = !expanded)}>{expanded ? 'Less' : 'Details'}</button
    >
    {#if expanded}<div class="stats-detail">
            <span>{snapshot.logical_cpus} CPU cores</span><span
                >{memory(snapshot.swap_used_bytes)} swap</span
            ><span
                >Sampled {new Date(snapshot.captured_at * 1000).toLocaleTimeString([], {
                    hour: 'numeric',
                    minute: '2-digit',
                    second: '2-digit',
                })}</span
            ><span>CPU is measured across all cores.</span>
        </div>{/if}
</section>

<style>
    .monitor-stats {
        display: grid;
        grid-template-columns: 1fr 1.5fr 1fr auto;
        column-gap: 24px;
        padding: 12px 0;

        flex: 0 0 auto;
    }
    .stat {
        display: flex;
        flex-direction: column;
        gap: 5px;
        min-width: 0;
    }
    .stat > span {
        font-size: 12px;
        color: var(--muted);
    }
    strong {
        display: flex;
        gap: 8px;
        align-items: baseline;
        font-size: 23px;
        font-weight: 500;
        line-height: 1.3;
        font-variant-numeric: tabular-nums;
        letter-spacing: -0.6px;
    }
    small {
        font-size: 11px;
        color: var(--muted);
        font-weight: 400;
        letter-spacing: 0;
    }
    .health strong {
        align-items: center;
    }
    .health i {
        height: 5px;
        width: 5px;
        border-radius: 50%;
        background: var(--success);
    }
    .health .strained {
        color: var(--warning);
    }
    .health .strained i {
        background: var(--warning);
    }
    .details-toggle {
        align-self: center;
        padding: 4px 0;
        font-size: 12px;
    }
    .stats-detail {
        display: flex;
        flex-wrap: wrap;
        gap: 12px 24px;
        grid-column: 1/-1;
        color: var(--muted);
        font-size: 12px;
        margin-top: 20px;
    }
    @media (max-width: 700px) {
        .monitor-stats {
            grid-template-columns: 1fr 1.4fr;
            row-gap: 18px;
        }
        .health strong {
            font-size: 18px;
        }
        strong {
            font-size: 20px;
        }
        .details-toggle {
            justify-self: end;
        }
    }
</style>
