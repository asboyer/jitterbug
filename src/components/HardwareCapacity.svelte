<script lang="ts">
    import Icon from './Icon.svelte';
    import InfoTip from './InfoTip.svelte';
    import { memory, percent } from '../lib/format';
    import type { ValidationQueue } from '../lib/queue';
    import type { Snapshot } from '../lib/types';

    let { queue, snapshot }: { queue: ValidationQueue | null; snapshot: Snapshot } = $props();
    const resources = $derived(queue?.resources);
    const scheduling = $derived(queue?.scheduling);
    const cpu = $derived(resources?.cpu_percent ?? snapshot.cpu_percent);
    const total = $derived(resources?.total_bytes ?? snapshot.memory_total_bytes);
    const available = $derived(resources?.available_bytes ?? snapshot.memory_available_bytes);
    const usedMemory = $derived(total > 0 ? ((total - available) / total) * 100 : 0);
    const clamp = (value: number) => Math.min(100, Math.max(0, value));
</script>

<section class="hardware-capacity" aria-label="Hardware capacity">
    <div class="capacity-status">
        <span class="capacity-icon" class:sharing={scheduling?.state === 'parallel'}>
            <Icon name="cpu" size={19} />
        </span>
        <div class="status-copy">
            <strong>{scheduling?.title ?? 'Watching your hardware'}</strong>
            <span>
                {#if scheduling}
                    {scheduling.running} running <span aria-hidden="true">·</span>
                    {scheduling.waiting} waiting
                {:else}Reading capacity…{/if}
            </span>
        </div>
        <InfoTip label="How capacity sharing works">
            {scheduling?.detail ?? 'Waiting for fresh scheduling information.'}
            Familiar checks can overlap after two successful runs with enough samples. Estimates include
            headroom; they are not resource caps. Checks in the same folder take turns.
            {#if scheduling}
                {scheduling.learned_profiles} profiles learned. Admission leaves
                {memory(scheduling.memory_reserve_bytes)} for your apps and uses a
                {scheduling.cpu_ceiling}% CPU budget for overlap. Running checks finish normally.
            {/if}
        </InfoTip>
    </div>
    <div class="capacity-readings">
        {#each [{ label: 'CPU', value: percent(cpu), used: cpu, reserve: 100 - (scheduling?.cpu_ceiling ?? 55) }, { label: 'Memory', value: `${memory(available)} free`, used: usedMemory, reserve: total > 0 ? ((scheduling?.memory_reserve_bytes ?? 0) / total) * 100 : 0 }] as reading}
            <div class="capacity-reading">
                <div class="reading-label">
                    <span>{reading.label}</span><strong>{reading.value}</strong>
                </div>
                <div
                    class="capacity-track"
                    role="meter"
                    aria-label={reading.label === 'CPU' ? 'Total CPU usage' : 'Total memory usage'}
                    aria-valuemin="0"
                    aria-valuemax="100"
                    aria-valuenow={clamp(reading.used)}
                >
                    <span class="reserve" style:width={`${clamp(reading.reserve)}%`}></span>
                    <span class="used" style:width={`${clamp(reading.used)}%`}></span>
                </div>
            </div>
        {/each}
    </div>
</section>

<style>
    .hardware-capacity {
        border: 1px solid var(--border);
        background: var(--panel);
        border-radius: 12px;
        padding: 18px 20px;
        display: grid;
        grid-template-columns: minmax(200px, 1.2fr) minmax(220px, 1fr);
        gap: 24px;
    }
    .capacity-status {
        display: flex;
        align-items: center;
        gap: 12px;
        min-width: 0;
    }
    .capacity-icon {
        display: grid;
        place-items: center;
        width: 38px;
        height: 38px;
        border-radius: 11px;
        background: var(--accent-soft);
        color: var(--accent);
        flex-shrink: 0;
    }
    .capacity-icon.sharing {
        color: var(--success);
    }
    .status-copy {
        display: grid;
        gap: 5px;
    }
    .status-copy strong {
        font-size: 13px;
        font-weight: 600;
    }
    .status-copy > span {
        color: var(--muted);
        font-size: 12px;
        display: flex;
        gap: 7px;
    }
    .capacity-readings {
        display: grid;
        gap: 13px;
        align-content: center;
    }
    .reading-label {
        display: flex;
        justify-content: space-between;
        gap: 12px;
        font-size: 11px;
        color: var(--muted);
    }
    .reading-label strong {
        color: var(--text);
        font-weight: 500;
        font-variant-numeric: tabular-nums;
    }
    .capacity-track {
        height: 5px;
        border-radius: 5px;
        background: var(--track);
        margin-top: 7px;
        position: relative;
        overflow: hidden;
    }
    .capacity-track > span {
        position: absolute;
        height: 100%;
    }
    .used {
        left: 0;
        background: var(--accent);
        transition: width 220ms;
    }
    .reserve {
        right: 0;
        border-left: 1px solid var(--muted);
        background: repeating-linear-gradient(
            120deg,
            transparent,
            transparent 3px,
            var(--border) 3px,
            var(--border) 5px
        );
    }
    @media (max-width: 900px) {
        .hardware-capacity {
            grid-template-columns: 1fr;
            gap: 18px;
        }
        .capacity-readings {
            grid-template-columns: 1fr 1fr;
            gap: 20px;
        }
    }
    @media (prefers-reduced-motion: reduce) {
        .used {
            transition: none;
        }
    }
</style>
