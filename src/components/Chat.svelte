<script lang="ts">
    import { tick } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage } from '../lib/format';
    import Investigation from './Investigation.svelte';
    import InsightCard from './InsightCard.svelte';
    import InfoTip from './InfoTip.svelte';
    import Icon from './Icon.svelte';
    import ActionResult from './ActionResult.svelte';
    import type {
        ActionReport,
        AutomationCase,
        ChatMessage,
        ProcessRecord,
        Reply,
        Snapshot,
        Target,
        Workload,
    } from '../lib/types';

    let {
        snapshot,
        incident,
        workload,
        questionProcess = null,
        initialReply = null,
        desktop,
        onclose,
        onusage,
        onaction,
        actionReport,
        actionBusy,
    }: {
        snapshot: Snapshot;
        incident: AutomationCase | null;
        workload: Workload | null;
        questionProcess?: ProcessRecord | null;
        initialReply?: Reply | null;
        desktop: boolean;
        onclose: () => void;
        onusage: () => void;
        onaction: (process: ProcessRecord, action: string) => void;
        actionReport: ActionReport | null;
        actionBusy: boolean;
    } = $props();
    let question = $state('');
    let tier = $state('auto');
    let busy = $state(false);
    let error = $state('');
    let conversation: { message: ChatMessage; reply?: Reply; nativeSummary?: boolean }[] = $state(
        []
    );
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
        const context = `${snapshot.source}:${workload?.id ?? 'system'}:${incident?.id ?? ''}:${questionProcess?.pid ?? ''}:${questionProcess?.start_time ?? ''}:${initialReply?.text ?? ''}`;
        if (context !== previousContext) {
            conversation = initialReply
                ? [
                      {
                          message: { role: 'assistant', content: initialReply.text },
                          reply: initialReply,
                      },
                  ]
                : [];
            if (incident?.diagnosis) {
                conversation.push({
                    message: { role: 'assistant', content: incident.diagnosis.text },
                    reply: incident.diagnosis,
                });
            }
            if (incident) {
                // Only fixed native summary text enters context; never serialize the raw case,
                // workload label, or its private evidence as a chat message.
                const content =
                    incident.verification?.summary ??
                    (incident.report || incident.steps.some((step) => step.stage === 'verifying')
                        ? 'A priority change was applied; its resource effect has not been verified.'
                        : 'No optimization was applied for this incident.');
                conversation.push({
                    message: { role: 'assistant', content },
                    nativeSummary: true,
                    reply: {
                        text: content,
                        source: 'guidance',
                        tier: 'rules',
                        provider: null,
                        model: null,
                        route: ['Native incident result; no model call.'],
                        estimated_cost_usd: 0,
                        investigation: null,
                        choices: [],
                    },
                });
            }
            focused = null;
            previousContext = context;
            question = '';
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
        const anchoredProcess =
            !choice &&
            questionProcess &&
            (!focused ||
                (focused.pid === questionProcess.pid &&
                    focused.workload_id === questionProcess.workload_id))
                ? questionProcess
                : null;
        const requestContext = previousContext;
        conversation = [...conversation, { message: { role: 'user', content } }];
        question = '';
        void revealLatest();
        try {
            // Rules-only replies can contain private process/project labels. Keep them on-device.
            let messages = conversation
                .filter(
                    (entry) =>
                        entry.message.role === 'user' ||
                        entry.reply?.source === 'model' ||
                        entry.nativeSummary
                )
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
                          text: 'The sample project is running a build. This is synthetic data; no model was called.',
                          insight: {
                              headline: 'A build is using the CPU',
                              finding:
                                  'This demo shows development work running alongside your apps.',
                              next_step:
                                  'Let the build finish, or manage this workload to schedule heavy checks.',
                          },
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
                          pid: anchoredProcess?.pid ?? null,
                          startTime: anchoredProcess?.start_time ?? null,
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
        <div class="assistant-title">
            <span class="assistant-mark"><Icon name="companion" size={24} /></span>
            <div>
                <p class="eyebrow">Jitterbug insights</p>
                <h2>{focused?.label ?? workload?.label ?? 'Your computer'}</h2>
            </div>
        </div>
        <button class="icon-button" aria-label="Close assistant" onclick={onclose} disabled={busy}
            >×</button
        >
    </div>
    <div class="context-line">
        <span class="status-dot"></span><span
            >{snapshot.source === 'demo'
                ? 'Demo · synthetic readings'
                : questionProcess
                  ? `Selected process · PID ${questionProcess.pid}`
                  : workload
                    ? 'Selected workload · live readings'
                    : 'Live readings'}</span
        ><InfoTip label="Privacy and model settings"
            ><p>
                Models receive the selected app’s name, process names, measurements, recent history
                and your question. Paths, environment variables and transcripts stay on this device.
                Actions require confirmation.
            </p>
            <label
                >Reasoning tier<select aria-label="Reasoning tier" bind:value={tier} disabled={busy}
                    ><option value="auto">Automatic</option><option value="rules"
                        >Observations only</option
                    ><option value="cheap">Definitions</option><option value="intelligent"
                        >Intelligent</option
                    ></select
                ></label
            ></InfoTip
        >
    </div>
    <div class="conversation" bind:this={conversationElement} aria-live="polite" aria-busy={busy}>
        {#if !conversation.length}<div class="chat-welcome">
                <h3>{workload ? 'What would help?' : 'Let’s find what needs attention.'}</h3>
                <p>
                    {workload
                        ? 'This workload is already selected.'
                        : 'Check the whole computer, or ask about an app.'}
                </p>
                <div class="quick-questions">
                    {#each workload ? ['Does this need attention?', 'What is it doing?', 'How can I reduce its impact?'] : ['What needs attention?', 'What is using the most memory?'] as prompt}<button
                            disabled={busy}
                            onclick={() => {
                                question = prompt;
                                void send();
                            }}>{prompt}<span aria-hidden="true">↗</span></button
                        >{/each}
                </div>
            </div>{/if}
        {#each conversation as entry}<article
                class:user-message={entry.message.role === 'user'}
                class="chat-message"
            >
                {#if entry.message.role === 'user'}<p>{entry.message.content}</p>
                {:else if entry.reply}<InsightCard reply={entry.reply} />
                    {#if entry.reply.investigation}<Investigation
                            evidence={entry.reply.investigation}
                            disabled={busy || actionBusy || snapshot.source !== 'live'}
                            {onaction}
                        />{/if}
                    {#if entry.reply.choices.length}<div class="quick-questions">
                            {#each entry.reply.choices as choice}<button
                                    disabled={busy}
                                    onclick={() => void send(choice)}>{choice.label}</button
                                >{/each}
                        </div>{/if}
                {:else}<p>{entry.message.content}</p>{/if}
            </article>{/each}
        {#if actionReport}<ActionResult report={actionReport} />{/if}
        {#if actionBusy}<p class="muted">Applying your change and measuring the result…</p>{/if}
        {#if busy}<p class="muted" role="status">Checking current activity…</p>{/if}
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
            placeholder={workload
                ? `Ask about ${workload.label}…`
                : 'Ask about an app or your computer…'}
            rows="2"
            maxlength="2000"
            bind:value={question}
            disabled={busy}></textarea>
        <div class="form-actions">
            <span class="muted"
                >{tier === 'auto'
                    ? 'Automatic routing'
                    : tier === 'rules'
                      ? 'No model'
                      : `${tier} tier`}</span
            ><button class="primary" disabled={busy || !question.trim()}
                >{busy ? 'Checking…' : 'Ask ↑'}</button
            >
        </div>
    </form>
</aside>

<style>
    .assistant {
        padding: 20px;
        width: 390px;
    }
    .assistant-title {
        display: flex;
        gap: 10px;
        align-items: center;
        min-width: 0;
    }
    .assistant-mark {
        display: grid;
        place-items: center;
        width: 38px;
        height: 38px;
        flex-shrink: 0;
        background: var(--accent-soft);
        color: var(--accent);
        border-radius: 12px;
    }
    .eyebrow {
        font-size: 10px;
        text-transform: none;
        letter-spacing: 0;
        margin: 0 0 4px;
    }
    h2 {
        font-size: 15px;
        margin: 0;
    }
    .context-line {
        display: flex;
        align-items: center;
        gap: 8px;
        color: var(--muted);
        font-size: 10px;
        padding: 10px 0;
        border-bottom: 1px solid var(--border);
    }
    .context-line > span:nth-child(2) {
        flex: 1;
    }
    .context-line select {
        width: 100%;
        margin-top: 8px;
    }
    .chat-welcome {
        text-align: left;
        padding: 10px 0;
    }
    .chat-welcome h3 {
        font-size: 18px;
        letter-spacing: -0.3px;
    }
    .chat-welcome p {
        line-height: 1.6;
    }
    .quick-questions {
        display: grid;
        gap: 9px;
        margin-top: 22px;
    }
    .quick-questions button {
        display: flex;
        align-items: center;
        justify-content: space-between;
        text-align: left;
        gap: 12px;
        padding: 12px;
        font-size: 12px;
        border-radius: 8px;
        background: transparent;
    }
    .quick-questions button span {
        color: var(--muted);
    }
    .chat-message {
        padding: 0 0 22px;
        border: 0;
        border-bottom: 1px solid var(--border);
        border-radius: 0;
        margin-bottom: 22px;
    }
    .chat-message.user-message {
        padding: 10px 12px;
        border: 0;
        border-radius: 9px;
        background: var(--hover);
        margin-bottom: 12px;
    }
    .user-message p {
        font-size: 12px;
        line-height: 1.5;
    }
    .form-actions {
        align-items: center;
        font-size: 10px;
    }
    .chat-form textarea {
        border-radius: 9px;
        padding: 12px;
        line-height: 1.5;
        resize: vertical;
    }
</style>
