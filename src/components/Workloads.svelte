<script lang="ts">
    import { invoke } from '@tauri-apps/api/core';
    import { untrack, onDestroy } from 'svelte';
    import type {
        AppProfile,
        ProcessRecord,
        Snapshot,
        Workload,
        WorkloadDetails,
    } from '../lib/types';
    import { memory, percent, errorMessage } from '../lib/format';
    import {
        identityKey,
        queueInference,
        cancelled,
        type Inventory,
        type Query,
        type MonitorRow,
        type Explanation,
        type Identity,
    } from '../lib/monitor';
    import { appFallback, processRole } from '../lib/monitor-copy';
    import AppIcon from './AppIcon.svelte';
    import Logo from './Logo.svelte';
    let {
        snapshot,
        onselect,
        selectedId = null,
        selectedProcess = null,
        settingsKey = '',
        pauseEnrichment = false,
        view = 'apps',
        search = '',
        filter = 'all',
        workloadIds = null,
        activityName = null,
        aiAnswer = null,
        onclearactivity = () => {},
        onusage,
    }: {
        snapshot: Snapshot;
        onselect: (workload: Workload, process?: ProcessRecord | null) => void;
        selectedId?: string | null;
        selectedProcess?: ProcessRecord | null;
        settingsKey?: string;
        pauseEnrichment?: boolean;
        view?: string;
        search?: string;
        filter?: string;
        workloadIds?: string[] | null;
        activityName?: string | null;
        aiAnswer?: string | null;
        onclearactivity?: () => void;
        onusage: () => void;
    } = $props();
    let sort = $state('impact');
    let offset = $state(0);
    let inventory = $state<Inventory | null>(null);
    let loading = $state(false);
    let error = $state('');
    let expanded = $state<string | null>(null);
    let members = $state<ProcessRecord[]>([]);
    let memberError = $state('');
    let profileError = $state('');
    let explaining = $state(false);
    let profiles = $state<Record<string, AppProfile>>({});
    let scroller: HTMLDivElement | undefined = $state();
    let attempted = new Set<string>();
    let generation = 0;
    let disposed = false;
    let requestId = 0;
    let refresh = $state(0);
    let lastQuery = '';
    let inventoryTimer: ReturnType<typeof setTimeout>;
    let enrichmentTimer: ReturnType<typeof setTimeout>;
    const rows = $derived(inventory?.rows ?? []);
    const query = $derived<Query>({
        search,
        sort,
        view,
        filter,
        offset,
        workloads: workloadIds ?? [],
    });
    const queryKey = $derived(
        JSON.stringify([snapshot.source, search, sort, view, filter, offset, refresh, workloadIds])
    );
    const rowKey = (row: MonitorRow) => (row.process ? identityKey(row.process) : row.workload.id);
    onDestroy(() => {
        disposed = true;
        generation++;
        clearTimeout(enrichmentTimer);
        clearTimeout(inventoryTimer);
    });
    $effect(() => {
        search;
        sort;
        view;
        filter;
        workloadIds;
        untrack(() => {
            offset = 0;
            expanded = null;
        });
    });
    $effect(() => {
        snapshot.source;
        settingsKey;
        untrack(() => {
            generation++;
            profiles = {};
            attempted = new Set();
            profileError = '';
        });
    });
    $effect(() => {
        const sample = snapshot;
        const next = query;
        const key = queryKey;
        untrack(() => {
            const id = ++requestId;
            const changed = key !== lastQuery;
            if (changed) {
                loading = true;
                scroller?.scrollTo({ top: 0 });
            }
            const pinned = changed ? [] : rows.map(rowKey);
            lastQuery = key;
            clearTimeout(inventoryTimer);
            inventoryTimer = setTimeout(
                async () => {
                    try {
                        const result = await invoke<Inventory>('get_monitor_inventory', {
                            query: { ...next, pinned },
                        });
                        if (id !== requestId || disposed) return;
                        inventory = result;
                        error = '';
                        clearTimeout(enrichmentTimer);
                        enrichmentTimer = setTimeout(() => void enrich(), 350);
                    } catch (cause) {
                        if (id === requestId) error = errorMessage(cause);
                    } finally {
                        if (id === requestId) loading = false;
                    }
                },
                changed && search ? 160 : 0
            );
            // Request sequencing retains the displayed inventory while telemetry refreshes.
            // No loading-state opacity, disabled rows, or sort changes on periodic samples.
        });
    });
    $effect(() => {
        const id = expanded;
        const sample = snapshot;
        let active = true;
        if (!id) return;
        void invoke<WorkloadDetails>('get_workload_details', {
            workloadId: id,
            pid: null,
            startTime: null,
        })
            .then((detail) => {
                if (active) {
                    const order = new Map(members.map((p, i) => [identityKey(p), i]));
                    members = [...detail.members].sort(
                        (a, b) =>
                            (order.get(identityKey(a)) ?? 1000) -
                            (order.get(identityKey(b)) ?? 1000)
                    );
                    memberError = '';
                }
            })
            .catch((cause) => {
                if (active) memberError = errorMessage(cause);
            });
        return () => {
            active = false;
        };
    });
    function profile(row: MonitorRow) {
        return (
            (row.identity ? profiles[identityKey(row.identity)] : null) ??
            (row.profile?.source !== 'inference' ? row.profile : null)
        );
    }
    async function enrich() {
        if (pauseEnrichment || explaining || disposed || !snapshot.sample_ready || document.hidden)
            return;
        const start = Math.max(0, Math.floor((scroller?.scrollTop ?? 0) / 66) - 1);
        const candidates = rows
            .slice(start, start + 16)
            .filter(
                (row) =>
                    row.identity &&
                    profile(row)?.source !== 'model' &&
                    !attempted.has(identityKey(row.identity))
            )
            .slice(0, 6);
        const identities = candidates.map((r) => r.identity!);
        if (!identities.length) return;
        const current = generation;
        explaining = true;
        identities.forEach((p) => attempted.add(identityKey(p)));
        const valid = () => !disposed && current === generation && !pauseEnrichment;
        try {
            const result = await queueInference(
                () =>
                    invoke<Explanation[]>('explain_monitor_profiles', {
                        identities,
                        generate: true,
                    }),
                -1,
                valid
            );
            if (valid()) {
                for (const item of result) profiles[identityKey(item.identity)] = item.profile;
                profileError = '';
            }
            onusage();
        } catch (cause) {
            identities.forEach((id) => attempted.delete(identityKey(id)));
            if (
                valid() &&
                cause !== cancelled &&
                !/deferred for your question/.test(errorMessage(cause))
            )
                profileError = errorMessage(cause);
        } finally {
            explaining = false;
            if (Object.keys(profiles).length > 240)
                profiles = Object.fromEntries(Object.entries(profiles).slice(-240));
            if (attempted.size > 240) attempted = new Set([...attempted].slice(-240));
            if (valid() && !profileError) enrichmentTimer = setTimeout(() => void enrich(), 100);
        }
    }
    function description(row: MonitorRow): string {
        return (
            profile(row)?.text ??
            (row.process ? processRole(row.process, row.workload.label) : appFallback(row.workload))
        );
    }
    function identified(row: MonitorRow) {
        return !!profile(row) || !!row.process || row.workload.category === 'project';
    }
    // "Opened by you" style origins are the default for apps; only show launchers worth knowing.
    function launcher(row: MonitorRow) {
        const origin = row.workload.origin;
        if (!origin || /opened by you|^started by launchd$/i.test(origin)) return null;
        return origin;
    }
    function orderBy(value: string) {
        sort = value;
        refresh++;
    }
