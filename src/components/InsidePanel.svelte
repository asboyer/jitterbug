<script lang="ts">
    import { untrack } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage, memory, percent } from '../lib/format';
    import { demoInside } from '../lib/demo';
    import type { InsideUnit, InsideView, ProcessRecord, Snapshot, Workload } from '../lib/types';
    import InfoTip from './InfoTip.svelte';

    let {
        snapshot,
        workload,
        demo,
        actionBusy,
        onaction,
        onview,
    }: {
        snapshot: Snapshot;
        workload: Workload;
        demo: boolean;
        actionBusy: boolean;
        onaction: (process: ProcessRecord, action: string) => void;
        /** Every loaded view, including one only used to name the workload's processes. */
        onview?: (view: InsideView) => void;
    } = $props();
    let view = $state<InsideView | null>(null);
    let loading = $state(false);
    let asking = $state(false);
    let error = $state('');
    let notice = $state('');
    let confirming: string | null = $state(null);
    let tabBusy = $state(false);
    let showAll = $state(false);
    let sleepingOpen = $state(false);
    let sequence = 0;
    let measured = 0;
    // Rows shown before "Show all"; a browser re-measures at most this often by itself.
    const TOP = 8;
    const BROWSER_REFRESH_MS = 30_000;
    // The workload object is replaced every sample; only a different workload resets the view.
    const workloadId = $derived(workload.id);
    const browser = $derived(workload.category === 'browser');
    const noun = $derived(browser ? 'tabs' : 'commands');
    // Tabs without a renderer of their own are discarded (sleeping) or not yet loaded. A tab
    // Jitterbug paused stays in the main list, where it can be resumed.
    const isSleeping = (u: InsideUnit) => u.kind === 'tab' && u.pid === null && !u.paused_until;
    const sleeping = $derived(view?.units.filter(isSleeping) ?? []);
    const active = $derived(view?.units.filter((u) => !isSleeping(u)) ?? []);
    const shown = $derived(showAll ? active : active.slice(0, TOP));

    /** `connect` is only true for the person's own request: it may make the browser ask once. */
    async function load(connect: boolean) {
        const request = ++sequence;
        loading = true;
        asking = connect && browser;
        if (browser) measured = Date.now();
        try {
            const next = demo
                ? demoInside(workload, snapshot)
                : await invoke<InsideView>('get_inside_view', {
                      workloadId: workload.id,
                      connect,
                  });
            if (request !== sequence) return;
            onview?.(next);
            // Without an allowed browser connection, keep offering Show tabs.
            if (connect || !browser || next.status === 'ready') view = next;
            error = '';
        } catch (cause) {
            if (request === sequence && (connect || !browser)) error = errorMessage(cause);
        } finally {
            if (request === sequence) loading = asking = false;
        }
    }

    $effect(() => {
        workloadId;
        untrack(() => {
            view = null;
            loading = false;
            showAll = sleepingOpen = false;
            measured = 0;
        });
        return () => {
            sequence++;
        };
    });

    $effect(() => {
        // Sessions reuse the five-second inventory. A browser is measured automatically only
        // through a connection the person already allowed, so it never asks by itself.
        snapshot.captured_at;
        workloadId;
        untrack(() => {
            if (loading || (browser && Date.now() - measured < BROWSER_REFRESH_MS)) return;
            void load(false);
        });
    });

    async function tab(
        unit: InsideUnit,
        action: 'focus_tab' | 'pause_tab' | 'resume_tab' | 'close_tab'
    ) {
        if (demo || tabBusy) return;
        if (action === 'close_tab' && confirming !== unit.id) {
            confirming = unit.id;
            return;
        }
        confirming = null;
        tabBusy = true;
        try {
            notice = await invoke<string>('tab_action', {
                targetId: unit.id,
                url: unit.detail,
                action,
            });
            if (action === 'close_tab' && view)
                view = { ...view, units: view.units.filter((u) => u.id !== unit.id) };
            if (action === 'pause_tab' || action === 'resume_tab') await load(false);
        } catch (cause) {
            notice = errorMessage(cause);
        } finally {
            tabBusy = false;
        }
    }

    async function session(unit: InsideUnit, action: 'pause' | 'resume') {
        if (demo || tabBusy || unit.pid === null || unit.start_time === null) return;
        tabBusy = true;
        try {
            notice = await invoke<string>('session_control', {
                pid: unit.pid,
                startTime: unit.start_time,
                action,
                label: unit.title,
            });
            await load(false);
        } catch (cause) {
            notice = errorMessage(cause);
        } finally {
            tabBusy = false;
        }
    }
    const until = (time: number) =>
        new Date(time * 1000).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

    function processAction(unit: InsideUnit, action: string) {
        if (unit.pid === null || unit.start_time === null) return;
        onaction(
            {
                pid: unit.pid,
                start_time: unit.start_time,
                parent_pid: null,
                name: unit.host ?? unit.title,
                cpu_percent: unit.cpu_percent,
                memory_bytes: unit.memory_bytes,
                workload_id: workload.id,
                description: '',
                evidence: unit.evidence,
                actionable: true,
                actions: unit.actions,
                disk_read_bytes: 0,
                disk_written_bytes: 0,
            },
            action
        );
    }
