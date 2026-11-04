<script module lang="ts">
    import type { Scene } from '../lib/types';
    // Survives navigating away from Monitor and back; the backend also caches by signature.
    let saved: { scene: Scene; settings: string } | null = null;
    // Each distinct reading becomes a line in tonight's journal: what Jitterbug noticed, when.
    let journal: { at: number; headline: string }[] = [];
</script>

<script lang="ts">
    import { invoke } from '@tauri-apps/api/core';
    import { onDestroy, untrack } from 'svelte';
    import type { Snapshot } from '../lib/types';
    import { memory, percent, ago, errorMessage } from '../lib/format';
    import { queueInference, cancelled } from '../lib/monitor';
    import Logo from './Logo.svelte';
    let {
        snapshot,
        settingsKey,
        activity = $bindable(null),
        onfocus,
        onask,
        onusage,
        onscene = () => {},
        compact = false,
    }: {
        snapshot: Snapshot;
        settingsKey: string;
        activity?: string | null;
        onfocus: (workloadId: string) => void;
        onask: (question: string) => void;
        onusage: () => void;
        onscene?: (scene: Scene) => void;
        compact?: boolean;
    } = $props();

    let scene = $state<Scene | null>(
        untrack(() => (saved?.settings === settingsKey ? saved.scene : null))
    );
    let busy = $state(false);
    let entries = $state(journal);
    let showJournal = $state(false);
    let error = $state('');
    let disposed = false;
    let now = $state(Date.now() / 1000);
    untrack(() => {
        if (scene) queueMicrotask(() => onscene(scene!));
    });
    const clock = setInterval(() => (now = Date.now() / 1000), 15_000);
    onDestroy(() => {
        disposed = true;
        clearInterval(clock);
    });

    const used = $derived(snapshot.memory_total_bytes - snapshot.memory_available_bytes);
    const total = $derived(Math.max(snapshot.memory_total_bytes, 1));
    const shades = [
        'var(--data-1)',
        'var(--data-2)',
        'var(--data-3)',
        'var(--data-4)',
        'var(--data-5)',
        'var(--data-6)',
    ];
    // Segments: each activity's resident memory, scaled so the bar never claims more than is in use.
    const segments = $derived.by(() => {
        if (!scene) return [];
        const listed = scene.activities.reduce((sum, a) => sum + a.memory_bytes, 0);
        const scale = listed > used ? used / listed : 1;
        return scene.activities.map((a, i) => ({
            ...a,
            share: ((a.memory_bytes * scale) / total) * 100,
            color: shades[Math.min(i, shades.length - 1)],
        }));
    });
    const accounted = $derived(segments.reduce((sum, s) => sum + s.share, 0));
    const usedShare = $derived((used / total) * 100);
    const pressure = $derived(snapshot.pressure.level);
    const ready = $derived(snapshot.sample_ready);

    async function read(force = false) {
        if (busy || !snapshot.sample_ready) return;
        busy = true;
        error = '';
        const key = settingsKey;
        try {
            const next = await queueInference(
                () => invoke<Scene>('read_scene', { force }),
                force ? 6 : 3,
                () => !disposed && key === settingsKey
            );
            if (disposed || key !== settingsKey) return;
            scene = next;
            saved = { scene: next, settings: key };
            if (journal.at(-1)?.headline !== next.headline) {
                journal = [...journal, { at: next.generated_at, headline: next.headline }].slice(
                    -12
                );
                entries = journal;
            }
            onscene(next);
            if (activity && !next.activities.some((a) => a.name === activity)) activity = null;
            onusage();
        } catch (cause) {
            if (!disposed && cause !== cancelled) error = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
    // Event-driven: ask again when the coarse picture may have changed. The backend returns
    // its cached reading for the same signature, so this costs nothing while things are steady.
    $effect(() => {
        settingsKey;
        if (!ready) return;
        const first = untrack(() => !scene);
        const timer = setTimeout(() => void read(), first ? 400 : 0);
        const repeat = setInterval(() => void read(), 90_000);
        return () => {
            clearTimeout(timer);
            clearInterval(repeat);
        };
    });
</script>

<section class="reading" class:compact aria-label="What your Mac is doing" aria-busy={busy}>
    <div class="reading-main">
        <div class="headline-block">
            {#if scene}
                <p class="headline selectable">
                    {#if scene.focus_workload_id}<button
                            class="headline-link"
                            onclick={() => onfocus(scene!.focus_workload_id!)}
                            title="Open in the inspector">{scene.headline}</button
                        >{:else}{scene.headline}{/if}
                </p>
                <p class="detail">{scene.detail}</p>
                {#if scene.suggestion}<div class="suggestion">
                        <span>{scene.suggestion}</span>
                        <button
                            class="link"
                            onclick={() =>
                                onask(
                                    `${scene!.headline}. ${scene!.suggestion} How should I do that?`
                                )}>Ask how</button
                        >
                    </div>{/if}
            {:else if error}
                <p class="headline">Jitterbug couldn’t read your Mac just now</p>
                <p class="detail">{error}</p>
            {:else}
                <p class="headline reading-wait"><Logo size={18} live /> Reading what’s running…</p>
                <p class="detail">
                    Meta is grouping {snapshot.workloads.length} apps and tools into what you’re doing.
                </p>
            {/if}
        </div>
        <dl class="vitals">
            <div>
                <dt>CPU</dt>
                <dd>{snapshot.sample_ready ? percent(snapshot.cpu_percent) : '—'}</dd>
                <small>{snapshot.logical_cpus} cores</small>
            </div>
            <div>
                <dt>Memory</dt>
                <dd>{memory(used)}</dd>
                <small>of {memory(snapshot.memory_total_bytes)}</small>
            </div>
            <div>
                <dt>Pressure</dt>
                <dd class:strained={pressure !== 'normal'}>
                    <i></i>{pressure === 'normal'
                        ? 'Normal'
                        : pressure === 'watch'
                          ? 'Elevated'
                          : 'High'}
                </dd>
                <small
                    >{snapshot.platform.thermal === 'nominal' || !snapshot.platform.thermal
                        ? 'Cool'
                        : `Thermal ${snapshot.platform.thermal}`}{snapshot.swap_used_bytes > 0
                        ? ` · ${memory(snapshot.swap_used_bytes)} swap`
                        : ''}</small
                >
            </div>
        </dl>
    </div>

    <div class="memory-map">
        <div
            class="bar"
            role="img"
            aria-label={`Memory in use by activity: ${memory(used)} of ${memory(snapshot.memory_total_bytes)}`}
        >
            {#if segments.length}
                {#each segments as s (s.name)}<button
                        class="segment"
                        class:dim={activity && activity !== s.name}
                        style={`width:${s.share}%;background:${s.color}`}
                        title={`${s.name} · ${memory(s.memory_bytes)}`}
                        aria-label={`Show ${s.name}`}
                        onclick={() => (activity = activity === s.name ? null : s.name)}
                    ></button>{/each}
                <span
                    class="segment other"
                    style={`width:${Math.max(usedShare - accounted, 0)}%`}
                    title="macOS, caches and small processes"
                ></span>
            {:else}
                <span class="segment pending" style={`width:${usedShare}%`}></span>
            {/if}
        </div>
        <div class="legend">
            {#each segments as s (s.name)}<button
                    class="activity"
                    class:active={activity === s.name}
                    aria-pressed={activity === s.name}
                    title={s.summary}
                    onclick={() => (activity = activity === s.name ? null : s.name)}
                    ><i style={`background:${s.color}`}></i><span class="name">{s.name}</span><span
                        class="amount">{memory(s.memory_bytes)}</span
                    ></button
                >{/each}
            {#if segments.length}<span class="activity static"
                    ><i class="other-dot"></i><span class="name">macOS &amp; caches</span><span
                        class="amount"
                        >{memory(
                            Math.max(
                                used -
                                    segments.reduce((sum, s) => sum + (s.share / 100) * total, 0),
                                0
                            )
                        )}</span
                    ></span
                ><span class="activity static"
                    ><i class="free-dot"></i><span class="name">Free</span><span class="amount"
                        >{memory(snapshot.memory_available_bytes)}</span
                    ></span
                >{/if}
        </div>
        {#if activity}
            {@const chosen = segments.find((s) => s.name === activity)}
            {#if chosen}<p class="activity-summary">
                    {chosen.summary}
                </p>{/if}
        {/if}
    </div>
    <footer class="provenance">
        {#if scene}<span
                >Read by {scene.provider === 'meta' ? 'Meta' : scene.provider} · {scene.model} · {ago(
                    scene.generated_at,
                    now
                )}</span
            >{/if}
        <span class="spacer"></span>
        {#if entries.length > 1}<button
                class="link"
                aria-expanded={showJournal}
                onclick={() => (showJournal = !showJournal)}
                >{showJournal ? 'Hide earlier' : `Earlier (${entries.length - 1})`}</button
            >{/if}
        <button
            class="link"
            disabled={busy || !snapshot.sample_ready}
            onclick={() => void read(true)}
            >{busy ? 'Reading…' : scene ? 'Read again' : 'Read now'}</button
        >
    </footer>
    {#if showJournal && entries.length > 1}<ol class="journal">
            {#each entries.slice(0, -1).reverse() as entry}<li>
                    <time
                        >{new Date(entry.at * 1000).toLocaleTimeString([], {
                            hour: 'numeric',
                            minute: '2-digit',
                        })}</time
                    ><span>{entry.headline}</span>
                </li>{/each}
        </ol>{/if}
</section>

<style>
    .reading {
        padding: 20px 28px 16px;
        display: grid;
        gap: 16px;
        flex: none;
        box-shadow: inset 0 -1px 0 var(--rule);
        container-type: inline-size;
    }
    /* Beside the inspector: the finding and the map stay, the explanation folds away. */
    .compact {
        padding: 14px 24px 12px;
        gap: 10px;
    }
    .compact .detail,
    .compact .suggestion,
    .compact .legend,
    .compact .activity-summary,
    .compact .provenance,
    .compact .vitals small {
        display: none;
    }
    .compact .headline-link {
        display: block;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .compact .headline {
        font-size: 15px;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .compact .vitals {
        gap: 18px;
    }
    .compact .vitals div {
        min-width: 0;
    }
    .compact .vitals dt {
        font-size: 11px;
    }
    .compact .vitals dd {
        font-size: 14px;
    }
    .compact .reading-main {
        align-items: center;
    }
    .compact .bar {
        height: 8px;
    }
    .reading-main {
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto;
        gap: 32px;
        align-items: start;
    }
    .headline-block {
        min-width: 0;
        max-width: 760px;
    }
    .headline {
        font-size: 20px;
        line-height: 1.3;
        font-weight: 650;
        letter-spacing: -0.02em;
        margin: 0;
        text-wrap: balance;
    }
    .headline-link {
        all: unset;
        cursor: pointer;
        text-decoration: underline dotted;
        text-decoration-color: #ffffff40;
        text-underline-offset: 5px;
        text-decoration-thickness: 1.5px;
    }
    .headline-link:hover {
        text-decoration-color: var(--brand);
    }
    .headline-link:focus-visible {
        outline: 3px solid #4c9dff80;
        border-radius: 4px;
    }
    .reading-wait {
        display: flex;
        align-items: center;
        gap: 10px;
        color: var(--muted);
    }
    .detail {
        margin: 6px 0 0;
        color: var(--muted);
        font-size: 13px;
        text-wrap: pretty;
    }
    .suggestion {
        margin-top: 10px;
        display: flex;
        flex-wrap: wrap;
        gap: 4px 12px;
        align-items: baseline;
        font-size: 13px;
    }
    .suggestion span {
        color: var(--text);
    }
    .vitals {
        display: grid;
        grid-template-columns: repeat(3, auto);
        gap: 28px;
        margin: 0;
    }
    .vitals div {
        display: grid;
        gap: 2px;
        min-width: 76px;
    }
    .vitals dt {
        font-size: 12px;
        color: var(--muted);
    }
    .vitals dd {
        margin: 0;
        font-size: 20px;
        font-weight: 600;
        letter-spacing: -0.02em;
        font-variant-numeric: tabular-nums;
        display: flex;
        align-items: center;
        gap: 7px;
        white-space: nowrap;
    }
    .vitals dd i {
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: var(--brand);
    }
    .vitals dd.strained i {
        background: var(--warning);
    }
    .vitals small {
        font-size: 12px;
        color: var(--faint);
        white-space: nowrap;
    }
    .memory-map {
        display: grid;
        gap: 10px;
    }
    .bar {
        display: flex;
        height: 12px;
        border-radius: 6px;
        overflow: hidden;
        background: var(--track);
        gap: 2px;
    }
    .segment {
        all: unset;
        display: block;
        height: 100%;
        min-width: 3px;
        cursor: pointer;
        transition:
            opacity 160ms,
            width 400ms cubic-bezier(0.2, 0.8, 0.2, 1);
    }
    .segment:focus-visible {
        outline: 2px solid var(--focus);
        outline-offset: 1px;
    }
    .segment.dim {
        opacity: 0.28;
    }
    .segment.other {
        background: #ffffff2e;
        cursor: default;
    }
    .segment.pending {
        background: #ffffff2e;
        cursor: default;
    }
    .legend {
        display: flex;
        flex-wrap: wrap;
        gap: 2px 4px;
        margin-left: -8px;
    }
    .activity {
        display: inline-flex;
        align-items: center;
        gap: 7px;
        min-height: 26px;
        padding: 2px 8px;
        border-radius: 6px;
        background: transparent;
        box-shadow: none;
        font-size: 12px;
        font-weight: 450;
        color: var(--text);
    }
    button.activity:hover:not(:disabled) {
        background: var(--hover);
    }
    .activity.active {
        background: var(--selected);
    }
    .activity i {
        width: 8px;
        height: 8px;
        border-radius: 2px;
        flex: none;
    }
    .activity .amount {
        color: var(--muted);
        font-variant-numeric: tabular-nums;
    }
    .activity.static {
        color: var(--muted);
    }
    .other-dot {
        background: #ffffff2e;
    }
    .free-dot {
        background: var(--track);
        box-shadow: inset 0 0 0 1px #ffffff26;
    }
    .activity-summary {
        margin: 0;
        font-size: 13px;
        color: var(--muted);
        display: flex;
        flex-wrap: wrap;
        gap: 4px 12px;
        align-items: baseline;
    }
    .provenance {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 12px;
        font-size: 11px;
        color: var(--faint);
        margin-top: -6px;
    }
    .provenance .link {
        font-size: 11px;
    }
    .spacer {
        flex: 1;
    }
    .journal {
        list-style: none;
        margin: -4px 0 0;
        padding: 0;
        display: grid;
        gap: 4px;
        font-size: 12px;
        color: var(--muted);
    }
    .journal li {
        display: grid;
        grid-template-columns: 64px minmax(0, 1fr);
        gap: 8px;
    }
    .journal time {
        color: var(--faint);
        font-variant-numeric: tabular-nums;
    }
    .compact .journal {
        display: none;
    }
    @container (max-width: 760px) {
        .reading:not(.compact) .reading-main {
            grid-template-columns: minmax(0, 1fr);
            gap: 14px;
        }
    }
</style>