</script>

<section class="activity-list" data-tour="workloads" aria-label="Activity monitor">
    {#if aiAnswer}<div class="activity-filter answer" role="status">
            <Logo size={14} /><span>{aiAnswer}</span><button class="link" onclick={onclearactivity}
                >Show everything</button
            >
        </div>{:else if activityName}<div class="activity-filter">
            <span>Showing <strong>{activityName}</strong></span><button
                class="link"
                onclick={onclearactivity}>Show everything</button
            >
        </div>{/if}
    <div class="table-head" role="row">
        <button class:active={sort === 'name'} onclick={() => orderBy('name')}
            >{view === 'apps' ? 'App' : 'Process'}<i>{sort === 'name' ? '▴' : ''}</i></button
        ><button class:active={sort === 'cpu'} onclick={() => orderBy('cpu')}
            >CPU<i>{sort === 'cpu' ? '▾' : ''}</i></button
        ><button class:active={sort === 'memory'} onclick={() => orderBy('memory')}
            >Memory<i>{sort === 'memory' ? '▾' : ''}</i></button
        >
    </div>
    <div class="activity-scroll" bind:this={scroller} onscroll={() => void enrich()}>
        {#each rows as row (rowKey(row))}
            {@const readings = row.process ?? row.workload}
            {@const chosen =
                selectedId === row.workload.id &&
                (!row.process ||
                    (selectedProcess?.pid === row.process.pid &&
                        selectedProcess.start_time === row.process.start_time))}
            {@const share =
                (readings.memory_bytes / Math.max(snapshot.memory_total_bytes, 1)) * 100}
            <div class="activity-row" class:chosen>
                {#if !row.process && row.workload.process_count > 1}<button
                        class="expand"
                        class:open={expanded === row.workload.id}
                        aria-label={`Show processes for ${row.workload.label}`}
                        aria-expanded={expanded === row.workload.id}
                        onclick={() => {
                            members = [];
                            expanded = expanded === row.workload.id ? null : row.workload.id;
                        }}
                        ><svg width="9" height="9" viewBox="0 0 10 10" aria-hidden="true"
                            ><path
                                d="M3.5 1.5 7 5l-3.5 3.5"
                                fill="none"
                                stroke="currentColor"
                                stroke-width="1.6"
                                stroke-linecap="round"
                                stroke-linejoin="round"
                            /></svg
                        ></button
                    >{:else}<span class="indent"></span>{/if}
                <button
                    class="inspect"
                    aria-pressed={chosen}
                    onclick={() => onselect(row.workload, row.process)}
                >
                    <span class="identity"
                        ><AppIcon
                            workloadId={row.workload.id}
                            category={row.workload.category}
                            label={row.workload.label}
                            size={30}
                        /><span class="identity-copy"
                            ><span class="name-line"
                                ><strong>{row.process?.name ?? row.workload.label}</strong
                                >{#if row.process}<span class="meta"
                                        >{row.workload.label} · PID {row.process.pid}</span
                                    >{:else if row.workload.process_count > 1}<span class="meta"
                                        >{row.workload.process_count} processes</span
                                    >{/if}{#if launcher(row)}<span class="origin"
                                        >{launcher(row)}</span
                                    >{/if}</span
                            >{#if identified(row)}<span class="purpose" title={description(row)}
                                    >{description(row)}</span
                                >{:else}<span class="purpose pending">Identifying with Meta…</span
                                >{/if}</span
                        ></span
                    >
                    <span class="numeric cpu" class:hot={readings.cpu_percent >= 25}
                        >{snapshot.sample_ready ? percent(readings.cpu_percent) : '—'}</span
                    ><span class="numeric mem"
                        ><span>{memory(readings.memory_bytes)}</span><i
                            ><b style={`width:${Math.min(share * 4, 100)}%`}></b></i
                        ></span
                    >
                </button>
            </div>
            {#if expanded === row.workload.id && !row.process}<div
                    class="children"
                    role="group"
                    aria-label={`${row.workload.label} processes`}
                >
                    {#if memberError}<p class="error">{memberError}</p>{/if}
                    {#each members.slice(0, 20) as p (identityKey(p))}<button
                            class="child-row"
                            class:chosen={selectedProcess?.pid === p.pid &&
                                selectedProcess.start_time === p.start_time}
                            onclick={() => onselect(row.workload, p)}
                            ><span
                                ><strong>{p.name}</strong><small
                                    >{processRole(p, row.workload.label)} · PID {p.pid}</small
                                ></span
                            ><span class="numeric">{percent(p.cpu_percent)}</span><span
                                class="numeric">{memory(p.memory_bytes)}</span
                            ></button
                        >{:else}<p class="empty">Reading processes…</p>{/each}
                    {#if members.length > 20}<p class="more">
                            {members.length - 20} more in the inspector’s Processes tab.
                        </p>{/if}
                </div>{/if}
        {:else}<p class="empty">
                {loading
                    ? 'Reading activity…'
                    : search
                      ? `Nothing running matches “${search}”.`
                      : 'No matching activity.'}
            </p>{/each}
        {#if error}<p class="error">{error}</p>{/if}
    </div>
    <footer>
        <span
            >{inventory
                ? `${inventory.total} ${view === 'apps' ? 'apps and tools' : 'processes'} · ${inventory.system_processes} processes on this Mac`
                : 'Reading…'}{explaining ? ' · Meta is identifying apps' : ''}{profileError
                ? ' · Descriptions paused'
                : ''}</span
        >
        <div>
            {#if profileError}<button
                    class="link"
                    title={profileError}
                    onclick={() => {
                        attempted.clear();
                        profileError = '';
                        void enrich();
                    }}>Retry descriptions</button
                >{/if}
            {#if (inventory?.total ?? 0) > 40}<button
                    class="icon-button"
                    aria-label="Previous page"
                    disabled={!inventory?.offset}
                    onclick={() => (offset = Math.max((inventory?.offset ?? 0) - 40, 0))}>‹</button
                ><span class="numeric"
                    >{(inventory?.offset ?? 0) + (rows.length ? 1 : 0)}–{(inventory?.offset ?? 0) +
                        rows.length}</span
                ><button
                    class="icon-button"
                    aria-label="Next page"
                    disabled={(inventory?.offset ?? 0) + rows.length >= (inventory?.total ?? 0)}
                    onclick={() => (offset = (inventory?.offset ?? 0) + 40)}>›</button
                >{/if}
        </div>
    </footer>
</section>

<style>
    .activity-list {
        flex: 1;
        display: flex;
        flex-direction: column;
        min-height: 0;
        min-width: 0;
    }
    .activity-filter {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 10px 28px 0;
        font-size: 12px;
        color: var(--muted);
    }
    .activity-filter.answer {
        color: var(--text);
        font-size: 13px;
    }
    .activity-filter strong {
        color: var(--text);
        font-weight: 600;
    }
    .table-head {
        display: grid;
        grid-template-columns: minmax(0, 1fr) 72px 128px;
        padding: 8px 28px 6px 54px;
        flex-shrink: 0;
        box-shadow: inset 0 -1px 0 var(--rule);
    }
    .table-head button {
        text-align: right;
        background: none;
        box-shadow: none;
        color: var(--muted);
        font-size: 12px;
        font-weight: 500;
        padding: 0;
        min-height: 22px;
        display: flex;
        justify-content: flex-end;
        align-items: center;
        gap: 4px;
    }
    .table-head button:hover:not(:disabled) {
        background: none;
        color: var(--text);
    }
    .table-head button:first-child {
        justify-content: flex-start;
    }
    .table-head button.active {
        color: var(--text);
    }
    .table-head i {
        font-style: normal;
        font-size: 10px;
        width: 8px;
    }
    .activity-scroll {
        flex: 1;
        min-height: 0;
        overflow: auto;
        overflow-anchor: none;
        padding: 4px 16px 8px;
    }
    .activity-row {
        display: flex;
        align-items: stretch;
        min-height: 54px;
        border-radius: 8px;
    }
    .activity-row:hover {
        background: var(--hover);
    }
    .activity-row.chosen,
    .child-row.chosen {
        background: var(--selected);
    }
    .expand,
    .indent {
        width: 26px;
        flex: 0 0 26px;
    }
    .expand {
        padding: 0;
        min-height: 0;
        background: none;
        box-shadow: none;
        color: var(--faint);
        display: grid;
        place-items: center;
    }
    .expand:hover:not(:disabled) {
        background: none;
        color: var(--text);
    }
    .expand svg {
        transition: transform 140ms;
    }
    .expand.open svg {
        transform: rotate(90deg);
    }
    .inspect {
        flex: 1;
        min-width: 0;
        display: grid;
        grid-template-columns: minmax(0, 1fr) 72px 128px;
        align-items: center;
        gap: 0;
        text-align: left;
        padding: 8px 12px 8px 0;
        background: none;
        box-shadow: none;
        border-radius: 0;
        font-weight: 400;
    }
    .inspect:hover:not(:disabled),
    .inspect:active:not(:disabled) {
        background: none;
    }
    .identity {
        display: flex;
        align-items: center;
        gap: 11px;
        min-width: 0;
        padding-right: 16px;
    }
    .identity-copy {
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 2px;
    }
    .name-line {
        display: flex;
        align-items: baseline;
        gap: 8px;
        min-width: 0;
    }
    strong {
        font-size: 13px;
        font-weight: 600;
        overflow: hidden;
        white-space: nowrap;
        text-overflow: ellipsis;
        flex: none;
        max-width: 60%;
    }
    .meta {
        font-size: 12px;
        color: var(--faint);
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        flex: none;
    }
    .origin {
        font-size: 11px;
        color: var(--brand);
        background: var(--accent-soft);
        border-radius: 4px;
        padding: 0 6px;
        line-height: 18px;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        min-width: 0;
    }
    .purpose,
    small {
        font-size: 12px;
        line-height: 1.4;
        color: var(--muted);
        white-space: nowrap;
        text-overflow: ellipsis;
        overflow: hidden;
    }
    .purpose.pending {
        color: var(--faint);
        font-style: italic;
    }
    .numeric {
        text-align: right;
        font-size: 13px;
        font-variant-numeric: tabular-nums;
    }
    .cpu.hot {
        color: var(--warning);
        font-weight: 600;
    }
    .mem {
        display: flex;
        flex-direction: column;
        align-items: flex-end;
        gap: 5px;
    }
    .mem i {
        width: 72px;
        height: 3px;
        border-radius: 2px;
        background: var(--track);
        display: block;
        overflow: hidden;
    }
    .mem b {
        display: block;
        height: 100%;
        background: var(--data-2);
        border-radius: 2px;
    }
    .children {
        margin: 2px 0 6px 26px;
        padding: 4px 0;
        border-radius: 8px;
        background: #ffffff06;
    }
    .child-row {
        width: 100%;
        display: grid;
        grid-template-columns: minmax(0, 1fr) 72px 128px;
        padding: 7px 12px 7px 41px;
        min-height: 0;
        background: none;
        box-shadow: none;
        border-radius: 6px;
        text-align: left;
        gap: 0;
        align-items: center;
        font-weight: 400;
    }
    .child-row:hover:not(:disabled) {
        background: var(--hover);
    }
    .child-row > span:first-child {
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 1px;
        padding-right: 12px;
    }
    .child-row strong {
        font-weight: 500;
        max-width: 100%;
    }
    .more {
        font-size: 12px;
        color: var(--faint);
        margin: 4px 0 4px 41px;
    }
    .empty {
        padding: 28px 12px;
        color: var(--muted);
        font-size: 13px;
    }
    .error {
        font-size: 12px;
        padding: 12px;
        color: var(--danger);
    }
    footer {
        min-height: 34px;
        padding: 4px 20px 4px 28px;
        display: flex;
        gap: 8px;
        align-items: center;
        justify-content: space-between;
        flex-shrink: 0;
        color: var(--faint);
        font-size: 11px;
        box-shadow: inset 0 1px 0 var(--rule);
    }
    footer div {
        display: flex;
        align-items: center;
        gap: 4px;
    }
    footer .link {
        font-size: 11px;
    }
    footer .icon-button {
        width: 22px;
        height: 22px;
        min-height: 22px;
    }
    @media (max-width: 1180px) {
        .table-head,
        .inspect,
        .child-row {
            grid-template-columns: minmax(0, 1fr) 62px 96px;
        }
        .mem i {
            width: 56px;
        }
        .origin {
            display: none;
        }
    }
</style>
