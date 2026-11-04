<script lang="ts">
    import '../settings.css';
    import SettingSwitch from './SettingSwitch.svelte';
    import AiActivityLog from './AiActivityLog.svelte';
    import { untrack } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage, memory } from '../lib/format';
    import { applyMode, missingKeys, modeChoices, providerNames } from '../lib/ai-mode';
    import type {
        AutomationCase,
        Bootstrap,
        LocalModel,
        Mode,
        Provider,
        Tier,
        TierName,
    } from '../lib/types';

    let {
        bootstrap,
        desktop,
        onrefresh,
        cases = [],
    }: {
        bootstrap: Bootstrap;
        desktop: boolean;
        onrefresh: () => Promise<void>;
        /** Incidents, so an investigation can show what happened next. */
        cases?: AutomationCase[];
    } = $props();

    let settings = $state(untrack(() => $state.snapshot(bootstrap.settings)));
    let saved = $state(untrack(() => JSON.stringify(bootstrap.settings)));
    const dirty = $derived(JSON.stringify(settings) !== saved);
    const missing = $derived(missingKeys(settings, bootstrap.credentials));
    let status = $state('');
    let error = $state('');
    let busy = $state(false);
    let editing: Provider | null = $state(null);
    let credential = $state('');
    let models: LocalModel[] | null = $state(null);

    /** Every cloud ID here has a reviewed price in settings.rs. */
    const catalog: (Tier & { label: string; routing?: boolean })[] = [
        { provider: 'ollama', model: 'qwen3.5:0.8b', label: 'Qwen 3.5 0.8B · local' },
        { provider: 'ollama', model: 'tev1:0.8b', label: 'Tev1 0.8B · local', routing: true },
        { provider: 'meta', model: 'muse-spark-1.3', label: 'Meta Muse Spark 1.3' },
        { provider: 'meta', model: 'muse-spark-1.2', label: 'Meta Muse Spark 1.2' },
        { provider: 'anthropic', model: 'claude-haiku-5-5', label: 'Claude Haiku 5.5' },
        { provider: 'anthropic', model: 'claude-sonnet-5-5', label: 'Claude Sonnet 5.5' },
        { provider: 'jev', model: 'jev-1.14.0', label: 'Jev 1.14 · routing', routing: true },
    ];
    const key = (tier: Tier) => `${tier.provider}/${tier.model}`;
    function options(tier: TierName) {
        const list = catalog.filter((item) =>
            tier === 'decision' ? item.routing || item.provider !== 'ollama' : !item.routing
        );
        const current = settings[tier];
        if (!list.some((item) => key(item) === key(current)))
            list.unshift({
                ...current,
                label: `${providerNames[current.provider]} · ${current.model}`,
            });
        return list;
    }
    const blocked = (tier: Tier) =>
        (settings.mode === 'local_only' && tier.provider !== 'ollama') ||
        (settings.mode === 'cloud_only' && tier.provider === 'ollama');
    function choose(tier: TierName, value: string) {
        const [provider, ...rest] = value.split('/');
        settings[tier] = { provider: provider as Provider, model: rest.join('/') };
    }
    const rows: { tier: TierName; title: string; text: string }[] = [
        {
            tier: 'cheap',
            title: 'Quick help',
            text: 'Short answers, like what a process is or why an app is busy.',
        },
        {
            tier: 'intelligent',
            title: 'Deep investigation',
            text: 'Looks into slowdowns that persist and proposes a fix.',
        },
    ];
    const cloudProviders = [
        { id: 'meta', text: 'Muse Spark models' },
        { id: 'anthropic', text: 'Claude Haiku and Sonnet' },
        { id: 'jev', text: 'Routing model only' },
    ] as const;
    function usedBy(name: Provider) {
        const used = [...rows.map((row) => row.tier), 'decision' as const]
            .filter((tier) => tier !== 'decision' || settings.decision_enabled)
            .filter((tier) => settings[tier].provider === name);
        return used.map((tier) =>
            tier === 'decision' ? 'routing' : rows.find((r) => r.tier === tier)!.title.toLowerCase()
        );
    }

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
        const submitted = $state.snapshot(settings);
        await perform(async () => {
            await invoke('save_settings', { settings: submitted });
            await onrefresh();
            saved = JSON.stringify(submitted);
            status = 'AI settings saved.';
        });
    }
    async function saveKey() {
        const provider = editing;
        if (!desktop || !provider) return;
        const value = credential;
        credential = '';
        await perform(async () => {
            await invoke('save_credential', { provider, credential: value });
            await onrefresh();
            editing = null;
            status = `${providerNames[provider]} key saved to your OS credential store.`;
        });
    }
