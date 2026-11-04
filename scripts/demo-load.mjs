// Demo load: keeps a few CPU threads busy so Jitterbug's slowdown detection has something
// real to find. Synthetic work only (hashing in memory); stops on its own.
// Usage: node scripts/demo-load.mjs [threads=8] [minutes=3]
import { Worker, isMainThread } from 'node:worker_threads';
import { createHash } from 'node:crypto';

if (isMainThread) {
    const threads = Math.max(1, Math.min(8, Number(process.argv[2] ?? 8)));
    const minutes = Math.max(0.5, Math.min(15, Number(process.argv[3] ?? 3)));
    console.log(`Busy on ${threads} threads for ${minutes} min. Ctrl+C stops it.`);
    const workers = Array.from(
        { length: threads },
        () => new Worker(new URL(import.meta.url), { argv: [] })
    );
    setTimeout(() => {
        workers.forEach((w) => void w.terminate());
        console.log('Done.');
    }, minutes * 60_000);
} else {
    let block = Buffer.alloc(4096, 1);
    for (;;) block = createHash('sha256').update(block).digest();
}
