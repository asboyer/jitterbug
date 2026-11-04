import type { ProcessRecord, Reply, Snapshot } from './types';

const ttl = 90_000;
const limit = 24;
type Entry = { reply: Reply; savedAt: number };
const findings = new Map<string, Entry>();

// Memory only: never persist process findings or send them to another process's context.
export function insightKey(snapshot: Snapshot, process: ProcessRecord, role: string): string {
    return JSON.stringify([
        snapshot.source,
        process.workload_id,
        process.pid,
        process.start_time,
        process.parent_pid,
        process.name,
        role,
        snapshot.pressure.level,
        snapshot.platform.thermal,
        snapshot.platform.memory_pressure,
        process.unresponsive,
        Math.floor(process.cpu_percent / 10),
        Math.floor(process.memory_bytes / (256 * 1024 * 1024)),
        Math.floor(Math.log2(Math.max(process.memory_bytes / 1048576, 1))),
        Math.floor(Math.log2(process.disk_read_bytes + 1) / 4),
        Math.floor(Math.log2(process.disk_written_bytes + 1) / 4),
    ]);
}

export function readInsight(key: string): Entry | null {
    const now = Date.now();
    for (const [id, entry] of findings) {
        if (now - entry.savedAt >= ttl) findings.delete(id);
    }
    const entry = findings.get(key);
    return entry ? structuredClone(entry) : null;
}

export function saveInsight(key: string, reply: Reply): Entry {
    const entry = { reply: JSON.parse(JSON.stringify(reply)) as Reply, savedAt: Date.now() };
    // A failed/provider-unavailable answer must remain retryable.
    if (reply.source === 'model' || reply.source === 'demo') {
        findings.delete(key);
        findings.set(key, entry);
        while (findings.size > limit) findings.delete(findings.keys().next().value!);
    }
    return entry;
}

export function clearInsights() {
    findings.clear();
}
