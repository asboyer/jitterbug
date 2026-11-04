<script lang="ts">
    import { onMount, untrack } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { activityLabel, activityOutcome, ago, demoActivity } from '../lib/ai-activity';
    import { providerNames } from '../lib/ai-mode';
    import type { AiActivity, AutomationCase } from '../lib/types';

    let { desktop, cases }: { desktop: boolean; cases: AutomationCase[] } = $props();
    // `desktop` never changes while the app runs.
    let entries: AiActivity[] = $state(untrack(() => (desktop ? [] : demoActivity())));
    let showAll = $state(false);
    let now = $state(Date.now() / 1000);
    const shown = $derived(showAll ? entries : entries.slice(0, 5));
    const spent = $derived(entries.reduce((sum, entry) => sum + entry.cost_usd, 0));
    const failed = $derived(entries.filter((entry) => !entry.ok).length);

    onMount(() => {
        async function load() {
            now = Date.now() / 1000;
            if (!desktop) return;
            try {
                entries = await invoke<AiActivity[]>('get_ai_activity');
            } catch {
                /* The log is optional; the rest of the page still works. */
            }
        }
        void load();
        const timer = setInterval(load, 5000);
        return () => clearInterval(timer);
    });
    const seconds = (ms: number) => (ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`);
</script>

<section class="settings-group ai-activity" aria-labelledby="ai-activity">
    <h2 id="ai-activity">Recent activity</h2>
    {#if entries.length}
        <p class="group-note">
            {entries.length}
            {entries.length === 1 ? 'call' : 'calls'} this session · ${spent.toFixed(4)}{failed
                ? ` · ${failed} failed`
                : ''}
        </p>
        <ul class="activity-list">
            {#each shown as entry (entry.at + entry.purpose + entry.duration_ms)}
                {@const outcome = activityOutcome(entry, cases)}
                <li>
                    <span class="activity-dot" class:failed={!entry.ok} aria-hidden="true"></span>
                    <div class="activity-copy">
                        <strong>{activityLabel(entry)}</strong>
                        <span
                            >{entry.ok
                                ? `${providerNames[entry.provider]} · ${entry.model} · ${seconds(entry.duration_ms)}${entry.cost_usd ? ` · $${entry.cost_usd.toFixed(4)}` : ''}`
                                : (entry.error ?? 'Failed')}</span
                        >
                    </div>
                    <div class="activity-meta">
                        {#if outcome}<span class="integration-status">{outcome}</span>{/if}
                        <time>{ago(entry.at, now)}</time>
                    </div>
                </li>
            {/each}
        </ul>
        {#if entries.length > 5}
            <button class="text-button" onclick={() => (showAll = !showAll)}
                >{showAll ? 'Show less' : `Show all ${entries.length}`}</button
            >
        {/if}
    {:else}
        <p class="group-note">
            No AI calls yet this session. Jitterbug calls a model only when you ask, open an
            explanation, or a slowdown persists.
        </p>
    {/if}
</section>

<style>
    .activity-list {
        list-style: none;
        margin: 6px 0 4px;
        padding: 0;
    }
    .activity-list li {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 10px 0;
        border-top: 1px solid var(--border);
    }
    .activity-dot {
        width: 7px;
        height: 7px;
        flex: 0 0 7px;
        border-radius: 50%;
        background: var(--success);
    }
    .activity-dot.failed {
        background: var(--warning);
    }
    .activity-copy {
        display: grid;
        gap: 3px;
        min-width: 0;
        flex: 1;
    }
    .activity-copy strong {
        font-size: 13px;
        font-weight: 550;
    }
    .activity-copy span {
        color: var(--muted);
        font-size: 12px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .activity-meta {
        display: flex;
        align-items: center;
        gap: 10px;
        flex-shrink: 0;
    }
    time {
        color: var(--muted);
        font-size: 11px;
        min-width: 64px;
        text-align: right;
    }
</style>
