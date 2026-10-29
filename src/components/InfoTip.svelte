<script lang="ts">
    import type { Snippet } from 'svelte';
    import Icon from './Icon.svelte';

    let { label, children }: { label: string; children: Snippet } = $props();
    let open = $state(false);
    let element: HTMLDivElement;
    let trigger: HTMLButtonElement;

    function outside(event: Event) {
        if (open && event.target instanceof Node && !element.contains(event.target)) open = false;
    }
    function escape(event: KeyboardEvent) {
        if (open && event.key === 'Escape') {
            event.preventDefault();
            event.stopPropagation();
            open = false;
            trigger?.focus();
        }
    }
</script>

<svelte:window onpointerdown={outside} onclick={outside} onkeydown={escape} />
<div class="info-tip" class:active={open} bind:this={element}>
    <button
        class="info-button"
        type="button"
        aria-label={label}
        aria-expanded={open}
        title={label}
        bind:this={trigger}
        onclick={() => {
            open = !open;
        }}><Icon name="info" size={17} /></button
    >
    {#if open}<div class="tip-content" role="region" aria-label={label}>
            {@render children()}
        </div>{/if}
</div>

<style>
    .info-tip {
        position: relative;
        flex: 0 0 auto;
    }
    .info-button {
        display: flex;
        align-items: center;
        justify-content: center;
        border: 0;
        padding: 0;
        background: transparent;
        cursor: pointer;
        color: var(--muted);
        width: 28px;
        height: 28px;
        border-radius: 6px;
    }
    .info-button:hover,
    .info-tip.active > .info-button {
        color: var(--text);
        background: var(--hover);
    }
    .tip-content {
        position: absolute;
        z-index: 12;
        right: 0;
        top: 34px;
        width: 275px;
        max-width: calc(100vw - 48px);
        padding: 14px;
        border-radius: 9px;
        border: 1px solid var(--border);
        background: var(--panel);
        box-shadow: 0 8px 24px #0003;
        font-size: 12px;
        line-height: 1.6;
        color: var(--text);
    }
    .tip-content :global(p) {
        margin: 0 0 10px;
        font-size: 12px;
    }
    .tip-content :global(p:last-child) {
        margin-bottom: 0;
    }
</style>
