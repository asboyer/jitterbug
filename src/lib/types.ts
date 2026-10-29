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
    memory_total_bytes: number;
    memory_available_bytes: number;
    swap_used_bytes: number;
    swap_growth_bytes: number;
    process_count: number;
    processes: ProcessRecord[];
    workloads: Workload[];
    platform: {
        os: string;
        memory_pressure: string | null;
        thermal: string | null;
        process_actions: boolean;
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
    source: 'rules' | 'model' | 'demo';
    provider: string | null;
    model: string | null;
    tier: string;
    route: string[];
    estimated_cost_usd: number;
}

export interface LocalModel {
    name: string;
    size_bytes: number;
    eligible: boolean;
}
