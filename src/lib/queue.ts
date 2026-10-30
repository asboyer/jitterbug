export type QueueStage =
    'queued' | 'starting' | 'running' | 'passed' | 'failed' | 'stale' | 'cancelled' | 'interrupted';

export interface ValidationJob {
    id: string;
    held?: boolean;
    hold_reason?: string | null;
    next?: boolean;
    task: string;
    harness: string;
    session: string;
    profile: string;
    worktree: string;
    revision: string | null;
    fingerprinted: boolean;
    stage: QueueStage;
    submitted_at: number;
    started_at: number | null;
    ended_at: number | null;
    exit_code: number | null;
    message: string;
    peak_cpu_percent: number;
    peak_memory_bytes: number;
    observed_samples: number;
}

export interface ValidationQueue {
    paused: boolean;
    control?: WorkflowControls;
    capacity: number;
    reason: string;
    profiles: string[];
    jobs: ValidationJob[];
    resources: {
        captured_at: number;
        ready: boolean;
        cpu_percent: number;
        available_bytes: number;
        total_bytes: number;
        thermal: string | null;
        memory_pressure: string | null;
        suggestions: {
            workload_id: string;
            label: string;
            cpu_percent: number;
            memory_bytes: number;
            recommendation: string;
        }[];
    };
}

export interface CheckpointSession {
    id: string;
    profile: string;
    harness: string;
    session: string;
    task: string;
    phase: 'planning' | 'editing' | 'tests' | 'review' | 'complete';
    pause_requested: boolean;
    waiting: boolean;
    last_seen_at: number;
}

export interface WorkflowControls {
    goal: 'responsive' | 'throughput';
    protected_until: number | null;
    next_job: string | null;
    sessions: CheckpointSession[];
    events: { at: number; message: string }[];
    last_block: string | null;
}

export function demoQueue(): ValidationQueue {
    const now = Math.floor(Date.now() / 1000);
    const base: ValidationJob = {
        id: 'demo-1',
        task: 'Search keyboard navigation',
        harness: 'Claude',
        session: 'Search behavior',
        profile: 'unit-tests',
        worktree: 'search-navigation',
        revision: 'abc1234',
        fingerprinted: true,
        stage: 'running',
        submitted_at: now - 60,
        started_at: now - 30,
        ended_at: null,
        exit_code: null,
        message: 'Running the approved validation command.',
        peak_cpu_percent: 18,
        peak_memory_bytes: 800 * 1024 * 1024,
        observed_samples: 6,
    };
    return {
        paused: false,
        capacity: 1,
        reason: 'One validation is running. Other sessions wait their turn.',
        profiles: ['unit-tests', 'typecheck'],
        jobs: [
            base,
            {
                ...base,
                id: 'demo-2',
                task: 'Search accessibility',
                harness: 'Codex',
                session: 'Accessibility',
                stage: 'queued',
                started_at: null,
                message: 'Waiting for the running validation.',
                observed_samples: 0,
            },
        ],
        resources: {
            captured_at: now,
            ready: true,
            cpu_percent: 32,
            available_bytes: 6 * 1024 ** 3,
            total_bytes: 16 * 1024 ** 3,
            thermal: 'nominal',
            memory_pressure: 'normal',
            suggestions: [],
        },
    };
}
