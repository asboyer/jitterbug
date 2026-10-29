<script lang="ts">
    import { memory, percent } from '../lib/format';
    import type { ActionReport } from '../lib/types';
    let { report }: { report: ActionReport } = $props();
</script>

<section class="investigation" role="status" aria-label="Action measurement">
    <p class="eyebrow">ACTION RESULT · MEASURED</p>
    <strong>{report.before.name} · PID {report.before.pid}</strong>
    <p>{report.message}</p>
    {#if !report.fresh}
        <p>
            A fresh follow-up sample was unavailable. The action was applied, but its effect has not
            been measured.
        </p>
    {:else if report.after}
        <p>
            After {report.measured_after_seconds}s: CPU {percent(report.before.cpu_percent)} → {percent(
                report.after.cpu_percent
            )}; memory {memory(report.before.memory_bytes)} → {memory(report.after.memory_bytes)}.
        </p>
        <small>This is an observed change, not proof the action caused an improvement.</small>
    {:else}
        <p>
            After {report.measured_after_seconds}s, this process identity is no longer in the
            collected inventory. An app may restart with a new PID.
        </p>
    {/if}
</section>
