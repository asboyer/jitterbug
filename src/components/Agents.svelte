<script lang="ts">
    import { onMount } from 'svelte';
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import HarnessBadge from './HarnessBadge.svelte';
    import Icon from './Icon.svelte';
    import AgentSessionSheet from './AgentSessionSheet.svelte';
    import { ago, errorMessage } from '../lib/format';
    import { cpuShare, elapsed, size, type AgentsBrief, type AgentsView } from '../lib/agents';

    const desktop = isTauri();
    let view = $state<AgentsView | null>(null);
    let brief = $state<AgentsBrief | null>(null);
    let briefBusy = $state(false);
    let error = $state('');
    let busy = $state(false);
    let now = $state(Math.floor(Date.now() / 1000));
    let selected = $state<string[]>([]);
    let connectedAt = $state<number | null>(null);
    let setupOpen = $state(false);
    // The conversation whose details are open in the side sheet.
    let openSession = $state<string | null>(null);
    let lastSignature = '';
    let lastAsked = 0;

    const connected = $derived(view?.harnesses.filter((h) => h.connected) ?? []);
    const available = $derived(view?.harnesses.filter((h) => h.installed && !h.connected) ?? []);
    const showSetup = $derived(!!view && (setupOpen || connectedAt !== null));
    const working = $derived(view?.sessions.filter((s) => s.state === 'working') ?? []);
    const quiet = $derived(view?.sessions.filter((s) => s.state !== 'working') ?? []);

    async function refresh() {
        if (!desktop) return;
        try {
            view = await invoke<AgentsView>('get_agents');
            error = '';
        } catch (e) {
            error = errorMessage(e);
        }
    }

    async function explain() {
        if (!view || !working.length) return;
        if (!desktop) return;
        briefBusy = true;
        try {
            brief = await invoke<AgentsBrief>('explain_agents', { force: false });
        } catch {
            // Notes are optional; each conversation's activity stands on its own.
        } finally {
            briefBusy = false;
            lastAsked = Date.now();
        }
    }

    // Ask the model only when what the working agents are doing changes, at most every 20
    // seconds. A quiet computer costs nothing.
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
            view = await invoke<AgentsView>('connect_agents', { keys: selected });
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
            view = await invoke<AgentsView>('disconnect_agents', { keys: [key] });
            await refresh();
        } catch (e) {
            error = errorMessage(e);
        } finally {
            busy = false;
        }
    }

    async function toggleTurns(enabled: boolean) {
        if (desktop) {
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
        const poll = setInterval(() => void refresh(), 3000);
        const clock = setInterval(() => (now = Math.floor(Date.now() / 1000)), 1000);
        return () => {
            clearInterval(poll);
            clearInterval(clock);
        };
    });
</script>

