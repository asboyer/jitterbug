import type { AgentSession, AgentWorkflow } from './types';

/** Handwritten example, never captured telemetry. No process or model is started. */
export function demoWorkflow(time: number, pressure: boolean): AgentWorkflow {
    const cpu = pressure ? 72 : 18;
    const ram = 2.4 * 1024 ** 3;
    const request =
        'Add search and keyboard navigation to the example app, with tests for filtering and focus.';
    const tip =
        'Run one full test suite after the agents finish. Keep focused checks in each worktree while they work.';
    const agents: AgentSession[] = ['Search interface', 'Keyboard navigation'].map(
        (agent, index) => ({
            run_id: `demo-run-${index}`,
            agent,
            harness: index === 0 ? 'claude' : 'codex',
            worktree: `/example/worktrees/task-${index}`,
            role: 'agent',
            status: 'active',
            root_pid: 8100 + index,
            root_start_time: time - 120,
            started_at: time - 120,
            ended_at: null,
            exit_code: null,
            cpu_percent: cpu / 2,
            memory_bytes: ram / 2,
            process_count: 3,
            peak_cpu_percent: cpu / 2 + 2,
            peak_memory_bytes: ram / 2,
            peak_process_count: 3,
            observed_samples: 24,
            context: {
                session_id: `example-${index}`,
                request,
                latest_request: '',
                source: 'demo',
                updated_at: time - 120,
                last_activity: 'command',
                last_activity_at: time - 5,
                completed_tools: { read: 6, edit: 2, command: 3 },
            },
            roles: [
                {
                    role: 'test',
                    process_count: 2,
                    cpu_percent: cpu * 0.4,
                    memory_bytes: ram * 0.35,
                },
                {
                    role: 'agent',
                    process_count: 1,
                    cpu_percent: cpu * 0.1,
                    memory_bytes: ram * 0.15,
                },
            ],
            process_ids: [9000],
            workload_ids: ['demo-project'],
        })
    );
    return {
        task_id: 'EXAMPLE-42',
        task_title: 'Add search and keyboard navigation',
        status: 'active',
        active_agents: 2,
        cpu_percent: cpu,
        memory_bytes: ram,
        process_count: 6,
        peak_cpu_percent: cpu + 4,
        peak_memory_bytes: ram,
        peak_process_count: 6,
        started_at: time - 120,
        last_seen_at: time,
        observed_samples: 24,
        agents,
        peak_roles: [],
        narrative: {
            assignments: [{ agent: 'Example task', request, source: 'demo' }],
            task_key: 'synthetic-example',
            activity:
                'Synthetic tool events: 12 reads/searches, 4 edits and 6 commands. These do not establish completion.',
            resources:
                'Synthetic example: test workers use most of this task’s CPU. The agent harness uses the remainder. These are illustrative measurements, not a diagnosis of your computer.',
            tip,
            tip_basis: 'Demo · local rule example · no action applied',
        },
        history: Array.from({ length: 24 }, (_, index) => ({
            time: time - (23 - index) * 5,
            cpu_percent: index === 23 ? cpu : cpu + Math.sin(index * 0.8) * 4,
            memory_bytes: ram,
        })),
        suggestions: [
            {
                id: 'overlapping-test',
                title: 'Stagger overlapping test work',
                recommendation: tip,
                evidence: ['Two example agents have test workers running at the same time.'],
                uncertainty:
                    'Overlap may be intentional. Measure the effect before changing your workflow.',
                observed_at: time,
                current: true,
            },
        ],
        evidence: ['Synthetic demonstration. No real agents, processes, or requests.'],
    };
}
