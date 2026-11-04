<script module lang="ts">
    type Entry = { role: 'user' | 'assistant'; text: string; answer?: Answer };
    type Answer = {
        text: string;
        sources: { title: string; url: string }[];
        action: { process: ProcessRecord; action: string; reason: string } | null;
        provider: string;
        model: string;
        captured_at: number;
        contributors?: ProcessRecord[];
        inside?: InsideView | null;
    };
    const conversations = new Map<string, Entry[]>();
</script>

<script lang="ts">
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import { onDestroy, tick, untrack } from 'svelte';
    import type {
        Snapshot,
        Workload,
        ProcessRecord,
        WorkloadDetails,
        AppProfile,
        ActionReport,
        InsideView,
        ProcessLabel,
    } from '../lib/types';
    import { demoDetails } from '../lib/demo';
    import { queueInference, cancelled, identityKey, type Explanation } from '../lib/monitor';
    import { appFallback, processRole } from '../lib/monitor-copy';
    import { memory, percent, errorMessage } from '../lib/format';
    import AppIcon from './AppIcon.svelte';
    import Icon from './Icon.svelte';
    import InsidePanel from './InsidePanel.svelte';
    let {
        snapshot,
        workload,
        initialProcess,
        settingsKey,
        actionBusy,
        actionReport,
        onclose,
        onusage,
        onaction,
    }: {
        snapshot: Snapshot;
        workload: Workload | null;
        initialProcess: ProcessRecord | null;
        settingsKey: string;
        actionBusy: boolean;
        actionReport: ActionReport | null;
        onclose: () => void;
        onusage: () => void;
        onaction: (process: ProcessRecord, action: string) => void;
    } = $props();
    let selected = $state<{ pid: number; start_time: number } | null>(
        untrack(() => initialProcess)
    );
    let detail = $state<WorkloadDetails | null>(null);
    let error = $state('');
    let tab = $state('chat');
    let processSearch = $state('');
    let labels = $state<Record<string, ProcessLabel>>({});
    function named(view: InsideView) {
        if (view.workload_id === workload?.id)
            labels = Object.fromEntries(
                view.labels.map((label) => [`${label.pid}:${label.start_time}`, label])
            );
    }
    function label(p: ProcessRecord) {
        return labels[`${p.pid}:${p.start_time}`];
    }
    let profile = $state<AppProfile | null>(null);
    let profileError = $state('');
    let describing = $state(false);
    let conversation = $state<Entry[]>([]);
    let question = $state('');
    let pendingContext = $state<string | null>(null);
    let chatError = $state('');
    let log: HTMLDivElement | undefined = $state();
    let input: HTMLTextAreaElement | undefined = $state();
    let sequence = 0;
    let disposed = false;
    let lastProfile = '';
    let previousContext = '';
    const demo = $derived(snapshot.source !== 'live' || !isTauri());
    const context = $derived(
        `${snapshot.source}:${settingsKey}:${workload?.id ?? 'system'}:${selected?.pid ?? 'app'}:${selected?.start_time ?? ''}`
    );
    const busy = $derived(pendingContext === context);
    const process = $derived.by(() => {
        const identity = selected;
        if (!identity) return null;
        return (
            [detail?.process, ...(detail?.members ?? []), initialProcess].find(
                (p) => p?.pid === identity.pid && p?.start_time === identity.start_time
            ) ?? null
        );
    });
    const readings = $derived(process ?? detail?.workload ?? workload);
    const title = $derived(
        (process ? label(process)?.title : null) ??
            process?.name ??
            workload?.label ??
            'Your computer'
    );
    const summary = $derived(
        detail?.launch && !selected
            ? detail.launch.summary
            : profile?.source === 'model'
              ? profile.text
              : process && workload
                ? processRole(process, workload.label)
                : profile?.source === 'reference'
                  ? profile.text
                  : workload
                    ? appFallback(workload, demo)
                    : 'Ask about resource use, slowdowns, or which apps need attention.'
    );
    const result = $derived(
        actionReport &&
            (!workload || actionReport.before.workload_id === workload.id) &&
            (!selected ||
                (actionReport.before.pid === selected.pid &&
                    actionReport.before.start_time === selected.start_time))
            ? actionReport
            : null
    );
    function remember() {
        if (!previousContext) return;
        conversations.delete(previousContext);
        conversations.set(previousContext, JSON.parse(JSON.stringify(conversation.slice(-12))));
        while (conversations.size > 12) conversations.delete(conversations.keys().next().value!);
    }
    onDestroy(() => {
        remember();
        disposed = true;
        sequence++;
    });
    $effect(() => {
        const key = context;
        untrack(() => {
            if (key !== previousContext) {
                remember();
                conversation = conversations.get(key) ?? [];
                question = '';
                chatError = '';
                previousContext = key;
            }
        });
    });
    $effect(() => {
        const sample = snapshot;
        const target = selected;
        const id = workload?.id;
        if (!id) return;
        const request = ++sequence;
        void (
            demo
                ? Promise.resolve(demoDetails(sample, id, target))
                : invoke<WorkloadDetails>('get_workload_details', {
                      workloadId: id,
                      pid: target?.pid ?? null,
                      startTime: target?.start_time ?? null,
                  })
        )
            .then((next) => {
                if (disposed || request !== sequence) return;
                detail = next;
                error = '';
                const key = `${settingsKey}:${next.profile_key ?? id}:${next.process.pid}:${next.process.start_time}`;
                if (key !== lastProfile) {
                    lastProfile = key;
                    profile = next.profile;
                    profileError = '';
                    void describe(next, key);
                } else if (next.profile.source === 'model') profile = next.profile;
            })
            .catch((cause) => {
                if (!disposed && request === sequence) error = errorMessage(cause);
            });
    });
    async function describe(next: WorkloadDetails, key: string) {
        if (demo || next.profile.source === 'model') return;
        describing = true;
        const current = () => !disposed && key === lastProfile;
        const valid = () => current() && !busy && !conversation.length;
        try {
            const values = await queueInference(
                () =>
                    invoke<Explanation[]>('explain_monitor_profiles', {
                        identities: [
                            {
                                workload_id: next.workload.id,
                                pid: next.process.pid,
                                start_time: next.process.start_time,
                            },
                        ],
                        generate: true,
                    }),
                5,
                valid
            );
            if (valid() && values[0]) profile = values[0].profile;
            onusage();
        } catch (cause) {
            if (
                valid() &&
                cause !== cancelled &&
                !/deferred for your question/.test(errorMessage(cause))
            )
                profileError = errorMessage(cause);
        } finally {
            if (current()) describing = false;
        }
    }
    async function reveal() {
        await tick();
        if (log) log.scrollTop = log.scrollHeight;
    }
    async function send(suggestion?: string) {
        if (busy || error || !(suggestion ?? question).trim()) return;
        const text = (suggestion ?? question).trim();
        const key = context;
        const target = selected ? { ...selected } : null;
        const id = workload?.id ?? null;
        conversation = [...conversation, { role: 'user', text }];
        question = '';
        pendingContext = key;
        chatError = '';
        void reveal();
        let messages = conversation
            .slice(-8)
            .map((entry) => ({ role: entry.role, content: entry.text.slice(0, 1600) }));
        while (
            messages.length > 1 &&
            new TextEncoder().encode(JSON.stringify(messages)).length > 4000
        )
            messages = messages.slice(1);
        try {
            if (!demo) await invoke('prioritize_monitor_chat');
            const answer: Answer = demo
                ? {
                      text: process
                          ? `${process.name} is one of ${workload?.label}’s processes. ${processRole(process, workload?.label ?? 'the app')} In this example it uses ${memory(process.memory_bytes)} and ${percent(process.cpu_percent)} CPU. There’s no sign of sustained pressure.`
                          : `${summary} In this example, ${workload?.label ?? 'your computer'} has room to keep working. You can look at its individual processes to see which part is using memory.`,
                      sources: [],
                      action: null,
                      provider: 'demo',
                      model: 'demo',
                      captured_at: snapshot.captured_at,
                  }
                : await queueInference(
                      () =>
                          invoke<Answer>('ask_monitor', {
                              messages,
                              workloadId: id,
                              pid: target?.pid ?? null,
                              startTime: target?.start_time ?? null,
                          }),
                      20,
                      () => !disposed && key === context
                  );
            if (!disposed && key === context) {
                if (answer.inside) named(answer.inside);
                conversation = [
                    ...conversation,
                    { role: 'assistant' as const, text: answer.text, answer },
                ].slice(-12);
                remember();
                void reveal();
            }
            if (!demo) onusage();
        } catch (cause) {
            if (!disposed && key === context && cause !== cancelled)
                chatError = errorMessage(cause);
        } finally {
            if (pendingContext === key) pendingContext = null;
            void tick().then(() => input?.focus({ preventScroll: true }));
        }
    }
    function currentAction(answer: Answer) {
        const action = answer.action;
        if (!action || demo || error || actionBusy) return null;
        return (
            detail?.members.find(
                (p) =>
                    p.pid === action.process.pid &&
                    p.start_time === action.process.start_time &&
                    p.actions.includes(action.action)
            ) ?? null
        );
    }
    function choose(p: ProcessRecord | null) {
        lastProfile = '';
        profile = null;
        describing = false;
        error = '';
        selected = p ? { pid: p.pid, start_time: p.start_time } : null;
        tab = 'chat';
    }
