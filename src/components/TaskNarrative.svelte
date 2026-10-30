<script lang="ts">
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import Icon from './Icon.svelte';
    import { errorMessage } from '../lib/format';
    import type { AgentWorkflow, AppProfile } from '../lib/types';

    let {
        workflow,
        demo,
        onusage,
    }: {
        workflow: AgentWorkflow;
        demo: boolean;
        onusage: () => void;
    } = $props();
    let result: { key: string; profile: AppProfile } | null = $state(null);
    let pending = $state(false);
    let cacheLoading = $state(false);
    let error = $state('');
    const narrative = $derived(workflow.narrative);
    const assignments = $derived([
        ...new Map(narrative.assignments.map((item) => [item.request, item])).values(),
    ]);
    const taskKey = $derived(workflow.narrative.task_key);
    const taskId = $derived(workflow.task_id);
    const summary = $derived.by(() => (result?.key === narrative.task_key ? result.profile : null));

    $effect(() => {
        const key = taskKey;
        const id = taskId;
        if (demo || !isTauri()) return;
        let disposed = false;
        cacheLoading = true;
        void invoke<AppProfile | null>('get_agent_task_summary', { taskId: id, taskKey: key })
            .then((profile) => {
                if (!disposed && profile) result = { key, profile };
            })
            .catch(() => {})
            .finally(() => {
                if (!disposed) cacheLoading = false;
            });
        return () => {
            disposed = true;
        };
    });

    async function summarize() {
        if (pending || demo || !isTauri()) return;
        pending = true;
        error = '';
        const key = narrative.task_key;
        try {
            const profile = await invoke<AppProfile>('summarize_agent_task', {
                taskId: workflow.task_id,
                taskKey: key,
            });
            result = { key, profile };
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            pending = false;
            onusage();
        }
    }
    const activity = $derived.by(() => {
        const totals: Record<string, number> = {};
        for (const agent of workflow.agents) {
            for (const [role, count] of Object.entries(agent.context.completed_tools)) {
                totals[role] = (totals[role] ?? 0) + count;
            }
        }
        const names: Record<string, string> = {
            read: 'reads / searches',
            edit: 'edits',
            command: 'commands',
            other: 'other calls',
        };
        return Object.entries(totals).map(([role, count]) => ({
            label: names[role] ?? role,
            count,
        }));
    });
    const recommendation = $derived.by(() => {
        const priority = (id: string) =>
            id === 'memory-headroom'
                ? 4
                : id.startsWith('overlapping-')
                  ? 3
                  : id === 'worker-amplification'
                    ? 2
                    : 1;
        return [...workflow.suggestions].sort(
            (a, b) => Number(b.current) - Number(a.current) || priority(b.id) - priority(a.id)
        )[0];
    });
</script>

