import { demoWorkflow, demoCompletedWorkflow, demoUnobservedWorkflow } from './demo-workflow';
import type { AutomationCase, Bootstrap, Snapshot, WorkloadDetails } from './types';

export function demoDetails(snapshot: Snapshot, id: string): WorkloadDetails {
    const workload = snapshot.workloads.find((w) => w.id === id)!;
    const process = snapshot.processes.find((p) => p.workload_id === id)!;
    return {
        captured_at: snapshot.captured_at,
        workload,
        process,
        members: [process],
        member_count: 1,
        independent_roots: 1,
        application: null,
        can_generate_profile: false,
        profile: {
            text: workload.description,
            source: 'demo',
            cached: false,
            provider: null,
            model: null,
            generated_at: null,
        },
        ancestry: [
            {
                pid: 8000,
                start_time: process.start_time - 60,
                name: 'Example Terminal',
                executable: '/example/Terminal.app/terminal',
                working_directory: null,
            },
            {
                pid: 8001,
                start_time: process.start_time - 30,
                name: 'Example shell',
                executable: '/example/bin/shell',
                working_directory: '/example/project',
            },
            {
                pid: process.pid,
                start_time: process.start_time,
                name: process.name,
                executable: `/example/bin/${process.name}`,
                working_directory: '/example/project',
            },
        ],
        origin_summary:
            'Synthetic example: a terminal started a shell, which started this process. No real process was inspected.',
        origin_limit:
            'Demo only. A real trace displays observed OS parents and explicitly identifies missing or ambiguous origins.',
    };
}

/** Synthetic fixtures only. Never replace these with a captured machine snapshot. */
export function demoSnapshot(pressure = false): Snapshot {
    const time = Math.floor(Date.now() / 1000);
    const workloads = [
        {
            id: 'demo-project',
            label: 'sample-web-app',
            category: 'project',
            description: 'A development server and build worker share a project working directory.',
            attribution: 'Simulated project-directory match',
            cpu_percent: pressure ? 72 : 18,
            memory_bytes: 2.4 * 1024 ** 3,
            history: [],
            process_count: 6,
        },
        {
            id: 'demo-browser',
            label: 'Example Browser',
            category: 'browser',
            description: 'Browser processes belonging to one application bundle.',
            attribution: 'Simulated application ancestry',
            cpu_percent: 5,
            memory_bytes: 1.2 * 1024 ** 3,
            history: [],
            process_count: 8,
        },
        {
            id: 'demo-model',
            label: 'Tiny local model',
            category: 'model',
            description: 'A simulated local inference runtime. No model is actually running.',
            attribution: 'Synthetic fixture',
            cpu_percent: 0,
            memory_bytes: 0.8 * 1024 ** 3,
            history: [],
            process_count: 1,
        },
    ];
    return {
        source: 'demo',
        captured_at: time,
        sample_ready: true,
        interval_ms: 5000,
        cpu_percent: pressure ? 91 : 26,
        logical_cpus: 8,
        cpu_cores: pressure ? [100, 90, 95, 82, 99, 88, 92, 82] : [64, 45, 32, 21, 18, 10, 10, 8],
        memory_total_bytes: 16 * 1024 ** 3,
        memory_available_bytes: (pressure ? 1.4 : 8) * 1024 ** 3,
        swap_used_bytes: pressure ? 1024 ** 3 : 0,
        swap_growth_bytes: 0,
        swap_in_bytes: pressure ? 24 * 1024 ** 2 : 0,
        swap_out_bytes: pressure ? 48 * 1024 ** 2 : 0,
        process_count: 120,
        workloads,
        agent_workflows: [
            demoWorkflow(time, pressure),
            demoCompletedWorkflow(time),
            demoUnobservedWorkflow(time),
        ],
        processes: workloads.map((w, index) => ({
            pid: 9000 + index,
            start_time: time - 60,
            parent_pid: null,
            name: ['node', 'example-browser', 'ollama'][index],
            cpu_percent: w.cpu_percent,
            memory_bytes: w.memory_bytes,
            workload_id: w.id,
            description: w.description,
            evidence: ['Synthetic example; no real process is associated with this row.'],
            actionable: false,
            actions: [],
            disk_read_bytes: 0,
            disk_written_bytes: 0,
        })),
        platform: {
            os: 'demo',
            memory_pressure: pressure ? 'warning' : 'normal',
            thermal: 'nominal',
            process_actions: false,
            memory: {
                app_bytes: (pressure ? 10.6 : 5) * 1024 ** 3,
                wired_bytes: 2 * 1024 ** 3,
                compressed_bytes: (pressure ? 2 : 1) * 1024 ** 3,
                cached_bytes: (pressure ? 0.5 : 3) * 1024 ** 3,
                used_bytes: (pressure ? 14.6 : 8) * 1024 ** 3,
                swap_in_total_bytes: 0,
                swap_out_total_bytes: 0,
            },
        },
        pressure: {
            level: pressure ? 'high' : 'normal',
            reasons: pressure ? ['Simulated sustained CPU and memory pressure.'] : [],
            sustained_samples: pressure ? 3 : 0,
        },
        incident: pressure
            ? {
                  id: time,
                  created_at: time,
                  title: 'Demo: your computer is under sustained pressure',
                  reasons: ['Synthetic incident. No load was generated on your computer.'],
                  workload_id: 'demo-project',
              }
            : null,
        history: Array.from({ length: 60 }, (_, index) => ({
            time: time - (59 - index) * 5,
            cpu_percent: 18 + Math.sin(index / 5) * 6 + (pressure && index > 48 ? 65 : 0),
            memory_percent: pressure && index > 48 ? 91 : 50,
        })),
        collector_ms: 0,
    };
}

