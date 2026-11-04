<script lang="ts">
    import { tick } from 'svelte';
    import type {
        Snapshot,
        Workload,
        ProcessRecord,
        ActionReport,
        AutomationCase,
    } from '../lib/types';
    import MonitorStats from './MonitorStats.svelte';
    import Workloads from './Workloads.svelte';
    import MonitorInspector from './MonitorInspector.svelte';
    import Icon from './Icon.svelte';
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
    let inspector: HTMLDivElement | undefined = $state();
    let trigger: HTMLElement | null = null;
    function inspect(workload: Workload, target?: ProcessRecord | null) {
        trigger = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        process = target ?? null;
        systemChat = false;
        onselect(workload);
        void tick().then(() => inspector?.focus({ preventScroll: true }));
    }
    function close() {
        systemChat = false;
        process = null;
        onclose();
        void tick().then(() => trigger?.focus({ preventScroll: true }));
    }
</script>

<div class="monitor-page">
    <header class="monitor-title">
        <h1>Monitor</h1>
        <button
            class="text-button"
            onclick={() => {
                systemChat = true;
                onclose();
            }}><Icon name="companion" size={16} /> Ask Jitterbug</button
        >
    </header>
    <MonitorStats {snapshot} />
    <div class="monitor-workspace" class:inspecting={!!selected || systemChat}>
        <Workloads
            {snapshot}
            {settingsKey}
            pauseEnrichment={!!selected || systemChat}
            selectedId={selected?.id ?? null}
            selectedProcess={process}
            onselect={inspect}
            {onusage}
        />
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
                {#key `${snapshot.source}:${selected?.id ?? 'system'}:${process?.pid ?? 'app'}:${process?.start_time ?? ''}`}<MonitorInspector
                        {snapshot}
                        workload={selected}
                        initialProcess={process}
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
        gap: 20px;
    }
    .monitor-title {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 16px;
        flex-shrink: 0;
    }
    .monitor-title button {
        display: flex;
        align-items: center;
        gap: 8px;
        font-size: 12px;
    }
    .monitor-workspace {
        flex: 1;
        display: grid;
        min-height: 0;
        grid-template-columns: minmax(0, 1fr);
        gap: 20px;
    }
    .monitor-workspace.inspecting {
        grid-template-columns: minmax(340px, 1fr) 400px;
    }
    .monitor-inspector {
        min-height: 0;
        overflow: hidden;
        background: var(--panel);
        border-radius: 6px;
        border: 0;
        outline: none;
    }
    @media (max-width: 1000px) {
        .monitor-workspace.inspecting {
            grid-template-columns: minmax(280px, 1fr) minmax(340px, 0.95fr);
            gap: 16px;
        }
    }
    @media (max-width: 760px) {
        .monitor-workspace.inspecting {
            grid-template-columns: minmax(0, 1fr);
        }
        .monitor-workspace.inspecting :global(.activity-list) {
            display: none;
        }
        .monitor-inspector {
            width: 100%;
        }
        .monitor-page {
            gap: 18px;
        }
    }
</style>
