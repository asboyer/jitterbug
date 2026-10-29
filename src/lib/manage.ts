export interface ManagementSelection {
    workload_id?: string;
    task_id?: string;
    pid?: number;
    start_time?: number;
    worktree?: string;
}

export interface ValidationProfile {
    name: string;
    worktree: string;
    command: string[];
    memory_bytes: number;
    search_path: string | null;
}

export interface ManagementCandidate {
    label: string;
    worktree: string;
    command: string[];
    memory_mib: number;
    evidence: string;
    script: string | null;
}

export interface ManagementDiscovery {
    candidates: ManagementCandidate[];
    message: string;
    worktree: string | null;
}

export interface ManagedProfile {
    profile: ValidationProfile;
    state: 'managed' | 'waiting' | 'running' | 'needs_review';
    last_result: string | null;
    connections: {
        client: string;
        configured: boolean;
        last_seen: number | null;
        message: string;
    }[];
}

export interface ManagementPreview {
    token: string;
    profile: ValidationProfile;
    existing: boolean;
    changes: { path: string; description: string }[];
    project_scripts: string | null;
}
