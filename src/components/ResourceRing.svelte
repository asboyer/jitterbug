<script lang="ts">
    let { value, available = true }: { value: number; available?: boolean } = $props();
</script>

<svg class="resource-ring" viewBox="0 0 48 48" aria-hidden="true">
    <circle cx="24" cy="24" r="19" class="track" />
    {#if available}
        <circle
            cx="24"
            cy="24"
            r="19"
            class="value"
            pathLength="100"
            stroke-dasharray={`${Math.min(Math.max(value, 0), 100)} 100`}
        />
    {/if}
</svg>

<style>
    svg {
        width: 52px;
        height: 52px;
        flex-shrink: 0;
        transform: rotate(-90deg);
    }
    circle {
        fill: none;
        stroke-width: 4;
    }
    .track {
        stroke: var(--track);
    }
    .value {
        stroke: var(--accent);
        stroke-linecap: round;
    }
</style>
