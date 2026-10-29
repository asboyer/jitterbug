<script lang="ts">
    import { memory, percent } from '../lib/format';
    import type { Investigation, ProcessRecord } from '../lib/types';

    let {
        evidence,
        disabled,
        onaction,
    }: {
        evidence: Investigation;
        disabled: boolean;
        onaction: (process: ProcessRecord, action: string) => void;
    } = $props();
    const windowSeconds = $derived(
        evidence.history.length > 1
            ? evidence.history[evidence.history.length - 1].time - evidence.history[0].time
            : 0
    );
    const average = $derived(
        evidence.history.reduce((sum, p) => sum + p.cpu_percent, 0) /
            Math.max(evidence.history.length, 1)
    );
    const peak = $derived(Math.max(0, ...evidence.history.map((p) => p.cpu_percent)));
</script>

<section class="investigation" aria-label="Process investigation evidence">
    <p class="eyebrow">
        MEASURED EVIDENCE · {new Date(evidence.captured_at * 1000).toLocaleTimeString()}
    </p>
    <h3>{evidence.target.label}{evidence.target.pid ? ` · PID ${evidence.target.pid}` : ''}</h3>
    <p>
        {percent(evidence.cpu_percent)} CPU · {memory(evidence.memory_bytes)} summed memory · {evidence.process_count}
        processes
    </p>
    <p class="muted">
        {windowSeconds > 0
            ? `Whole workload over ${windowSeconds}s: ${percent(average)} average CPU, ${percent(peak)} peak.`
            : 'Still collecting history; a sustained trend is not established yet.'}
    </p>
    <details>
        <summary>Inspect processes & supported actions</summary>
        {#each evidence.processes as process}
            <div class="investigation-process">
                <strong>{process.name}</strong>
                <small
                    >PID {process.pid} · {percent(process.cpu_percent)} CPU · {memory(
                        process.memory_bytes
                    )}</small
                >
                <small
                    >Last interval: {memory(process.disk_read_bytes)} read · {memory(
                        process.disk_written_bytes
                    )} written</small
                >
                <div class="process-actions">
                    {#each process.actions as action}
                        <button {disabled} onclick={() => onaction(process, action)}>
                            {action === 'lower_priority' ? 'Lower priority…' : 'Stop process…'}
                        </button>
                    {/each}
                    {#if !process.actions.length}<small>Protected / action unavailable</small>{/if}
                </div>
            </div>
        {/each}
        <p class="footnote">{evidence.limits}</p>
        <p class="footnote">
            Actions require confirmation. Lower priority helps competing work; it does not free RAM
            or fix an app’s underlying problem. Stop interrupts the selected process.
        </p>
    </details>
</section>
