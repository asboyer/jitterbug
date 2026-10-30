<script lang="ts">
    import { tick } from 'svelte';
    import { memory, percent } from '../lib/format';
    import HarnessBadge from './HarnessBadge.svelte';
    import TaskDetail from './TaskDetail.svelte';
    import QueuePreview from './QueuePreview.svelte';
    import Icon from './Icon.svelte';
    import type { Snapshot, Workload, AgentWorkflow } from '../lib/types';
    import type { ManagementSelection } from '../lib/manage';

    let {
        snapshot,
        onselect,
        onusage,
        onmanage,
        selectedId = $bindable(null),
    }: {
        snapshot: Snapshot;
        selectedId?: string | null;
        onselect: (workload: Workload) => void;
        onusage: () => void;
        onmanage: (selection: ManagementSelection) => void;
    } = $props();
    let preview = $state<AgentWorkflow | null>(null);
    let opener: HTMLElement | null = null;
    const active = $derived(snapshot.agent_workflows.filter((task) => task.status === 'active'));
    const recent = $derived(snapshot.agent_workflows.filter((task) => task.status !== 'active'));
    const status = (value: string) => value.charAt(0).toUpperCase() + value.slice(1);
    $effect(() => {
        const id = selectedId;
        if (id)
            void tick().then(() => {
                const summary = [
                    ...document.querySelectorAll<HTMLElement>('[data-task-view]'),
                ].find((element) => element.dataset.taskView === id);
                summary?.focus({ preventScroll: true });
                summary?.scrollIntoView({ block: 'nearest' });
            });
    });
    function manage(workflow: AgentWorkflow) {
        opener = document.activeElement as HTMLElement;
        if (snapshot.source === 'demo') preview = workflow;
        else onmanage({ task_id: workflow.task_id });
    }
</script>

