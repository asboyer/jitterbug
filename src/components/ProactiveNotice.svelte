<script lang="ts">
    import Icon from './Icon.svelte';
    import Logo from './Logo.svelte';
    import { incidentOutcome, noticeTitle } from '../lib/incidents';
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
    // Prefer the AI investigation's own words once it has them.
    const insight = $derived(incident.diagnosis?.insight ?? null);
    const title = $derived(
        !checking && insight?.headline
            ? insight.headline
            : noticeTitle(incident).replace(/^Hey, (\w)/, (_, c: string) => c.toUpperCase())
    );
    const body = $derived(
        checking && incident.jitter
            ? `Captured, analyzing… ${incident.jitter.happened}`
            : checking
              ? 'I’m checking recent activity and looking for a useful next step.'
              : overlap && incident.stage === 'awaiting_approval'
                ? 'Your coding sessions have overlapping checks. A shared queue could help future runs take turns.'
                : incident.stage === 'awaiting_approval'
                  ? (insight?.finding ??
                    'I found a priority change you can review to give other work more room.')
                  : (incident.diagnosis?.insight?.next_step ?? incidentOutcome(incident))
    );
</script>

<section class="proactive-notice" aria-label="Jitterbug noticed something" aria-live="polite">
    <div class="notice-top">
        <span class="bug-mark"><Logo size={22} live={checking} /></span><span class="notice-name"
            >Jitterbug<small
                >{checking
                    ? 'Investigating with Meta…'
                    : insight
                      ? 'Investigated with Meta'
                      : 'Noticed just now'}</small
            ></span
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
        padding: 18px 18px 16px;
        z-index: 18;
        background: #2a2a2e;
        border: 0;
        border-radius: 14px;
        box-shadow: var(--shadow-float);
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
        border-radius: 10px;
        background: #ffffff0d;
    }
    .notice-name {
        flex: 1;
        font-size: 12px;
        font-weight: 600;
    }
    .notice-name small {
        display: block;
        font-size: 11px;
        font-weight: 400;
        color: var(--muted);
    }
    h2 {
        font-size: 16px;
        line-height: 1.35;
        margin: 14px 0 6px;
        text-wrap: balance;
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
    @media (prefers-reduced-motion: reduce) {
        .proactive-notice {
            animation: none;
        }
    }
</style>