<section class="agents" aria-label="Agents" data-tour="agents">
    <div class="heading">
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
        {:else}<span></span>{/if}
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
                            ? 'Tests and builds run one at a time'
                            : 'Off: everything runs at once'}</small
                    >
                </span>
            </label>
        {:else if view && available.length}
            <button class="primary" onclick={() => (setupOpen = true)}>Let agents take turns</button
            >
        {/if}
    </div>

    {#if !view && !error}<p class="muted" role="status">Reading agent activity…</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}

    {#if showSetup && view}
        <section class="panel setup" aria-label="Connect your agents">
            {#if connectedAt === null}
                <div class="setup-head">
                    <h2>Let your agents take turns</h2>
                    <p class="muted">
                        Connected agents run tests and builds one at a time instead of all at once,
                        and keep working while they wait.
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
                    <span class="muted">Each file is backed up first. Disconnect anytime.</span>
                    <div class="actions">
                        <button onclick={() => (setupOpen = false)}>Cancel</button>
                        <button
                            class="primary"
                            disabled={busy || !selected.length}
                            onclick={connect}
                        >
                            {busy ? 'Connecting…' : 'Connect'}
                        </button>
                    </div>
                </div>
            {:else}
                <div class="setup-head">
                    <h2>Connected</h2>
                    <p class="muted">
                        Open sessions pick this up when restarted. New sessions use it right away.
                    </p>
                </div>
                {#each connected as harness (harness.key)}
                    {@const contact = heard(harness.last_seen)}
                    <div class="harness-row">
                        <span class="check" class:live={!!contact}
                            >{#if contact}<Icon name="check" size={18} />{:else}<Icon
                                    name="activity"
                                    size={15}
                                />{/if}</span
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
                    <span></span>
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

    {#if view && view.line.length}
        <section class="panel" aria-label="Tests and builds taking turns">
            <h2>Tests and builds</h2>
            {#each view.line as item, index (item.command + item.agent + item.harness + index)}
                <div class="turn" class:running={item.state === 'running'}>
                    <span class="slot"
                        >{#if item.state === 'running'}<i class="pulse"></i>{:else}{index -
                                view.line.filter((l) => l.state === 'running').length +
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
                    <span class="usage"
                        >{#if item.state === 'running'}CPU {cpuShare(item.cpu_percent)} · {size(
                                item.memory_bytes
                            )}{/if}</span
                    >
                </div>
            {/each}
        </section>
    {/if}

    {#each working as session (session.id)}
        <section
            class="panel conversation"
            aria-label={`${session.harness_name}: ${session.title}`}
        >
            <button
                class="conversation-head"
                aria-label={`Details for ${session.title || session.folder}`}
                onclick={() => (openSession = session.id)}
            >
                <HarnessBadge harness={session.harness} compact />
                <span class="name">
                    <strong>{session.title || session.folder}</strong>
                    <span class="muted">{session.harness_name} · {session.folder}</span>
                </span>
                <span class="usage" title="Share of this computer's CPU, and memory in use"
                    >CPU {cpuShare(session.cpu_percent)} · {size(session.memory_bytes)}</span
                >
                <span class="details-hint">Details ›</span>
            </button>
            <p class="doing"><i class="pulse"></i>{session.doing ?? 'Working'}</p>
            {#if session.request}<p class="request">“{session.request}”</p>{/if}
            {#if brief?.notes[session.id] && (session.cpu_percent >= 5 || session.agents.length)}<p
                    class="note"
                >
                    {brief.notes[session.id]}
                </p>{/if}
            {#each session.agents as agent, index (agent.label + index)}
                <div class="subagent">
                    <span class="sub-label">{agent.label || 'Main conversation'}</span>
                    <span class="sub-doing muted"
                        >{#if agent.state === 'waiting'}Waiting to run <code>{agent.command}</code
                            >{#if agent.position}
                                · #{agent.position} in line{/if}{:else if agent.state === 'running'}Running
                            <code>{agent.command}</code>{:else}{agent.doing ?? 'Working'}{/if}</span
                    >
                    <span class="usage"
                        >{#if agent.state === 'running'}CPU {cpuShare(agent.cpu_percent)} · {size(
                                agent.memory_bytes
                            )}{/if}</span
                    >
                </div>
            {/each}
        </section>
    {/each}

    {#if view && quiet.length}
        <section class="panel" aria-label="Other running sessions">
            <h2>Other running sessions</h2>
            <div class="quiet-head" aria-hidden="true">
                <span></span><span>Conversation</span><span>Last active</span><span>Memory</span
                ><span></span>
            </div>
            {#each quiet as session (session.id)}
                <button class="quiet-row" onclick={() => (openSession = session.id)}>
                    <HarnessBadge harness={session.harness} compact />
                    <span class="name"
                        ><strong
                            >{session.title || session.folder}<em
                                class:waiting={session.state === 'waiting'}
                                >{session.state === 'waiting' ? 'Needs your answer' : 'Done'}</em
                            ></strong
                        ><span class="muted"
                            >{session.asks
                                ? `Asked: “${session.asks}”`
                                : session.request
                                  ? `You: “${session.request}”`
                                  : `${session.harness_name} · ${session.folder}`}</span
                        ></span
                    >
                    <span class="muted when"
                        >{session.last_active ? ago(session.last_active, now) : '—'}</span
                    >
                    <span class="usage" title="Memory still in use"
                        >{size(session.memory_bytes)}</span
                    >
                    <span class="chevron" aria-hidden="true">›</span>
                </button>
            {/each}
        </section>
    {/if}

    {#if view && !view.sessions.length}
        <section class="panel empty-state">
            <Icon name="code" size={22} />
            <h2>No coding agents are running</h2>
            <p class="muted">
                Start Claude Code, Codex, or Muse Code and they appear here with what they're doing.
            </p>
        </section>
    {/if}

    {#if view}
        <div class="footer muted">
            <span
                >Other apps: {view.other.labels.join(', ') || 'nothing busy'} · CPU {cpuShare(
                    view.other.cpu_percent
                )}</span
            >
            {#if connected.length}
                <div class="connections" role="group" aria-label="Connected agents">
                    <span class="connections-label">Connected</span>
                    {#each connected as harness (harness.key)}
                        <span class="connection"
                            ><HarnessBadge harness={harness.key} compact /><span
                                >{harness.name}</span
                            ><button
                                class="disconnect"
                                aria-label={`Disconnect ${harness.name}`}
                                title={`Disconnect ${harness.name}`}
                                disabled={busy}
                                onclick={() => disconnect(harness.key)}
                                ><svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true"
                                    ><path
                                        d="M1.5 1.5l5 5m0-5-5 5"
                                        stroke="currentColor"
                                        stroke-width="1.4"
                                        stroke-linecap="round"
                                    /></svg
                                ></button
                            ></span
                        >
                    {/each}
                    {#if available.length}<button
                            class="connect-more"
                            onclick={() => (setupOpen = true)}>Connect another…</button
                        >{/if}
                </div>
            {/if}
        </div>
    {/if}
</section>
{#if openSession}<AgentSessionSheet id={openSession} onclose={() => (openSession = null)} />{/if}

<style>
    .agents {
        display: flex;
        flex-direction: column;
        gap: 24px;
        max-width: none;
    }
    .heading {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 16px;
    }
    .turns-switch {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 6px 12px;
        border-radius: 8px;
        background: var(--raised);
        cursor: default;
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
        gap: 28px;
        flex: 1;
        max-width: 620px;
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
        padding: 20px;
        border: 0;
        border-radius: 6px;
        box-shadow: none;
    }
    h2 {
        font-size: 13px;
        margin: 0 0 6px;
    }
    .setup {
        border: 1px solid var(--border);
        display: flex;
        flex-direction: column;
        gap: 4px;
    }
    .setup-head {
        margin-bottom: 6px;
    }
    .setup-head h2 {
        font-size: 16px;
        margin: 0 0 4px;
    }
    .setup-head p {
        margin: 0;
    }
    .harness-row {
        display: flex;
        align-items: flex-start;
        gap: 10px;
        padding: 10px 0;
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

        font-size: 12px;
    }
    .actions {
        display: flex;
        gap: 8px;
    }
    .turn,
    .subagent,
    .quiet-row {
        display: grid;
        align-items: center;
        gap: 10px;
        padding: 7px 0;

        font-size: 13px;
    }
    .turn {
        grid-template-columns: 22px minmax(100px, auto) 1fr 110px 150px;
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
        display: inline-block;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: var(--success);

        flex: none;
    }
    code {
        font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
        font-size: 12px;
        background: var(--canvas);
        border-radius: 5px;
        padding: 1px 6px;
        white-space: nowrap;
    }
    .turn code {
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
    .usage {
        text-align: right;
        font-variant-numeric: tabular-nums;
        font-size: 12px;
        color: var(--muted);
        white-space: nowrap;
    }
    .conversation {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .conversation-head {
        display: flex;
        align-items: center;
        gap: 10px;
    }
    .conversation-head .name {
        display: flex;
        flex-direction: column;
        flex: 1;
        min-width: 0;
    }
    .conversation-head .name strong {
        font-size: 14px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .conversation-head .name span {
        font-size: 12px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .doing {
        display: flex;
        align-items: center;
        gap: 8px;
        margin: 2px 0 0;
        font-size: 14px;
    }
    .request,
    .note {
        margin: 0;
        font-size: 12px;
        color: var(--muted);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .subagent {
        grid-template-columns: minmax(140px, 0.9fr) minmax(0, 1.4fr) 150px;
    }
    .sub-label,
    .sub-doing {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .sub-doing {
        font-size: 12px;
    }
    .quiet-row,
    .quiet-head {
        grid-template-columns: 30px minmax(0, 1fr) 96px 72px 12px;
    }
    .quiet-head {
        display: grid;
        gap: 10px;
        padding: 0 8px 6px;
        font-size: 11px;
        color: var(--faint);
    }
    .quiet-row {
        width: 100%;
        min-height: 0;
        padding: 8px;
        margin: 0;
        border-radius: 8px;
        background: transparent;
        box-shadow: none;
        text-align: left;
        font-weight: 400;
    }
    .quiet-row:hover:not(:disabled) {
        background: var(--hover);
    }
    .quiet-row .name {
        display: grid;
        gap: 2px;
        min-width: 0;
    }
    .quiet-row .name strong {
        display: flex;
        align-items: center;
        gap: 8px;
        min-width: 0;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .quiet-row .name em {
        flex: none;
        font-style: normal;
        font-size: 11px;
        font-weight: 500;
        color: var(--faint);
    }
    .quiet-row .name em.waiting {
        color: var(--brand);
    }
    .quiet-row .name .muted {
        font-size: 12px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .quiet-row .when,
    .quiet-row .usage {
        font-size: 12px;
        font-variant-numeric: tabular-nums;
    }
    .chevron {
        color: var(--faint);
        font-size: 15px;
    }
    .conversation-head {
        width: 100%;
        min-height: 0;
        padding: 0;
        background: transparent;
        box-shadow: none;
        text-align: left;
        font-weight: 400;
    }
    .conversation-head:hover:not(:disabled) {
        background: transparent;
    }
    .details-hint {
        font-size: 12px;
        color: var(--brand);
        opacity: 0;
        transition: opacity 0.12s;
    }
    .conversation-head:hover .details-hint,
    .conversation-head:focus-visible .details-hint {
        opacity: 1;
    }
    .empty-state {
        text-align: left;
        padding: 32px;
    }
    .empty-state h2 {
        margin: 8px 0 4px;
        font-size: 14px;
    }
    .footer {
        display: flex;
        justify-content: space-between;
        align-items: flex-start;
        gap: 12px;
        flex-wrap: wrap;
        font-size: 12px;
    }
    .connections {
        display: flex;
        align-items: center;
        gap: 6px;
        flex-wrap: wrap;
        justify-content: flex-end;
    }
    .connections-label {
        color: var(--faint);
        margin-right: 2px;
    }
    /* The badge's tile is for cards; inside a chip only the mark itself shows. */
    .connection :global(.harness-badge) {
        padding: 0;
        background: none;
    }
    .connection :global(.harness-badge .logo) {
        width: 14px;
        height: 14px;
    }
    .connection {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        height: 26px;
        padding: 0 4px 0 9px;
        border-radius: 13px;
        background: #ffffff0d;
        box-shadow: inset 0 0 0 0.5px #ffffff1a;
        color: var(--text);
    }
    .disconnect {
        display: grid;
        place-items: center;
        width: 18px;
        height: 18px;
        min-height: 0;
        padding: 0;
        border-radius: 50%;
        background: transparent;
        box-shadow: none;
        color: var(--faint);
    }
    .disconnect:hover:not(:disabled) {
        background: #ffffff1f;
        color: var(--text);
    }
    .connect-more {
        min-height: 26px;
        padding: 0 10px;
        font-size: 12px;
    }
    @media (max-width: 760px) {
        .gauges {
            grid-template-columns: 1fr;
        }
        .turn,
        .subagent,
        .quiet-row {
            grid-template-columns: 1fr auto;
        }
        .turn .who,
        .turn .when,
        .subagent .sub-doing,
        .quiet-row .when {
            display: none;
        }
    }
</style>
