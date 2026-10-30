export type Provider = 'ollama' | 'meta' | 'anthropic' | 'jev';
export type Mode = 'hybrid' | 'local_only' | 'cloud_only';
export type TierName = 'decision' | 'cheap' | 'intelligent';

export interface Tier {
    provider: Provider;
    model: string;
}

export interface Settings {
    mode: Mode;
    decision: Tier;
    cheap: Tier;
    intelligent: Tier;
    decision_enabled: boolean;
    budget_usd: number;
    automation: AutomationPolicy;
}

export interface Spending {
    reserved_usd: number;
    reported_estimate_usd: number;
    requests: number;
}

export interface Bootstrap {
    settings: Settings;
    credentials: Provider[];
    spending: Spending;
    automation_cases: AutomationCase[];
    automation_snoozed_until: number;
}

export interface Workload {
    unresponsive?: boolean | null;
    id: string;
    label: string;
    category: string;
    description: string;
    attribution: string;
    cpu_percent: number;
    memory_bytes: number;
    process_count: number;
    history: ResourcePoint[];
}

export interface ProcessRecord {
    pid: number;
    start_time: number;
    parent_pid: number | null;
    name: string;
    cpu_percent: number;
    memory_bytes: number;
    workload_id: string;
    description: string;
    evidence: string[];
    actionable: boolean;
    actions: string[];
    disk_read_bytes: number;
    disk_written_bytes: number;
}

export interface AppProfile {
    text: string;
    source: 'reference' | 'inference' | 'model' | 'demo';
    provider: string | null;
    model: string | null;
    generated_at: number | null;
    cached: boolean;
}

export interface WorkloadDetails {
    captured_at: number;
    workload: Workload;
    process: ProcessRecord;
    members: ProcessRecord[];
    member_count: number;
    independent_roots: number;
    application: {
        name: string;
        executable_name: string;
        component: string | null;
        component_kind: string;
        installation: string;
        identifier: string | null;
        version: string | null;
        identity_source: string;
    } | null;
    profile: AppProfile;
    launch?: {
        program: string;
        task: string;
        summary: string;
        launcher: string;
        evidence: string[];
        chain: { pid: number; role: string }[];
        limitation: string;
        reference_url: string | null;
    } | null;
    ancestry: {
        pid: number;
        start_time: number;
        name: string;
        executable: string | null;
        working_directory: string | null;
    }[];
    origin_summary: string;
    origin_limit: string;
    can_generate_profile: boolean;
    profile_key: string | null;
}

export interface Incident {
    id: number;
    created_at: number;
    title: string;
    reasons: string[];
    workload_id: string | null;
}

export interface Snapshot {
    source: 'live' | 'demo';
    captured_at: number;
    sample_ready: boolean;
    interval_ms: number;
    cpu_percent: number;
    logical_cpus: number;
    cpu_cores: number[];
    memory_total_bytes: number;
    memory_available_bytes: number;
    swap_used_bytes: number;
    swap_growth_bytes: number;
    swap_in_bytes: number | null;
    swap_out_bytes: number | null;
    process_count: number;
    processes: ProcessRecord[];
    workloads: Workload[];
    agent_workflows: AgentWorkflow[];
    platform: {
        os: string;
        memory_pressure: string | null;
        thermal: string | null;
        process_actions: boolean;
        memory: {
            app_bytes: number;
            wired_bytes: number;
            compressed_bytes: number;
            cached_bytes: number;
            used_bytes: number;
            swap_in_total_bytes: number;
            swap_out_total_bytes: number;
        } | null;
    };
    pressure: { level: string; reasons: string[]; sustained_samples: number };
    incident: Incident | null;
    history: { time: number; cpu_percent: number; memory_percent: number }[];
    collector_ms: number;
}

export interface ChatMessage {
    role: 'user' | 'assistant';
    content: string;
}

export interface Insight {
    headline: string;
    finding: string;
    next_step: string;
}

