import { invoke, isTauri } from '@tauri-apps/api/core';

const cache = new Map<string, { expires: number; promise: Promise<string | null> }>();
const queue: (() => void)[] = [];
let active = 0;

async function request(workloadId: string): Promise<string | null> {
    if (active >= 2) await new Promise<void>((resolve) => queue.push(resolve));
    else active++;
    try {
        return await invoke<string | null>('get_workload_icon', { workloadId });
    } catch {
        return null;
    } finally {
        const next = queue.shift();
        if (next) next();
        else active--;
    }
}

/** Artwork only, in memory. No process details or machine data are persisted by the UI. */
export function applicationIcon(workloadId: string): Promise<string | null> {
    if (!isTauri()) return Promise.resolve(null);
    const existing = cache.get(workloadId);
    if (existing && existing.expires > Date.now()) return existing.promise;
    if (cache.size >= 128) cache.delete(cache.keys().next().value!);
    const promise = request(workloadId);
    cache.set(workloadId, { expires: Date.now() + 600_000, promise });
    return promise;
}
