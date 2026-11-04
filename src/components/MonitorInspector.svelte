<script module lang="ts">
    type Entry = {
        role: 'user' | 'assistant';
        text: string;
        answer?: Answer;
        /** The automatic first look: its prompt is not shown as a user message. */
        auto?: boolean;
    };
    type Answer = {
        text: string;
        sources: { title: string; url: string }[];
        action: { process: ProcessRecord; action: string; reason: string } | null;
        provider: string;
        model: string;
        captured_at: number;
        contributors?: ProcessRecord[];
        inside?: InsideView | null;
        follow_ups?: string[];
    };
    const conversations = new Map<string, Entry[]>();
</script>

<script lang="ts">
    import { invoke } from '@tauri-apps/api/core';
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
    import { queueInference, cancelled, identityKey, type Explanation } from '../lib/monitor';
    import { appFallback, processRole } from '../lib/monitor-copy';
    import { memory, percent, errorMessage } from '../lib/format';
    import AppIcon from './AppIcon.svelte';
    import Logo from './Logo.svelte';
    import InsidePanel from './InsidePanel.svelte';
    let {
        snapshot,
        workload,
        initialProcess,
        initialQuestion = null,
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
        initialQuestion?: string | null;
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
    let tab = $state('ask');
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
            'Ask Jitterbug'
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
                    ? appFallback(workload)
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
        void invoke<WorkloadDetails>('get_workload_details', {
            workloadId: id,
            pid: target?.pid ?? null,
            startTime: target?.start_time ?? null,
        })
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
    async function describe(next: WorkloadDetails, key: string, requested = false) {
        if (next.profile.source === 'model') return;
        describing = true;
        const current = () => !disposed && key === lastProfile;
        const valid = () => current();
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
        if (!log) return;
        const latest =
            conversation.at(-1)?.role === 'assistant'
                ? Array.from(log.querySelectorAll('.turn')).at(-1)
                : null;
        if (latest)
            log.scrollTop += latest.getBoundingClientRect().top - log.getBoundingClientRect().top;
        else log.scrollTop = log.scrollHeight;
    }
    async function send(suggestion?: string, auto = false) {
        if (busy || error || !(suggestion ?? question).trim()) return;
        const text = (suggestion ?? question).trim();
        const key = context;
        const target = selected ? { ...selected } : null;
        const id = workload?.id ?? null;
        conversation = [...conversation, { role: 'user', text, auto }];
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
            await invoke('prioritize_monitor_chat');
            const answer: Answer = await queueInference(
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
                    { role: 'assistant' as const, text: answer.text, answer, auto },
                ].slice(-12);
                remember();
                void reveal();
            }
            onusage();
        } catch (cause) {
            if (!disposed && key === context && cause !== cancelled)
                chatError = errorMessage(cause);
        } finally {
            if (pendingContext === key) pendingContext = null;
            if (!auto) void tick().then(() => input?.focus({ preventScroll: true }));
        }
    }
    // The first look happens on its own: opening an app asks Jitterbug for its read, once per
    // app or process (the conversation is remembered while the app keeps running).
    let autoAsked = '';
    $effect(() => {
        const key = context;
        const ready = workload ? detail?.workload.id === workload.id : true;
        if (!ready || error || busy || conversation.length || autoAsked === key) return;
        autoAsked = key;
        untrack(() => {
            if (!workload) {
                if (initialQuestion) void send(initialQuestion);
                return;
            }
            const name = process
                ? `the ${process.name} process (PID ${process.pid})`
                : workload.label;
            void send(
                `Give me your read on ${name}: what is it doing on my Mac right now, and is this much CPU and memory normal for it? Mention the process doing the most work.`,
                true
            );
        });
    });
    const followUps = $derived.by(() => {
        const last = conversation.at(-1);
        const asked = new Set(conversation.filter((e) => e.role === 'user').map((e) => e.text));
        const suggested =
            last?.role === 'assistant' && last.answer?.follow_ups?.length
                ? last.answer.follow_ups
                : workload
                  ? [
                        'Why is it using this much memory?',
                        'Can I reduce its impact?',
                        'What started it?',
                    ]
                  : [
                        'What’s using the most resources?',
                        'Why does my Mac feel slow?',
                        'What can I safely close?',
                    ];
        return suggested.filter((q) => !asked.has(q)).slice(0, 3);
    });
    function currentAction(answer: Answer) {
        const action = answer.action;
        if (!action || error || actionBusy) return null;
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
        tab = 'ask';
    }
