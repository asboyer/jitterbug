<script lang="ts">
    // Development Lab: the Inside panel, agent sessions, the action popup, and an agent console
    // on live data.
    // Everything here goes through the same commands, socket, and approvals as the real app.
    import { onMount } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { listen } from '@tauri-apps/api/event';
    import ActionPopup from './ActionPopup.svelte';
    import AgentSessions from './AgentSessions.svelte';
    import InsidePanel from './InsidePanel.svelte';
    import { demoActionPopup } from '../lib/demo';
    import { errorMessage, memory, percent } from '../lib/format';
    import type {
        AutomationPolicy,
        PriorityChange,
        PriorityChanges,
        ProcessRecord,
        Snapshot,
    } from '../lib/types';

    type Unit = { unit_id: string; title: string; actions: string[]; is_your_session?: boolean };
    type Entry = { at: string; request: unknown; response: unknown; failed: boolean };

    let snapshot = $state<Snapshot | null>(null);
    let workloadId = $state('');
    let policy = $state<AutomationPolicy | null>(null);
    let actionBusy = $state(false);
    let notice = $state('');
    let log = $state<Entry[]>([]);
    let units = $state<Unit[]>([]);
    let requests = $state<number[]>([]);
    let reason = $state('Testing from the Jitterbug Lab.');
    let lowered = $state<PriorityChanges | null>(null);
    let sequence = 0;
    const sources: Record<PriorityChange['source'], string> = {
        approval: 'you approved',
        rule: 'saved rule',
        agent: 'agent request',
    };
    let area = $state<'inside' | 'sessions'>('inside');

    const workloads = $derived(
        [...(snapshot?.workloads ?? [])].sort((a, b) => b.cpu_percent - a.cpu_percent)
    );
    const workload = $derived(workloads.find((w) => w.id === workloadId) ?? null);
    const examples = ['', 'tab', 'pause', 'agent'].map((name) => ({
        name: name || 'priority',
        payload: demoActionPopup(name || null),
    }));

    onMount(() => {
        let stop: (() => void) | undefined;
        void (async () => {
            stop = await listen<Snapshot>('telemetry', (event) => (snapshot = event.payload));
            snapshot = await invoke<Snapshot>('get_snapshot');
            workloadId ||= workloads.find((w) => w.category === 'browser')?.id ?? workloads[0]?.id;
            await loadPolicy();
        })().catch((cause) => (notice = errorMessage(cause)));
        void loadLowered();
        // Processes that exit drop off the list on the next refresh.
        const timer = setInterval(() => void loadLowered(), 5000);
        return () => {
            stop?.();
            clearInterval(timer);
        };
    });

    async function loadLowered() {
        try {
            lowered = await invoke<PriorityChanges>('get_priority_changes');
        } catch (cause) {
            notice = errorMessage(cause);
        }
    }

    async function restorePriority(change: PriorityChange) {
        try {
            notice = await invoke<string>('restore_priority', { id: change.id });
        } catch (cause) {
            notice = errorMessage(cause);
        }
        await loadLowered();
    }

    async function removeRule(rule: AutomationPolicy['rules'][number]) {
        try {
            await invoke('remove_automation_rule', { workloadId: rule.workload_id });
            notice = `${rule.label} is no longer lowered automatically.`;
            await loadPolicy();
        } catch (cause) {
            notice = errorMessage(cause);
        }
        await loadLowered();
    }

    function since(at: number): string {
        const minutes = Math.floor((Date.now() / 1000 - at) / 60);
        if (minutes < 1) return 'just now';
        if (minutes < 60) return `${minutes} min ago`;
        return `${Math.floor(minutes / 60)} h ${minutes % 60} min ago`;
    }

    async function loadPolicy() {
        policy = (await invoke<{ settings: { automation: AutomationPolicy } }>('get_bootstrap'))
            .settings.automation;
    }

    async function toggleAgentControl() {
        if (!policy) return;
        try {
            await invoke('save_automation_policy', {
                policy: { ...policy, agent_control: !policy.agent_control },
            });
            await loadPolicy();
        } catch (cause) {
            notice = errorMessage(cause);
        }
    }

    async function processAction(process: ProcessRecord, action: string) {
        actionBusy = true;
        try {
            await invoke('process_action', {
                pid: process.pid,
                startTime: process.start_time,
                action,
            });
            notice = `${action} applied to ${process.name}.`;
        } catch (cause) {
            notice = errorMessage(cause);
        } finally {
            actionBusy = false;
        }
        await loadLowered();
    }

    async function previewPopup() {
        await invoke('preview_action_popup');
        notice = 'The example popup appears in 5 seconds, here and on screen.';
    }

    // The agent console sends JSON-RPC exactly as an MCP client would.
    async function rpc(method: string, params: Record<string, unknown> = {}) {
        const request = { jsonrpc: '2.0', id: ++sequence, method, params };
        const at = new Date().toLocaleTimeString();
        try {
            const response = await invoke<Record<string, any> | null>('lab_mcp', { request });
            const text = response?.result?.content?.[0]?.text;
            const parsed = parse(text);
            const failed = !!response?.error || !!response?.result?.isError;
            log = [{ at, request, response: parsed ?? response, failed }, ...log].slice(0, 40);
            if (parsed && !failed) collect(parsed);
        } catch (cause) {
            log = [{ at, request, response: errorMessage(cause), failed: true }, ...log];
        }
    }
    const tool = (name: string, args: Record<string, unknown> = {}) =>
        rpc('tools/call', { name, arguments: args });

    function parse(text: unknown) {
        if (typeof text !== 'string') return null;
        try {
            return JSON.parse(text);
        } catch {
            return text;
        }
    }

    // Units and request IDs from any answer become one-click targets.
    function collect(value: unknown) {
        const found: Unit[] = [];
        const walk = (node: unknown) => {
            if (Array.isArray(node)) return node.forEach(walk);
            if (!node || typeof node !== 'object') return;
            const item = node as Record<string, unknown>;
            if (typeof item.unit_id === 'string' && Array.isArray(item.actions)) {
                found.push(item as unknown as Unit);
            }
            if (typeof item.request_id === 'number' && !requests.includes(item.request_id)) {
                requests = [item.request_id, ...requests];
            }
            Object.values(item).forEach(walk);
        };
        walk(value);
        if (found.length) units = found;
    }

    const toolFor: Record<string, string> = {
        pause: 'jitterbug_pause_session',
        resume: 'jitterbug_resume_session',
        pause_tab: 'jitterbug_pause_tab',
        resume_tab: 'jitterbug_resume_tab',
        close_tab: 'jitterbug_close_tab',
    };
    function act(unit: Unit, action: string) {
        const args: Record<string, unknown> = { unit_id: unit.unit_id };
        if (action !== 'resume' && action !== 'resume_tab') args.reason = reason;
        void tool(toolFor[action], args);
    }
