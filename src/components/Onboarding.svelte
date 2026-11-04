<script lang="ts">
    import { onMount } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import AgentConnect from './AgentConnect.svelte';
    import Icon from './Icon.svelte';
    import { applyMode, frontierChoices, localModel, providerNames } from '../lib/ai-mode';
    import { errorMessage, memory } from '../lib/format';
    import type { Bootstrap, LocalModel, Settings, Tier } from '../lib/types';

    let {
        bootstrap,
        desktop,
        onsave,
        onfinish,
        onagent,
    }: {
        bootstrap: Bootstrap;
        desktop: boolean;
        onsave: (settings: Settings) => Promise<void>;
        /** `tour` is true when the person asked to see the tour next. */
        onfinish: (tour: boolean) => void;
        /** An agent was connected, which also turned agent access on. */
        onagent?: () => Promise<void>;
    } = $props();

    let dialog: HTMLDialogElement;
    let step = $state<'choose' | 'connect' | 'done'>('choose');
    let kind = $state<'frontier' | 'local'>('frontier');
    let model = $state<Tier>({ provider: 'meta', model: 'muse-spark-1.3' });
    let localHelp = $state(true);
    let credential = $state('');
    let budget = $state(2);
    let busy = $state(false);
    let error = $state('');
    let keySaved = $state(false);
    let models: LocalModel[] | null = $state(null);
    const chosen = $derived(
        frontierChoices.find((c) => c.provider === model.provider && c.model === model.model)
    );
    const hasKey = $derived(bootstrap.credentials.includes(model.provider));
    const steps = ['choose', 'connect', 'done'] as const;

    onMount(() => {
        dialog.showModal();
        return () => dialog.close();
    });

    function configured(): Settings {
        const base = $state.snapshot(bootstrap.settings) as Settings;
        if (kind === 'local') return applyMode(base, 'local_only');
        const next = applyMode(base, localHelp ? 'hybrid' : 'cloud_only', model);
        next.intelligent = { ...model };
        if (!localHelp) next.cheap = { ...model };
        next.budget_usd = Math.min(100, Math.max(0, Number(budget) || 0));
        return next;
    }

    async function connect() {
        busy = true;
        error = '';
        try {
            if (kind === 'frontier' && credential && desktop) {
                const value = credential;
                credential = '';
                await invoke('save_credential', { provider: model.provider, credential: value });
                keySaved = true;
            } else if (kind === 'frontier' && credential) {
                credential = '';
                keySaved = true;
            }
            await onsave(configured());
            step = 'done';
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }

    async function checkOllama() {
        error = '';
        if (!desktop) {
            models = [];
            return;
        }
        busy = true;
        try {
            models = await invoke<LocalModel[]>('list_local_models');
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
</script>

<dialog
    bind:this={dialog}
    class="onboarding"
    aria-labelledby="onboarding-title"
    oncancel={(event) => {
        event.preventDefault();
        onfinish(false);
    }}
>
    <div class="onboarding-head">
        <span class="onboarding-mark"><Icon name="companion" size={22} /></span>
        <ol class="onboarding-progress" aria-label="Setup progress">
            {#each steps as item, index}
                <li
                    class:current={item === step}
                    class:complete={steps.indexOf(step) > index}
                    aria-current={item === step ? 'step' : undefined}
                >
                    <span class="sr-only">Step {index + 1}</span>
                </li>
            {/each}
        </ol>
        <button class="text-button" onclick={() => onfinish(false)}>Skip setup</button>
    </div>

    {#if step === 'choose'}
        <p class="eyebrow">WELCOME TO JITTERBUG</p>
        <h2 id="onboarding-title">Where should Jitterbug think?</h2>
        <p class="muted lead">
            Watching for slowdowns always runs on this computer without AI. A model explains what it
            finds and suggests fixes.
        </p>
        <div class="kind-options" role="radiogroup" aria-label="Model type">
            <label class:active={kind === 'frontier'}>
                <input type="radio" bind:group={kind} value="frontier" />
                <span class="kind-title"><Icon name="model" size={18} /> Frontier model</span>
                <small>Deepest investigations. Uses your API key and a spending ceiling.</small>
            </label>
            <label class:active={kind === 'local'}>
                <input type="radio" bind:group={kind} value="local" />
                <span class="kind-title"><Icon name="cpu" size={18} /> Local model</span>
                <small>Private and free. A small model through Ollama for lighter answers.</small>
            </label>
        </div>
        {#if kind === 'frontier'}
            <fieldset class="model-options">
                <legend>Model</legend>
                {#each frontierChoices as choice}
                    <label
                        class:active={model.provider === choice.provider &&
                            model.model === choice.model}
                    >
                        <input
                            type="radio"
                            name="frontier-model"
                            checked={model.provider === choice.provider &&
                                model.model === choice.model}
                            onchange={() => {
                                model = { provider: choice.provider, model: choice.model };
                            }}
                        />
                        <span
                            ><strong
                                >{choice.label}{#if choice.recommended}<em>Recommended</em
                                    >{/if}</strong
                            ><small>{choice.text}</small></span
                        >
                    </label>
                {/each}
            </fieldset>
            <label class="check-row">
                <input type="checkbox" bind:checked={localHelp} />
                <span
                    >Use a local model for quick help when one is installed
                    <small>Hybrid. Saves cost on simple explanations.</small></span
                >
            </label>
        {:else}
            <div class="local-summary">
                <strong>Qwen 3.5 · 0.8B</strong>
                <small
                    >Runs one request at a time, only when the computer has room, and unloads right
                    after.</small
                >
            </div>
        {/if}
        <div class="onboarding-actions">
            <span></span>
            <button
                class="primary"
                onclick={() => {
                    error = '';
                    step = 'connect';
                }}>Continue</button
            >
        </div>
    {:else if step === 'connect' && kind === 'frontier'}
        <p class="eyebrow">CONNECT {providerNames[model.provider].toUpperCase()}</p>
        <h2 id="onboarding-title">Add your {providerNames[model.provider]} API key</h2>
        <p class="muted lead">
            The key goes to your operating system’s credential store. Jitterbug never writes it to
            its own files or shows it again.
        </p>
        {#if hasKey}<p class="success">
                A {providerNames[model.provider]} key is already configured. Paste a new one to replace
                it.
            </p>{/if}
        <label class="field"
            >API key<input
                type="password"
                autocomplete="off"
                placeholder="Paste a key"
                bind:value={credential}
            /></label
        >
        <label class="field budget"
            >Spending ceiling · USD<input
                type="number"
                min="0"
                max="100"
                step="0.25"
                bind:value={budget}
            /><small
                >Jitterbug stops calling {chosen?.label ?? 'the model'} once its estimate reaches this.</small
            ></label
        >
        {#if !desktop}<p class="muted">Preview: the key is not stored.</p>{/if}
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        <div class="onboarding-actions">
            <button class="text-button" disabled={busy} onclick={() => (step = 'choose')}
                >Back</button
            >
            <div>
                {#if !credential}<button disabled={busy} onclick={connect}>Skip for now</button
                    >{/if}
                <button class="primary" disabled={busy || !credential} onclick={connect}
                    >{busy ? 'Saving…' : 'Save and continue'}</button
                >
            </div>
        </div>
    {:else if step === 'connect'}
        <p class="eyebrow">SET UP OLLAMA</p>
        <h2 id="onboarding-title">Install a small model</h2>
        <p class="muted lead">
            Jitterbug never downloads models for you. With <a
                href="https://ollama.com"
                target="_blank"
                rel="noreferrer">Ollama</a
            > installed, run:
        </p>
        <pre class="command"><code>ollama pull {localModel.model}</code></pre>
        <button disabled={busy} onclick={checkOllama}>{busy ? 'Checking…' : 'Check Ollama'}</button>
        {#if models}
            {#if models.length}
                <ul class="found-models">
                    {#each models as found}<li>
                            {found.name} · {memory(found.size_bytes)}{found.eligible
                                ? ''
                                : ' · too large'}
                        </li>{/each}
                </ul>
            {:else}
                <p class="muted">
                    {desktop
                        ? 'No models found yet. You can finish now and pull one later.'
                        : 'Preview: Ollama is not checked.'}
                </p>
            {/if}
        {/if}
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        <div class="onboarding-actions">
            <button class="text-button" disabled={busy} onclick={() => (step = 'choose')}
                >Back</button
            >
            <button class="primary" disabled={busy} onclick={connect}
                >{busy ? 'Saving…' : 'Use local model'}</button
            >
        </div>
    {:else}
        <p class="eyebrow">ALL SET</p>
        <h2 id="onboarding-title">Jitterbug is ready</h2>
        <ul class="summary">
            <li>
                <Icon name="model" size={16} />
                {#if kind === 'local'}Local · Qwen 3.5 0.8B through Ollama{:else}{localHelp
                        ? 'Hybrid'
                        : 'Frontier'} · {chosen?.label}{/if}
            </li>
            {#if kind === 'frontier'}<li>
                    <Icon name="info" size={16} />
                    {keySaved || hasKey
                        ? `${providerNames[model.provider]} key ${desktop ? 'saved' : 'accepted (preview)'}`
                        : 'No key yet. Add one on the AI page.'}
                    · ${Number(budget).toFixed(2)} ceiling
                </li>{/if}
        </ul>
        <section class="onboarding-agents" aria-label="Connect coding agents">
            <h3><Icon name="code" size={16} /> Using Claude Code or Codex?</h3>
            <p class="muted">
                Connect them so they can see what loads this computer, and hear about it before
                heavy work.
            </p>
            <AgentConnect {desktop} onconnect={onagent} />
        </section>
        <p class="muted">Switch between local and frontier any time from the sidebar.</p>
        <div class="onboarding-actions">
            <button onclick={() => onfinish(false)}>Go to Jitterbug</button>
            <button class="primary" onclick={() => onfinish(true)}>Take the quick tour</button>
        </div>
    {/if}
</dialog>

<style>
    .onboarding {
        width: min(560px, calc(100vw - 40px));
        max-height: calc(100vh - 40px);
        overflow: auto;
        padding: 26px 30px 24px;
        border: 1px solid var(--border);
        border-radius: 16px;
        background: var(--panel);
        color: var(--text);
        box-shadow: 0 30px 90px #0007;
    }
    .onboarding::backdrop {
        background: #0008;
        backdrop-filter: blur(8px);
    }
    .onboarding-head {
        display: flex;
        align-items: center;
        gap: 14px;
        margin-bottom: 20px;
    }
    .onboarding-mark {
        display: grid;
        place-items: center;
        width: 36px;
        height: 36px;
        border-radius: 10px;
        background: var(--accent-soft);
        color: var(--accent);
    }
    .onboarding-progress {
        display: flex;
        gap: 6px;
        list-style: none;
        margin: 0 auto 0 0;
        padding: 0;
    }
    .onboarding-progress li {
        width: 22px;
        height: 4px;
        border-radius: 2px;
        background: var(--border);
    }
    .onboarding-progress li.current,
    .onboarding-progress li.complete {
        background: var(--accent);
    }
    h2 {
        font-size: 22px;
        margin: 6px 0 8px;
    }
    .lead {
        margin: 0 0 18px;
        line-height: 1.55;
    }
    .kind-options {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 10px;
        margin-bottom: 16px;
    }
    .kind-options label,
    .model-options label {
        display: flex;
        gap: 10px;
        border: 1px solid var(--border);
        border-radius: 10px;
        cursor: pointer;
    }
    .kind-options label {
        flex-direction: column;
        gap: 6px;
        padding: 14px;
        position: relative;
    }
    .kind-options input {
        position: absolute;
        opacity: 0;
    }
    .kind-options label:has(input:focus-visible) {
        outline: 2px solid var(--accent);
    }
    .kind-title {
        display: flex;
        align-items: center;
        gap: 8px;
        font-weight: 600;
    }
    label.active {
        border-color: var(--accent);
        background: var(--accent-soft);
    }
    small {
        color: var(--muted);
        line-height: 1.45;
        font-size: 12px;
    }
    .model-options {
        border: 0;
        padding: 0;
        margin: 0 0 12px;
        display: grid;
        gap: 6px;
    }
    .model-options legend {
        font-size: 12px;
        color: var(--muted);
        margin-bottom: 6px;
    }
    .model-options label {
        align-items: flex-start;
        padding: 10px 12px;
    }
    .model-options span {
        display: grid;
        gap: 2px;
    }
    em {
        font-style: normal;
        font-size: 10px;
        font-weight: 600;
        color: var(--accent);
        margin-left: 8px;
        padding: 1px 6px;
        border-radius: 4px;
        background: var(--accent-soft);
    }
    .check-row {
        display: flex;
        align-items: flex-start;
        gap: 10px;
        font-size: 13px;
    }
    .check-row span {
        display: grid;
        gap: 2px;
    }
    .local-summary {
        display: grid;
        gap: 4px;
        padding: 12px 14px;
        border-radius: 10px;
        background: var(--sidebar);
    }
    .field {
        display: grid;
        gap: 6px;
        font-size: 12px;
        color: var(--muted);
        margin-bottom: 14px;
    }
    .field input {
        font-size: 14px;
    }
    .budget input {
        max-width: 120px;
    }
    .command {
        padding: 10px 12px;
        border-radius: 8px;
        background: var(--sidebar);
        margin: 0 0 12px;
    }
    .found-models {
        margin: 12px 0 0;
        padding-left: 18px;
        font-size: 13px;
    }
    .summary {
        list-style: none;
        padding: 0;
        margin: 14px 0;
        display: grid;
        gap: 10px;
    }
    .summary li {
        display: flex;
        align-items: flex-start;
        gap: 10px;
        line-height: 1.45;
    }
    .summary :global(svg) {
        flex-shrink: 0;
        margin-top: 2px;
        color: var(--accent);
    }
    .onboarding-agents h3 {
        display: flex;
        align-items: center;
        gap: 10px;
        margin: 18px 0 2px;
        font-size: 14px;
    }
    .onboarding-agents h3 :global(svg) {
        color: var(--accent);
    }
    .onboarding-agents p {
        margin: 0;
        font-size: 13px;
    }
    .onboarding-agents :global(.integration-list) {
        margin-top: 8px;
    }
    .onboarding-actions {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 10px;
        margin-top: 22px;
    }
    .onboarding-actions div {
        display: flex;
        gap: 8px;
    }
    @media (max-width: 560px) {
        .kind-options {
            grid-template-columns: 1fr;
        }
    }
</style>
