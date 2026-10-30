<script lang="ts">
    import { onMount, tick } from 'svelte';
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import { listen, type UnlistenFn } from '@tauri-apps/api/event';
    import AutomationPanel from './components/AutomationPanel.svelte';
    import GuidedTour from './components/GuidedTour.svelte';
    import ProactiveNotice from './components/ProactiveNotice.svelte';
    import { tourPreference, tourSteps } from './lib/tour';
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
    let tourIndex = $state<number | null>(null);
    let tourSeen = $state(true);
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
        { id: 'dashboard', label: 'Activity', icon: 'activity' },
        { id: 'workloads', label: 'Workloads', icon: 'app' },
        { id: 'agents', label: 'Agent tasks', icon: 'code' },
        { id: 'queue', label: 'Validation queue', icon: 'queue' },
        { id: 'inbox', label: 'Inbox', icon: 'inbox' },
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
                        if (page === 'dashboard') {
                            stopDemoSequence();
                            notice = null;
                            demoMenuOpen = false;
                            demo = false;
                            agentTaskId = null;
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
        if (desktop) {
            try {
                await invoke('simulate_pressure');
            } catch (cause) {
                error = errorMessage(cause);
            }
        } else startDemoIncident();
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
            <span class="tag" title="AI mode. Change this in Settings."
                >{bootstrap.settings.mode.replaceAll('_', ' ')}</span
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
            {#if !tourSeen && tourIndex === null && page === 'dashboard' && !selectedId}
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
                    {#if !agentTaskId}<div class="page-heading">
                            <div>
                                <h1>Agent tasks</h1>
                                <p class="muted">
                                    What your coding tools are working on, and the resources they
                                    use.
                                </p>
                            </div>
                        </div>
                    {/if}<AgentWorkflows
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
                {:else}
                    {#if demo}<section class="demo-story" aria-label="Try Jitterbug in action">
                            <span class="story-mark"><Icon name="bug" size={24} /></span>
                            <div>
                                <span class="eyebrow">TRY AN EXAMPLE</span>
                                <h2>Two agents. Two test runs. One busy computer.</h2>
                                <p>
                                    See Jitterbug notice the overlap and help plan a better next
                                    run.
                                </p>
                            </div>
                            <button class="primary" onclick={simulatePressure}
                                >See Jitterbug step in</button
                            >
                        </section>{/if}
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

{#if noticedCase && (noticedCase.source === 'demo') === demo && tourIndex === null && !management && !pendingAction && !actionBusy && !automationOpen && !chatOpen}
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
            notice = null;
        }}
    />
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
