<script lang="ts">
    import '../settings.css';
    import Icon from './Icon.svelte';
    import SettingSwitch from './SettingSwitch.svelte';
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
    const sections = [
        { id: 'assistance', label: 'Assistance', icon: 'activity' },
        { id: 'models', label: 'AI & budget', icon: 'model' },
        { id: 'connections', label: 'Connections', icon: 'code' },
        { id: 'system', label: 'System', icon: 'settings' },
    ] as const;
    let section: (typeof sections)[number]['id'] = $state('assistance');
    let savedSettings = $state(untrack(() => JSON.stringify(bootstrap.settings)));
    let dirty = $derived(JSON.stringify(settings) !== savedSettings);
    const providerNames: Record<Provider, string> = {
        ollama: 'Ollama',
        meta: 'Meta',
        anthropic: 'Claude',
        jev: 'Jev',
    };
    let budgetPercent = $derived(
        settings.budget_usd > 0
            ? Math.min(100, (bootstrap.spending.reserved_usd / settings.budget_usd) * 100)
            : 0
    );
    function chooseSection(next: typeof section) {
        section = next;
        document.getElementById('settings-content')?.closest('main')?.scrollTo({ top: 0 });
    }
    function navigate(event: KeyboardEvent) {
        const current = sections.findIndex((item) => item.id === section);
        let index = current;
        if (event.key === 'ArrowRight') index = (current + 1) % sections.length;
        else if (event.key === 'ArrowLeft')
            index = (current + sections.length - 1) % sections.length;
        else if (event.key === 'Home') index = 0;
        else if (event.key === 'End') index = sections.length - 1;
        else return;
        event.preventDefault();
        chooseSection(sections[index].id);
        document.getElementById(`settings-tab-${section}`)?.focus();
    }
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
        const submitted = $state.snapshot(settings);
        await perform(async () => {
            await invoke('save_settings', { settings: submitted });
            await onrefresh();
            savedSettings = JSON.stringify(submitted);
            status = 'AI settings saved.';
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

<div class="settings-workspace">
    <div class="settings-title">
        <h1>Settings</h1>
        {#if !desktop}<span class="tag">Preview · changes stay here</span>{/if}
    </div>
    <div class="settings-tabs" role="tablist" aria-label="Settings categories">
        {#each sections as item}
            <button
                id={`settings-tab-${item.id}`}
                role="tab"
                aria-selected={section === item.id}
                aria-controls="settings-content"
                tabindex={section === item.id ? 0 : -1}
                onclick={() => chooseSection(item.id)}
                onkeydown={navigate}><Icon name={item.icon} size={17} />{item.label}</button
            >
        {/each}
    </div>
    <div
        id="settings-content"
        role="tabpanel"
        aria-labelledby={`settings-tab-${section}`}
        tabindex="0"
    >
        <AutomationSettings
            {bootstrap}
            {desktop}
            {onrefresh}
            section={section === 'connections' ? 'connections' : 'assistance'}
            visible={section === 'assistance' || section === 'connections'}
        />
        <div hidden={section !== 'models'}>
            <section class="preference-card" aria-label="AI mode and models">
                <div class="preference-heading">
                    <span class="preference-icon"><Icon name="model" size={20} /></span>
                    <div>
                        <h2>Where AI runs</h2>
                        <p>Monitoring always stays on this computer.</p>
                    </div>
                </div>
                <div class="mode-options">
                    {#each [{ value: 'hybrid', label: 'Hybrid', text: 'Local for light work. Cloud for depth.' }, { value: 'local_only', label: 'Local only', text: 'No cloud calls. Wait when hardware is busy.' }, { value: 'cloud_only', label: 'Cloud only', text: 'No local inference. Uses your API keys.' }] as mode}
                        <label class:active={settings.mode === mode.value}>
                            <input
                                type="radio"
                                name="mode"
                                value={mode.value}
                                bind:group={settings.mode}
                            />
                            <strong>{mode.label}</strong>
                            <p>{mode.text}</p>
                        </label>
                    {/each}
                </div>
                <details class="model-configuration">
                    <summary>
                        <span>Model routing</span>
                        <span class="routing-preview"
                            >{settings.decision_enabled
                                ? providerNames[settings.decision.provider]
                                : 'Rules'} <span aria-hidden="true">→</span>
                            {providerNames[settings.cheap.provider]}
                            <span aria-hidden="true">/</span>
                            {providerNames[settings.intelligent.provider]}</span
                        >
                    </summary>
                    <SettingSwitch
                        title="Use a decision model"
                        description="Adds a routing request. Rules are used by default; modes and limits always apply."
                        bind:checked={settings.decision_enabled}
                    />
                    {#each tiers as tier}
                        <div class="tier-row">
                            <div>
                                <h3>
                                    {tier === 'decision'
                                        ? 'Routing'
                                        : tier === 'cheap'
                                          ? 'Quick help'
                                          : 'Deep investigation'}
                                </h3>
                                <p class="muted">
                                    {tier === 'decision'
                                        ? 'Optional model'
                                        : tier === 'cheap'
                                          ? 'Simple explanations'
                                          : 'Complex problems'}
                                </p>
                            </div>
                            <label
                                >Provider<select
                                    aria-label={`${tier} provider`}
                                    bind:value={settings[tier].provider}
                                    onchange={() => {
                                        settings[tier].model =
                                            settings[tier].provider === 'ollama' &&
                                            tier === 'decision'
                                                ? 'tev1:0.8b'
                                                : defaults[settings[tier].provider];
                                    }}
                                >
                                    <option value="ollama">Ollama · local</option><option
                                        value="meta">Meta · cloud</option
                                    ><option value="anthropic">Claude · cloud</option
                                    >{#if tier === 'decision'}<option value="jev"
                                            >Jev · cloud</option
                                        >{/if}
                                </select></label
                            >
                            <label
                                >Model<input
                                    aria-label={`${tier} model`}
                                    bind:value={settings[tier].model}
                                    list="model-suggestions"
                                /></label
                            >
                        </div>
                    {/each}
                    <p class="footnote">Each provider needs its own key. Jev is optional.</p>
                </details>
                <datalist id="model-suggestions"
                    ><option value="tev1:0.8b"></option><option value="qwen3.5:0.8b"
                    ></option><option value="muse-spark-1.3"></option><option value="muse-spark-1.2"
                    ></option><option value="claude-haiku-5-5"></option><option
                        value="claude-sonnet-5-5"
                    ></option><option value="jev-1.14.0"></option></datalist
                >
            </section>
            <section class="preference-card" aria-label="Cloud budget">
                <div class="budget-overview">
                    <div>
                        <h2>Cloud budget</h2>
                        <span class="budget-amount"
                            >${bootstrap.spending.reported_estimate_usd.toFixed(2)}</span
                        ><span class="muted">
                            estimated · {bootstrap.spending.requests} requests</span
                        >
                    </div>
                    <label
                        >Spending ceiling · USD<input
                            type="number"
                            min="0"
                            max="100"
                            step="0.25"
                            bind:value={settings.budget_usd}
                        /></label
                    >
                </div>
                <div
                    class="budget-track"
                    role="meter"
                    aria-label="Budget accounted for"
                    aria-valuemin="0"
                    aria-valuemax={Math.max(0, settings.budget_usd || 0)}
                    aria-valuenow={Math.min(
                        bootstrap.spending.reserved_usd,
                        Math.max(0, settings.budget_usd || 0)
                    )}
                    aria-valuetext={`$${bootstrap.spending.reserved_usd.toFixed(4)} accounted against $${settings.budget_usd || 0}`}
                >
                    <span style:width={`${budgetPercent}%`}></span>
                </div>
                <p class="budget-caption">
                    ${bootstrap.spending.reserved_usd.toFixed(4)} accounted against your ceiling
                </p>
                <details class="inline-help">
                    <summary>Usage estimate, not a provider bill</summary>
                    <p>
                        ${bootstrap.spending.reserved_usd.toFixed(4)} is accounted against the budget.
                        Successful requests settle to reported usage; unknown charges stay reserved after
                        errors and restarts. The lower of this ceiling and the environment ceiling applies.
                        Prototype caps: $0.10 per request, 200 requests, $100 maximum. This is Jitterbug’s
                        ledger, not your provider’s billing balance.
                    </p>
                </details>
            </section>
            <div class="preference-footer">
                <span class="muted"
                    >{dirty
                        ? 'Unsaved changes'
                        : 'Your mode applies before every model call.'}</span
                ><button class="primary" onclick={save} disabled={busy}
                    >{busy ? 'Saving…' : 'Save AI settings'}</button
                >
            </div>
            <details class="preference-detail local-models">
                <summary
                    ><span class="detail-name"><Icon name="cpu" /> Local model limits</span><span
                        class="detail-value">One request at a time</span
                    ></summary
                >
                <div class="detail-body">
                    <div class="local-guardrails">
                        <div>
                            <strong>4 GiB + 20%</strong><span>Available memory required</span>
                        </div>
                        <div><strong>Below 75%</strong><span>CPU before inference</span></div>
                        <div><strong>1.5 GiB</strong><span>Maximum model download</span></div>
                    </div>
                    <p class="muted">
                        Defers under resource or thermal pressure. Models unload immediately after
                        use and are never downloaded automatically.
                    </p>
                    <details class="inline-help">
                        <summary>Install a small model</summary>
                        <p><code>ollama pull qwen3.5:0.8b</code></p>
                        <p>Optional routing model: <code>ollama pull tev1:0.8b</code></p>
                        <p>
                            Chat context is limited to 2,048 tokens and output to 256 tokens.
                            Download size is not peak memory use.
                        </p>
                    </details>
                    <button
                        disabled={!desktop || busy}
                        onclick={() =>
                            perform(async () => {
                                models = await invoke('list_local_models');
                                status =
                                    'Installed model inventory refreshed. No inference was run.';
                            })}>Check Ollama</button
                    >
                    {#each models as model}<div class="local-model">
                            <span
                                >{model.name} · {memory(model.size_bytes)} · {model.eligible
                                    ? 'size eligible'
                                    : 'excluded'}</span
                            ><button
                                disabled={!model.eligible || busy}
                                onclick={() =>
                                    perform(async () => {
                                        status = await invoke('unload_model', {
                                            model: model.name,
                                        });
                                    })}>Unload</button
                            >
                        </div>{/each}
                </div>
            </details>
        </div>
        <div hidden={section !== 'connections'}>
            <section class="preference-card" aria-label="Cloud credentials">
                <div class="preference-heading">
                    <span class="preference-icon"><Icon name="model" size={20} /></span>
                    <div>
                        <h2>Cloud providers</h2>
                        <p>Bring your own key. Stored in your OS credential store.</p>
                    </div>
                </div>
                <div class="provider-statuses">
                    {#each ['meta', 'anthropic', 'jev'] as const as name}
                        <div>
                            <strong>{providerNames[name as Provider]}</strong><span
                                class:configured={bootstrap.credentials.includes(name)}
                                >{bootstrap.credentials.includes(name)
                                    ? 'Key configured'
                                    : 'No key'}</span
                            >
                        </div>
                    {/each}
                </div>
                <details class="inline-help credential-details">
                    <summary>Add or replace a key</summary>
                    <div class="credential-row">
                        <label
                            >Provider<select bind:value={provider}
                                ><option value="meta">Meta</option><option value="anthropic"
                                    >Claude</option
                                ><option value="jev">Jev</option></select
                            ></label
                        ><label
                            >API key<input
                                type="password"
                                autocomplete="off"
                                placeholder="Paste a key"
                                bind:value={credential}
                                disabled={!desktop}
                            /></label
                        ><button onclick={saveKey} disabled={!desktop || busy || !credential}
                            >Store key</button
                        >
                    </div>
                    <p class="muted">
                        Keys stay in the backend. Development .env values take precedence over
                        stored keys.
                    </p>
                </details>
            </section>
        </div>
        <div hidden={section !== 'system'}>
            <SettingsDefaults {desktop} />
            <details class="preference-detail">
                <summary
                    ><span class="detail-name"
                        ><Icon name="info" /> Open-source acknowledgments</span
                    ></summary
                >
                <div class="detail-body"><pre class="license-text">{notices}</pre></div>
            </details>
        </div>
        {#if status && (section === 'models' || section === 'connections')}<p
                class="success"
                role="status"
            >
                {status}
            </p>{/if}
        {#if error && (section === 'models' || section === 'connections')}<p
                class="error"
                role="alert"
            >
                {error}
            </p>{/if}
    </div>
</div>
