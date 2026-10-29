<script lang="ts">
    import { memory, percent } from '../lib/format';
    import WorkflowResources from './WorkflowResources.svelte';
    import TaskNarrative from './TaskNarrative.svelte';
    import type { Snapshot, Workload } from '../lib/types';
    import type { ManagementSelection } from '../lib/manage';

    let {
        snapshot,
        onselect,
        onusage,
        onmanage,
    }: {
        snapshot: Snapshot;
        onselect: (workload: Workload) => void;
        onusage: () => void;
        onmanage: (selection: ManagementSelection) => void;
    } = $props();

    function time(value: number): string {
        return new Date(value * 1000).toLocaleTimeString([], {
            hour: 'numeric',
            minute: '2-digit',
        });
    }

    function status(value: string): string {
        return value.charAt(0).toUpperCase() + value.slice(1);
    }
</script>

<section class="panel workflow-panel" aria-label="Agent workflows">
    <div class="section-title">
        <div>
            <h2>Agent tasks <span class="task-count">{snapshot.agent_workflows.length}</span></h2>
        </div>
        <span class="muted"
            >{snapshot.source === 'demo' ? 'Synthetic example' : 'On this device'}</span
        >
    </div>
    {#each snapshot.agent_workflows as workflow, index (workflow.task_id)}
        <details class="workflow" open={workflow.status === 'active' || index === 0}>
            <summary class="workflow-heading">
                <div>
                    <span class="task-id">{workflow.task_id}</span>
                    <h3>{workflow.task_title}</h3>
                </div>
                <span class:active={workflow.status === 'active'} class="run-status">
                    {status(workflow.status)}
                </span>
                <span class="workflow-chevron" aria-hidden="true">›</span>
            </summary>
            <p class="workflow-summary">
                {#if workflow.status === 'active'}
                    {workflow.active_agents} active
                    {workflow.active_agents === 1 ? 'session' : 'sessions'} ·
                    {workflow.process_count} attributed processes
                {:else}
                    {workflow.agents.length} registered {workflow.agents.length === 1
                        ? 'session'
                        : 'sessions'} ·
                    {workflow.observed_samples > 0
                        ? `last observed ${time(workflow.last_seen_at)}`
                        : 'no resource samples in this app session'}
                {/if}
            </p>
            <WorkflowResources {workflow} interval={snapshot.interval_ms} />
            <TaskNarrative {workflow} {onusage} demo={snapshot.source === 'demo'} />
            <button
                disabled={snapshot.source === 'demo'}
                onclick={() => onmanage({ task_id: workflow.task_id })}>Manage this workload</button
            >
            <details class="technical-detail">
                <summary>Sessions & evidence · {workflow.agents.length}</summary>
                <div class="agent-list">
                    {#each workflow.agents as agent (agent.run_id)}
                        <details class="agent">
                            <summary>
                                <span class="agent-name"
                                    ><strong>{agent.agent}</strong><small
                                        >{agent.harness} · {status(agent.status)}</small
                                    ></span
                                >
                                <span class="agent-usage">
                                    {#if agent.status === 'active'}
                                        {agent.observed_samples
                                            ? percent(agent.cpu_percent)
                                            : 'Sampling…'} · {memory(agent.memory_bytes)}
                                    {:else}
                                        {agent.peak_process_count === 0
                                            ? 'Not sampled'
                                            : `Peak ${memory(agent.peak_memory_bytes)}`}
                                    {/if}
                                </span>
                            </summary>
                            <div class="agent-detail">
                                {#if agent.context.request}
                                    <p><strong>Assignment:</strong> {agent.context.request}</p>
                                    <p class="muted">
                                        {agent.context.source === 'hook'
                                            ? 'Harness request excerpt'
                                            : 'Provided task description'} · saved {time(
                                            agent.context.updated_at
                                        )}
                                    </p>
                                {/if}
                                {#if agent.context.latest_request}
                                    <p>
                                        <strong>Latest follow-up:</strong>
                                        {agent.context.latest_request}
                                    </p>
                                {/if}
                                {#if agent.context.last_activity_at}
                                    <p class="muted">
                                        Last reported activity: {agent.context.last_activity} · {time(
                                            agent.context.last_activity_at
                                        )}. This may not reflect work still running.
                                    </p>
                                {/if}
                                <dl>
                                    <div>
                                        <dt>Worktree</dt>
                                        <dd class="path">{agent.worktree}</dd>
                                    </div>
                                    <div>
                                        <dt>Launch identity</dt>
                                        <dd>
                                            Wrapper PID {agent.root_pid} · started {time(
                                                agent.root_start_time
                                            )}
                                        </dd>
                                    </div>
                                    <div>
                                        <dt>Observed peak</dt>
                                        <dd>
                                            {agent.observed_samples === 0
                                                ? 'Not sampled'
                                                : `${percent(agent.peak_cpu_percent)} sampled CPU`} ·
                                            {agent.peak_process_count
                                                ? memory(agent.peak_memory_bytes)
                                                : 'memory not sampled'} · {agent.peak_process_count} processes
                                        </dd>
                                    </div>
                                    <div>
                                        <dt>Measurement</dt>
                                        <dd>
                                            {agent.observed_samples} samples{agent.exit_code !==
                                            null
                                                ? ` · exit ${agent.exit_code}`
                                                : ''}
                                        </dd>
                                    </div>
                                </dl>
                                {#if agent.status === 'active' && agent.roles.length > 0}
                                    <div class="roles" aria-label="Observed process roles">
                                        {#each agent.roles as role}
                                            <span
                                                >{role.role} × {role.process_count} · {memory(
                                                    role.memory_bytes
                                                )}</span
                                            >
                                        {/each}
                                    </div>
                                {/if}
                                {#if agent.process_ids.length > 0}
                                    <p class="muted">
                                        Observed PIDs: {agent.process_ids.join(', ')}
                                    </p>
                                {/if}
                                <div class="workload-links">
                                    {#each snapshot.workloads.filter( (workload) => agent.workload_ids.includes(workload.id) ) as workload (workload.id)}
                                        <button onclick={() => onselect(workload)}
                                            >Inspect {workload.label} ↗</button
                                        >
                                    {/each}
                                </div>
                            </div>
                        </details>
                    {/each}
                </div>
                {#if workflow.suggestions.length > 0}
                    <div class="workflow-advice">
                        <h4>Ways to reduce resource use <span>Rule-based suggestions</span></h4>
                        {#each workflow.suggestions as suggestion (suggestion.id)}
                            <details class="suggestion">
                                <summary
                                    ><strong>{suggestion.title}</strong><small
                                        >{suggestion.current
                                            ? 'Observed now'
                                            : `Observed at ${time(suggestion.observed_at)}`}</small
                                    ></summary
                                >
                                <p>{suggestion.recommendation}</p>
                                <ul>
                                    {#each suggestion.evidence as evidence}<li>
                                            {evidence}
                                        </li>{/each}
                                </ul>
                                <p class="muted">{suggestion.uncertainty}</p>
                            </details>
                        {/each}
                    </div>
                {/if}
                <details class="workflow-evidence">
                    <summary>Attribution & measurement limits</summary>
                    <ul>
                        {#each workflow.evidence as evidence}<li>{evidence}</li>{/each}
                    </ul>
                    <p>
                        Task titles come from the launcher; request excerpts come from opted-in
                        harness hooks or provided descriptions. Ownership requires a matching
                        process ID, start time, and observed parent chain. Working directories alone
                        are not proof.
                    </p>
                    <p>
                        Peaks are sampled every {snapshot.interval_ms / 1000} seconds and may miss short
                        bursts. Shared memory may be counted more than once. Completed measurements remain
                        in memory for this app session. Suggestions do not change your agents or tests.
                    </p>
                </details>
            </details>
        </details>
    {:else}
        <p class="workflow-empty">
            Launch with <code>jitterbug-run</code> to see tasks, agents, and tests together.
        </p>
    {/each}
</section>

<style>
    .technical-detail {
        margin: 12px 0 0;
    }
    .technical-detail > summary {
        cursor: pointer;
        color: var(--muted);
        font-size: 11px;
    }
    .technical-detail[open] > summary {
        margin-bottom: 12px;
    }
    .workflow-panel {
        margin-bottom: 20px;
    }
    .workflow-panel > .section-title {
        padding: 12px 16px;
        margin-bottom: 0;
        flex-wrap: wrap;
        gap: 8px;
    }
    .workflow {
        padding: 16px;
        border-top: 1px solid var(--border);
    }
    .workflow-heading {
        color: var(--text);
        display: flex;
        align-items: center;
        padding: 0;
        list-style: none;
        cursor: pointer;
        justify-content: space-between;
        gap: 14px;
    }
    .workflow-heading::-webkit-details-marker {
        display: none;
    }
    .workflow-heading > div {
        flex: 1;
    }
    .workflow-chevron {
        font-size: 18px;
        color: var(--muted);
    }
    .workflow[open] > .workflow-heading .workflow-chevron {
        transform: rotate(90deg);
    }
    .task-count {
        font-size: 10px;
        font-weight: 500;
        color: var(--muted);
        margin-left: 6px;
    }
    .task-id {
        font-size: 11px;
        font-weight: 650;
        color: var(--muted);
    }
    h3 {
        margin: 5px 0 0;
        font-size: 14px;
        line-height: 1.4;
    }
    .run-status {
        border-radius: 5px;
        background: var(--hover);
        padding: 3px 7px;
        font-size: 11px;
        white-space: nowrap;
    }
    .run-status.active {
        background: var(--success-soft);
        color: var(--success);
    }
    .workflow-summary {
        color: var(--muted);
        margin: 6px 0 0;
        font-size: 11px;
    }
    .agent-list {
        border: 1px solid var(--border);
        border-radius: 8px;
        overflow: hidden;
    }
    .agent + .agent {
        border-top: 1px solid var(--border);
    }
    .agent > summary {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 12px;
        cursor: pointer;
    }
    .agent > summary::before {
        content: '›';
        color: var(--muted);
    }
    .agent[open] > summary::before {
        transform: rotate(90deg);
    }
    .agent-name {
        display: flex;
        flex: 1;
        flex-direction: column;
        gap: 3px;
    }
    .agent-name small,
    .agent-usage {
        color: var(--muted);
        font-variant-numeric: tabular-nums;
    }
    .agent-detail {
        padding: 0 14px 14px 28px;
    }
    dl {
        margin: 0;
    }
    dl > div {
        display: grid;
        grid-template-columns: 110px 1fr;
        gap: 12px;
        margin: 8px 0;
    }
    dt {
        color: var(--muted);
    }
    dd {
        margin: 0;
        min-width: 0;
    }
    .path {
        overflow-wrap: anywhere;
    }
    .roles,
    .workload-links {
        display: flex;
        gap: 6px;
        flex-wrap: wrap;
        margin-top: 10px;
    }
    .roles span {
        font-size: 11px;
        background: var(--hover);
        border-radius: 5px;
        padding: 4px 7px;
    }
    .workflow-advice {
        margin-top: 18px;
    }
    h4 {
        font-size: 12px;
        margin: 0 0 6px;
    }
    h4 span {
        margin-left: 8px;
        color: var(--muted);
        font-size: 11px;
        font-weight: 400;
    }
    .suggestion {
        padding: 10px 0;
        border-bottom: 1px solid var(--border);
    }
    .suggestion summary {
        cursor: pointer;
    }
    .suggestion summary small {
        color: var(--muted);
        margin-left: 8px;
    }
    .suggestion p,
    .suggestion ul,
    .workflow-evidence p,
    .workflow-evidence ul {
        line-height: 1.6;
        margin: 8px 0;
    }
    .suggestion ul,
    .workflow-evidence ul {
        padding-left: 18px;
    }
    .workflow-evidence {
        margin-top: 15px;
        color: var(--muted);
        font-size: 11px;
    }
    .workflow-evidence summary {
        cursor: pointer;
    }
    .workflow-empty {
        margin: 0;
        padding: 6px 18px 18px;
        line-height: 1.6;
        color: var(--muted);
    }
    @media (max-width: 720px) {
        .agent-usage {
            font-size: 11px;
        }
        h4 span {
            display: block;
            margin: 5px 0 0;
        }
    }
</style>
