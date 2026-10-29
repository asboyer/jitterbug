<script lang="ts">
    import { tick } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage, memory, percent } from '../lib/format';
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
    let conversationElement: HTMLDivElement;
    $effect(() => {
        if (incident.verification) {
            void tick().then(() => {
                const result = conversationElement?.querySelector<HTMLElement>(
                    '[aria-label="Automatic verification result"]'
                );
                if (result)
                    conversationElement.scrollTop +=
                        result.getBoundingClientRect().top -
                        conversationElement.getBoundingClientRect().top;
            });
        }
    });

    const runningAction = $derived(['applying', 'verifying'].includes(incident.stage));
    const stageLabels: Record<string, string> = {
        detected: 'Problem detected',
        investigating: 'Investigating automatically',
        awaiting_approval: 'Optimization proposed',
        applying: 'Applying optimization',
        verifying: 'Measuring the result',
        completed: 'Investigation complete',
        resolved: 'Trigger cleared',
        blocked: 'Needs attention',
        dismissed: 'Dismissed',
    };

    async function perform(command: string, args: Record<string, unknown>) {
        if (!desktop || incident.source !== 'live' || busy) return;
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
    <div class="section-title">
        <div>
            <p class="eyebrow">
                {incident.source === 'live' ? 'LIVE · AUTOMATIC ASSISTANT' : 'MOCKED · DEMO'}
            </p>
            <h2>{incident.trigger.title}</h2>
        </div>
        <button class="icon-button" aria-label="Close automatic assistant" onclick={onclose}
            >×</button
        >
    </div>
    {#if cases.length > 1}
        <label class="incident-picker"
            >Recent incidents
            <select
                aria-label="Select incident"
                value={incident.id}
                onchange={(event) => onselect(Number(event.currentTarget.value))}
            >
                {#each cases as item}<option value={item.id}
                        >{item.trigger.target?.label ?? 'System'} · {stageLabels[item.stage]} · {new Date(
                            item.detected_at * 1000
                        ).toLocaleTimeString()}</option
                    >{/each}
            </select>
        </label>
    {/if}
    <div class="context-note">
        <strong>{stageLabels[incident.stage]}</strong>
        <p>{incident.steps[incident.steps.length - 1]?.message}</p>
    </div>
    <div
        class="conversation automatic-conversation"
        bind:this={conversationElement}
        aria-live="polite"
    >
        <p>{incident.trigger.reason}</p>
        {#if incident.diagnosis}
            <article class="chat-message">
                <div class="message-label">
                    {incident.diagnosis.source === 'model'
                        ? `Model · ${incident.diagnosis.provider} / ${incident.diagnosis.model}`
                        : 'Observed · deterministic rules'}
                </div>
                <p>{incident.diagnosis.text}</p>
                {#if incident.diagnosis.source === 'model'}<small
                        >Estimated token cost: ${incident.diagnosis.estimated_cost_usd.toFixed(
                            5
                        )}</small
                    >{/if}
                <details>
                    <summary>Model routing and limits</summary>
                    <ul>
                        {#each incident.diagnosis.route as step}<li>{step}</li>{/each}
                    </ul>
                </details>
            </article>
        {/if}
        {#if incident.proposal}
            <section class="investigation">
                <p class="eyebrow">
                    {incident.report
                        ? 'APPLIED OPTIMIZATION'
                        : incident.automatic
                          ? 'PERMITTED AUTOMATIC ACTION'
                          : 'PROPOSED OPTIMIZATION'}
                </p>
                <h3>Lower {incident.proposal.process.name}’s CPU priority</h3>
                <p>
                    PID {incident.proposal.process.pid} · {percent(
                        incident.proposal.process.cpu_percent
                    )} CPU
                </p>
                <p>{incident.proposal.rationale}</p>
                {#if incident.stage === 'awaiting_approval'}
                    <div class="automation-actions">
                        <button
                            class="primary"
                            disabled={busy || !desktop}
                            onclick={() =>
                                perform('approve_optimization', {
                                    id: incident.id,
                                    remember: false,
                                })}>Apply once & measure</button
                        >
                        <button
                            disabled={busy || !desktop}
                            onclick={() =>
                                perform('approve_optimization', {
                                    id: incident.id,
                                    remember: true,
                                })}>Allow for this workload & apply</button
                        >
                    </div>
                    <small
                        >“Allow” also permits future automatic priority changes for {incident
                            .trigger.target?.label}. It never permits stopping processes. Remove
                        permission in Settings.</small
                    >
                {/if}
            </section>
        {/if}
        {#if incident.verification}
            <section class="investigation" aria-label="Automatic verification result">
                <p class="eyebrow">VERIFICATION</p>
                <p>{incident.verification.summary}</p>
                {#if incident.verification.system_cpu_after !== null}
                    <p>
                        Whole-machine CPU: {percent(incident.verification.system_cpu_before)} → {percent(
                            incident.verification.system_cpu_after
                        )}
                    </p>
                    <p>
                        Available memory: {memory(incident.verification.available_memory_before)} → {memory(
                            incident.verification.available_memory_after ?? 0
                        )}
                    </p>
                    <p>
                        Pressure: {incident.verification.pressure_before} → {incident.verification
                            .pressure_after}
                    </p>
                {/if}
            </section>
        {/if}
        {#if incident.report}<ActionResult report={incident.report} />{/if}
        <details class="automation-timeline" open>
            <summary>What Jitterbug did</summary>
            <ol>
                {#each incident.steps as step}<li>
                        <strong>{stageLabels[step.stage]}</strong><small
                            >{new Date(step.at * 1000).toLocaleTimeString()}</small
                        >
                        <p>{step.message}</p>
                    </li>{/each}
            </ol>
        </details>
        {#if incident.evidence}<details>
                <summary>Evidence used for this investigation</summary><Investigation
                    evidence={incident.evidence}
                    disabled={true}
                    onaction={() => {}}
                />
            </details>{/if}
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="automation-actions">
        <button disabled={busy || runningAction} onclick={() => onask(incident)}
            >Ask a follow-up</button
        >
        <button
            disabled={busy || runningAction || !desktop || incident.stage === 'dismissed'}
            onclick={() => perform('dismiss_automation_case', { id: incident.id })}
            >Dismiss incident</button
        >
        <button
            disabled={busy || !desktop}
            onclick={() => perform('snooze_automation', { minutes: 15 })}
            >Pause automation 15 min</button
        >
    </div>
    <p class="footnote">
        Closing this panel keeps monitoring active. Pause prevents new investigations and actions;
        already-applied changes remain in effect.
    </p>
</aside>
