import type { AppProfile, ProcessRecord, Reply, Snapshot, Workload } from './types';
import { demoDetails } from './demo';
import { errorMessage } from './format';

export type Identity = Pick<ProcessRecord, 'pid' | 'start_time' | 'workload_id'>;
export type MonitorRow = {
    workload: Workload;
    process: ProcessRecord | null;
    identity: Identity | null;
};
export type Inventory = {
    rows: MonitorRow[];
    total: number;
    offset: number;
    captured_at: number;
    observed_processes: number;
    system_processes: number;
};
export type Query = { search: string; view: string; sort: string; filter: string; offset: number };
export type Explanation = { identity: Identity; profile: AppProfile };
export const identityKey = (p: Identity) => `${p.workload_id}:${p.pid}:${p.start_time}`;

export function demoInventory(snapshot: Snapshot, query: Query): Inventory {
    const search = query.search.trim().toLowerCase();
    const rows: MonitorRow[] = [];
    let observed = 0;
    for (const workload of snapshot.workloads) {
        const detail = demoDetails(snapshot, workload.id);
        observed += detail.members.length;
        if (query.filter === 'system' && workload.category !== 'system') continue;
        if (
            query.filter === 'apps' &&
            !['application', 'app', 'browser', 'editor'].includes(workload.category)
        )
            continue;
        const groupMatch = !search || workload.label.toLowerCase().includes(search);
        const matching = detail.members.filter(
            (p) => groupMatch || p.name.toLowerCase().includes(search) || String(p.pid) === search
        );
        if (query.view === 'apps') {
            if (matching.length) rows.push({ workload, process: null, identity: detail.process });
        } else for (const process of matching) rows.push({ workload, process, identity: process });
    }
    rows.sort((a, b) => {
        const first = a.process ?? a.workload;
        const second = b.process ?? b.workload;
        if (query.sort === 'memory') return second.memory_bytes - first.memory_bytes;
        if (query.sort === 'name')
            return (a.process?.name ?? a.workload.label).localeCompare(
                b.process?.name ?? b.workload.label
            );
        if (query.sort === 'cpu') return second.cpu_percent - first.cpu_percent;
        return (
            second.cpu_percent +
            (second.memory_bytes / snapshot.memory_total_bytes) * 100 -
            (first.cpu_percent + (first.memory_bytes / snapshot.memory_total_bytes) * 100)
        );
    });
    const offset = Math.min(query.offset, Math.floor(Math.max(rows.length - 1, 0) / 40) * 40);
    return {
        rows: rows.slice(offset, offset + 40),
        total: rows.length,
        offset,
        captured_at: snapshot.captured_at,
        observed_processes: observed,
        system_processes: snapshot.process_count,
    };
}

// One request from Monitor at a time, with selected-process requests ahead of background work.
// Native inference serialization still covers incidents, other windows and external callers.
type Request = {
    priority: number;
    run: () => Promise<unknown>;
    valid: () => boolean;
    resolve: (value: unknown) => void;
    reject: (cause: unknown) => void;
};
const requests: Request[] = [];
let running = false;
export const cancelled = 'This selection changed before the investigation started.';
export function queueInference<T>(
    run: () => Promise<T>,
    priority = 0,
    valid: () => boolean = () => true
): Promise<T> {
    return new Promise<T>((resolve, reject) => {
        requests.push({ run, priority, valid, resolve: (value) => resolve(value as T), reject });
        void drain();
    });
}
async function drain() {
    if (running) return;
    running = true;
    // Allow simultaneous background and selection requests to be ordered together.
    await new Promise((resolve) => setTimeout(resolve, 40));
    while (requests.length) {
        requests.sort((a, b) => b.priority - a.priority);
        const next = requests.shift()!;
        if (!next.valid()) {
            next.reject(cancelled);
            continue;
        }
        try {
            let result: unknown;
            for (let attempt = 0; ; attempt++) {
                if (!next.valid()) throw cancelled;
                try {
                    result = await next.run();
                    break;
                } catch (cause) {
                    if (attempt >= 2 || !/model request.*active/i.test(errorMessage(cause)))
                        throw cause;
                    await new Promise((resolve) => setTimeout(resolve, 1500));
                }
            }
            next.resolve(result);
        } catch (cause) {
            next.reject(cause);
        }
    }
    running = false;
}

export function overviewKey(snapshot: Snapshot): string {
    return JSON.stringify([
        snapshot.source,
        snapshot.pressure.level,
        snapshot.platform.thermal,
        Math.floor(snapshot.cpu_percent / 15),
        Math.floor(
            (snapshot.memory_available_bytes / Math.max(snapshot.memory_total_bytes, 1)) * 10
        ),
        snapshot.processes.slice(0, 3).map((p) => [p.name, Math.floor(p.cpu_percent / 15)]),
    ]);
}
export function demoOverview(snapshot: Snapshot): Reply {
    const busy = snapshot.pressure.level !== 'normal';
    const insight = {
        headline: busy
            ? 'Several processes are competing for capacity'
            : 'Your computer has room to work',
        finding: busy
            ? 'In this synthetic example, development tools are using most of the CPU while available memory is shrinking.'
            : 'In this synthetic example, the browser and editor have available capacity. No sustained system pressure is detected.',
        next_step: busy
            ? 'Inspect a busy process to understand its role before changing anything.'
            : 'Keep working. Select an unfamiliar process to see what it does.',
    };
    return {
        text: Object.values(insight).join('\n\n'),
        insight,
        source: 'demo',
        tier: 'demo',
        provider: null,
        model: null,
        route: ['Synthetic data. No model called.'],
        estimated_cost_usd: 0,
        investigation: null,
        choices: [],
    };
}
