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

/** A separate finished run, with historical measurements and no live processes or advice. */
export function demoCompletedWorkflow(time: number): AgentWorkflow {
    const endedAt = time - 120;
    const startedAt = endedAt - 120;
    const request =
        'Check Tab, Shift+Tab, and arrow-key focus behavior in the example search results. Run the focused keyboard regression checks.';
    const readings = [1, 2, 2, 3, 3, 5, 6, 7, 8, 9, 10, 11, 12, 11, 8, 7, 5, 4, 3, 2, 1, 1, 0, 0];
    const peakMemory = 512 * 1024 ** 2;
    const agent: AgentSession = {
        run_id: 'demo-completed-focus-run',
        agent: 'Focus regression checks',
        harness: 'muse',
        worktree: '/example/worktrees/focus-checks',
        role: 'agent',
        status: 'completed',
        root_pid: 8200,
        root_start_time: startedAt,
        started_at: startedAt,
        ended_at: endedAt,
        exit_code: 0,
        cpu_percent: 0,
        memory_bytes: 0,
        process_count: 0,
        peak_cpu_percent: 12,
        peak_memory_bytes: peakMemory,
        peak_process_count: 3,
        observed_samples: readings.length,
        context: {
            session_id: 'example-completed-focus',
            request,
            latest_request: '',
            source: 'demo',
            updated_at: startedAt,
            last_activity: 'command',
            last_activity_at: endedAt - 5,
            completed_tools: { read: 4, command: 1 },
        },
        roles: [],
        process_ids: [],
        workload_ids: [],
    };
    return {
        task_id: 'EXAMPLE-41',
        task_title: 'Check keyboard focus behavior',
        status: 'completed',
        active_agents: 0,
        cpu_percent: 0,
        memory_bytes: 0,
        process_count: 0,
        peak_cpu_percent: 12,
        peak_memory_bytes: peakMemory,
        peak_process_count: 3,
        started_at: startedAt,
        last_seen_at: endedAt,
        observed_samples: readings.length,
        agents: [agent],
        peak_roles: [
            {
                role: 'test',
                process_count: 2,
                cpu_percent: 10,
                memory_bytes: 384 * 1024 ** 2,
            },
            {
                role: 'agent',
                process_count: 1,
                cpu_percent: 2,
                memory_bytes: 128 * 1024 ** 2,
            },
        ],
        narrative: {
            assignments: [{ agent: agent.agent, request, source: 'demo' }],
            task_key: 'synthetic-completed-focus-checks',
            activity:
                'Synthetic example: the Muse session reported four reads and one command, then exited successfully. These events do not prove that every keyboard behavior is correct.',
            resources:
                'Synthetic example: focused test workers accounted for most CPU use at the recorded peak. The session has ended and has no current attributed processes.',
            tip: 'No current optimization is suggested for this finished example.',
            tip_basis: 'Demo · historical measurements only · no action applied',
        },
        history: readings.map((cpu, index) => ({
            time: startedAt + (index + 1) * 5,
            cpu_percent: cpu,
            memory_bytes: cpu === 0 ? 0 : Math.round((256 + (cpu / 12) * 256) * 1024 ** 2),
        })),
        suggestions: [],
        evidence: ['Synthetic completed session. No real agents, processes, or requests.'],
    };
}

/** A completed short task without retained samples, to show the honest empty state. */
export function demoUnobservedWorkflow(time: number): AgentWorkflow {
    const completed = demoCompletedWorkflow(time - 60);
    const request = 'Check how the utility handles out-of-range inputs.';
    return {
        ...completed,
        task_id: 'EXAMPLE-40',
        task_title: 'Check a small utility function',
        observed_samples: 0,
        peak_cpu_percent: 0,
        peak_memory_bytes: 0,
        peak_process_count: 0,
        peak_roles: [],
        history: [],
        agents: completed.agents.map((agent) => ({
            ...agent,
            run_id: 'demo-unsampled-run',
            harness: 'claude',
            agent: 'Utility check',
            observed_samples: 0,
            peak_cpu_percent: 0,
            peak_memory_bytes: 0,
            peak_process_count: 0,
            context: { ...agent.context, request, session_id: 'example-utility-check' },
        })),
        narrative: {
            ...completed.narrative,
            task_key: 'demo-unsampled-task',
            assignments: [{ agent: 'Utility check', request, source: 'demo' }],
            resources: 'No samples captured in this synthetic example.',
            activity: 'Synthetic completed session without resource samples.',
        },
    };
}