</script>

{#snippet row(unit: InsideUnit)}
    <div class="unit" role="listitem">
        <div class="unit-name">
            <strong title={unit.title}>{unit.title}</strong>
            <small title={unit.detail}
                >{unit.kind === 'tab'
                    ? (unit.host ?? unit.detail)
                    : unit.detail || unit.confidence}{unit.agent_status && !unit.paused_until
                    ? ` · ${unit.agent_status}`
                    : ''}{unit.paused_until
                    ? unit.kind === 'tab'
                        ? ` · Paused · resumes ${until(unit.paused_until)}`
                        : ` · paused, resumes by ${until(unit.paused_until)}`
                    : ''}{unit.shared_with
                    ? ` · shares a process with ${unit.shared_with} other tab${unit.shared_with === 1 ? '' : 's'}`
                    : ''}</small
            >
        </div>
        <span class="numeric">{percent(unit.cpu_percent)}<small>CPU</small></span>
        <span class="numeric"
            >{unit.shared_with ? '≈' : ''}{memory(unit.memory_bytes)}<small>Memory</small></span
        >
        <div class="unit-actions">
            <InfoTip label={`${unit.confidence}: how this was measured`}>
                <p><strong>{unit.confidence}</strong></p>
                {#each unit.evidence as line}<p>{line}</p>{/each}
            </InfoTip>
            {#if unit.kind === 'tab'}
                {#if unit.actions.includes('resume_tab')}<button
                        class="primary"
                        disabled={demo || tabBusy}
                        onclick={() => tab(unit, 'resume_tab')}>Resume</button
                    >{:else}<button
                        disabled={demo || tabBusy}
                        onclick={() => tab(unit, 'focus_tab')}>Show</button
                    >{/if}{#if unit.actions.includes('pause_tab')}<button
                        disabled={demo || tabBusy}
                        title="Freeze this tab's scripts and timers. Nothing is closed; it resumes by itself after 15 minutes. Its memory stays allocated."
                        onclick={() => tab(unit, 'pause_tab')}>Pause</button
                    >{/if}<button
                    class="danger-text"
                    disabled={demo || tabBusy}
                    onclick={() => tab(unit, 'close_tab')}
                    >{confirming === unit.id ? 'Confirm close' : 'Close tab…'}</button
                >
            {:else}
                {#each unit.actions.filter((a) => a === 'pause' || a === 'resume') as action}<button
                        class:primary={action === 'resume'}
                        disabled={demo || tabBusy}
                        title={action === 'pause'
                            ? 'Freeze this command and everything it started. Nothing is closed; it resumes by itself after 15 minutes.'
                            : ''}
                        onclick={() => session(unit, action as 'pause' | 'resume')}
                        >{action === 'pause' ? 'Pause' : 'Resume'}</button
                    >{/each}
                {#each unit.actions.filter((a) => a !== 'pause' && a !== 'resume') as action}<button
                        class:danger-text={action === 'terminate'}
                        disabled={demo || actionBusy}
                        onclick={() => processAction(unit, action)}
                        >{action === 'terminate' ? 'Stop…' : 'Lower priority…'}</button
                    >{/each}
            {/if}
        </div>
    </div>
{/snippet}

{#if !(view?.status === 'unavailable' && view.source === 'None')}
    <section class="panel inside" aria-label={`Inside ${workload.label}`}>
        <div class="section-title">
            <div>
                <h2>Inside {workload.label}</h2>
                <span class="muted"
                    >{view?.status === 'ready'
                        ? view.message
                        : browser
                          ? 'See which tabs use the CPU and memory, then pause or close one.'
                          : 'Agent sessions, dev servers, and other commands started in this app.'}</span
                >
            </div>
            {#if browser || view?.status !== 'ready'}<button
                    class:primary={!view}
                    disabled={loading}
                    onclick={() => load(true)}
                    >{asking ? `Measuring ${noun}…` : view ? 'Refresh' : `Show ${noun}`}</button
                >{/if}
        </div>
        {#if asking && !view}<p class="muted">
                Measuring for about a second. If the browser asks to allow a debugging connection,
                choose Allow.
            </p>{/if}
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        {#if notice}<p class="notice" role="status">{notice}</p>{/if}
        {#if view && view.status !== 'ready'}
            <p>{view.message}</p>
            {#if view.setup.length}<ol class="setup">
                    {#each view.setup as step}<li>{step}</li>{/each}
                </ol>{/if}
        {:else if view}
            <div class="units" role="list">
                {#each shown as unit (unit.id)}
                    {@render row(unit)}
                {:else}{#if !sleeping.length}<p class="muted empty">Nothing to show.</p>{/if}{/each}
                {#if active.length > TOP}<div class="more" role="listitem">
                        <button class="text-button" onclick={() => (showAll = !showAll)}
                            >{showAll ? 'Show fewer' : `Show all ${active.length}`}</button
                        >
                    </div>{/if}
                {#if sleeping.length}
                    <div class="unit remainder" role="listitem">
                        <div class="unit-name">
                            <strong
                                >{sleeping.length} sleeping tab{sleeping.length === 1
                                    ? ''
                                    : 's'}</strong
                            >
                            <small>No process of their own; they reload when you open them</small>
                        </div>
                        <span class="numeric"
                            >{percent(sleeping.reduce((sum, u) => sum + u.cpu_percent, 0))}<small
                                >CPU</small
                            ></span
                        >
                        <span class="numeric"
                            >{memory(sleeping.reduce((sum, u) => sum + u.memory_bytes, 0))}<small
                                >Memory</small
                            ></span
                        >
                        <div class="unit-actions">
                            <button
                                class="text-button"
                                aria-expanded={sleepingOpen}
                                onclick={() => (sleepingOpen = !sleepingOpen)}
                                >{sleepingOpen ? 'Hide' : 'List'}</button
                            >
                        </div>
                    </div>
                    {#if sleepingOpen}{#each sleeping as unit (unit.id)}{@render row(
                                unit
                            )}{/each}{/if}
                {/if}
                {#each view.remainder as rest}
                    <div class="unit remainder" role="listitem">
                        <div class="unit-name">
                            <strong>{rest.label}</strong>
                            <small
                                >{rest.process_count} process{rest.process_count === 1
                                    ? ''
                                    : 'es'}</small
                            >
                        </div>
                        <span class="numeric">{percent(rest.cpu_percent)}<small>CPU</small></span>
                        <span class="numeric">{memory(rest.memory_bytes)}<small>Memory</small></span
                        >
                    </div>
                {/each}
            </div>
            <details class="reading-notes">
                <summary>{demo ? 'Synthetic example · ' : ''}About these readings</summary>
                <p class="footnote">{view.limits}</p>
            </details>
            {#if view.units.some((u) => u.kind === 'tab')}
                <p class="footnote">Closing a tab discards unsaved form input in it.</p>
            {/if}
        {/if}
    </section>
{/if}

<style>
    .inside {
        margin-bottom: 18px;
        padding: 18px 20px;
    }
    .section-title {
        flex-wrap: wrap;
        gap: 10px;
    }
    .setup {
        margin: 8px 0 0;
        padding-left: 20px;
        line-height: 1.7;
    }
    .notice {
        font-size: 12px;
    }
    .units {
        margin-top: 12px;
    }
    .unit {
        display: grid;
        grid-template-columns: minmax(140px, 1fr) 70px 90px auto;
        align-items: center;
        gap: 4px 14px;
        padding: 10px 8px;
        border-radius: 6px;
    }
    .unit:hover {
        background: var(--hover);
    }
    .unit-name {
        min-width: 0;
    }
    .unit-name strong {
        display: block;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-weight: 500;
    }
    .unit small {
        display: block;
        color: var(--muted);
        font-size: 10px;
        margin-top: 3px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .unit-actions {
        display: flex;
        align-items: center;
        gap: 6px;
        justify-content: flex-end;
    }
    .unit-actions :global(.info-button) {
        width: 22px;
        height: 22px;
        opacity: 0.6;
    }
    .more {
        padding: 4px 8px;
    }
    .remainder strong {
        color: var(--muted);
    }
    .empty {
        padding: 8px;
    }
    .reading-notes {
        font-size: 11px;
        color: var(--muted);
        margin-top: 12px;
    }
    .reading-notes summary {
        cursor: pointer;
    }
    .footnote {
        margin: 8px 0 0;
    }
</style>
