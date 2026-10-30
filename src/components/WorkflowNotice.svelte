<script lang="ts">
    import Icon from './Icon.svelte';
    import InfoTip from './InfoTip.svelte';
    let {
        demo,
        busy,
        error,
        onprotect,
        onlater,
    }: { demo: boolean; busy: boolean; error: string; onprotect: () => void; onlater: () => void } =
        $props();
</script>

<section class="workflow-notice" aria-label="Make room for your apps" aria-live="polite">
    <header>
        <Icon name="companion" size={21} /><strong>Jitterbug</strong><small
            >{demo ? 'DEMO' : 'LIVE'}</small
        ><button class="icon-button" aria-label="Dismiss scheduling suggestion" onclick={onlater}
            >×</button
        >
    </header>
    <h2>Make room for your apps?</h2>
    <p>Keep upcoming checks waiting a little longer.</p>
    <div class="notice-actions">
        <button disabled={busy} onclick={onlater}>Not now</button><button
            class="primary"
            disabled={busy}
            onclick={onprotect}>Hold for 20 min</button
        ><InfoTip label="What changes?"
            >Running checks finish normally. Upcoming checks stay held for 20 minutes, then resume
            when resource limits allow. You can end protection in Workflows.</InfoTip
        >
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
</section>

<style>
    .workflow-notice {
        position: fixed;
        right: 22px;
        bottom: 22px;
        z-index: 19;
        width: min(380px, calc(100vw - 40px));
        padding: 20px;
        background: var(--panel);
        border: 1px solid var(--border);
        border-radius: 16px;
        box-shadow: 0 12px 40px #0003;
    }
    header {
        display: flex;
        gap: 9px;
        align-items: center;
    }
    header strong {
        font-size: 12px;
    }
    header small {
        font-size: 10px;
        color: var(--muted);
    }
    header button {
        margin-left: auto;
    }
    h2 {
        font-size: 20px;
        letter-spacing: -0.4px;
        margin: 20px 0 8px;
    }
    p {
        font-size: 12px;
        color: var(--muted);
        margin: 0;
    }
    .notice-actions {
        display: flex;
        gap: 10px;
        align-items: center;
        margin-top: 20px;
    }
    .notice-actions > button {
        flex: 1;
        min-height: 36px;
    }
</style>
