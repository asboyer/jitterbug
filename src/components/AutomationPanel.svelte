<script lang="ts">
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage, memory, percent } from '../lib/format';
    import {
        incidentObservation,
        incidentOutcome,
        incidentStageLabels,
        incidentTitle,
    } from '../lib/incidents';
    import type { AutomationCase } from '../lib/types';
    import ActionResult from './ActionResult.svelte';
    import Investigation from './Investigation.svelte';

    let {
        incident,
        cases,
        onselect,
        desktop,
        onclose,
        onrefresh,
        onask,
    }: {
        incident: AutomationCase;
        cases: AutomationCase[];
        onselect: (id: number) => void;
        desktop: boolean;
        onclose: () => void;
        onrefresh: () => Promise<void>;
        onask: (incident: AutomationCase) => void;
    } = $props();
    let busy = $state(false);
    let error = $state('');
    const runningAction = $derived(['applying', 'verifying'].includes(incident.stage));
    const canMutate = $derived(desktop && incident.source === 'live' && !busy);
    const success = $derived(['completed', 'resolved'].includes(incident.stage));

    async function perform(command: string, args: Record<string, unknown>) {
        if (!canMutate) return;
        busy = true;
        error = '';
        try {
            await invoke(command, args);
            await onrefresh();
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
</script>

<aside class="assistant panel automation-assistant" aria-label="Automatic resource assistant">
    <div class="section-title incident-heading">
        <div>
            <p class="eyebrow">
                {incident.source === 'live' ? 'RESOURCE ASSISTANT · LIVE' : 'SYNTHETIC EXAMPLE'}
            </p>
            <h2>{incidentTitle(incident)}</h2>
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
            <section class="incident-summary">
                <h3>What happened</h3>
                <p>{incidentObservation(incident)}</p>
                <small>
                    {incident.source === 'live' ? 'Observed by Jitterbug' : 'Synthetic telemetry'} · {new Date(
                        incident.detected_at * 1000
                    ).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}
                </small>
            </section>
            {#if incident.proposal}
                <section class="proposal-card">
                    <p class="eyebrow">
                        {incident.report
                            ? 'ACTION TAKEN'
                            : incident.automatic
                              ? 'SAVED PERMISSION'
                              : 'SUGGESTED ACTION'}
                    </p>
                    <h3>Give other apps more CPU time</h3>
                    <p>
                        Lower the CPU priority of <strong>{incident.proposal.process.name}</strong>.
                    </p>
                    <p class="action-limit">
                        It keeps running. This changes scheduling preference; it does not free
                        memory or guarantee lower CPU use.
                    </p>
                    {#if incident.stage === 'awaiting_approval'}
                        <p class="action-limit">
                            Priority stays lower until the process exits. Restoring it may require
                            administrator privileges.
                        </p>
                        <div class="approval-actions">
                            <button
                                class="primary"
                                disabled={!canMutate}
                                onclick={() =>
                                    perform('approve_optimization', {
                                        id: incident.id,
                                        remember: false,
                                    })}>Apply once & measure</button
                            >
                            <button
                                disabled={!canMutate}
                                onclick={() =>
                                    perform('approve_optimization', {
                                        id: incident.id,
                                        remember: true,
                                    })}>Always allow for this workload</button
                            >
                        </div>
                        <small>
                            “Always allow” applies this change now and permits future automatic
                            priority changes for {incident.trigger.target?.label ??
                                'this workload'}. It never permits stopping processes. Remove
                            permission in Settings.
                        </small>
                    {/if}
                    <details class="proposal-detail">
                        <summary>Why this action & process details</summary>
                        <p>{incident.proposal.rationale}</p>
                        <p>
                            PID {incident.proposal.process.pid} · {percent(
                                incident.proposal.process.cpu_percent
                            )} CPU at proposal
                        </p>
                    </details>
                </section>
            {/if}
            <section class="incident-summary outcome" aria-label="Investigation outcome">
                <h3>{incident.verification ? 'Result' : 'Where things stand'}</h3>
                <p>{incidentOutcome(incident)}</p>
                {#if incident.verification}
                    <small
                        >Observed changes do not prove that the action improved performance.</small
                    >
                {/if}
            </section>
            <div class="incident-details">
                {#if incident.verification || incident.report}
                    <details class="detail-row">
                        <summary>Result & measurements</summary>
                        <div class="detail-content">
                            {#if incident.verification}
                                <section aria-label="Automatic verification result">
                                    <p>{incident.verification.summary}</p>
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
            </div>
        {/key}
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="incident-footer">
        <button
            class="followup-button"
            disabled={busy || runningAction}
            onclick={() => onask(incident)}>Ask a follow-up</button
        >
        <div class="secondary-actions">
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
        </div>
        <details class="monitoring-note">
            <summary>Monitoring stays on when you close this panel</summary>
            <p>
                Pause prevents new investigations and actions. Already-applied changes remain in
                effect.
            </p>
        </details>
    </div>
</aside>

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
    .incident-summary h3,
    .proposal-card h3 {
        font-size: 13px;
        margin: 0 0 8px;
    }
    .incident-summary p,
    .proposal-card p {
        margin: 0 0 8px;
        line-height: 1.6;
        font-size: 12px;
    }
    .incident-summary small,
    .proposal-card small {
        color: var(--muted);
        line-height: 1.55;
        font-size: 10px;
        display: block;
    }
    .proposal-card {
        border: 1px solid var(--border);
        background: var(--hover);
        padding: 16px;
        border-radius: 9px;
        margin-bottom: 20px;
    }
    .proposal-card .eyebrow {
        font-size: 9px;
        margin-bottom: 10px;
    }
    .proposal-card .action-limit {
        color: var(--muted);
        font-size: 11px;
    }
    .approval-actions {
        display: grid;
        gap: 8px;
        margin: 16px 0 10px;
    }
    .approval-actions button {
        padding: 8px 10px;
        font-size: 11px;
    }
    .proposal-detail {
        margin-top: 14px;
        color: var(--muted);
        font-size: 10px;
    }
    .proposal-detail p {
        margin-top: 10px;
        font-size: 11px;
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
        border-top: 1px solid var(--border);
        padding-top: 14px;
    }
    .followup-button {
        width: 100%;
        padding: 8px 10px;
    }
    .secondary-actions {
        display: flex;
        justify-content: space-between;
        flex-wrap: wrap;
        gap: 8px;
        margin-top: 10px;
    }
    .secondary-actions button {
        font-size: 10px;
        color: var(--muted);
    }
    .monitoring-note {
        color: var(--muted);
        font-size: 9px;
        line-height: 1.6;
        margin-top: 14px;
    }
</style>
