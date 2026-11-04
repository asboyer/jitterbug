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
    doing: string | null;
    state: 'running' | 'waiting' | 'working';
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
    state: 'working' | 'waiting' | 'idle';
    doing: string | null;
    request: string | null;
    /** The question the agent's last reply asked; set only when state is `waiting`. */
    asks?: string | null;
    last_active: number | null;
    cpu_percent: number;
    memory_bytes: number;
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

export interface AgentsView {
    captured_at: number;
    live: boolean;
    cpu_percent: number;
    memory_used_percent: number;
    enabled: boolean;
    harnesses: Harness[];
    sessions: AgentSession[];
    stale: number;
    line: LineItem[];
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

export function elapsed(since: number | null, now: number): string {
    if (!since) return '';
    const seconds = Math.max(0, Math.round(now - since));
    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}m`;
    const hours = Math.floor(minutes / 60);
    return hours < 24 ? `${hours}h` : `${Math.floor(hours / 24)}d`;
}

/** Share of the whole computer's CPU, rounded the way people read it. */
export function cpuShare(value: number): string {
    if (value < 0.5) return '0%';
    return `${Math.round(value)}%`;
}

export function size(bytes: number): string {
    if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
    return `${Math.max(1, Math.round(bytes / 1024 ** 2))} MB`;
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
        doing: 'Running pytest',
        state: 'running',
        command: 'pytest',
        position: null,
        since: now - 42,
        cpu_percent: together ? 31 : 44,
        memory_bytes: 900 * MIB,
    };
    const rust: AgentRow = {
        label: 'Validate the Rust change',
        doing: together ? 'Running cargo test' : 'Editing turns.rs',
        state: together ? 'running' : 'waiting',
        command: 'cargo test',
        position: together ? null : 1,
        since: now - 30,
        cpu_percent: together ? 28 : 0,
        memory_bytes: together ? 1.4 * GIB : 0,
    };
    const build: AgentRow = {
        label: '',
        doing: null,
        state: together ? 'running' : 'waiting',
        command: 'npm run build',
        position: together ? null : 2,
        since: now - 18,
        cpu_percent: together ? 24 : 0,
        memory_bytes: together ? 1.1 * GIB : 0,
    };
    const sessions: AgentSession[] = [
        {
            id: 'demo-claude',
            harness: 'claude',
            harness_name: 'Claude Code',
            title: 'Agents tab redesign',
            folder: '~/Developer/jitterbug',
            state: 'working',
            doing: 'Delegating: Validate the Rust change',
            request: 'Make the agents tab show what each agent is doing',
            last_active: now - 2,
            cpu_percent: python.cpu_percent + rust.cpu_percent + 1.2,
            memory_bytes: python.memory_bytes + rust.memory_bytes + 320 * MIB,
            agents: [python, rust],
        },
        {
            id: 'demo-codex',
            harness: 'codex',
            harness_name: 'Codex',
            title: 'web',
            folder: '~/Developer/web',
            state: 'working',
            doing: together ? 'Running npm run build' : 'Editing checkout.tsx',
            request: 'Fix the checkout build and run it',
            last_active: now - 5,
            cpu_percent: build.cpu_percent + 0.4,
            memory_bytes: build.memory_bytes + 240 * MIB,
            agents: [build],
        },
        {
            id: 'demo-claude-2',
            harness: 'claude',
            harness_name: 'Claude Code',
            title: 'API docs',
            folder: '~/Developer/api',
            state: 'waiting',
            doing: 'Waiting for you',
            request: 'Update the README for the new endpoints',
            last_active: now - 35 * 60,
            cpu_percent: 0,
            memory_bytes: 210 * MIB,
            agents: [],
        },
    ];
    const line: LineItem[] = connected
        ? (
              [
                  ['claude', 'Claude Code', python],
                  ['claude', 'Claude Code', rust],
                  ['codex', 'Codex', build],
              ] as [string, string, AgentRow][]
          ).map(([harness, harness_name, row]) => ({
              harness,
              harness_name,
              agent: row.label,
              command: row.command ?? '',
              folder: 'jitterbug',
              state: row.state === 'waiting' ? 'waiting' : 'running',
              since: row.since ?? now,
              cpu_percent: row.cpu_percent,
              memory_bytes: row.memory_bytes,
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
        stale: 2,
        line,
        other: { cpu_percent: 6, memory_bytes: 9 * GIB, labels: ['Firefox', 'Safari', 'Slack'] },
        signature: `demo-${connected}-${together}`,
    };
}

export function demoBrief(view: AgentsView): AgentsBrief {
    const together = view.line.length === 0 || view.line.every((l) => l.state === 'running');
    return {
        headline: together
            ? 'Three test runs from two agents are fighting for your CPU at once: pytest, cargo test, and a Codex build.'
            : 'pytest is running for Claude Code; cargo test and the Codex build wait their turn while their agents keep editing.',
        notes: {
            'demo-claude': together
                ? 'Two subagents are running test suites side by side, so each runs slower.'
                : 'One subagent has the slot; the other keeps editing until its turn.',
            'demo-codex': together
                ? 'A production build competing with two test suites for memory.'
                : 'Second in line. Its build starts when cargo test finishes.',
        },
        provider: 'demo',
        model: 'synthetic',
        signature: view.signature,
        generated_at: Math.floor(Date.now() / 1000),
    };
}

export interface SessionPoint {
    time: number;
    cpu_percent: number;
    memory_bytes: number;
}

export interface SessionCall {
    label: string;
    tool: string;
    started_at: number;
    ended_at: number | null;
    seconds: number;
    samples: number;
    average_cpu_percent: number;
    peak_cpu_percent: number;
    peak_memory_bytes: number;
    cpu_seconds: number;
}

export interface SessionDetail {
    session: AgentSession;
    started_at: number | null;
    process_count: number;
    tracked_since: number | null;
    peak_cpu_percent: number;
    peak_memory_bytes: number;
    series: SessionPoint[];
    processes: { pid: number; name: string; cpu_percent: number; memory_bytes: number }[];
    calls: SessionCall[];
    heaviest: SessionCall[];
    heavy: {
        command: string;
        agent: string;
        waited_seconds: number;
        started_at: number;
        ended_at: number;
        peak_cpu_percent: number;
        peak_memory_bytes: number;
    }[];
    calls_available: boolean;
}

/** "1m 30s", "45s", "2h 5m": how long something ran. */
export function span(seconds: number): string {
    if (seconds < 60) return `${Math.max(0, Math.round(seconds))}s`;
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}m ${Math.round(seconds % 60)}s`;
    return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}
