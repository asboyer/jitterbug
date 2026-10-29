<script lang="ts">
    import { invoke, isTauri } from '@tauri-apps/api/core';
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
</script>

<div class="narrative">
    <section aria-label="What this task is building">
        <h4>What this task is building <span>Intended work</span></h4>
        {#if summary}
            <p>{summary.text}</p>
            <small class="muted"
                >AI summary · {summary.provider} / {summary.model} · saved for this app session</small
            >
        {:else if narrative.assignments.length === 0}
            <p>{workflow.task_title}</p>
            <small class="muted">Launcher title only · no harness request captured</small>
        {/if}
        {#if narrative.assignments.length > 0}
            <details open={!summary} class="assignments">
                <summary>Task excerpts · kept locally</summary>
                {#each narrative.assignments as assignment, index (index)}
                    <div class="assignment">
                        <strong>{assignment.agent}</strong>
                        <p>{assignment.request}</p>
                        <small class="muted"
                            >{assignment.source === 'hook'
                                ? 'Harness request and latest follow-up'
                                : 'Provided description'} · excerpt, not an AI explanation</small
                        >
                    </div>
                {/each}
            </details>
            {#if !demo && !summary}
                <div class="summary-action">
                    <button disabled={pending || cacheLoading} onclick={summarize}>
                        {pending
                            ? 'Summarizing…'
                            : cacheLoading
                              ? 'Checking saved summary…'
                              : 'Summarize task with AI'}
                    </button>
                    <small class="muted"
                        >Sends the excerpts above to your intelligent provider, which may use cloud.
                        Cached until the request changes; budget and local resource limits apply.</small
                    >
                </div>
            {/if}
            {#if result && !summary}
                <small class="muted"
                    >The task request changed; its earlier AI summary is no longer shown.</small
                >
            {/if}
            {#if error}<p class="error" role="alert">{error}</p>{/if}
        {/if}
        <p class="activity muted">{narrative.activity}</p>
    </section>
    <section aria-label="Why this task uses resources">
        <h4>Where the resources went <span>Measured · role interpretation</span></h4>
        <p>{narrative.resources}</p>
    </section>
    <section class="tip" aria-label="Task efficiency tip">
        <h4>Efficiency tip</h4>
        <p>{narrative.tip}</p>
        <small class="muted">{narrative.tip_basis}</small>
    </section>
</div>

<style>
    .narrative {
        display: grid;
        gap: 16px;
        margin: 18px 0;
    }
    h4 {
        margin: 0 0 7px;
        font-size: 12px;
        font-weight: 650;
    }
    h4 span {
        margin-left: 6px;
        font-weight: 400;
        color: var(--muted);
        font-size: 11px;
    }
    p {
        margin: 5px 0 8px;
        line-height: 1.55;
        overflow-wrap: anywhere;
    }
    small {
        line-height: 1.5;
    }
    .assignments {
        margin-top: 10px;
    }
    .assignments > summary {
        cursor: pointer;
        color: var(--muted);
        font-size: 12px;
    }
    .assignment {
        margin: 12px 0;
        padding-left: 12px;
        border-left: 2px solid var(--border);
    }
    .assignment strong {
        font-size: 12px;
    }
    .summary-action {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 10px;
        margin: 12px 0;
    }
    .summary-action small {
        flex: 1;
        min-width: 180px;
    }
    .activity {
        font-size: 12px;
        margin-top: 12px;
    }
    .tip {
        border-radius: 8px;
        background: var(--hover);
        padding: 12px;
    }
    .error {
        color: var(--warning);
    }
</style>