</script>

<div class="inspector-shell">
    <header class="inspector-head">
        {#if workload}<AppIcon
                workloadId={workload.id}
                category={workload.category}
                size={40}
            />{:else}<span class="assistant-mark"><Logo size={26} live /></span>{/if}
        <div class="heading">
            <h2 {title}>{title}</h2>
            <span
                >{process
                    ? `${workload?.label} · PID ${process.pid}`
                    : workload
                      ? `${detail?.member_count ?? workload.process_count} ${(detail?.member_count ?? workload.process_count) === 1 ? 'process' : 'processes'}${detail?.application?.version ? ` · version ${detail.application.version}` : ''}`
                      : `Ask about anything running on this Mac`}</span
            >
        </div>
        <button class="icon-button close" aria-label="Close inspector" onclick={onclose}
            ><svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"
                ><path
                    d="M2.5 2.5l7 7m0-7-7 7"
                    stroke="currentColor"
                    stroke-width="1.6"
                    stroke-linecap="round"
                /></svg
            ></button
        >
    </header>
    {#if workload}<section class="about" aria-label="App summary">
            <p class="about-text">
                {#if (!profile || profile.source === 'inference') && !selected && !detail?.launch}
                    {#if describing}<span class="pending">Meta is identifying this app…</span
                        >{:else}<button
                            class="link"
                            disabled={busy || !detail}
                            onclick={() => {
                                if (detail) void describe(detail, lastProfile, true);
                            }}>Identify this app with Meta</button
                        >{/if}
                {:else}{summary}{#if profile?.source === 'model'}<button
                            class="credit"
                            title="Described by Meta · view sources"
                            onclick={() => (tab = 'details')}>Meta</button
                        >{/if}{/if}
            </p>
            {#if workload.origin && !process}<p class="origin-line">
                    <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"
                        ><path
                            d="M2.5 1.5v4.5a2 2 0 0 0 2 2h5m-2.5-2.5L9.5 8 7 10.5"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="1.4"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                        /></svg
                    >{workload.origin}
                </p>{/if}
            {#if readings}<dl class="figures">
                    <div>
                        <dt>CPU</dt>
                        <dd>{percent(readings.cpu_percent)}</dd>
                    </div>
                    <div>
                        <dt>Memory</dt>
                        <dd>{memory(readings.memory_bytes)}</dd>
                    </div>
                    <div>
                        <dt>Share of RAM</dt>
                        <dd>
                            {(
                                (readings.memory_bytes / Math.max(snapshot.memory_total_bytes, 1)) *
                                100
                            ).toFixed(1)}%
                        </dd>
                    </div>
                    {#if process}<button class="link whole" onclick={() => choose(null)}
                            >Whole app</button
                        >{/if}
                </dl>{/if}
            {#if profileError}<p class="quiet-error">
                    Couldn’t describe this app: {profileError}
                    <button
                        class="link"
                        onclick={() => {
                            if (detail) void describe(detail, lastProfile, true);
                        }}>Try again</button
                    >
                </p>{/if}
        </section>
        <nav class="tabs" aria-label="Inspector views">
            <button aria-pressed={tab === 'ask'} onclick={() => (tab = 'ask')}>Ask</button><button
                aria-pressed={tab === 'processes'}
                onclick={() => (tab = 'processes')}
                >Processes <span>{detail?.member_count ?? workload.process_count}</span></button
            ><button aria-pressed={tab === 'details'} onclick={() => (tab = 'details')}
                >Details</button
            >
        </nav>{/if}
    {#if error}<p class="quiet-error pad" role="status">{error}</p>{/if}
    {#if tab === 'ask'}
        <!-- A focusable log lets keyboard users scroll the conversation. -->
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div
            class="conversation"
            role="log"
            aria-label="Process conversation"
            tabindex="0"
            bind:this={log}
        >
            {#if !conversation.length && !workload}<div class="intro">
                    <p>
                        Ask in your own words. Jitterbug reads the live measurements, what launched
                        each process, and recent history, then explains it plainly.
                    </p>
                </div>{/if}
            {#each conversation as entry}
                {#if entry.role === 'user'}
                    {#if !entry.auto}<div class="turn mine"><p>{entry.text}</p></div>{/if}
                {:else}
                    <div class="turn reply">
                        <div class="who">
                            <Logo size={14} /><span
                                >{entry.auto ? 'Jitterbug’s read' : 'Jitterbug'}</span
                            >{#if entry.answer}<small
                                    >· {entry.answer.provider === 'meta'
                                        ? 'Meta'
                                        : entry.answer.provider}</small
                                >{/if}
                        </div>
                        <p>{entry.text}</p>
                        {#if entry.answer?.contributors?.length}<div class="evidence">
                                <div class="evidence-heading">
                                    <span>Processes checked</span><button
                                        class="link"
                                        onclick={() => (tab = 'processes')}>All processes</button
                                    >
                                </div>
                                {#each entry.answer.contributors as p}
                                    {@const known = entry.answer.inside?.labels.find(
                                        (l) => l.pid === p.pid && l.start_time === p.start_time
                                    )}
                                    {@const current = detail?.members.find(
                                        (member) =>
                                            member.pid === p.pid &&
                                            member.start_time === p.start_time
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
                                        <span class="numeric">{memory(p.memory_bytes)}</span>
                                    </button>
                                {/each}
                            </div>{/if}
                        {#if entry.answer?.sources?.length}<div class="sources">
                                {#each entry.answer.sources.slice(0, 2) as source}<a
                                        href={source.url}
                                        target="_blank"
                                        rel="noreferrer">{source.title}</a
                                    >{/each}
                            </div>{/if}
                        {#if entry.answer?.action}{@const action =
                                entry.answer.action}{@const current = currentAction(entry.answer)}
                            <div class="proposal">
                                <p>{action.reason}</p>
                                <button
                                    disabled={!current}
                                    onclick={() => {
                                        if (current) onaction(current, action.action);
                                    }}
                                    >{action.action === 'lower_priority'
                                        ? 'Review lowering its priority…'
                                        : 'Review stopping this process…'}</button
                                >
                            </div>{/if}
                    </div>
                {/if}
            {/each}
            {#if busy}<div class="thinking" role="status">
                    <span class="dots"><i></i><i></i><i></i></span>
                    {conversation.length <= 1 && conversation[0]?.auto
                        ? `Looking at ${title}…`
                        : 'Thinking…'}
                </div>{/if}
            {#if chatError}<div class="quiet-error" role="alert">
                    {chatError}<button
                        class="link"
                        onclick={() => {
                            question =
                                conversation.filter((e) => e.role === 'user' && !e.auto).at(-1)
                                    ?.text ?? '';
                            if (!question) {
                                autoAsked = '';
                                conversation = [];
                            }
                            input?.focus();
                        }}>Try again</button
                    >
                </div>{/if}
            {#if result}<div class="result" role="status">{result.message}</div>{/if}
            {#if !busy && followUps.length && !error}<div class="follow-ups">
                    {#each followUps as prompt}<button onclick={() => void send(prompt)}
                            >{prompt}</button
                        >{/each}
                </div>{/if}
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
                placeholder={workload ? `Ask about ${workload.label}` : 'Ask about your Mac'}
                rows="1"
                maxlength="1800"
                disabled={!!error}
                onkeydown={(event) => {
                    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
                        event.preventDefault();
                        void send();
                    }
                }}></textarea>
            <button
                class="send"
                aria-label="Send message"
                disabled={busy || !!error || !question.trim()}
                ><svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"
                    ><path
                        d="M7 11.5v-9m0 0L3 6.5m4-4 4 4"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.8"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                    /></svg
                ></button
            >
        </form>
    {:else if tab === 'processes'}<div
            class="detail-scroll"
            role="group"
            aria-label="App processes"
        >
            <input
                class="process-search"
                aria-label="Filter processes"
                placeholder="Find a process"
                bind:value={processSearch}
            />
            {#each (detail?.members ?? []).filter( (p) => `${p.name} ${p.pid} ${label(p)?.title ?? ''} ${label(p)?.subtitle ?? ''}`
                        .toLowerCase()
                        .includes(processSearch.toLowerCase()) ) as p (identityKey(p))}<button
                    class="member"
                    class:chosen={selected?.pid === p.pid && selected?.start_time === p.start_time}
                    onclick={() => choose(p)}
                    ><span
                        ><strong>{label(p)?.title ?? p.name}</strong><small
                            >{label(p)?.subtitle || processRole(p, workload?.label ?? 'this app')} · PID
                            {p.pid}</small
                        ></span
                    ><span class="numeric"
                        >{memory(p.memory_bytes)}<small>{percent(p.cpu_percent)} CPU</small></span
                    ></button
                >{/each}{#if detail && detail.members.length < detail.member_count}<p class="muted">
                    Showing {detail.members.length} of {detail.member_count} processes.
                </p>{/if}{#if detail && workload}<InsidePanel
                    {snapshot}
                    workload={detail.workload}
                    {actionBusy}
                    {onaction}
                    onview={named}
                />{/if}
        </div>
    {:else}<div class="detail-scroll technical">
            {#if detail}
                <section>
                    <h3>Where it came from</h3>
                    {#if workload?.origin}<p>{workload.origin}.</p>{/if}
                    <p class="muted">{detail.origin_summary}</p>
                    {#if detail.launch}<p>{detail.launch.summary}</p>
                        <ul>
                            {#each detail.launch.evidence as evidence}<li>{evidence}</li>{/each}
                        </ul>{/if}
                    <ol class="chain">
                        {#each detail.ancestry as node, i}<li style={`--depth:${i}`}>
                                <strong>{node.name}</strong><span>PID {node.pid}</span>
                            </li>{/each}
                    </ol>
                </section>
                {#if detail.application}<section>
                        <h3>Identity</h3>
                        <dl class="facts">
                            <dt>Kind</dt>
                            <dd>{detail.application.installation}</dd>
                            {#if detail.application.identifier}<dt>Bundle ID</dt>
                                <dd>{detail.application.identifier}</dd>{/if}
                            {#if detail.application.version}<dt>Version</dt>
                                <dd>{detail.application.version}</dd>{/if}
                        </dl>
                    </section>{/if}
                {#if profile?.source === 'model' || profile?.sources?.length}<section>
                        <h3>Description sources</h3>
                        {#each profile?.sources ?? [] as source}<a
                                href={source.url}
                                target="_blank"
                                rel="noreferrer">{source.title}</a
                            >{/each}
                        {#if profile?.source === 'model'}<p class="muted">
                                Written by {profile.provider === 'meta' ? 'Meta' : profile.provider} ·
                                {profile.model}{profile.generated_at
                                    ? ` · ${new Date(profile.generated_at * 1000).toLocaleDateString()}`
                                    : ''}
                            </p>{/if}
                    </section>{/if}
                <details>
                    <summary>Executable and folder</summary
                    >{#each detail.ancestry.slice(-1) as node}<dl class="facts">
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
    .inspector-shell {
        display: flex;
        flex-direction: column;
        height: 100%;
        min-height: 0;
    }
    .inspector-head {
        padding: 16px 14px 10px 18px;
        display: flex;
        align-items: center;
        gap: 12px;
        flex-shrink: 0;
    }
    .assistant-mark {
        width: 40px;
        height: 40px;
        border-radius: 10px;
        display: grid;
        place-items: center;
        background: var(--raised);
        flex: none;
    }
    .heading {
        min-width: 0;
        flex: 1;
    }
    h2 {
        font-size: 16px;
        margin: 0;
        font-weight: 650;
        letter-spacing: -0.015em;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .heading span {
        font-size: 12px;
        color: var(--muted);
    }
    .close {
        align-self: start;
    }
    .about {
        padding: 0 18px 12px;
        flex-shrink: 0;
        display: grid;
        gap: 8px;
    }
    .about-text {
        font-size: 13px;
        line-height: 1.5;
        margin: 0;
    }
    .pending {
        color: var(--faint);
        font-style: italic;
    }
    .credit {
        all: unset;
        cursor: pointer;
        margin-left: 6px;
        font-size: 10px;
        font-weight: 600;
        color: var(--brand);
        background: var(--accent-soft);
        border-radius: 4px;
        padding: 1px 5px;
        vertical-align: 1px;
    }
    .origin-line {
        margin: 0;
        display: flex;
        align-items: center;
        gap: 6px;
        font-size: 12px;
        color: var(--brand);
    }
    .figures {
        display: flex;
        gap: 22px;
        margin: 4px 0 0;
        align-items: end;
    }
    .figures div {
        display: grid;
        gap: 1px;
    }
    .figures dt {
        font-size: 11px;
        color: var(--faint);
    }
    .figures dd {
        margin: 0;
        font-size: 15px;
        font-weight: 600;
        font-variant-numeric: tabular-nums;
        letter-spacing: -0.01em;
    }
    .whole {
        margin-left: auto;
        font-size: 12px;
    }
    .tabs {
        display: flex;
        margin: 2px 18px 0;
        padding: 2px;
        border-radius: 7px;
        background: #ffffff0f;
        flex-shrink: 0;
    }
    .tabs button {
        flex: 1;
        min-height: 24px;
        padding: 1px 8px;
        font-size: 12px;
        font-weight: 500;
        background: transparent;
        box-shadow: none;
        border-radius: 5px;
        color: var(--muted);
    }
    .tabs button:hover:not(:disabled) {
        background: transparent;
        color: var(--text);
    }
    .tabs button[aria-pressed='true'] {
        background: #5a5a60;
        color: var(--text);
        box-shadow:
            0 1px 2px #0000004d,
            inset 0 0.5px 0 #ffffff26;
    }
    .tabs span {
        color: var(--faint);
        margin-left: 3px;
        font-variant-numeric: tabular-nums;
    }
    .tabs button[aria-pressed='true'] span {
        color: var(--muted);
    }
    .conversation {
        flex: 1;
        min-height: 0;
        overflow: auto;
        padding: 16px 18px 8px;
        overflow-wrap: anywhere;
        display: flex;
        flex-direction: column;
        gap: 14px;
        outline: none;
    }
    .intro p {
        margin: 0;
        color: var(--muted);
        font-size: 13px;
    }
    .turn p {
        white-space: pre-wrap;
        margin: 0;
        font-size: 13px;
        line-height: 1.55;
    }
    .turn.mine {
        align-self: flex-end;
        max-width: 86%;
        background: #3a3a40;
        padding: 7px 12px;
        border-radius: 16px 16px 4px 16px;
    }
    .turn {
        flex: none;
    }
    .turn.reply {
        display: grid;
        gap: 6px;
        align-content: start;
    }
    .who {
        display: flex;
        align-items: center;
        gap: 6px;
        font-size: 12px;
        font-weight: 600;
    }
    .who small {
        font-size: 11px;
        font-weight: 400;
        color: var(--faint);
    }
    .evidence {
        margin-top: 4px;
        padding: 6px;
        border-radius: 10px;
        background: #ffffff08;
    }
    .evidence-heading {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 8px;
        font-size: 11px;
        color: var(--faint);
        padding: 2px 6px 4px;
    }
    .evidence-heading .link {
        font-size: 11px;
    }
    .contributor {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 12px;
        width: 100%;
        text-align: left;
        background: none;
        box-shadow: none;
        padding: 5px 6px;
        border-radius: 6px;
        font-weight: 400;
        min-height: 0;
    }
    .contributor:disabled {
        opacity: 1;
    }
    .contributor > span:first-child {
        min-width: 0;
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
        font-size: 12px;
    }
    .contributor small {
        color: var(--faint);
        font-size: 11px;
    }
    .numeric {
        font-variant-numeric: tabular-nums;
        font-size: 12px;
        flex: none;
    }
    .sources {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
    }
    .sources a {
        font-size: 11px;
        padding: 2px 8px;
        border-radius: 10px;
        background: #ffffff0d;
        color: var(--muted);
        max-width: 100%;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .proposal {
        margin-top: 4px;
        padding: 10px 12px;
        border-radius: 10px;
        background: var(--accent-soft);
        display: grid;
        gap: 8px;
        justify-items: start;
    }
    .proposal p {
        font-size: 12px;
    }
    .thinking {
        display: flex;
        align-items: center;
        gap: 8px;
        font-size: 12px;
        color: var(--muted);
    }
    .dots {
        display: inline-flex;
        gap: 3px;
    }
    .dots i {
        width: 5px;
        height: 5px;
        border-radius: 50%;
        background: var(--brand);
        animation: blink 1.2s infinite ease-in-out;
    }
    .dots i:nth-child(2) {
        animation-delay: 0.15s;
    }
    .dots i:nth-child(3) {
        animation-delay: 0.3s;
    }
    @keyframes blink {
        0%,
        80%,
        100% {
            opacity: 0.25;
        }
        40% {
            opacity: 1;
        }
    }
    .follow-ups {
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: 6px;
        margin-top: 2px;
    }
    .follow-ups button {
        background: transparent;
        box-shadow: inset 0 0 0 1px #ffffff24;
        border-radius: 14px;
        min-height: 26px;
        padding: 3px 11px;
        font-size: 12px;
        font-weight: 450;
        color: var(--text);
        text-align: left;
    }
    .follow-ups button:hover:not(:disabled) {
        background: var(--hover);
        box-shadow: inset 0 0 0 1px #5fe0b366;
    }
    .composer {
        margin: 6px 14px 14px;
        padding: 5px 5px 5px 13px;
        border-radius: 18px;
        background: #ffffff0d;
        box-shadow: inset 0 0 0 0.5px #ffffff1f;
        display: flex;
        align-items: flex-end;
        gap: 8px;
        flex-shrink: 0;
    }
    .composer:focus-within {
        box-shadow:
            inset 0 0 0 1px #4c9dffaa,
            0 0 0 3px #4c9dff33;
    }
    .composer textarea,
    .composer textarea:focus {
        flex: 1;
        border: 0;
        background: transparent;
        box-shadow: none;
        padding: 4px 0;
        min-height: 26px;
        max-height: 140px;
        resize: none;
        font-size: 13px;
        line-height: 18px;
        field-sizing: content;
        outline: none;
    }
    .send {
        width: 26px;
        height: 26px;
        min-height: 26px;
        padding: 0;
        border-radius: 50%;
        display: grid;
        place-items: center;
        background: var(--brand);
        color: #08251b;
        box-shadow: none;
        flex: none;
    }
    .send:hover:not(:disabled) {
        background: var(--accent-hover);
    }
    .send:disabled {
        background: #ffffff1a;
        color: var(--faint);
        opacity: 1;
    }
    .detail-scroll {
        flex: 1;
        min-height: 0;
        overflow: auto;
        padding: 14px 12px;
    }
    .process-search {
        width: calc(100% - 12px);
        margin: 0 6px 8px;
        font-size: 12px;
    }
    .member {
        width: 100%;
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 14px;
        padding: 7px 8px;
        min-height: 0;
        background: none;
        box-shadow: none;
        border-radius: 6px;
        font-size: 12px;
        text-align: left;
        font-weight: 400;
    }
    .member:hover:not(:disabled) {
        background: var(--hover);
    }
    .member.chosen {
        background: var(--selected);
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
        font-size: 11px;
        color: var(--faint);
        overflow: hidden;
        white-space: nowrap;
        text-overflow: ellipsis;
    }
    .member > .numeric {
        text-align: right;
        white-space: nowrap;
    }
    .detail-scroll :global(.inside) {
        padding: 14px 8px;
        margin: 14px 0 0;
        border: 0;
        border-radius: 0;
        box-shadow: none;
        background: transparent;
    }
    .technical {
        padding: 16px 18px;
        font-size: 12px;
        line-height: 1.55;
        display: grid;
        gap: 18px;
        align-content: start;
    }
    .technical section {
        display: grid;
        gap: 6px;
    }
    .technical h3 {
        font-size: 12px;
        margin: 0;
        color: var(--muted);
        font-weight: 600;
    }
    .technical p,
    .technical ul {
        margin: 0;
    }
    .technical a {
        display: block;
        overflow-wrap: anywhere;
    }
    .chain {
        list-style: none;
        padding: 0;
        margin: 4px 0 0;
        display: grid;
        gap: 2px;
    }
    .chain li {
        padding: 3px 0 3px calc(var(--depth) * 10px);
        display: flex;
        gap: 10px;
        font-size: 12px;
    }
    .chain li span {
        color: var(--faint);
        font-variant-numeric: tabular-nums;
    }
    .facts {
        display: grid;
        grid-template-columns: 90px minmax(0, 1fr);
        gap: 4px 12px;
        margin: 0;
    }
    .facts dt {
        color: var(--faint);
    }
    .facts dd {
        margin: 0;
        overflow-wrap: anywhere;
    }
    .technical details summary {
        cursor: default;
        color: var(--muted);
    }
    .technical details .facts {
        margin-top: 10px;
    }
    .quiet-error {
        font-size: 12px;
        color: var(--warning);
        margin: 0;
        line-height: 1.5;
    }
    .quiet-error.pad {
        padding: 8px 18px 0;
    }
    .quiet-error .link {
        font-size: 12px;
        margin-left: 6px;
    }
    .result {
        padding: 8px 12px;
        border-radius: 10px;
        background: var(--success-soft);
        font-size: 12px;
    }
</style>