</script>

<main class="lab">
    <header>
        <h1>Jitterbug Lab</h1>
        <span class="muted">Development only · live data · real actions</span>
        {#if notice}<span class="notice">{notice}</span>{/if}
    </header>

    <section class="column">
        <div class="head">
            <div class="tabs" role="tablist">
                <button
                    role="tab"
                    aria-selected={area === 'inside'}
                    class:active={area === 'inside'}
                    onclick={() => (area = 'inside')}>Inside</button
                >
                <button
                    role="tab"
                    aria-selected={area === 'sessions'}
                    class:active={area === 'sessions'}
                    onclick={() => (area = 'sessions')}>Agent sessions</button
                >
            </div>
            {#if area === 'inside'}
                <select bind:value={workloadId}>
                    {#each workloads as w (w.id)}
                        <option value={w.id}
                            >{w.label} · {percent(w.cpu_percent)} · {memory(w.memory_bytes)}</option
                        >
                    {/each}
                </select>
            {/if}
        </div>
        <div class="scroll">
            {#if area === 'sessions'}
                <AgentSessions />
            {:else if snapshot && workload}
                {#key workload.id}
                    <InsidePanel
                        {snapshot}
                        {workload}
                        demo={false}
                        {actionBusy}
                        onaction={processAction}
                    />
                {/key}
            {:else}
                <p class="muted">Waiting for live telemetry…</p>
            {/if}
        </div>
    </section>

    <section class="column">
        <div class="head">
            <h2>Popup</h2>
            <button onclick={previewPopup}>Trigger example</button>
        </div>
        <div class="scroll">
            <h3>Live</h3>
            <p class="muted small">
                Exactly what the floating popup shows now. Empty while nothing needs you.
            </p>
            <div class="frame live"><ActionPopup /></div>
            <div class="lowered" aria-label="Currently lowered">
                <h3>Currently lowered</h3>
                {#if lowered?.changes.length}
                    <ul>
                        {#each lowered.changes as change (change.id)}
                            <li>
                                <span class="title">
                                    <span>{change.label}</span>
                                    <small class="muted">
                                        {change.process} · {since(change.at)} · {sources[
                                            change.source
                                        ]} · nice
                                        {change.previous} → {change.lowered_to}
                                    </small>
                                </span>
                                <button class="small" onclick={() => restorePriority(change)}
                                    >Restore</button
                                >
                            </li>
                        {/each}
                    </ul>
                {:else}
                    <p class="muted small">Nothing Jitterbug lowered is still running.</p>
                {/if}
                <h3>Always lower</h3>
                {#if lowered?.rules.length}
                    <ul>
                        {#each lowered.rules as rule (rule.workload_id)}
                            <li>
                                <span class="title">{rule.label}</span>
                                <button class="small" onclick={() => removeRule(rule)}
                                    >Remove</button
                                >
                            </li>
                        {/each}
                    </ul>
                {:else}
                    <p class="muted small">No saved rules.</p>
                {/if}
            </div>
            <h3>Examples</h3>
            <p class="muted small">Played locally; nothing is applied.</p>
            {#each examples as example (example.name)}
                <h4>{example.name}</h4>
                <div class="frame"><ActionPopup preset={example.payload} /></div>
            {/each}
        </div>
    </section>

    <section class="column">
        <div class="head">
            <h2>Agent (MCP)</h2>
            {#if policy}
                <label class="toggle">
                    <input
                        type="checkbox"
                        checked={policy.agent_control}
                        onchange={toggleAgentControl}
                    />
                    Agent control
                </label>
            {/if}
        </div>
        <div class="controls">
            <button onclick={() => rpc('tools/list')}>tools/list</button>
            <button onclick={() => tool('jitterbug_overview')}>overview</button>
            <button
                disabled={!workload}
                onclick={() => tool('jitterbug_inspect_app', { workload_id: workloadId })}
                >inspect {workload?.label ?? 'app'}</button
            >
            {#each requests.slice(0, 3) as id (id)}
                <button onclick={() => tool('jitterbug_request_status', { request_id: id })}
                    >status #{String(id).slice(-4)}</button
                >
            {/each}
        </div>
        <input class="reason" bind:value={reason} aria-label="Reason shown to the person" />
        {#if units.length}
            <ul class="units">
                {#each units as unit (unit.unit_id)}
                    <li>
                        <span class="title"
                            >{unit.title}{#if unit.is_your_session}
                                <small class="muted"> (Lab's own session)</small>{/if}</span
                        >
                        {#each unit.actions as action (action)}
                            <button class="small" onclick={() => act(unit, action)}>{action}</button
                            >
                        {/each}
                    </li>
                {/each}
            </ul>
        {/if}
        <div class="scroll log">
            {#each log as entry, index (index)}
                <details open={index === 0} class:failed={entry.failed}>
                    <summary
                        >{entry.at} · {(entry.request as any).params?.name ??
                            (entry.request as any).method}</summary
                    >
                    <pre>→ {JSON.stringify((entry.request as any).params, null, 2)}</pre>
                    <pre>← {typeof entry.response === 'string'
                            ? entry.response
                            : JSON.stringify(entry.response, null, 2)}</pre>
                </details>
            {:else}
                <p class="muted small">
                    Calls go through the real control socket, as an agent's would. Turn on agent
                    control, then start with overview.
                </p>
            {/each}
        </div>
    </section>
</main>

<style>
    .lab {
        height: 100vh;
        display: grid;
        grid-template-columns: minmax(0, 1.3fr) 400px minmax(0, 1fr);
        grid-template-rows: auto minmax(0, 1fr);
        gap: 12px;
        padding: 12px;
        box-sizing: border-box;
        background: var(--canvas);
    }
    header {
        grid-column: 1 / -1;
        display: flex;
        align-items: baseline;
        gap: 12px;
    }
    h1 {
        font-size: 17px;
        margin: 0;
    }
    h2 {
        font-size: 14px;
        margin: 0;
    }
    h3 {
        font-size: 13px;
        margin: 4px 0 2px;
    }
    h4 {
        font-size: 12px;
        margin: 12px 0 4px;
        color: var(--muted);
        text-transform: capitalize;
    }
    .column {
        display: flex;
        flex-direction: column;
        gap: 8px;
        min-height: 0;
        padding: 12px;
        border: 1px solid var(--border);
        border-radius: 10px;
        background: var(--panel);
    }
    .head,
    .controls {
        display: flex;
        align-items: center;
        gap: 8px;
        flex-wrap: wrap;
    }
    .head h2,
    .tabs {
        flex: 1;
    }
    .tabs {
        display: flex;
        gap: 4px;
    }
    .tabs button {
        font-weight: 600;
        border-color: transparent;
        background: transparent;
    }
    .tabs button.active {
        background: var(--accent-soft);
        color: var(--accent);
    }
    .scroll {
        flex: 1;
        min-height: 0;
        overflow: auto;
    }
    .frame {
        width: 360px;
        height: 300px;
    }
    .frame.live {
        height: auto;
    }
    .frame.live:has(:global(.action-popup)) {
        height: 300px;
    }
    .frame :global(.action-popup) {
        height: 100%;
        box-sizing: border-box;
        animation: none;
    }
    .muted {
        color: var(--muted);
    }
    .small {
        font-size: 12px;
    }
    .notice {
        margin-left: auto;
        font-size: 12px;
        color: var(--warning);
    }
    .toggle {
        display: flex;
        gap: 6px;
        font-size: 12px;
    }
    .reason {
        font-size: 12px;
        padding: 4px 6px;
    }
    .units {
        list-style: none;
        margin: 0;
        padding: 0;
        max-height: 200px;
        overflow: auto;
        font-size: 12px;
    }
    .units li {
        display: flex;
        align-items: center;
        gap: 6px;
        padding: 3px 0;
        border-bottom: 1px solid var(--border);
    }
    .units .title {
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .lowered {
        max-height: 220px;
        overflow: auto;
        border-top: 1px solid var(--border);
        padding-top: 6px;
    }
    .lowered ul {
        list-style: none;
        margin: 0 0 6px;
        padding: 0;
        font-size: 12px;
    }
    .lowered li {
        display: flex;
        align-items: center;
        gap: 6px;
        padding: 3px 0;
        border-bottom: 1px solid var(--border);
    }
    .lowered .title {
        flex: 1;
        min-width: 0;
        display: grid;
    }
    .lowered small {
        font-size: 11px;
    }
    .log details {
        border-bottom: 1px solid var(--border);
        padding: 4px 0;
        font-size: 12px;
    }
    .log details.failed summary {
        color: var(--danger);
    }
    pre {
        margin: 4px 0;
        font-size: 11px;
        white-space: pre-wrap;
        word-break: break-word;
    }
</style>
