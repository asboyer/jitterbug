<script lang="ts">
    import type { Snapshot } from '../lib/types';
    let { snapshot }: { snapshot: Snapshot } = $props();
    const points = $derived(
        snapshot.history
            .map(
                (p, i) =>
                    `${(i / Math.max(snapshot.history.length - 1, 1)) * 600},${60 - p.cpu_percent * 0.6}`
            )
            .join(' ')
    );
</script>

<svg
    class="activity-graph"
    viewBox="0 0 600 60"
    preserveAspectRatio="none"
    role="img"
    aria-label="Recent total CPU use, zero to 100 percent"
>
    <path d="M0 15H600 M0 30H600 M0 45H600" stroke="var(--border)" stroke-width="0.5" />
    {#if points}
        <polygon points={`0,60 ${points} 600,60`} fill="var(--accent-soft)" />
        <polyline
            {points}
            fill="none"
            stroke="var(--accent)"
            stroke-width="1.5"
            vector-effect="non-scaling-stroke"
        />
    {/if}
</svg>
