<script lang="ts">
    import { onMount } from 'svelte';
    import { invoke } from '@tauri-apps/api/core';
    import HarnessBadge from './HarnessBadge.svelte';
    import { ago, errorMessage } from '../lib/format';
    import { cpuShare, size, span, type SessionCall, type SessionDetail } from '../lib/agents';

    let { id, onclose }: { id: string; onclose: () => void } = $props();
    let detail = $state<SessionDetail | null>(null);
    let error = $state('');
    let now = $state(Date.now() / 1000);
    let sheet: HTMLElement | undefined = $state();

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

    const clock = (time: number) =>
        new Date(time * 1000).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
    const cost = (call: SessionCall) =>
        call.samples
            ? `${span(call.seconds)} · avg ${cpuShare(call.average_cpu_percent)} CPU · peak ${size(call.peak_memory_bytes)}`
            : span(call.seconds);

    // CPU over the last hour as an area, scaled to the busiest moment (at least 10%).
    const chart = $derived.by(() => {
        const points = detail?.series ?? [];
        if (points.length < 2) return null;
        const start = points[0].time;
        const width = Math.max(points.at(-1)!.time - start, 1);
        const top = Math.max(10, ...points.map((p) => p.cpu_percent));
        const xy = points.map(
            (p) =>
                `${(((p.time - start) / width) * 300).toFixed(1)},${(56 - (p.cpu_percent / top) * 52).toFixed(1)}`
        );
        return { line: xy.join(' '), area: `0,56 ${xy.join(' ')} 300,56`, top, start };
    });
</script>

<div
    class="session-sheet"
    role="dialog"
    aria-label="Conversation details"
    tabindex="-1"
    bind:this={sheet}
    onkeydown={(event) => {
        if (event.key === 'Escape') onclose();
    }}
