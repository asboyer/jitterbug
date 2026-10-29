<script lang="ts">
    import { tick } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage } from '../lib/format';
    import Investigation from './Investigation.svelte';
    import ActionResult from './ActionResult.svelte';
    import type {
        ActionReport,
        ChatMessage,
        ProcessRecord,
        Reply,
        Snapshot,
        Target,
        Workload,
    } from '../lib/types';

    let {
        snapshot,
        workload,
        desktop,
        onclose,
        onusage,
        onaction,
        actionReport,
        actionBusy,
    }: {
        snapshot: Snapshot;
        workload: Workload | null;
        desktop: boolean;
        onclose: () => void;
        onusage: () => void;
        onaction: (process: ProcessRecord, action: string) => void;
        actionReport: ActionReport | null;
        actionBusy: boolean;
    } = $props();
    let question = $state('Why is my computer struggling?');
    let tier = $state('auto');
    let busy = $state(false);
    let error = $state('');
    let conversation: { message: ChatMessage; reply?: Reply }[] = $state([]);
    let previousContext = $state('');
    let focused: Target | null = $state(null);
    let conversationElement: HTMLDivElement;

    async function revealLatest() {
        await tick();
        const latest = conversationElement?.querySelector<HTMLElement>(
            '.chat-message:last-of-type'
        );
        if (latest) conversationElement.scrollTop = latest.offsetTop;
    }

    $effect(() => {
        if (actionReport) {
            void tick().then(() => {
                const result = conversationElement?.querySelector<HTMLElement>(
                    '[aria-label="Action measurement"]'
                );
                if (result) conversationElement.scrollTop = result.offsetTop;
            });
        }
    });

    $effect(() => {
        const context = `${snapshot.source}:${workload?.id ?? 'system'}`;
        if (context !== previousContext) {
            conversation = [];
            focused = null;
            previousContext = context;
            question = workload
                ? `Why is ${workload.label} using these resources? What can we improve?`
                : 'Why is my computer struggling?';
        }
    });

    async function send(choice?: Target) {
        if (busy || (!choice && !question.trim())) return;
        busy = true;
        error = '';
        const content = choice
            ? choice.pid
                ? `Investigate PID ${choice.pid}. What can we improve?`
                : 'Investigate this selected workload. What can we improve?'
            : question.trim();
        const requestContext = previousContext;
        conversation = [...conversation, { message: { role: 'user', content } }];
        question = '';
        void revealLatest();
        try {
            // Rules-only replies can contain private process/project labels. Keep them on-device.
            let messages = conversation
                .filter((entry) => entry.message.role === 'user' || entry.reply?.source === 'model')
                .slice(-8)
                .map((entry) => ({
                    ...entry.message,
                    content: entry.message.content.slice(0, 2000),
                }));
            while (
                messages.length > 1 &&
                new TextEncoder().encode(JSON.stringify(messages)).length > 5500
            )
                messages = messages.slice(1);
            const reply: Reply =
                !desktop || snapshot.source === 'demo'
                    ? {
                          text: 'This is a mocked explanation of synthetic data. The sample project is doing build work. In a real session, Jitterbug would compare sustained CPU use, available memory, swap growth, and process ancestry before suggesting a next step. No model was called and no process was changed.',
                          source: 'demo',
                          provider: null,
                          model: null,
                          tier: 'demo',
                          route: ['Synthetic demonstration only.'],
                          estimated_cost_usd: 0,
                          investigation: null,
                          choices: [],
                      }
                    : await invoke('ask', {
                          messages,
                          workloadId:
                              choice?.workload_id ?? focused?.workload_id ?? workload?.id ?? null,
                          tier,
                      });
            onusage();
            if (requestContext !== previousContext) return;
            focused = reply.investigation?.target ?? null;
            conversation = [
                ...conversation,
                { message: { role: 'assistant' as const, content: reply.text }, reply },
            ].slice(-10);
            void revealLatest();
        } catch (cause) {
            if (requestContext === previousContext) error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
</script>

<aside class="assistant panel" aria-label="Contextual assistant">
    <div class="section-title">
        <div>
            <p class="eyebrow">JITTERBUG ASSISTANT</p>
            <h2>{focused?.label ?? workload?.label ?? 'Ask about an app or process'}</h2>
        </div>
        <button class="icon-button" aria-label="Close assistant" onclick={onclose} disabled={busy}
            >×</button
        >
    </div>
    <div class="context-note">
        <span class="status-dot"></span>
        {snapshot.source === 'demo'
            ? 'Synthetic context · mocked replies'
            : 'Live context · no automatic model calls'}
        <p>
            For a specific investigation, models receive that app’s name, process names, metrics,
            recent history, and your messages. Paths, environment variables, transcripts, and
            unrelated app names stay out of telemetry sent to models. Actions require your
            confirmation.
        </p>
    </div>
    <div class="conversation" bind:this={conversationElement} aria-live="polite" aria-busy={busy}>
        {#if !conversation.length}<div class="chat-welcome">
                <span aria-hidden="true">✳</span>
                <h3>What is slowing you down?</h3>
                <p>
                    Try “Why is Spotify using so much CPU?” or “Investigate PID 123.” I’ll inspect
                    the evidence, suggest a test or supported action, and measure changes.
                </p>
            </div>{/if}
        {#each conversation as entry}
            <article class:user-message={entry.message.role === 'user'} class="chat-message">
                <div class="message-label">
                    {entry.message.role === 'user'
                        ? 'You'
                        : entry.reply?.source === 'model'
                          ? `Model · ${entry.reply.provider} / ${entry.reply.model}`
                          : entry.reply?.source === 'demo'
                            ? 'MOCKED · demo explanation'
                            : entry.reply?.source === 'guidance'
                              ? 'Jitterbug · no model call'
                              : 'Observed · deterministic rules'}
                </div>
                <p>{entry.message.content}</p>
                {#if entry.reply?.investigation}
                    <Investigation
                        evidence={entry.reply.investigation}
                        disabled={busy || actionBusy || snapshot.source !== 'live'}
                        {onaction}
                    />
                {/if}
                {#if entry.reply?.choices.length}
                    <div class="process-actions">
                        {#each entry.reply.choices as choice}
                            <button disabled={busy} onclick={() => void send(choice)}
                                >{choice.label}</button
                            >
                        {/each}
                    </div>
                {/if}
                {#if entry.reply?.route.length}<details>
                        <summary>Routing details</summary>
                        <ul>
                            {#each entry.reply.route as step}<li>{step}</li>{/each}
                        </ul>
                    </details>{/if}
                {#if entry.reply?.source === 'model'}<small
                        >Estimated token cost: ${entry.reply.estimated_cost_usd.toFixed(5)} · recommendations
                        may be wrong; review evidence.</small
                    >{/if}
            </article>
        {/each}
        {#if actionReport}<ActionResult report={actionReport} />{/if}
        {#if actionBusy}<p class="muted">
                Applying the confirmed action and waiting for a fresh measurement…
            </p>{/if}
        {#if busy}<p class="muted">Checking policy and asking one model at a time…</p>{/if}
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <form
        class="chat-form"
        onsubmit={(event) => {
            event.preventDefault();
            void send();
        }}
    >
        <label for="question" class="sr-only">Ask Jitterbug</label><textarea
            id="question"
            placeholder="Why is Spotify using so much CPU?"
            rows="3"
            maxlength="2000"
            bind:value={question}
            disabled={busy}></textarea>
        <div class="form-actions">
            <select aria-label="Reasoning tier" bind:value={tier} disabled={busy}
                ><option value="auto">Automatic routing</option><option value="rules"
                    >Observations only · no model</option
                ><option value="cheap">Cheap tier · definitions</option><option value="intelligent"
                    >Intelligent tier</option
                ></select
            ><button class="primary" disabled={busy || !question.trim()}
                >{busy ? 'Working…' : 'Ask Jitterbug ↑'}</button
            >
        </div>
    </form>
</aside>
