<script lang="ts">
    import { tick } from 'svelte';
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import { errorMessage, memory, percent } from '../lib/format';
    import { demoDetails } from '../lib/demo';
    import type {
        AppProfile,
        ProcessRecord,
        Snapshot,
        Workload,
        WorkloadDetails,
    } from '../lib/types';
    import AppIcon from './AppIcon.svelte';

    let {
        snapshot,
        workload,
        actionBusy,
        onback,
        onask,
        onaction,
        onusage,
    }: {
        snapshot: Snapshot;
        workload: Workload;
        actionBusy: boolean;
        onback: () => void;
        onask: (process: ProcessRecord | null) => void;
        onaction: (process: ProcessRecord, action: string) => void;
        onusage: () => void;
    } = $props();
    let detail: WorkloadDetails | null = $state(null);
    let selected: { pid: number; start_time: number } | null = $state(null);
    let error = $state('');
    let profileError = $state('');
    let generating = $state(false);
    let search = $state('');
    let sequence = 0;
    let revealOrigin = false;
    let originPanel: HTMLElement | undefined = $state();
    const demo = $derived(snapshot.source === 'demo' || !isTauri());
    const current = $derived.by(() => detail?.workload ?? workload);
    const members = $derived.by(
        () =>
            detail?.members.filter((p) =>
                `${p.name} ${p.pid}`.toLowerCase().includes(search.toLowerCase())
            ) ?? []
    );
    const process = $derived.by(() => detail?.process ?? null);
    const leaf = $derived.by(() => detail?.ancestry.at(-1));
    const date = (time: number) => (time ? new Date(time * 1000).toLocaleString() : 'Unavailable');

    async function load(id: string, identity: typeof selected, sample: Snapshot) {
        const request = ++sequence;
        try {
            const next =
                sample.source === 'demo' || !isTauri()
                    ? demoDetails(sample, id)
                    : await invoke<WorkloadDetails>('get_workload_details', {
                          workloadId: id,
                          pid: identity?.pid ?? null,
                          startTime: identity?.start_time ?? null,
                      });
            if (request !== sequence) return;
            detail = next;
            error = '';
            if (revealOrigin) {
                revealOrigin = false;
                await tick();
                originPanel?.scrollIntoView({ block: 'start', behavior: 'smooth' });
            }
        } catch (cause) {
            if (request === sequence) error = errorMessage(cause);
        }
    }

    $effect(() => {
        // Telemetry events stop when the window is hidden. No extra polling or model calls.
        snapshot.captured_at;
        void load(workload.id, selected, snapshot);
        return () => {
            sequence++;
        };
    });

    async function explain() {
        if (!detail || demo || generating) return;
        generating = true;
        profileError = '';
        const identity = `${detail.process.pid}:${detail.process.start_time}`;
        try {
            const profile = await invoke<AppProfile>('explain_application', {
                workloadId: workload.id,
                pid: detail.process.pid,
                startTime: detail.process.start_time,
            });
            if (detail && identity === `${detail.process.pid}:${detail.process.start_time}`)
                detail = { ...detail, profile };
        } catch (cause) {
            profileError = errorMessage(cause);
        } finally {
            generating = false;
            onusage();
        }
    }
</script>

<button class="text-button back" onclick={onback}>‹ All workloads</button>
<div class="detail-heading">
    <AppIcon workloadId={workload.id} category={workload.category} size={56} {demo} />
    <div class="heading-copy">
        <h1>{workload.label}</h1>
        <p class="muted">{workload.attribution}</p>
    </div>
    <button class="primary" onclick={() => onask(null)}>Ask about this workload ↗</button>
</div>
<div class="detail-stats" aria-label="Workload resource use">
    <div><span>CPU · total capacity</span><strong>{percent(current.cpu_percent)}</strong></div>
    <div><span>Resident memory</span><strong>{memory(current.memory_bytes)}</strong></div>
    <div><span>Processes</span><strong>{current.process_count}</strong></div>
