<script lang="ts">
    import { onMount } from 'svelte';
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import HarnessBadge from './HarnessBadge.svelte';
    import Icon from './Icon.svelte';
    import { memory, percent, errorMessage } from '../lib/format';
    import {
        demoAgents,
        demoBrief,
        elapsed,
        rowId,
        type AgentsBrief,
        type AgentsView,
    } from '../lib/agents';

    let { demo }: { demo: boolean } = $props();

    const preview = $derived(demo || !isTauri());
    let view = $state<AgentsView | null>(null);
    let brief = $state<AgentsBrief | null>(null);
    let briefError = $state('');
    let briefBusy = $state(false);
    let error = $state('');
    let busy = $state(false);
    let now = $state(Math.floor(Date.now() / 1000));
    let selected = $state<string[]>([]);
    let connectedAt = $state<number | null>(null);
    let setupOpen = $state(false);
    // Demo only: whether the synthetic agents are connected, and whether turns have kicked in.
    let demoConnected = $state(false);
    let demoStep = $state(0);
    let lastSignature = '';
    let lastAsked = 0;

    const connected = $derived(view?.harnesses.filter((h) => h.connected) ?? []);
    const available = $derived(view?.harnesses.filter((h) => h.installed && !h.connected) ?? []);
    const showSetup = $derived(
        !!view && (setupOpen || connected.length === 0 || connectedAt !== null)
    );
    const heavy = $derived(view?.line.filter((l) => l.state === 'running') ?? []);
    const waiting = $derived(view?.line.filter((l) => l.state === 'waiting') ?? []);
    const title = $derived.by(() => {
        if (!view) return 'Finding your agents…';
        const agents = view.sessions.reduce((t, s) => t + s.agents.length, 0);
        if (!view.sessions.length) return 'No coding agents running';
        const parts = [`${agents} agent${agents === 1 ? '' : 's'}`];
        if (view.line.length)
            parts.push(`${view.line.length} heavy command${view.line.length === 1 ? '' : 's'}`);
        return parts.join(', ');
    });

    async function refresh() {
        if (preview) {
            view = demoAgents(demoStep, demoConnected);
            return;
        }
        try {
            view = await invoke<AgentsView>('get_agents');
            error = '';
        } catch (e) {
            error = errorMessage(e);
        }
    }

    async function explain(force = false) {
        if (!view || !view.sessions.length) return;
        if (preview) {
            brief = demoBrief(view);
            return;
        }
        briefBusy = true;
        try {
            brief = await invoke<AgentsBrief>('explain_agents', { force });
            briefError = '';
        } catch (e) {
            briefError = errorMessage(e);
        } finally {
            briefBusy = false;
            lastAsked = Date.now();
        }
    }

    // Ask the model again only when the agents or their heavy commands change, at most every
    // 20 seconds. Steady state costs nothing.
    $effect(() => {
        const signature = view?.signature ?? '';
        if (!signature || signature === lastSignature || briefBusy) return;
        if (Date.now() - lastAsked < 20_000 && brief) return;
        lastSignature = signature;
        void explain();
    });

    $effect(() => {
        if (view && !selected.length && available.length) selected = available.map((h) => h.key);
    });

    async function connect() {
        if (!selected.length) return;
        busy = true;
        error = '';
        try {
            if (preview) {
                demoConnected = true;
                setTimeout(() => (demoStep = 1), 2500);
            } else {
                view = await invoke<AgentsView>('connect_agents', { keys: selected });
            }
            connectedAt = Math.floor(Date.now() / 1000);
            selected = [];
            await refresh();
        } catch (e) {
            error = errorMessage(e);
        } finally {
            busy = false;
        }
    }

    async function disconnect(key: string) {
        busy = true;
        try {
            if (preview) demoConnected = false;
            else view = await invoke<AgentsView>('disconnect_agents', { keys: [key] });
            await refresh();
        } catch (e) {
            error = errorMessage(e);
        } finally {
            busy = false;
        }
    }

    async function toggleTurns(enabled: boolean) {
        if (preview) {
            demoConnected = enabled;
            demoStep = enabled ? 1 : 0;
        } else {
            try {
                await invoke('set_turns_enabled', { enabled });
            } catch (e) {
                error = errorMessage(e);
            }
        }
        await refresh();
    }

    function heard(lastSeen: number | null) {
        if (!lastSeen) return null;
        if (connectedAt && lastSeen < connectedAt) return null;
        return `heard from it ${elapsed(lastSeen, now)} ago`;
    }

    onMount(() => {
        void refresh();
        const poll = setInterval(() => void refresh(), preview ? 2500 : 4000);
        const clock = setInterval(() => (now = Math.floor(Date.now() / 1000)), 1000);
        return () => {
            clearInterval(poll);
            clearInterval(clock);
        };
    });
