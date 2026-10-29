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

export interface Reply {
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
    sustained_samples: number;
    cooldown_seconds: number;
    max_investigations_per_hour: number;
    rules: { workload_id: string; label: string }[];
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
}
