<script lang="ts">
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import { untrack } from 'svelte';
    import { memory, percent, errorMessage } from '../lib/format';
    import type {
        AppProfile,
        ProcessRecord,
        Snapshot,
        Workload,
        WorkloadDetails,
    } from '../lib/types';
    import { demoDetails } from '../lib/demo';
    import {
        demoInventory,
        identityKey,
        queueInference,
        cancelled,
        type Inventory,
        type Query,
        type MonitorRow,
        type Explanation,
        type Identity,
    } from '../lib/monitor';
    import AppIcon from './AppIcon.svelte';
    let {
        snapshot,
        onselect,
        selectedId = null,
        paused = false,
        settingsKey = '',
        onusage,
    }: {
        snapshot: Snapshot;
        onselect: (workload: Workload, process?: ProcessRecord | null) => void;
        selectedId?: string | null;
        paused?: boolean;
        settingsKey?: string;
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
    let retry = $state(0);
    let profiles = $state<Record<string, AppProfile>>({});
    let attempted = $state<string[]>([]);
    const demo = $derived(snapshot.source !== 'live' || !isTauri());
    const query = $derived<Query>({ search, sort, view, filter, offset });
    const rows = $derived(inventory?.rows ?? []);
    const queryKey = $derived(
        JSON.stringify([snapshot.source, search, sort, view, filter, offset])
    );
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
            profiles = {};
            attempted = [];
            profileError = '';
        });
    });
    $effect(() => {
        const sample = snapshot;
        const request = query;
        let active = true;
        untrack(() => {
            loading = true;
        });
        const timer = setTimeout(
            async () => {
                try {
                    const result = demo
                        ? demoInventory(sample, request)
                        : await invoke<Inventory>('get_monitor_inventory', { query: request });
                    if (active) {
                        inventory = result;
                        error = '';
                    }
                } catch (cause) {
                    if (active) error = errorMessage(cause);
                } finally {
                    if (active) loading = false;
                }
            },
            search ? 160 : 0
        );
        return () => {
            active = false;
            clearTimeout(timer);
        };
    });
    $effect(() => {
        const id = expanded;
        const sample = snapshot;
        let active = true;
        if (!id) return;
        void (
            demo
                ? Promise.resolve(demoDetails(sample, id))
                : invoke<WorkloadDetails>('get_workload_details', {
                      workloadId: id,
                      pid: null,
                      startTime: null,
                  })
        )
            .then((detail) => {
                if (active) {
                    members = detail.members;
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
    const enrichmentKey = $derived(
        rows
            .slice(0, 12)
            .map((r) => (r.identity ? identityKey(r.identity) : ''))
            .join('|')
    );
    $effect(() => {
        const signature = `${queryKey}:${enrichmentKey}`;
        retry;
        const settings = settingsKey;
        if (demo || paused || !snapshot.sample_ready) return;
        const candidates = untrack(() =>
            rows
                .slice(0, 12)
                .map((r) => r.identity)
                .filter((p): p is Identity => p !== null && !attempted.includes(identityKey(p)))
        );
        if (!candidates.length) return;
        let active = true;
        const timer = setTimeout(async () => {
            explaining = true;
            const valid = () =>
                active &&
                signature === `${queryKey}:${enrichmentKey}` &&
                settings === settingsKey &&
                !paused;
            try {
                for (let index = 0; index < candidates.length && valid(); index += 6) {
                    const identities = candidates.slice(index, index + 6);
                    const ids = identities.map(identityKey);
                    attempted = [...attempted, ...ids].slice(-240);
                    try {
                        const cached = await invoke<Explanation[]>('explain_monitor_profiles', {
                            identities,
                            generate: false,
                        });
                        if (!valid()) {
                            attempted = attempted.filter((id) => !ids.includes(id));
                            break;
                        }
                        for (const item of cached)
                            profiles[identityKey(item.identity)] = item.profile;
                        const missing = identities.filter(
                            (p) => !cached.some((c) => identityKey(c.identity) === identityKey(p))
                        );
                        if (missing.length) {
                            const result = await queueInference(
                                () =>
                                    invoke<Explanation[]>('explain_monitor_profiles', {
                                        identities: missing,
                                        generate: true,
                                    }),
                                -1,
                                valid
                            );
                            if (valid()) {
                                for (const item of result)
                                    profiles[identityKey(item.identity)] = item.profile;
                                profileError = '';
                            }
                            onusage();
                        }
                    } catch (cause) {
                        if (cause === cancelled || !valid())
                            attempted = attempted.filter((id) => !ids.includes(id));
                        else {
                            profileError = errorMessage(cause);
                            break;
                        }
                    }
                }
                if (Object.keys(profiles).length > 240)
                    profiles = Object.fromEntries(Object.entries(profiles).slice(-240));
            } finally {
                explaining = false;
            }
        }, 1800);
        return () => {
            active = false;
            clearTimeout(timer);
        };
    });
    function profile(row: MonitorRow) {
        return row.identity ? profiles[identityKey(row.identity)] : undefined;
    }
    function spark(row: MonitorRow) {
        return row.workload.history
            .map(
                (p, i, points) =>
                    `${(i / Math.max(points.length - 1, 1)) * 64},${20 - Math.min(p.cpu_percent, 100) * 0.2}`
            )
            .join(' ');
    }
</script>

<section class="panel workload-panel" data-tour="workloads" aria-label="Activity monitor">
    <div class="section-title">
        <div>
            <h2>What’s running</h2>
            <span class="muted"
                >{inventory?.total ?? '…'}
                {view === 'apps' ? 'apps & groups' : 'processes'}{search
                    ? ' matching'
                    : ' observed'}</span
            >
        </div>
        <div class="view-switch" aria-label="Activity view">
            <button
                aria-pressed={view === 'apps'}
                onclick={() => {
                    view = 'apps';
                }}>Apps &amp; groups</button
            >
            <button
                aria-pressed={view === 'processes'}
                onclick={() => {
                    view = 'processes';
                }}>Processes</button
            >
        </div>
    </div>
    <div class="monitor-filters">
        <input
            aria-label="Filter workloads"
            placeholder="Search apps, processes, or PID"
            maxlength="128"
            bind:value={search}
        />
        <select aria-label="Filter activity" bind:value={filter}
            ><option value="all">All activity</option><option value="apps">Applications</option
            ><option value="system">System</option></select
        >
        <select aria-label="Sort workloads" bind:value={sort}
            ><option value="impact">Highest impact</option><option value="cpu">CPU</option><option
                value="memory">Memory</option
            ><option value="name">Name</option></select
        >
    </div>
    {#if error}<p class="error table-note" role="alert">{error}</p>{/if}
    <div class="monitor-header">
        <span>{view === 'apps' ? 'Application / group' : 'Process / PID'}</span><span>CPU</span
        ><span>Memory</span><span class="trend">Trend</span>
    </div>
    <div aria-busy={loading}>
        {#each rows as row (`${row.workload.id}:${row.process?.pid ?? ''}:${row.process?.start_time ?? ''}`)}
            {@const readings = row.process ?? row.workload}
            {@const explanation = profile(row)}
            <div class="monitor-row" class:chosen={selectedId === row.workload.id}>
                <div class="identity-cell">
                    {#if !row.process}<button
                            class="expand"
                            aria-label={`Show processes for ${row.workload.label}`}
                            aria-expanded={expanded === row.workload.id}
                            onclick={() => {
                                members = [];
                                expanded = expanded === row.workload.id ? null : row.workload.id;
                            }}>{expanded === row.workload.id ? '▾' : '▸'}</button
                        >{/if}
                    <button
                        disabled={loading}
                        class="inspect"
                        onclick={() => onselect(row.workload, row.process)}
                    >
                        <AppIcon
                            workloadId={row.workload.id}
                            category={row.workload.category}
                            {demo}
                        />
                        <span class="identity-copy"
                            ><strong>{row.process?.name ?? row.workload.label}</strong><small
                                >{row.process
                                    ? `PID ${row.process.pid} · ${row.workload.label}`
                                    : `${row.workload.process_count} processes`}</small
                            >
                            <span class="purpose"
                                >{explanation?.text ??
                                    row.process?.description ??
                                    row.workload.description}</span
                            >
                            <small class="evidence-label"
                                >{explanation?.source === 'model'
                                    ? 'AI description · cached on this device'
                                    : explanation?.source === 'reference'
                                      ? 'Built-in reference'
                                      : demo
                                        ? 'Synthetic example'
                                        : 'Local attribution'}</small
                            >
                        </span>
                    </button>
                </div>
                <span class="numeric usage-cell"
                    ><span>{snapshot.sample_ready ? percent(readings.cpu_percent) : '—'}</span><span
                        class="usage-bar"
                        ><i style:width={`${Math.min(readings.cpu_percent, 100)}%`}></i></span
                    ></span
                >
                <span class="numeric">{memory(readings.memory_bytes)}</span>
                <span class="trend"
                    >{#if !row.process}<svg
                            viewBox="0 0 64 22"
                            role="img"
                            aria-label={`${row.workload.label} group CPU history`}
                            ><polyline
                                points={spark(row)}
                                fill="none"
                                stroke="var(--accent)"
                                stroke-width="1.5"
                            /></svg
                        >{:else}<span title="Individual history is collected when inspected">—</span
                        >{/if}</span
                >
            </div>
            {#if expanded === row.workload.id && !row.process}
                <div
                    class="expanded-processes"
                    role="group"
                    aria-label={`${row.workload.label} processes`}
                >
                    {#if memberError}<p class="error">{memberError}</p>{/if}
                    {#each members.slice(0, 20) as process (`${process.pid}:${process.start_time}`)}<button
                            class="process-row"
                            onclick={() => onselect(row.workload, process)}
                            ><span>{process.name}<small>PID {process.pid}</small></span><span
                                >{percent(process.cpu_percent)}</span
                            ><span>{memory(process.memory_bytes)}</span></button
                        >{:else}<p class="muted">Reading processes…</p>{/each}
                    {#if members.length > 20}<button
                            class="text-button"
                            onclick={() => {
                                view = 'processes';
                                search = row.workload.label;
                            }}>Find all processes in this group</button
                        >{/if}
                </div>
            {/if}
        {:else}<p class="empty">
                {loading
                    ? 'Reading process inventory…'
                    : 'No matching activity in the observed inventory.'}
            </p>{/each}
    </div>
    <footer>
        <span
            >{explaining
                ? 'Explaining unfamiliar processes…'
                : 'CPU: share of all cores · Memory: resident usage'}</span
        >
        <div class="pagination">
            <button
                disabled={!inventory || inventory.offset === 0}
                onclick={() => {
                    offset = Math.max(0, (inventory?.offset ?? 0) - 40);
                }}>Previous</button
            ><span
                >{inventory?.total
                    ? `${inventory.offset + 1}–${inventory.offset + rows.length} of ${inventory.total}`
                    : '0 results'}</span
            ><button
                disabled={!inventory || inventory.offset + 40 >= inventory.total}
                onclick={() => {
                    offset = (inventory?.offset ?? 0) + 40;
                }}>Next</button
            >
        </div>
    </footer>
    {#if profileError}<details class="table-note">
            <summary>Some AI descriptions are unavailable</summary>
            <p>{profileError}</p>
            <button
                onclick={() => {
                    attempted = [];
                    profileError = '';
                    retry++;
                }}>Retry descriptions</button
            >
        </details>{/if}
    <p class="footnote table-note">
        Shared memory may be counted more than once. Inventory is bounded to 4,096 processes and 512
        groups; trends cover each group. AI descriptions explain general purpose; select a process
        to investigate its current activity.
    </p>
</section>

<style>
    .section-title {
        gap: 12px;
        flex-wrap: wrap;
        padding: 18px 16px 0;
    }
    .view-switch {
        display: flex;
        padding: 3px;
        background: var(--control);
        border-radius: 8px;
    }
    .view-switch button {
        border: 0;
        background: transparent;
        font-size: 11px;
    }
    .view-switch button[aria-pressed='true'] {
        background: var(--panel);
        color: var(--accent);
        box-shadow: 0 1px 4px #0002;
    }
    .monitor-filters {
        display: flex;
        gap: 8px;
        padding: 4px 16px 16px;
        flex-wrap: wrap;
    }
    .monitor-filters input {
        flex: 1;
        min-width: 160px;
    }
    .monitor-header,
    .monitor-row {
        display: grid;
        grid-template-columns: minmax(190px, 1fr) 70px 80px 64px;
        gap: 16px;
        align-items: center;
        padding: 12px 16px;
        border-top: 1px solid var(--border);
    }
    .monitor-header {
        font-size: 10px;
        color: var(--muted);
    }
    .monitor-header span:not(:first-child) {
        text-align: right;
    }
    .monitor-row.chosen {
        background: var(--accent-soft);
    }
    .identity-cell {
        display: flex;
        align-items: flex-start;
        gap: 6px;
        min-width: 0;
    }
    .expand {
        border: 0;
        background: transparent;
        padding: 4px;
        color: var(--muted);
    }
    .inspect {
        display: flex;
        align-items: flex-start;
        gap: 10px;
        border: 0;
        background: transparent;
        padding: 0;
        text-align: left;
        min-width: 0;
        flex: 1;
    }
    .identity-copy {
        display: grid;
        gap: 4px;
        min-width: 0;
    }
    .identity-copy strong {
        overflow-wrap: anywhere;
        font-size: 13px;
    }
    .identity-copy small {
        font-size: 10px;
    }
    .purpose {
        color: var(--muted);
        font-size: 12px;
        line-height: 1.5;
    }
    .evidence-label {
        opacity: 0.8;
    }
    .numeric {
        font-variant-numeric: tabular-nums;
        font-size: 12px;
        text-align: right;
    }
    .usage-cell {
        display: grid;
        gap: 7px;
    }
    .usage-bar {
        height: 3px;
        border-radius: 2px;
        background: var(--track);
        overflow: hidden;
    }
    .usage-bar i {
        display: block;
        height: 100%;
        background: var(--accent);
    }
    .trend {
        text-align: right;
    }
    .trend svg {
        width: 64px;
        height: 22px;
    }
    .expanded-processes {
        padding: 4px 16px 10px 48px;
        background: var(--control);
    }
    .process-row {
        display: grid;
        grid-template-columns: minmax(0, 1fr) 70px 80px;
        width: 100%;
        text-align: left;
        gap: 10px;
        border: 0;
        background: transparent;
        border-bottom: 1px solid var(--border);
        padding: 10px 0;
        font-size: 12px;
    }
    .process-row small {
        display: block;
    }
    .process-row > span:not(:first-child) {
        text-align: right;
    }
    footer {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 12px;
        flex-wrap: wrap;
        padding: 14px 16px;
        border-top: 1px solid var(--border);
        color: var(--muted);
        font-size: 10px;
    }
    .pagination {
        display: flex;
        align-items: center;
        gap: 10px;
    }
    .pagination button {
        font-size: 11px;
    }
    .table-note {
        margin: 8px 16px 14px;
    }
    @media (max-width: 1100px) {
        .trend {
            display: none;
        }
        .monitor-header,
        .monitor-row {
            grid-template-columns: minmax(160px, 1fr) 60px 70px;
            gap: 10px;
        }
    }
</style>
