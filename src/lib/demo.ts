import { demoWorkflow, demoCompletedWorkflow, demoUnobservedWorkflow } from './demo-workflow';
import type {
    ActionPopupCase,
    AutomationCase,
    Bootstrap,
    InsideView,
    ProcessLabel,
    ProcessRecord,
    Snapshot,
    Workload,
    WorkloadDetails,
} from './types';

/** Synthetic helper processes, so the demo shows processes named by tab, session, or role. */
function demoMembers(snapshot: Snapshot, id: string): ProcessRecord[] {
    const main = snapshot.processes.find((p) => p.workload_id === id)!;
    const mib = 1024 ** 2;
    const helper = (pid: number, name: string, cpu: number, mem: number, parent = main.pid) => ({
        ...main,
        pid,
        parent_pid: parent,
        name,
        cpu_percent: cpu,
        memory_bytes: mem * mib,
    });
    const renderer = 'Example Browser Helper (Renderer)';
    if (id === 'demo-browser')
        return [
            helper(9101, renderer, 2.4, 380),
            helper(9102, renderer, 1.1, 260),
            helper(9103, renderer, 0.1, 190),
            helper(9104, 'Example Browser Helper (GPU)', 0.8, 150),
            helper(main.pid, main.name, 0.4, 120, 1),
            helper(9105, 'Example Browser Helper', 0.1, 40),
            helper(9106, 'Example Browser Helper', 0.1, 30),
            helper(9107, renderer, 0, 60),
        ];
    if (id === 'demo-editor')
        return [
            helper(9203, 'npm', 1.2, 520, 9202),
            helper(9204, 'node', 0.9, 350, 9203),
            helper(9206, 'claude', 0.5, 330, 9205),
            helper(main.pid, main.name, 0.2, 180, 1),
            helper(9201, 'Example Editor Helper (Renderer)', 0.2, 160),
            helper(9202, 'zsh', 0, 8),
            helper(9205, 'zsh', 0, 8),
            helper(9207, 'zsh', 0, 7),
            helper(9208, 'zsh', 0, 7),
        ];
    return [main];
}

