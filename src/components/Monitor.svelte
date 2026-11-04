<script lang="ts">
    import { tick } from 'svelte';
    import type {
        Snapshot,
        Workload,
        ProcessRecord,
        ActionReport,
        AutomationCase,
    } from '../lib/types';
    import Workloads from './Workloads.svelte';
    import MonitorInspector from './MonitorInspector.svelte';
    import Logo from './Logo.svelte';
    import { invoke } from '@tauri-apps/api/core';
    import { queueInference } from '../lib/monitor';
    import { errorMessage } from '../lib/format';
    let {
        snapshot,
        selected,
        settingsKey,
        actionBusy,
        actionReport,
        cases = [],
        onselect,
        onclose,
        onaction,
        onusage,
    }: {
        snapshot: Snapshot;
        selected: Workload | null;
        settingsKey: string;
        actionBusy: boolean;
        actionReport: ActionReport | null;
        cases?: AutomationCase[];
        onselect: (workload: Workload) => void;
        onclose: () => void;
        onaction: (process: ProcessRecord, action: string) => void;
        onusage: () => void;
    } = $props();
    let process = $state<ProcessRecord | null>(null);
    let systemChat = $state(false);
    let seedQuestion = $state<string | null>(null);
    let inspector: HTMLDivElement | undefined = $state();
    let trigger: HTMLElement | null = null;
    let view = $state('apps');
    let search = $state('');
    let filter = $state('all');
    // Search in your own words: Enter asks Meta which running apps match the request.
    let found = $state<{ query: string; answer: string; ids: string[] } | null>(null);
    let finding = $state(false);
    let findError = $state('');
    async function findInWords() {
        const query = search.trim();
        if (!query || finding) return;
        finding = true;
        findError = '';
        try {
            const result = await queueInference(
                () =>
                    invoke<{ answer: string; workload_ids: string[] }>('find_workloads', {
                        query,
                    }),
                15
            );
            found = { query, answer: result.answer, ids: result.workload_ids };
            onusage();
        } catch (cause) {
            findError = errorMessage(cause);
        } finally {
            finding = false;
        }
    }
    $effect(() => {
        if (found && search.trim() !== found.query) found = null;
    });
    function inspect(workload: Workload, target?: ProcessRecord | null) {
        trigger = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        process = target ?? null;
        systemChat = false;
        seedQuestion = null;
        onselect(workload);
        void tick().then(() => inspector?.focus({ preventScroll: true }));
    }
    function close() {
        systemChat = false;
        seedQuestion = null;
        process = null;
        onclose();
        void tick().then(() => trigger?.focus({ preventScroll: true }));
    }
    function askAbout(question: string | null) {
        trigger = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        process = null;
        onclose();
        seedQuestion = question;
        systemChat = true;
    }
</script>

