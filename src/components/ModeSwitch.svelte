<script lang="ts">
    import { applyMode, missingKeys, modeChoices, modeLabel, providerNames } from '../lib/ai-mode';
    import type { Bootstrap, Mode, Settings } from '../lib/types';

    let {
        bootstrap,
        onsave,
        onkeys,
    }: {
        bootstrap: Bootstrap;
        onsave: (settings: Settings) => Promise<void>;
        onkeys: () => void;
    } = $props();
    let open = $state(false);
    let busy = $state(false);
    let error = $state('');
    const missing = $derived(missingKeys(bootstrap.settings, bootstrap.credentials));

    async function choose(mode: Mode) {
        if (busy || mode === bootstrap.settings.mode) return;
        busy = true;
        error = '';
        try {
            await onsave(applyMode($state.snapshot(bootstrap.settings), mode));
            // Stay open when the new mode still needs a key, so the warning is seen.
            if (!missingKeys(bootstrap.settings, bootstrap.credentials).length) open = false;
        } catch (cause) {
            error = String(cause);
        } finally {
            busy = false;
        }
    }
</script>

<details class="mode-switch" bind:open>
    <summary aria-label={`AI mode: ${modeLabel(bootstrap.settings.mode)}. Change`}>
        <span class="mode-caption">AI runs</span>
        <span class="mode-current"
            ><i class:warn={missing.length} aria-hidden="true"></i>{modeLabel(
                bootstrap.settings.mode
            )}</span
        >
    </summary>
    <div class="mode-menu" role="radiogroup" aria-label="Where AI runs">
        {#each modeChoices as choice}
            <button
                role="radio"
                aria-checked={bootstrap.settings.mode === choice.value}
                disabled={busy}
                onclick={() => choose(choice.value)}
            >
                <strong>{choice.label}</strong><small>{choice.hint}</small>
            </button>
        {/each}
        {#if missing.length}
            <p class="mode-warning">
                {missing.map((p) => providerNames[p]).join(' and ')} needs an API key.
                <button
                    class="text-button"
                    onclick={() => {
                        open = false;
                        onkeys();
                    }}>Add key</button
                >
            </p>
        {/if}
        {#if error}<p class="error" role="alert">{error}</p>{/if}
    </div>
</details>

<style>
    .mode-switch {
        position: relative;
        margin-top: 10px;
    }
    summary {
        list-style: none;
        display: flex;
        flex-direction: column;
        gap: 2px;
        padding: 7px 9px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--control);
        cursor: pointer;
    }
    summary::-webkit-details-marker {
        display: none;
    }
    .mode-caption {
        font-size: 10px;
        color: var(--muted);
    }
    .mode-current {
        display: flex;
        align-items: center;
        gap: 6px;
        font-size: 12px;
        font-weight: 600;
    }
    .mode-current::after {
        content: '⌃';
        margin-left: auto;
        color: var(--muted);
        font-weight: 400;
    }
    .mode-current i {
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: var(--success);
    }
    .mode-current i.warn {
        background: var(--warning);
    }
    .mode-menu {
        position: absolute;
        bottom: calc(100% + 6px);
        left: 0;
        right: 0;
        z-index: 15;
        display: grid;
        gap: 4px;
        padding: 4px;
        background: var(--panel);
        backdrop-filter: blur(24px) saturate(140%);
        -webkit-backdrop-filter: blur(24px) saturate(140%);
        border: 1px solid var(--border);
        border-radius: 10px;
        box-shadow: 0 12px 36px #0003;
    }
    .mode-menu > button {
        display: grid;
        gap: 2px;
        text-align: left;
        border: 1px solid transparent;
        background: transparent;
        min-height: 0;
        padding: 7px 9px;
        border-radius: 7px;
        box-shadow: none;
    }
    .mode-menu > button:hover {
        background: var(--hover);
    }
    .mode-menu strong {
        font-size: 12px;
    }
    .mode-menu > button[aria-checked='true'] {
        border-color: var(--accent);
        background: var(--accent-soft);
    }
    .mode-menu small {
        color: var(--muted);
        font-size: 10.5px;
        line-height: 1.4;
    }
    .mode-warning {
        display: grid;
        justify-items: start;
        gap: 2px;
        margin: 4px 5px 2px;
        font-size: 11px;
        color: var(--warning);
        line-height: 1.5;
    }
    .mode-warning button {
        min-height: 0;
        padding: 0;
        font-size: 11px;
    }
</style>