</script>

<div class="inspector-shell">
    <header class="inspector-head">
        <div class="heading">
            {#if workload}<AppIcon
                    workloadId={workload.id}
                    category={workload.category}
                    size={34}
                    {demo}
                />{:else}<Icon name="companion" size={28} />{/if}
            <div>
                <h2 {title}>{title}</h2>
                <span
                    >{process
                        ? `${workload?.label} · PID ${process.pid}`
                        : workload
                          ? `${detail?.member_count ?? workload.process_count} processes`
                          : 'Jitterbug assistant'}</span
                >
            </div>
        </div>
        <button class="icon-button" aria-label="Close inspector" onclick={onclose}>×</button>
    </header>
    <section class="identity-summary" aria-label="App summary">
        <p>{summary}</p>
        {#if readings}<div class="quick-readings">
                <span><strong>{percent(readings.cpu_percent)}</strong> CPU</span><span
                    ><strong>{memory(readings.memory_bytes)}</strong> memory</span
                >{#if process}<button class="text-button" onclick={() => choose(null)}
                        >View whole app</button
                    >{/if}
            </div>{/if}
        <div class="summary-meta">
            <button class="text-button" onclick={() => (tab = 'details')}
                >{profile?.source === 'model'
                    ? profile.sources?.length
                        ? 'AI summary · sources'
                        : 'AI summary'
                    : demo
                      ? 'Demo'
                      : 'About this app'} ↗</button
            >{#if profileError}<button
                    class="text-button"
                    title={profileError}
                    onclick={() => {
                        if (detail) void describe(detail, lastProfile);
                    }}>Retry summary</button
                >{:else if describing}<span>Updating summary…</span>{/if}
        </div>
    </section>
    {#if workload}<nav class="inspector-tabs" aria-label="Inspector views">
            <button class:active={tab === 'chat'} onclick={() => (tab = 'chat')}>Chat</button
            ><button class:active={tab === 'processes'} onclick={() => (tab = 'processes')}
                >Processes <span>{detail?.member_count ?? workload.process_count}</span></button
            ><button class:active={tab === 'details'} onclick={() => (tab = 'details')}
                >Details</button
            >
        </nav>{/if}
    {#if error}<p class="inline-error" role="status">{error}</p>{/if}
    {#if tab === 'chat'}
        <!-- A focusable log lets keyboard users scroll the conversation. -->
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div
            class="conversation"
            role="log"
            aria-label="Process conversation"
            tabindex="0"
            bind:this={log}
        >
            {#if !conversation.length}<div class="conversation-start">
                    <div class="suggestions">
                        {#each workload ? ['Why is it using this much memory?', 'Can I reduce its impact?'] : ['What’s using the most resources?', 'Why does my computer feel slow?'] as prompt}<button
                                disabled={!!error || busy}
                                onclick={() => void send(prompt)}>{prompt}<span>↗</span></button
                            >{/each}
                    </div>
                </div>{/if}
            {#each conversation as entry}<div class="message" class:user={entry.role === 'user'}>
                    {#if entry.role === 'assistant'}<span class="assistant-name"
                            >{demo ? 'Jitterbug · demo' : 'Jitterbug'}</span
                        >{/if}
                    <p>{entry.text}</p>
                    {#if entry.answer?.contributors?.length}<div class="contributors">
                            <div class="evidence-heading">
                                <span>Processes checked</span><button
                                    class="text-button"
                                    onclick={() => (tab = 'processes')}
                                    >See tabs & processes ↗</button
                                >
                            </div>
                            {#each entry.answer.contributors as p}
                                {@const known = entry.answer.inside?.labels.find(
                                    (l) => l.pid === p.pid && l.start_time === p.start_time
                                )}
                                {@const current = detail?.members.find(
                                    (member) =>
                                        member.pid === p.pid && member.start_time === p.start_time
                                )}
                                <button
                                    class="contributor"
                                    disabled={!current}
                                    onclick={() => {
                                        if (current) choose(current);
                                    }}
                                >
                                    <span
                                        ><strong>{known?.title ?? p.name}</strong><small
                                            >{known?.subtitle || `PID ${p.pid}`}</small
                                        ></span
                                    >
                                    <span>{memory(p.memory_bytes)}</span>
                                </button>
                            {/each}
                        </div>{/if}
                    {#if entry.answer?.sources?.length}<div class="answer-sources">
                            {#each entry.answer.sources.slice(0, 2) as source}<a
                                    href={source.url}
                                    target="_blank"
                                    rel="noreferrer">{source.title} ↗</a
                                >{/each}
                        </div>{/if}
                    {#if entry.answer?.action}{@const action = entry.answer.action}{@const current =
                            currentAction(entry.answer)}
                        <div class="action-proposal">
                            <p>{action.reason}</p>
                            <button
                                disabled={!current}
                                onclick={() => {
                                    if (current) onaction(current, action.action);
                                }}
                                >{action.action === 'lower_priority'
                                    ? 'Review background mode'
                                    : 'Review stopping process'} <span>↗</span></button
                            >
                        </div>{/if}
                </div>{/each}
            {#if busy}<div class="thinking" role="status"><i></i> Thinking…</div>{/if}
            {#if chatError}<div class="inline-error" role="alert">
                    {chatError}<button
                        class="text-button"
                        onclick={() => {
                            question =
                                conversation.filter((e) => e.role === 'user').at(-1)?.text ?? '';
                            input?.focus();
                        }}>Try again</button
                    >
                </div>{/if}
            {#if result}<div class="action-result" role="status">{result.message}</div>{/if}
        </div>
        <form
            class="composer"
            onsubmit={(event) => {
                event.preventDefault();
                void send();
            }}
        >
            <textarea
                bind:this={input}
                bind:value={question}
                aria-label="Message Jitterbug"
                placeholder={workload ? `Ask about ${workload.label}…` : 'Ask about your computer…'}
                rows="2"
                maxlength="1800"
                disabled={!!error}
                onkeydown={(event) => {
                    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
                        event.preventDefault();
                        void send();
                    }
                }}></textarea>
            <div>
                <span>{demo ? 'Demo · no model calls' : 'Actions require review'}</span><button
                    class="send"
                    aria-label="Send message"
                    disabled={busy || !!error || !question.trim()}>↑</button
                >
            </div>
        </form>
    {:else if tab === 'processes'}<div
            class="detail-scroll"
            role="group"
            aria-label="App processes"
        >
            <input
                class="process-search"
                aria-label="Filter processes"
                placeholder="Find a process…"
                bind:value={processSearch}
            />
            {#each (detail?.members ?? []).filter( (p) => `${p.name} ${p.pid} ${label(p)?.title ?? ''} ${label(p)?.subtitle ?? ''}`
                        .toLowerCase()
                        .includes(processSearch.toLowerCase()) ) as p (identityKey(p))}<button
                    class="member"
                    onclick={() => choose(p)}
                    ><span
                        ><strong>{label(p)?.title ?? p.name}</strong><small
                            >{label(p)?.subtitle || processRole(p, workload?.label ?? 'this app')} · PID
                            {p.pid}</small
                        ></span
                    ><span>{percent(p.cpu_percent)}<small>{memory(p.memory_bytes)}</small></span
                    ></button
                >{/each}{#if detail && detail.members.length < detail.member_count}<p class="muted">
                    Showing {detail.members.length} of {detail.member_count} observed processes.
                </p>{/if}{#if detail && workload}<InsidePanel
                    {snapshot}
                    workload={detail.workload}
                    {demo}
                    {actionBusy}
                    {onaction}
                    onview={named}
                />{/if}
        </div>
    {:else}<div class="detail-scroll technical">
            {#if profile?.sources?.length}<h3>Sources</h3>
                {#each profile.sources as source}<a
                        href={source.url}
                        target="_blank"
                        rel="noreferrer">{source.title} ↗</a
                    >{/each}{/if}
            {#if profile?.source === 'model'}<p class="muted">
                    Explained by {profile.provider} · {profile.model}{profile.generated_at
                        ? ` · ${new Date(profile.generated_at * 1000).toLocaleDateString()}`
                        : ''}
                </p>{/if}
            {#if detail}<h3>Observed origin</h3>
                <p>{detail.origin_summary}</p>
                {#if detail.launch}<p>{detail.launch.summary}</p>
                    <ul>
                        {#each detail.launch.evidence as evidence}<li>{evidence}</li>{/each}
                    </ul>{/if}
                <ol>
                    {#each detail.ancestry as node}<li>
                            <strong>{node.name}</strong><span>PID {node.pid}</span>
                        </li>{/each}
                </ol>
                <details>
                    <summary>Technical details</summary>{#each detail.ancestry.slice(-1) as node}<dl
                        >
                            <dt>Executable</dt>
                            <dd>{node.executable ?? 'Unavailable'}</dd>
                            <dt>Working folder</dt>
                            <dd>{node.working_directory ?? 'Unavailable'}</dd>
                            <dt>Started</dt>
                            <dd>{new Date(node.start_time * 1000).toLocaleString()}</dd>
                        </dl>{/each}
                    <p class="muted">{detail.origin_limit}</p>
                </details>{/if}
        </div>{/if}
</div>

<style>
    .process-search {
        width: 100%;
        margin-bottom: 12px;
    }
    .detail-scroll :global(.inside) {
        padding: 14px 8px;
        margin: 14px 0 0;
        border: 0;
        border-top: 1px solid var(--border);
        border-radius: 0;
        box-shadow: none;
    }
    .detail-scroll :global(.inside h2) {
        font-size: 13px;
    }
    .detail-scroll :global(.inside .muted) {
        font-size: 11px;
    }
    .detail-scroll :global(.unit) {
        grid-template-columns: minmax(0, 1fr) 48px 62px;
        gap: 8px;
        padding: 10px 0;
        font-size: 11px;
    }
    .detail-scroll :global(.unit-actions) {
        grid-column: 1 / -1;
        justify-content: flex-start;
        flex-wrap: wrap;
    }
    .detail-scroll :global(.unit-actions button) {
        font-size: 10px;
        padding: 4px 8px;
    }
    .inspector-shell {
        display: flex;
        flex-direction: column;
        height: 100%;
        min-height: 0;
    }
    .inspector-head {
        padding: 19px 20px 12px;
        display: flex;
        align-items: start;
        justify-content: space-between;
        gap: 8px;
        flex-shrink: 0;
    }
    .heading {
        display: flex;
        gap: 11px;
        align-items: center;
        min-width: 0;
    }
    .heading > div {
        min-width: 0;
    }
    h2 {
        font-size: 17px;
        margin: 0 0 4px;
        font-weight: 600;
        letter-spacing: -0.3px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .heading span {
        font-size: 10px;
        color: var(--muted);
    }
    .identity-summary {
        padding: 0 20px 12px;
        flex-shrink: 0;
    }
    .identity-summary > p {
        font-size: 12px;
        line-height: 1.6;
        margin: 0 0 12px;
    }
    .quick-readings {
        display: flex;
        gap: 18px;
        align-items: center;
        font-size: 11px;
        color: var(--muted);
        flex-wrap: wrap;
    }
    .quick-readings strong {
        color: var(--text);
        font-weight: 550;
        font-variant-numeric: tabular-nums;
    }
    .quick-readings button {
        padding: 0;
        font-size: 10px;
        min-height: 20px;
    }
    .summary-meta {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 8px;
        font-size: 10px;
        color: var(--muted);
        padding-top: 7px;
    }
    .summary-meta button {
        font-size: 10px;
        padding: 0;
        min-height: 18px;
    }
    .inspector-tabs {
        display: flex;
        padding: 0 20px;
        gap: 21px;
        border-bottom: 1px solid var(--border);
        flex-shrink: 0;
    }
    .inspector-tabs button {
        border: 0;
        border-bottom: 2px solid transparent;
        border-radius: 0;
        padding: 10px 0;
        background: none;
        box-shadow: none;
        color: var(--muted);
        font-size: 11px;
    }
    .inspector-tabs button.active {
        color: var(--text);
        border-bottom-color: var(--accent);
    }
    .inspector-tabs span {
        margin-left: 4px;
        color: var(--muted);
        font-size: 10px;
    }
    .conversation {
        flex: 1;
        min-height: 0;
        overflow: auto;
        padding: 20px;
        scrollbar-width: thin;
        overflow-wrap: anywhere;
    }
    .conversation-start {
        margin: 0;
    }
    .suggestions {
        display: flex;
        flex-direction: column;
        gap: 5px;
    }
    .suggestions button {
        display: flex;
        justify-content: space-between;
        gap: 8px;
        text-align: left;
        padding: 8px 10px;
        font-size: 11px;
        background: none;
        border: 1px solid var(--border);
        border-radius: 8px;
        box-shadow: none;
        font-weight: 400;
    }
    .suggestions span {
        color: var(--muted);
    }
    .message {
        margin-bottom: 22px;
        font-size: 13px;
        line-height: 1.75;
    }
    .message p {
        white-space: pre-wrap;
        margin: 6px 0 0;
    }
    .assistant-name {
        font-weight: 600;
        font-size: 11px;
    }
    .message.user {
        background: var(--hover);
        margin: 0 0 22px 28px;
        padding: 10px 13px;
        border-radius: 11px 11px 3px 11px;
    }
    .message.user p {
        margin: 0;
    }
    .contributors {
        margin-top: 16px;
        border-top: 1px solid var(--border);
        padding-top: 9px;
    }
    .evidence-heading {
        display: flex;
        justify-content: space-between;
        gap: 8px;
        align-items: center;
        font-size: 10px;
        color: var(--muted);
    }
    .evidence-heading button {
        font-size: 10px;
        padding: 0;
    }
    .contributor {
        display: flex;
        justify-content: space-between;
        gap: 12px;
        width: 100%;
        text-align: left;
        border: 0;
        box-shadow: none;
        background: none;
        padding: 8px 0;
        border-radius: 0;
        font-size: 11px;
    }
    .contributor > span:first-child {
        min-width: 0;
    }
    .contributor > span:last-child {
        flex-shrink: 0;
        font-variant-numeric: tabular-nums;
    }
    .contributor strong,
    .contributor small {
        display: block;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .contributor strong {
        font-weight: 500;
    }
    .contributor small {
        color: var(--muted);
        font-size: 10px;
        margin-top: 3px;
    }
    .answer-sources {
        font-size: 10px;
        margin-top: 10px;
    }
    .answer-sources a {
        display: block;
        color: var(--muted);
        overflow-wrap: anywhere;
    }
    .thinking {
        display: flex;
        align-items: center;
        gap: 8px;
        font-size: 11px;
        color: var(--muted);
        padding: 4px 0 14px;
    }
    .thinking i {
        height: 6px;
        width: 6px;
        border-radius: 50%;
        background: var(--accent);
    }
    .composer {
        margin: 0 14px 14px;
        padding: 8px 10px 6px;
        border: 1px solid var(--control-border);
        border-radius: 10px;
        background: var(--control);
        flex-shrink: 0;
    }
    .composer:focus-within {
        border-color: var(--accent);
    }
    textarea {
        display: block;
        width: 100%;
        border: none;
        background: transparent;
        box-shadow: none !important;
        font-size: 12px;
        padding: 4px;
        resize: none;
        min-height: 42px;
        line-height: 1.5;
    }
    .composer > div {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 8px;
    }
    .composer span {
        color: var(--muted);
        font-size: 9px;
    }
    .send {
        background: var(--accent);
        color: white;
        border: 0;
        border-radius: 7px;
        min-height: 26px;
        height: 26px;
        width: 27px;
        padding: 0;
        font-size: 16px;
    }
    .detail-scroll {
        flex: 1;
        min-height: 0;
        overflow: auto;
        padding: 12px;
        scrollbar-width: thin;
    }
    .member {
        width: 100%;
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 14px;
        padding: 12px 8px;
        border: 0;
        border-bottom: 1px solid var(--border);
        border-radius: 0;
        background: none;
        box-shadow: none;
        font-size: 11px;
        text-align: left;
    }
    .member > span:first-child {
        min-width: 0;
    }
    .member strong {
        display: block;
        overflow: hidden;
        white-space: nowrap;
        text-overflow: ellipsis;
        font-weight: 500;
    }
    .member small {
        display: block;
        font-size: 10px;
        color: var(--muted);
        margin-top: 5px;
    }
    .member > span:last-child {
        text-align: right;
        white-space: nowrap;
        font-variant-numeric: tabular-nums;
    }
    .technical {
        padding: 20px;
        font-size: 12px;
        line-height: 1.6;
    }
    .technical h3 {
        font-size: 12px;
        margin: 0 0 10px;
    }
    .technical a {
        display: block;
        color: var(--accent);
        margin: 8px 0;
        overflow-wrap: anywhere;
    }
    .technical p {
        margin: 12px 0;
    }
    .technical ol {
        list-style: none;
        padding: 0 0 0 12px;
        border-left: 1px solid var(--border);
    }
    .technical li {
        padding: 6px 0;
        display: flex;
        flex-wrap: wrap;
        gap: 12px;
        font-size: 11px;
    }
    .technical li span {
        color: var(--muted);
    }
    .technical details {
        margin-top: 20px;
    }
    .technical dd {
        margin: 4px 0 14px;
        font-size: 11px;
        overflow-wrap: anywhere;
        color: var(--muted);
    }
    .inline-error {
        font-size: 11px;
        color: var(--warning, #bb7733);
        margin: 8px 20px;
        line-height: 1.6;
    }
    .inline-error button {
        font-size: 11px;
    }
    .action-proposal {
        padding: 12px;
        margin-top: 12px;
        border: 1px solid var(--border);
        border-radius: 8px;
    }
    .action-proposal p {
        margin: 0 0 8px;
        font-size: 11px;
    }
    .action-proposal button {
        font-size: 11px;
    }
    .action-result {
        border-left: 2px solid #39ad79;
        padding-left: 12px;
        font-size: 11px;
        line-height: 1.6;
    }
</style>
