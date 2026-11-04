<script lang="ts">
    import { onDestroy, untrack } from 'svelte';
    import { queueInference, cancelled } from '../lib/monitor';
    import { invoke } from '@tauri-apps/api/core';
    import type { Snapshot, Workload, Reply, ProcessRecord, ResourcePoint } from '../lib/types';
    import { errorMessage } from '../lib/format';
    import InsightCard from './InsightCard.svelte';
    import Investigation from './Investigation.svelte';
    import Icon from './Icon.svelte';
    import ActivityGraph from './ActivityGraph.svelte';
    import { insightKey, readInsight, saveInsight } from '../lib/process-insight';
    import { memory, percent } from '../lib/format';
    let {
        snapshot,
        workload,
        process,
        role,
        demo,
        disabled,
        onusage,
        onask,
        onaction,
        settingsKey = '',
        evidenceKey = '',
        history = [],
    }: {
        settingsKey?: string;
        evidenceKey?: string;
        history?: ResourcePoint[];
        snapshot: Snapshot;
        workload: Workload;
        process: ProcessRecord;
        role: string;
        demo: boolean;
        disabled: boolean;
        onusage: () => void;
        onask: (reply: Reply | null) => void;
        onaction: (process: ProcessRecord, action: string) => void;
    } = $props();
    let reply: Reply | null = $state(null);
    let busy = $state(false);
    let error = $state('');
    let cached = $state(false);
    let savedAt = $state(0);
    let answerKey = $state('');
    let context = '';
    let disposed = false;
    let autoContext = '';
    onDestroy(() => {
        disposed = true;
    });
    const key = $derived(
        `${settingsKey}:${insightKey(snapshot, process, `${evidenceKey}:${role}`)}`
    );
    const stale = $derived(
        !!reply && (answerKey !== key || snapshot.captured_at * 1000 - savedAt >= 90_000)
    );
    $effect(() => {
        const identity = `${settingsKey}:${snapshot.source}:${workload.id}:${process.pid}:${process.start_time}:${evidenceKey}`;
        const currentKey = key;
        untrack(() => {
            if (identity !== context) {
                context = identity;
                const saved = readInsight(currentKey);
                reply = saved?.reply ?? null;
                savedAt = saved?.savedAt ?? 0;
                cached = !!saved;
                answerKey = saved ? currentKey : '';
                error = '';
            }
        });
    });
    $effect(() => {
        const identity = `${settingsKey}:${snapshot.source}:${workload.id}:${process.pid}:${process.start_time}:${evidenceKey}`;
        if (disabled || busy || !snapshot.sample_ready || identity === autoContext) return;
        const timer = setTimeout(() => {
            autoContext = identity;
            if (!reply) void check();
        }, 350);
        return () => clearTimeout(timer);
    });
    async function check() {
        if (busy || disabled) return;
        const identity = context;
        const requestKey = key;
        const target = { ...process };
        const sample = snapshot;
        const observedRole = role;
        busy = true;
        error = '';
        try {
            const result: Reply = demo
                ? {
                      text: `${target.name}: ${observedRole}`,
                      source: 'demo',
                      tier: 'demo',
                      provider: null,
                      model: null,
                      route: ['Synthetic process-specific example. No model called.'],
                      estimated_cost_usd: 0,
                      investigation: {
                          target: { workload_id: workload.id, label: target.name, pid: target.pid },
                          captured_at: sample.captured_at,
                          cpu_percent: target.cpu_percent,
                          memory_bytes: target.memory_bytes,
                          process_count: 1,
                          history: [],
                          processes: [target],
                          limits: 'Synthetic measurements. This finding concerns only the selected process.',
                      },
                      choices: [],
                      insight: {
                          headline: `${target.name} is ${target.cpu_percent >= 20 ? 'busy' : 'lightly active'}`,
                          finding: `${observedRole} This example process uses ${percent(target.cpu_percent)} of total CPU and ${memory(target.memory_bytes)} of memory.`,
                          next_step:
                              target.cpu_percent >= 20
                                  ? 'Let this task finish before starting another heavy check.'
                                  : 'Keep it running; these readings alone do not identify a slowdown.',
                      },
                  }
                : await queueInference<Reply>(
                      () =>
                          invoke('ask', {
                              messages: [
                                  {
                                      role: 'user',
                                      content:
                                          'Investigate this selected process. Explain what this specific instance is doing now using its observed launch role, parent, CPU, memory and disk activity. Distinguish measured evidence from inference and say what you cannot determine. Give one useful next step.',
                                  },
                              ],
                              workloadId: workload.id,
                              tier: 'auto',
                              pid: target.pid,
                              startTime: target.start_time,
                          }),
                      10,
                      () => !disposed && identity === context
                  );
            const saved = saveInsight(requestKey, result);
            if (!disposed && identity === context) {
                reply = result;
                savedAt = saved.savedAt;
                answerKey = requestKey;
                cached = false;
            }
            if (!demo) onusage();
        } catch (cause) {
            if (!disposed && identity === context && cause !== cancelled)
                error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
</script>

<section class="activity-insight panel" aria-label="Process activity insight" aria-busy={busy}>
    <div class="reading">
        <span class="reading-icon"><Icon name="activity" size={20} /></span>
        <div>
            <small>SELECTED PROCESS · PID {process.pid}</small>
            <h2>What is this process doing?</h2>
        </div>
    </div>
    <p class="process-target">
        <strong>{process.name}</strong> · {percent(process.cpu_percent)} CPU · {memory(
            process.memory_bytes
        )}
    </p>
    {#if history.length > 1}<div class="process-history">
            <ActivityGraph
                values={history.map((p) => p.cpu_percent)}
                label="Selected process CPU history since inspection"
            /><small>CPU since inspection · {history.length} observed samples</small>
        </div>{/if}
    {#if reply}
        <InsightCard {reply} />
        {#if stale}<p class="freshness" role="status">
                Activity has changed or this finding has expired. Investigate again for a fresh
                answer.
            </p>
        {:else if cached}<p class="freshness" role="status">
                Reused recent finding · {new Date(savedAt).toLocaleTimeString([], {
                    hour: 'numeric',
                    minute: '2-digit',
                })} · no new model call.
            </p>{/if}
        {#if reply.investigation}<Investigation
                evidence={reply.investigation}
                disabled={disabled || demo || busy || stale || cached}
                {onaction}
            />{/if}
    {:else}
        <p class="intro">{role}</p>
        <p class="freshness">
            {demo ? 'Synthetic example' : 'Local evidence · not an AI finding'}. {busy
                ? 'Connecting its role with its current activity…'
                : 'AI connects its role with its current activity.'}
        </p>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="insight-actions">
        <button class="primary" disabled={busy || disabled} onclick={check}
            >{busy
                ? 'Investigating process…'
                : reply
                  ? 'Investigate again'
                  : 'Investigate process'}</button
        >
        <button class="text-button" disabled={busy} onclick={() => onask(stale ? null : reply)}
            >Ask a follow-up <span aria-hidden="true">↗</span></button
        >
    </div>
</section>

<style>
    .process-history {
        margin-bottom: 16px;
    }
    .process-history :global(svg) {
        height: 40px;
    }
    .process-target {
        font-size: 12px;
        margin: 14px 0;
    }
    .freshness {
        font-size: 11px;
        color: var(--muted);
        line-height: 1.6;
    }
    .activity-insight {
        padding: 20px;
        margin: 0 0 20px;
    }
    .reading {
        display: flex;
        gap: 12px;
        align-items: center;
    }
    .reading-icon {
        display: grid;
        place-items: center;
        width: 40px;
        height: 40px;
        border-radius: 12px;
        color: var(--accent);
        background: var(--accent-soft);
    }
    small {
        color: var(--muted);
        font-size: 11px;
    }
    h2 {
        font-size: 17px;
        margin: 4px 0 0;
        letter-spacing: -0.2px;
    }
    .intro {
        color: var(--muted);
        font-size: 12px;
        line-height: 1.6;
        margin: 14px 0 0;
    }
    .insight-actions {
        display: flex;
        gap: 12px;
        align-items: center;
        flex-wrap: wrap;
        margin-top: 18px;
    }
</style>
