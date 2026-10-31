<script lang="ts">
    import { onMount, untrack } from 'svelte';
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import { listen, type UnlistenFn } from '@tauri-apps/api/event';
    import Icon from './Icon.svelte';
    import { demoActionPopup } from '../lib/demo';
    import { errorMessage } from '../lib/format';
    import {
        actionResult,
        helperNote,
        incidentObservation,
        incidentOutcome,
        nowLine,
        popupTitle,
        priorityAfter,
        priorityLabel,
        snapshotBadge,
        stopAfter,
        stopLabel,
        stopWarning,
    } from '../lib/incidents';
    import type {
        ActionPopupCase,
        ActionPopupOption,
        ActionPopupPayload,
        AutomationStage,
    } from '../lib/types';

    // A preset renders one fixed example (the Lab's gallery): played locally, never applied.
    let { preset = null }: { preset?: ActionPopupPayload | null } = $props();
    const fixed = untrack(() => preset);
    const desktop = isTauri() && !fixed;
    const finished: AutomationStage[] = ['completed', 'resolved', 'blocked'];
    let payload = $state<ActionPopupPayload | null>(
        fixed ??
            (desktop ? null : demoActionPopup(new URLSearchParams(location.search).get('example')))
    );
    const capture = $derived(payload?.kind === 'capturing' ? payload : null);
    const popup = $derived(payload?.kind === 'case' ? payload : null);
    // Synthetic cases never reach an action command; their progress is played locally.
    let example = $state<ActionPopupCase | null>(null);
    let busy = $state(false);
    let error = $state('');
    let hovering = $state(false);
    // A stop needs a second click within a few seconds.
    let confirming = $state(false);
    // Closing a tab offered after pausing it needs its own second click.
    let confirmingFallback = $state(false);
    // The priority change this result offered to undo was restored.
    let restored = $state(false);
    let timers: ReturnType<typeof setTimeout>[] = [];
    const shown = $derived(example && example.id === popup?.id ? example : popup);
    const asking = $derived(shown?.stage === 'awaiting_approval');
    const analyzing = $derived(shown?.stage === 'detected' || shown?.stage === 'investigating');
    const working = $derived(
        !!capture || analyzing || shown?.stage === 'applying' || shown?.stage === 'verifying'
    );
    const done = $derived(!!shown && finished.includes(shown.stage));
    const badge = $derived(shown ? snapshotBadge(shown) : null);
    const victim = $derived(shown?.jitter?.victim?.label ?? null);
    const result = $derived(
        shown?.report && done
            ? actionResult(shown.report, shown.verification, shown.proposal, victim)
            : null
    );
    const undo = $derived(done && !restored ? (shown?.report?.undo ?? null) : null);
    // The option the numbers describe: the priority change if offered, else the stop.
    const subject = $derived(shown ? (shown.proposal ?? shown.alternative) : null);
    const stopping = $derived(
        ['terminate', 'pause', 'pause_tab', 'close_tab'].includes(shown?.proposal?.action ?? '')
    );
    const title = $derived.by(() => {
        if (!shown) return '';
        switch (shown.stage) {
            case 'applying':
                return stopping
                    ? `${stopLabel(shown.proposal!).replace(/^(Quit|Stop|Pause|Close)/, (verb) => ({ Quit: 'Quitting', Stop: 'Stopping', Pause: 'Pausing', Close: 'Closing' })[verb] ?? verb)}…`
                    : 'Applying the priority change…';
            case 'verifying':
                return 'Checking the result…';
            case 'completed':
                if (restored)
                    return `Restored ${shown.proposal?.impact?.label ?? shown.report?.process ?? 'the app'}`;
                return result?.title ?? popupTitle(shown);
            case 'resolved':
                return 'No change was needed';
            case 'blocked':
                return 'Jitterbug couldn’t finish this step';
            default:
                return popupTitle(shown);
        }
    });

    function stopTimers() {
        timers.forEach(clearTimeout);
        timers = [];
    }
    onMount(() => {
        if (!desktop) return () => stopTimers();
        let stop: UnlistenFn | undefined;
        let disposed = false;
        async function initialize() {
            try {
                stop = await listen<ActionPopupPayload | null>('action-popup', (event) => {
                    receive(event.payload);
                });
                if (disposed) {
                    stop();
                    return;
                }
                receive(await invoke<ActionPopupPayload | null>('get_action_popup'));
            } catch (cause) {
                error = errorMessage(cause);
            }
        }
        void initialize();
        return () => {
            disposed = true;
            stop?.();
            stopTimers();
        };
    });
    function receive(next: ActionPopupPayload | null) {
        if (next?.id !== payload?.id) {
            error = '';
            example = null;
            confirming = confirmingFallback = false;
            restored = false;
            stopTimers();
        }
        payload = next;
    }

    // A finished result hides itself; pointing at it keeps it open.
    $effect(() => {
        if (!done || hovering || !shown) return;
        const id = shown.id;
        const timer = setTimeout(() => void close(id), 12000);
        return () => clearTimeout(timer);
    });

    async function close(id = shown?.id) {
        if (id === undefined) return;
        stopTimers();
        example = null;
        if (!desktop) {
            payload = null;
            return;
        }
        try {
            await invoke('close_action_popup', { id });
        } catch (cause) {
            error = errorMessage(cause);
        }
    }
    async function details() {
        if (!shown || !desktop) return;
        try {
            await invoke('open_action_case', { id: shown.id, source: shown.source });
        } catch (cause) {
            error = errorMessage(cause);
        }
    }
    async function apply() {
        if (!shown || !asking || busy) return;
        error = '';
        if (shown.source !== 'live') {
            playExample(shown);
            return;
        }
        busy = true;
        try {
            // The stored proposal is rechecked against fresh process identity before acting.
            // Saved "always lower" permissions are managed in Settings, never from here.
            await invoke('approve_optimization', { id: shown.id, remember: false });
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
    async function restore() {
        if (undo === null || busy) return;
        error = '';
        if (shown?.source !== 'live') {
            restored = true;
            return;
        }
        busy = true;
        try {
            // The change is undone only for the same process, after a fresh identity check.
            await invoke('restore_priority', { id: undo });
            restored = true;
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
    async function resume() {
        if (!shown || busy) return;
        busy = true;
        error = '';
        try {
            await invoke('resume_case', { id: shown.id });
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
    async function stop(fallback = false) {
        const option = fallback ? shown?.fallback : shown?.alternative;
        if (!shown || !option || !asking || busy) return;
        if (!(fallback ? confirmingFallback : confirming)) {
            if (fallback) confirmingFallback = true;
            else confirming = true;
            timers.push(setTimeout(() => (confirming = confirmingFallback = false), 5000));
            return;
        }
        confirming = confirmingFallback = false;
        error = '';
        if (shown.source !== 'live') {
            playStopExample(shown, option);
            return;
        }
        busy = true;
        try {
            // Only the offered process is stopped, after fresh identity and policy rechecks.
            await invoke('approve_stop', { id: shown.id, fallback });
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
    function playStopExample(base: ActionPopupCase, option: ActionPopupOption) {
        stopTimers();
        example = { ...base, proposal: option, stage: 'applying' };
        timers.push(
            setTimeout(() => {
                example = {
                    ...base,
                    proposal: option,
                    stage: 'completed',
                    report: {
                        fresh: true,
                        after: null,
                        action: option.action,
                        process: option.unit?.title ?? option.process.name,
                        cpu_before: option.process.cpu_percent,
                        cpu_after: null,
                        memory_before: option.process.memory_bytes,
                        memory_after: null,
                    },
                    verification: {
                        priority_verified: false,
                        system_cpu_before: 62,
                        system_cpu_after: 18,
                        available_memory_before: 4.1 * 1024 ** 3,
                        available_memory_after: 7.3 * 1024 ** 3,
                    },
                };
            }, 1500)
        );
    }
    function playExample(base: ActionPopupCase) {
        stopTimers();
        example = { ...base, stage: 'applying' };
        timers.push(
            setTimeout(() => {
                example = { ...base, stage: 'verifying' };
            }, 1000),
            setTimeout(() => {
                example = {
                    ...base,
                    stage: 'completed',
                    report: {
                        fresh: true,
                        after: true,
                        action: 'lower_priority',
                        process: base.proposal?.process.name ?? 'the process',
                        cpu_before: 12.5,
                        cpu_after: 12.4,
                        memory_before: base.proposal?.process.memory_bytes ?? 0,
                        memory_after: base.proposal?.process.memory_bytes ?? 0,
                        undo: 1,
                    },
                    verification: {
                        priority_verified: true,
                        system_cpu_before: 24,
                        system_cpu_after: 23,
                        available_memory_before: 8 * 1024 ** 3,
                        available_memory_after: 8 * 1024 ** 3,
                    },
                };
            }, 2500)
        );
    }
</script>

<svelte:window
    onkeydown={(event) => {
        if (event.key === 'Escape' && shown && !busy) void close();
    }}
/>
{#if capture}
    <section class="action-popup" aria-label="Jitterbug is capturing a snapshot" aria-live="polite">
        <header>
            <span class="bug-mark working"><Icon name="bug" size={16} /></span>
            <span class="name">Jitterbug <small>LIVE</small></span>
        </header>
        <h2>Capturing snapshot…</h2>
        <p class="reason outcome">{capture.message}</p>
        <div class="recording" aria-hidden="true"><span></span></div>
    </section>
{:else if shown}
    <section
        class="action-popup"
        aria-label="Jitterbug needs you"
        aria-live="polite"
        onpointerenter={() => (hovering = true)}
        onpointerleave={() => (hovering = false)}
    >
        <header>
            <span class="bug-mark" class:working><Icon name="bug" size={16} /></span>
            <span class="name"
                >Jitterbug <small>{shown.source === 'live' ? 'LIVE' : 'EXAMPLE'}</small></span
            >
            {#if badge}<span class="more">{badge}</span>{/if}
            {#if shown.more > 0}<span class="more">+{shown.more} more</span>{/if}
            <button
                class="icon-button"
                aria-label="Dismiss popup"
                disabled={busy}
                onclick={() => close()}>×</button
            >
        </header>
        {#if undo !== null}<h2 class="with-undo">
                <span>{title}</span> ·
                <button class="text-button" disabled={busy} onclick={restore}
                    >{busy ? 'Undoing…' : 'Undo'}</button
                >
            </h2>{:else}<h2>{title}</h2>{/if}
        {#if asking && shown.trigger.reason}<p class="note clamp">
                Reason given: {shown.trigger.reason}
            </p>{/if}
        {#if asking}
            {#if subject}<p class="measured">{nowLine(subject)}</p>
                {#if helperNote(subject)}<p class="note clamp">{helperNote(subject)}</p>{/if}
            {:else}<p class="reason">{incidentObservation(shown)}</p>{/if}
            <ul class="options">
                {#if shown.proposal}<li>
                        <div>
                            <strong>{priorityLabel(victim)}</strong>
                            <span>{priorityAfter()}</span>
                        </div>
                        <button class="primary" disabled={busy} onclick={apply}
                            >{busy ? 'Applying…' : 'Apply'}</button
                        >
                    </li>{/if}
                {#if shown.alternative}<li>
                        <div>
                            <strong title={stopLabel(shown.alternative)}
                                >{stopLabel(shown.alternative)}</strong
                            >
                            <span>{stopAfter(shown.alternative)}</span>
                            {#if shown.alternative.closes}<span
                                    class="closes"
                                    title={shown.alternative.closes}
                                    >{stopWarning(shown.alternative)}</span
                                >{/if}
                        </div>
                        <div class="choices">
                            <button class:danger={confirming} disabled={busy} onclick={() => stop()}
                                >{confirming
                                    ? 'Confirm'
                                    : ['pause', 'pause_tab'].includes(shown.alternative.action)
                                      ? 'Pause…'
                                      : shown.alternative.action === 'close_tab'
                                        ? 'Close…'
                                        : shown.alternative.impact?.whole_app
                                          ? 'Quit…'
                                          : 'Stop…'}</button
                            >
                            <!-- Closing the tab is offered second, in the same row. -->
                            {#if shown.fallback}<button
                                    class:danger={confirmingFallback}
                                    disabled={busy}
                                    title={`${stopLabel(shown.fallback)}. ${stopAfter(shown.fallback)}`}
                                    onclick={() => stop(true)}
                                    >{confirmingFallback ? 'Confirm' : 'Close…'}</button
                                >{/if}
                        </div>
                    </li>{/if}
            </ul>
        {:else if restored}
            <p class="reason outcome">
                It is back to its usual priority and competes for CPU as before.
            </p>
        {:else if result}
            <p class="proposal">{result.meaning}</p>
            <p class="measured">{result.measured}</p>
            {#if result.note}<p class="note">{result.note}</p>{/if}
            {#if shown.report?.resumable && shown.source === 'live'}<button
                    class="resume"
                    disabled={busy}
                    onclick={resume}>{busy ? 'Resuming…' : 'Resume now'}</button
                >{/if}
        {:else if shown.jitter && (analyzing || (shown.stage === 'completed' && !shown.report))}
            <p class="reason outcome clamp">
                {analyzing ? shown.jitter.happened : shown.jitter.likely_cause}
            </p>
            {#if analyzing}<p class="note">
                    Analyzing what happened…
                </p>{:else if shown.jitter.manual}<p class="proposal">{shown.jitter.manual}</p>{/if}
        {:else}
            <p class="reason outcome">{incidentOutcome(shown)}</p>
        {/if}
        {#if error}<p class="error" role="alert" title={error}>
                {error}
            </p>{:else if asking && shown.source !== 'live'}<p class="note">
                Example only. Nothing will be applied.
            </p>{/if}
        <div class="actions">
            <span class="links">
                <button class="text-button" disabled={!desktop || busy} onclick={details}
                    >{done && !shown.report ? 'Review' : 'Details'}</button
                >
                {#if asking}<button class="text-button" disabled={busy} onclick={() => close()}
                        >Later</button
                    >{:else if !working}<button class="text-button" onclick={() => close()}
                        >Close</button
                    >{/if}
            </span>
        </div>
    </section>
{/if}

<style>
    .action-popup {
        height: 100vh;
        display: flex;
        flex-direction: column;
        padding: 12px 14px;
        border: 1px solid var(--border);
        border-radius: 12px;
        background: color-mix(in srgb, var(--panel) 87%, transparent);
        overflow: hidden;
        animation: arrive 180ms ease-out;
    }
    .action-popup > * {
        flex-shrink: 0;
    }
    header {
        display: flex;
        align-items: center;
        gap: 8px;
    }
    .bug-mark {
        width: 24px;
        height: 24px;
        display: grid;
        place-items: center;
        border-radius: 7px;
        background: var(--accent-soft);
        color: var(--accent);
    }
    .working {
        animation: breathe 1.8s ease-in-out infinite;
    }
    .name {
        flex: 1;
        font-size: 11px;
        font-weight: 600;
    }
    .name small,
    .more {
        font-size: 8px;
        letter-spacing: 0.8px;
        color: var(--muted);
    }
    .name small {
        margin-left: 6px;
    }
    .more {
        font-size: 10px;
        letter-spacing: 0;
        padding: 1px 7px;
        border-radius: 9px;
        background: var(--hover);
    }
    header .icon-button {
        font-size: 20px;
        line-height: 16px;
        padding: 2px 4px;
        color: var(--muted);
    }
    h2 {
        margin: 9px 0 3px;
        font-size: 13px;
        line-height: 1.35;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .with-undo {
        display: flex;
        align-items: baseline;
        gap: 4px;
    }
    .with-undo span {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .with-undo button {
        font-size: 13px;
        padding: 0 2px;
        flex-shrink: 0;
    }
    p {
        margin: 0;
        font-size: 11px;
        line-height: 1.45;
    }
    .reason {
        color: var(--muted);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .outcome {
        white-space: normal;
    }
    .options {
        list-style: none;
        margin: 8px 0 0;
        padding: 0;
        display: grid;
        gap: 5px;
        /* The actions row always stays in the window; long options scroll instead. */
        flex-shrink: 1;
        min-height: 0;
        overflow-y: auto;
    }
    .options li {
        display: flex;
        align-items: center;
        gap: 8px;
        padding: 5px 8px;
        border-radius: 8px;
        background: var(--hover);
    }
    .options div {
        flex: 1;
        min-width: 0;
        display: grid;
        font-size: 11px;
        line-height: 1.3;
    }
    .options span {
        color: var(--muted);
        font-variant-numeric: tabular-nums;
    }
    .options strong {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .options .closes {
        color: var(--warning, var(--muted));
        display: -webkit-box;
        -webkit-line-clamp: 2;
        line-clamp: 2;
        -webkit-box-orient: vertical;
        overflow: hidden;
    }
    .options button {
        font-size: 11px;
        padding: 4px 9px;
        flex-shrink: 0;
    }
    .options .choices {
        flex: none;
        gap: 4px;
    }
    .options .danger {
        background: var(--danger);
        color: white;
    }
    .measured {
        margin-top: 4px;
        font-variant-numeric: tabular-nums;
        color: var(--muted);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .clamp {
        display: -webkit-box;
        -webkit-line-clamp: 3;
        line-clamp: 3;
        -webkit-box-orient: vertical;
    }
    .recording {
        margin-top: auto;
        height: 3px;
        border-radius: 2px;
        background: var(--hover);
        overflow: hidden;
    }
    .recording span {
        display: block;
        width: 100%;
        height: 100%;
        background: var(--accent);
        transform-origin: left;
        animation: record 10s linear forwards;
    }
    @keyframes record {
        from {
            transform: scaleX(0);
        }
        to {
            transform: scaleX(1);
        }
    }
    .proposal {
        margin-top: 4px;
        display: -webkit-box;
        -webkit-line-clamp: 2;
        line-clamp: 2;
        -webkit-box-orient: vertical;
        overflow: hidden;
    }
    .note,
    .error {
        margin-top: 4px;
        font-size: 10px;
        color: var(--muted);
    }
    .error {
        color: var(--danger);
        display: -webkit-box;
        -webkit-line-clamp: 2;
        line-clamp: 2;
        -webkit-box-orient: vertical;
        overflow: hidden;
    }
    .actions {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 6px;
        margin-top: auto;
        padding-top: 8px;
    }
    .actions button {
        font-size: 11px;
        padding: 4px 9px;
    }
    .links {
        display: flex;
        margin-left: auto;
    }
    .links button {
        padding: 4px 6px;
    }
    @keyframes arrive {
        from {
            opacity: 0;
            transform: translateY(-6px);
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
        .action-popup,
        .working,
        .recording span {
            animation: none;
        }
    }
</style>
