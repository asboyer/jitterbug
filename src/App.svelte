<script lang="ts">
    import { onMount } from 'svelte';
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import { listen, type UnlistenFn } from '@tauri-apps/api/event';
    import AutomationPanel from './components/AutomationPanel.svelte';
    import Icon from './components/Icon.svelte';
    import Dashboard from './components/Dashboard.svelte';
    import Workloads from './components/Workloads.svelte';
    import AgentWorkflows from './components/AgentWorkflows.svelte';
    import ValidationQueue from './components/ValidationQueue.svelte';
    import IncidentInbox from './components/IncidentInbox.svelte';
    import WorkloadDetails from './components/WorkloadDetails.svelte';
    import ManageWorkload from './components/ManageWorkload.svelte';
    import type { ManagementSelection } from './lib/manage';
    import Chat from './components/Chat.svelte';
    import ActionResult from './components/ActionResult.svelte';
    import ProviderSettings from './components/ProviderSettings.svelte';
    import { demoBootstrap, demoSnapshot, demoIncidents } from './lib/demo';
    import { errorMessage } from './lib/format';
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
    let demoMenuOpen = $state(false);
    const pages = [
        { id: 'dashboard', label: 'Activity', icon: 'activity' },
        { id: 'workloads', label: 'Workloads', icon: 'app' },
        { id: 'agents', label: 'Agent tasks', icon: 'code' },
        { id: 'queue', label: 'Validation queue', icon: 'queue' },
        { id: 'inbox', label: 'Inbox', icon: 'inbox' },
        { id: 'settings', label: 'Settings', icon: 'settings' },
    ] as const;
    let demoTick = $state(0);
    const demoCases = demoIncidents();
    let management: ManagementSelection | null = $state(null);
    let live: Snapshot | null = $state(null);
    let demo = $state(!desktop);
    let demoPressure = $state(false);
    let bootstrap: Bootstrap = $state(structuredClone(demoBootstrap));
    let selectedId: string | null = $state(null);
    let chatOpen = $state(false);
    let questionProcess: ProcessRecord | null = $state(null);
    let followUpContext: AutomationCase | null = $state(null);
    let automationOpen = $state(false);
    let automaticId: number | null = $state(null);
    let cases: AutomationCase[] = $state([]);
    const visibleCases = $derived(demo ? demoCases : cases);
    const attentionCount = $derived(
        visibleCases.filter(
            (item) => item.stage === 'awaiting_approval' || item.stage === 'blocked'
        ).length
    );
    let selectedContext: Workload | null = $state(null);
    const automaticCase = $derived(
        visibleCases.find((c) => c.id === automaticId) ?? visibleCases[0] ?? null
    );
    let error = $state('');
    let actionReport: ActionReport | null = $state(null);
    let alert: Incident | null = $state(null);
    let pendingAction: { process: ProcessRecord; action: string } | null = $state(null);
    let actionBusy = $state(false);
    let mainElement: HTMLElement | undefined = $state();
    const snapshot = $derived.by(() => {
        if (!demo) return live;
        demoTick;
        return demoSnapshot(demoPressure);
    });
    $effect(() => {
        if (!demo) return;
        const timer = setInterval(() => {
            demoTick += 1;
        }, 5000);
        return () => clearInterval(timer);
    });
    const selected = $derived.by(
        () =>
            snapshot?.workloads.find((w) => w.id === selectedId) ??
            (selectedContext?.id === selectedId ? selectedContext : null)
    );

    $effect(() => {
        selectedId;
        page;
        mainElement?.scrollTo({ top: 0 });
    });

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
                    if (incident.stage === 'detected' && !demo) {
                        demo = false;
                        demoPressure = false;
                        alert = null;
                        // Keep the current reading context. The inbox badge surfaces new incidents.
                        if (
                            !automationOpen &&
                            !chatOpen &&
                            !selectedId &&
                            !management &&
                            !demoMenuOpen &&
                            page === 'dashboard'
                        ) {
                            automaticId = incident.id;
                            automationOpen = true;
                        }
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
                        navigate(event.payload.page);
                        if (page === 'dashboard') {
                            demoMenuOpen = false;
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

    function navigate(next: string) {
        page = next;
        demoMenuOpen = false;
        selectedId = null;
        selectedContext = null;
        chatOpen = false;
        automationOpen = false;
    }
    function showIncident(id: number) {
        automaticId = id;
        automationOpen = true;
        chatOpen = false;
    }
    function select(workload: Workload) {
        followUpContext = null;
        selectedId = workload.id;
        selectedContext = workload;
        automationOpen = false;
        chatOpen = false;
        questionProcess = null;
    }
    function ask() {
        questionProcess = null;
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
        chatOpen = true;
        questionProcess = null;
        followUpContext = incident;
        actionReport = incident.report;
    }
    async function simulatePressure() {
        demoMenuOpen = false;
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
                navigate('dashboard');
            }}><Icon name="bug" size={22} /> Jitterbug</a
        >
        <div class="nav-links">
            {#each pages as item}
                <button
                    class:active={page === item.id}
                    aria-current={page === item.id ? 'page' : undefined}
                    onclick={() => navigate(item.id)}
                >
                    <Icon name={item.icon} size={17} /><span>{item.label}</span>
                    {#if item.id === 'inbox' && attentionCount}<span class="nav-count"
                            >{attentionCount}</span
                        >{/if}
                </button>
            {/each}
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
    <main bind:this={mainElement}>
        <header class="topbar">
            <span class:demo-badge={demo} class="live-badge"
                ><i class="status-dot"></i>{demo
                    ? 'Demo · synthetic data'
                    : live?.sample_ready
                      ? 'Live activity'
                      : 'Connecting…'}</span
            >
            <div>
                {#if attentionCount}<button class="text-button" onclick={() => navigate('inbox')}
                        >{attentionCount}
                        {attentionCount === 1 ? 'item needs' : 'items need'} attention</button
                    >{/if}
                {#if demo}<button
                        class="text-button"
                        disabled={!desktop}
                        onclick={() => {
                            demoMenuOpen = false;
                            demo = false;
                            demoPressure = false;
                            alert = null;
                            selectedId = null;
                            automationOpen = false;
                            chatOpen = false;
                        }}>Return to live</button
                    >{/if}
                <details class="preview-menu" bind:open={demoMenuOpen}>
                    <summary>Demo &amp; testing</summary>
                    <div class="preview-actions">
                        <button
                            onclick={() => {
                                demoMenuOpen = false;
                                demo = true;
                                demoPressure = false;
                                alert = null;
                                selectedId = null;
                                chatOpen = false;
                                automationOpen = false;
                            }}>Explore demo</button
                        >
                        <button onclick={simulatePressure}>Simulate pressure</button>
                        <small>Synthetic data. No extra CPU or memory load.</small>
                    </div>
                </details>
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
        <div class="page-content">
            {#if page === 'settings'}
                <ProviderSettings {bootstrap} {desktop} onrefresh={refreshBootstrap} />
            {:else if page === 'inbox'}
                <div class="page-heading">
                    <div>
                        <h1>Inbox</h1>
                        <p class="muted">
                            Resource alerts, proposed changes, and what happened next.
                        </p>
                    </div>
                </div>
                <IncidentInbox cases={visibleCases} onselect={showIncident} />
            {:else if snapshot && snapshot.memory_total_bytes > 0}
                {#if selected}
                    {#key `${snapshot.source}:${selected.id}`}
                        <WorkloadDetails
                            {snapshot}
                            workload={selected}
                            {actionBusy}
                            onmanage={(selection) => {
                                management = selection;
                            }}
                            onback={() => {
                                selectedId = null;
                                chatOpen = false;
                            }}
                            onask={(process) => {
                                questionProcess = process;
                                followUpContext = null;
                                automationOpen = false;
                                chatOpen = true;
                            }}
                            onaction={(process, action) => {
                                error = '';
                                pendingAction = { process, action };
                            }}
                            onusage={() => {
                                void refreshBootstrap().catch(() => {});
                            }}
                        />
                    {/key}
                {:else if page === 'workloads'}
                    <Workloads {snapshot} onselect={select} />
                {:else if page === 'agents'}
                    <div class="page-heading">
                        <div>
                            <h1>Agent tasks</h1>
                            <p class="muted">
                                What your coding tools are working on, and the resources they use.
                            </p>
                        </div>
                    </div>
                    <AgentWorkflows
                        {snapshot}
                        onselect={select}
                        onmanage={(selection) => {
                            management = selection;
                        }}
                        onusage={() => {
                            void refreshBootstrap().catch(() => {});
                        }}
                    />
                {:else if page === 'queue'}
                    <div class="page-heading">
                        <div>
                            <h1>Validation queue</h1>
                            <p class="muted">
                                Let heavy checks take turns across your coding sessions.
                            </p>
                        </div>
                    </div>
                    <ValidationQueue
                        {snapshot}
                        onselect={select}
                        onmanage={(selection) => {
                            management = selection;
                        }}
                    />
                {:else}
                    <Dashboard
                        {snapshot}
                        onselect={select}
                        onask={ask}
                        onworkloads={() => navigate('workloads')}
                    />
                {/if}
            {:else}<div class="empty loading">
                    <span class="brand-mark">✳</span>
                    <h1>Getting to know your workloads</h1>
                    <p>Collecting the first resource sample. CPU rates need a second sample.</p>
                </div>{/if}
        </div>
    </main>
    {#if automationOpen && automaticCase}{#key automaticCase.id}<AutomationPanel
                incident={automaticCase}
                cases={visibleCases}
                onselect={(id) => {
                    automaticId = id;
                }}
                {desktop}
                onrefresh={refreshBootstrap}
                onclose={() => {
                    automationOpen = false;
                }}
                onask={followUp}
            />{/key}{:else if chatOpen && snapshot}<Chat
            {snapshot}
            workload={selected}
            {questionProcess}
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

{#if management && snapshot && !demo}
    <ManageWorkload
        selection={management}
        {snapshot}
        onclose={() => {
            management = null;
        }}
    />
{/if}

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
