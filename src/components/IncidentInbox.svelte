<script lang="ts">
    import {
        incidentObservation,
        incidentOutcome,
        incidentStageLabels,
        incidentTitle,
    } from '../lib/incidents';
    import type { AutomationCase } from '../lib/types';

    let {
        cases,
        onselect,
    }: {
        cases: AutomationCase[];
        onselect: (id: number) => void;
    } = $props();
    const proposals = $derived(
        cases
            .filter((incident) => incident.stage === 'awaiting_approval')
            .sort((a, b) => b.detected_at - a.detected_at)
    );
    const recent = $derived(
        cases
            .filter((incident) => incident.stage !== 'awaiting_approval')
            .sort((a, b) => b.detected_at - a.detected_at)
    );
    const groups = $derived([
        { title: 'Proposed optimizations', cases: proposals },
        { title: 'Recent activity', cases: recent },
    ]);

    function timestamp(value: number): string {
        return new Date(value * 1000).toLocaleString([], {
            month: 'short',
            day: 'numeric',
            hour: 'numeric',
            minute: '2-digit',
        });
    }
</script>

<section class="incident-inbox" aria-label="Incident inbox">
    {#if cases.length === 0}
        <div class="panel empty-inbox">
            <span class="empty-mark" aria-hidden="true">✓</span>
            <h2>Nothing needs your attention</h2>
            <p>Resource alerts and proposed optimizations will appear here.</p>
        </div>
    {:else}
        {#each groups as group (group.title)}
            {#if group.cases.length > 0}
                <section class="inbox-group" aria-label={group.title}>
                    <div class="inbox-group-heading">
                        <h2>{group.title}</h2>
                        <span>{group.cases.length}</span>
                    </div>
                    <div class="panel incident-list">
                        {#each group.cases as incident (incident.id)}
                            <button class="incident-card" onclick={() => onselect(incident.id)}>
                                <span
                                    class="incident-mark"
                                    class:is-success={['completed', 'resolved'].includes(
                                        incident.stage
                                    )}
                                    class:is-attention={incident.stage === 'awaiting_approval'}
                                    class:is-blocked={incident.stage === 'blocked'}
                                    aria-hidden="true"
                                >
                                    {['completed', 'resolved'].includes(incident.stage)
                                        ? '✓'
                                        : incident.stage === 'awaiting_approval'
                                          ? '↗'
                                          : incident.stage === 'blocked'
                                            ? '!'
                                            : '·'}
                                </span>
                                <span class="incident-copy">
                                    <span class="incident-card-heading">
                                        <strong>{incidentTitle(incident)}</strong>
                                        <span class="incident-time"
                                            >{timestamp(incident.detected_at)}</span
                                        >
                                    </span>
                                    <span class="incident-description">
                                        {incident.stage === 'awaiting_approval' && incident.proposal
                                            ? `Lower ${incident.proposal.process.name}’s CPU priority to give other work more room.`
                                            : incidentObservation(incident)}
                                    </span>
                                    <span class="incident-meta">
                                        <span
                                            class="incident-status"
                                            class:is-success={['completed', 'resolved'].includes(
                                                incident.stage
                                            )}
                                            class:is-attention={incident.stage ===
                                                'awaiting_approval'}
                                            >{incidentStageLabels[incident.stage]}</span
                                        >
                                        {#if incident.source !== 'live'}<span
                                                >Synthetic example</span
                                            >{/if}
                                        {#if incident.verification}<span
                                                >{incidentOutcome(incident)}</span
                                            >{/if}
                                    </span>
                                </span>
                                <span class="incident-chevron" aria-hidden="true">›</span>
                            </button>
                        {/each}
                    </div>
                </section>
            {/if}
        {/each}
    {/if}
</section>

<style>
    .incident-inbox {
        display: grid;
        gap: 28px;
        padding-bottom: 24px;
    }
    .inbox-group-heading {
        display: flex;
        align-items: center;
        gap: 8px;
        margin-bottom: 12px;
    }
    .inbox-group-heading h2 {
        margin: 0;
        font-size: 13px;
    }
    .inbox-group-heading > span {
        color: var(--muted);
        font-size: 11px;
    }
    .incident-list {
        overflow: hidden;
    }
    .incident-card {
        border: 0;
        border-radius: 0;
        background: transparent;
        display: flex;
        align-items: flex-start;
        gap: 14px;
        text-align: left;
        width: 100%;
        padding: 18px;
    }
    .incident-card + .incident-card {
        border-top: 1px solid var(--border);
    }
    .incident-card:hover {
        background: var(--hover);
    }
    .incident-mark {
        width: 30px;
        height: 30px;
        flex-shrink: 0;
        border-radius: 8px;
        background: var(--hover);
        display: grid;
        place-items: center;
        color: var(--muted);
        font-size: 17px;
    }
    .incident-mark.is-success {
        background: var(--success-soft);
    }
    .incident-mark.is-attention {
        background: var(--warning-soft);
    }
    .is-success {
        color: var(--success);
    }
    .is-attention {
        color: var(--warning);
    }
    .is-blocked {
        color: var(--danger);
        background: var(--danger-soft);
    }
    .incident-copy {
        display: grid;
        gap: 7px;
        flex: 1;
        min-width: 0;
    }
    .incident-card-heading {
        display: flex;
        justify-content: space-between;
        align-items: baseline;
        flex-wrap: wrap;
        gap: 6px 12px;
    }
    .incident-card-heading strong {
        font-size: 13px;
        line-height: 1.4;
        overflow-wrap: anywhere;
    }
    .incident-time {
        font-size: 10px;
        color: var(--muted);
        white-space: nowrap;
    }
    .incident-description {
        display: -webkit-box;
        -webkit-box-orient: vertical;
        -webkit-line-clamp: 2;
        line-clamp: 2;
        overflow: hidden;
        font-size: 12px;
        color: var(--muted);
        line-height: 1.5;
    }
    .incident-meta {
        display: flex;
        flex-wrap: wrap;
        align-items: baseline;
        gap: 7px 12px;
        font-size: 10px;
        color: var(--muted);
        line-height: 1.5;
    }
    .incident-status {
        font-weight: 500;
    }
    .incident-chevron {
        align-self: center;
        color: var(--muted);
        font-size: 21px;
    }
    .empty-inbox {
        display: grid;
        justify-items: center;
        padding: 56px 24px;
        text-align: center;
    }
    .empty-mark {
        color: var(--success);
        background: var(--success-soft);
        width: 42px;
        height: 42px;
        border-radius: 50%;
        display: grid;
        place-items: center;
        font-size: 21px;
        margin-bottom: 18px;
    }
    .empty-inbox h2 {
        font-size: 15px;
        margin: 0 0 8px;
    }
    .empty-inbox p {
        color: var(--muted);
        font-size: 12px;
        margin: 0;
        line-height: 1.5;
    }
</style>
