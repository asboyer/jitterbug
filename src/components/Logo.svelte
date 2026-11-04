<script lang="ts">
    // The jitterbug: a beetle seen from above whose wing seam is a pulse line.
    let { size = 22, live = false }: { size?: number; live?: boolean } = $props();
</script>

<svg
    class="logo"
    class:live
    width={size}
    height={size}
    viewBox="0 0 32 32"
    aria-hidden="true"
    focusable="false"
>
    <path
        class="antenna"
        d="M12.6 7.4c-.9-1.8-2.4-3-4.3-3.4M19.4 7.4c.9-1.8 2.4-3 4.3-3.4"
        fill="none"
        stroke-width="1.8"
        stroke-linecap="round"
    />
    <path class="head" d="M10.6 9.6a5.4 4.2 0 0 1 10.8 0z" />
    <ellipse class="body" cx="16" cy="18.4" rx="9.6" ry="10.4" />
    <path
        class="seam"
        d="M16 9.4v4.2l-2.6 3.1 5.2 3.4-2.6 3.1v5.4"
        fill="none"
        stroke-width="1.9"
        stroke-linecap="round"
        stroke-linejoin="round"
    />
</svg>

<style>
    .logo {
        display: block;
        flex: none;
        overflow: visible;
    }
    .antenna {
        stroke: var(--brand-ink, var(--text));
    }
    .head {
        fill: var(--brand-ink, var(--text));
    }
    .body {
        fill: var(--brand);
    }
    .seam {
        stroke: var(--brand-deep);
    }
    .live .seam {
        stroke-dasharray: 26;
        animation: pulse 2.4s ease-in-out infinite;
    }
    @keyframes pulse {
        0%,
        100% {
            stroke-dashoffset: 0;
        }
        50% {
            stroke-dashoffset: 52;
        }
    }
    @media (prefers-reduced-motion: reduce) {
        .live .seam {
            animation: none;
        }
    }
</style>
