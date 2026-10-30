<script lang="ts">
    import notices from '../../THIRD_PARTY_NOTICES.md?raw';
    import AutomationSettings from './AutomationSettings.svelte';
    import SettingsDefaults from './SettingsDefaults.svelte';
    import { untrack } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage, memory } from '../lib/format';
    import type { Bootstrap, LocalModel, Provider, TierName } from '../lib/types';

    let {
        bootstrap,
        desktop,
        onrefresh,
    }: { bootstrap: Bootstrap; desktop: boolean; onrefresh: () => Promise<void> } = $props();
    let settings = $state(untrack(() => $state.snapshot(bootstrap.settings)));
    let status = $state('');
    let error = $state('');
    let busy = $state(false);
    let models: LocalModel[] = $state([]);
    let provider: Provider = $state('meta');
    let credential = $state('');
    const tiers: TierName[] = ['decision', 'cheap', 'intelligent'];
    const defaults: Record<Provider, string> = {
        ollama: 'qwen3.5:0.8b',
        meta: 'muse-spark-1.3',
        anthropic: 'claude-haiku-5-5',
        jev: 'jev-1.14.0',
    };

    async function perform(action: () => Promise<void>) {
        busy = true;
        error = '';
        status = '';
        try {
            await action();
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }

    async function save() {
        if (!desktop) {
            status = 'Preview only. Settings are not saved.';
            return;
        }
        await perform(async () => {
            await invoke('save_settings', { settings });
            await onrefresh();
            status = 'Settings saved. Mode boundaries are enforced in Rust before inference.';
        });
    }

    async function saveKey() {
        if (!desktop) return;
        const value = credential;
        credential = '';
        await perform(async () => {
            await invoke('save_credential', { provider, credential: value });
            await onrefresh();
            status = 'Credential saved in your operating-system credential store.';
        });
    }
</script>

<div class="page-heading">
    <div>
        <h1>Settings</h1>
        <p class="muted">Choose how Jitterbug investigates and when it can help.</p>
    </div>
</div>
<AutomationSettings {bootstrap} {desktop} {onrefresh} />
<section class="panel settings-panel">
    <h2>Where should AI run?</h2>
    <div class="mode-options">
        {#each [{ value: 'hybrid', label: 'Hybrid', text: 'Tiny local models when resources allow; cloud for deeper work.' }, { value: 'local_only', label: 'Local only', text: 'No cloud inference or cloud fallback. Defer when hardware is busy.' }, { value: 'cloud_only', label: 'Cloud only', text: 'No local inference. Monitoring and rules still run on your computer.' }] as mode}
            <label class:active={settings.mode === mode.value}
                ><input
                    type="radio"
                    name="mode"
                    value={mode.value}
                    bind:group={settings.mode}
                /><strong>{mode.label}</strong>
                <p>{mode.text}</p></label
            >
        {/each}
    </div>
    <div class="section-title">
        <h2>Model tiers</h2>
        <label class="toggle"
            ><input type="checkbox" bind:checked={settings.decision_enabled} /> Enable optional decision
            model</label
        >
    </div>
    <p class="muted">
        Routing uses code by default. A decision model adds a request; it cannot override your mode
        or limits. Jev is optional and requires its own key.
    </p>
    {#each tiers as tier}
        <div class="tier-row">
            <div>
                <h3 class="capitalize">{tier}</h3>
                <p class="muted">
                    {tier === 'decision'
                        ? 'Choose a reasoning tier'
                        : tier === 'cheap'
                          ? 'Simple explanations'
                          : 'Deeper analysis and tradeoffs'}
                </p>
            </div>
            <label
                >Provider<select
                    bind:value={settings[tier].provider}
                    onchange={() => {
                        settings[tier].model =
                            settings[tier].provider === 'ollama' && tier === 'decision'
                                ? 'tev1:0.8b'
                                : defaults[settings[tier].provider];
                    }}
                    ><option value="ollama">Ollama · local</option><option value="meta"
                        >Meta · cloud</option
                    ><option value="anthropic">Claude · cloud</option
                    >{#if tier === 'decision'}<option value="jev">Jev · cloud</option>{/if}</select
                ></label
            ><label>Model<input bind:value={settings[tier].model} list="model-suggestions" /></label
            >
        </div>
    {/each}
    <datalist id="model-suggestions"
        ><option value="tev1:0.8b"></option><option value="qwen3.5:0.8b"></option><option
            value="muse-spark-1.3"
        ></option><option value="muse-spark-1.2"></option><option value="claude-haiku-5-5"
        ></option><option value="claude-sonnet-5-5"></option><option value="jev-1.14.0"
        ></option></datalist
    >
    <div class="budget-row">
        <label
            >Cloud budget ceiling (USD)<input
                type="number"
                min="0"
                max="100"
                step="0.25"
                bind:value={settings.budget_usd}
            /></label
        >
        <div>
            <strong>${bootstrap.spending.reported_estimate_usd.toFixed(5)}</strong> estimated usage
            <span class="muted">· {bootstrap.spending.requests} requests</span>
            <p class="muted">
                ${bootstrap.spending.reserved_usd.toFixed(4)} accounted against the budget. Successful
                requests settle to reported usage; unknown charges stay reserved after errors and restarts.
                The lower of this budget and the environment ceiling applies. Prototype caps: $0.10/request,
                200 requests, $100 maximum. This is an app ledger, not your provider’s billing balance.
            </p>
        </div>
    </div>
    <button class="primary" onclick={save} disabled={busy}>Save settings</button>
</section>

<section class="panel settings-panel">
    <h2>Credentials</h2>
    <p class="muted">
        {bootstrap.credentials.length
            ? `Configured: ${bootstrap.credentials.join(', ')}`
            : 'No cloud credentials configured.'} Keys stay in the Rust backend. Development .env values
        take precedence over stored keys.
    </p>
    <div class="credential-row">
        <label
            >Provider<select bind:value={provider}
                ><option value="meta">Meta</option><option value="anthropic">Claude</option><option
                    value="jev">Jev</option
                ></select
            ></label
        ><label
            >API key<input
                type="password"
                autocomplete="off"
                placeholder="Paste a key to store securely"
                bind:value={credential}
                disabled={!desktop}
            /></label
        ><button onclick={saveKey} disabled={!desktop || busy || !credential}>Store key</button>
    </div>
</section>

<section class="panel settings-panel">
    <div class="section-title">
        <h2>Local model limits</h2>
        <button
            disabled={!desktop || busy}
            onclick={() =>
                perform(async () => {
                    models = await invoke('list_local_models');
                    status = 'Installed model inventory refreshed. No inference was run.';
                })}>Check Ollama</button
        >
    </div>
    <details>
        <summary>Resource limits & setup</summary>
        <p class="muted">
            One request at a time. Up to 1.5 GiB model download size, 2,048-token chat context, 256
            generated tokens, and immediate unloading. Requires at least 4 GiB and 20% available
            memory, CPU below 75%, and no reported resource or thermal pressure. These gates reduce
            risk; download size is not peak memory usage. Models are never downloaded automatically.
        </p>
        <code>ollama pull qwen3.5:0.8b</code>
        <p class="muted">Optional decision model: <code>ollama pull tev1:0.8b</code></p>
    </details>
    {#each models as model}<div class="local-model">
            <span
                >{model.name} · {memory(model.size_bytes)} · {model.eligible
                    ? 'size eligible'
                    : 'excluded'}</span
            ><button
                disabled={!model.eligible || busy}
                onclick={() =>
                    perform(async () => {
                        status = await invoke('unload_model', { model: model.name });
                    })}>Unload</button
            >
        </div>{/each}
</section>
<SettingsDefaults {desktop} />
{#if status}<p class="success" role="status">{status}</p>{/if}{#if error}<p
        class="error"
        role="alert"
    >
        {error}
    </p>{/if}

<section class="panel settings-panel">
    <details>
        <summary>Open-source acknowledgments</summary>
        <pre class="license-text">{notices}</pre>
    </details>
</section>
