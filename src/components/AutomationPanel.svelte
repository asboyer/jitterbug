<script lang="ts">
    import { tick } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage, memory, percent } from '../lib/format';
    import {
        incidentObservation,
        incidentOutcome,
        incidentStageLabels,
        incidentTitle,
    } from '../lib/incidents';
    import type { AutomationCase, ProcessRecord } from '../lib/types';
    import Icon from './Icon.svelte';
    import InfoTip from './InfoTip.svelte';
    import OptimizationReview from './OptimizationReview.svelte';
    import ActionResult from './ActionResult.svelte';
    import Investigation from './Investigation.svelte';
    import InsightCard from './InsightCard.svelte';
    import JitterReport from './JitterReport.svelte';

    let {
        incident,
        cases,
        onselect,
        desktop,
        onclose,
        onrefresh,
        onask,
        onaction,
    }: {
        incident: AutomationCase;
        cases: AutomationCase[];
        onselect: (id: number) => void;
        desktop: boolean;
        onclose: () => void;
        onrefresh: () => Promise<void>;
        onask: (incident: AutomationCase) => void;
        onaction?: (process: ProcessRecord, action: string) => void;
    } = $props();
    let busy = $state(false);
    let review: { key: string; incident: AutomationCase } | null = $state(null);
    const proposalKey = $derived(
        `${incident.id}:${incident.proposal?.process.pid}:${incident.proposal?.process.start_time}:${incident.proposal?.action}`
    );
    const reviewCurrent = $derived.by(
        () => review?.key === proposalKey && incident.stage === 'awaiting_approval'
    );
    let error = $state('');
    let reviewButton: HTMLButtonElement | undefined = $state();
    let panelTitle: HTMLHeadingElement;
    function openReview() {
        error = '';
        review = { key: proposalKey, incident: structuredClone($state.snapshot(incident)) };
    }
    function closeReview() {
        review = null;
        void tick().then(() => {
            if (reviewButton?.isConnected) reviewButton.focus();
            else panelTitle?.focus();
        });
    }
    const runningAction = $derived(['applying', 'verifying'].includes(incident.stage));
    const canMutate = $derived(desktop && incident.source === 'live' && !busy);
    const success = $derived(['completed', 'resolved'].includes(incident.stage));

    // A pause or tab close an agent asked for (or an incident offered): never applied
    // without this explicit, two-step choice.
    const granular = $derived(
        incident.stage === 'awaiting_approval' &&
            ['pause', 'close_tab'].includes(incident.alternative?.action ?? '')
            ? incident.alternative
            : null
    );
    let confirming = $state(false);
    async function approveGranular() {
        if (!confirming) {
            confirming = true;
            return;
        }
        confirming = false;
        await perform('approve_stop', { id: incident.id });
    }
    async function perform(command: string, args: Record<string, unknown>) {
        if (!canMutate) return false;
        busy = true;
        error = '';
        try {
            await invoke(command, args);
            await onrefresh();
            return true;
        } catch (cause) {
            error = errorMessage(cause);
            return false;
        } finally {
            busy = false;
        }
    }
</script>

