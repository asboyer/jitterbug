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

<details class="investigation" aria-label="Process investigation evidence">
    <summary
        ><span>Measurements & actions</span><span class="evidence-preview"
            >{percent(evidence.cpu_percent)} CPU · {memory(evidence.memory_bytes)}</span
        ></summary
    >
    <div class="evidence-readings">
        <div><small>CPU now</small><strong>{percent(evidence.cpu_percent)}</strong></div>
        <div><small>Memory</small><strong>{memory(evidence.memory_bytes)}</strong></div>
        <div><small>Processes</small><strong>{evidence.process_count}</strong></div>
    </div>
    <p class="muted">
        {windowSeconds > 0
            ? `Workload history · ${windowSeconds}s · ${percent(average)} average · ${percent(peak)} peak`
            : 'Collecting resource history…'}
    </p>
    {#each evidence.processes as process}
        <div class="investigation-process">
            <div class="process-reading">
                <strong>{process.name}</strong><span>PID {process.pid}</span><span
                    >{percent(process.cpu_percent)} CPU</span
                >
            </div>
            {#if process.actions.length}<div class="process-actions">
                    {#each process.actions as action}<button
                            {disabled}
                            onclick={() => onaction(process, action)}
                            >{action === 'lower_priority'
                                ? 'Lower priority…'
                                : 'Stop process…'}</button
                        >{/each}
                </div>{/if}
        </div>
    {/each}
    <details>
        <summary>Measurement details</summary>
        <p class="footnote">{evidence.limits}</p>
        <p class="footnote">
            Captured {new Date(evidence.captured_at * 1000).toLocaleTimeString()}. Actions require
            confirmation.
        </p>
    </details>
</details>

<style>
    .investigation {
        margin-top: 18px;
        padding: 12px 0 0;
        border: 0;
        border-top: 1px solid var(--border);
        background: transparent;
        border-radius: 0;
        font-size: 11px;
    }
    summary {
        color: var(--muted);
        line-height: 1.6;
    }
    .evidence-preview {
        display: inline-block;
        margin-left: 12px;
        font-variant-numeric: tabular-nums;
    }
    .evidence-readings {
        display: grid;
        grid-template-columns: repeat(3, 1fr);
        gap: 14px;
        padding: 18px 0 8px;
    }
    .evidence-readings div {
        display: grid;
        gap: 6px;
    }
    small {
        font-size: 10px;
        color: var(--muted);
    }
    .evidence-readings strong {
        font-size: 18px;
        font-weight: 500;
    }
    .process-reading {
        display: flex;
        gap: 8px 12px;
        align-items: baseline;
        flex-wrap: wrap;
    }
    .process-reading strong {
        font-size: 12px;
    }
    .process-reading span {
        color: var(--muted);
        font-size: 10px;
    }
    .process-actions {
        display: flex;
        gap: 8px;
        margin-top: 10px;
    }
</style>