export interface Reply {
    follow_ups?: string[];
    unavailable?: string | null;
    research?: {
        title: string;
        url: string;
        summary: string;
        checked_at: number | null;
        status: string;
    } | null;
    insight?: Insight | null;
    text: string;
    source: 'rules' | 'model' | 'demo' | 'guidance';
    provider: string | null;
    model: string | null;
    tier: string;
    route: string[];
    estimated_cost_usd: number;
    investigation: Investigation | null;
    choices: Target[];
}

export interface LocalModel {
    name: string;
    size_bytes: number;
    eligible: boolean;
}

export interface ResourcePoint {
    time: number;
    cpu_percent: number;
    memory_bytes: number;
}

export interface Target {
    workload_id: string;
    label: string;
    pid: number | null;
}

export interface Investigation {
    stack_checks?: { role: string; state: string; summary: string; captured_at: number }[];
    responsiveness?: string;
    coverage?: string;
    recently_unresponsive?: boolean;
    target: Target;
    captured_at: number;
    cpu_percent: number;
    memory_bytes: number;
    process_count: number;
    history: ResourcePoint[];
    processes: ProcessRecord[];
    limits: string;
}

export interface ActionReport {
    action: string;
    message: string;
    before: ProcessRecord;
    after: ProcessRecord | null;
    measured_after_seconds: number;
    fresh: boolean;
}

export interface AutomationPolicy {
    enabled: boolean;
    model_analysis: boolean;
    system_cpu_percent: number;
    available_memory_percent: number;
    app_cpu_core_percent: number;
    app_cpu_percent: number;
    sustained_samples: number;
    cooldown_seconds: number;
    max_investigations_per_hour: number;
    rules: { workload_id: string; label: string }[];
    jitter_mode: JitterMode;
    action_popups: boolean;
    auto_snapshots: boolean;
}

export type JitterMode = 'auto' | 'snapshot_only';
export type CaseOrigin = 'automatic' | 'snapshot' | 'spike';
export type FindingStrength = 'strong' | 'possible' | 'unexplained';

export interface JitterFinding {
    kind:
        | 'process_spike'
        | 'core_pegged'
        | 'memory_thrash'
        | 'thermal_escalation'
        | 'scheduler_lag'
        | 'victim_busy'
        | 'nothing_found';
    strength: FindingStrength;
    title: string;
    observed: string[];
    inferred: string;
    workload_id: string | null;
    label: string | null;
    category: string | null;
}

export interface JitterParty {
    workload_id: string | null;
    label: string;
    category: string;
}

export interface JitterReport {
    origin: CaseOrigin;
    press_at_ms: number;
    forward_only: boolean;
    window: [number, number];
    suspect: [number, number];
    timeline: {
        offset_s: number;
        cpu_percent: number;
        core_max: number;
        lag_ms: number;
        paging_mib_s: number | null;
    }[];
    findings: JitterFinding[];
    victim: JitterParty | null;
    culprit: JitterParty | null;
    happened: string;
    likely_cause: string;
    in_app: { action: string; process: ProcessRecord; label: string } | null;
    manual: string | null;
    if_again: string;
    recurrence: number;
    offer_permission: boolean;
    limits: string;
}

export interface JitterTally {
    entries: { label: string; count: number; last_at: number }[];
    mode: JitterMode;
    sampling: boolean;
}

export type AutomationStage =
    | 'detected'
    | 'investigating'
    | 'awaiting_approval'
    | 'applying'
    | 'verifying'
    | 'completed'
    | 'resolved'
    | 'blocked'
    | 'dismissed';
export interface AutomationCase {
    id: number;
    source: string;
    detected_at: number;
    trigger: { key: string; kind: string; title: string; reason: string; target: Target | null };
    stage: AutomationStage;
    steps: { at: number; stage: AutomationStage; message: string }[];
    evidence: Investigation | null;
    diagnosis: Reply | null;
    proposal: { process: ProcessRecord; action: string; rationale: string } | null;
    report: ActionReport | null;
    verification: {
        summary: string;
        system_cpu_before: number;
        system_cpu_after: number | null;
        available_memory_before: number;
        available_memory_after: number | null;
        pressure_before: string;
        pressure_after: string | null;
        priority_verified: boolean;
    } | null;
    automatic: boolean;
    origin?: CaseOrigin;
    jitter?: JitterReport | null;
}

