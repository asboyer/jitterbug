import type { AgentWorkflow, Snapshot } from './types';
import { demoQueue, type ValidationQueue, type WorkflowControls } from './queue';

export interface WorkflowRow {
    id: string;
    kind: 'job' | 'session' | 'observed';
    title: string;
    harnesses: string[];
    phase: string;
    state: string;
    cpu: number | null;
    memory: number | null;
    peak: boolean;
    detail: string;
    action: string | null;
    actionLabel: string;
    canPrioritize: boolean;
    next: boolean;
    complete: boolean;
    workflow?: AgentWorkflow;
}

export function workflowRows(queue: ValidationQueue, snapshot: Snapshot): WorkflowRow[] {
    const rows: WorkflowRow[] = [];
    const now = Math.floor(Date.now() / 1000);
    for (const session of queue.control?.sessions ?? []) {
        const current = session.last_seen_at + 60 >= now;
        const paused =
            session.pause_requested && session.waiting && session.last_seen_at + 3 >= now;
        rows.push({
            id: session.id,
            kind: 'session',
            title: session.task,
            harnesses: [session.harness],
            phase: session.phase,
            state:
                session.phase === 'complete'
                    ? 'Completed'
                    : paused
                      ? 'Paused at checkpoint'
                      : session.pause_requested
                        ? 'Pause requested'
                        : current
                          ? 'Checked in'
                          : 'Awaiting check-in',
            cpu: null,
            memory: null,
            peak: false,
            detail: `Cooperative checkpoint · ${session.profile}. Pause takes effect when the agent checks in; existing processes and memory remain.`,
            action:
                session.phase === 'complete'
                    ? null
                    : session.pause_requested
                      ? 'resume_session'
                      : 'pause_session',
            actionLabel: session.pause_requested ? 'Resume agent' : 'Pause at checkpoint',
            canPrioritize: false,
            next: false,
            complete: session.phase === 'complete',
        });
    }
    for (const job of queue.jobs) {
        const workflow = snapshot.agent_workflows.find((w) => w.task_id === job.id);
        const active = ['running', 'starting'].includes(job.stage);
        const complete = !['running', 'starting', 'queued', 'interrupted'].includes(job.stage);
        rows.push({
            id: job.id,
            kind: 'job',
            title: job.task,
            harnesses: [job.harness],
            phase: 'check',
            state:
                job.stage === 'queued'
                    ? job.held || (queue.control?.protected_until ?? 0) > now
                        ? 'Held'
                        : job.hold_reason
                          ? 'Waiting'
                          : job.next
                            ? 'Next'
                            : 'Queued'
                    : job.stage === 'passed'
                      ? 'Completed'
                      : job.stage[0].toUpperCase() + job.stage.slice(1),
            cpu:
                active && workflow?.observed_samples
                    ? workflow.cpu_percent
                    : !active && job.observed_samples
                      ? job.peak_cpu_percent
                      : null,
            memory:
                active && workflow?.observed_samples
                    ? workflow.memory_bytes
                    : !active && job.observed_samples
                      ? job.peak_memory_bytes
                      : null,
            peak: !active,
            detail: job.hold_reason ?? job.message,
            action: job.stage === 'queued' ? (job.held ? 'resume_job' : 'hold_job') : null,
            actionLabel: job.held ? 'Release check' : 'Hold check',
            canPrioritize: job.stage === 'queued',
            next: queue.control?.next_job === job.id,
            complete,
            workflow,
        });
    }
    for (const workflow of snapshot.agent_workflows) {
        if (queue.jobs.some((job) => job.id === workflow.task_id)) continue;
        const active = workflow.status === 'active';
        rows.push({
            id: workflow.task_id,
            kind: 'observed',
            title: workflow.task_title,
            harnesses: [...new Set(workflow.agents.map((agent) => agent.harness))],
            phase: active ? 'observed' : 'complete',
            state: active
                ? 'Observed'
                : workflow.status[0].toUpperCase() + workflow.status.slice(1),
            cpu: workflow.observed_samples
                ? active
                    ? workflow.cpu_percent
                    : workflow.peak_cpu_percent
                : null,
            memory: workflow.observed_samples
                ? active
                    ? workflow.memory_bytes
                    : workflow.peak_memory_bytes
                : null,
            peak: !active,
            detail: 'Resource attribution is live. Connect approved checks to control this workload.',
            action: null,
            actionLabel: '',
            canPrioritize: false,
            next: false,
            complete: !active,
            workflow,
        });
    }
    return rows;
}