</div>
{#if error}<div class="banner error" role="alert">
        {error}
        {detail ? 'The details below are from an earlier sample.' : ''}
        <button
            onclick={() => {
                selected = null;
                void load(workload.id, null, snapshot);
            }}>Refresh selection</button
        >
    </div>{/if}
{#if detail}
    <section class="panel app-explanation">
        <div class="section-title">
            <h2>What is it?</h2>
            <span class="tag">
                {detail.profile.source === 'reference'
                    ? 'Built-in reference · no model call'
                    : detail.profile.source === 'model'
                      ? `${detail.profile.cached ? 'Cached AI explanation' : 'AI explanation'} · ${detail.profile.provider}`
                      : demo
                        ? 'Synthetic explanation'
                        : 'Inferred from executable / grouping'}
            </span>
        </div>
        <p class="explanation-text">{detail.profile.text}</p>
        {#if detail.application}<p class="muted identity">
                {detail.application.name}{detail.application.version
                    ? ` · Version ${detail.application.version}`
                    : ''}
                {#if detail.application.identifier}<br />{detail.application.identifier}{/if}
                <br />{detail.application.identity_source}. This does not verify the publisher.
            </p>{/if}
        {#if detail.profile.generated_at}<p class="muted">
                {detail.profile.model} · Generated {date(detail.profile.generated_at)}. Saved on
                this device for up to 30 days; refreshed for a new app version.
            </p>{/if}
        {#if detail.can_generate_profile && detail.profile.source !== 'model'}<div
                class="profile-generation"
            >
                <button disabled={generating || !!error} onclick={explain}
                    >{generating ? 'Writing app explanation…' : 'Generate app explanation'}</button
                >
                <small class="muted"
                    >Uses your intelligent provider with only the app name and bundle identifier.
                    Cached locally; no live metrics or paths are sent.</small
                >
            </div>{/if}
        {#if profileError}<p class="error" role="alert">{profileError}</p>{/if}
        <p class="footnote">
            This describes the app’s general purpose. Ask Jitterbug for a fresh diagnosis of its
            current resource use.
        </p>
    </section>
    <section class="panel process-section">
        <div class="section-title">
            <div>
                <h2>Processes in this workload</h2>
                <span class="muted"
                    >Select a process to trace its origins. {detail.independent_roots > 1
                        ? `${detail.independent_roots} independent roots; this group is not a single launch.`
                        : ''}</span
                >
            </div>
            <input aria-label="Filter processes" placeholder="Name or PID" bind:value={search} />
        </div>
        <div class="members" role="group" aria-label="Workload processes">
            {#each members as member (`${member.pid}:${member.start_time}`)}
                <button
                    aria-pressed={member.pid === process?.pid &&
                        member.start_time === process?.start_time}
                    class="member"
                    class:chosen={member.pid === process?.pid &&
                        member.start_time === process?.start_time}
                    onclick={() => {
                        revealOrigin = true;
                        selected = { pid: member.pid, start_time: member.start_time };
                    }}
                    aria-label={`Inspect ${member.name}, PID ${member.pid}`}
                >
                    <span class="member-name"
                        ><AppIcon
                            workloadId={workload.id}
                            category={workload.category}
                            size={24}
                            {demo}
                        /><span><strong>{member.name}</strong><small>PID {member.pid}</small></span
                        ></span
                    >
                    <span class="numeric">{percent(member.cpu_percent)}<small>CPU</small></span
                    ><span class="numeric">{memory(member.memory_bytes)}<small>Memory</small></span
                    ><span aria-hidden="true">›</span>
                </button>
            {:else}<p class="muted empty">No matching processes.</p>{/each}
        </div>
        <p class="footnote">
            Showing {detail.members.length} of {detail.member_count} observed members (up to 200). {detail.member_count <
            current.process_count
                ? 'Some members are outside the bounded collector inventory.'
                : ''} Memory is summed RSS; shared pages may be counted more than once.
        </p>
    </section>
    {#if process}<section
            class="panel origin-panel"
            aria-label="Process origins"
            bind:this={originPanel}
        >
            <div class="section-title">
                <div>
                    <h2>{process.name}</h2>
                    <span class="muted"
                        >PID {process.pid} · {demo
                            ? 'Synthetic example'
                            : `Observed ${new Date(detail.captured_at * 1000).toLocaleTimeString()}`}</span
                    >
                </div>
                <button disabled={!!error} onclick={() => onask(process)}
                    >Investigate this process ↗</button
                >
            </div>
            <h3>Where did it come from?</h3>
            <p>{detail.origin_summary}</p>
            <ol class="origin-chain" aria-label="Observed parent chain">
                {#each detail.ancestry as ancestor, index}
                    <li class:current={index === detail.ancestry.length - 1}>
                        <div class="origin-dot" aria-hidden="true"></div>
                        <div>
                            <strong>{ancestor.name}</strong><span class="muted">
                                PID {ancestor.pid}{index === detail.ancestry.length - 1
                                    ? ' · Selected process'
                                    : ''}</span
                            >
                            <small>Started {date(ancestor.start_time)}</small>
                            {#if ancestor.executable}<details>
                                    <summary>Executable location</summary><code
                                        >{ancestor.executable}</code
                                    >
                                </details>{/if}
                        </div>
                    </li>
                {/each}
            </ol>
            <p class="footnote">{detail.origin_limit}</p>
            <dl class="process-facts">
                <div>
                    <dt>Executable</dt>
                    <dd>{leaf?.executable ?? 'Not exposed by the OS'}</dd>
                </div>
                <div>
                    <dt>Working directory</dt>
                    <dd>{leaf?.working_directory ?? 'Not exposed by the OS'}</dd>
                </div>
                <div>
                    <dt>Started</dt>
                    <dd>{date(process.start_time)}</dd>
                </div>
                <div>
                    <dt>Disk · last {snapshot.interval_ms / 1000}s</dt>
                    <dd>
                        {memory(process.disk_read_bytes)} read · {memory(
                            process.disk_written_bytes
                        )} written
                    </dd>
                </div>
                <div>
                    <dt>Grouping evidence</dt>
                    <dd>{process.evidence.join(' ')}</dd>
                </div>
            </dl>
            <div class="process-actions">
                {#if process.actionable && !demo}{#each process.actions as action}<button
                            disabled={actionBusy || !!error}
                            class:danger-text={action === 'terminate'}
                            onclick={() => onaction(process, action)}
                            >{action === 'terminate' ? 'Stop process…' : 'Lower priority…'}</button
                        >{/each}
                {:else}<span class="muted"
                        >{demo
                            ? 'Actions disabled in demo'
                            : 'Process actions protected or unavailable'}</span
                    >{/if}
            </div>
            <p class="footnote">
                Paths stay in this local detail view. Process actions are checked again against the
                current identity and require confirmation.
            </p>
        </section>{/if}
{:else if !error}<p class="muted">Reading process details…</p>{/if}

<style>
    .back {
        margin: 0 0 14px -8px;
    }
    .detail-heading {
        display: flex;
        align-items: center;
        gap: 14px;
        margin-bottom: 22px;
        flex-wrap: wrap;
    }
    .heading-copy {
        flex: 1;
        min-width: 120px;
    }
    h1 {
        margin: 0;
        overflow-wrap: anywhere;
    }
    .heading-copy p {
        margin: 4px 0 0;
    }
    .detail-stats {
        display: grid;
        grid-template-columns: repeat(3, 1fr);
        margin-bottom: 22px;
    }
    .detail-stats > div {
        padding: 0 20px;
        border-left: 1px solid var(--border);
    }
    .detail-stats > div:first-child {
        padding-left: 0;
        border: 0;
    }
    .detail-stats span {
        display: block;
        color: var(--muted);
        font-size: 11px;
        margin-bottom: 5px;
    }
    .detail-stats strong {
        font-size: 24px;
        font-weight: 600;
        font-variant-numeric: tabular-nums;
    }
    .app-explanation,
    .process-section,
    .origin-panel {
        margin-bottom: 18px;
        padding: 18px 20px;
    }
    .section-title {
        flex-wrap: wrap;
        gap: 10px;
    }
    .section-title .tag {
        font-size: 10px;
    }
    .explanation-text {
        font-size: 14px;
        line-height: 1.65;
        white-space: pre-wrap;
    }
    .identity {
        font-size: 11px;
        line-height: 1.65;
        overflow-wrap: anywhere;
    }
    .profile-generation {
        display: flex;
        align-items: center;
        gap: 14px;
    }
    .profile-generation button {
        flex: 0 0 auto;
    }
    .profile-generation small {
        font-size: 11px;
        max-width: 400px;
    }
    .members {
        max-height: 280px;
        overflow: auto;
        margin-top: 12px;
    }
    .member {
        display: grid;
        grid-template-columns: minmax(130px, 1fr) 70px 90px 8px;
        align-items: center;
        gap: 14px;
        width: 100%;
        padding: 10px 8px;
        border: 0;
        box-shadow: none;
        background: transparent;
        text-align: left;
        border-radius: 6px;
    }
    .member:hover {
        background: var(--hover);
    }
    .member.chosen {
        background: var(--accent-soft);
    }
    .member-name {
        display: flex;
        gap: 9px;
        align-items: center;
        min-width: 0;
    }
    .member-name > span {
        min-width: 0;
    }
    .member strong {
        display: block;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-weight: 500;
    }
    .member small {
        display: block;
        color: var(--muted);
        font-size: 10px;
        margin-top: 3px;
    }
    h3 {
        margin-top: 24px;
        font-size: 13px;
    }
    .origin-chain {
        padding: 0;
        margin: 20px 0;
        list-style: none;
    }
    .origin-chain li {
        display: flex;
        position: relative;
        gap: 14px;
        padding-bottom: 20px;
    }
    .origin-chain li:last-child {
        padding-bottom: 0;
    }
    .origin-chain li:not(:last-child)::before {
        content: '';
        position: absolute;
        left: 5px;
        top: 10px;
        height: 100%;
        border-left: 1px solid var(--border);
    }
    .origin-dot {
        z-index: 1;
        border: 2px solid var(--muted);
        border-radius: 50%;
        width: 11px;
        height: 11px;
        flex: 0 0 11px;
        margin-top: 4px;
        background: var(--panel);
    }
    .current .origin-dot {
        border-color: var(--accent);
        background: var(--accent);
    }
    .origin-chain li > div:last-child {
        min-width: 0;
    }
    .origin-chain small {
        display: block;
        color: var(--muted);
        font-size: 11px;
        margin-top: 4px;
    }
    .origin-chain details {
        font-size: 11px;
        color: var(--muted);
        margin-top: 5px;
    }
    .origin-chain code {
        display: block;
        margin-top: 5px;
        overflow-wrap: anywhere;
    }
    .process-facts {
        border-top: 1px solid var(--border);
        padding-top: 14px;
        margin-top: 20px;
        font-size: 12px;
    }
    .process-facts > div {
        display: grid;
        grid-template-columns: 140px minmax(0, 1fr);
        gap: 16px;
        padding: 6px 0;
    }
    dt {
        color: var(--muted);
    }
    dd {
        margin: 0;
        overflow-wrap: anywhere;
    }
    .process-actions {
        margin-top: 18px;
    }
    .footnote {
        margin-bottom: 0;
    }
</style>
