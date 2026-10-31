<script lang="ts">
    import { onMount, untrack } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import Icon from './Icon.svelte';
    import SettingSwitch from './SettingSwitch.svelte';
    import { errorMessage } from '../lib/format';
    import type { AutomationPolicy, Bootstrap, JitterTally } from '../lib/types';
    let {
        bootstrap,
        desktop,
        onrefresh,
        section,
        visible,
    }: {
        bootstrap: Bootstrap;
        desktop: boolean;
        onrefresh: () => Promise<void>;
        section: 'assistance' | 'connections';
        visible: boolean;
    } = $props();
    // App priorities save on their own (`move_to_bucket`), so they never make the form dirty.
    const comparable = (p: AutomationPolicy) => JSON.stringify({ ...p, buckets: undefined });
    let policy = $state(untrack(() => $state.snapshot(bootstrap.settings.automation)));
    let savedPolicy = $state(untrack(() => comparable(bootstrap.settings.automation)));
    let dirty = $derived(comparable(policy) !== savedPolicy);
    let busy = $state(false);
    let status = $state('');
    let error = $state('');
    let tally: JitterTally | null = $state(null);
    const shortcut = navigator.userAgent.includes('Mac') ? '⌘⇧J' : 'Ctrl+Shift+J';
    async function loadTally() {
        if (!desktop) return;
        try {
            tally = await invoke<JitterTally>('get_jitter_tally');
        } catch (cause) {
            error = errorMessage(cause);
        }
    }
    onMount(() => {
        void loadTally();
    });
    async function clearTally() {
        if (!desktop) return;
        try {
            await invoke('clear_jitter_tally');
            await loadTally();
            status = 'Snapshot history cleared.';
        } catch (cause) {
            error = errorMessage(cause);
        }
    }
    async function save() {
        if (!desktop) {
            status = 'Preview only. No automation policy was changed.';
            return;
        }
        const submitted = $state.snapshot(policy);
        busy = true;
        error = '';
        try {
            await invoke('save_automation_policy', { policy: submitted });
            await onrefresh();
            await loadTally();
            savedPolicy = comparable(submitted);
            status = 'Preferences saved.';
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
    async function moveToNormal(workloadId: string) {
        if (!desktop) return;
        try {
            status = await invoke<string>('move_to_bucket', { workloadId, bucket: 'normal' });
            policy.buckets = policy.buckets.filter((b) => b.workload_id !== workloadId);
            await onrefresh();
        } catch (cause) {
            error = errorMessage(cause);
        }
    }
    async function resume() {
        try {
            await invoke('snooze_automation', { minutes: 0 });
            await onrefresh();
        } catch (cause) {
            error = errorMessage(cause);
        }
    }
</script>

<div hidden={!visible} class="automation-settings">
    {#if section === 'assistance'}
        <section class="preference-card" aria-label="Automatic assistance policy">
            <div class="preference-heading">
                <span class="preference-icon"><Icon name="activity" size={20} /></span>
                <div>
                    <h2>Stay ahead of slowdowns</h2>
                    <p>Choose when Jitterbug steps in.</p>
                </div>
            </div>
            <SettingSwitch
                title="Watch for slowdowns"
                description="Detect sustained pressure and investigate in the background."
                bind:checked={policy.enabled}
            />
            <SettingSwitch
                title="Pause agents when the computer is strained"
                description="When memory or CPU stays tight, pause the busiest working agent session and resume it once there is room. Sessions waiting for you, and the last one working, keep running."
                bind:checked={policy.hold_agents}
                disabled={!policy.enabled}
            />
            <SettingSwitch
                title="Investigate with AI"
                description="Use your intelligent model when a problem persists. Cloud costs may apply."
                bind:checked={policy.model_analysis}
            />
            <SettingSwitch
                title="Show action popups"
                description="Bring suggestions to you while you work in other apps."
                bind:checked={policy.action_popups}
            />
        </section>
        <div class="preference-disclosures">
            <details class="preference-detail">
                <summary>
                    <span class="detail-name"><Icon name="activity" /> Jitter Snapshot</span>
                    <span class="detail-value"
                        >{policy.jitter_mode === 'auto' ? '60-second look-back' : 'On demand'} · {shortcut}</span
                    >
                </summary>
                <div class="detail-body">
                    <p class="muted">
                        Press <kbd>{shortcut}</kbd> when something feels slow. Review what changed before
                        taking action.
                    </p>
                    <fieldset class="snapshot-modes">
                        <legend>Capture mode</legend>
                        <label class:active={policy.jitter_mode === 'auto'}>
                            <input type="radio" bind:group={policy.jitter_mode} value="auto" />
                            <span
                                ><strong>Look back</strong><small
                                    >Keep the preceding 60 seconds available.</small
                                ></span
                            >
                        </label>
                        <label class:active={policy.jitter_mode === 'snapshot_only'}>
                            <input
                                type="radio"
                                bind:group={policy.jitter_mode}
                                value="snapshot_only"
                            />
                            <span
                                ><strong>Only when I ask</strong><small
                                    >Record the next 15 seconds. No background sampler.</small
                                ></span
                            >
                        </label>
                    </fieldset>
                    <SettingSwitch
                        title="Take snapshots automatically"
                        description="Capture short hitches as well as sustained incidents. Requires look-back mode."
                        bind:checked={policy.auto_snapshots}
                        disabled={policy.jitter_mode !== 'auto'}
                    />
                    <details class="inline-help">
                        <summary>Sampling & history</summary>
                        <p>
                            Look-back sampling keeps 120 one-second readings in memory.
                            Snapshot-only mode cannot recover what happened before you pressed. {#if tally}The
                                sampler is {tally.sampling ? 'running' : 'stopped'}.{/if}
                        </p>
                        <p>
                            Only workload names and counts are saved privately on this device. A
                            workload appearing in three of your last six snapshots can offer a
                            priority permission.
                        </p>
                        {#each tally?.entries.slice(0, 8) ?? [] as entry}
                            <div class="local-model">
                                <span>{entry.label}</span><span class="muted"
                                    >{entry.count} snapshots</span
                                >
                            </div>
                        {:else}
                            <p class="muted">No workloads in snapshot history.</p>
                        {/each}
                        <button disabled={!desktop || !tally?.entries.length} onclick={clearTally}
                            >Clear snapshot history</button
                        >
                    </details>
                </div>
            </details>
            <details class="preference-detail">
                <summary>
                    <span class="detail-name"><Icon name="settings" /> App priorities</span>
                    <span class="detail-value"
                        >{policy.buckets.length
                            ? `${policy.buckets.length} not Normal`
                            : 'All Normal'}</span
                    >
                </summary>
                <div class="detail-body">
                    <p class="muted">
                        Approving a priority suggestion moves an app to Background. Protected apps
                        are never offered for changes or agent requests.
                    </p>
                    {#each policy.buckets as entry (entry.workload_id)}
                        <div class="local-model">
                            <span
                                >{entry.label} · {entry.bucket === 'background'
                                    ? 'Background'
                                    : 'Protected'}</span
                            ><button onclick={() => moveToNormal(entry.workload_id)}
                                >Move to Normal</button
                            >
                        </div>
                    {:else}
                        <p class="muted">Every app is Normal.</p>
                    {/each}
                    <p class="muted">
                        Background mode gives other apps CPU and disk first. It does not cap CPU or
                        free memory, and the app in front always runs at full speed.
                    </p>
                </div>
            </details>
            <details class="preference-detail">
                <summary>
                    <span class="detail-name"><Icon name="cpu" /> Thresholds & privacy</span>
                    <span class="detail-value"
                        >{policy.system_cpu_percent}% CPU · {policy.sustained_samples * 5}s
                        sustained</span
                    >
                </summary>
                <div class="detail-body">
                    <p class="muted">
                        Ordinary wake-ups and memory growth alone stay quiet. On macOS, an
                        unresponsive app can alert even at low CPU.
                    </p>
                    <div class="automation-fields">
                        <label
                            >Whole-machine CPU threshold (%)<input
                                type="number"
                                min="50"
                                max="100"
                                bind:value={policy.system_cpu_percent}
                            /></label
                        >
                        <label
                            >Memory fallback threshold (%)<input
                                type="number"
                                min="5"
                                max="30"
                                bind:value={policy.available_memory_percent}
                            /></label
                        >
                        <label
                            >Minimum app CPU (% of total capacity)<input
                                type="number"
                                min="5"
                                max="100"
                                bind:value={policy.app_cpu_percent}
                            /></label
                        >
                        <label
                            >App CPU threshold (100% = one core)<input
                                type="number"
                                min="50"
                                max="1600"
                                bind:value={policy.app_cpu_core_percent}
                            /></label
                        >
                        <label
                            >Consecutive samples (5s each)<input
                                type="number"
                                min="3"
                                max="12"
                                bind:value={policy.sustained_samples}
                            /></label
                        >
                        <label
                            >Cooldown per trigger (seconds)<input
                                type="number"
                                min="60"
                                max="3600"
                                bind:value={policy.cooldown_seconds}
                            /></label
                        >
                        <label
                            >Investigations per hour (this session)<input
                                type="number"
                                min="1"
                                max="12"
                                bind:value={policy.max_investigations_per_hour}
                            /></label
                        >
                    </div>
                    <details class="inline-help">
                        <summary>How alerts are filtered</summary>
                        <p>
                            Unusual CPU activity must exceed the app minimum, reach 3× its recent
                            baseline, and rise by five percentage points of total capacity. Repeated
                            alerts wait for recovery, cooldown, and a global 60-second gap.
                        </p>
                        <p>
                            Memory alerts need 30 seconds of OS memory pressure or swap growth above
                            256 MiB per sample. Without a native pressure reading, available memory
                            must fall below both the fallback threshold and 2 GiB. Critical OS
                            pressure uses a 10-second minimum. Your consecutive-sample limit also
                            applies.
                        </p>
                        <p>
                            Memory alerts cover the whole computer. Another alert requires a minute
                            of recovery and at least 15 minutes since the last. Incident history
                            stays in memory.
                        </p>
                    </details>
                    <details class="inline-help">
                        <summary>What is sent to a model</summary>
                        <p>
                            Selected app/process names and resource history. Paths, environment
                            variables, transcripts and raw stack samples stay local. Your AI mode,
                            spending ceiling and local resource limits always apply.
                        </p>
                    </details>
                </div>
            </details>
        </div>
    {:else}
        <section class="preference-card" aria-label="Agent access">
            <div class="preference-heading">
                <span class="preference-icon"><Icon name="code" size={20} /></span>
                <div>
                    <h2>Connected agents</h2>
                    <p>Give agents a view of the work on this computer.</p>
                </div>
            </div>
            <SettingSwitch
                title="Allow agent access"
                description="Let local agents inspect apps and request session pauses or tab closures."
                bind:checked={policy.agent_control}
            />
            <p class="consent-note">
                <Icon name="info" size={15} /> App labels, folder names and tab titles may reach the agent’s
                model provider.
            </p>
            <SettingSwitch
                title="Allow pauses without asking"
                description="Agents may pause sessions automatically. Paused sessions resume after 15 minutes."
                bind:checked={policy.agent_pause_without_asking}
                disabled={!policy.agent_control}
            />
            <p class="consent-note">
                Closing a tab always requires approval. Pausing stops CPU work; it keeps memory
                allocated.
            </p>
            <details class="inline-help">
                <summary>Connection setup & permissions</summary>
                <p>
                    Connect with <code>jitterbug-run control-mcp</code> on macOS or Linux. Windows control
                    is not available. Any process running as your OS user can connect when access is enabled.
                </p>
                <p>
                    Agents receive bounded app, session and tab labels, folder basenames and tab
                    hosts. Full paths, URLs, command lines and page content are excluded.
                </p>
                <p>
                    An agent cannot pause its own session. Snoozing blocks automatic pauses;
                    requests still need your saved permissions. Agents can resume only pauses
                    requested by that agent. Pauses keep RAM allocated and expire after 15 minutes.
                </p>
            </details>
        </section>
    {/if}
    <div class="preference-footer">
        <span class="muted">{dirty ? 'Unsaved changes' : 'Changes apply when you save.'}</span>
        <button class="primary" disabled={busy} onclick={save}
            >{busy ? 'Saving…' : 'Save preferences'}</button
        >
    </div>
    {#if bootstrap.automation_snoozed_until > Date.now() / 1000}
        <p class="muted">
            Paused until {new Date(bootstrap.automation_snoozed_until * 1000).toLocaleTimeString()}
            <button disabled={!desktop} onclick={resume}>Resume now</button>
        </p>
    {/if}
    {#if status}<p class="success" role="status">{status}</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

<style>
    .automation-settings[hidden] {
        display: none;
    }
    .snapshot-modes {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 12px;
        border: 0;
        padding: 0;
        margin: 16px 0 0;
    }
    .snapshot-modes legend {
        margin-bottom: 10px;
        color: var(--muted);
        font-size: 12px;
    }
    .snapshot-modes label {
        display: flex;
        align-items: flex-start;
        gap: 8px;
        padding: 12px;
        border: 1px solid var(--border);
        border-radius: 8px;
        cursor: pointer;
    }
    .snapshot-modes label.active {
        background: var(--accent-soft);
        border-color: var(--accent);
    }
    .snapshot-modes label span {
        display: grid;
        gap: 6px;
    }
    .snapshot-modes small {
        color: var(--muted);
        line-height: 1.5;
    }
    .consent-note {
        display: flex;
        align-items: flex-start;
        gap: 8px;
        color: var(--muted);
        font-size: 12px;
        line-height: 1.5;
        margin: 0 0 12px;
    }
    .consent-note :global(svg) {
        flex-shrink: 0;
        margin-top: 2px;
    }
    @media (max-width: 680px) {
        .snapshot-modes {
            grid-template-columns: 1fr;
        }
    }
</style>
