<script lang="ts">
    import { applicationIcon } from '../lib/icons';
    import Icon from './Icon.svelte';
    import claude from '../assets/harnesses/claude.svg';
    import openai from '../assets/harnesses/openai.svg';
    import meta from '../assets/harnesses/meta.svg';

    let {
        workloadId,
        category = 'other',
        size = 32,
        demo = false,
        label = '',
    }: {
        workloadId: string;
        category?: string;
        label?: string;
        size?: number;
        demo?: boolean;
    } = $props();
    let url: string | null = $state(null);
    // Command-line agents have no artwork of their own: show the agent's mark on a tile.
    // A desktop app keeps its own icon.
    const agent = $derived.by(() => {
        const name = label.toLowerCase();
        const cli =
            name.includes(' · ') ||
            /^(claude|codex|gemini|opencode|aider|goose|amp|cursor-agent)$/.test(name);
        if (category !== 'agent' || !cli) return null;
        if (name.includes('claude')) return { image: claude, tint: '#d97757' };
        if (name.includes('codex')) return { image: openai, tint: '#2f2f34' };
        if (name.includes('muse') || name.includes('meta')) return { image: meta, tint: '#0866ff' };
        return null;
    });
    $effect(() => {
        let cancelled = false;
        url = null;
        if (!demo)
            void applicationIcon(workloadId).then((value) => {
                if (!cancelled) url = value;
            });
        return () => {
            cancelled = true;
        };
    });
</script>

<span class="app-icon" style={`width:${size}px;height:${size}px`} aria-hidden="true">
    {#if agent}<span class="agent-tile" style={`background:${agent.tint}`}
            ><img
                src={agent.image}
                alt=""
                width={Math.round(size * 0.58)}
                height={Math.round(size * 0.58)}
            /></span
        >
    {:else if url}<img src={url} alt="" width={size} height={size} />
    {:else}<span class="fallback"
            ><Icon
                name={category === 'project' || category === 'agent'
                    ? 'code'
                    : category === 'model'
                      ? 'model'
                      : 'app'}
                size={Math.round(size * 0.65)}
            /></span
        >{/if}
</span>

<style>
    .app-icon {
        display: inline-flex;
        flex: 0 0 auto;
        align-items: center;
        justify-content: center;
    }
    img {
        width: 100%;
        height: 100%;
        object-fit: contain;
    }
    .agent-tile {
        width: 86%;
        height: 86%;
        border-radius: 22%;
        background: #2f2f34;
        box-shadow: inset 0 0 0 0.5px #ffffff26;
        display: grid;
        place-items: center;
    }
    .agent-tile img {
        width: 58%;
        height: 58%;
        filter: brightness(0) invert(1);
    }
    .fallback {
        display: flex;
        width: 88%;
        height: 88%;
        align-items: center;
        justify-content: center;

        color: var(--muted);
    }
</style>