/** The workload around a popup option, measured when it was proposed. */
export interface ActionImpact {
    label: string;
    workload_cpu_percent: number;
    workload_memory_bytes: number;
    process_count: number;
    whole_app: boolean;
}

export interface ActionPopupOption {
    action: 'lower_priority' | 'terminate';
    process: { name: string; cpu_percent: number; memory_bytes: number };
    impact: ActionImpact | null;
    /** For a stop: what ends, in plain words. */
    closes: string | null;
}

/** Trimmed case for the out-of-app popup: no paths, PIDs, evidence, or diagnosis. */
export interface ActionPopupCase {
    kind: 'case';
    id: number;
    source: string;
    stage: AutomationStage;
    trigger: { kind: string; title: string; target: { label: string } | null };
    proposal: ActionPopupOption | null;
    alternative: ActionPopupOption | null;
    report: {
        fresh: boolean;
        after: true | null;
        action: string;
        process: string;
        cpu_before: number;
        cpu_after: number | null;
        memory_before: number;
        memory_after: number | null;
    } | null;
    verification: {
        priority_verified: boolean;
        system_cpu_before: number;
        system_cpu_after: number | null;
        available_memory_before: number;
        available_memory_after: number | null;
    } | null;
    origin: CaseOrigin;
    jitter: {
        culprit: { label: string } | null;
        victim: { label: string } | null;
        happened: string;
        likely_cause: string;
        manual: string | null;
    } | null;
    more: number;
}

/** A snapshot is recording; its case does not exist yet. */
export interface ActionPopupCapture {
    kind: 'capturing';
    id: 0;
    message: string;
}

export type ActionPopupPayload = ActionPopupCase | ActionPopupCapture;

export interface AgentRoleUsage {
    role: string;
    process_count: number;
    cpu_percent: number;
    memory_bytes: number;
}

export interface AgentSession {
    run_id: string;
    agent: string;
    harness: string;
    worktree: string;
    role: string;
    status: 'active' | 'completed' | 'failed' | 'interrupted';
    root_pid: number;
    root_start_time: number;
    started_at: number;
    ended_at: number | null;
    exit_code: number | null;
    cpu_percent: number;
    memory_bytes: number;
    process_count: number;
    peak_cpu_percent: number;
    peak_memory_bytes: number;
    peak_process_count: number;
    observed_samples: number;
    context: {
        session_id: string;
        request: string;
        latest_request: string;
        source: string;
        updated_at: number;
        last_activity: string;
        last_activity_at: number;
        completed_tools: Record<string, number>;
    };
    roles: AgentRoleUsage[];
    process_ids: number[];
    workload_ids: string[];
}

export interface WorkflowSuggestion {
    id: string;
    title: string;
    recommendation: string;
    evidence: string[];
    uncertainty: string;
    observed_at: number;
    current: boolean;
}

export interface AgentWorkflow {
    task_id: string;
    task_title: string;
    status: 'active' | 'completed' | 'failed' | 'interrupted';
    active_agents: number;
    cpu_percent: number;
    memory_bytes: number;
    process_count: number;
    peak_cpu_percent: number;
    peak_memory_bytes: number;
    peak_process_count: number;
    started_at: number;
    last_seen_at: number;
    observed_samples: number;
    narrative: {
        assignments: { agent: string; request: string; source: string }[];
        task_key: string;
        activity: string;
        resources: string;
        tip: string;
        tip_basis: string;
    };
    peak_roles: AgentRoleUsage[];
    agents: AgentSession[];
    history: ResourcePoint[];
    suggestions: WorkflowSuggestion[];
    evidence: string[];
}
