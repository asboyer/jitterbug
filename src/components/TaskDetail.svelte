<script lang="ts">
    import { memory, percent } from '../lib/format';
    import WorkflowResources from './WorkflowResources.svelte';
    import HarnessBadge from './HarnessBadge.svelte';
    import TaskNarrative from './TaskNarrative.svelte';
    import type { Snapshot, Workload, AgentWorkflow } from '../lib/types';
    import type { ManagementSelection } from '../lib/manage';
    let {
        workflow,
        snapshot,
        onselect,
        onusage,
        onmanage,
    }: {
        workflow: AgentWorkflow;
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
    const status = (value: string) => value.charAt(0).toUpperCase() + value.slice(1);
</script>

<div class="workflow-body">
    <TaskNarrative {workflow} {onusage} demo={snapshot.source === 'demo'} />
    {#if workflow.observed_samples > 0}<section class="resource-detail" aria-label="Task resources">
            <h3>Where resources are going</h3>
            <WorkflowResources {workflow} interval={snapshot.interval_ms} />
        </section>{:else}<p class="no-measurements">
            No resource samples are available for this task.
        </p>{/if}
    {#if workflow.status === 'active'}<div class="workflow-actions">
            <button onclick={() => onmanage({ task_id: workflow.task_id })}
                >Set up managed checks</button
            >
            <span class="muted">Let heavy checks take turns.</span>
        </div>{/if}
    <details class="technical-detail">
        <summary>Sessions & evidence · {workflow.agents.length}</summary>
        <p class="task-id">Task ID: {workflow.task_id}</p>
        <div class="agent-list">
            {#each workflow.agents as agent (agent.run_id)}
                <details class="agent">
                    <summary>
                        <HarnessBadge harness={agent.harness} compact />
                        <span class="agent-name"
                            ><strong>{agent.agent}</strong><small
                                class:completed={agent.status === 'completed'}
                                >{status(agent.status)}</small
                            ></span
                        >
                        <span class="agent-usage">
                            {#if agent.status === 'active'}
                                {agent.observed_samples ? percent(agent.cpu_percent) : 'Sampling…'} ·
                                {memory(agent.memory_bytes)}
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
                                        : `${percent(agent.peak_cpu_percent)} sampled CPU`}
                                    ·
                                    {agent.peak_process_count
                                        ? memory(agent.peak_memory_bytes)
                                        : 'memory not sampled'} · {agent.peak_process_count}
                                    processes
                                </dd>
                            </div>
                            <div>
                                <dt>Measurement</dt>
                                <dd>
                                    {agent.observed_samples} samples{agent.exit_code !== null
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
                Task titles come from the launcher; request excerpts come from opted-in harness
                hooks or provided descriptions. Ownership requires a matching process ID, start
                time, and observed parent chain. Working directories alone are not proof.
            </p>
            <p>
                Peaks are sampled every {snapshot.interval_ms / 1000} seconds and may miss short bursts.
                Shared memory may be counted more than once. Completed measurements remain in memory for
                this app session. Suggestions do not change your agents or tests.
            </p>
        </details>
    </details>
</div>

<style>
    .no-measurements {
        color: var(--muted);
        font-size: 12px;
        margin: 24px 0;
    }
    .workflow-body {
        padding: 20px;
    }
    .workflow-actions {
        display: flex;
        gap: 12px;
        align-items: center;
        flex-wrap: wrap;
        padding: 18px 0;
    }
    .workflow-actions > span {
        font-size: 11px;
    }
    .resource-detail {
        margin-top: 20px;
        border-top: 1px solid var(--border);
        padding-top: 16px;
    }
    .technical-detail > summary {
        cursor: pointer;
        color: var(--muted);
        padding: 12px 0;
    }
    .task-id {
        font-size: 11px;
        font-weight: 650;
        color: var(--muted);
    }
    h3 {
        margin: 0;
        font-size: 14px;
        line-height: 1.4;
    }
    .agent-name small.completed {
        color: var(--success);
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
    @media (max-width: 720px) {
        .agent-usage {
            font-size: 11px;
        }
        h4 span {
            display: block;
            margin: 0;
        }
    }
</style>
