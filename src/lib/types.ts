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

export interface SupportedSettings {
    mode: Mode;
    decision: Tier;
    cheap: Tier;
    intelligent: Tier;
    decision_enabled: boolean;
    budget_usd: number;
}

export interface SettingsDefaultSnapshot {
    id: string;
    current: boolean;
    created_at_ms: number | null;
    summary: string | null;
    settings: SupportedSettings | null;
    error: string | null;
}

export interface SettingsDefaultHistory {
    entries: SettingsDefaultSnapshot[];
    next_cursor: string | null;
}

export interface SettingsRestorePreview {
    token: string;
    expires_at_ms: number;
    target: SettingsDefaultSnapshot;
    changes: { field: string; before: string; after: string }[];
}

export interface SystemCapability {
    included?: boolean;
    id: string;
    label: string;
    group: string;
    status: 'supported' | 'needs_permission' | 'read_only' | 'manual' | 'unavailable';
    detail: string;
    verification: string;
    pane: string | null;
}

export type SystemPreferenceValue =
    | { kind: 'absent' }
    | { kind: 'boolean'; value: boolean }
    | { kind: 'integer' | 'real'; value: number };

export interface SystemAudioFormat {
    sample_rate: number;
    format_id: number;
    flags: number;
    bytes_per_packet: number;
    frames_per_packet: number;
    bytes_per_frame: number;
    channels: number;
    bits_per_channel: number;
    reserved: number;
}

export type SystemHardwareValue =
    | {
          kind: 'audio';
          uid: string;
          rate: number | null;
          streams: { starting_channel: number; terminal_type: number; format: SystemAudioFormat }[];
      }
    | {
          kind: 'displays';
          displays: {
              uuid: string;
              x: number;
              y: number;
              rotation: number;
              main: boolean;
              active: boolean;
              mirror: string | null;
              mode: {
                  id: number;
                  width: number;
                  height: number;
                  pixel_width: number;
                  pixel_height: number;
                  refresh_hz: number;
                  flags: number;
                  desktop_usable: boolean;
              };
          }[];
      }
    | { kind: 'power'; source: string; key: 'sleep' | 'displaysleep'; minutes: number }
    | {
          kind: 'energy_policy';
          source: string;
          mode: number;
          high_power_supported: boolean;
      }
    | { kind: 'time_machine'; automatic: boolean | null; interval_seconds: number | null }
    | { kind: 'login_items'; items: { path: string; hidden: boolean }[] }
    | { kind: 'background_items'; values: Record<string, boolean> };

export interface SystemSettingValues {
    output_volume: number | null;
    output_muted: boolean | null;
    preferences?: Record<string, SystemPreferenceValue>;
    hardware?: Record<string, SystemHardwareValue>;
}

export interface SystemDefaultSnapshot {
    coverage?: SystemCapability[];
    id: string;
    current: boolean;
    created_at_ms: number | null;
    summary: string | null;
    values: SystemSettingValues | null;
    compatible: boolean;
    error: string | null;
}

export interface SystemDefaultHistory {
    entries: SystemDefaultSnapshot[];
    next_cursor: string | null;
    interrupted_operations: number;
}

export interface SystemRestorePreview {
    coverage?: SystemCapability[];
    token: string;
    expires_at_ms: number;
    target: SystemDefaultSnapshot;
    changes: {
        field: string;
        before: string;
        after: string;
        group?: string;
        verification?: string;
    }[];
}

export type SystemRollbackStatus = 'no_changes' | 'verified' | 'partial_failure';
export type SystemOutcomeStatus =
    'pending' | 'applying' | 'verified' | 'write_failed' | 'readback_mismatch' | 'external_drift';

export interface SystemRollbackResult {
    status: SystemRollbackStatus;
    outcomes: {
        setting: 'output_volume' | 'output_muted' | { preference: string } | { hardware: number };
        field: string;
        before: string;
        target: string;
        status: SystemOutcomeStatus;
        error: string | null;
    }[];
    current: SystemSettingValues | null;
    journal_id: string | null;
    message: string;
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
    governor: Governor;
}