</script>

<div class="settings-workspace ai-page">
    <div class="settings-title">
        <div>
            <h1>AI</h1>
            <p class="muted">Monitoring always runs on this computer. AI explains and suggests.</p>
        </div>
        {#if !desktop}<span class="tag">Preview · changes stay here</span>{/if}
    </div>

    <AiActivityLog {desktop} {cases} />

    <section class="settings-group" aria-labelledby="ai-where">
        <h2 id="ai-where">Where AI runs</h2>
        <div class="mode-options">
            {#each modeChoices as mode}
                <label class:active={settings.mode === mode.value}>
                    <input
                        type="radio"
                        name="mode"
                        value={mode.value}
                        checked={settings.mode === mode.value}
                        onchange={() => {
                            settings = applyMode($state.snapshot(settings), mode.value as Mode);
                        }}
                    />
                    <strong>{mode.label}</strong>
                    <p>{mode.text}</p>
                </label>
            {/each}
        </div>
        {#if missing.length}<p class="group-note warn">
                {missing.map((p) => providerNames[p]).join(' and ')} needs an API key. Add it under Model
                providers below.
            </p>{/if}
    </section>

    <section class="settings-group" aria-label="Models">
        {#each rows as row}
            <div class="setting-row">
                <div class="setting-copy">
                    <strong>{row.title}</strong>
                    <span>{row.text}</span>
                </div>
                <select
                    aria-label={`${row.title} model`}
                    value={key(settings[row.tier])}
                    onchange={(event) => choose(row.tier, event.currentTarget.value)}
                >
                    {#each options(row.tier) as item}
                        <option value={key(item)} disabled={blocked(item)}>{item.label}</option>
                    {/each}
                </select>
            </div>
        {/each}
        <SettingSwitch
            title="Smart routing"
            description="A small model decides which of the two answers each question. Off: built-in rules decide, at no cost."
            bind:checked={settings.decision_enabled}
        />
        {#if settings.decision_enabled}
            <div class="setting-row nested">
                <div class="setting-copy">
                    <strong>Routing model</strong>
                    <span>Adds one small request per question.</span>
                </div>
                <select
                    aria-label="Routing model"
                    value={key(settings.decision)}
                    onchange={(event) => choose('decision', event.currentTarget.value)}
                >
                    {#each options('decision') as item}
                        <option value={key(item)} disabled={blocked(item)}>{item.label}</option>
                    {/each}
                </select>
            </div>
        {/if}
    </section>

    <section class="settings-group" aria-labelledby="ai-providers">
        <h2 id="ai-providers">Model providers</h2>
        <p class="group-note">Bring your own keys. They stay in your OS credential store.</p>
        <ul class="integration-list" aria-label="Model providers">
            {#each cloudProviders as item}
                {@const configured = bootstrap.credentials.includes(item.id)}
                {@const uses = usedBy(item.id)}
                <li>
                    <div class="integration-row">
                        <div class="integration-name">
                            <strong>{providerNames[item.id]}</strong>
                            <small
                                >{uses.length ? `Used for ${uses.join(' and ')}` : item.text}</small
                            >
                        </div>
                        <span
                            class="integration-status"
                            class:configured
                            class:needed={!configured && uses.length > 0}
                            >{configured
                                ? 'Connected'
                                : uses.length
                                  ? 'Key needed'
                                  : 'Not connected'}</span
                        >
                        <button
                            aria-expanded={editing === item.id}
                            onclick={() => {
                                credential = '';
                                editing = editing === item.id ? null : item.id;
                            }}
                            >{editing === item.id
                                ? 'Cancel'
                                : configured
                                  ? 'Replace key'
                                  : 'Add key'}</button
                        >
                    </div>
                    {#if editing === item.id}
                        <div class="credential-row">
                            <label
                                >{providerNames[item.id]} API key<input
                                    type="password"
                                    autocomplete="off"
                                    placeholder="Paste a key"
                                    bind:value={credential}
                                    disabled={!desktop}
                                /></label
                            ><button
                                class="primary"
                                onclick={saveKey}
                                disabled={!desktop || busy || !credential}>Store key</button
                            >
                        </div>
                        <p class="muted footnote">
                            {desktop
                                ? 'Development .env values take precedence over stored keys.'
                                : 'Preview: keys cannot be stored here.'}
                        </p>
                    {/if}
                </li>
            {/each}
            <li>
                <div class="integration-row">
                    <div class="integration-name">
                        <strong>Ollama</strong>
                        <small
                            >Local, no key. One request at a time, only with 4 GiB + 20% memory free
                            and CPU under 75%; unloads after each use.</small
                        >
                    </div>
                    <span class="integration-status local">Local</span>
                    <button
                        disabled={!desktop || busy}
                        onclick={() =>
                            perform(async () => {
                                models = await invoke<LocalModel[]>('list_local_models');
                            })}>Check Ollama</button
                    >
                </div>
                {#if models}
                    {#each models as model}<div class="local-model">
                            <span
                                >{model.name} · {memory(model.size_bytes)}{model.eligible
                                    ? ''
                                    : ' · too large'}</span
                            ><button
                                disabled={!model.eligible || busy}
                                onclick={() =>
                                    perform(async () => {
                                        status = await invoke('unload_model', {
                                            model: model.name,
                                        });
                                    })}>Unload</button
                            >
                        </div>{:else}<p class="muted footnote">
                            No models yet. Run <code>ollama pull qwen3.5:0.8b</code>.
                        </p>{/each}
                {/if}
            </li>
        </ul>
    </section>

    <section class="settings-group" aria-labelledby="ai-spending">
        <h2 id="ai-spending">Spending</h2>
        <div class="setting-row">
            <div class="setting-copy">
                <strong>Spending ceiling</strong>
                <span
                    >${bootstrap.spending.reported_estimate_usd.toFixed(2)} estimated across {bootstrap
                        .spending.requests} requests. Cloud calls stop at the ceiling.</span
                >
            </div>
            <label class="ceiling"
                ><span class="sr-only">Spending ceiling · USD</span>$<input
                    type="number"
                    min="0"
                    max="100"
                    step="0.25"
                    bind:value={settings.budget_usd}
                /></label
            >
        </div>
        <details class="inline-help">
            <summary>Usage estimate, not a provider bill</summary>
            <p>
                ${bootstrap.spending.reserved_usd.toFixed(4)} is accounted against the ceiling. Successful
                requests settle to reported usage; unknown charges stay reserved after errors and restarts.
                The lower of this ceiling and the environment ceiling applies. Prototype caps: $0.10 per
                request, 200 requests, $100 maximum.
            </p>
        </details>
    </section>

    {#if status}<p class="success" role="status">{status}</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="preference-footer sticky-save">
        <span class="muted">{dirty ? 'Unsaved changes' : 'Applies before every model call.'}</span>
        <button class="primary" onclick={save} disabled={busy || !dirty}
            >{busy ? 'Saving…' : 'Save AI settings'}</button
        >
    </div>
</div>
