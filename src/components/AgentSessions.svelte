<script lang="ts">
    // Agent sessions: every command running inside terminals and editors, coding agents first,
    // found from the process tree with no setup. A new session opens a terminal window in a
    // chosen folder running an allowlisted agent.
    import { onMount } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { listen } from '@tauri-apps/api/event';
    import { errorMessage, memory, percent } from '../lib/format';

    type Session = {
        id: string;
        app: string;
        agent: string | null;
        command: string;
        task: string | null;
        folder: string;
        path: string;
        started_at: number;
        cpu_percent: number;
        memory_bytes: number;
        process_count: number;
        paused_until: number | null;
        actions: string[];
        connected: boolean;
        pid: number;
        start_time: number;
    };
    type Group = { app: string; sessions: Session[] };
    type Overview = {
        agents: Group[];
        other: Group[];
        folders: string[];
        terminals: string[];
        connected_launch: boolean;
        vscode: boolean;
        launch_supported: boolean;
        message: string;
        captured_at: number;
    };

    const agentChoices = [
        { value: 'claude', label: 'Claude Code' },
        { value: 'codex', label: 'Codex' },
        { value: 'aider', label: 'Aider' },
        { value: 'gemini', label: 'Gemini CLI' },
    ];
    const terminalLabels: Record<string, string> = { terminal: 'Terminal', iterm: 'iTerm' };

    let overview = $state<Overview | null>(null);
    let error = $state('');
    let notice = $state('');
    let busy = $state(false);
    let folder = $state('');
    let agent = $state('claude');
    let terminal = $state('terminal');
    let terminalChosen = false;
    let sequence = 0;

    const agentCount = $derived(
        overview?.agents.reduce((sum, group) => sum + group.sessions.length, 0) ?? 0
    );
    const otherCount = $derived(
        overview?.other.reduce((sum, group) => sum + group.sessions.length, 0) ?? 0
    );

    async function load() {
        const request = ++sequence;
        try {
            const next = await invoke<Overview>('get_agent_sessions');
            if (request !== sequence) return;
            overview = next;
            error = '';
            folder ||= next.folders[0] ?? '';
            // Default to iTerm when that is where agents already run.
            if (!terminalChosen && next.terminals.includes('iterm')) {
                const usesITerm = [...next.agents, ...next.other].some((g) =>
                    g.app.toLowerCase().startsWith('iterm')
                );
                terminal = usesITerm ? 'iterm' : 'terminal';
            }
        } catch (cause) {
            if (request === sequence) error = errorMessage(cause);
        }
    }

    onMount(() => {
        let stop: (() => void) | undefined;
        void (async () => {
            stop = await listen('telemetry', () => void load());
            await load();
        })().catch((cause) => (error = errorMessage(cause)));
        return () => {
            sequence++;
            stop?.();
        };
    });

    async function start() {
        busy = true;
        try {
            notice = await invoke<string>('launch_agent_session', {
                request: { folder, agent, terminal },
            });
        } catch (cause) {
            notice = errorMessage(cause);
        } finally {
            busy = false;
        }
    }

    async function openInCode() {
        busy = true;
        try {
            notice = await invoke<string>('open_folder_in_vscode', { folder });
        } catch (cause) {
            notice = errorMessage(cause);
        } finally {
            busy = false;
        }
    }

    async function control(session: Session, action: 'pause' | 'resume') {
        busy = true;
        try {
            notice = await invoke<string>('session_control', {
                pid: session.pid,
                startTime: session.start_time,
                action,
                label: name(session),
            });
            await load();
        } catch (cause) {
            notice = errorMessage(cause);
        } finally {
            busy = false;
        }
    }

    const name = (session: Session) => session.agent ?? session.command;
    // Show the command only when it is not the agent's own (a runtime or wrapper carrying it).
    const carrier = (session: Session) =>
        !!session.agent && !session.agent.toLowerCase().startsWith(session.command.toLowerCase());
    const clock = (time: number) =>
        new Date(time * 1000).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

    function running(session: Session, now: number) {
        if (!session.started_at) return '';
        const seconds = Math.max(0, now - session.started_at);
        if (seconds < 60) return 'just started';
        const minutes = Math.floor(seconds / 60);
        if (minutes < 60) return `${minutes} min`;
        const hours = Math.floor(minutes / 60);
        if (hours < 24) return `${hours} h ${minutes % 60} min`;
        return `${Math.floor(hours / 24)} d ${hours % 24} h`;
    }
</script>

