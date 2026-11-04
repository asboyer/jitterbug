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
    let {
        snapshot,
        onselect,
        selectedId = null,
        selectedProcess = null,
        settingsKey = '',
        pauseEnrichment = false,
        onusage,
    }: {
        snapshot: Snapshot;
        onselect: (workload: Workload, process?: ProcessRecord | null) => void;
        selectedId?: string | null;
        selectedProcess?: ProcessRecord | null;
        settingsKey?: string;
        pauseEnrichment?: boolean;
        onusage: () => void;
    } = $props();
    let search = $state('');
    let sort = $state('impact');
    let view = $state('apps');
    let filter = $state('all');
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
    const query = $derived<Query>({ search, sort, view, filter, offset });
    const queryKey = $derived(
        JSON.stringify([snapshot.source, search, sort, view, filter, offset, refresh])
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
    function orderBy(value: string) {
        sort = value;
        refresh++;
    }
</script>

<section class="activity-list" data-tour="workloads" aria-label="Activity monitor">
    <div class="list-title">
        <h2>What’s running <span>{inventory?.total ?? '…'}</span></h2>
        <div class="view-switch" aria-label="Activity view">
            <button aria-pressed={view === 'apps'} onclick={() => (view = 'apps')}>Apps</button
            ><button aria-pressed={view === 'processes'} onclick={() => (view = 'processes')}
                >Processes</button
            >
        </div>
    </div>
    <div class="list-tools">
        <input
            aria-label="Filter workloads"
            placeholder="Search name or PID"
            maxlength="128"
            bind:value={search}
        /><select aria-label="Filter activity" bind:value={filter}
            ><option value="all">All</option><option value="apps">Applications</option><option
                value="system">System</option
            ></select
        ><button
            class="text-button reorder"
            onclick={() => refresh++}
            title="Re-sort using current readings"
            aria-label="Refresh order">↻</button
        >
    </div>
    <div class="table-head">
        <button class:active={sort === 'name'} onclick={() => orderBy('name')}
            >Name {sort === 'name' ? '↑' : ''}</button
        ><button class:active={sort === 'cpu'} onclick={() => orderBy('cpu')}
            >CPU {sort === 'cpu' ? '↓' : ''}</button
        ><button class:active={sort === 'memory'} onclick={() => orderBy('memory')}
            >Memory {sort === 'memory' ? '↓' : ''}</button
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
            <div class="activity-row" class:chosen>
                {#if !row.process}<button
                        class="expand"
                        aria-label={`Show processes for ${row.workload.label}`}
                        aria-expanded={expanded === row.workload.id}
                        onclick={() => {
                            members = [];
                            expanded = expanded === row.workload.id ? null : row.workload.id;
                        }}>{expanded === row.workload.id ? '⌄' : '›'}</button
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
                            size={28}
                        /><span class="identity-copy"
                            ><strong>{row.process?.name ?? row.workload.label}</strong><span
                                class="purpose"
                                title={description(row)}>{description(row)}</span
                            >{#if row.process}<small
                                    >{row.workload.label} · PID {row.process.pid}</small
                                >{/if}</span
                        ></span
                    >
                    <span class="numeric"
                        >{snapshot.sample_ready ? percent(readings.cpu_percent) : '—'}</span
                    ><span class="numeric">{memory(readings.memory_bytes)}</span>
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
                    {#if members.length > 20}<button
                            class="text-button"
                            onclick={() => {
                                view = 'processes';
                                search = row.workload.label;
                            }}>View all {members.length} processes</button
                        >{/if}
                </div>{/if}
        {:else}<p class="empty">{loading ? 'Reading activity…' : 'No matching activity.'}</p>{/each}
        {#if error}<p class="error">{error}</p>{/if}
    </div>
    <footer>
        <span
            >{explaining ? 'Identifying apps…' : 'Live readings'}{profileError
                ? ' · Descriptions paused'
                : ''}</span
        >
        <div>
            {#if profileError}<button
                    title={profileError}
                    onclick={() => {
                        attempted.clear();
                        profileError = '';
                        void enrich();
                    }}>Retry AI</button
                >{/if}<button
                aria-label="Previous page"
                disabled={!inventory?.offset}
                onclick={() => (offset = Math.max((inventory?.offset ?? 0) - 40, 0))}>‹</button
            ><span
                >{(inventory?.offset ?? 0) + (rows.length ? 1 : 0)}–{(inventory?.offset ?? 0) +
                    rows.length}</span
            ><button
                aria-label="Next page"
                disabled={(inventory?.offset ?? 0) + rows.length >= (inventory?.total ?? 0)}
                onclick={() => (offset = (inventory?.offset ?? 0) + 40)}>›</button
            >
        </div>
    </footer>
</section>

<style>
    .activity-list {
        display: flex;
        flex-direction: column;
        min-height: 0;
        min-width: 0;
        border: 0;
        border-radius: 0;
        overflow: hidden;
        background: transparent;
    }
    .list-title {
        display: flex;
        gap: 8px;
        align-items: center;
        justify-content: space-between;
        padding: 0 0 16px;
        flex-wrap: wrap;
    }
    h2 {
        margin: 0;
        font-size: 14px;
        font-weight: 600;
    }
    h2 span {
        color: var(--muted);
        font-size: 12px;
        font-weight: 400;
        margin-left: 6px;
    }
    .view-switch {
        display: flex;
        gap: 16px;
        padding: 0;
        background: transparent;
        border-radius: 0;
    }
    .view-switch button {
        min-height: 25px;
        padding: 3px 0 7px;
        font-size: 12px;
        border: 0;
        box-shadow: none;
        background: none;
        color: var(--muted);
        border-radius: 0;
    }
    .view-switch button[aria-pressed='true'] {
        background: transparent;
        color: var(--text);
        box-shadow: none;
    }
    .list-tools {
        display: flex;
        gap: 6px;
        padding: 0 0 12px;
    }
    input {
        flex: 1;
        width: 80px;
        font-size: 12px;
    }
    select {
        font-size: 12px;
        max-width: 100px;
    }
    .reorder {
        font-size: 18px;
        padding: 4px 7px;
    }
    .table-head {
        display: grid;
        grid-template-columns: minmax(0, 1fr) 62px 80px;
        padding: 0 12px 0 40px;

        flex-shrink: 0;
        background: transparent;
    }
    .table-head button {
        text-align: right;
        background: none;
        border: none;
        border-radius: 0;
        box-shadow: none;
        color: var(--muted);
        font-size: 12px;
        padding: 6px 0;
        min-height: 28px;
    }
    .table-head button:first-child {
        text-align: left;
    }
    .table-head button.active {
        color: var(--text);
    }
    .activity-scroll {
        flex: 1;
        min-height: 0;
        overflow: auto;
        scrollbar-width: thin;
        scrollbar-gutter: stable;
        overflow-anchor: none;
    }
    .activity-row {
        display: flex;
        align-items: stretch;

        min-height: 59px;
    }
    .activity-row.chosen,
    .child-row.chosen {
        background: var(--hover);
    }
    .expand,
    .indent {
        width: 25px;
        flex: 0 0 25px;
    }
    .expand {
        padding: 0;
        border: 0;
        border-radius: 0;
        background: none;
        box-shadow: none;
        color: var(--muted);
        font-size: 17px;
    }
    .inspect {
        flex: 1;
        min-width: 0;
        display: grid;
        grid-template-columns: minmax(0, 1fr) 62px 80px;
        align-items: center;
        gap: 0;
        text-align: left;
        padding: 10px 12px 10px 0;
        background: none;
        border: none;
        box-shadow: none;
        border-radius: 0;
        font-weight: 400;
    }
    .inspect:hover {
        background: none !important;
    }
    .activity-row:hover {
        background: var(--hover);
    }
    .activity-row.chosen:hover {
        background: var(--hover);
    }
    .identity {
        display: flex;
        align-items: center;
        gap: 10px;
        min-width: 0;
        padding-right: 12px;
    }
    .identity-copy {
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 4px;
    }
    strong {
        font-size: 12px;
        font-weight: 550;
        overflow: hidden;
        white-space: nowrap;
        text-overflow: ellipsis;
    }
    .purpose,
    small {
        font-size: 12px;
        line-height: 1.35;
        color: var(--muted);
        white-space: nowrap;
        text-overflow: ellipsis;
        overflow: hidden;
    }
    small {
        font-size: 12px;
    }
    .numeric {
        text-align: right;
        font-size: 12px;
        font-variant-numeric: tabular-nums;
    }
    .children {
        background: var(--hover);
    }
    .child-row {
        width: 100%;
        display: grid;
        grid-template-columns: minmax(0, 1fr) 62px 80px;
        padding: 10px 12px 10px 40px;
        border: 0;

        border-radius: 0;
        background: none;
        box-shadow: none;
        text-align: left;
        gap: 0;
        align-items: center;
    }
    .child-row > span:first-child {
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 4px;
        padding-right: 12px;
    }
    .children > .text-button {
        font-size: 12px;
        margin: 5px 16px;
    }
    .empty {
        padding: 28px 18px;
        color: var(--muted);
        font-size: 12px;
    }
    .error {
        font-size: 12px;
        padding: 12px;
    }
    footer {
        min-height: 40px;
        padding: 7px 12px;

        display: flex;
        gap: 8px;
        align-items: center;
        justify-content: space-between;
        flex-shrink: 0;
        color: var(--muted);
        font-size: 12px;
    }
    footer div {
        display: flex;
        align-items: center;
        gap: 8px;
    }
    footer button {
        min-height: 23px;
        padding: 1px 7px;
        font-size: 12px;
        border: 0;
        background: none;
        box-shadow: none;
    }
</style>