>
    <header>
        {#if detail}<HarnessBadge harness={detail.session.harness} compact />{/if}
        <div class="title">
            <h2>{detail?.session.title || detail?.session.folder || 'Conversation'}</h2>
            {#if detail}<span>{detail.session.harness_name} · {detail.session.folder}</span>{/if}
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
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if detail}
        {@const s = detail.session}
        <div class="body">
            <p class="status">
                <i class:live={s.state === 'working'}></i>{s.state === 'working'
                    ? (s.doing ?? 'Working')
                    : s.state === 'waiting'
                      ? 'Needs your answer'
                      : 'Done'}{#if s.last_active}<span>
                        · active {ago(s.last_active, now)}</span
                    >{/if}{#if detail.started_at}<span>
                        · started {ago(detail.started_at, now)}</span
                    >{/if}
            </p>
            {#if s.asks}<p class="asks">“{s.asks}”</p>{/if}
            {#if s.request}<p class="request">You asked: “{s.request}”</p>{/if}

            <dl class="figures">
                <div>
                    <dt>CPU now</dt>
                    <dd>{cpuShare(s.cpu_percent)}</dd>
                </div>
                <div>
                    <dt>Memory now</dt>
                    <dd>{size(s.memory_bytes)}</dd>
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

            <section>
                <h3>CPU, last hour</h3>
                {#if chart}<svg
                        class="chart"
                        viewBox="0 0 300 58"
                        preserveAspectRatio="none"
                        role="img"
                        aria-label={`CPU use of this conversation, peaking at ${cpuShare(detail.peak_cpu_percent)}`}
                        ><polygon points={chart.area} /><polyline points={chart.line} /></svg
                    >
                    <p class="caption">
                        Since {clock(chart.start)} · scale to {Math.round(chart.top)}% of this Mac
                    </p>{:else}<p class="quiet">
                        Jitterbug started watching this conversation moments ago. The chart fills in
                        as it runs.
                    </p>{/if}
            </section>

            <section>
                <h3>Heaviest tool calls</h3>
                {#each detail.heaviest as call (call.started_at + call.label)}
                    <div class="call heavy">
                        <span class="label"
                            ><strong>{call.label}</strong><small>{cost(call)}</small></span
                        >
                        <span class="score" title="CPU time, in seconds of one core"
                            >{Math.round(call.cpu_seconds)} CPU-s</span
                        >
                    </div>
                {:else}<p class="quiet">
                        {detail.calls_available
                            ? 'Nothing heavy measured yet. Calls that keep the CPU busy (builds, tests, installs) rank here.'
                            : `${s.harness_name} doesn’t keep a log Jitterbug can read tool calls from.`}
                    </p>{/each}
            </section>

            {#if detail.heavy.length}<section>
                    <h3>Tests and builds that took turns</h3>
                    {#each detail.heavy as item (item.started_at + item.command)}
                        <div class="call">
                            <span class="label"
                                ><code>{item.command}</code><small
                                    >ran {span(item.ended_at - item.started_at)}{item.waited_seconds
                                        ? ` after waiting ${span(item.waited_seconds)}`
                                        : ''} · peak {cpuShare(item.peak_cpu_percent)} CPU, {size(
                                        item.peak_memory_bytes
                                    )}</small
                                ></span
                            >
                            <span class="time">{clock(item.ended_at)}</span>
                        </div>
                    {/each}
                </section>{/if}

            {#if detail.calls.length}<section>
                    <h3>Recent tool calls</h3>
                    {#each detail.calls.slice(0, 15) as call (call.started_at + call.label)}
                        <div class="call">
                            <span class="label"
                                ><strong>{call.label}</strong><small
                                    >{call.ended_at === null ? 'running' : cost(call)}</small
                                ></span
                            >
                            <span class="time">{clock(call.started_at)}</span>
                        </div>
                    {/each}
                </section>{/if}

            {#if detail.processes.length}<section>
                    <h3>Processes</h3>
                    {#each detail.processes as p (p.pid)}
                        <div class="call">
                            <span class="label"
                                ><strong>{p.name}</strong><small>PID {p.pid}</small></span
                            >
                            <span class="time"
                                >{cpuShare(p.cpu_percent)} · {size(p.memory_bytes)}</span
                            >
                        </div>
                    {/each}
                </section>{/if}
            <p class="footnote">
                Measured on this Mac. Tool-call figures cover the whole conversation while each call
                ran, so calls running side by side share them.
            </p>
        </div>
    {:else if !error}<p class="quiet pad">Reading this conversation…</p>{/if}
</div>

<style>
    .session-sheet {
        position: fixed;
        top: 0;
        right: 0;
        bottom: 0;
        width: min(420px, calc(100vw - 240px));
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
    .body {
        flex: 1;
        overflow: auto;
        padding: 0 18px 20px;
    }
    .status {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 0 4px;
        margin: 0;
        font-size: 13px;
    }
    .status span {
        color: var(--muted);
    }
    .status i {
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: var(--faint);
        margin-right: 6px;
    }
    .status i.live {
        background: var(--brand);
    }
    .request {
        margin: 6px 0 0;
        color: var(--muted);
        font-size: 12px;
        line-height: 1.45;
        display: -webkit-box;
        -webkit-line-clamp: 2;
        line-clamp: 2;
        -webkit-box-orient: vertical;
        overflow: hidden;
    }
    .asks {
        margin: 8px 0 0;
        padding: 8px 10px;
        border-radius: 8px;
        background: var(--accent-soft);
        font-size: 13px;
        line-height: 1.45;
    }
    .figures {
        display: grid;
        grid-template-columns: repeat(4, 1fr);
        gap: 8px;
        margin: 16px 0 4px;
    }
    .figures dt {
        font-size: 11px;
        color: var(--muted);
    }
    .figures dd {
        margin: 2px 0 0;
        font-size: 17px;
        font-weight: 600;
        font-variant-numeric: tabular-nums;
    }
    section {
        margin-top: 20px;
    }
    h3 {
        margin: 0 0 8px;
        font-size: 12px;
        font-weight: 600;
        color: var(--muted);
    }
    .chart {
        display: block;
        width: 100%;
        height: 64px;
    }
    .chart polygon {
        fill: var(--brand);
        opacity: 0.16;
    }
    .chart polyline {
        fill: none;
        stroke: var(--brand);
        stroke-width: 1.5;
        vector-effect: non-scaling-stroke;
    }
    .caption,
    .footnote {
        margin: 4px 0 0;
        font-size: 11px;
        color: var(--faint);
    }
    .footnote {
        margin-top: 20px;
    }
    .call {
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
    .label strong,
    .label code {
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
    .score,
    .time {
        flex: none;
        font-size: 12px;
        color: var(--muted);
        font-variant-numeric: tabular-nums;
    }
    .heavy .score {
        color: var(--brand);
        font-weight: 600;
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
</style>
