<script lang="ts">
    import { onMount } from 'svelte';
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import { listen, type UnlistenFn } from '@tauri-apps/api/event';
    import Dashboard from './components/Dashboard.svelte';
    import Chat from './components/Chat.svelte';
    import ProviderSettings from './components/ProviderSettings.svelte';
    import { demoBootstrap, demoSnapshot } from './lib/demo';
    import { errorMessage, memory, percent } from './lib/format';
    import type { Bootstrap, Incident, ProcessRecord, Snapshot, Workload } from './lib/types';

    const desktop = isTauri();
    let page = $state('dashboard');
    let live: Snapshot | null = $state(null);
    let demo = $state(!desktop);
    let demoPressure = $state(false);
    let bootstrap: Bootstrap = $state(structuredClone(demoBootstrap));
    let selectedId: string | null = $state(null);
    let chatOpen = $state(false);
    let error = $state('');
    let notice = $state('');
    let alert: Incident | null = $state(null);
    let snoozedUntil = $state(0);
    let pendingAction: { process: ProcessRecord; action: string } | null = $state(null);
    let actionBusy = $state(false);
    const snapshot = $derived(demo ? demoSnapshot(demoPressure) : live);
    const selected = $derived(snapshot?.workloads.find((w) => w.id === selectedId) ?? null);
    const selectedProcesses = $derived(
        snapshot?.processes.filter((p) => p.workload_id === selectedId) ?? []
    );

    async function refreshBootstrap() {
        if (desktop) bootstrap = await invoke('get_bootstrap');
    }

    onMount(() => {
        if (!desktop) return;
        const cleanup: UnlistenFn[] = [];
        let disposed = false;
        async function initialize() {
            try {
                const telemetry = await listen<Snapshot>('telemetry', (event) => {
                    live = event.payload;
                });
                if (disposed) {
                    telemetry();
                    return;
                }
                cleanup.push(telemetry);
                const pressure = await listen<Incident>('pressure-incident', (event) => {
                    if (!demo && Date.now() > snoozedUntil) {
                        alert = event.payload;
                        chatOpen = true;
                    }
                });
                if (disposed) {
                    pressure();
                    return;
                }
                cleanup.push(pressure);
                live = await invoke('get_snapshot');
                await refreshBootstrap();
            } catch (cause) {
                error = errorMessage(cause);
            }
        }
        void initialize();
        return () => {
            disposed = true;
            cleanup.forEach((fn) => fn());
        };
    });

    function select(workload: Workload) {
        selectedId = workload.id;
    }
    function ask() {
        selectedId = null;
        chatOpen = true;
    }
    function simulatePressure() {
        demo = true;
        demoPressure = true;
        selectedId = null;
        alert = demoSnapshot(true).incident;
        chatOpen = true;
    }
    async function confirmAction() {
        if (!pendingAction || demo || !desktop) return;
        actionBusy = true;
        error = '';
        try {
            notice = await invoke('process_action', {
                pid: pendingAction.process.pid,
                startTime: pendingAction.process.start_time,
                action: pendingAction.action,
            });
            pendingAction = null;
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            actionBusy = false;
        }
    }
</script>

