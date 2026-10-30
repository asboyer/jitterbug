<script lang="ts">
    import type { Snapshot } from '../lib/types';
    let {
        snapshot,
        values,
        marker,
        shade,
        label = 'Recent total CPU use, zero to 100 percent',
    }: {
        snapshot?: Snapshot;
        /** Explicit 0–100 series; replaces the snapshot's CPU history. */
        values?: number[];
        /** Fraction of the width (0–1) for a vertical marker line. */
        marker?: number;
        /** Fractions of the width (0–1) to shade. */
        shade?: [number, number];
        label?: string;
    } = $props();
    const series = $derived(values ?? snapshot?.history.map((p) => p.cpu_percent) ?? []);
    const points = $derived(
        series
            .map(
                (value, i) =>
                    `${(i / Math.max(series.length - 1, 1)) * 600},${60 - Math.min(Math.max(value, 0), 100) * 0.6}`
            )
            .join(' ')
    );
</script>

<svg
    class="activity-graph"
    viewBox="0 0 600 60"
    preserveAspectRatio="none"
    role="img"
    aria-label={label}
>
    {#if shade}
        <rect
            x={shade[0] * 600}
            y="0"
            width={Math.max(shade[1] - shade[0], 0) * 600}
            height="60"
            fill="var(--warning-soft)"
        />
    {/if}
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
    {#if marker !== undefined}
        <line
            x1={marker * 600}
            x2={marker * 600}
            y1="0"
            y2="60"
            stroke="var(--warning)"
            stroke-width="1.5"
            vector-effect="non-scaling-stroke"
        />
    {/if}
</svg>