export function hubDemo(): ValidationQueue {
    const queue = demoQueue();
    queue.jobs[0].task = 'Validate checkout';
    queue.jobs[1].task = 'Check search changes';
    queue.jobs[1].observed_samples = 0;
    queue.jobs[1].peak_cpu_percent = 0;
    queue.jobs[1].next = true;
    queue.control = {
        goal: 'responsive',
        protected_until: null,
        next_job: null,
        sessions: [
            {
                id: 'demo-session',
                profile: 'unit-tests',
                harness: 'Meta',
                session: 'demo',
                task: 'Improve sign-in',
                phase: 'editing',
                pause_requested: false,
                waiting: false,
                last_seen_at: Math.floor(Date.now() / 1000),
            },
        ],
        events: [
            { at: Math.floor(Date.now() / 1000), message: 'Search checks will follow checkout.' },
        ],
        last_block: null,
    };
    return queue;
}

export function demoControl(
    value: ValidationQueue,
    action: string,
    id: string | null
): ValidationQueue {
    const queue = structuredClone(value);
    const control = queue.control as WorkflowControls;
    const job = queue.jobs.find((item) => item.id === id);
    const session = control.sessions.find((item) => item.id === id);
    const now = Math.floor(Date.now() / 1000);
    let message = '';
    switch (action) {
        case 'pause':
        case 'resume':
            queue.paused = action === 'pause';
            message = queue.paused ? 'New checks paused.' : 'Checks resume when there is room.';
            break;
        case 'protect':
            control.protected_until = now + 1200;
            message = 'Upcoming checks held for 20 minutes.';
            break;
        case 'unprotect':
            control.protected_until = null;
            message = 'Protection ended.';
            break;
        case 'responsive':
        case 'throughput':
            control.goal = action;
            message = 'Scheduling goal updated.';
            break;
        case 'run_next':
            control.next_job = id;
            message = 'Selected check gets the next eligible slot.';
            break;
        case 'hold_job':
        case 'resume_job':
            if (job) job.held = action === 'hold_job';
            message = action === 'hold_job' ? 'Check held before launch.' : 'Check released.';
            break;
        case 'pause_session':
        case 'resume_session':
            if (session) {
                session.pause_requested = action === 'pause_session';
                session.waiting = false;
                session.last_seen_at = now;
            }
            message =
                action === 'pause_session'
                    ? 'Agent pause requested at its next checkpoint.'
                    : 'Agent released from its checkpoint.';
            break;
        case 'demo_pressure':
            queue.resources.cpu_percent = queue.resources.cpu_percent >= 85 ? 25 : 88;
            message =
                queue.resources.cpu_percent >= 85
                    ? 'Other apps are busy. Upcoming checks are waiting.'
                    : 'Pressure eased.';
            break;
        case 'demo_checkpoint':
            control.sessions.forEach((item) => {
                item.waiting = item.pause_requested;
                item.last_seen_at = now;
            });
            queue.jobs
                .filter((item) => item.stage === 'running')
                .forEach((item) => {
                    item.stage = 'passed';
                    item.ended_at = now;
                    item.message = 'Demo check finished.';
                });
            message = 'Checkpoint reached.';
            break;
    }
    if (control.protected_until && control.protected_until <= now) control.protected_until = null;
    for (const item of queue.jobs.filter((item) => item.stage === 'queued')) {
        item.hold_reason = item.held
            ? 'Held by you.'
            : control.protected_until
              ? 'Protected session: upcoming checks are held.'
              : queue.paused
                ? 'New checks are paused.'
                : queue.resources.cpu_percent >= (control.goal === 'responsive' ? 55 : 70)
                  ? 'Waiting for CPU headroom.'
                  : null;
    }
    if (!queue.jobs.some((item) => ['running', 'starting'].includes(item.stage))) {
        const eligible = queue.jobs.filter((item) => item.stage === 'queued' && !item.hold_reason);
        const next = eligible.find((item) => item.id === control.next_job) ?? eligible[0];
        if (next) {
            next.stage = 'running';
            next.started_at = now;
            next.message = 'Demo check running.';
            control.next_job = null;
            message = 'Next eligible check started.';
        }
    }
    control.events = [...control.events, { at: now, message }].slice(-24);
    return queue;
}