<div class="monitor-page">
    <header class="toolbar monitor-toolbar" data-tauri-drag-region>
        <h1 data-tauri-drag-region>Monitor</h1>
        <div class="segmented" role="group" aria-label="Activity view">
            <button aria-pressed={view === 'apps'} onclick={() => (view = 'apps')}>Apps</button
            ><button aria-pressed={view === 'processes'} onclick={() => (view = 'processes')}
                >Processes</button
            >
        </div>
        <div class="toolbar-spacer" data-tauri-drag-region></div>
        <select aria-label="Filter activity" bind:value={filter}
            ><option value="all">All activity</option><option value="apps">Applications</option
            ><option value="system">macOS services</option></select
        >
        <label class="search-field">
            <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true"
                ><path
                    d="M7 12.2A5.2 5.2 0 1 0 7 1.8a5.2 5.2 0 0 0 0 10.4Zm3.9-1.3L14.5 14.5"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    stroke-linecap="round"
                /></svg
            ><input
                aria-label="Filter workloads"
                placeholder="Search, or ask in words"
                title="Type a name to filter. Press Return to ask Jitterbug, like “AI apps” or “what can I quit”."
                maxlength="128"
                bind:value={search}
                onkeydown={(event) => {
                    if (event.key === 'Enter' && !event.isComposing) {
                        event.preventDefault();
                        void findInWords();
                    }
                    if (event.key === 'Escape') search = '';
                }}
            />{#if finding}<span class="finding"><Logo size={13} live /></span>{/if}
        </label>
        <button class="ask-button" onclick={() => askAbout(null)}
            ><Logo size={16} /> Ask Jitterbug</button
        >
    </header>
    <div class="monitor-body" class:inspecting={!!selected || systemChat}>
        <div class="monitor-main">
            <Workloads
                {snapshot}
                {settingsKey}
                {view}
                search={found ? '' : search}
                {filter}
                workloadIds={found?.ids ?? null}
                activityName={null}
                aiAnswer={found?.answer ??
                    (finding ? `Looking for “${search.trim()}”…` : findError || null)}
                onclearactivity={() => {
                    found = null;
                    findError = '';
                }}
                pauseEnrichment={false}
                selectedId={selected?.id ?? null}
                selectedProcess={process}
                onselect={inspect}
                {onusage}
            />
        </div>
        {#if selected || systemChat}<div
                class="monitor-inspector"
                role="dialog"
                aria-label="Process inspector"
                bind:this={inspector}
                tabindex="-1"
                onkeydown={(event) => {
                    if (event.key === 'Escape' && !(event.target instanceof HTMLTextAreaElement))
                        close();
                }}
            >
                {#key `${snapshot.source}:${selected?.id ?? 'system'}:${process?.pid ?? 'app'}:${process?.start_time ?? ''}:${seedQuestion ?? ''}`}<MonitorInspector
                        {snapshot}
                        workload={selected}
                        initialProcess={process}
                        initialQuestion={seedQuestion}
                        {settingsKey}
                        {actionBusy}
                        {actionReport}
                        {onusage}
                        {onaction}
                        onclose={close}
                    />{/key}
            </div>{/if}
    </div>
</div>

<style>
    .monitor-page {
        display: flex;
        flex-direction: column;
        min-height: 0;
        height: 100%;
    }
    .monitor-toolbar {
        gap: 14px;
    }
    .segmented {
        display: inline-flex;
        padding: 2px;
        border-radius: 7px;
        background: #ffffff0f;
        box-shadow: inset 0 0 0 0.5px #ffffff12;
        margin-left: 10px;
    }
    .segmented button {
        min-height: 22px;
        padding: 1px 12px;
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
        box-shadow:
            0 1px 2px #0000004d,
            inset 0 0.5px 0 #ffffff26;
    }
    select {
        font-size: 12px;
        min-height: 26px;
    }
    .search-field {
        position: relative;
        display: flex;
        align-items: center;
        width: 230px;
        color: var(--faint);
    }
    .search-field svg {
        position: absolute;
        left: 9px;
        pointer-events: none;
    }
    .search-field input {
        width: 100%;
        min-height: 26px;
        padding-left: 28px;
        font-size: 12px;
        border-radius: 7px;
    }
    .finding {
        position: absolute;
        right: 8px;
        display: grid;
    }
    .ask-button {
        display: inline-flex;
        align-items: center;
        gap: 7px;
        min-height: 26px;
        font-size: 12px;
        padding: 2px 11px 2px 9px;
    }
    .monitor-body {
        flex: 1;
        min-height: 0;
        display: grid;
        grid-template-columns: minmax(0, 1fr);
    }
    .monitor-body.inspecting {
        grid-template-columns: minmax(0, 1fr) 400px;
    }
    .monitor-main {
        display: flex;
        flex-direction: column;
        min-height: 0;
        min-width: 0;
    }
    .monitor-inspector {
        min-height: 0;
        overflow: hidden;
        background: var(--inspector);
        box-shadow: inset 1px 0 0 #00000066;
        outline: none;
    }
    @media (max-width: 1180px) {
        .monitor-body.inspecting {
            grid-template-columns: minmax(0, 1fr) 360px;
        }
        .search-field {
            width: 150px;
        }
    }
    @media (max-width: 1000px) {
        .monitor-toolbar select {
            display: none;
        }
    }
</style>
