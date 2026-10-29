<script lang="ts">
    import { onMount } from 'svelte';
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import { listen, type UnlistenFn } from '@tauri-apps/api/event';
    import AutomationPanel from './components/AutomationPanel.svelte';
    import Icon from './components/Icon.svelte';
    import Dashboard from './components/Dashboard.svelte';
    import Chat from './components/Chat.svelte';
    import ActionResult from './components/ActionResult.svelte';
    import ProviderSettings from './components/ProviderSettings.svelte';
    import { demoBootstrap, demoSnapshot } from './lib/demo';
    import { errorMessage, memory, percent } from './lib/format';
    import type {
        ActionReport,
        AutomationCase,
        Bootstrap,
        Incident,
        ProcessRecord,
        Snapshot,
        Workload,
    } from './lib/types';

    const desktop = isTauri();
    let page = $state('dashboard');
    let live: Snapshot | null = $state(null);
    let demo = $state(!desktop);
    let demoPressure = $state(false);
    let bootstrap: Bootstrap = $state(structuredClone(demoBootstrap));
    let selectedId: string | null = $state(null);
    let chatOpen = $state(false);
    let followUpContext: AutomationCase | null = $state(null);
    let automationOpen = $state(false);
    let automaticId: number | null = $state(null);
    let cases: AutomationCase[] = $state([]);
    let selectedContext: Workload | null = $state(null);
    const automaticCase = $derived(cases.find((c) => c.id === automaticId) ?? cases[0] ?? null);
    let error = $state('');
    let actionReport: ActionReport | null = $state(null);
    let alert: Incident | null = $state(null);
    let pendingAction: { process: ProcessRecord; action: string } | null = $state(null);
    let actionBusy = $state(false);
    const snapshot = $derived(demo ? demoSnapshot(demoPressure) : live);
    const selected = $derived.by(
        () =>
            snapshot?.workloads.find((w) => w.id === selectedId) ??
            (selectedContext?.id === selectedId ? selectedContext : null)
    );
    const selectedProcesses = $derived(
        snapshot?.processes.filter((p) => p.workload_id === selectedId) ?? []
    );

    async function refreshBootstrap() {
        if (desktop) {
            bootstrap = await invoke('get_bootstrap');
            cases = bootstrap.automation_cases;
        }
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
                const automatic = await listen<AutomationCase>('automation-case', (event) => {
                    const incident = event.payload;
                    cases = [incident, ...cases.filter((c) => c.id !== incident.id)].slice(0, 12);
                    if (incident.stage === 'detected') {
                        demo = false;
                        demoPressure = false;
                        alert = null;
                        automaticId = incident.id;
                        automationOpen = true;
                        chatOpen = false;
                    }
                    if (
                        [
                            'completed',
                            'resolved',
                            'blocked',
                            'dismissed',
                            'awaiting_approval',
                        ].includes(incident.stage)
                    ) {
                        void refreshBootstrap().catch(() => {});
                    }
                });
                if (disposed) {
                    automatic();
                    return;
                }
                cleanup.push(automatic);
                const simulated = await listen<Incident>('demo-pressure-incident', (event) => {
                    demo = true;
                    demoPressure = true;
                    selectedId = null;
                    alert = event.payload;
                    chatOpen = true;
                });
                if (disposed) {
                    simulated();
                    return;
                }
                cleanup.push(simulated);
                const navigation = await listen<{ page: string; workload: Workload | null }>(
                    'navigate',
                    (event) => {
                        page = event.payload.page;
                        if (page === 'dashboard') {
                            demo = false;
                            demoPressure = false;
                            alert = null;
                        }
                        if (page === 'settings') {
                            chatOpen = false;
                            automationOpen = false;
                        }
                        if (event.payload.workload) {
                            demo = false;
                            select(event.payload.workload);
                        }
                    }
                );
                if (disposed) {
                    navigation();
                    return;
                }
                cleanup.push(navigation);
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
        followUpContext = null;
        selectedId = workload.id;
        selectedContext = workload;
        automationOpen = false;
        chatOpen = true;
    }
    function ask() {
        followUpContext = null;
        automationOpen = false;
        selectedContext = null;
        selectedId = null;
        chatOpen = true;
    }
    function followUp(incident: AutomationCase) {
        const target = incident.trigger.target;
        if (target) {
            select(
                snapshot?.workloads.find((w) => w.id === target.workload_id) ?? {
                    id: target.workload_id,
                    label: target.label,
                    category: 'other',
                    description: '',
                    attribution: 'Incident context',
                    cpu_percent: incident.evidence?.cpu_percent ?? 0,
                    memory_bytes: incident.evidence?.memory_bytes ?? 0,
                    process_count: incident.evidence?.process_count ?? 0,
                    history: incident.evidence?.history ?? [],
                }
            );
        } else {
            ask();
        }
        followUpContext = incident;
        actionReport = incident.report;
    }
    async function simulatePressure() {
        followUpContext = null;
        automationOpen = false;
        if (desktop) {
            try {
                await invoke('simulate_pressure');
            } catch (cause) {
                error = errorMessage(cause);
            }
            return;
        }
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
            actionReport = await invoke('process_action', {
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
            }}><Icon name="bug" size={22} /> Jitterbug</a
        >
        <div class="nav-links">
            <button
                class:active={page === 'dashboard'}
                onclick={() => {
                    page = 'dashboard';
                }}><Icon name="activity" /><span>Workloads</span></button
            ><button
                class:active={page === 'settings'}
                onclick={() => {
                    page = 'settings';
                    chatOpen = false;
                    automationOpen = false;
                }}><Icon name="settings" /><span>Settings</span></button
            >
        </div>
        <div class="sidebar-bottom">
            <span class="tag">{bootstrap.settings.mode.replaceAll('_', ' ')}</span>
            <p>Resource assistant</p>
            <small
                >Monitoring every 5 seconds. {bootstrap.settings.automation.enabled
                    ? bootstrap.settings.automation.model_analysis
                        ? 'Automatic diagnosis enabled.'
                        : 'Automatic observations enabled.'
                    : 'Automatic assistance disabled.'}</small
            >
        </div>
    </nav>
    <main>
        <header class="topbar">
            <span class:demo-badge={demo} class="live-badge"
                ><i class="status-dot"></i>{demo
                    ? 'Demo · synthetic data'
                    : live?.sample_ready
                      ? 'Live activity'
                      : 'Connecting…'}</span
            >
            <div>
                <button
                    class="text-button"
                    disabled={!cases.length || demo}
                    onclick={() => {
                        automationOpen = true;
                        chatOpen = false;
                    }}>Incidents ({cases.length})</button
                >
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
                    }}>Dismiss demo</button
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
        {#if actionReport && !chatOpen}<ActionResult report={actionReport} />{/if}
        {#if cases.length && !demo}<div class="incident-strip">
                {#each cases.slice(0, 4) as incident}
                    <button
                        onclick={() => {
                            automaticId = incident.id;
                            automationOpen = true;
                            chatOpen = false;
                        }}
                    >
                        {incident.trigger.target?.label ?? 'System'} · {incident.stage.replaceAll(
                            '_',
                            ' '
                        )}
                    </button>
                {/each}
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
                                    }}>Ask about this workload</button
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
                            Select a process action or ask the assistant to investigate this
                            workload’s recent history. Exact agent task ownership is not available
                            yet.
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
                                                        {#if process.actions.includes('lower_priority')}<button
                                                                disabled={actionBusy}
                                                                onclick={() => {
                                                                    pendingAction = {
                                                                        process,
                                                                        action: 'lower_priority',
                                                                    };
                                                                }}>Lower priority</button
                                                            >{/if}
                                                        {#if process.actions.includes('terminate')}<button
                                                                disabled={actionBusy}
                                                                class="danger-text"
                                                                onclick={() => {
                                                                    pendingAction = {
                                                                        process,
                                                                        action: 'terminate',
                                                                    };
                                                                }}>Stop…</button
                                                            >{/if}
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
    {#if automationOpen && automaticCase && !demo}<AutomationPanel
            incident={automaticCase}
            {cases}
            onselect={(id) => {
                automaticId = id;
            }}
            {desktop}
            onrefresh={refreshBootstrap}
            onclose={() => {
                automationOpen = false;
            }}
            onask={followUp}
        />{:else if chatOpen && snapshot}<Chat
            {snapshot}
            workload={selected}
            incident={followUpContext}
            {desktop}
            onclose={() => {
                chatOpen = false;
            }}
            {actionReport}
            {actionBusy}
            onaction={(process, action) => {
                error = '';
                pendingAction = { process, action };
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
            {#if error}<p class="error" role="alert">{error}</p>{/if}
            <div class="form-actions">
                <button
                    disabled={actionBusy}
                    onclick={() => {
                        pendingAction = null;
                    }}>Cancel</button
                ><button class="primary" disabled={actionBusy} onclick={confirmAction}
                    >{actionBusy ? 'Applying & measuring…' : 'Confirm action'}</button
                >
            </div>
        </div>
    </div>{/if}