/** Agent sessions held to keep the computer responsive. */
export interface Governor {
    enabled: boolean;
    held: {
        pid: number;
        start_time: number;
        title: string;
        reason: string;
        paused_at: number;
        until: number;
    }[];
    note: string;
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
    /** Apps in the Protected or Background bucket; changed only with `move_to_bucket`. */
    buckets: BucketAssignment[];
    jitter_mode: JitterMode;
    action_popups: boolean;
    auto_snapshots: boolean;
    agent_control: boolean;
    agent_pause_without_asking: boolean;
    hold_agents: boolean;
}

export type PriorityBucket = 'protected' | 'normal' | 'background';
export type BucketSource = 'you' | 'agent' | 'migrated_rule';

export interface BucketAssignment {
    workload_id: string;
    label: string;
    bucket: PriorityBucket;
    since: number;
    source: BucketSource;
}

export type JitterMode = 'auto' | 'snapshot_only';
export type CaseOrigin = 'automatic' | 'snapshot' | 'spike' | 'agent';
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
    /** A stop, pause, or tab close offered instead of (or besides) the priority change. */
    alternative?: {
        process: ProcessRecord;
        action: string;
        rationale: string;
        unit?: { title: string; kind: 'tab' | 'session' } | null;
    } | null;
    /** A firmer option offered after the alternative: closing a tab it would pause. */
    fallback?: {
        process: ProcessRecord;
        action: string;
        rationale: string;
        unit?: { title: string; kind: 'tab' | 'session' } | null;
    } | null;
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
    action: 'lower_priority' | 'terminate' | 'pause' | 'pause_tab' | 'close_tab';
    process: { name: string; cpu_percent: number; memory_bytes: number };
    impact: ActionImpact | null;
    /** For a stop, pause, or tab close: what it does, in plain words. */
    closes: string | null;
    /** The tab or command session a granular action targets. */
    unit?: { title: string; kind: 'tab' | 'session' } | null;
}

/** Trimmed case for the out-of-app popup: no paths, PIDs, evidence, or diagnosis. */
export interface ActionPopupCase {
    kind: 'case';
    id: number;
    source: string;
    stage: AutomationStage;
    trigger: {
        kind: string;
        title: string;
        target: { label: string } | null;
        /** An agent's stated reason, for agent requests only. */
        reason?: string | null;
    };
    proposal: ActionPopupOption | null;
    alternative: ActionPopupOption | null;
    /** Offered after the alternative: closing the tab it would pause. */
    fallback?: ActionPopupOption | null;
    report: {
        fresh: boolean;
        after: true | null;
        action: string;
        process: string;
        cpu_before: number;
        cpu_after: number | null;
        memory_before: number;
        memory_after: number | null;
        resumable?: boolean;
        /** The app this case moved to Background is still there: Undo moves it to Normal. */
        undo?: boolean;
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

/** One app in the priorities view. Names and counts only: no process identity. */
export interface AppPriority {
    workload_id: string;
    label: string;
    bucket: PriorityBucket;
    since: number | null;
    source: BucketSource | null;
    running: boolean;
    frontmost: boolean;
    /** Its processes Jitterbug keeps in background mode right now. */
    affected: number;
}

export type Priorities = Record<PriorityBucket, AppPriority[]>;

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

export interface InsideUnit {
    id: string;
    kind: 'tab' | 'session';
    title: string;
    detail: string;
    host: string | null;
    cpu_percent: number;
    memory_bytes: number;
    process_count: number;
    shared_with: number;
    pid: number | null;
    start_time: number | null;
    actions: string[];
    evidence: string[];
    confidence: string;
    paused_until: number | null;
    /** A coding agent's own state, e.g. Claude Code's `working` or `idle`. */
    agent_status?: string | null;
}

export interface ProcessLabel {
    pid: number;
    start_time: number;
    title: string;
    subtitle: string;
}

export interface InsideView {
    workload_id: string;
    source: string;
    status: 'ready' | 'setup' | 'unavailable';
    message: string;
    setup: string[];
    units: InsideUnit[];
    remainder: {
        label: string;
        cpu_percent: number;
        memory_bytes: number;
        process_count: number;
    }[];
    measured_seconds: number;
    captured_at: number;
    limits: string;
    labels: ProcessLabel[];
}
