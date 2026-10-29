<script lang="ts">
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage } from '../lib/format';
    import type { ChatMessage, Reply, Snapshot, Workload } from '../lib/types';

    let {
        snapshot,
        workload,
        desktop,
        onclose,
        onusage,
    }: {
        snapshot: Snapshot;
        workload: Workload | null;
        desktop: boolean;
        onclose: () => void;
        onusage: () => void;
    } = $props();
    let question = $state('Why is my computer struggling?');
    let tier = $state('auto');
    let busy = $state(false);
    let error = $state('');
    let conversation: { message: ChatMessage; reply?: Reply }[] = $state([]);
    let previousContext = $state('');

    $effect(() => {
        const context = `${snapshot.source}:${workload?.id ?? 'system'}`;
        if (context !== previousContext) {
            conversation = [];
            previousContext = context;
            question = workload
                ? 'Explain this workload and suggest one useful next step.'
                : 'Why is my computer struggling?';
        }
    });

    async function send() {
        if (busy || !question.trim()) return;
        busy = true;
        error = '';
        const content = question.trim();
        const requestContext = previousContext;
        conversation = [...conversation, { message: { role: 'user', content } }];
        question = '';
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
                      }
                    : await invoke('ask', { messages, workloadId: workload?.id ?? null, tier });
            onusage();
            if (requestContext !== previousContext) return;
            conversation = [
                ...conversation,
                { message: { role: 'assistant' as const, content: reply.text }, reply },
            ].slice(-10);
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
            <h2>{workload?.label ?? 'Let’s understand the pressure'}</h2>
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
            Models receive anonymized workload metrics and your messages. Process names, project
            paths, and transcripts are excluded from telemetry sent to models. No actions run from
            chat.
        </p>
    </div>
    <div class="conversation" aria-live="polite" aria-busy={busy}>
        {#if !conversation.length}<div class="chat-welcome">
                <span aria-hidden="true">✳</span>
                <h3>A calmer look at what’s happening.</h3>
                <p>Ask for a quick explanation, deeper analysis, or an observation-only summary.</p>
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
                            : 'Observed · deterministic rules'}
                </div>
                <p>{entry.message.content}</p>
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
            placeholder="Ask about this workload…"
            rows="3"
            maxlength="2000"
            bind:value={question}
            disabled={busy}></textarea>
        <div class="form-actions">
            <select aria-label="Reasoning tier" bind:value={tier} disabled={busy}
                ><option value="auto">Automatic routing</option><option value="rules"
                    >Observations only · no model</option
                ><option value="cheap">Cheap tier</option><option value="intelligent"
                    >Intelligent tier</option
                ></select
            ><button class="primary" disabled={busy || !question.trim()}
                >{busy ? 'Working…' : 'Ask Jitterbug ↑'}</button
            >
        </div>
    </form>
</aside>