<aside class="assistant panel automation-assistant" aria-label="Automatic resource assistant">
    <div class="section-title incident-heading">
        <div>
            <p class="eyebrow">
                {incident.source !== 'live'
                    ? 'SYNTHETIC EXAMPLE'
                    : incident.origin === 'snapshot'
                      ? 'JITTER SNAPSHOT · LIVE'
                      : incident.origin === 'spike'
                        ? 'SHORT HITCH · LIVE'
                        : 'RESOURCE ASSISTANT · LIVE'}
            </p>
            <h2 bind:this={panelTitle} tabindex="-1">{incidentTitle(incident)}</h2>
        </div>
        <button class="icon-button" aria-label="Close automatic assistant" onclick={onclose}
            >×</button
        >
    </div>
    {#if cases.length > 1}
        <label class="incident-picker"
            >Incident
            <select
                aria-label="Select incident"
                value={incident.id}
                onchange={(event) => onselect(Number(event.currentTarget.value))}
            >
                {#each cases as item}
                    <option value={item.id}>
                        {item.trigger.target?.label ?? 'System'} · {incidentStageLabels[item.stage]} ·
                        {new Date(item.detected_at * 1000).toLocaleTimeString([], {
                            hour: 'numeric',
                            minute: '2-digit',
                        })}
                    </option>
                {/each}
            </select>
        </label>
    {/if}
    <div
        class="incident-state"
        class:is-success={success}
        class:is-attention={incident.stage === 'awaiting_approval'}
    >
        <span class="state-dot" aria-hidden="true"></span>
        <span aria-live="polite">{incidentStageLabels[incident.stage]}</span>
    </div>
    <div class="conversation automatic-conversation">
        {#key incident.id}
            {#if incident.jitter}
                <JitterReport
                    report={incident.jitter}
                    {incident}
                    canAct={canMutate}
                    onreview={openReview}
                    onstop={onaction ? (process) => onaction(process, 'terminate') : undefined}
                />
                {#if incident.verification}
                    <section class="incident-summary outcome" aria-label="Snapshot action result">
                        <h3>Result</h3>
                        <p>{incident.verification.summary}</p>
                    </section>
                {/if}
            {:else if incident.diagnosis?.insight && !incident.proposal}
                <InsightCard reply={incident.diagnosis} />
            {:else}
                <p class="observation">{incidentObservation(incident)}</p>
            {/if}
            {#if granular}
                <section class="proposal-card" aria-label="Requested action">
                    <div class="tip-heading">
                        <Icon name="bulb" size={20} />
                        <h3>
                            {granular.action === 'pause'
                                ? `Pause ${granular.unit?.title ?? granular.process.name}`
                                : `Close the “${granular.unit?.title ?? granular.process.name}” tab`}
                        </h3>
                    </div>
                    {#if incident.origin === 'agent'}<p class="muted">
                            Requested by an agent: {incident.trigger.reason}
                        </p>{/if}
                    <p>{granular.rationale}</p>
                    <button
                        class:primary={!confirming}
                        class:danger={confirming}
                        disabled={busy || !canMutate}
                        onclick={approveGranular}
                        >{confirming
                            ? 'Confirm'
                            : granular.action === 'pause'
                              ? 'Pause…'
                              : 'Close tab…'}</button
                    >
                </section>
            {/if}
            {#if incident.report?.action === 'pause' && incident.stage === 'completed'}
                <button
                    disabled={busy || !canMutate}
                    onclick={() => perform('resume_case', { id: incident.id })}>Resume now</button
                >
            {/if}
            {#if incident.proposal && !incident.jitter}
                <section class="proposal-card" data-tour="proposal">
                    <div class="tip-heading">
                        <Icon name="bulb" size={20} />
                        <h3>
                            {incident.report
                                ? 'Priority change requested'
                                : 'Give other apps more room'}
                        </h3>
                        <InfoTip label="About this optimization">
                            <p>
                                Lower the CPU priority of <strong
                                    >{incident.proposal.process.name}</strong
                                > so other work gets scheduling preference. It keeps running.
                            </p>
                            <p>
                                This does not free memory or guarantee lower CPU use. It lasts until
                                the process exits; undo may require administrator access.
                            </p>
                            {#if incident.automatic}<p>
                                    You previously allowed automatic priority changes for this
                                    workload.
                                </p>{/if}
                        </InfoTip>
                    </div>
                    {#if incident.stage === 'awaiting_approval'}
                        <button
                            class="primary review-button"
                            bind:this={reviewButton}
                            disabled={busy}
                            onclick={openReview}>Review change</button
                        >
                    {/if}
                </section>
            {/if}
            {#if !incident.jitter && (!incident.diagnosis?.insight || incident.report) && !['awaiting_approval', 'detected', 'investigating', 'applying', 'verifying'].includes(incident.stage)}
                <section class="incident-summary outcome" aria-label="Investigation outcome">
                    <h3>{incident.verification ? 'Result' : 'Outcome'}</h3>
                    <p>{incidentOutcome(incident)}</p>
                </section>
            {/if}
            <details class="incident-details">
                <summary class="details-toggle"><Icon name="eye" size={16} /> View details</summary>

                {#if incident.verification || incident.report}
                    <details class="detail-row">
                        <summary>Result & measurements</summary>
                        <div class="detail-content">
                            {#if incident.verification}
                                <section aria-label="Automatic verification result">
                                    <p>{incident.verification.summary}</p>
                                    <p class="muted">
                                        Observed changes do not prove that the action improved
                                        performance.
                                    </p>
                                    {#if incident.verification.system_cpu_after !== null}
                                        <dl class="measurement-list">
                                            <div>
                                                <dt>Total CPU</dt>
                                                <dd>
                                                    {percent(
                                                        incident.verification.system_cpu_before
                                                    )} → {percent(
                                                        incident.verification.system_cpu_after
                                                    )}
                                                </dd>
                                            </div>
                                            <div>
                                                <dt>Available memory</dt>
                                                <dd>
                                                    {memory(
                                                        incident.verification
                                                            .available_memory_before
                                                    )} → {incident.verification
                                                        .available_memory_after === null
                                                        ? 'Unavailable'
                                                        : memory(
                                                              incident.verification
                                                                  .available_memory_after
                                                          )}
                                                </dd>
                                            </div>
                                            <div>
                                                <dt>Pressure</dt>
                                                <dd>
                                                    {incident.verification.pressure_before} → {incident
                                                        .verification.pressure_after ??
                                                        'Unavailable'}
                                                </dd>
                                            </div>
                                        </dl>
                                    {/if}
                                </section>
                            {/if}
                            {#if incident.report}<ActionResult report={incident.report} />{/if}
                        </div>
                    </details>
                {/if}
                {#if incident.diagnosis}
                    <details class="detail-row analysis-detail">
                        <summary>
                            <span>Full analysis</span>
                            <span class="detail-hint"
                                >{incident.diagnosis.source === 'model'
                                    ? 'AI interpretation'
                                    : 'Rules'}</span
                            >
                        </summary>
                        <div class="detail-content">
                            <p class="analysis-provenance">
                                {incident.diagnosis.source === 'model'
                                    ? `${incident.diagnosis.provider} / ${incident.diagnosis.model} · AI interpretation of collected evidence`
                                    : 'Deterministic rules applied to collected evidence'}
                            </p>
                            <p class="full-analysis">{incident.diagnosis.text}</p>
                            {#if incident.diagnosis.source === 'model'}
                                <small
                                    >Estimated token cost: ${incident.diagnosis.estimated_cost_usd.toFixed(
                                        5
                                    )}</small
                                >
                            {/if}
                            <details class="routing-detail">
                                <summary>Model routing and limits</summary>
                                <ul>
                                    {#each incident.diagnosis.route as step}<li>{step}</li>{/each}
                                </ul>
                            </details>
                        </div>
                    </details>
                {/if}
                <details class="detail-row">
                    <summary>Detection & evidence</summary>
                    <div class="detail-content">
                        <p>{incident.trigger.reason}</p>
                        <p class="muted">
                            {new Date(incident.detected_at * 1000).toLocaleString()} · {incident.source ===
                            'live'
                                ? 'Observed telemetry'
                                : 'Synthetic example'}
                        </p>
                        {#if incident.proposal}<p>{incident.proposal.rationale}</p>{/if}
                        {#if incident.evidence}
                            <Investigation
                                evidence={incident.evidence}
                                disabled={true}
                                onaction={() => {}}
                            />
                        {/if}
                    </div>
                </details>
                <details class="detail-row">
                    <summary
                        >{incident.stage === 'blocked'
                            ? 'Why this step needs attention'
                            : 'Activity log'}</summary
                    >
                    <ol class="incident-timeline">
                        {#each incident.steps as step}
                            <li>
                                <div>
                                    <strong>{incidentStageLabels[step.stage]}</strong><time
                                        >{new Date(step.at * 1000).toLocaleTimeString()}</time
                                    >
                                </div>
                                <p>{step.message}</p>
                            </li>
                        {/each}
                    </ol>
                </details>
            </details>
        {/key}
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="incident-footer">
        <button
            class="followup-button"
            disabled={busy || runningAction}
            onclick={() => onask(incident)}>Ask a follow-up</button
        >
        <details class="more-actions">
            <summary aria-label="More incident options" title="More incident options"
                ><Icon name="more" size={20} /></summary
            >
            <div class="more-menu">
                <button
                    disabled={!canMutate || runningAction || incident.stage === 'dismissed'}
                    onclick={() => perform('dismiss_automation_case', { id: incident.id })}
                    >Dismiss incident</button
                >
                <button
                    disabled={!canMutate}
                    onclick={() => perform('snooze_automation', { minutes: 15 })}
                    >Pause automation 15 min</button
                >
                <p>
                    Closing keeps monitoring on. Pausing prevents new investigations and actions;
                    earlier changes remain.
                </p>
            </div>
        </details>
    </div>
</aside>

{#if review}
    <OptimizationReview
        incident={review.incident}
        current={reviewCurrent}
        {busy}
        {error}
        canApply={canMutate}
        onclose={closeReview}
        onapply={async () => {
            if (!reviewCurrent || !canMutate) return;
            if (await perform('approve_optimization', { id: incident.id })) closeReview();
        }}
    />
{/if}

<style>
    .incident-heading {
        gap: 16px;
        margin-bottom: 18px;
    }
    .incident-heading h2 {
        font-size: 16px;
        line-height: 1.4;
    }
    .incident-heading .icon-button {
        flex-shrink: 0;
    }
    .incident-picker {
        display: grid;
        grid-template-columns: auto minmax(0, 1fr);
        align-items: center;
        gap: 12px;
        color: var(--muted);
        font-size: 11px;
    }
    .incident-picker select {
        min-width: 0;
        width: 100%;
    }
    .incident-state {
        display: flex;
        align-items: center;
        gap: 7px;
        color: var(--muted);
        font-size: 11px;
        padding: 7px 0 0;
    }
    .incident-state.is-success {
        color: var(--success);
    }
    .incident-state.is-attention {
        color: var(--warning);
    }
    .state-dot {
        width: 6px;
        height: 6px;
        border-radius: 50%;
        background: currentColor;
    }
    .automatic-conversation {
        padding: 22px 0;
    }
    .incident-summary {
        padding: 0 1px 20px;
    }
    .incident-summary h3 {
        font-size: 13px;
        margin: 0 0 8px;
    }
    .incident-summary p,
    .observation {
        font-size: 13px;
        line-height: 1.6;
        margin: 0 0 20px;
    }
    .proposal-card {
        border: 1px solid var(--border);
        background: var(--hover);
        padding: 16px;
        border-radius: 9px;
        margin-bottom: 20px;
    }
    .tip-heading {
        display: flex;
        align-items: center;
        gap: 10px;
    }
    .tip-heading h3 {
        flex: 1;
        margin: 0;
        font-size: 14px;
        line-height: 1.4;
    }
    .review-button {
        width: 100%;
        margin-top: 16px;
    }
    .details-toggle {
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 14px 0;
        color: var(--muted);
        font-size: 12px;
        list-style: none;
        cursor: pointer;
    }
    .details-toggle::-webkit-details-marker {
        display: none;
    }
    .outcome {
        padding-bottom: 22px;
    }
    .incident-details {
        border-top: 1px solid var(--border);
    }
    .detail-row {
        border-bottom: 1px solid var(--border);
    }
    .detail-row > summary {
        padding: 13px 0;
        cursor: pointer;
        font-size: 11px;
        color: var(--muted);
    }
    .detail-hint {
        float: right;
        font-size: 10px;
        color: var(--muted);
    }
    .detail-content {
        padding: 0 0 14px;
        font-size: 12px;
        line-height: 1.6;
    }
    .detail-content > p:first-child {
        margin-top: 0;
    }
    .analysis-provenance {
        color: var(--muted);
        font-size: 10px;
        overflow-wrap: anywhere;
    }
    .full-analysis {
        white-space: pre-wrap;
        overflow-wrap: anywhere;
    }
    .routing-detail {
        margin-top: 14px;
        color: var(--muted);
        font-size: 11px;
    }
    .measurement-list {
        display: grid;
        gap: 10px;
        font-size: 11px;
    }
    .measurement-list div {
        display: flex;
        justify-content: space-between;
        flex-wrap: wrap;
        gap: 8px;
    }
    .measurement-list dt {
        color: var(--muted);
    }
    .measurement-list dd {
        margin: 0;
        font-variant-numeric: tabular-nums;
    }
    .incident-timeline {
        padding-left: 18px;
        margin: 0 0 16px;
        font-size: 11px;
    }
    .incident-timeline li {
        margin: 0 0 16px;
        padding-left: 4px;
    }
    .incident-timeline li > div {
        display: flex;
        justify-content: space-between;
        gap: 8px;
    }
    .incident-timeline time {
        color: var(--muted);
        font-size: 10px;
        white-space: nowrap;
    }
    .incident-timeline p {
        color: var(--muted);
        line-height: 1.6;
        margin: 7px 0 0;
    }
    .incident-footer {
        display: flex;
        gap: 10px;
        align-items: center;
        border-top: 1px solid var(--border);
        padding-top: 14px;
    }
    .followup-button {
        flex: 1;
    }
    .more-actions {
        position: relative;
    }
    .more-actions > summary {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 32px;
        height: 32px;
        list-style: none;
        cursor: pointer;
        border-radius: 6px;
        color: var(--muted);
    }
    .more-actions > summary::-webkit-details-marker {
        display: none;
    }
    .more-actions[open] > summary {
        background: var(--hover);
    }
    .more-menu {
        position: absolute;
        bottom: 42px;
        right: 0;
        width: 240px;
        border: 1px solid var(--border);
        border-radius: 9px;
        padding: 10px;
        display: grid;
        gap: 8px;
        background: var(--panel);
        box-shadow: 0 8px 24px #0003;
        z-index: 12;
    }
    .more-menu p {
        margin: 4px;
        font-size: 11px;
        color: var(--muted);
        line-height: 1.6;
    }
</style>