</script>

<section class="agents" aria-label="Agents" data-tour="agents">
    <div class="heading">
        <div>
            <span class="eyebrow">AGENTS</span>
            <h1>{title}</h1>
            <p class="muted">
                Every coding agent on this computer, what its subagents are running, and heavy
                commands taking turns.
            </p>
        </div>
        {#if view && connected.length}
            <label class="turns-switch">
                <input
                    type="checkbox"
                    checked={view.enabled}
                    onchange={(e) => toggleTurns(e.currentTarget.checked)}
                />
                <span>
                    <strong>Take turns</strong>
                    <small
                        >{view.enabled
                            ? 'One heavy command at a time'
                            : 'Off: everything runs at once'}</small
                    >
                </span>
            </label>
        {/if}
    </div>

    {#if view}
        <div class="gauges" aria-label="Whole computer">
            {#each [['CPU', view.cpu_percent], ['Memory', view.memory_used_percent]] as [label, value]}
                <div class="gauge">
                    <span>{label}</span>
                    <div class="track" class:hot={(value as number) >= 80}>
                        <i style={`width:${Math.min(100, value as number)}%`}></i>
                    </div>
                    <strong>{Math.round(value as number)}%</strong>
                </div>
            {/each}
        </div>
    {/if}

    {#if error}<p class="error" role="alert">{error}</p>{/if}

    {#if showSetup && view}
        <section class="panel setup" aria-label="Connect your agents">
            {#if connectedAt === null}
                <div class="setup-head">
                    <span class="step">1 of 2</span>
                    <h2>
                        {available.length
                            ? `Found ${available.length} coding agent${available.length === 1 ? '' : 's'} on this computer`
                            : 'No coding agents found yet'}
                    </h2>
                    <p class="muted">
                        Connect them and their heavy commands (tests, builds) take turns instead of
                        all running at once. Agents keep working while they wait.
                    </p>
                </div>
                {#each view.harnesses.filter((h) => h.installed) as harness (harness.key)}
                    <div class="harness-row">
                        <input
                            type="checkbox"
                            aria-label={`Connect ${harness.name}`}
                            disabled={harness.connected}
                            checked={harness.connected || selected.includes(harness.key)}
                            onchange={(e) =>
                                (selected = e.currentTarget.checked
                                    ? [...selected, harness.key]
                                    : selected.filter((k) => k !== harness.key))}
                        />
                        <HarnessBadge harness={harness.key} compact />
                        <div class="harness-copy">
                            <strong>{harness.name}</strong>
                            <span class="muted"
                                >{harness.version
                                    ? `v${harness.version} · `
                                    : ''}{harness.config}</span
                            >
                            <details>
                                <summary>See exactly what changes</summary>
                                <pre>{harness.entry}</pre>
                            </details>
                        </div>
                        {#if harness.connected}<span class="ok">Connected</span>{/if}
                    </div>
                {/each}
                <div class="setup-foot">
                    <span class="muted"
                        >A backup of each file is saved first. Disconnect anytime below.</span
                    >
                    <div class="actions">
                        {#if connected.length}<button onclick={() => (setupOpen = false)}
                                >Cancel</button
                            >{/if}
                        <button
                            class="primary"
                            disabled={busy || !selected.length}
                            onclick={connect}
                        >
                            {busy
                                ? 'Connecting…'
                                : `Connect ${selected.length || ''} agent${selected.length === 1 ? '' : 's'}`}
                        </button>
                    </div>
                </div>
            {:else}
                <div class="setup-head">
                    <span class="step">2 of 2</span>
                    <h2>Connected. Checking each agent…</h2>
                    <p class="muted">
                        Sessions that were already open pick this up when you restart them. New
                        sessions use it right away.
                    </p>
                </div>
                {#each connected as harness (harness.key)}
                    {@const contact = heard(harness.last_seen)}
                    <div class="harness-row">
                        <span class="check" class:live={!!contact}
                            >{#if contact}✓{:else}<Icon name="activity" size={15} />{/if}</span
                        >
                        <HarnessBadge harness={harness.key} compact />
                        <div class="harness-copy">
                            <strong>{harness.name}</strong>
                            <span class="muted"
                                >{contact ??
                                    (harness.note || 'Waiting for its first command')}</span
                            >
                        </div>
                    </div>
                {/each}
                <div class="setup-foot">
                    <span class="muted">Heavy commands now line up below.</span>
                    <button
                        class="primary"
                        onclick={() => {
                            connectedAt = null;
                            setupOpen = false;
                        }}>Done</button
                    >
                </div>
            {/if}
        </section>
    {/if}

    {#if view && view.sessions.length}
        <section class="panel brief" aria-live="polite">
            <div class="brief-head">
                <Icon name="companion" size={16} />
                <span
                    >{brief && brief.provider !== 'demo'
                        ? `${brief.provider === 'meta' ? 'Meta' : brief.provider} · inferred from live processes`
                        : preview
                          ? 'Demo · synthetic agents'
                          : 'AI read of your agents'}</span
                >
                <button
                    class="text-button"
                    disabled={briefBusy}
                    onclick={() => explain(true)}
                    aria-label="Explain again">{briefBusy ? 'Thinking…' : 'Refresh'}</button
                >
            </div>
            {#if brief}
                <p class="headline">{brief.headline}</p>
            {:else if briefError}
                <p class="muted">{briefError}</p>
            {:else}
                <p class="muted">Reading what your agents are doing…</p>
            {/if}
        </section>
    {/if}

    {#if view}
        <section class="panel line" aria-label="Heavy commands taking turns">
            <div class="section-head">
                <h2>Taking turns</h2>
                <span class="muted">{heavy.length} running · {waiting.length} waiting</span>
            </div>
            {#if !view.line.length}
                <p class="muted empty">
                    {connected.length
                        ? 'No heavy commands right now. Tests and builds from your agents line up here.'
                        : 'Connect your agents to see their tests and builds line up here.'}
                </p>
            {/if}
            {#each view.line as item, index (item.command + item.agent + item.harness + index)}
                <div class="turn" class:running={item.state === 'running'}>
                    <span class="slot"
                        >{#if item.state === 'running'}<i class="pulse"></i>{:else}#{index -
                                heavy.length +
                                1}{/if}</span
                    >
                    <code>{item.command}</code>
                    <span class="who">
                        <HarnessBadge harness={item.harness} compact />
                        {item.agent || item.harness_name}
                    </span>
                    <span class="muted when"
                        >{item.state === 'running' ? 'running' : 'waiting'}
                        {elapsed(item.since, now)}</span
                    >
                    <span class="num"
                        >{item.state === 'running' ? percent(item.cpu_percent) : ''}</span
                    >
                    <span class="num muted"
                        >{item.state === 'running' && item.memory_bytes
                            ? memory(item.memory_bytes)
                            : ''}</span
                    >
                </div>
            {/each}
        </section>

        {#each view.sessions as session (session.id)}
            <section class="panel session" aria-label={`${session.harness_name} session`}>
                <div class="session-head">
                    <HarnessBadge harness={session.harness} />
                    <strong class="session-title">{session.title}</strong>
                    <span class="muted folder">{session.folder}</span>
                    <span class="num">{percent(session.cpu_percent)}</span>
                    <span class="num muted">{memory(session.memory_bytes)}</span>
                </div>
                {#each session.agents as agent, index (index)}
                    {@const note = brief?.notes[rowId(session, index)]}
                    <div class="agent">
                        <div class="agent-main">
                            <span class="agent-label"
                                >{agent.label}{#if agent.subagent}<small>subagent</small>{/if}</span
                            >
                            <span class={`state ${agent.state}`}
                                >{agent.state === 'waiting' && agent.position
                                    ? `waiting · #${agent.position}`
                                    : agent.state}</span
                            >
                            {#if agent.command}<code>{agent.command}</code>{/if}
                            <span class="muted when">{elapsed(agent.since, now)}</span>
                            <span class="num">{percent(agent.cpu_percent)}</span>
                            <span class="num muted">{memory(agent.memory_bytes)}</span>
                        </div>
                        {#if note}<p class="note">
                                <Icon name="companion" size={12} />
                                {note}
                            </p>{/if}
                    </div>
                {/each}
            </section>
        {/each}

        {#if !view.sessions.length}
            <section class="panel empty-state">
                <Icon name="code" size={22} />
                <h2>No coding agents are running</h2>
                <p class="muted">
                    Start Claude Code, Codex, or Muse Code and they appear here with what they're
                    using.
                </p>
            </section>
        {/if}

        {#if view.recent.length}
            <details class="panel recent">
                <summary>Recent heavy commands · {view.recent.length}</summary>
                {#each view.recent as item}
                    <div class="turn">
                        <span class="slot">✓</span>
                        <code>{item.command}</code>
                        <span class="who">{item.agent || item.harness}</span>
                        <span class="muted when"
                            >{elapsed(item.started_at, item.ended_at)}{item.waited_seconds
                                ? ` · waited ${elapsed(item.ended_at - item.waited_seconds, item.ended_at)}`
                                : ''}</span
                        >
                        <span class="num">{percent(item.peak_cpu_percent)}</span>
                        <span class="num muted"
                            >{item.peak_memory_bytes ? memory(item.peak_memory_bytes) : ''}</span
                        >
                    </div>
                {/each}
            </details>
        {/if}

        <div class="footer">
            <span class="muted"
                >Not from an agent: {view.other.labels.join(', ') || 'nothing busy'} · {percent(
                    view.other.cpu_percent
                )} CPU</span
            >
            <div class="connections">
                {#each connected as harness (harness.key)}
                    <span class="chip">
                        <HarnessBadge harness={harness.key} compact />
                        {harness.name}
                        <button
                            class="text-button"
                            disabled={busy}
                            onclick={() => disconnect(harness.key)}
                            aria-label={`Disconnect ${harness.name}`}>Disconnect</button
                        >
                    </span>
                {/each}
                {#if connected.length && available.length}<button
                        class="text-button"
                        onclick={() => (setupOpen = true)}>Connect more</button
                    >{/if}
            </div>
        </div>
    {/if}
</section>

<style>
    .agents {
        display: flex;
        flex-direction: column;
        gap: 14px;
        max-width: 980px;
    }
    .heading {
        display: flex;
        justify-content: space-between;
        align-items: flex-start;
        gap: 16px;
    }
    .heading h1 {
        margin: 2px 0 4px;
    }
    .heading p {
        margin: 0;
    }
    .turns-switch {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 8px 12px;
        border: 1px solid var(--border);
        border-radius: 10px;
        background: var(--panel);
        cursor: pointer;
        white-space: nowrap;
    }
    .turns-switch span {
        display: flex;
        flex-direction: column;
    }
    .turns-switch small {
        color: var(--muted);
        font-size: 11px;
    }
    .gauges {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 18px;
    }
    .gauge {
        display: grid;
        grid-template-columns: 64px 1fr 44px;
        align-items: center;
        gap: 10px;
        font-size: 12px;
        color: var(--muted);
    }
    .gauge strong {
        color: var(--text);
        text-align: right;
        font-variant-numeric: tabular-nums;
    }
    .track {
        height: 6px;
        border-radius: 3px;
        background: var(--track);
        overflow: hidden;
    }
    .track i {
        display: block;
        height: 100%;
        background: var(--accent);
        transition: width 0.6s ease;
    }
    .track.hot i {
        background: var(--warning);
    }
    .error {
        color: var(--danger);
        margin: 0;
    }
    .panel {
        padding: 14px 16px;
    }
    h2 {
        font-size: 14px;
        margin: 0;
    }
    .setup {
        border: 1px solid var(--accent);
        display: flex;
        flex-direction: column;
        gap: 4px;
    }
    .setup-head {
        margin-bottom: 6px;
    }
    .setup-head h2 {
        font-size: 16px;
        margin: 4px 0;
    }
    .setup-head p {
        margin: 0;
    }
    .step {
        font-size: 11px;
        color: var(--accent);
    }
    .harness-row {
        display: flex;
        align-items: flex-start;
        gap: 10px;
        padding: 10px 0;
        border-top: 1px solid var(--border);
    }
    .harness-copy {
        display: flex;
        flex-direction: column;
        gap: 2px;
        flex: 1;
        min-width: 0;
        font-size: 12px;
    }
    .harness-copy strong {
        font-size: 13px;
    }
    details summary {
        cursor: pointer;
        color: var(--muted);
        font-size: 12px;
    }
    pre {
        font-size: 11px;
        background: var(--canvas);
        border-radius: 6px;
        padding: 8px;
        margin: 6px 0 0;
        white-space: pre-wrap;
        word-break: break-all;
        max-height: 220px;
        overflow: auto;
    }
    .ok {
        color: var(--success);
        font-size: 12px;
    }
    .check {
        color: var(--muted);
        display: inline-flex;
        margin-top: 2px;
    }
    .check.live {
        color: var(--success);
    }
    .setup-foot {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 12px;
        padding-top: 10px;
        border-top: 1px solid var(--border);
        font-size: 12px;
    }
    .actions {
        display: flex;
        gap: 8px;
    }
    .brief {
        display: flex;
        flex-direction: column;
        gap: 4px;
        background: var(--accent-soft);
        border-color: transparent;
    }
    .brief-head {
        display: flex;
        align-items: center;
        gap: 6px;
        font-size: 12px;
        color: var(--muted);
    }
    .brief-head button {
        margin-left: auto;
    }
    .headline {
        margin: 0;
        font-size: 15px;
        line-height: 1.45;
    }
    .section-head {
        display: flex;
        justify-content: space-between;
        align-items: baseline;
        margin-bottom: 6px;
        font-size: 12px;
    }
    .empty {
        margin: 4px 0 0;
        font-size: 13px;
    }
    .turn,
    .agent-main {
        display: grid;
        grid-template-columns: 34px minmax(110px, auto) 1fr 110px 64px 74px;
        align-items: center;
        gap: 10px;
        padding: 7px 0;
        border-top: 1px solid var(--border);
        font-size: 13px;
    }
    .turn.running code {
        color: var(--success);
    }
    .slot {
        color: var(--muted);
        font-size: 12px;
        display: inline-flex;
        justify-content: center;
    }
    .pulse {
        width: 9px;
        height: 9px;
        border-radius: 50%;
        background: var(--success);
        animation: pulse 1.6s ease-in-out infinite;
    }
    @keyframes pulse {
        50% {
            opacity: 0.35;
        }
    }
    code {
        font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
        font-size: 12px;
        background: var(--canvas);
        border-radius: 5px;
        padding: 2px 6px;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        justify-self: start;
        max-width: 100%;
    }
    .who {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        min-width: 0;
        overflow: hidden;
        white-space: nowrap;
        text-overflow: ellipsis;
    }
    .when {
        font-size: 12px;
        white-space: nowrap;
    }
    .num {
        text-align: right;
        font-variant-numeric: tabular-nums;
        font-size: 12px;
        white-space: nowrap;
    }
    .session-head {
        display: grid;
        grid-template-columns: auto auto 1fr 64px 74px;
        align-items: center;
        gap: 10px;
        padding-bottom: 6px;
    }
    .session-title {
        font-size: 14px;
    }
    .folder {
        font-size: 12px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .agent-main {
        grid-template-columns: minmax(150px, 1.3fr) 104px minmax(0, 1fr) 64px 64px 74px;
    }
    .agent-label {
        display: inline-flex;
        align-items: baseline;
        gap: 6px;
        min-width: 0;
        overflow: hidden;
        white-space: nowrap;
        text-overflow: ellipsis;
    }
    .agent-label small {
        color: var(--muted);
        font-size: 10px;
    }
    .state {
        font-size: 11px;
        padding: 2px 8px;
        border-radius: 999px;
        background: var(--canvas);
        color: var(--muted);
        justify-self: start;
        white-space: nowrap;
    }
    .state.running {
        background: var(--success-soft);
        color: var(--success);
    }
    .state.waiting {
        background: var(--warning-soft);
        color: var(--warning);
    }
    .state.working {
        background: var(--accent-soft);
        color: var(--accent);
    }
    .note {
        margin: 0 0 6px;
        font-size: 12px;
        color: var(--muted);
        display: flex;
        gap: 6px;
        align-items: center;
    }
    .empty-state {
        text-align: center;
        padding: 28px;
    }
    .empty-state h2 {
        margin: 8px 0 4px;
    }
    .recent summary {
        font-size: 13px;
    }
    .footer {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 12px;
        flex-wrap: wrap;
        font-size: 12px;
    }
    .connections {
        display: flex;
        gap: 8px;
        flex-wrap: wrap;
        align-items: center;
    }
    .chip {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 3px 4px 3px 8px;
        border: 1px solid var(--border);
        border-radius: 999px;
    }
    @media (max-width: 760px) {
        .gauges {
            grid-template-columns: 1fr;
        }
        .turn,
        .agent-main {
            grid-template-columns: 1fr auto auto;
        }
        .turn .when,
        .agent-main .when,
        .turn .who {
            display: none;
        }
    }
</style>