<div class="shell">
    <nav class="sidebar" aria-label="Main navigation">
        <a
            class="brand"
            href="#dashboard"
            onclick={(event) => {
                event.preventDefault();
                page = 'dashboard';
            }}
            ><span class="brand-mark" aria-hidden="true">✳</span> jitterbug<span class="version"
                >EARLY BUILD</span
            ></a
        >
        <div class="nav-links">
            <button
                class:active={page === 'dashboard'}
                onclick={() => {
                    page = 'dashboard';
                }}>◫ <span>Workloads</span></button
            ><button
                class:active={page === 'settings'}
                onclick={() => {
                    page = 'settings';
                }}>⚙ <span>Providers & policy</span></button
            >
        </div>
        <div class="sidebar-bottom">
            <span class="tag">{bootstrap.settings.mode.replaceAll('_', ' ')}</span>
            <p>Observe first.<br />Reason when needed.</p>
            <small>Local monitoring every 5 seconds. No background AI calls.</small>
        </div>
    </nav>
    <main>
        <header class="topbar">
            <span class:demo-badge={demo} class="live-badge"
                ><i class="status-dot"></i>{demo
                    ? 'DEMO · SYNTHETIC DATA'
                    : live?.sample_ready
                      ? 'LIVE · THIS COMPUTER'
                      : 'CONNECTING TO COLLECTOR'}</span
            >
            <div>
                <button class="text-button" onclick={simulatePressure}>Simulate pressure</button
                ><button
                    class="text-button"
                    disabled={!desktop}
                    onclick={() => {
                        demo = !demo;
                        demoPressure = false;
                        alert = null;
                        selectedId = null;
                    }}>{demo ? 'Return to live' : 'Explore demo'}</button
                >
            </div>
        </header>
        {#if !desktop}<div class="banner demo-banner">
                Browser preview: all telemetry and assistant replies are mocked. Run the desktop app
                for real data and actions.
            </div>{/if}
        {#if alert}<div class="banner pressure-banner" role="alert">
                <div>
                    <strong>{alert.title}</strong>
                    <p>{alert.reasons.join(' ')}</p>
                </div>
                <button
                    onclick={() => {
                        chatOpen = true;
                        page = 'dashboard';
                    }}>Investigate</button
                ><button
                    class="text-button"
                    onclick={() => {
                        alert = null;
                        snoozedUntil = Date.now() + 300000;
                    }}>Dismiss for 5 minutes</button
                >
            </div>{/if}
        {#if error}<div class="banner error" role="alert">
                {error}<button
                    class="text-button"
                    onclick={() => {
                        error = '';
                    }}>Dismiss</button
                >
            </div>{/if}
        {#if notice}<div class="banner success" role="status">
                {notice}<button
                    class="text-button"
                    onclick={() => {
                        notice = '';
                    }}>Dismiss</button
                >
            </div>{/if}
        <div class="page-content">
            {#if page === 'settings'}
                <ProviderSettings {bootstrap} {desktop} onrefresh={refreshBootstrap} />
            {:else if snapshot && snapshot.memory_total_bytes > 0}
                <Dashboard {snapshot} {selectedId} onselect={select} onask={ask} />
                {#if selected}<section class="panel detail-panel">
                        <div class="section-title">
                            <div>
                                <p class="eyebrow">
                                    WORKLOAD DETAILS · {demo ? 'SYNTHETIC' : 'LIVE'}
                                </p>
                                <h2>{selected.label}</h2>
                            </div>
                            <div>
                                <button
                                    onclick={() => {
                                        chatOpen = true;
                                    }}>Explain this workload ↗</button
                                ><button
                                    class="icon-button"
                                    aria-label="Close workload details"
                                    onclick={() => {
                                        selectedId = null;
                                    }}>×</button
                                >
                            </div>
                        </div>
                        <p>{selected.description}</p>
                        <p class="muted">
                            Model context alias: Workload {snapshot.workloads.findIndex(
                                (w) => w.id === selectedId
                            ) + 1}. Exact agent task ownership is not available yet.
                        </p>
                        <div class="process-table">
                            <table>
                                <thead
                                    ><tr
                                        ><th>Process</th><th>CPU</th><th>Memory</th><th
                                            >Explicit actions</th
                                        ></tr
                                    ></thead
                                ><tbody
                                    >{#each selectedProcesses as process}<tr
                                            ><td
                                                ><strong>{process.name}</strong><small
                                                    >PID {process.pid} · {process.evidence.join(
                                                        ' '
                                                    )}</small
                                                ></td
                                            ><td>{percent(process.cpu_percent)}</td><td
                                                >{memory(process.memory_bytes)}</td
                                            ><td
                                                >{#if process.actionable && !demo}<div
                                                        class="process-actions"
                                                    >
                                                        <button
                                                            onclick={() => {
                                                                pendingAction = {
                                                                    process,
                                                                    action: 'lower_priority',
                                                                };
                                                            }}>Lower priority</button
                                                        ><button
                                                            class="danger-text"
                                                            onclick={() => {
                                                                pendingAction = {
                                                                    process,
                                                                    action: 'terminate',
                                                                };
                                                            }}>Stop…</button
                                                        >
                                                    </div>{:else}<span class="muted"
                                                        >{demo
                                                            ? 'Disabled in demo'
                                                            : 'Protected / unavailable'}</span
                                                    >{/if}</td
                                            ></tr
                                        >{/each}</tbody
                                >
                            </table>
                        </div>
                        <p class="footnote">
                            Showing members present in the top 160 processes. Actions apply to one
                            process, not its entire tree. Native process actions are currently
                            implemented on macOS only.
                        </p>
                    </section>{/if}
            {:else}<div class="empty loading">
                    <span class="brand-mark">✳</span>
                    <h1>Getting to know your workloads</h1>
                    <p>Collecting the first resource sample. CPU rates need a second sample.</p>
                </div>{/if}
        </div>
    </main>
    {#if chatOpen && snapshot}<Chat
            {snapshot}
            workload={selected}
            {desktop}
            onclose={() => {
                chatOpen = false;
            }}
            onusage={() => {
                void refreshBootstrap().catch(() => {});
            }}
        />{/if}
</div>

{#if pendingAction}<div class="modal-backdrop">
        <div
            class="modal panel"
            role="dialog"
            aria-modal="true"
            aria-labelledby="confirm-title"
            tabindex="-1"
        >
            <p class="eyebrow">EXPLICIT PROCESS ACTION</p>
            <h2 id="confirm-title">
                {pendingAction.action === 'terminate'
                    ? 'Request this process to stop?'
                    : 'Lower this process’s CPU priority?'}
            </h2>
            <p><strong>{pendingAction.process.name}</strong> · PID {pendingAction.process.pid}</p>
            <p>
                {pendingAction.action === 'terminate'
                    ? 'Unsaved work or an active task may be lost. This sends a termination request to this process only. It does not force-kill or stop a whole workload.'
                    : 'The operating system will favor other work when CPU is contested. This does not free memory and is not automatically reversed.'}
            </p>
            <p class="muted">Jitterbug rechecks ownership and process start time before acting.</p>
            <div class="form-actions">
                <button
                    disabled={actionBusy}
                    onclick={() => {
                        pendingAction = null;
                    }}>Cancel</button
                ><button class="primary" disabled={actionBusy} onclick={confirmAction}
                    >{actionBusy ? 'Applying…' : 'Confirm action'}</button
                >
            </div>
        </div>
    </div>{/if}