export function demoDetails(
    snapshot: Snapshot,
    id: string,
    selected: { pid: number; start_time: number } | null = null
): WorkloadDetails {
    const workload = snapshot.workloads.find((w) => w.id === id)!;
    const members = demoMembers(snapshot, id);
    const main = snapshot.processes.find((p) => p.workload_id === id)!;
    const process =
        members.find((p) => p.pid === selected?.pid && p.start_time === selected.start_time) ??
        members.find((p) => p.pid === main.pid)!;
    return {
        captured_at: snapshot.captured_at,
        workload,
        process,
        members,
        member_count: members.length,
        independent_roots: 1,
        application: null,
        can_generate_profile: false,
        profile_key: null,
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
        {
            id: 'demo-editor',
            label: 'Example Editor',
            category: 'editor',
            description: 'A simulated code editor with terminal sessions.',
            attribution: 'Simulated application ancestry',
            cpu_percent: 3,
            memory_bytes: 1.6 * 1024 ** 3,
            history: [],
            process_count: 9,
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
            name: ['node', 'example-browser', 'ollama', 'example-editor'][index],
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
            app_cpu_percent: 10,
            sustained_samples: 3,
            cooldown_seconds: 300,
            max_investigations_per_hour: 4,
            rules: [],
            jitter_mode: 'auto',
            action_popups: true,
            auto_snapshots: false,
            agent_control: false,
            agent_pause_without_asking: false,
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
    const press = snapshot.captured_at - 300;
    const builder = { ...snapshot.processes[0], pid: 9100, name: 'example-bundler' };
    const snapshotCase: AutomationCase = {
        ...base,
        id: -3,
        origin: 'snapshot',
        detected_at: press,
        stage: 'completed',
        trigger: {
            key: 'demo-snapshot',
            kind: 'user_reported',
            title: 'Jitter Snapshot',
            reason: 'Synthetic example.',
            target: { workload_id: 'demo-project', label: 'sample-web-app', pid: null },
        },
        proposal: null,
        evidence: null,
        diagnosis: {
            ...base.diagnosis!,
            text: 'Synthetic explanation: a bundler rebuild saturated one core while you typed in the example editor. This is not a diagnosis of this device.',
        },
        steps: [
            {
                at: press,
                stage: 'detected',
                message: 'Example snapshot. No samples were collected on this device.',
            },
            {
                at: press + 12,
                stage: 'completed',
                message: 'Example analysis complete. Nothing has been changed.',
            },
        ],
        jitter: {
            origin: 'snapshot',
            press_at_ms: press * 1000,
            forward_only: false,
            window: [-60, 10],
            suspect: [-15, 10],
            timeline: Array.from({ length: 71 }, (_, index) => {
                const offset = index - 60;
                const busy = offset >= -6 && offset <= 3;
                return {
                    offset_s: offset,
                    cpu_percent: busy ? 21 : 9 + Math.sin(index / 4) * 2,
                    core_max: busy ? 100 : 28 + Math.sin(index / 3) * 8,
                    lag_ms: busy ? 4 : 1,
                    paging_mib_s: 0,
                };
            }),
            findings: [
                {
                    kind: 'core_pegged',
                    strength: 'strong',
                    title: 'sample-web-app saturated one core',
                    observed: [
                        'A CPU core stayed at or near 100% for 10 s starting −6s, while whole-machine CPU averaged 21%.',
                        'sample-web-app (example-bundler) averaged 99% of one core in that run.',
                    ],
                    inferred:
                        'Single-threaded work cannot use the other cores, so anything waiting on that thread stalls even when total CPU looks low.',
                    workload_id: 'demo-project',
                    label: 'sample-web-app',
                    category: 'project',
                },
            ],
            victim: { workload_id: 'demo-browser', label: 'Example Editor', category: 'app' },
            culprit: { workload_id: 'demo-project', label: 'sample-web-app', category: 'project' },
            happened:
                'A CPU core stayed at or near 100% for 10 s starting −6s, while whole-machine CPU averaged 21%.',
            likely_cause:
                'Single-threaded work cannot use the other cores, so anything waiting on that thread stalls even when total CPU looks low.',
            in_app: { action: 'lower_priority', process: builder, label: 'sample-web-app' },
            manual: 'If sample-web-app is not urgent, pause or quit it while you work in Example Editor.',
            if_again:
                'Press ⌘⇧J the moment it happens. Jitterbug keeps the last 60 seconds of one-second samples in memory only.',
            recurrence: 1,
            offer_permission: false,
            limits: 'Synthetic example. GPU, display, disk, and network activity are not measured.',
        },
    };
    return [
        base,
        snapshotCase,
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

/** Synthetic out-of-app popup, trimmed exactly like the desktop payload. Nothing is applied. */
export function demoActionPopup(example: string | null = null): ActionPopupCase {
    const incident = demoIncidents()[0];
    const label = incident.trigger.target?.label ?? 'the app';
    const source = incident.proposal?.process;
    const process = source && {
        name: source.name,
        cpu_percent: source.cpu_percent,
        memory_bytes: source.memory_bytes,
    };
    // The fixture incident carries no evidence block; describe a plausible three-process app.
    const impact = process && {
        label,
        workload_cpu_percent: incident.evidence?.cpu_percent ?? process.cpu_percent + 6,
        workload_memory_bytes: incident.evidence?.memory_bytes ?? process.memory_bytes * 1.5,
        process_count: incident.evidence?.process_count ?? 3,
        whole_app: false,
    };
    return {
        kind: 'case',
        id: incident.id,
        source: 'demo',
        stage: 'awaiting_approval',
        trigger:
            example === 'agent'
                ? {
                      kind: 'agent_request',
                      title: 'An agent in claude · fix-login asks to close the tab “Video call”',
                      target: { label: 'Example Browser' },
                      reason: '“The video call tab uses 640 MiB while the test run needs memory.”',
                  }
                : {
                      kind: incident.trigger.kind,
                      title: incident.trigger.title,
                      target: incident.trigger.target && { label: incident.trigger.target.label },
                  },
        proposal:
            process && example !== 'agent'
                ? { action: 'lower_priority', process, impact: impact ?? null, closes: null }
                : null,
        alternative: !process
            ? null
            : example === 'tab' || example === 'agent'
              ? {
                    action: 'close_tab',
                    process,
                    impact: {
                        label: 'Video call tab',
                        workload_cpu_percent: 9.4,
                        workload_memory_bytes: 640 * 1024 ** 2,
                        process_count: 1,
                        whole_app: false,
                    },
                    closes: 'Close the “Video call” tab (meet.example.com). Anything running in it ends, such as a call, video, or unsaved form input. The rest of Example Browser stays open.',
                    unit: { title: 'Video call', kind: 'tab' },
                }
              : example === 'pause'
                ? {
                      action: 'pause',
                      process,
                      impact: {
                          label: 'claude · feat-search in Example Editor',
                          workload_cpu_percent: 31,
                          workload_memory_bytes: 2.1 * 1024 ** 3,
                          process_count: 7,
                          whole_app: false,
                      },
                      closes: 'Pause claude · feat-search in Example Editor: its 7 processes stop using CPU until you resume it (automatically after 15 minutes). Nothing is closed.',
                      unit: { title: 'claude · feat-search in Example Editor', kind: 'session' },
                  }
                : {
                      action: 'terminate',
                      process,
                      impact: impact ? { ...impact, whole_app: true } : null,
                      closes: `Quit ${label}. Every ${label} window closes, including any unsaved work.`,
                  },
        report: null,
        verification: null,
        origin: example === 'agent' ? 'agent' : 'automatic',
        jitter: null,
        more: example === 'agent' ? 0 : 1,
    };
}

export function demoInside(workload: Workload, snapshot: Snapshot): InsideView {
    const mib = 1024 ** 2;
    const members = demoMembers(snapshot, workload.id);
    const named = (pid: number, title: string, subtitle = ''): ProcessLabel => ({
        pid,
        start_time: members.find((p) => p.pid === pid)?.start_time ?? 0,
        title,
        subtitle,
    });
    const tab = (
        id: string,
        title: string,
        host: string,
        cpu: number,
        mem: number,
        shared = 0,
        pid: number | null = null
    ) => ({
        id,
        kind: 'tab' as const,
        title,
        detail: `https://${host}/`,
        host,
        cpu_percent: cpu,
        memory_bytes: mem * mib,
        process_count: 1,
        shared_with: shared,
        pid,
        start_time: pid === null ? null : (members[0]?.start_time ?? null),
        actions: ['focus_tab', 'close_tab'],
        evidence: ['Synthetic example: no browser was contacted.'],
        confidence: 'Matched by CPU time',
        paused_until: null,
    });
    const session = (
        id: string,
        title: string,
        host: string,
        cpu: number,
        mem: number,
        detail: string,
        actions: string[],
        pid: number
    ) => ({
        ...tab(id, title, host, cpu, mem, 0, pid),
        kind: 'session' as const,
        detail,
        process_count: 4,
        actions,
        evidence: ['Synthetic example: no process was inspected.'],
        confidence: 'Process tree and working directory',
    });
    const browser = workload.category === 'browser';
    if (!browser && workload.category !== 'editor')
        return {
            workload_id: workload.id,
            source: 'None',
            status: 'unavailable',
            message: 'No inside view for this app yet.',
            setup: [],
            units: [],
            remainder: [],
            measured_seconds: 0,
            captured_at: Math.floor(Date.now() / 1000),
            limits: '',
            labels: [],
        };
    return {
        workload_id: workload.id,
        source: browser ? 'Browser DevTools' : 'Process tree',
        status: 'ready',
        message: browser
            ? '4 tabs measured over one second.'
            : '2 commands running inside the app.',
        setup: [],
        units: browser
            ? [
                  tab('d1', 'Video call', 'meet.example.com', 9.4, 640, 0, 9101),
                  tab('d2', 'Browser game', 'games.example.com', 6.1, 410, 0, 9102),
                  tab('d3', 'Docs', 'docs.example.com', 0.2, 180, 1, 9103),
                  tab('d4', 'Mail', 'mail.example.com', 0.1, 150, 1, 9103),
                  tab('d5', 'Recipes', 'recipes.example.com', 0, 0),
                  tab('d6', 'Travel plans', 'travel.example.com', 0, 0),
              ]
            : [
                  session(
                      's2',
                      'npm · sample-web-app',
                      'npm',
                      11.5,
                      870,
                      '~/example/sample-web-app',
                      ['pause', 'lower_priority', 'terminate'],
                      9203
                  ),
                  session(
                      's1',
                      'claude · feat-search',
                      'claude',
                      3.2,
                      520,
                      '~/example/repo.worktrees/feat-search',
                      ['pause'],
                      9206
                  ),
              ],
        remainder: [
            {
                label: browser ? 'Graphics (GPU process, shared by all tabs)' : 'The app itself',
                cpu_percent: 1.2,
                memory_bytes: 300 * mib,
                process_count: browser ? 1 : 6,
            },
        ],
        measured_seconds: browser ? 1 : 0,
        captured_at: Math.floor(Date.now() / 1000),
        limits: 'Measurements are illustrative.',
        labels: browser
            ? [
                  named(9101, 'Video call', 'meet.example.com'),
                  named(9102, 'Browser game', 'games.example.com'),
                  named(9103, '2 tabs: Docs, Mail', 'docs.example.com, mail.example.com'),
                  named(9104, 'Graphics (shared by all tabs)', 'Draws every tab on screen'),
                  named(9105, 'Network & storage', 'Network, storage, audio, and utility services'),
                  named(9106, 'Network & storage', 'Network, storage, audio, and utility services'),
                  named(9107, 'Extensions', 'Extensions, background pages, and workers'),
                  named(
                      members[4]?.pid ?? 0,
                      'Browser window',
                      'Windows, tab strip, and downloads'
                  ),
              ]
            : [
                  ...[9202, 9203, 9204].map((pid) =>
                      named(pid, 'npm · sample-web-app', 'Command session')
                  ),
                  ...[9205, 9206].map((pid) =>
                      named(pid, 'Claude Code session · feat-search', 'Coding agent session')
                  ),
                  named(9207, 'Idle shell', 'A terminal shell with nothing running in it'),
                  named(9208, 'Idle shell', 'A terminal shell with nothing running in it'),
                  named(9201, 'Window or extension', 'Draws a window, panel, or web view'),
                  named(members[3]?.pid ?? 0, 'App window', 'The app’s main process'),
              ],
    };
}
