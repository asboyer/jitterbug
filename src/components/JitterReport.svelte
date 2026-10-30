<script lang="ts">
    import type { AutomationCase, JitterReport, ProcessRecord } from '../lib/types';
    import ActivityGraph from './ActivityGraph.svelte';

    let {
        report,
        incident,
        canAct,
        onreview,
        onstop,
    }: {
        report: JitterReport;
        incident: AutomationCase;
        canAct: boolean;
        onreview: () => void;
        onstop?: (process: ProcessRecord) => void;
    } = $props();

    const span = $derived(Math.max(report.window[1] - report.window[0], 1));
    const fraction = (offset: number) =>
        Math.min(Math.max((offset - report.window[0]) / span, 0), 1);
    // Place samples by their real offset so a short or gappy capture is not stretched.
    const values = $derived.by(() => {
        const series: number[] = Array(span + 1).fill(0);
        for (const point of report.timeline) {
            const index = Math.round(point.offset_s - report.window[0]);
            if (index >= 0 && index < series.length) series[index] = point.core_max;
        }
        return series;
    });
    const lagPeak = $derived(Math.max(0, ...report.timeline.map((p) => p.lag_ms)));
    const strengthLabels = {
        strong: 'Strong evidence',
        possible: 'Possible',
        unexplained: 'Unexplained',
    };
    const reviewable = $derived(
        incident.stage === 'awaiting_approval' && incident.proposal?.action === 'lower_priority'
    );
</script>

<section class="jitter-report" aria-label="Jitter Snapshot findings">
    <figure class="jitter-timeline">
        <ActivityGraph
            {values}
            marker={fraction(0)}
            shade={[fraction(report.suspect[0]), fraction(report.suspect[1])]}
            label={`Busiest CPU core each second from ${report.window[0]} to +${report.window[1]} seconds around the ${report.origin === 'spike' ? 'detected hitch' : 'snapshot'}`}
        />
        <figcaption>
            <span>{report.window[0]}s</span>
            <span
                >Busiest core · shaded: suspect window · line: {report.origin === 'spike'
                    ? 'hitch'
                    : 'your press'}{lagPeak >= 20
                    ? ` · timer delay up to ${lagPeak.toFixed(0)} ms`
                    : ''}</span
            >
            <span>+{report.window[1]}s</span>
        </figcaption>
    </figure>
    {#if report.victim}<p class="victim">
            You were using <strong>{report.victim.label}</strong>.
        </p>{/if}

    <h3>What happened</h3>
    <p><span class="evidence-tag">Observed</span>{report.happened}</p>

    <h3>Likely cause</h3>
    <p><span class="evidence-tag inferred">Inferred</span>{report.likely_cause}</p>

    <h3>What you can do</h3>
    <ul class="jitter-actions">
        {#if report.in_app}
            <li>
                {#if report.in_app.action === 'terminate'}
                    <span>Stop <strong>{report.in_app.process.name}</strong> to free memory.</span>
                    <button
                        disabled={!canAct || !onstop || incident.source !== 'live'}
                        onclick={() => report.in_app && onstop?.(report.in_app.process)}
                        >Stop process…</button
                    >
                {:else}
                    <span
                        >Lower <strong>{report.in_app.process.name}</strong>’s priority so {report
                            .victim?.label ?? 'your app'} gets the CPU first.</span
                    >
                    <button class="primary" disabled={!reviewable || !canAct} onclick={onreview}
                        >Review change</button
                    >
                {/if}
            </li>
        {/if}
        {#if report.manual}<li><span>{report.manual}</span></li>{/if}
    </ul>

    <h3>If it happens again</h3>
    <p>{report.if_again}</p>

    <details class="jitter-findings">
        <summary>Findings ({report.findings.length})</summary>
        {#each report.findings as finding}
            <article class="jitter-finding">
                <div>
                    <strong>{finding.title}</strong>
                    <span class={`strength ${finding.strength}`}
                        >{strengthLabels[finding.strength]}</span
                    >
                </div>
                <ul>
                    {#each finding.observed as line}<li>
                            <span class="evidence-tag">Observed</span>{line}
                        </li>{/each}
                    <li><span class="evidence-tag inferred">Inferred</span>{finding.inferred}</li>
                </ul>
            </article>
        {/each}
        <p class="footnote">{report.limits}</p>
    </details>
</section>

<style>
    .jitter-report h3 {
        font-size: 13px;
        margin: 18px 0 6px;
    }
    .jitter-report p {
        font-size: 13px;
        line-height: 1.6;
        margin: 0;
    }
    .jitter-timeline {
        margin: 0 0 12px;
    }
    .jitter-timeline figcaption {
        display: flex;
        justify-content: space-between;
        gap: 8px;
        color: var(--muted);
        font-size: 10px;
        margin-top: 4px;
    }
    .victim {
        color: var(--muted);
    }
    .evidence-tag {
        display: inline-block;
        font-size: 9px;
        letter-spacing: 0.4px;
        text-transform: uppercase;
        color: var(--muted);
        border: 1px solid var(--border);
        border-radius: 4px;
        padding: 1px 4px;
        margin-right: 6px;
        vertical-align: 1px;
    }
    .evidence-tag.inferred {
        border-style: dashed;
    }
    .jitter-actions {
        list-style: none;
        padding: 0;
        margin: 0;
        display: grid;
        gap: 10px;
        font-size: 13px;
        line-height: 1.5;
    }
    .jitter-actions li {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 10px;
        flex-wrap: wrap;
    }
    .jitter-findings {
        margin-top: 18px;
        border-top: 1px solid var(--border);
        font-size: 12px;
    }
    .jitter-findings > summary {
        padding: 12px 0;
        cursor: pointer;
        color: var(--muted);
        font-size: 11px;
    }
    .jitter-finding {
        padding: 0 0 12px;
    }
    .jitter-finding > div {
        display: flex;
        justify-content: space-between;
        gap: 8px;
        flex-wrap: wrap;
    }
    .jitter-finding ul {
        margin: 6px 0 0;
        padding-left: 0;
        list-style: none;
        display: grid;
        gap: 4px;
        line-height: 1.5;
    }
    .strength {
        font-size: 10px;
        color: var(--muted);
    }
    .strength.strong {
        color: var(--warning);
    }
</style>