<section aria-label="Agent workflows" data-tour="agent-tasks">
    {#each [{ label: 'Working now', tasks: active }, { label: 'Recent activity', tasks: recent }] as group}
        {#if group.tasks.length}
            <div class="group-heading">
                <h2>{group.label}</h2>
                <span>{group.tasks.length}</span>
            </div>
            <div class="task-cards">
                {#each group.tasks as workflow (workflow.task_id)}
                    {@const running = workflow.status === 'active'}
                    <article class="panel task-card" aria-label={workflow.task_title}>
                        <details class="task-disclosure" open={selectedId === workflow.task_id}>
                            <summary
                                data-task-view={workflow.task_id}
                                onclick={(event) => {
                                    // Commit disclosure state synchronously, before telemetry can refresh.
                                    event.preventDefault();
                                    selectedId =
                                        selectedId === workflow.task_id ? null : workflow.task_id;
                                }}
                            >
                                <div class="task-copy">
                                    <h3>{workflow.task_title}</h3>
                                    <div class="task-meta">
                                        <div class="harnesses">
                                            {#each [...new Set(workflow.agents.map((agent) => agent.harness))] as harness}
                                                <HarnessBadge {harness} />
                                            {/each}
                                        </div>
                                        <span
                                            >{workflow.agents.length}
                                            {workflow.agents.length === 1
                                                ? 'session'
                                                : 'sessions'}</span
                                        >
                                    </div>
                                </div>
                                <div
                                    class="row-readings"
                                    aria-label={running ? 'Current resources' : 'Sampled peaks'}
                                >
                                    {#if workflow.observed_samples}
                                        <span
                                            ><strong
                                                >{percent(
                                                    running
                                                        ? workflow.cpu_percent
                                                        : workflow.peak_cpu_percent
                                                )}</strong
                                            ><small>{running ? 'CPU' : 'peak CPU'}</small></span
                                        >
                                        <span
                                            ><strong
                                                >{memory(
                                                    running
                                                        ? workflow.memory_bytes
                                                        : workflow.peak_memory_bytes
                                                )}</strong
                                            ><small>{running ? 'memory' : 'peak memory'}</small
                                            ></span
                                        >
                                    {:else}<small
                                            >{running
                                                ? 'Sampling…'
                                                : 'No resource samples available'}</small
                                        >{/if}
                                </div>
                                <span
                                    class="run-status"
                                    class:active={running}
                                    class:completed={workflow.status === 'completed'}
                                    class:failed={['failed', 'interrupted'].includes(
                                        workflow.status
                                    )}>{status(workflow.status)}</span
                                >
                                <span class="chevron" aria-hidden="true">›</span>
                            </summary>
                            {#if selectedId === workflow.task_id}
                                <section class="expanded-task" aria-label="Task detail">
                                    <TaskDetail
                                        {workflow}
                                        {snapshot}
                                        {onselect}
                                        {onusage}
                                        onmanage={() => manage(workflow)}
                                    />
                                </section>
                            {/if}
                        </details>
                    </article>
                {/each}
            </div>
        {/if}
    {/each}
    {#if !snapshot.agent_workflows.length}
        <div class="panel empty-tasks">
            <Icon name="code" size={28} />
            <h2>See the work behind your coding tools</h2>
            <p>
                Connect a harness to see each task, what uses resources, and which checks could take
                turns. Your other apps are already in Workloads.
            </p>
            <button onclick={() => onmanage({})}>Connect a workload</button>
        </div>
    {/if}
</section>
{#if preview}<QueuePreview
        workflow={preview}
        onclose={() => {
            preview = null;
            void tick().then(() => {
                if (opener?.isConnected) opener.focus();
            });
        }}
    />{/if}

<style>
    .group-heading {
        display: flex;
        gap: 10px;
        align-items: center;
        margin: 6px 0 12px;
    }
    .group-heading h2 {
        font-size: 13px;
        margin: 0;
    }
    .group-heading > span {
        font-size: 11px;
        color: var(--muted);
    }
    .task-cards {
        display: grid;
        gap: 10px;
        margin-bottom: 26px;
    }
    .task-card {
        padding: 0;
        overflow: hidden;
    }
    .task-disclosure > summary {
        display: flex;
        align-items: center;
        gap: 18px;
        padding: 18px 20px;
        list-style: none;
        cursor: pointer;
    }
    .task-disclosure > summary::-webkit-details-marker {
        display: none;
    }
    .task-disclosure > summary:hover {
        background: var(--hover);
    }
    .task-disclosure > summary:focus-visible {
        outline: 2px solid var(--accent);
        outline-offset: -3px;
        border-radius: 10px;
    }
    .task-copy {
        flex: 1;
        min-width: 0;
    }
    h3 {
        font-size: 14px;
        line-height: 1.4;
        margin: 0 0 8px;
        font-weight: 550;
    }
    .task-meta,
    .harnesses {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 8px 12px;
    }
    .task-meta {
        font-size: 10px;
        color: var(--muted);
    }
    .row-readings {
        display: flex;
        gap: 22px;
        text-align: right;
    }
    .row-readings > span {
        display: grid;
        gap: 4px;
    }
    .row-readings strong {
        font-size: 13px;
        font-weight: 550;
        font-variant-numeric: tabular-nums;
        white-space: nowrap;
    }
    .row-readings small {
        font-size: 10px;
        color: var(--muted);
    }
    .run-status {
        font-size: 10px;
        padding: 4px 7px;
        border-radius: 5px;
        background: var(--hover);
        color: var(--muted);
    }
    .run-status.active {
        color: var(--accent);
        background: var(--accent-soft);
    }
    .run-status.completed {
        color: var(--success);
        background: var(--success-soft);
    }
    .run-status.failed {
        color: var(--warning);
        background: var(--warning-soft);
    }
    .chevron {
        color: var(--muted);
        font-size: 20px;
        transition: transform 120ms;
    }
    .task-disclosure[open] > summary .chevron {
        transform: rotate(90deg);
    }
    .expanded-task {
        border-top: 1px solid var(--border);
    }
    .empty-tasks {
        padding: 28px;
    }
    .empty-tasks h2 {
        font-size: 18px;
    }
    .empty-tasks p {
        max-width: 440px;
        color: var(--muted);
        line-height: 1.6;
        margin-bottom: 20px;
    }
    @media (max-width: 900px) {
        .task-disclosure > summary {
            gap: 12px;
            flex-wrap: wrap;
            padding: 16px;
        }
        .task-copy {
            flex-basis: calc(100% - 115px);
        }
        .row-readings {
            order: 1;
            width: 100%;
            text-align: left;
            padding-left: 0;
        }
        .row-readings > span {
            display: flex;
            align-items: baseline;
            gap: 5px;
        }
    }
</style>
