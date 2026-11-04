<script module lang="ts">
    let savedOverview: { reply: Reply; key: string; settings: string; at: number } | null = null;
    let lastAutomatic = 0;
    let lastAttemptKey = '';
</script>

<script lang="ts">
    import { onDestroy, untrack, tick } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import type { AutomationCase, Snapshot, Workload, ProcessRecord, Reply } from '../lib/types';
    import { overviewKey, demoOverview, queueInference, cancelled } from '../lib/monitor';
    import { errorMessage } from '../lib/format';
    import Dashboard from './Dashboard.svelte';
    import Workloads from './Workloads.svelte';
    import WorkloadDetails from './WorkloadDetails.svelte';
    import InsightCard from './InsightCard.svelte';
    import Icon from './Icon.svelte';
    import AiActivityLog from './AiActivityLog.svelte';
    let {
        snapshot,
        selected,
        settingsKey,
        provider,
        actionBusy,
        assistantOpen = false,
        cases = [],
        onselect,
        onclose,
        onask,
        onprocessask,
        onaction,
        onusage,
    }: {
        snapshot: Snapshot;
        selected: Workload | null;
        settingsKey: string;
        provider: string;
        actionBusy: boolean;
        assistantOpen?: boolean;
        /** Incidents, so an investigation in AI activity can show what happened next. */
        cases?: AutomationCase[];
        onselect: (workload: Workload) => void;
        onclose: () => void;
        onask: () => void;
        onprocessask: (process: ProcessRecord | null, reply?: Reply | null) => void;
        onaction: (process: ProcessRecord, action: string) => void;
        onusage: () => void;
    } = $props();
    let process = $state<ProcessRecord | null>(null);
    let reply = $state<Reply | null>(null);
    let busy = $state(false);
    let error = $state('');
    let generatedAt = $state(0);
    let resultKey = $state('');
    let disposed = false;
    let inspector: HTMLElement | undefined = $state();
    let lastSelection: HTMLButtonElement | null = null;
    const signature = $derived(overviewKey(snapshot));
    const demo = $derived(snapshot.source === 'demo');
    const changed = $derived(!!reply && resultKey !== signature);
    onDestroy(() => {
        disposed = true;
    });
    $effect(() => {
        const source = snapshot.source;
        const settings = settingsKey;
        untrack(() => {
            const saved = savedOverview;
            if (saved?.settings === `${source}:${settings}`) {
                reply = saved.reply;
                generatedAt = saved.at;
                resultKey = saved.key;
            } else {
                reply = null;
                error = '';
                resultKey = '';
            }
        });
    });
    $effect(() => {
        const key = `${settingsKey}:${signature}`;
        snapshot.captured_at;
        if (!snapshot.sample_ready || selected || busy || document.hidden) return;
        if (
            key === lastAttemptKey ||
            (lastAttemptKey.startsWith(`${settingsKey}:`) && Date.now() - lastAutomatic < 60_000)
        )
            return;
        const timer = setTimeout(() => {
            void summarize(false);
        }, 600);
        return () => clearTimeout(timer);
    });
    async function summarize(manual = true) {
        if (busy || !snapshot.sample_ready) return;
        const requestKey = signature;
        const settings = settingsKey;
        const source = snapshot.source;
        lastAttemptKey = `${settings}:${requestKey}`;
        lastAutomatic = Date.now();
        busy = true;
        error = '';
        const valid = () =>
            !disposed &&
            source === snapshot.source &&
            settings === settingsKey &&
            (manual || !selected);
        try {
            const result = demo
                ? demoOverview(snapshot)
                : await queueInference<Reply>(
                      () => invoke('summarize_monitor'),
                      manual ? 5 : 0,
                      valid
                  );
            if (!demo) onusage();
            if (!valid()) return;
            reply = result;
            generatedAt = Date.now();
            resultKey = requestKey;
            savedOverview = {
                reply: result,
                key: requestKey,
                settings: `${source}:${settings}`,
                at: generatedAt,
            };
        } catch (cause) {
            if (cause === cancelled) {
                lastAttemptKey = '';
                lastAutomatic = 0;
            }
            if (valid() && cause !== cancelled) error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
    $effect(() => {
        if (process && process.workload_id !== selected?.id) process = null;
    });
    function inspect(workload: Workload, target?: ProcessRecord | null) {
        lastSelection =
            document.activeElement instanceof HTMLButtonElement ? document.activeElement : null;
        process = target ?? null;
        onselect(workload);
        void tick().then(() => inspector?.focus({ preventScroll: true }));
    }
    function close() {
        onclose();
        process = null;
        lastSelection?.focus({ preventScroll: true });
    }
</script>

<div class="monitor-page">
    <header class="monitor-heading">
        <div>
            <span class="eyebrow">YOUR COMPUTER, EXPLAINED</span>
            <h1>Monitor</h1>
            <p>See what’s running. Understand what it’s doing.</p>
        </div>
        <button onclick={onask}><Icon name="companion" size={16} /> Ask Jitterbug</button>
    </header>
    <div class="monitor-overview" class:condensed={!!selected && !assistantOpen}>
        <Dashboard {snapshot} />
        <section
            class="monitor-summary panel"
            aria-label="Computer explanation"
            data-tour="monitor-summary"
            aria-busy={busy}
        >
            <div class="summary-heading">
                <span
                    ><Icon name="companion" size={19} /><strong
                        >{demo ? 'Demo overview' : `${provider} · computer overview`}</strong
                    ></span
                ><button
                    class="text-button"
                    disabled={busy || !snapshot.sample_ready}
                    onclick={() => summarize()}
                    >{busy ? 'Reading activity…' : 'Refresh explanation'}</button
                >
            </div>
            {#if reply}<InsightCard {reply} />
                <p class="summary-status">
                    As of {new Date(generatedAt).toLocaleTimeString([], {
                        hour: 'numeric',
                        minute: '2-digit',
                    })}{changed
                        ? ' · Activity has changed since this finding.'
                        : ' · Reused until activity meaningfully changes.'}
                </p>
            {:else}<h2>
                    {busy
                        ? 'Making sense of your computer’s activity…'
                        : 'Your activity, in plain English.'}
                </h2>
                <p class="muted">
                    {busy
                        ? 'Connecting resource measurements, recent changes, and the busiest processes.'
                        : 'AI explains the measurements and suggests where to look next.'}
                </p>{/if}
            {#if error}<p class="error" role="status">AI overview unavailable: {error}</p>{/if}
        </section>
    </div>
    {#if !(selected && !assistantOpen)}<AiActivityLog live={!demo} {cases} />{/if}
    <div class="monitor-workspace" class:inspecting={!!selected && !assistantOpen}>
        <Workloads
            {snapshot}
            selectedId={selected?.id ?? null}
            {settingsKey}
            paused={!!selected}
            onselect={inspect}
            {onusage}
        />
        {#if selected}<div
                class="monitor-inspector panel"
                hidden={assistantOpen}
                role="dialog"
                aria-label="Process inspector"
                bind:this={inspector}
                tabindex="-1"
                onkeydown={(event) => {
                    if (event.key === 'Escape' && !(event.target instanceof HTMLInputElement))
                        close();
                }}
            >
                {#key `${snapshot.source}:${selected.id}:${process?.pid ?? ''}:${process?.start_time ?? ''}`}<WorkloadDetails
                        {snapshot}
                        workload={selected}
                        embedded
                        initialProcess={process}
                        {settingsKey}
                        {actionBusy}
                        onback={close}
                        onask={onprocessask}
                        {onaction}
                        {onusage}
                        onmanage={() => {}}
                    />{/key}
            </div>{/if}
    </div>
</div>

<style>
    .monitor-overview {
        display: grid;
        grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
        gap: 18px;
        margin-bottom: 20px;
        align-items: stretch;
    }
    .monitor-overview.condensed {
        display: none;
    }
    .monitor-overview :global(.dashboard) {
        min-width: 0;
    }
    .monitor-summary :global(.insight-card h3) {
        font-size: 16px;
    }
    .monitor-summary :global(.insight-card .next-step) {
        padding: 10px;
    }
    .monitor-summary :global(.insight-card p) {
        font-size: 12px;
    }
    .monitor-summary :global(.insight-label) {
        margin-bottom: 10px;
    }
    .monitor-page {
        /* A transformed entry animation would trap the fixed inspector inside the scroller. */
        animation: none !important;
        max-width: 1440px;
        margin: 0 auto;
    }
    .monitor-heading {
        display: flex;
        justify-content: space-between;
        gap: 16px;
        align-items: center;
        margin-bottom: 24px;
    }
    .monitor-heading h1 {
        font-size: 28px;
        margin: 6px 0;
        letter-spacing: -0.7px;
    }
    .monitor-heading p {
        margin: 0;
        font-size: 13px;
        color: var(--muted);
    }
    .monitor-heading button,
    .summary-heading span {
        display: flex;
        align-items: center;
        gap: 8px;
    }
    .monitor-summary {
        padding: 18px 20px;
        margin: 0;
        border-left: 3px solid var(--accent);
    }
    .summary-heading {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        flex-wrap: wrap;
        margin-bottom: 16px;
    }
    .summary-heading span {
        font-size: 12px;
        color: var(--accent);
    }
    .summary-heading button {
        font-size: 11px;
    }
    .summary-status {
        color: var(--muted);
        font-size: 10px;
        margin: 14px 0 0;
    }
    .monitor-workspace {
        display: grid;
        gap: 18px;
        align-items: start;
    }
    .monitor-workspace.inspecting {
        grid-template-columns: minmax(360px, 1fr) minmax(380px, 0.95fr);
    }
    .monitor-inspector {
        padding: 20px;
        background: linear-gradient(var(--panel), var(--panel)), var(--canvas);
        position: sticky;
        top: 16px;
        max-height: calc(100vh - 100px);
        overflow-y: auto;
        min-width: 0;
    }
    .monitor-workspace :global(.workload-panel) {
        min-width: 0;
    }
    @media (max-width: 950px) {
        .monitor-overview {
            grid-template-columns: minmax(0, 1fr);
        }
    }
    @media (max-width: 1200px) {
        .monitor-workspace.inspecting {
            grid-template-columns: minmax(0, 1fr);
        }
        .monitor-inspector {
            position: fixed;
            top: 70px;
            right: 20px;
            width: min(560px, calc(100vw - 200px));
            max-height: calc(100vh - 95px);
            z-index: 20;
            box-shadow: 0 12px 48px #0006;
            border-color: var(--control-border);
        }
    }
</style>
