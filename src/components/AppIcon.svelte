<script lang="ts">
    import { applicationIcon } from '../lib/icons';
    import Icon from './Icon.svelte';

    let {
        workloadId,
        category = 'other',
        size = 32,
        demo = false,
    }: {
        workloadId: string;
        category?: string;
        size?: number;
        demo?: boolean;
    } = $props();
    let url: string | null = $state(null);
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
    {#if url}<img src={url} alt="" width={size} height={size} />
    {:else}<span class="fallback"
            ><Icon
                name={category === 'project' ? 'code' : category === 'model' ? 'model' : 'app'}
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
    .fallback {
        display: flex;
        width: 88%;
        height: 88%;
        align-items: center;
        justify-content: center;
        border-radius: 22%;
        background: var(--surface-secondary, rgba(128, 128, 128, 0.1));
        color: var(--muted);
    }
</style>