export const demoBootstrap: Bootstrap = {
    settings: {
        mode: 'hybrid',
        decision: { provider: 'ollama', model: 'tev1:0.8b' },
        cheap: { provider: 'ollama', model: 'qwen3.5:0.8b' },
        intelligent: { provider: 'meta', model: 'muse-spark-1.3' },
        decision_enabled: false,
        budget_usd: 2,
        automation: {
            enabled: true,
            model_analysis: true,
            system_cpu_percent: 85,
            available_memory_percent: 12,
            app_cpu_core_percent: 100,
            sustained_samples: 3,
            cooldown_seconds: 300,
            max_investigations_per_hour: 4,
            rules: [],
        },
    },
    automation_cases: [],
    automation_snoozed_until: 0,
    credentials: [],
    spending: { reserved_usd: 0, reported_estimate_usd: 0, requests: 0 },
};

/** Handwritten incidents for UI exploration; all mutation controls remain disabled. */
export function demoIncidents(): AutomationCase[] {
    const snapshot = demoSnapshot(true);
    const process = snapshot.processes[0];
    const base: AutomationCase = {
        id: -1,
        source: 'demo',
        detected_at: snapshot.captured_at,
        trigger: {
            key: 'demo-cpu',
            kind: 'app_cpu',
            title: 'Build workers are busy',
            reason: 'Synthetic example: overlapping checks are using CPU.',
            target: { workload_id: process.workload_id, label: 'sample-web-app', pid: null },
        },
        stage: 'awaiting_approval',
        steps: [
            {
                at: snapshot.captured_at,
                stage: 'awaiting_approval',
                message: 'Example proposal ready to review. Nothing has been changed.',
            },
        ],
        evidence: null,
        diagnosis: {
            source: 'demo',
            provider: null,
            model: null,
            text: 'Synthetic explanation: two checks overlap. Letting checks take turns may leave more room for other apps. This is not a diagnosis of this device.',
            route: ['Synthetic fixture. No model called.'],
            estimated_cost_usd: 0,
            investigation: null,
            choices: [],
            tier: 'demo',
        },
        proposal: {
            process,
            action: 'lower_priority',
            rationale:
                'Give other work scheduling preference. This does not free memory or guarantee lower CPU use.',
        },
        report: null,
        verification: null,
        automatic: false,
    };
    return [
        base,
        {
            ...base,
            id: -2,
            detected_at: snapshot.captured_at - 120,
            stage: 'resolved',
            trigger: {
                ...base.trigger,
                kind: 'memory_pressure',
                title: 'Memory pressure eased',
                target: null,
            },
            proposal: null,
            diagnosis: null,
            steps: [
                {
                    at: snapshot.captured_at - 90,
                    stage: 'resolved',
                    message: 'The example pressure cleared before an action was needed.',
                },
            ],
        },
    ];
}