<div class="narrative">
    <details class="intent">
        <summary>
            <span class="intent-icon"><Icon name="code" size={16} /></span>
            <span class="intent-copy">
                <small
                    >{summary
                        ? 'AI summary · intended work'
                        : narrative.assignments.length
                          ? 'Task request · excerpt'
                          : 'Task context'}</small
                >
                <span class="preview"
                    >{summary?.text ??
                        narrative.assignments[0]?.request ??
                        'No task request captured for this launch.'}</span
                >
            </span>
            <span class="chevron" aria-hidden="true">›</span>
        </summary>
        <div class="intent-body">
            {#if summary}
                <p>{summary.text}</p>
                <small class="muted"
                    >AI · {summary.provider} / {summary.model} · cached for this app session. Describes
                    intended work, not completion.</small
                >
            {/if}
            {#each assignments as assignment, index (index)}
                <div class="assignment">
                    <strong>{assignment.agent}</strong>
                    <p>{assignment.request}</p>
                    <small class="muted"
                        >{assignment.source === 'hook'
                            ? 'Harness request and latest follow-up'
                            : 'Provided description'} · kept locally</small
                    >
                </div>
            {/each}
            {#if narrative.assignments.length > 0 && !demo && !summary}
                <div class="summary-action">
                    <p class="muted">
                        Summarizing sends the excerpts above to your intelligent provider, which may
                        use cloud. Budget and resource limits apply. The result is cached until the
                        request changes.
                    </p>
                    <button disabled={pending || cacheLoading} onclick={summarize}>
                        {pending
                            ? 'Summarizing…'
                            : cacheLoading
                              ? 'Checking cache…'
                              : 'Summarize with AI'}
                    </button>
                </div>
            {/if}
            {#if demo}<small class="muted">Synthetic example · no model called.</small>{/if}
            {#if result && !summary}<p class="muted">
                    The request changed; its earlier AI summary is no longer shown.
                </p>{/if}
            {#if error}<p class="error" role="alert">{error}</p>{/if}
            {#if activity.length}
                <div class="activity" aria-label="Reported harness tool events">
                    <span class="muted">Reported</span>
                    {#each activity as item}<span><strong>{item.count}</strong> {item.label}</span
                        >{/each}
                </div>
            {/if}
            <p class="muted">{narrative.activity}</p>
        </div>
    </details>
    {#if recommendation}
        <details class="tip">
            <summary>
                <span class="tip-icon"><Icon name="bulb" size={17} /></span>
                <span class="tip-copy"
                    ><small
                        >Suggestion · {recommendation.current
                            ? 'observed now'
                            : 'observed earlier'}</small
                    ><strong>{recommendation.title}</strong></span
                >
                <span class="chevron" aria-hidden="true">›</span>
            </summary>
            <div class="tip-body">
                <p>{recommendation.recommendation}</p>
                <ul>
                    {#each recommendation.evidence as evidence}<li>{evidence}</li>{/each}
                </ul>
                <p class="muted">{recommendation.uncertainty}</p>
                <small class="muted">Local rules · suggestion only · no action applied</small>
            </div>
        </details>
    {:else if workflow.status === 'active' && workflow.observed_samples > 0}
        <div class="no-tip">
            <Icon name="bulb" size={14} /><span
                >{workflow.observed_samples
                    ? 'No optimization suggested by these samples'
                    : 'Measure a running task to get suggestions'}</span
            >
        </div>
    {/if}
</div>

<style>
    .narrative {
        margin-top: 12px;
    }
    summary {
        display: flex;
        align-items: center;
        gap: 10px;
        cursor: pointer;
        list-style: none;
    }
    summary::-webkit-details-marker {
        display: none;
    }
    .intent > summary {
        padding: 10px 0;
    }
    .intent-icon,
    .tip-icon {
        color: var(--muted);
        flex-shrink: 0;
        display: flex;
    }
    .intent-copy,
    .tip-copy {
        min-width: 0;
        flex: 1;
        display: grid;
        gap: 4px;
    }
    small {
        color: var(--muted);
        font-size: 10px;
        line-height: 1.5;
    }
    .preview {
        color: var(--text);
        display: -webkit-box;
        -webkit-box-orient: vertical;
        -webkit-line-clamp: 2;
        line-clamp: 2;
        overflow: hidden;
        line-height: 1.5;
        font-size: 12px;
        overflow-wrap: anywhere;
    }
    .chevron {
        color: var(--muted);
        font-size: 18px;
    }
    details[open] > summary .chevron {
        transform: rotate(90deg);
    }
    .intent-body {
        margin-left: 26px;
        padding-bottom: 8px;
    }
    p {
        line-height: 1.6;
        overflow-wrap: anywhere;
        margin: 6px 0 10px;
        font-size: 12px;
    }
    .assignment {
        border-left: 2px solid var(--border);
        margin: 12px 0;
        padding-left: 10px;
    }
    .assignment strong {
        font-size: 11px;
    }
    .summary-action {
        margin: 12px 0;
    }
    .activity {
        display: flex;
        flex-wrap: wrap;
        gap: 6px 14px;
        margin: 0 0 12px 26px;
        font-size: 10px;
        color: var(--muted);
    }
    .activity strong {
        color: var(--text);
        font-weight: 550;
        font-variant-numeric: tabular-nums;
    }
    .tip {
        border-radius: 6px;
        background: var(--hover);
        margin-top: 8px;
    }
    .tip > summary {
        padding: 10px 12px;
    }
    .tip-copy strong {
        font-size: 12px;
        font-weight: 550;
    }
    .tip-body {
        padding: 0 12px 12px 39px;
    }
    ul {
        padding-left: 16px;
        font-size: 11px;
        line-height: 1.6;
    }
    .no-tip {
        color: var(--muted);
        display: flex;
        align-items: center;
        gap: 7px;
        font-size: 11px;
        margin: 10px 0 4px;
    }
    .error {
        color: var(--warning);
    }
</style>
