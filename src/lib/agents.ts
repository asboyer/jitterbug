export interface Harness {
    key: string;
    name: string;
    installed: boolean;
    version: string | null;
    config: string;
    connected: boolean;
    last_seen: number | null;
    entry: string;
    note: string;
}

export interface AgentRow {
    label: string;
    subagent: boolean;
    state: 'running' | 'waiting' | 'working' | 'idle';
    command: string | null;
    position: number | null;
    since: number | null;
    cpu_percent: number;
    memory_bytes: number;
}

export interface AgentSession {
    id: string;
    harness: string;
    harness_name: string;
    title: string;
    folder: string;
    status: string | null;
    cpu_percent: number;
    memory_bytes: number;
    process_count: number;
    agents: AgentRow[];
}

export interface LineItem {
    harness: string;
    harness_name: string;
    agent: string;
    command: string;
    folder: string;
    state: 'running' | 'waiting';
    since: number;
    cpu_percent: number;
    memory_bytes: number;
}

export interface Finished {
    harness: string;
    agent: string;
    command: string;
    waited_seconds: number;
    started_at: number;
    ended_at: number;
    peak_cpu_percent: number;
    peak_memory_bytes: number;
}

export interface AgentsView {
    captured_at: number;
    live: boolean;
    cpu_percent: number;
    memory_used_percent: number;
    enabled: boolean;
    harnesses: Harness[];
    sessions: AgentSession[];
    line: LineItem[];
    recent: Finished[];
    other: { cpu_percent: number; memory_bytes: number; labels: string[] };
    signature: string;
}

export interface AgentsBrief {
    headline: string;
    notes: Record<string, string>;
    provider: string;
    model: string;
    signature: string;
    generated_at: number;
}

export const rowId = (session: AgentSession, index: number) => `${session.id}#${index}`;

export function elapsed(since: number | null, now: number): string {
    if (!since) return '';
    const seconds = Math.max(0, Math.round(now - since));
    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.floor(seconds / 60);
    return minutes < 60 ? `${minutes}m ${seconds % 60}s` : `${Math.floor(minutes / 60)}h`;
}

const GIB = 1024 ** 3;
const MIB = 1024 ** 2;

/** Synthetic agents for the browser preview and the in-app demo. Never real telemetry. */
export function demoAgents(step: number, connected: boolean): AgentsView {
    const now = Math.floor(Date.now() / 1000);
    const harnesses: Harness[] = [
        ['claude', 'Claude Code', '2.1.318', '~/.claude/settings.json'],
        ['codex', 'Codex', '0.164.0', '~/.codex/config.toml'],
        ['muse', 'Muse Code', '1.4.14', 'Muse plugin "jitterbug-agents"'],
    ].map(([key, name, version, config]) => ({
        key,
        name,
        installed: true,
        version,
        config,
        connected,
        last_seen: connected ? now : null,
        entry: 'Demo: no settings are changed.',
        note: '',
    }));
    // Without turns, all three suites run at once; with them, one at a time.
    const together = !connected || step === 0;
    const python: AgentRow = {
        label: 'Run the Python test suite',
        subagent: true,
        state: 'running',
        command: 'pytest',
        position: null,
        since: now - 42,
        cpu_percent: together ? 31 : 44,
        memory_bytes: 900 * MIB,
    };
    const rust: AgentRow = {
        label: 'Validate the Rust change',
        subagent: true,
        state: together ? 'running' : 'waiting',
        command: 'cargo test',
        position: together ? null : 1,
        since: now - 30,
        cpu_percent: together ? 28 : 0.4,
        memory_bytes: together ? 1.4 * GIB : 40 * MIB,
    };
    const build: AgentRow = {
        label: 'Main conversation',
        subagent: false,
        state: together ? 'running' : 'waiting',
        command: 'npm run build',
        position: together ? null : 2,
        since: now - 18,
        cpu_percent: together ? 24 : 0.3,
        memory_bytes: together ? 1.1 * GIB : 30 * MIB,
    };
    const main: AgentRow = {
        label: 'Main conversation',
        subagent: false,
        state: 'working',
        command: null,
        position: null,
        since: null,
        cpu_percent: 1.2,
        memory_bytes: 320 * MIB,
    };
    const sum = (rows: AgentRow[]) => ({
        cpu_percent: rows.reduce((t, r) => t + r.cpu_percent, 0),
        memory_bytes: rows.reduce((t, r) => t + r.memory_bytes, 0),
    });
    const claudeRows = [main, python, rust];
    const sessions: AgentSession[] = [
        {
            id: 'demo-claude',
            harness: 'claude',
            harness_name: 'Claude Code',
            title: 'Agents tab redesign',
            folder: '~/Developer/jitterbug',
            status: 'working',
            process_count: 14,
            agents: claudeRows,
            ...sum(claudeRows),
        },
        {
            id: 'demo-codex',
            harness: 'codex',
            harness_name: 'Codex',
            title: 'web',
            folder: '~/Developer/web',
            status: null,
            process_count: 6,
            agents: [build],
            ...sum([build]),
        },
    ];
    const line: LineItem[] = connected
        ? [
              ['claude', 'Claude Code', python.label, 'pytest', 'running', python],
              [
                  'claude',
                  'Claude Code',
                  rust.label,
                  'cargo test',
                  together ? 'running' : 'waiting',
                  rust,
              ],
              ['codex', 'Codex', '', 'npm run build', together ? 'running' : 'waiting', build],
          ].map(([harness, harness_name, agent, command, state, row]) => ({
              harness: harness as string,
              harness_name: harness_name as string,
              agent: agent as string,
              command: command as string,
              folder: 'jitterbug',
              state: state as 'running' | 'waiting',
              since: (row as AgentRow).since ?? now,
              cpu_percent: (row as AgentRow).cpu_percent,
              memory_bytes: (row as AgentRow).memory_bytes,
          }))
        : [];
    const cpu = sessions.reduce((t, s) => t + s.cpu_percent, 0) + 6;
    return {
        captured_at: now,
        live: false,
        cpu_percent: Math.min(cpu, 98),
        memory_used_percent: together ? 81 : 58,
        enabled: connected,
        harnesses,
        sessions,
        line,
        recent: [],
        other: { cpu_percent: 6, memory_bytes: 9 * GIB, labels: ['Firefox', 'Safari', 'Slack'] },
        signature: `demo-${connected}-${together}`,
    };
}

export function demoBrief(view: AgentsView): AgentsBrief {
    const together = view.sessions[0]?.agents[2]?.state === 'running';
    return {
        headline: together
            ? '3 test runs from 2 agents are fighting for your CPU: pytest, cargo test, and a Codex build are all running at once.'
            : 'One heavy command at a time: pytest is running for Claude Code while cargo test and the Codex build wait their turn.',
        notes: {
            'demo-claude#1':
                'CPU-bound test workers. Safe to let finish; the next check starts right after.',
            'demo-claude#2': together
                ? 'Compiling and testing at the same time as two other suites, so each runs slower.'
                : 'Waiting for its turn. The subagent keeps editing in the meantime.',
            'demo-codex#0': together
                ? 'A production build competing with two test suites for memory.'
                : 'Second in line. Starts when cargo test finishes and CPU has room.',
        },
        provider: 'demo',
        model: 'synthetic',
        signature: view.signature,
        generated_at: Math.floor(Date.now() / 1000),
    };
}
