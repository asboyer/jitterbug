<script module lang="ts">
    // Questions asked about each conversation, kept while the app runs.
    type Turn = { role: 'user' | 'assistant'; text: string; follow_ups?: string[] };
    const asked = new Map<string, Turn[]>();
</script>

<script lang="ts">
    import { onMount, tick, untrack } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import HarnessBadge from './HarnessBadge.svelte';
    import Logo from './Logo.svelte';
    import { ago, errorMessage } from '../lib/format';
    import { queueInference, cancelled } from '../lib/monitor';
    import {
        cpuShare,
        size,
        span,
        stepKind,
        type SessionAnswer,
        type SessionCall,
        type SessionDetail,
    } from '../lib/agents';

    let { id, onclose }: { id: string; onclose: () => void } = $props();
    let detail = $state<SessionDetail | null>(null);
    let error = $state('');
    let now = $state(Date.now() / 1000);
    let sheet: HTMLElement | undefined = $state();
    let view = $state<'overview' | 'ask'>('overview');
    let showAll = $state(false);
    let showProcesses = $state(false);
    let turns = $state<Turn[]>(untrack(() => asked.get(id) ?? []));
    let question = $state('');
    let busy = $state(false);
    let askError = $state('');
    let log: HTMLElement | undefined = $state();
    let input: HTMLTextAreaElement | undefined = $state();

    async function load() {
        try {
            detail = await invoke<SessionDetail>('get_agent_session', { id });
            error = '';
        } catch (cause) {
            error = errorMessage(cause);
        }
        now = Date.now() / 1000;
    }
    onMount(() => {
        void load();
        sheet?.focus({ preventScroll: true });
        const timer = setInterval(() => void load(), 5000);
        return () => clearInterval(timer);
    });

    async function ask(text = question) {
        const q = text.trim();
        if (!q || busy) return;
        view = 'ask';
        question = '';
        askError = '';
        turns = [...turns, { role: 'user', text: q }];
        busy = true;
        await tick();
        log?.scrollTo({ top: log.scrollHeight });
        try {
            const messages = turns
                .slice(-8)
                .map((t) => ({ role: t.role, content: t.text.slice(0, 3000) }));
            const answer = await queueInference(
                () => invoke<SessionAnswer>('ask_agent_session', { id, messages }),
                20
            );
            turns = [
                ...turns,
                { role: 'assistant', text: answer.answer, follow_ups: answer.follow_ups },
            ];
            asked.set(id, turns.slice(-12));
        } catch (cause) {
            if (cause !== cancelled) askError = errorMessage(cause);
        } finally {
            busy = false;
            await tick();
            const replies = log?.querySelectorAll('.reply');
            replies?.[replies.length - 1]?.scrollIntoView({ block: 'start' });
            input?.focus({ preventScroll: true });
        }
    }

    const clock = (time: number) =>
        new Date(time * 1000).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
    // Today: the time. Yesterday: "Yesterday 9:51 PM". Earlier: the weekday and time.
    const when = (time: number) => {
        const day = (t: number) => new Date(t * 1000).toDateString();
        if (day(time) === day(now)) return clock(time);
        if (day(time) === day(now - 86400)) return `Yesterday ${clock(time)}`;
        return new Date(time * 1000).toLocaleString([], {
            weekday: 'short',
            hour: 'numeric',
            minute: '2-digit',
        });
    };

    const session = $derived(detail?.session ?? null);
    const calls = $derived(detail?.calls ?? []);
    // Counts by kind, computed from the calls shown.
    const counts = $derived.by(() => {
        const c = { command: 0, edit: 0, read: 0, other: 0 };
        for (const call of calls) c[stepKind(call.tool)]++;
        return c;
    });
    const summary = $derived.by(() => {
        if (!calls.length) return '';
        const parts = [
            counts.command && `${counts.command} command${counts.command === 1 ? '' : 's'}`,
            counts.edit && `${counts.edit} edit${counts.edit === 1 ? '' : 's'}`,
            counts.read && `${counts.read} read${counts.read === 1 ? '' : 's'}`,
            counts.other && `${counts.other} other`,
        ].filter(Boolean);
        return `${calls.length} steps since ${when(calls.at(-1)!.started_at)}: ${parts.join(', ')}.`;
    });
    const longest = $derived(
        [...calls]
            .sort((a, b) => b.seconds - a.seconds)
            .filter((c) => c.seconds >= 5)
            .slice(0, 4)
    );
    // Every step on the conversation's own timeline: x is when it started, width how long it ran.
    const strip = $derived.by(() => {
        if (calls.length < 2) return null;
        const start = calls.at(-1)!.started_at;
        const end = Math.max(
            calls[0].ended_at ?? now,
            ...calls.map((c) => c.ended_at ?? c.started_at)
        );
        const width = Math.max(end - start, 60);
        return {
            start,
            end,
            marks: calls.map((c) => ({
                x: ((c.started_at - start) / width) * 100,
                w: Math.max((c.seconds / width) * 100, 0.35),
                kind: stepKind(c.tool),
                label: c.label,
            })),
        };
    });
    const chart = $derived.by(() => {
        const points = detail?.series ?? [];
        if (points.length < 2 || (detail?.peak_cpu_percent ?? 0) < 3) return null;
        const start = points[0].time;
        const width = Math.max(points.at(-1)!.time - start, 1);
        const top = Math.max(10, ...points.map((p) => p.cpu_percent));
        const xy = points.map(
            (p) =>
                `${(((p.time - start) / width) * 300).toFixed(1)},${(56 - (p.cpu_percent / top) * 52).toFixed(1)}`
        );
        return { line: xy.join(' '), area: `0,56 ${xy.join(' ')} 300,56`, top, start };
    });
    const suggestions = $derived(
        [
            session?.asks ? 'What should I answer?' : null,
            'What did this session get done?',
            'Where did the time go?',
            'How could this session have gone better?',
        ].filter((q): q is string => !!q && !turns.some((t) => t.text === q))
    );
    const cost = (call: SessionCall) =>
        call.samples
            ? `${span(call.seconds)} · avg ${cpuShare(call.average_cpu_percent)} CPU · peak ${size(call.peak_memory_bytes)}`
            : span(call.seconds);
