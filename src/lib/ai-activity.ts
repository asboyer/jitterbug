import type { AgentCall, AiActivity, AutomationCase, AutomationStage } from './types';

const purposes: Record<string, string> = {
    question: 'Answered your question',
    monitor: 'Explained the Monitor overview',
    app_explanation: 'Explained an app',
    investigation: 'Investigated a slowdown',
    snapshot: 'Analyzed a Jitter Snapshot',
    task_summary: 'Summarized an agent task',
    queue_plan: 'Planned the validation queue',
    routing: 'Chose a model for a question',
};

export const activityLabel = (entry: AiActivity) =>
    entry.routing ? purposes.routing : (purposes[entry.purpose] ?? 'Other model call');

const outcomes: Partial<Record<AutomationStage, string>> = {
    investigating: 'Still investigating',
    awaiting_approval: 'Waiting for you',
    applying: 'Applying fix',
    verifying: 'Checking the fix',
    completed: 'Fix applied',
    resolved: 'Cleared on its own',
    blocked: 'Blocked',
    dismissed: 'You dismissed it',
};

/** What happened to the incident an investigation was about, if it is still in the list. */
export function activityOutcome(entry: AiActivity, cases: AutomationCase[]) {
    if (entry.case_id === null) return null;
    const stage = cases.find((item) => item.id === entry.case_id)?.stage;
    return stage ? (outcomes[stage] ?? null) : null;
}

export function ago(at: number, now = Date.now() / 1000) {
    const seconds = Math.max(0, Math.round(now - at));
    if (seconds < 60) return 'just now';
    if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
    return `${Math.floor(seconds / 3600)} h ago`;
}

/** Synthetic entries for the browser preview. */
export function demoActivity(now = Math.floor(Date.now() / 1000)): AiActivity[] {
    const entry = (
        seconds: number,
        purpose: string,
        provider: AiActivity['provider'],
        model: string,
        duration_ms: number,
        cost_usd: number,
        extra: Partial<AiActivity> = {}
    ): AiActivity => ({
        at: now - seconds,
        purpose,
        case_id: null,
        provider,
        model,
        routing: false,
        duration_ms,
        cost_usd,
        ok: true,
        error: null,
        ...extra,
    });
    return [
        entry(40, 'question', 'ollama', 'qwen3.5:0.8b', 1800, 0),
        entry(260, 'investigation', 'meta', 'muse-spark-1.3', 6400, 0.0061, { case_id: -1 }),
        entry(900, 'app_explanation', 'meta', 'muse-spark-1.3', 2300, 0.0018),
        entry(1500, 'monitor', 'meta', 'muse-spark-1.3', 0, 0, {
            ok: false,
            error: 'No credential is configured for this provider.',
            duration_ms: 3,
        }),
        entry(2400, 'task_summary', 'meta', 'muse-spark-1.3', 4100, 0.0034),
    ];
}

const methods: Record<string, string> = {
    overview: 'Checked the machine',
    inspect_app: 'Looked inside an app',
    pause_session: 'Asked to pause a session',
    resume_session: 'Resumed a session',
    pause_tab: 'Asked to pause a tab',
    resume_tab: 'Resumed a tab',
    close_tab: 'Asked to close a tab',
    request_background: 'Asked to move an app to Background',
    request_status: 'Checked a request',
};

export function agentCallLabel(call: AgentCall) {
    if (call.method === 'pressure_hint')
        return call.hinted ? 'Warned before heavy work' : 'Checked before heavy work';
    return methods[call.method] ?? 'Unknown request';
}

/** Who asked: the session it runs in, led by the agent unless the title already names it. */
export function agentCallSource(call: AgentCall) {
    const agent = call.agent ?? 'An agent';
    if (!call.session) return agent;
    return call.agent && call.session.includes(call.agent)
        ? call.session
        : `${agent} · ${call.session}`;
}

/** Synthetic agent calls for the browser preview. */
export function demoAgentCalls(now = Math.floor(Date.now() / 1000)): AgentCall[] {
    const call = (seconds: number, method: string, extra: Partial<AgentCall> = {}): AgentCall => ({
        at: now - seconds,
        method,
        agent: 'Claude Code',
        session: 'TEST1',
        ok: true,
        error: null,
        request_id: null,
        hinted: null,
        ...extra,
    });
    return [
        call(20, 'request_status', { request_id: 4 }),
        call(70, 'pause_session', { request_id: 4 }),
        call(80, 'overview'),
        call(90, 'pressure_hint', { hinted: true }),
        call(1300, 'pressure_hint', { hinted: false, agent: 'Codex', session: 'TEST2' }),
    ];
}
