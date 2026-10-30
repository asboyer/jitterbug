<script lang="ts">
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage, memory, percent } from '../lib/format';
    import { demoInside } from '../lib/demo';
    import type { InsideUnit, InsideView, ProcessRecord, Snapshot, Workload } from '../lib/types';

    let {
        snapshot,
        workload,
        demo,
        actionBusy,
        onaction,
    }: {
        snapshot: Snapshot;
        workload: Workload;
        demo: boolean;
        actionBusy: boolean;
        onaction: (process: ProcessRecord, action: string) => void;
    } = $props();
    let view: InsideView | null = $state(null);
    let loading = $state(false);
    let error = $state('');
    let notice = $state('');
    let confirming: string | null = $state(null);
    let tabBusy = $state(false);
    let sequence = 0;
    // A browser view samples for a second and may ask the browser for permission: on request only.
    const browser = $derived(workload.category === 'browser');
    const noun = $derived(browser ? 'tabs' : 'commands');

    async function load() {
        const request = ++sequence;
        loading = true;
        try {
            const next = demo
                ? demoInside(workload)
                : await invoke<InsideView>('get_inside_view', { workloadId: workload.id });
            if (request !== sequence) return;
            view = next;
            error = '';
        } catch (cause) {
            if (request === sequence) error = errorMessage(cause);
        } finally {
            if (request === sequence) loading = false;
        }
    }

    $effect(() => {
        // Sessions reuse the five-second inventory; browsers refresh only when asked.
        workload.id;
        if (!browser) {
            snapshot.captured_at;
            void load();
        }
        return () => {
            sequence++;
        };
    });

    async function tab(unit: InsideUnit, action: 'focus_tab' | 'close_tab') {
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
        } catch (cause) {
            notice = errorMessage(cause);
        } finally {
            tabBusy = false;
        }
    }

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

{#if !(view?.status === 'unavailable' && view.source === 'None')}
    <section class="panel inside" aria-label={`Inside ${workload.label}`}>
        <div class="section-title">
            <div>
                <h2>Inside {workload.label}</h2>
                <span class="muted"
                    >{view?.status === 'ready'
                        ? view.message
                        : browser
                          ? 'See which tabs use the CPU and memory, then focus or close one.'
                          : 'Agent sessions, dev servers, and other commands started in this app.'}</span
                >
            </div>
            {#if browser || view?.status !== 'ready'}<button
                    class:primary={!view}
                    disabled={loading}
                    onclick={load}
                    >{loading ? `Measuring ${noun}…` : view ? 'Refresh' : `Show ${noun}`}</button
                >{/if}
        </div>
        {#if loading && browser && !view}<p class="muted">
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
                {#each view.units as unit (unit.id)}
                    <div class="unit" role="listitem">
                        <div class="unit-name">
                            <strong title={unit.title}>{unit.title}</strong>
                            <small title={unit.detail}
                                >{unit.kind === 'tab'
                                    ? (unit.host ?? unit.detail)
                                    : unit.detail || unit.confidence}{unit.shared_with
                                    ? ` · shares a process with ${unit.shared_with} other tab${unit.shared_with === 1 ? '' : 's'}`
                                    : ''}</small
                            >
                        </div>
                        <span class="numeric">{percent(unit.cpu_percent)}<small>CPU</small></span>
                        <span class="numeric"
                            >{unit.shared_with ? '≈' : ''}{memory(unit.memory_bytes)}<small
                                >Memory</small
                            ></span
                        >
                        <div class="unit-actions">
                            {#if unit.kind === 'tab'}
                                <button
                                    disabled={demo || tabBusy}
                                    onclick={() => tab(unit, 'focus_tab')}>Show</button
                                ><button
                                    class="danger-text"
                                    disabled={demo || tabBusy}
                                    onclick={() => tab(unit, 'close_tab')}
                                    >{confirming === unit.id
                                        ? 'Confirm close'
                                        : 'Close tab…'}</button
                                >
                            {:else}
                                {#each unit.actions as action}<button
                                        class:danger-text={action === 'terminate'}
                                        disabled={demo || actionBusy}
                                        onclick={() => processAction(unit, action)}
                                        >{action === 'terminate'
                                            ? 'Stop…'
                                            : 'Lower priority…'}</button
                                    >{/each}
                            {/if}
                        </div>
                        <details class="evidence">
                            <summary>{unit.confidence}</summary>
                            <ul>
                                {#each unit.evidence as line}<li>{line}</li>{/each}
                            </ul>
                        </details>
                    </div>
                {:else}<p class="muted empty">Nothing to show.</p>{/each}
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
            <p class="footnote">
                {demo ? 'Synthetic example. ' : ''}{view.limits}
                {view.units.some((u) => u.kind === 'tab')
                    ? 'Closing a tab discards unsaved form input in it.'
                    : ''}
            </p>
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
        max-height: 420px;
        overflow: auto;
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
    .unit strong {
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
        gap: 6px;
        justify-content: flex-end;
    }
    .evidence {
        grid-column: 1 / -1;
        font-size: 11px;
        color: var(--muted);
    }
    .evidence ul {
        margin: 6px 0 0;
        padding-left: 18px;
    }
    .remainder strong {
        color: var(--muted);
    }
    .empty {
        padding: 8px;
    }
    .footnote {
        margin-bottom: 0;
    }
</style>
