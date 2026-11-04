<script lang="ts">
    import './sessions.css';
    import { onMount } from 'svelte';
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import { listen, type UnlistenFn } from '@tauri-apps/api/event';
    import Monitor from './components/Monitor.svelte';
    import Agents from './components/Agents.svelte';
    import Sessions from './components/Sessions.svelte';
    import IncidentInbox from './components/IncidentInbox.svelte';
    import ProviderSettings from './components/ProviderSettings.svelte';
    import AutomationPanel from './components/AutomationPanel.svelte';
    import ProactiveNotice from './components/ProactiveNotice.svelte';
    import Chat from './components/Chat.svelte';
    import Icon from './components/Icon.svelte';
    import Logo from './components/Logo.svelte';
    import { errorMessage } from './lib/format';
    import type {
        ActionReport,
        AutomationCase,
        Bootstrap,
        ProcessRecord,
        Snapshot,
        Workload,
    } from './lib/types';

    const desktop = isTauri();
    const pages = [
        { id: 'monitor', label: 'Monitor', icon: 'activity', sub: 'What is running and why' },
        { id: 'agents', label: 'Agents', icon: 'code', sub: 'Coding agents and their heavy work' },
        { id: 'sessions', label: 'Sessions', icon: 'play', sub: 'Saved setups for focused work' },
        { id: 'inbox', label: 'Inbox', icon: 'inbox', sub: 'Slowdowns, suggestions and results' },
    ] as const;
    function pressureWord(level: string) {
        return level === 'normal'
            ? 'Running smoothly'
            : level === 'watch'
              ? 'Getting busy'
              : 'Under strain';
    }
    function memoryShare(value: Snapshot) {
        return Math.round(
            ((value.memory_total_bytes - value.memory_available_bytes) /
                Math.max(value.memory_total_bytes, 1)) *
                100
        );
    }
    let page = $state('monitor');
    let settingsSection = $state<'assistance' | 'integrations' | 'system' | 'ai'>('assistance');
    let snapshot = $state<Snapshot | null>(null);
    let bootstrap = $state<Bootstrap | null>(null);
    let cases = $state<AutomationCase[]>([]);
    let selectedContext = $state<Workload | null>(null);
    const selected = $derived(
        snapshot?.workloads.find((w) => w.id === selectedContext?.id) ?? selectedContext
    );
    let automaticId = $state<number | null>(null);
    const automaticCase = $derived(cases.find((c) => c.id === automaticId) ?? null);
    const attentionCount = $derived(
        cases.filter((c) => ['awaiting_approval', 'blocked'].includes(c.stage)).length
    );
    let noticeId = $state<number | null>(null);
    const noticedCase = $derived(cases.find((c) => c.id === noticeId));
    let followUpContext = $state<AutomationCase | null>(null);
    let actionReport = $state<ActionReport | null>(null);
    let pendingAction = $state<{ process: ProcessRecord; action: string } | null>(null);
    let actionBusy = $state(false);
    let error = $state('');
    let captureStatus = $state('');
    let accessAsk = $state<{ agent: string | null } | null>(null);
    let accessDeclined = false;
    let accessBusy = $state(false);
    let mainElement: HTMLElement | undefined = $state();

    async function refreshBootstrap() {
        if (!desktop) return;
        const value = await invoke<Bootstrap>('get_bootstrap');
        bootstrap = value;
        cases = value.automation_cases.filter((c) => c.source === 'live');
    }
    function navigate(next: string) {
        page = ['dashboard', 'workloads', 'hub'].includes(next) ? 'monitor' : next;
        if (page === 'ai') {
            page = 'settings';
            settingsSection = 'ai';
        }
        selectedContext = null;
        automaticId = null;
        followUpContext = null;
        mainElement?.scrollTo({ top: 0 });
    }
    function showIncident(id: number) {
        noticeId = null;
        followUpContext = null;
        automaticId = id;
    }
    function followUp(incident: AutomationCase) {
        automaticId = null;
        followUpContext = incident;
        const target = incident.trigger.target;
        selectedContext = snapshot?.workloads.find((w) => w.id === target?.workload_id) ?? null;
        actionReport = incident.report;
    }
    function requestAction(process: ProcessRecord, action: string) {
        error = '';
        pendingAction = { process, action };
    }
    async function confirmAction() {
        if (!pendingAction || !desktop) return;
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
    async function allowAgentAccess() {
        if (!bootstrap || !desktop) return;
        accessBusy = true;
        try {
            await invoke('save_automation_policy', {
                policy: { ...$state.snapshot(bootstrap.settings.automation), agent_control: true },
            });
            await refreshBootstrap();
            accessAsk = null;
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            accessBusy = false;
        }
    }
    onMount(() => {
        if (!desktop) return;
        const cleanup: UnlistenFn[] = [];
        let disposed = false;
        async function observe<T>(event: string, handler: (payload: T) => void) {
            const stop = await listen<T>(event, ({ payload }) => {
                if (!disposed) handler(payload);
            });
            if (disposed) stop();
            else cleanup.push(stop);
        }
        async function initialize() {
            try {
                await observe<Snapshot>('telemetry', (value) => {
                    if (value.source === 'live') snapshot = value;
                });
                await observe<AutomationCase>('automation-case', (incident) => {
                    if (incident.source !== 'live') return;
                    cases = [incident, ...cases.filter((c) => c.id !== incident.id)].slice(0, 24);
                    if (incident.stage === 'detected') noticeId = incident.id;
                    if (
                        ['resolved', 'dismissed'].includes(incident.stage) &&
                        noticeId === incident.id
                    )
                        noticeId = null;
                });
                await observe<{ phase: string; message: string }>('jitter-capture', (value) => {
                    if (value.phase === 'error') error = value.message;
                    captureStatus = value.phase === 'recording' ? value.message : '';
                });
                await observe<{ page: string; workload: Workload | null }>('navigate', (value) => {
                    navigate(value.page);
                    if (value.workload) {
                        page = 'monitor';
                        selectedContext = value.workload;
                    }
                });
                await observe<{ id: number; source: string }>('open-incident', (value) => {
                    if (value.source === 'live') showIncident(value.id);
                });
                await observe<{ agent: string | null }>('agent-access-requested', (value) => {
                    if (!accessDeclined && !bootstrap?.settings.automation.agent_control)
                        accessAsk = value;
                });
                const value = await invoke<Snapshot>('get_snapshot');
                if (!disposed && value.source === 'live') snapshot = value;
                if (!disposed) await refreshBootstrap();
            } catch (cause) {
                if (!disposed) error = errorMessage(cause);
            }
        }
        void initialize();
        return () => {
            disposed = true;
            cleanup.forEach((stop) => stop());
        };
    });
</script>

<div class="product-shell">
    <aside class="app-sidebar">
        <div class="sidebar-drag" data-tauri-drag-region></div>
        <a
            class="product-brand"
            href="#monitor"
            onclick={(event) => {
                event.preventDefault();
                navigate('monitor');
            }}
        >
            <Logo size={26} live={!!snapshot?.sample_ready} /><span
                >Jitterbug<small>Your Mac, explained</small></span
            >
        </a>
        <nav class="product-nav" aria-label="Main navigation">
            {#each pages as item}
                <button
                    class:active={page === item.id}
                    aria-current={page === item.id ? 'page' : undefined}
                    onclick={() => navigate(item.id)}
                >
                    <Icon
                        name={item.icon}
                        size={16}
                    />{item.label}{#if item.id === 'inbox' && attentionCount}<span class="nav-count"
                            >{attentionCount}</span
                        >{/if}
                </button>
            {/each}
        </nav>
        <div class="sidebar-footer">
            <nav class="product-nav" aria-label="Preferences">
                <button
                    class:active={page === 'settings'}
                    aria-current={page === 'settings' ? 'page' : undefined}
                    onclick={() => navigate('settings')}
                    ><Icon name="settings" size={16} />Settings</button
                >
            </nav>
            <div class="machine-card" aria-label="This Mac">
                <div class="machine-line">
                    <i class="live-dot" class:on={snapshot?.sample_ready}></i><strong
                        >This Mac</strong
                    ><span
                        >{snapshot?.sample_ready
                            ? pressureWord(snapshot.pressure.level)
                            : desktop
                              ? 'Connecting…'
                              : 'Not connected'}</span
                    >
                </div>
                {#if snapshot?.sample_ready}
                    <div class="mini-meter">
                        <span>CPU</span><i
                            ><b style={`width:${Math.min(snapshot.cpu_percent, 100)}%`}></b></i
                        ><span>{Math.round(snapshot.cpu_percent)}%</span>
                    </div>
                    <div class="mini-meter">
                        <span>RAM</span><i><b style={`width:${memoryShare(snapshot)}%`}></b></i
                        ><span>{memoryShare(snapshot)}%</span>
                    </div>
                {/if}
            </div>
        </div>
    </aside>
    <div class="product-workspace">
        {#if page !== 'monitor'}
            <header class="toolbar" data-tauri-drag-region>
                <h1 data-tauri-drag-region>
                    {pages.find((p) => p.id === page)?.label ?? 'Settings'}
                </h1>
                <span class="toolbar-sub" data-tauri-drag-region
                    >{pages.find((p) => p.id === page)?.sub ??
                        'Assistance, integrations and AI'}</span
                >
                <div class="toolbar-spacer" data-tauri-drag-region></div>
            </header>
        {/if}
        <main
            class="product-main"
            class:monitor-active={page === 'monitor'}
            bind:this={mainElement}
        >
            {#if captureStatus}<div class="inline-notice" role="status">{captureStatus}</div>{/if}
            {#if error && !pendingAction}<div class="inline-notice error" role="alert">
                    {error}<button class="text-button" onclick={() => (error = '')}>Dismiss</button>
                </div>{/if}
            {#if accessAsk}<div class="access-request" role="alert">
                    <div>
                        <strong>{accessAsk.agent ?? 'A coding agent'} requested access</strong>
                        <p>
                            Share app, folder and tab names with this agent. Pauses and closures
                            still require review.
                        </p>
                    </div>
                    <button
                        onclick={() => {
                            accessDeclined = true;
                            accessAsk = null;
                        }}>Not now</button
                    ><button class="primary" disabled={accessBusy} onclick={allowAgentAccess}
                        >Allow access</button
                    >
                </div>{/if}
            {#if !desktop}
                <section class="connection-empty">
                    <Logo size={56} />
                    <h1>Connect to your computer</h1>
                    <p>Open the Jitterbug desktop app to see live processes and agent activity.</p>
                    <p class="muted">This browser has no connection to system telemetry.</p>
                </section>
            {:else if page === 'settings' && bootstrap}
                <ProviderSettings
                    {bootstrap}
                    {desktop}
                    initialSection={settingsSection}
                    onrefresh={refreshBootstrap}
                />
            {:else if page === 'agents'}
                <Agents />
            {:else if page === 'sessions'}
                <Sessions {desktop} />
            {:else if page === 'inbox'}
                <IncidentInbox {cases} onselect={showIncident} />
            {:else if snapshot && snapshot.memory_total_bytes > 0}
                <Monitor
                    {snapshot}
                    {selected}
                    {cases}
                    {actionBusy}
                    {actionReport}
                    settingsKey={JSON.stringify([bootstrap?.settings, bootstrap?.credentials])}
                    onselect={(workload) => (selectedContext = workload)}
                    onclose={() => (selectedContext = null)}
                    onaction={requestAction}
                    onusage={() => void refreshBootstrap().catch(() => {})}
                />
            {:else}<section class="connection-empty">
                    <Logo size={56} live />
                    <h1>Reading your Mac</h1>
                    <p>Collecting the first resource samples.</p>
                </section>{/if}
        </main>
    </div>
</div>

{#if automaticCase}
    <AutomationPanel
        incident={automaticCase}
        {cases}
        {desktop}
        onselect={(id) => (automaticId = id)}
        onrefresh={refreshBootstrap}
        onclose={() => (automaticId = null)}
        onask={followUp}
        onaction={requestAction}
    />
{:else if followUpContext && snapshot}
    <Chat
        {snapshot}
        workload={selected}
        questionProcess={null}
        initialReply={null}
        incident={followUpContext}
        {desktop}
        onclose={() => (followUpContext = null)}
        {actionReport}
        {actionBusy}
        onaction={requestAction}
        onusage={() => void refreshBootstrap().catch(() => {})}
    />
{/if}
{#if noticedCase && !automaticCase && !followUpContext && !pendingAction}
    <ProactiveNotice
        incident={noticedCase}
        workflow={undefined}
        onreview={() => showIncident(noticedCase.id)}
        ontask={() => {
            noticeId = null;
            navigate('agents');
        }}
        onlater={() => {
            const id = noticeId;
            noticeId = null;
            if (id !== null) void invoke('close_action_popup', { id }).catch(() => {});
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
            <h2 id="confirm-title">
                {pendingAction.action === 'terminate'
                    ? 'Stop this process?'
                    : 'Lower CPU priority?'}
            </h2>
            <p><strong>{pendingAction.process.name}</strong> · PID {pendingAction.process.pid}</p>
            <p>
                {pendingAction.action === 'terminate'
                    ? 'Active work or unsaved changes may be lost. This requests termination of this process only.'
                    : 'Other work gets CPU first when the computer is busy. This does not free memory and is not automatically reversed.'}
            </p>
            <p class="muted">
                The process identity and ownership are checked again before applying.
            </p>
            {#if error}<p class="error" role="alert">{error}</p>{/if}
            <div class="form-actions">
                <button disabled={actionBusy} onclick={() => (pendingAction = null)}>Cancel</button
                ><button class="primary" disabled={actionBusy} onclick={confirmAction}
                    >{actionBusy ? 'Applying…' : 'Confirm action'}</button
                >
            </div>
        </div>
    </div>{/if}
