<script lang="ts">
    import claude from '../assets/harnesses/claude.svg';
    import openai from '../assets/harnesses/openai.svg';
    import meta from '../assets/harnesses/meta.svg';
    import Icon from './Icon.svelte';
    let { harness, compact = false }: { harness: string; compact?: boolean } = $props();
    const brand = $derived.by(() => {
        const name = harness.toLowerCase();
        if (name.includes('claude')) return { label: 'Claude Code', image: claude };
        if (name.includes('codex') || name.includes('openai') || name.includes('chatgpt'))
            return { label: 'Codex', image: openai };
        if (name.includes('muse') || name.includes('meta'))
            return { label: 'Meta · Muse Code', image: meta };
        return { label: harness || 'Unknown harness', image: null };
    });
</script>

<span class="harness-badge" class:compact title={brand.label}>
    {#if brand.image}<img
            class="logo"
            src={brand.image}
            alt=""
            width="17"
            height="17"
        />{:else}<Icon name="code" size={16} />{/if}
    <span class:visually-hidden={compact}>{brand.label}</span>
</span>

<style>
    .harness-badge {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        color: var(--muted);
        font-size: 11px;
        white-space: nowrap;
    }
    .logo {
        display: inline-block;
        width: 17px;
        height: 17px;
        object-fit: contain;
        opacity: 0.8;
    }
    @media (prefers-color-scheme: dark) {
        .logo {
            filter: invert(1);
        }
    }
    .compact {
        padding: 8px;
        background: var(--hover);
        border-radius: 9px;
    }
    .compact .logo {
        width: 22px;
        height: 22px;
    }
    .visually-hidden {
        position: absolute;
        width: 1px;
        height: 1px;
        overflow: hidden;
        clip-path: inset(50%);
    }
</style>