{#snippet list(groups: Group[])}
    {#each groups as group (group.app)}
        <h4>{group.app}</h4>
        <div class="rows" role="list">
            {#each group.sessions as session (session.id)}
                <div class="row" role="listitem">
                    <div class="name">
                        <strong title={session.task ?? name(session)}
                            >{#if session.connected}<span
                                    class="badge"
                                    title="Registered with Jitterbug as an agent task"
                                    >Connected</span
                                >{/if}{name(session)}{#if session.task}<span class="task">
                                    · {session.task}</span
                                >{/if}</strong
                        >
                        <small title={session.path}
                            ><span class:paused={!!session.paused_until}
                                >{session.paused_until
                                    ? `Paused until ${clock(session.paused_until)}`
                                    : 'Running'}</span
                            >
                            · {session.folder || 'folder unknown'}{carrier(session)
                                ? ` · via ${session.command}`
                                : ''}</small
                        >
                    </div>
                    <span class="numeric"
                        >{running(session, overview?.captured_at ?? 0) || '—'}<small
                            >Running for</small
                        ></span
                    >
                    <span class="numeric">{percent(session.cpu_percent)}<small>CPU</small></span>
                    <span class="numeric">{memory(session.memory_bytes)}<small>Memory</small></span>
                    <div class="actions">
                        {#each session.actions as action (action)}<button
                                class:primary={action === 'resume'}
                                disabled={busy}
                                title={action === 'pause'
                                    ? 'Freeze this session and everything it started. Nothing is closed; it resumes by itself after 15 minutes.'
                                    : ''}
                                onclick={() => control(session, action as 'pause' | 'resume')}
                                >{action === 'pause' ? 'Pause' : 'Resume'}</button
                            >{/each}
                    </div>
                </div>
            {/each}
        </div>
    {/each}
{/snippet}

<section class="agent-sessions" aria-label="Agent sessions">
    <form
        class="launch"
        onsubmit={(event) => {
            event.preventDefault();
            void start();
        }}
    >
        <h3>New agent session</h3>
        <div class="fields">
            <label class="folder">
                <span>Folder</span>
                <input
                    bind:value={folder}
                    list="agent-session-folders"
                    placeholder="/path/to/project or ~/project"
                    spellcheck="false"
                    autocomplete="off"
                />
                <datalist id="agent-session-folders">
                    {#each overview?.folders ?? [] as option (option)}<option value={option}
                        ></option>{/each}
                </datalist>
            </label>
            <label>
                <span>Agent</span>
                <select bind:value={agent}>
                    {#each agentChoices as choice (choice.value)}
                        <option value={choice.value}>{choice.label}</option>
                    {/each}
                </select>
            </label>
            {#if (overview?.terminals.length ?? 0) > 1}
                <label>
                    <span>Open in</span>
                    <select bind:value={terminal} onchange={() => (terminalChosen = true)}>
                        {#each overview?.terminals ?? [] as option (option)}
                            <option value={option}>{terminalLabels[option] ?? option}</option>
                        {/each}
                    </select>
                </label>
            {/if}
        </div>
        <div class="buttons">
            <button
                class="primary"
                type="submit"
                disabled={busy || !folder.trim() || !overview?.launch_supported}
                >Start in {terminalLabels[terminal] ?? 'Terminal'}</button
            >
            {#if overview?.vscode}
                <button type="button" disabled={busy || !folder.trim()} onclick={openInCode}
                    >Open folder in VS Code</button
                >
            {/if}
        </div>
        <p class="muted small">
            {#if overview && !overview.launch_supported}
                Starting sessions works on macOS for now. Sessions you start yourself still appear
                below.
            {:else if overview?.connected_launch}
                Opens a new window running the agent, registered with Jitterbug so it shows as a
                connected agent task.
            {:else}
                Opens a new window running the agent. It appears below within a few seconds.
            {/if}
        </p>
    </form>

    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if notice}<p class="notice" role="status">{notice}</p>{/if}

    {#if overview}
        <p class="muted small">{overview.message}</p>
        <h3>Agents <span class="count">{agentCount}</span></h3>
        {#if agentCount}
            {@render list(overview.agents)}
        {:else}
            <p class="muted small empty">
                No coding agents running. Start one above, or in any terminal or editor: it shows up
                here by itself.
            </p>
        {/if}
        {#if otherCount}
            <details class="other" open>
                <summary><h3>Other sessions <span class="count">{otherCount}</span></h3></summary>
                {@render list(overview.other)}
            </details>
        {/if}
    {:else if !error}
        <p class="muted">Looking for sessions…</p>
    {/if}
</section>

<style>
    .agent-sessions {
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    h3 {
        font-size: 13px;
        margin: 4px 0;
        display: flex;
        align-items: center;
        gap: 6px;
    }
    h4 {
        font-size: 11px;
        font-weight: 600;
        margin: 10px 0 2px;
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .count {
        font-size: 11px;
        font-weight: 500;
        color: var(--muted);
    }
    .small {
        font-size: 12px;
        margin: 0;
    }
    .launch {
        display: flex;
        flex-direction: column;
        gap: 8px;
        padding: 10px 12px;
        border: 1px solid var(--border);
        border-radius: 8px;
    }
    .launch h3 {
        margin: 0;
    }
    .fields {
        display: flex;
        flex-wrap: wrap;
        gap: 8px;
    }
    .fields label {
        display: flex;
        flex-direction: column;
        gap: 3px;
        font-size: 11px;
        color: var(--muted);
    }
    .fields .folder {
        flex: 1 1 260px;
        min-width: 0;
    }
    .buttons {
        display: flex;
        flex-wrap: wrap;
        gap: 8px;
    }
    .notice {
        font-size: 12px;
        margin: 0;
    }
    .rows {
        display: flex;
        flex-direction: column;
    }
    .row {
        display: grid;
        grid-template-columns: minmax(140px, 1fr) 76px 52px 66px 80px;
        align-items: center;
        gap: 4px 10px;
        padding: 8px 6px;
        border-radius: 6px;
    }
    .row:hover {
        background: var(--hover);
    }
    .name {
        min-width: 0;
    }
    .name strong,
    .name small {
        display: block;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .name strong {
        font-weight: 500;
    }
    .task {
        color: var(--muted);
        font-weight: 400;
    }
    small {
        display: block;
        color: var(--muted);
        font-size: 10px;
        margin-top: 2px;
    }
    .numeric {
        font-size: 12px;
        font-variant-numeric: tabular-nums;
    }
    .paused {
        color: var(--warning);
    }
    .badge {
        margin-right: 6px;
        font-size: 10px;
        padding: 1px 6px;
        border-radius: 999px;
        background: var(--accent-soft);
        color: var(--accent);
    }
    .actions {
        display: flex;
        justify-content: flex-end;
    }
    .empty {
        padding: 4px 6px;
    }
    .other summary {
        cursor: pointer;
        list-style-position: inside;
    }
    .other summary h3 {
        display: inline-flex;
    }
</style>
