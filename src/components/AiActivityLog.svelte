<script lang="ts">
    import { invoke } from '@tauri-apps/api/core';
    import {
        activityLabel,
        activityOutcome,
        agentCallLabel,
        agentCallSource,
        ago,
    } from '../lib/ai-activity';
    import { providerNames } from '../lib/ai-mode';
    import Icon from './Icon.svelte';
    import type { AgentCall, AiActivity, AutomationCase } from '../lib/types';

    /** Read activity only when connected to the native app. */
    let { live, cases }: { live: boolean; cases: AutomationCase[] } = $props();
    let entries: AiActivity[] = $state([]);
    let calls: AgentCall[] = $state([]);
    let showAll = $state(false);
    let showAllCalls = $state(false);
    let now = $state(Date.now() / 1000);
    const shown = $derived(showAll ? entries : entries.slice(0, 5));
    const spent = $derived(entries.reduce((sum, entry) => sum + entry.cost_usd, 0));
    const failed = $derived(entries.filter((entry) => !entry.ok).length);
    const shownCalls = $derived(showAllCalls ? calls : calls.slice(0, 5));

    $effect(() => {
        const reading = live;
        let stopped = false;
        async function load() {
            now = Date.now() / 1000;
            if (!reading) {
                entries = [];
                calls = [];
                return;
            }
            try {
                const [next, asked] = await Promise.all([
                    invoke<AiActivity[]>('get_ai_activity'),
                    invoke<AgentCall[]>('get_agent_activity'),
                ]);
                if (!stopped) {
                    entries = next;
                    calls = asked;
                }
            } catch {
                /* The log is optional; Monitor still works without it. */
            }
        }
        void load();
        const timer = setInterval(load, 5000);
        return () => {
            stopped = true;
            clearInterval(timer);
        };
    });
    const seconds = (ms: number) => (ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`);
</script>

<details class="ai-activity panel">
    <summary>
        <span class="activity-title"><Icon name="model" size={16} /> AI activity</span>
        <span class="activity-stats"
            >{entries.length
                ? `${entries.length} model ${entries.length === 1 ? 'call' : 'calls'} · $${spent.toFixed(4)}${failed ? ` · ${failed} failed` : ''}`
                : 'No model calls yet'} · {calls.length
                ? `${calls.length} agent ${calls.length === 1 ? 'request' : 'requests'}`
                : 'no agent requests'}</span
        >
    </summary>
    <section aria-label="AI activity">
        {#if entries.length}
            <ul class="activity-list">
                {#each shown as entry (entry.at + entry.purpose + entry.duration_ms)}
                    {@const outcome = activityOutcome(entry, cases)}
                    <li>
                        <span class="activity-dot" class:failed={!entry.ok} aria-hidden="true"
                        ></span>
                        <div class="activity-copy">
                            <strong>{activityLabel(entry)}</strong>
                            <span
                                >{entry.ok
                                    ? `${providerNames[entry.provider]} · ${entry.model} · ${seconds(entry.duration_ms)}${entry.cost_usd ? ` · $${entry.cost_usd.toFixed(4)}` : ''}`
                                    : (entry.error ?? 'Failed')}</span
                            >
                        </div>
                        <div class="activity-meta">
                            {#if outcome}<span class="activity-outcome">{outcome}</span>{/if}
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
            <p class="muted">
                Jitterbug calls a model only when you ask, open an explanation, or a slowdown
                persists. Each call shows here with its model, time and cost.
            </p>
        {/if}
        <h3>Agents</h3>
        {#if calls.length}
            <ul class="activity-list">
                {#each shownCalls as call, index (call.at + call.method + index)}
                    <li>
                        <span class="activity-dot" class:failed={!call.ok} aria-hidden="true"
                        ></span>
                        <div class="activity-copy">
                            <strong>{agentCallLabel(call)}</strong>
                            <span>{call.ok ? agentCallSource(call) : (call.error ?? 'Failed')}</span
                            >
                        </div>
                        <div class="activity-meta">
                            <time>{ago(call.at, now)}</time>
                        </div>
                    </li>
                {/each}
            </ul>
            {#if calls.length > 5}
                <button class="text-button" onclick={() => (showAllCalls = !showAllCalls)}
                    >{showAllCalls ? 'Show less' : `Show all ${calls.length}`}</button
                >
            {/if}
        {:else}
            <p class="muted">
                Connected agents’ requests and pressure checks show here. Set them up in Settings ›
                Integrations.
            </p>
        {/if}
    </section>
</details>

<style>
    .ai-activity {
        margin-bottom: 20px;
        padding: 0;
    }
    summary {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        padding: 12px 16px;
        cursor: pointer;
        list-style: none;
    }
    summary::-webkit-details-marker {
        display: none;
    }
    summary::after {
        content: '›';
        color: var(--muted);
        transition: transform 120ms;
    }
    .ai-activity[open] summary::after {
        transform: rotate(90deg);
    }
    .activity-title {
        display: flex;
        align-items: center;
        gap: 8px;
        font-weight: 600;
        font-size: 13px;
    }
    .activity-title :global(svg) {
        color: var(--accent);
    }
    .activity-stats {
        margin-left: auto;
        color: var(--muted);
        font-size: 12px;
    }
    section {
        padding: 0 16px 12px;
    }
    h3 {
        margin: 14px 0 4px;
        font-size: 12px;
        font-weight: 600;
        color: var(--muted);
    }
    section > p {
        margin: 0 0 4px;
        font-size: 12px;
    }
    .activity-list {
        list-style: none;
        margin: 0 0 4px;
        padding: 0;
    }
    .activity-list li {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 9px 0;
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
    .activity-outcome {
        font-size: 11px;
        padding: 2px 8px;
        border-radius: 999px;
        color: var(--muted);
        background: var(--sidebar);
    }
    time {
        color: var(--muted);
        font-size: 11px;
        min-width: 64px;
        text-align: right;
    }
</style>
