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
        AppPriority,
        AutomationPolicy,
        BucketSource,
        Priorities,
        PriorityBucket,
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
    let priorities = $state<Priorities | null>(null);
    let sequence = 0;
    const buckets: PriorityBucket[] = ['protected', 'normal', 'background'];
    const bucketNames: Record<PriorityBucket, string> = {
        protected: 'Protected',
        normal: 'Normal',
        background: 'Background',
    };
    const bucketEmpty: Record<PriorityBucket, string> = {
        protected: 'Nothing protected.',
        normal: 'No running apps to move.',
        background: 'Nothing in Background.',
    };
    const sources: Record<BucketSource, string> = {
        you: 'you',
        agent: 'agent request',
        migrated_rule: 'migrated rule',
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
        void loadPriorities();
        // Counts follow new and exited processes on the next refresh.
        const timer = setInterval(() => void loadPriorities(), 5000);
        return () => {
            stop?.();
            clearInterval(timer);
        };
    });

    async function loadPriorities() {
        try {
            priorities = await invoke<Priorities>('get_priority_buckets');
        } catch (cause) {
            notice = errorMessage(cause);
        }
    }

    async function move(app: AppPriority, bucket: PriorityBucket) {
        try {
            notice = await invoke<string>('move_to_bucket', {
                workloadId: app.workload_id,
                bucket,
            });
        } catch (cause) {
            notice = errorMessage(cause);
        }
        await loadPriorities();
    }

    function details(app: AppPriority): string {
        return [
            app.source && sources[app.source],
            app.since && since(app.since),
            app.affected && `${app.affected} in background mode`,
            app.frontmost && 'in front',
            !app.running && 'not running',
        ]
            .filter(Boolean)
            .join(' · ');
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
        await loadPriorities();
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
    <header class="lab-header">
        <span class="lab-mark" aria-hidden="true">✦</span>
        <div class="lab-title">
            <h1>Jitterbug Lab</h1>
            <span class="muted">Development only · live data · real actions</span>
        </div>
        <span class="lab-status"><i></i> Live workspace</span>
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
            <div class="priorities" aria-label="Priorities">
                <h3>Priorities</h3>
                {#each buckets as bucket (bucket)}
                    <h5>{bucketNames[bucket]}</h5>
                    {#if priorities?.[bucket].length}
                        <ul>
                            {#each priorities[bucket] as app (app.workload_id)}
                                <li>
                                    <span class="title">
                                        <span>{app.label}</span>
                                        {#if details(app)}<small class="muted">{details(app)}</small
                                            >{/if}
                                    </span>
                                    <select
                                        class="small"
                                        aria-label={`Move ${app.label} to`}
                                        value=""
                                        onchange={(event) =>
                                            move(app, event.currentTarget.value as PriorityBucket)}
                                    >
                                        <option value="" disabled>Move to…</option>
                                        {#each buckets.filter((b) => b !== bucket) as target (target)}
                                            <option value={target}>{bucketNames[target]}</option>
                                        {/each}
                                    </select>
                                </li>
                            {/each}
                        </ul>
                    {:else}
                        <p class="muted small">{bucketEmpty[bucket]}</p>
                    {/if}
                {/each}
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
    .priorities {
        max-height: 260px;
        overflow: auto;
        border-top: 1px solid var(--border);
        padding-top: 6px;
    }
    .priorities h5 {
        margin: 6px 0 2px;
        font-size: 11px;
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.5px;
    }
    .priorities p {
        margin: 0 0 4px;
    }
    .priorities ul {
        list-style: none;
        margin: 0 0 6px;
        padding: 0;
        font-size: 12px;
    }
    .priorities li {
        display: flex;
        align-items: center;
        gap: 6px;
        padding: 3px 0;
        border-bottom: 1px solid var(--border);
    }
    .priorities .title {
        flex: 1;
        min-width: 0;
        display: grid;
    }
    .priorities small {
        font-size: 11px;
    }
    .priorities select {
        width: auto;
        padding: 2px 4px;
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

    /* The Lab is a dense developer workspace, styled as a focused instrument panel. */
    .lab {
        grid-template-columns: minmax(340px, 1.25fr) minmax(360px, 0.95fr) minmax(330px, 1fr);
        grid-template-rows: 58px minmax(0, 1fr);
        gap: 16px;
        padding: 16px;
        background:
            radial-gradient(circle at 46% -15%, var(--accent-subtle), transparent 34%),
            radial-gradient(
                circle at 100% 110%,
                color-mix(in srgb, var(--accent) 5%, transparent),
                transparent 32%
            ),
            var(--canvas);
    }
    .lab-header {
        position: relative;
        align-items: center;
        gap: 12px;
        min-width: 0;
        padding: 0 4px;
    }
    .lab-header::after {
        content: '';
        position: absolute;
        left: 0;
        right: 0;
        bottom: -8px;
        height: 1px;
        background: linear-gradient(90deg, var(--border), transparent 75%);
    }
    .lab-mark {
        width: 38px;
        height: 38px;
        display: grid;
        place-items: center;
        flex: 0 0 38px;
        border: 1px solid color-mix(in srgb, var(--accent) 24%, var(--border));
        border-radius: 12px;
        color: var(--accent);
        background: var(--accent-soft);
        box-shadow:
            inset 0 1px 0 color-mix(in srgb, white 28%, transparent),
            0 8px 20px color-mix(in srgb, var(--accent) 12%, transparent);
        font-size: 19px;
    }
    .lab-title {
        display: flex;
        align-items: baseline;
        gap: 12px;
        min-width: 0;
    }
    .lab-title h1 {
        font-size: 20px;
        line-height: 1.1;
        letter-spacing: -0.55px;
        font-weight: 670;
    }
    .lab-title .muted {
        font-size: 11px;
        white-space: nowrap;
    }
    .lab-status {
        display: inline-flex;
        align-items: center;
        gap: 7px;
        margin-left: auto;
        padding: 5px 10px;
        border: 1px solid var(--border);
        border-radius: 999px;
        color: var(--muted);
        background: color-mix(in srgb, var(--panel) 80%, transparent);
        font-size: 10px;
        font-weight: 620;
        white-space: nowrap;
        box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06);
    }
    .lab-status i {
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: var(--success);
        box-shadow: 0 0 0 3px color-mix(in srgb, var(--success) 13%, transparent);
    }
    .notice {
        max-width: min(420px, 38vw);
        padding: 6px 10px;
        overflow: hidden;
        border: 1px solid color-mix(in srgb, var(--warning) 22%, var(--border));
        border-radius: 8px;
        background: var(--warning-soft);
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .column {
        position: relative;
        gap: 12px;
        padding: 0;
        overflow: hidden;
        border-color: var(--border);
        border-radius: 14px;
        background: color-mix(in srgb, var(--panel) 96%, transparent);
        box-shadow: var(--shadow-card);
    }
    .column::before {
        content: '';
        position: absolute;
        inset: 0 0 auto;
        z-index: 1;
        height: 2px;
        background: linear-gradient(
            90deg,
            var(--accent),
            color-mix(in srgb, var(--accent) 20%, transparent) 55%,
            transparent
        );
        opacity: 0.8;
    }
    .column:nth-of-type(2)::before {
        background: linear-gradient(
            90deg,
            var(--warning),
            color-mix(in srgb, var(--warning) 18%, transparent) 55%,
            transparent
        );
    }
    .column:nth-of-type(3)::before {
        background: linear-gradient(
            90deg,
            var(--success),
            color-mix(in srgb, var(--success) 18%, transparent) 55%,
            transparent
        );
    }
    .head {
        min-height: 56px;
        padding: 11px 14px;
        border-bottom: 1px solid var(--border);
        background: color-mix(in srgb, var(--panel) 91%, var(--accent-subtle));
    }
    .head h2 {
        font-size: 13px;
        letter-spacing: -0.15px;
    }
    .head > select {
        width: min(230px, 46%);
        min-height: 32px;
        background: var(--control);
    }
    .tabs {
        flex: 0 0 auto;
        padding: 3px;
        border: 1px solid var(--border);
        border-radius: 9px;
        background: color-mix(in srgb, var(--canvas) 72%, transparent);
    }
    .tabs button {
        min-height: 30px;
        padding: 5px 10px;
        border-radius: 6px;
        font-size: 11px;
        font-weight: 560;
        box-shadow: none;
    }
    .tabs button.active {
        color: var(--text);
        background: var(--control);
        box-shadow:
            0 1px 2px rgba(0, 0, 0, 0.08),
            0 4px 10px rgba(0, 0, 0, 0.06);
    }
    .tabs button.active::before {
        content: '';
        width: 6px;
        height: 6px;
        margin-right: 7px;
        display: inline-block;
        border-radius: 50%;
        background: var(--accent);
        box-shadow: 0 0 0 3px var(--accent-soft);
        vertical-align: 1px;
    }
    .scroll {
        padding: 0 14px 14px;
        overscroll-behavior: contain;
    }
    .column > .scroll > :first-child {
        margin-top: 2px;
    }
    .column > .scroll > .muted:first-child {
        display: grid;
        min-height: 180px;
        place-items: center;
        border: 1px dashed var(--border);
        border-radius: 11px;
        background: var(--accent-subtle);
        text-align: center;
    }
    .column:nth-of-type(2) .scroll > h3,
    .column:nth-of-type(3) .scroll > h3 {
        margin-top: 8px;
        font-size: 11px;
        letter-spacing: 0.65px;
        color: var(--muted);
        text-transform: uppercase;
    }
    .frame {
        width: min(360px, 100%);
        margin: 10px auto 14px;
        overflow: hidden;
        border: 1px solid var(--border);
        border-radius: 13px;
        background: color-mix(in srgb, var(--canvas) 72%, transparent);
        box-shadow: 0 12px 30px rgba(0, 0, 0, 0.12);
    }
    .frame :global(.action-popup) {
        border: 0;
        border-radius: 12px;
        box-shadow: none;
    }
    .priorities {
        max-height: 280px;
        padding: 12px;
        border: 1px solid var(--border);
        border-radius: 11px;
        background: color-mix(in srgb, var(--canvas) 62%, transparent);
    }
    .priorities h3 {
        margin-top: 0 !important;
        color: var(--text) !important;
        letter-spacing: -0.1px !important;
        text-transform: none !important;
    }
    .priorities h5 {
        margin: 10px 0 4px;
        font-size: 9px;
        letter-spacing: 0.9px;
        font-weight: 680;
    }
    .priorities li,
    .units li {
        min-height: 40px;
        padding: 6px 4px;
    }
    .priorities li:last-child,
    .units li:last-child {
        border-bottom: 0;
    }
    .priorities select {
        min-height: 28px;
        border-radius: 7px;
    }
    .controls {
        margin: 0 14px;
        padding: 10px;
        border: 1px solid var(--border);
        border-radius: 11px;
        background: color-mix(in srgb, var(--canvas) 62%, transparent);
    }
    .controls button {
        min-height: 30px;
        padding: 5px 9px;
        font-family: ui-monospace, 'SFMono-Regular', Menlo, monospace;
        font-size: 10px;
        box-shadow: none;
    }
    .controls button:first-child {
        color: var(--accent);
        border-color: color-mix(in srgb, var(--accent) 26%, var(--control-border));
        background: var(--accent-soft);
    }
    .reason {
        margin: 0 14px;
        min-height: 36px;
        padding: 7px 10px;
        font-family: ui-monospace, 'SFMono-Regular', Menlo, monospace;
        background: color-mix(in srgb, var(--control) 92%, var(--canvas));
    }
    .units {
        margin: 0 14px;
        padding: 5px 10px;
        border: 1px solid var(--border);
        border-radius: 10px;
        background: color-mix(in srgb, var(--canvas) 62%, transparent);
    }
    .log {
        margin: 0 14px 14px;
        padding: 0;
        border: 1px solid var(--border);
        border-radius: 11px;
        background: color-mix(in srgb, var(--canvas) 72%, transparent);
    }
    .log details {
        padding: 0;
    }
    .log summary {
        min-height: 36px;
        padding: 9px 11px;
        cursor: pointer;
        color: var(--muted);
        font-family: ui-monospace, 'SFMono-Regular', Menlo, monospace;
        font-size: 10px;
        font-weight: 560;
    }
    .log pre {
        margin: 0;
        padding: 10px 12px;
        border-top: 1px solid var(--border);
        color: color-mix(in srgb, var(--text) 88%, var(--accent));
        background: color-mix(in srgb, var(--panel) 62%, transparent);
        font-family: ui-monospace, 'SFMono-Regular', Menlo, monospace;
        line-height: 1.55;
    }
    .log > .muted {
        padding: 18px;
        line-height: 1.6;
    }
    .toggle {
        align-items: center;
        padding: 5px 8px;
        border: 1px solid var(--border);
        border-radius: 999px;
        background: var(--control);
        color: var(--muted);
        font-size: 10px;
    }
    .toggle input {
        min-height: 0;
    }

    @media (hover: hover) {
        .column {
            transition-property: border-color, box-shadow;
            transition-duration: 150ms;
        }
        .column:hover {
            border-color: color-mix(in srgb, var(--accent) 16%, var(--border));
            box-shadow:
                0 2px 4px rgba(0, 0, 0, 0.04),
                0 16px 40px rgba(0, 0, 0, 0.08);
        }
    }

    @media (max-width: 1100px) {
        .lab {
            grid-template-columns: minmax(330px, 1.15fr) minmax(350px, 0.85fr);
            grid-template-rows: 58px minmax(420px, 1fr) minmax(360px, 0.8fr);
            height: auto;
            min-height: 100vh;
            overflow: auto;
        }
        .column:nth-of-type(3) {
            grid-column: 1 / -1;
        }
        .lab-title .muted {
            display: none;
        }
    }

    @media (max-width: 760px) {
        .lab {
            display: flex;
            flex-direction: column;
            gap: 12px;
            padding: 12px;
        }
        .lab-header {
            min-height: 48px;
        }
        .lab-status {
            display: none;
        }
        .notice {
            max-width: 42vw;
        }
        .column {
            min-height: 500px;
        }
        .column:nth-of-type(2) {
            min-height: 680px;
        }
        .head {
            flex-wrap: wrap;
        }
        .head > select {
            width: 100%;
        }
    }
</style>
