<script lang="ts">
    import { untrack } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import type { Snapshot, Workload, Reply, ProcessRecord } from '../lib/types';
    import { errorMessage } from '../lib/format';
    import InsightCard from './InsightCard.svelte';
    import Investigation from './Investigation.svelte';
    import Icon from './Icon.svelte';
    let {
        snapshot,
        workload,
        demo,
        disabled,
        onusage,
        onask,
        onaction,
    }: {
        snapshot: Snapshot;
        workload: Workload;
        demo: boolean;
        disabled: boolean;
        onusage: () => void;
        onask: (reply: Reply | null) => void;
        onaction: (process: ProcessRecord, action: string) => void;
    } = $props();
    let reply: Reply | null = $state(null);
    let busy = $state(false);
    let error = $state('');
    let context = '';
    $effect(() => {
        const id = workload.id;
        untrack(() => {
            if (id !== context) {
                context = id;
                reply = null;
                error = '';
            }
        });
    });
    const reading = $derived(
        !snapshot.sample_ready
            ? 'Taking a reading'
            : workload.unresponsive
              ? 'Not responding'
              : workload.cpu_percent < 5
                ? 'Light CPU activity'
                : workload.cpu_percent < 20
                  ? 'Actively using CPU'
                  : 'Busy right now'
    );
    async function check() {
        if (busy || disabled) return;
        const id = workload.id;
        busy = true;
        error = '';
        try {
            const result: Reply = demo
                ? {
                      text: 'This is a demonstration of an activity insight.',
                      source: 'demo',
                      tier: 'demo',
                      provider: null,
                      model: null,
                      route: ['Synthetic data; no model or process action.'],
                      estimated_cost_usd: 0,
                      investigation: null,
                      choices: [],
                      insight: {
                          headline: 'Background work is using the CPU',
                          finding:
                              'This synthetic example shows several helpers working together. The computer still has available capacity.',
                          next_step:
                              'Keep it running. Recheck activity if the computer starts to feel slow.',
                      },
                  }
                : await invoke('ask', {
                      messages: [
                          {
                              role: 'user',
                              content:
                                  'Check this selected workload. Is its current resource use a problem, and what should I do now?',
                          },
                      ],
                      workloadId: id,
                      tier: 'auto',
                      pid: null,
                      startTime: null,
                  });
            if (id === context) reply = result;
            onusage();
        } catch (cause) {
            if (id === context) error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
</script>

<section class="activity-insight panel" aria-label="Workload activity insight" aria-busy={busy}>
    {#if reply}
        <InsightCard {reply} />
        {#if reply.investigation}<Investigation
                evidence={reply.investigation}
                disabled={disabled || demo || busy}
                {onaction}
            />{/if}
    {:else}
        <div class="reading">
            <span class="reading-icon"><Icon name="activity" size={20} /></span>
            <div>
                <small>Right now</small>
                <h2>{reading}</h2>
            </div>
        </div>
        <p class="intro">
            {snapshot.pressure.level === 'normal'
                ? 'A slowdown can happen even with low CPU.'
                : 'Your computer is under pressure.'}
        </p>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="insight-actions">
        <button class="primary" disabled={busy || disabled} onclick={check}
            >{busy ? 'Checking activity…' : reply ? 'Check again' : 'Investigate'}</button
        >
        <button class="text-button" disabled={busy} onclick={() => onask(reply)}
            >Ask a follow-up <span aria-hidden="true">↗</span></button
        >
    </div>
</section>

<style>
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
