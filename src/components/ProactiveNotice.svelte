<script lang="ts">
    import Icon from './Icon.svelte';
    import { incidentOutcome } from '../lib/incidents';
    import { currentTip } from '../lib/task-view';
    import type { AutomationCase, AgentWorkflow } from '../lib/types';
    let {
        incident,
        workflow,
        onreview,
        ontask,
        onlater,
    }: {
        incident: AutomationCase;
        workflow?: AgentWorkflow;
        onreview: () => void;
        ontask: () => void;
        onlater: () => void;
    } = $props();
    const checking = $derived(['detected', 'investigating'].includes(incident.stage));
    const overlap = $derived(workflow && currentTip(workflow)?.id.startsWith('overlapping-'));
    const subject = $derived(incident.trigger.target?.label ?? 'an app');
    const title = $derived.by(() => {
        switch (incident.trigger.kind) {
            case 'system_cpu':
                return 'Hey, your computer has stayed busy.';
            case 'memory_pressure':
                return 'Hey, your computer is short on memory.';
            case 'thermal_pressure':
                return 'Hey, your computer is running warm.';
            case 'memory_growth':
                return `Hey, ${subject}’s memory use has grown.`;
            case 'cpu_anomaly':
                return `Hey, ${subject} is busier than usual.`;
            default:
                return `Hey, ${subject} has stayed busy.`;
        }
    });
    const body = $derived(
        checking
            ? 'I’m checking recent activity and looking for a useful next step.'
            : overlap && incident.stage === 'awaiting_approval'
              ? 'Your coding sessions have overlapping checks. A shared queue could help future runs take turns.'
              : incident.stage === 'awaiting_approval'
                ? 'I found a priority change you can review to give other work more room.'
                : incidentOutcome(incident)
    );
</script>

<section class="proactive-notice" aria-label="Jitterbug noticed something" aria-live="polite">
    <div class="notice-top">
        <span class="bug-mark" class:checking><Icon name="bug" size={21} /></span><span
            class="notice-name"
            >Jitterbug <small>{incident.source === 'demo' ? 'EXAMPLE' : 'LIVE'}</small></span
        ><button class="icon-button" aria-label="Dismiss assistant prompt" onclick={onlater}
            >×</button
        >
    </div>
    <h2>{title}</h2>
    <p>{body}</p>
    {#if overlap && workflow && !checking}<div class="task-context">
            <Icon name="code" size={14} /><span>{workflow.task_title}</span>
        </div>{/if}
    <div class="notice-actions">
        {#if overlap && !checking && incident.stage === 'awaiting_approval'}<button
                class="primary"
                onclick={ontask}>See task</button
            ><button class="text-button" onclick={onreview}>Review options</button>
        {:else}<button class="primary" onclick={onreview}
                >{checking
                    ? 'See what I’m checking'
                    : incident.stage === 'awaiting_approval'
                      ? 'Review suggestion'
                      : 'See what happened'}</button
            >{/if}
        <button class="text-button later" onclick={onlater}>Later</button>
    </div>
</section>

<style>
    .proactive-notice {
        position: fixed;
        right: 22px;
        bottom: 22px;
        width: min(390px, calc(100vw - 40px));
        padding: 20px;
        z-index: 18;
        background: var(--panel);
        border: 1px solid var(--border);
        border-radius: 18px;
        box-shadow: 0 12px 50px #0004;
        animation: arrive 200ms ease-out;
    }
    .notice-top {
        display: flex;
        align-items: center;
        gap: 10px;
    }
    .bug-mark {
        width: 36px;
        height: 36px;
        display: grid;
        place-items: center;
        border-radius: 12px;
        background: var(--accent-soft);
        color: var(--accent);
    }
    .checking {
        animation: breathe 1.8s ease-in-out infinite;
    }
    .notice-name {
        flex: 1;
        font-size: 12px;
        font-weight: 600;
    }
    .notice-name small {
        margin-left: 8px;
        font-size: 8px;
        letter-spacing: 0.8px;
        color: var(--muted);
    }
    h2 {
        font-size: 18px;
        line-height: 1.35;
        margin: 15px 0 9px;
    }
    p {
        margin: 0;
        font-size: 13px;
        line-height: 1.6;
        color: var(--muted);
    }
    .task-context {
        display: flex;
        align-items: center;
        gap: 8px;
        background: var(--hover);
        margin-top: 14px;
        padding: 9px;
        border-radius: 7px;
        font-size: 11px;
    }
    .task-context span {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .notice-actions {
        display: flex;
        gap: 12px;
        flex-wrap: wrap;
        align-items: center;
        margin-top: 18px;
    }
    .later {
        margin-left: auto;
    }
    @keyframes arrive {
        from {
            opacity: 0;
            transform: translateY(10px);
        }
        to {
            opacity: 1;
            transform: translateY(0);
        }
    }
    @keyframes breathe {
        50% {
            opacity: 0.5;
        }
    }
    @media (prefers-reduced-motion: reduce) {
        .proactive-notice,
        .checking {
            animation: none;
        }
    }
</style>
