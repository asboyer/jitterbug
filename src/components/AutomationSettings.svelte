<script lang="ts">
    import { untrack } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage } from '../lib/format';
    import type { Bootstrap } from '../lib/types';
    let {
        bootstrap,
        desktop,
        onrefresh,
    }: { bootstrap: Bootstrap; desktop: boolean; onrefresh: () => Promise<void> } = $props();
    let policy = $state(untrack(() => $state.snapshot(bootstrap.settings.automation)));
    let busy = $state(false);
    let status = $state('');
    let error = $state('');
    async function save() {
        if (!desktop) {
            status = 'Preview only. No automation policy was changed.';
            return;
        }
        busy = true;
        error = '';
        try {
            await invoke('save_automation_policy', { policy });
            await onrefresh();
            status =
                'Automation policy saved. Permissions are rechecked immediately before every action.';
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
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

<section class="panel settings-panel" aria-label="Automatic assistance policy">
    <h2>Automatic assistance</h2>
    <label class="toggle"
        ><input type="checkbox" bind:checked={policy.enabled} /> Detect and investigate resource incidents
        in the background</label
    >
    <label class="toggle"
        ><input type="checkbox" bind:checked={policy.model_analysis} /> Ask the intelligent model automatically
        after a sustained incident</label
    >
    <p class="muted">
        Investigations may use your cloud provider. Process changes need your permission.
    </p>
    <details class="policy-details">
        <summary>Alert thresholds & privacy</summary>
        <p class="muted">
            Alerts need a sustained problem. Ordinary app wake-ups and memory growth alone stay
            quiet. A macOS app that stops responding can alert even at low CPU.
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
        <p class="muted">
            Unusual CPU activity must exceed the app minimum, reach 3× its recent baseline, and rise
            by five percentage points of total capacity. Repeated alerts wait for recovery,
            cooldown, and a global 60-second gap. Incident history is kept only in memory.
        </p>
        <details>
            <summary>When memory alerts appear</summary>
            <p class="muted">
                App memory growth alone stays quiet. Alerts need 30 seconds of OS memory pressure or
                swap growth above 256 MiB per sample. Without a native pressure reading, available
                memory must fall below both the fallback threshold and 2 GiB. Critical OS pressure
                uses the faster 10-second minimum. Your consecutive-sample limit also applies.
            </p>
            <p class="muted">
                Memory alerts cover the whole computer, with growing apps included as context.
                Another alert requires a full minute of recovery and at least 15 minutes since the
                last one.
            </p>
        </details>
        <p class="muted">
            Models receive selected app/process names and resource history. Paths, environment
            variables, transcripts and raw stack samples stay local. Your mode, spending ceiling and
            local model resource limits apply.
        </p>
    </details>
    <h3>Workloads allowed to change priority automatically</h3>
    <p>Allow priority changes from a suggestion. Jitterbug never stops an app automatically.</p>
    {#each policy.rules as rule}
        <div class="local-model">
            <span>{rule.label} · lower priority only</span><button
                onclick={() => {
                    policy.rules = policy.rules.filter((r) => r.workload_id !== rule.workload_id);
                }}>Remove permission</button
            >
        </div>
    {/each}
    {#if !policy.rules.length}<p class="muted">
            No automatic process changes are allowed yet.
        </p>{/if}
    <p class="footnote">
        Lower priority helps competing work; it is not a CPU cap or a memory fix. Restoring priority
        can require OS privileges, so Jitterbug does not promise automatic rollback.
    </p>
    {#if bootstrap.automation_snoozed_until > Date.now() / 1000}<p>
            Paused until {new Date(bootstrap.automation_snoozed_until * 1000).toLocaleTimeString()}
            <button disabled={!desktop} onclick={resume}>Resume now</button>
        </p>{/if}
    <button class="primary" disabled={busy} onclick={save}>Save automation policy</button>
    {#if status}<p class="success" role="status">{status}</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
</section>

<style>
    .policy-details {
        margin: 18px 0 22px;
    }
    .policy-details > summary {
        padding: 6px 0;
        font-weight: 500;
    }
    .policy-details[open] > summary {
        margin-bottom: 12px;
    }
</style>