</script>

<div
    class="session-sheet"
    role="dialog"
    aria-label="Conversation details"
    tabindex="-1"
    bind:this={sheet}
    onkeydown={(event) => {
        if (event.key === 'Escape' && !(event.target instanceof HTMLTextAreaElement)) onclose();
    }}
>
    <header>
        {#if session}<HarnessBadge harness={session.harness} compact />{/if}
        <div class="title">
            <h2>{session?.title || session?.folder || 'Conversation'}</h2>
            {#if session}<span>{session.harness_name} · {session.folder}</span>{/if}
        </div>
        <button class="icon-button" aria-label="Close conversation details" onclick={onclose}
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
    <div class="segmented" role="group" aria-label="Conversation view">
        <button aria-pressed={view === 'overview'} onclick={() => (view = 'overview')}
            >Overview</button
        ><button aria-pressed={view === 'ask'} onclick={() => (view = 'ask')}
            >Ask Jitterbug{#if turns.length}<span>{Math.ceil(turns.length / 2)}</span>{/if}</button
        >
    </div>

    {#if error && !detail}<p class="error pad" role="alert">
            {error} <button class="link" onclick={() => void load()}>Try again</button>
        </p>{/if}

    {#if view === 'overview' && session && detail}
        <div class="body">
            <section class="standing" class:asks={!!session.asks}>
                {#if session.state === 'working'}
                    <p class="state live"><i></i>Working</p>
                    <p class="lead">{session.doing ?? 'Working on your request'}</p>
                {:else if session.asks}
                    <p class="state">Needs your answer</p>
                    <p class="lead">{session.asks}</p>
                    <button class="primary help" onclick={() => void ask('What should I answer?')}
                        ><Logo size={14} /> Help me answer</button
                    >
                {:else}
                    <p class="state">Done</p>
                    <p class="lead">
                        Finished {session.last_active ? ago(session.last_active, now) : ''}
                    </p>
                {/if}
                <p class="meta">
                    {[
                        session.last_active
                            ? `Last active ${when(session.last_active).replace(/^Yesterday/, 'yesterday')}`
                            : '',
                        detail.started_at ? `started ${ago(detail.started_at, now)}` : '',
                    ]
                        .filter(Boolean)
                        .join(' · ')}
                </p>
                {#if session.request}<p class="request">
                        <span>Your last request</span>{session.request}
                    </p>{/if}
            </section>

            <section>
                <div class="heading">
                    <h3>What it did</h3>
                    {#if strip}<span class="legend"
                            ><i class="command"></i>Commands<i class="edit"></i>Edits<i class="read"
                            ></i>Reading</span
                        >{/if}
                </div>
                {#if calls.length}
                    <p class="summary">{summary}</p>
                    {#if strip}<div
                            class="strip"
                            role="img"
                            aria-label={`Timeline of ${calls.length} steps from ${when(strip.start)} to ${when(strip.end)}`}
                        >
                            {#each strip.marks as mark}<i
                                    class={mark.kind}
                                    style={`left:${mark.x}%;width:${mark.w}%`}
                                    title={mark.label}
                                ></i>{/each}
                        </div>
                        <p class="axis">
                            <span>{when(strip.start)}</span><span>{when(strip.end)}</span>
                        </p>{/if}
                    {#if longest.length}<h4>Longest steps</h4>
                        {#each longest as call (call.started_at + call.label)}
                            <div class="row">
                                <span class="label"
                                    ><strong>{call.label}</strong><small
                                        >{when(call.started_at)}</small
                                    ></span
                                >
                                <span class="figure">{span(call.seconds)}</span>
                            </div>
                        {/each}{/if}
                    <h4>Latest steps</h4>
                    {#each showAll ? calls.slice(0, 40) : calls.slice(0, 5) as call (call.started_at + call.label)}
                        <div class="row">
                            <span class="label"
                                ><strong>{call.label}</strong><small
                                    >{call.ended_at === null
                                        ? 'Running now'
                                        : when(call.started_at)}</small
                                ></span
                            >
                            <span class="figure"
                                >{call.ended_at === null ? '' : span(call.seconds)}</span
                            >
                        </div>
                    {/each}
                    {#if calls.length > 5}<button
                            class="link more"
                            onclick={() => (showAll = !showAll)}
                            >{showAll
                                ? 'Show fewer'
                                : `Show ${Math.min(calls.length, 40) - 5} more`}</button
                        >{/if}
                {:else}<p class="quiet">
                        {detail.calls_available
                            ? 'No tool calls in the recent part of this conversation.'
                            : `${session.harness_name} doesn’t keep a log of steps Jitterbug can read. Ask Jitterbug still works from its messages.`}
                    </p>{/if}
            </section>

            {#if detail.heavy.length}<section>
                    <h3>Tests and builds</h3>
                    {#each detail.heavy as item (item.started_at + item.command)}
                        <div class="row">
                            <span class="label"
                                ><strong>{item.command}</strong><small
                                    >{item.waited_seconds
                                        ? `waited ${span(item.waited_seconds)} · `
                                        : ''}peak {cpuShare(item.peak_cpu_percent)} CPU, {size(
                                        item.peak_memory_bytes
                                    )} · {when(item.ended_at)}</small
                                ></span
                            >
                            <span class="figure">{span(item.ended_at - item.started_at)}</span>
                        </div>
                    {/each}
                </section>{/if}

            <section>
                <h3>Resources</h3>
                <dl class="figures">
                    <div>
                        <dt>Memory</dt>
                        <dd>{size(session.memory_bytes)}</dd>
                    </div>
                    <div>
                        <dt>CPU now</dt>
                        <dd>{cpuShare(session.cpu_percent)}</dd>
                    </div>
                    <div>
                        <dt>Peak CPU</dt>
                        <dd>{cpuShare(detail.peak_cpu_percent)}</dd>
                    </div>
                    <div>
                        <dt>Processes</dt>
                        <dd>{detail.process_count || '—'}</dd>
                    </div>
                </dl>
                {#if chart}<svg
                        class="chart"
                        viewBox="0 0 300 58"
                        preserveAspectRatio="none"
                        role="img"
                        aria-label={`CPU use, peaking at ${cpuShare(detail.peak_cpu_percent)} of this Mac`}
                        ><polygon points={chart.area} /><polyline points={chart.line} /></svg
                    >
                    <p class="axis">
                        <span>{clock(chart.start)}</span><span
                            >Scale to {Math.round(chart.top)}% of this Mac</span
                        >
                    </p>{:else if detail.tracked_since}<p class="quiet">
                        Quiet since Jitterbug started watching at {clock(detail.tracked_since)}.
                    </p>{/if}
                {#if detail.heaviest.length}<h4>Heaviest steps while watched</h4>
                    {#each detail.heaviest as call (call.started_at + call.label)}
                        <div class="row">
                            <span class="label"
                                ><strong>{call.label}</strong><small>{cost(call)}</small></span
                            >
                            <span class="figure accent" title="CPU time in seconds of one core"
                                >{Math.round(call.cpu_seconds)} CPU-s</span
                            >
                        </div>
                    {/each}{/if}
                {#if detail.processes.length}<button
                        class="link more"
                        aria-expanded={showProcesses}
                        onclick={() => (showProcesses = !showProcesses)}
                        >{showProcesses ? 'Hide processes' : 'Show processes'}</button
                    >
                    {#if showProcesses}{#each detail.processes as p (p.pid)}
                            <div class="row">
                                <span class="label"
                                    ><strong>{p.name}</strong><small>PID {p.pid}</small></span
                                >
                                <span class="figure"
                                    >{cpuShare(p.cpu_percent)} · {size(p.memory_bytes)}</span
                                >
                            </div>
                        {/each}{/if}{/if}
            </section>
        </div>
    {:else if view === 'ask'}
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div
            class="body chat"
            role="log"
            aria-label="Questions about this conversation"
            tabindex="0"
            bind:this={log}
        >
            {#if !turns.length}<div class="intro">
                    <p>
                        Look back at this conversation: what it got done, where the time went, and
                        how the next one could go better.
                    </p>
                    <p class="privacy">
                        Your question, this conversation’s recent messages and its step summary go
                        to your AI model. Nothing is saved.
                    </p>
                </div>{/if}
            {#each turns as turn}
                {#if turn.role === 'user'}<div class="mine"><p>{turn.text}</p></div>
                {:else}<div class="reply">
                        <p class="who"><Logo size={14} /> Jitterbug</p>
                        {#each turn.text.split(/\n\s*\n/) as paragraph}<p>{paragraph}</p>{/each}
                    </div>{/if}
            {/each}
            {#if busy}<p class="thinking" role="status">
                    <span class="dots"><i></i><i></i><i></i></span>Reading the conversation…
                </p>{/if}
            {#if askError}<p class="error" role="alert">
                    {askError}
                    <button
                        class="link"
                        onclick={() => {
                            const last = turns.at(-1);
                            if (last?.role === 'user') {
                                turns = turns.slice(0, -1);
                                void ask(last.text);
                            }
                        }}>Try again</button
                    >
                </p>{/if}
            {#if !busy}
                {@const next = turns.at(-1)?.follow_ups?.length
                    ? turns.at(-1)!.follow_ups!
                    : suggestions}
                <div class="chips">
                    {#each next.slice(0, 4) as prompt}<button onclick={() => void ask(prompt)}
                            >{prompt}</button
                        >{/each}
                </div>
            {/if}
        </div>
    {/if}

    <form
        class="composer"
        onsubmit={(event) => {
            event.preventDefault();
            void ask();
        }}
    >
        <textarea
            bind:this={input}
            bind:value={question}
            rows="1"
            maxlength="1500"
            aria-label="Ask about this conversation"
            placeholder="Ask about this conversation"
            onkeydown={(event) => {
                if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
                    event.preventDefault();
                    void ask();
                }
            }}></textarea>
        <button class="send" aria-label="Send question" disabled={busy || !question.trim()}
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
</div>

<style>
    .session-sheet {
        position: fixed;
        top: 0;
        right: 0;
        bottom: 0;
        width: min(440px, calc(100vw - 240px));
        z-index: 30;
        display: flex;
        flex-direction: column;
        background: var(--inspector);
        box-shadow:
            -1px 0 0 #00000080,
            -16px 0 40px #00000059;
        outline: none;
        animation: slide 0.18s ease-out;
    }
    @keyframes slide {
        from {
            transform: translateX(16px);
            opacity: 0;
        }
    }
    @media (prefers-reduced-motion: reduce) {
        .session-sheet {
            animation: none;
        }
    }
    header {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 18px 14px 12px 18px;
    }
    .title {
        flex: 1;
        min-width: 0;
    }
    h2 {
        margin: 0;
        font-size: 15px;
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .title span {
        display: block;
        font-size: 12px;
        color: var(--muted);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .segmented {
        display: flex;
        margin: 0 18px 4px;
        padding: 2px;
        border-radius: 7px;
        background: #ffffff0f;
    }
    .segmented button {
        flex: 1;
        min-height: 24px;
        padding: 2px 10px;
        font-size: 12px;
        font-weight: 500;
        background: transparent;
        box-shadow: none;
        border-radius: 5px;
        color: var(--muted);
    }
    .segmented button:hover:not(:disabled) {
        background: transparent;
        color: var(--text);
    }
    .segmented button[aria-pressed='true'] {
        background: #5a5a60;
        color: var(--text);
        box-shadow: 0 1px 2px #0000004d;
    }
    .segmented span {
        margin-left: 6px;
        font-size: 11px;
        color: var(--muted);
    }
    .body {
        flex: 1;
        min-height: 0;
        overflow: auto;
        padding: 10px 18px 16px;
        outline: none;
    }
    section + section {
        margin-top: 26px;
    }
    .standing {
        padding: 14px 16px;
        border-radius: 10px;
        background: #ffffff08;
    }
    .standing.asks {
        background: var(--accent-soft);
    }
    .state {
        display: flex;
        align-items: center;
        gap: 7px;
        margin: 0;
        font-size: 12px;
        font-weight: 600;
        color: var(--muted);
    }
    .standing.asks .state,
    .state.live {
        color: var(--brand);
    }
    .state.live i {
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: var(--brand);
    }
    .lead {
        margin: 6px 0 0;
        font-size: 16px;
        font-weight: 600;
        line-height: 1.35;
    }
    .help {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        margin-top: 12px;
        min-height: 28px;
        font-size: 12px;
    }
    .meta {
        margin: 10px 0 0;
        font-size: 12px;
        color: var(--muted);
    }
    .request {
        margin: 10px 0 0;
        font-size: 12px;
        line-height: 1.45;
        color: var(--muted);
        display: -webkit-box;
        -webkit-line-clamp: 3;
        line-clamp: 3;
        -webkit-box-orient: vertical;
        overflow: hidden;
    }
    .request span {
        display: block;
        color: var(--faint);
        font-size: 11px;
        margin-bottom: 2px;
    }
    .heading {
        display: flex;
        align-items: baseline;
        justify-content: space-between;
        gap: 10px;
    }
    h3 {
        margin: 0 0 8px;
        font-size: 13px;
        font-weight: 600;
    }
    h4 {
        margin: 16px 0 2px;
        font-size: 12px;
        font-weight: 500;
        color: var(--muted);
    }
    .legend {
        display: flex;
        align-items: center;
        gap: 5px 10px;
        font-size: 11px;
        color: var(--muted);
    }
    .legend i {
        width: 8px;
        height: 8px;
        border-radius: 2px;
        margin-right: -6px;
    }
    .summary {
        margin: 0 0 10px;
        font-size: 13px;
        line-height: 1.45;
    }
    .strip {
        position: relative;
        height: 28px;
        border-radius: 4px;
        background: #ffffff08;
        overflow: hidden;
    }
    .strip i {
        position: absolute;
        top: 4px;
        bottom: 4px;
        min-width: 2px;
        border-radius: 1px;
    }
    .command {
        background: var(--brand);
    }
    .edit {
        background: #d8d8dc;
    }
    .read,
    .other {
        background: #6e6e74;
    }
    .axis {
        display: flex;
        justify-content: space-between;
        margin: 4px 0 0;
        font-size: 11px;
        color: var(--faint);
    }
    .row {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        padding: 7px 0;
        box-shadow: inset 0 -0.5px 0 var(--border);
    }
    .label {
        min-width: 0;
        display: grid;
        gap: 2px;
    }
    .label strong {
        font-weight: 500;
        font-size: 13px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .label small {
        font-size: 11px;
        color: var(--muted);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .figure {
        flex: none;
        font-size: 12px;
        color: var(--muted);
        font-variant-numeric: tabular-nums;
    }
    .figure.accent {
        color: var(--brand);
        font-weight: 600;
    }
    .more {
        margin-top: 8px;
        font-size: 12px;
    }
    .figures {
        display: grid;
        grid-template-columns: repeat(4, 1fr);
        gap: 8px;
        margin: 0 0 10px;
    }
    .figures dt {
        font-size: 11px;
        color: var(--muted);
    }
    .figures dd {
        margin: 2px 0 0;
        font-size: 16px;
        font-weight: 600;
        font-variant-numeric: tabular-nums;
    }
    .chart {
        display: block;
        width: 100%;
        height: 60px;
    }
    .chart polygon {
        fill: var(--brand);
        opacity: 0.14;
    }
    .chart polyline {
        fill: none;
        stroke: var(--brand);
        stroke-width: 1.5;
        vector-effect: non-scaling-stroke;
    }
    .quiet {
        margin: 0;
        font-size: 12px;
        color: var(--faint);
        line-height: 1.5;
    }
    .pad {
        padding: 0 18px;
    }
    .intro p {
        margin: 6px 0 0;
        font-size: 13px;
        line-height: 1.5;
    }
    .intro .privacy {
        margin-top: 10px;
        font-size: 11px;
        color: var(--faint);
    }
    .mine {
        display: flex;
        justify-content: flex-end;
        margin: 14px 0 0;
    }
    .mine p {
        margin: 0;
        max-width: 85%;
        padding: 8px 12px;
        border-radius: 14px 14px 4px 14px;
        background: #3a3a3e;
        font-size: 13px;
        line-height: 1.45;
    }
    .reply {
        margin: 16px 0 0;
    }
    .reply p {
        margin: 0 0 8px;
        font-size: 13px;
        line-height: 1.55;
    }
    .reply .who {
        display: flex;
        align-items: center;
        gap: 6px;
        font-size: 12px;
        font-weight: 600;
        margin-bottom: 6px;
    }
    .thinking {
        display: flex;
        align-items: center;
        gap: 8px;
        margin: 16px 0 0;
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
        background: var(--muted);
        animation: blink 1.2s infinite ease-in-out;
    }
    .dots i:nth-child(2) {
        animation-delay: 0.15s;
    }
    .dots i:nth-child(3) {
        animation-delay: 0.3s;
    }
    @keyframes blink {
        50% {
            opacity: 0.3;
        }
    }
    .chips {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
        margin-top: 14px;
    }
    .chips button {
        min-height: 26px;
        padding: 3px 11px;
        border-radius: 13px;
        font-size: 12px;
        font-weight: 400;
        background: transparent;
        box-shadow: inset 0 0 0 1px var(--control-border);
    }
    .chips button:hover:not(:disabled) {
        background: #ffffff0d;
    }
    .composer {
        display: flex;
        align-items: flex-end;
        gap: 8px;
        margin: 0 14px 14px;
        padding: 6px 6px 6px 12px;
        border-radius: 18px;
        background: #2c2c30;
        box-shadow: inset 0 0 0 1px var(--control-border);
    }
    .composer textarea {
        flex: 1;
        min-height: 24px;
        max-height: 120px;
        padding: 3px 0;
        border: 0;
        background: transparent;
        box-shadow: none;
        resize: none;
        font-size: 13px;
        outline: none;
    }
    .send {
        display: grid;
        place-items: center;
        width: 26px;
        height: 26px;
        min-height: 0;
        padding: 0;
        border-radius: 50%;
        background: var(--brand);
        color: #0b2a20;
    }
    .send:disabled {
        background: #48484c;
        color: var(--faint);
    }
    .error {
        font-size: 12px;
    }
</style>
