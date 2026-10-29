<script lang="ts">
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import { demoQueue, type ValidationQueue } from '../lib/queue';
    import { errorMessage, memory, percent } from '../lib/format';
    import type { Snapshot, Workload } from '../lib/types';
    import AppIcon from './AppIcon.svelte';
    import type { ManagementSelection } from '../lib/manage';

    let {
        snapshot,
        onselect,
        onmanage,
    }: {
        snapshot: Snapshot;
        onselect: (w: Workload) => void;
        onmanage: (selection: ManagementSelection) => void;
    } = $props();
    let queue: ValidationQueue | null = $state(null);
    let error = $state('');
    let busy = $state(false);
    let releaseId: string | null = $state(null);
    let reviewed = $state(false);
    const demo = $derived(snapshot.source === 'demo');
    const active = $derived.by(
        () => queue?.jobs.filter((j) => ['starting', 'running'].includes(j.stage)) ?? []
    );
    const waiting = $derived.by(() => queue?.jobs.filter((j) => j.stage === 'queued') ?? []);
    const jobs = $derived.by(() => {
        const ordered = [...(queue?.jobs ?? [])];
        const rank = (stage: string) =>
            stage === 'interrupted'
                ? 0
                : ['running', 'starting'].includes(stage)
                  ? 1
                  : stage === 'queued'
                    ? 2
                    : 3;
        return ordered.sort(
            (a, b) =>
                rank(a.stage) - rank(b.stage) ||
                (rank(a.stage) === 3
                    ? b.submitted_at - a.submitted_at
                    : a.submitted_at - b.submitted_at)
        );
    });

    $effect(() => {
        snapshot.captured_at;
        const isDemo = demo;
        let disposed = false;
        if (isDemo) queue = demoQueue();
        else if (isTauri()) {
            void invoke<ValidationQueue>('get_validation_queue')
                .then((value) => {
                    if (!disposed) {
                        queue = value;
                        error = '';
                    }
                })
                .catch((cause) => {
                    if (!disposed) error = errorMessage(cause);
                });
        }
        return () => {
            disposed = true;
        };
    });

    async function control(action: string, jobId: string | null = null) {
        if (demo || busy) return;
        busy = true;
        try {
            await invoke('control_validation_queue', { action, jobId });
            queue = await invoke('get_validation_queue');
            releaseId = null;
            reviewed = false;
            error = '';
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
</script>

<section class="panel validation-panel" aria-label="Shared validation queue">
    <div class="queue-heading" data-tour="validation">
        <div class="slot" class:occupied={active.length > 0} aria-hidden="true">
            <svg viewBox="0 0 24 24"><path d="M5 6h14M5 12h9M5 18h5m6-3 3 3-3 3" /></svg>
        </div>
        <div class="queue-title">
            <h2>Validation queue <span class="muted">{demo ? 'Demo' : 'All sessions'}</span></h2>
            <p>{queue?.reason ?? 'Reading queue status…'}</p>
        </div>
        <span class="count">{active.length}/1 running · {waiting.length} waiting</span>
        <button disabled={demo} onclick={() => onmanage({})}>Manage workloads</button>
        <button
            disabled={demo || busy || !queue}
            onclick={() => control(queue?.paused ? 'resume' : 'pause')}
        >
            {queue?.paused ? 'Resume' : 'Pause'}
        </button>
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if queue}
        {#each jobs.slice(0, 4) as job (job.id)}
            <details class="queue-job">
                <summary>
                    <span
                        class="job-mark"
                        class:passed={job.stage === 'passed'}
                        class:attention={['failed', 'stale', 'interrupted'].includes(job.stage)}
                    >
                        {job.stage === 'passed'
                            ? '✓'
                            : job.stage === 'queued'
                              ? '◷'
                              : job.stage === 'running'
                                ? '▶'
                                : '·'}
                    </span>
                    <strong>{job.task}</strong>
                    <span class="muted harness">{job.harness}</span>
                    <span class="stage" class:passed={job.stage === 'passed'}>{job.stage}</span>
                </summary>
                <div class="job-detail">
                    <p>{job.message}</p>
                    <dl>
                        <div>
                            <dt>Session</dt>
                            <dd>{job.session}</dd>
                        </div>
                        <div>
                            <dt>Worktree</dt>
                            <dd>{job.worktree}</dd>
                        </div>
                        <div>
                            <dt>Validation</dt>
                            <dd>{job.profile}</dd>
                        </div>
                        <div>
                            <dt>Revision</dt>
                            <dd>{job.revision?.slice(0, 8) ?? 'Not tracked'}</dd>
                        </div>
                        <div>
                            <dt>Sampled peak</dt>
                            <dd>
                                {job.observed_samples
                                    ? `${percent(job.peak_cpu_percent)} CPU · ${memory(job.peak_memory_bytes)}`
                                    : 'Not sampled'}
                            </dd>
                        </div>
                        <div>
                            <dt>Exit code</dt>
                            <dd>{job.exit_code ?? '—'}</dd>
                        </div>
                    </dl>
                    <small class="muted"
                        >Peaks are sampled every five seconds. Source checks cover Git files,
                        excluding ignored files and external dependencies.</small
                    >
                    {#if job.stage === 'queued'}<button
                            disabled={demo || busy}
                            onclick={() => control('cancel', job.id)}>Cancel waiting job</button
                        >{/if}
                    {#if job.stage === 'interrupted'}
                        <button
                            disabled={demo || busy}
                            onclick={() => {
                                releaseId = job.id;
                                reviewed = false;
                            }}>Review interrupted slot</button
                        >
                        {#if releaseId === job.id}
                            <label class="release-check"
                                ><input type="checkbox" bind:checked={reviewed} /> I checked for surviving
                                validation processes. It is safe to start the next job.</label
                            >
                            <button
                                disabled={!reviewed || busy}
                                onclick={() => control('release', job.id)}>Release slot</button
                            >
                        {/if}
                    {/if}
                </div>
            </details>
        {/each}
        {#if jobs.length > 4}
            <details class="queue-more">
                <summary>{jobs.length - 4} more {jobs.length === 5 ? 'job' : 'jobs'}</summary>
                {#each jobs.slice(4) as job (job.id)}
                    <div class="remaining-job">
                        <span>{job.task}</span><span class="muted">{job.harness} · {job.stage}</span
                        >
                        {#if job.stage === 'queued'}<button
                                disabled={demo || busy}
                                onclick={() => control('cancel', job.id)}>Cancel</button
                            >{/if}
                    </div>
                {/each}
            </details>
        {/if}
        {#if queue.resources.suggestions.length > 0 && queue.resources.captured_at + 15 >= Math.floor(Date.now() / 1000)}
            <details class="contention">
                <summary
                    >Make room <span class="muted"
                        >{queue.resources.suggestions.length} competing workloads</span
                    ></summary
                >
                {#each queue.resources.suggestions as suggestion (suggestion.workload_id)}
                    {@const workload = snapshot.workloads.find(
                        (w) => w.id === suggestion.workload_id
                    )}
                    <div class="suggestion">
                        <AppIcon
                            workloadId={suggestion.workload_id}
                            category={workload?.category ?? 'app'}
                            {demo}
                        />
                        <div>
                            <strong>{suggestion.label}</strong>
                            <span class="muted"
                                >{percent(suggestion.cpu_percent)} CPU · {memory(
                                    suggestion.memory_bytes
                                )}</span
                            >
                            <p>{suggestion.recommendation}</p>
                        </div>
                        <button
                            disabled={!workload}
                            onclick={() => {
                                if (workload) onselect(workload);
                            }}>Inspect</button
                        >
                    </div>
                {/each}
                <small class="muted"
                    >Measured usage; rule-based suggestions. No apps have been stopped or moved.</small
                >
            </details>
        {/if}
        <details class="queue-setup">
            <summary
                >{queue.profiles.length
                    ? `${queue.profiles.length} approved validations`
                    : 'Connect your coding tools'} <span class="muted">How it works</span></summary
            >
            <p>
                Claude, Codex, Muse Code, and your terminal can share this queue. Save a fixed
                validation command with the Jitterbug runner, then connect its MCP server to your
                coding tool. Each session submits its own job and receives the result.
            </p>
            {#if queue.profiles.length}<p>Approved here: {queue.profiles.join(', ')}.</p>{/if}
            <p class="muted">
                Only submitted jobs wait. Keep Jitterbug open to admit jobs. Running work finishes
                normally when paused. Memory estimates guide admission; they do not cap a job's
                actual usage. Remote offloading is not connected.
            </p>
            <code>jitterbug-run queue --help</code>
        </details>
    {/if}
</section>

<style>
    .stage.passed {
        color: var(--success);
    }

    .validation-panel {
        padding: 18px;
        container-type: inline-size;
        container-name: validation;
    }
    .queue-heading {
        display: flex;
        align-items: center;
        gap: 14px 12px;
        flex-wrap: wrap;
    }
    .slot {
        display: grid;
        place-items: center;
        border-radius: 10px;
        width: 38px;
        height: 38px;
        background: var(--hover);
        color: var(--muted);
        flex-shrink: 0;
    }
    .slot.occupied {
        color: var(--accent);
        background: var(--accent-soft);
    }
    .slot svg {
        width: 24px;
        fill: none;
        stroke: currentColor;
        stroke-width: 1.5;
        stroke-linecap: round;
        stroke-linejoin: round;
    }
    .queue-title {
        flex: 1 1 260px;
        min-width: 0;
    }
    .queue-title h2 {
        margin: 0;
        font-size: 14px;
    }
    .queue-title h2 span {
        font-size: 11px;
        font-weight: 400;
        margin-left: 8px;
    }
    .queue-title p {
        margin: 4px 0 0;
        color: var(--muted);
        font-size: 12px;
    }
    .count {
        font-size: 11px;
        color: var(--muted);
        white-space: nowrap;
    }
    .queue-job {
        border-top: 1px solid var(--border);
        margin-top: 10px;
    }
    .queue-job summary {
        display: flex;
        align-items: center;
        gap: 10px;
        padding-top: 10px;
        cursor: pointer;
    }
    .queue-job summary strong {
        flex: 1;
        min-width: 0;
        font-size: 12px;
    }
    .job-mark {
        width: 20px;
        color: var(--muted);
        text-align: center;
    }
    .job-mark.passed {
        color: var(--success);
    }
    .job-mark.attention {
        color: var(--warning);
    }
    .stage {
        text-transform: capitalize;
        font-size: 11px;
        background: var(--hover);
        border-radius: 5px;
        padding: 3px 7px;
    }
    .harness {
        font-size: 11px;
    }
    .job-detail {
        padding: 4px 0 8px 30px;
        font-size: 12px;
    }
    .job-detail dl {
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: 12px;
    }
    dt {
        color: var(--muted);
        font-size: 11px;
    }
    dd {
        margin: 3px 0 0;
        overflow-wrap: anywhere;
    }
    .job-detail small {
        display: block;
        margin-bottom: 12px;
    }
    .release-check {
        display: flex;
        align-items: flex-start;
        gap: 6px;
        margin: 12px 0;
    }
    .contention,
    .queue-setup,
    .queue-more {
        border-top: 1px solid var(--border);
        margin-top: 12px;
        padding-top: 10px;
        font-size: 12px;
    }
    summary {
        cursor: pointer;
    }
    summary .muted {
        margin-left: 8px;
        font-size: 11px;
    }
    .suggestion {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 12px 0;
    }
    .suggestion > div {
        flex: 1;
    }
    .suggestion p {
        margin: 4px 0 0;
        line-height: 1.5;
    }
    .suggestion span {
        font-size: 11px;
    }
    .queue-setup p {
        line-height: 1.6;
        max-width: 76ch;
    }
    code {
        font-size: 11px;
    }
    .remaining-job {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 10px 0;
    }
    .remaining-job > span:first-child {
        flex: 1;
    }
    @container validation (max-width: 700px) {
        .count {
            display: none;
        }
        .job-detail dl {
            grid-template-columns: repeat(2, minmax(0, 1fr));
        }
    }
</style>
