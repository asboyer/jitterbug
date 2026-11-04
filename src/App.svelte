<script lang="ts">
    import './sessions.css';
    import { onMount, tick } from 'svelte';
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import { listen, type UnlistenFn } from '@tauri-apps/api/event';
    import AutomationPanel from './components/AutomationPanel.svelte';
    import GuidedTour from './components/GuidedTour.svelte';
    import ProactiveNotice from './components/ProactiveNotice.svelte';
    import { tourPreference, tourSteps } from './lib/tour';
    import Icon from './components/Icon.svelte';
    import Monitor from './components/Monitor.svelte';
    import WorkflowNotice from './components/WorkflowNotice.svelte';
    import { hubDemo, demoControl } from './lib/workflow-hub';
    import type { ValidationQueue as QueueState } from './lib/queue';
    import AgentWorkflows from './components/AgentWorkflows.svelte';
    import Sessions from './components/Sessions.svelte';
    import ValidationQueue from './components/ValidationQueue.svelte';
    import IncidentInbox from './components/IncidentInbox.svelte';
    import WorkloadDetails from './components/WorkloadDetails.svelte';
    import ManageWorkload from './components/ManageWorkload.svelte';
    import type { ManagementSelection } from './lib/manage';
    import Chat from './components/Chat.svelte';
    import ActionResult from './components/ActionResult.svelte';
    import ProviderSettings from './components/ProviderSettings.svelte';
    import AiSettings from './components/AiSettings.svelte';
    import ModeSwitch from './components/ModeSwitch.svelte';
    import Onboarding from './components/Onboarding.svelte';
    import { demoBootstrap, demoSnapshot, demoIncidents } from './lib/demo';
    import { errorMessage } from './lib/format';
    import type {
        ActionReport,
        Settings,
        AutomationCase,
        Bootstrap,
        Incident,
        ProcessRecord,
        Reply,
        Snapshot,
        Workload,
    } from './lib/types';

    const desktop = isTauri();
    let page = $state('monitor');
    let demoMenuOpen = $state(false);
    let tourIndex = $state<number | null>(null);
    let tourSeen = $state(true);
    const onboardingPreference = 'jitterbug.onboarding.v1.done';
    let onboardingOpen = $state(false);
    let onboardingTourOffer = $state(false);
    let settingsRevision = $state(0);
    /** An agent tried to use Jitterbug while agent access was off. */
    let accessAsk = $state<{ agent: string | null } | null>(null);
    let accessDeclined = false;
    let accessBusy = $state(false);
    async function allowAgentAccess() {
        accessBusy = true;
        const policy = { ...$state.snapshot(bootstrap.settings.automation), agent_control: true };
        try {
            if (desktop && !demo) {
                await invoke('save_automation_policy', { policy });
                await refreshBootstrap();
            } else {
                bootstrap.settings.automation = policy;
            }
            settingsRevision++;
            accessAsk = null;
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            accessBusy = false;
        }
    }
    function onboardingDone() {
        try {
            return localStorage.getItem(onboardingPreference) === '1';
        } catch {
            return true;
        }
    }
    function openOnboarding() {
        if (tourIndex !== null) finishTour();
        demoMenuOpen = false;
        onboardingOpen = true;
    }
    function finishOnboarding(tour: boolean) {
        onboardingOpen = false;
        try {
            localStorage.setItem(onboardingPreference, '1');
        } catch {
            /* Optional preference only. */
        }
        navigate('monitor');
        if (tour) startTour();
        // Land on the front page with the tour offer, even if the tour was seen before.
        else {
            tourSeen = false;
            onboardingTourOffer = true;
        }
    }
    /** Saves AI settings from the sidebar switch or onboarding; the preview keeps them in memory. */
    async function saveAiSettings(next: Settings) {
        if (desktop) {
            await invoke('save_settings', { settings: next });
            await refreshBootstrap();
        } else {
            bootstrap.settings = next;
        }
        settingsRevision++;
    }
    onMount(() => {
        if (new URLSearchParams(location.search).has('onboarding')) onboardingOpen = true;
    });
    let beforeTour: {
        page: string;
        demo: boolean;
        demoPressure: boolean;
        selectedId: string | null;
        agentTaskId: string | null;
        selectedContext: Workload | null;
        chatOpen: boolean;
        automationOpen: boolean;
        automaticId: number | null;
        alert: Incident | null;
    } | null = null;
    onMount(() => {
        try {
            tourSeen = localStorage.getItem(tourPreference) === '1';
        } catch {
            tourSeen = false;
        }
    });
    function markTourSeen() {
        tourSeen = true;
        try {
            localStorage.setItem(tourPreference, '1');
        } catch {
            /* Optional preference only. */
        }
    }
    function showTourStep(index: number) {
        tourIndex = index;
        navigate(tourSteps[index].page);
        if (tourSteps[index].target === 'proposal') showIncident(demoCases[0].id);
    }
    function startTour() {
        if (tourIndex !== null || management || pendingAction || actionBusy) return;
        beforeTour = {
            page,
            demo,
            demoPressure,
            selectedId,
            agentTaskId,
            selectedContext,
            chatOpen,
            automationOpen,
            automaticId,
            alert,
        };
        stopDemoSequence();
        notice = null;
        demoCases = demoIncidents();
        demo = true;
        demoPressure = false;
        alert = null;
        showTourStep(0);
    }
    function finishTour() {
        tourIndex = null;
        markTourSeen();
        if (beforeTour) {
            ({
                page,
                demo,
                demoPressure,
                selectedId,
                agentTaskId,
                selectedContext,
                chatOpen,
                automationOpen,
                automaticId,
                alert,
            } = beforeTour);
            beforeTour = null;
        }
        void tick().then(() =>
            document.querySelector<HTMLButtonElement>('[data-tour-start]')?.focus()
        );
    }
    const pages = [
        { id: 'monitor', label: 'Monitor', icon: 'activity' },
        { id: 'agents', label: 'Agent tasks', icon: 'code' },
        { id: 'sessions', label: 'Sessions', icon: 'play' },
        { id: 'queue', label: 'Validation queue', icon: 'queue' },
        { id: 'inbox', label: 'Inbox', icon: 'inbox' },
        { id: 'ai', label: 'AI', icon: 'model' },
        { id: 'settings', label: 'Settings', icon: 'settings' },
    ] as const;
    let demoTick = $state(0);
    let demoCases = $state(demoIncidents());
    let agentTaskId = $state<string | null>(null);
    let notice = $state<{ id: number; source: string } | null>(null);
    let announced: string[] = [];
    let demoTimers: ReturnType<typeof setTimeout>[] = [];
    const noticedCase = $derived.by(() =>
        notice
            ? (notice.source === 'demo' ? demoCases : cases).find((item) => item.id === notice?.id)
            : undefined
    );
    const noticedWorkflow = $derived.by(() =>
        snapshot?.agent_workflows.find(
            (workflow) =>
                workflow.status === 'active' &&
                workflow.agents.some((agent) =>
                    agent.workload_ids.includes(noticedCase?.trigger.target?.workload_id ?? '')
                )
        )
    );
    function receiveCase(incident: AutomationCase) {
        if (incident.source === 'demo') {
            demoCases = [incident, ...demoCases.filter((item) => item.id !== incident.id)].slice(
                0,
                12
            );
        } else {
            cases = [incident, ...cases.filter((item) => item.id !== incident.id)].slice(0, 12);
        }
        const key = `${incident.source}:${incident.id}`;
        if (
            ['resolved', 'dismissed'].includes(incident.stage) &&
            notice?.id === incident.id &&
            notice.source === incident.source
        ) {
            notice = null;
        }
        if (incident.stage === 'detected' && !announced.includes(key)) {
            announced = [...announced, key].slice(-24);
            if ((incident.source === 'demo') === demo)
                notice = { id: incident.id, source: incident.source };
        }
    }
    function stopDemoSequence() {
        demoTimers.forEach(clearTimeout);
        demoTimers = [];
    }
    function startDemoIncident(detected?: AutomationCase) {
        stopDemoSequence();
        if (tourIndex !== null) finishTour();
        if (!demo) {
            agentTaskId = null;
            selectedId = null;
            selectedContext = null;
            automationOpen = false;
            chatOpen = false;
        }
        demo = true;
        demoPressure = true;
        alert = null;
        const example = demoIncidents()[0];
        if (detected?.evidence?.processes[0] && example.proposal) {
            example.proposal.process = detected.evidence.processes[0];
        }
        const first: AutomationCase = detected
            ? { ...detected, id: -1 }
            : {
                  ...example,
                  stage: 'detected',
                  diagnosis: null,
                  proposal: null,
                  steps: [
                      {
                          at: example.detected_at,
                          stage: 'detected',
                          message: 'Synthetic sustained-load example.',
                      },
                  ],
              };
        announced = announced.filter((key) => key !== 'demo:-1');
        receiveCase(first);
        demoTimers.push(
            setTimeout(() => {
                receiveCase({ ...first, stage: 'investigating' });
            }, 1000)
        );
        demoTimers.push(
            setTimeout(() => {
                receiveCase({
                    ...first,
                    stage: 'awaiting_approval',
                    diagnosis: example.diagnosis,
                    proposal: example.proposal,
                    steps: [...first.steps, ...example.steps],
                });
            }, 2500)
        );
    }
    onMount(() => () => stopDemoSequence());
    let management: ManagementSelection | null = $state(null);
    let live: Snapshot | null = $state(null);
    let demo = $state(!desktop);
    let demoPressure = $state(false);
    let bootstrap: Bootstrap = $state(structuredClone(demoBootstrap));
    let selectedId: string | null = $state(null);
    let chatOpen = $state(false);
    let questionInsight: Reply | null = $state(null);
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
    let captureStatus = $state('');
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
    let workflowQueue = $state<QueueState | null>(null);
    let workflowBusy = $state(false);
    let workflowError = $state('');
    let workflowNotice = $state(false);
    let queueSource = '';
    let queueVersion = 0;
    $effect(() => {
        const source = snapshot?.source;
        snapshot?.captured_at;
        const version = ++queueVersion;
        if (!source) return;
        if (queueSource !== source) {
            workflowNotice = false;
            workflowError = '';
            workflowQueue = null;
        }
        if (source === 'demo') {
            if (queueSource !== source) workflowQueue = hubDemo();
        } else if (desktop) {
            void invoke<QueueState>('get_validation_queue')
                .then((value) => {
                    if (version === queueVersion) {
                        workflowQueue = value;
                        workflowError = '';
                    }
                })
                .catch((cause) => {
                    if (version === queueVersion) workflowError = errorMessage(cause);
                });
        }
        queueSource = source;
        return () => {
            if (queueVersion === version) queueVersion++;
        };
    });
    async function controlWorkflow(action: string, id?: string) {
        if (workflowBusy || !workflowQueue) return false;
        const source = snapshot?.source;
        workflowBusy = true;
        queueVersion++;
        workflowError = '';
        try {
            if (demo) {
                workflowQueue = demoControl($state.snapshot(workflowQueue), action, id ?? null);
                if (action === 'demo_pressure' && workflowQueue.resources.cpu_percent >= 85)
                    workflowNotice = true;
            } else {
                await invoke('control_validation_queue', { action, jobId: id ?? null });
                const value = await invoke<QueueState>('get_validation_queue');
                if (source !== snapshot?.source) return false;
                queueVersion++;
                workflowQueue = value;
            }
            if (action === 'protect') workflowNotice = false;
            return true;
        } catch (cause) {
            if (source === snapshot?.source) workflowError = errorMessage(cause);
            return false;
        } finally {
            workflowBusy = false;
        }
    }
    $effect(() => {
        const queue = workflowQueue;
        if (
            queue &&
            (queue.paused ||
                (queue.control?.protected_until ?? 0) > (snapshot?.captured_at ?? 0) ||
                !queue.jobs.some((job) => job.stage === 'queued' && !job.held) ||
                (queue.resources.cpu_percent < 85 &&
                    !['warning', 'critical'].includes(queue.resources.memory_pressure ?? '')))
        ) {
            workflowNotice = false;
        }
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
                const asking = await listen<{ agent: string | null }>(
                    'agent-access-requested',
                    (event) => {
                        if (!accessDeclined && !bootstrap.settings.automation.agent_control)
                            accessAsk = event.payload;
                    }
                );
                if (disposed) {
                    asking();
                    return;
                }
                cleanup.push(asking);
                const scheduling = await listen('workflow-attention', () => {
                    if (!demo) workflowNotice = true;
                });
                if (disposed) {
                    scheduling();
                    return;
                }
                cleanup.push(scheduling);
                const automatic = await listen<AutomationCase>('automation-case', (event) => {
                    const incident = event.payload;
                    receiveCase(incident);
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
                const captured = await listen<{ phase: string; message: string }>(
                    'jitter-capture',
                    (event) => {
                        if (event.payload.phase === 'error') error = event.payload.message;
                        captureStatus =
                            event.payload.phase === 'recording' ? event.payload.message : '';
                    }
                );
                if (disposed) {
                    captured();
                    return;
                }
                cleanup.push(captured);
                const simulated = await listen<AutomationCase>('demo-pressure-case', (event) => {
                    startDemoIncident(event.payload);
                });
                if (disposed) {
                    simulated();
                    return;
                }
                cleanup.push(simulated);
                const navigation = await listen<{ page: string; workload: Workload | null }>(
                    'navigate',
                    (event) => {
                        if (tourIndex !== null) finishTour();
                        navigate(event.payload.page);
                        if (event.payload.page === 'dashboard') {
                            stopDemoSequence();
                            notice = null;
                            demoMenuOpen = false;
                            demo = false;
                            agentTaskId = null;
                            demoPressure = false;
                            alert = null;
                        }
                        if (page === 'settings' || page === 'ai') {
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
                const popupDetails = await listen<{ id: number; source: string }>(
                    'open-incident',
                    (event) => {
                        if (tourIndex !== null) finishTour();
                        const fromDemo = event.payload.source === 'demo';
                        if (fromDemo !== demo) {
                            navigate(page);
                            if (!fromDemo) {
                                stopDemoSequence();
                                notice = null;
                                demoPressure = false;
                                alert = null;
                            }
                        }
                        demo = fromDemo;
                        const list = fromDemo ? demoCases : cases;
                        const match = list.find((item) => item.id === event.payload.id) ?? list[0];
                        if (match) showIncident(match.id);
                    }
                );
                if (disposed) {
                    popupDetails();
                    return;
                }
                cleanup.push(popupDetails);
                live = await invoke('get_snapshot');
                await refreshBootstrap();
                // First launch only, and never once any provider key exists.
                if (!onboardingDone() && !bootstrap.credentials.length) onboardingOpen = true;
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
        page = ['hub', 'dashboard', 'workloads'].includes(next) ? 'monitor' : next;
        agentTaskId = null;
        demoMenuOpen = false;
        selectedId = null;
        selectedContext = null;
        chatOpen = false;
        automationOpen = false;
    }
    function showIncident(id: number) {
        if (notice?.id === id) notice = null;
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
        questionInsight = null;
    }
    function ask() {
        questionProcess = null;
        questionInsight = null;
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
        questionInsight = null;
        followUpContext = incident;
        actionReport = incident.report;
    }
    async function simulatePressure() {
        demoMenuOpen = false;
        if (desktop) {
            try {
                await invoke('simulate_pressure');
            } catch (cause) {
                error = errorMessage(cause);
            }
        } else startDemoIncident();
    }
    let popupStatus = $state('');
    async function previewPopup() {
        if (!desktop) {
            window.open('?view=popup', 'jitterbug-popup', 'popup,width=360,height=216');
            return;
        }
        try {
            await invoke('preview_action_popup');
            popupStatus = 'Switch to another app. An example popup appears in 5 seconds.';
            setTimeout(() => {
                popupStatus = '';
            }, 8000);
        } catch (cause) {
            error = errorMessage(cause);
        }
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
            href="#monitor"
            onclick={(event) => {
                event.preventDefault();
                navigate('monitor');
            }}><Icon name="companion" size={22} /> Jitterbug</a
        >
        <div class="nav-links">
            {#each pages as item}
                <button
                    data-tour={item.id === 'settings' ? 'settings' : undefined}
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
            <button
                class="text-button tour-trigger"
                data-tour-start
                disabled={!!management || !!pendingAction || actionBusy}
                onclick={startTour}><Icon name="info" size={16} /> Take a tour</button
            >
            <ModeSwitch
                {bootstrap}
                onsave={saveAiSettings}
                onkeys={() => {
                    settingsRevision++;
                    navigate('ai');
                }}
            />
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
                            stopDemoSequence();
                            notice = null;
                            demoMenuOpen = false;
                            demo = false;
                            agentTaskId = null;
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
                                stopDemoSequence();
                                notice = null;
                                demoCases = demoIncidents();
                                demoMenuOpen = false;
                                demo = true;
                                agentTaskId = null;
                                demoPressure = false;
                                alert = null;
                                selectedId = null;
                                chatOpen = false;
                                automationOpen = false;
                            }}>Explore demo</button
                        >
                        <button onclick={simulatePressure}>Simulate pressure</button>
                        <button onclick={previewPopup}>Preview action popup</button>
                        <button onclick={openOnboarding}>Replay onboarding</button>
                        <button
                            onclick={() => {
                                demoMenuOpen = false;
                                accessAsk = { agent: 'Claude Code' };
                            }}>Preview agent access request</button
                        >
                        <small
                            >{popupStatus || 'Synthetic data. No extra CPU or memory load.'}</small
                        >
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
                        page = 'monitor';
                    }}>Investigate</button
                ><button
                    class="text-button"
                    onclick={() => {
                        alert = null;
                    }}>Dismiss demo</button
                >
            </div>{/if}
        {#if accessAsk}<div class="banner access-banner" role="alert">
                <div>
                    <strong
                        >{accessAsk.agent ?? 'A coding agent'} wants to see what is using this computer</strong
                    >
                    <p>
                        Allowing lets agents read app names, folder names and tab titles, and ask
                        before pausing a session. You can turn it off in Settings › Integrations.
                    </p>
                </div>
                <button
                    class="text-button"
                    disabled={accessBusy}
                    onclick={() => {
                        accessDeclined = true;
                        accessAsk = null;
                    }}>Not now</button
                ><button class="primary" disabled={accessBusy} onclick={allowAgentAccess}
                    >{accessBusy ? 'Allowing…' : 'Allow'}</button
                >
            </div>{/if}
        {#if captureStatus}<div class="banner" role="status">{captureStatus}</div>{/if}
        {#if error}<div class="banner error" role="alert">
                {error}<button
                    class="text-button"
                    onclick={() => {
                        error = '';
                    }}>Dismiss</button
                >
            </div>{/if}
        {#if actionReport && !chatOpen && page !== 'monitor'}<ActionResult
                report={actionReport}
            />{/if}
        <div class="page-content">
            {#if onboardingTourOffer && !tourSeen && tourIndex === null && page === 'monitor' && !selectedId}
                <div class="tour-invitation">
                    <span>New to Jitterbug?</span><button class="text-button" onclick={startTour}
                        >Take a quick tour</button
                    ><button
                        class="icon-button"
                        aria-label="Dismiss tour invitation"
                        onclick={markTourSeen}>×</button
                    >
                </div>
            {/if}
            {#if page === 'ai'}
                {#key settingsRevision}
                    <AiSettings {bootstrap} {desktop} onrefresh={refreshBootstrap} />
                {/key}
            {:else if page === 'settings'}
                {#key settingsRevision}
                    <ProviderSettings {bootstrap} {desktop} onrefresh={refreshBootstrap} />
                {/key}
            {:else if page === 'sessions'}
                <Sessions />
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
                {#if page === 'monitor'}
                    <Monitor
                        {snapshot}
                        {selected}
                        {actionBusy}
                        {actionReport}
                        settingsKey={JSON.stringify([
                            bootstrap.settings.mode,
                            bootstrap.settings.intelligent,
                            bootstrap.settings.budget_usd,
                            bootstrap.credentials,
                        ])}
                        onselect={select}
                        onclose={() => {
                            selectedId = null;
                            selectedContext = null;
                            chatOpen = false;
                        }}
                        onaction={(process, action) => {
                            error = '';
                            pendingAction = { process, action };
                        }}
                        onusage={() => {
                            void refreshBootstrap().catch(() => {});
                        }}
                    />
                {:else if selected}
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
                            onask={(process, reply) => {
                                questionProcess = process;
                                questionInsight = reply ?? null;
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
                        bind:selectedId={agentTaskId}
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
                onaction={(process, action) => {
                    error = '';
                    pendingAction = { process, action };
                }}
            />{/key}{:else if chatOpen && snapshot}<Chat
            {snapshot}
            workload={selected}
            {questionProcess}
            initialReply={questionInsight}
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

{#if workflowNotice && workflowQueue && tourIndex === null && !management && !pendingAction && !actionBusy && !chatOpen && !automationOpen}
    <WorkflowNotice
        {demo}
        busy={workflowBusy}
        error={workflowError}
        onprotect={() => controlWorkflow('protect')}
        onlater={() => {
            workflowNotice = false;
        }}
    />
{/if}

{#if !workflowNotice && noticedCase && (noticedCase.source === 'demo') === demo && tourIndex === null && !management && !pendingAction && !actionBusy && !automationOpen && !chatOpen}
    <ProactiveNotice
        incident={noticedCase}
        workflow={noticedWorkflow}
        onreview={() => showIncident(noticedCase.id)}
        ontask={() => {
            const id = noticedWorkflow?.task_id;
            notice = null;
            navigate('agents');
            agentTaskId = id ?? null;
        }}
        onlater={() => {
            // Read the case before clearing the notice; noticedCase is derived from it.
            const later = noticedCase;
            notice = null;
            // The out-of-app popup honors the same "Later".
            if (desktop && later?.source === 'live')
                void invoke('close_action_popup', { id: later.id }).catch(() => {});
        }}
    />
{/if}

{#if onboardingOpen}
    <Onboarding {bootstrap} {desktop} onsave={saveAiSettings} onfinish={finishOnboarding} />
{/if}

{#if tourIndex !== null}
    <GuidedTour
        index={tourIndex}
        onclose={finishTour}
        onback={() => {
            if (tourIndex !== null && tourIndex > 0) showTourStep(tourIndex - 1);
        }}
        onnext={() => {
            if (tourIndex === null) return;
            if (tourIndex === tourSteps.length - 1) finishTour();
            else showTourStep(tourIndex + 1);
        }}
    />
{/if}

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
