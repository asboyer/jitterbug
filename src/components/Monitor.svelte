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
    import AiActivityLog from './AiActivityLog.svelte';
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
        trigger?.focus({ preventScroll: true });
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
    {#if !selected && !systemChat}<div class="monitor-activity">
            <AiActivityLog live={snapshot.source === 'live'} {cases} />
        </div>{/if}
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
        animation: none !important;
        display: flex;
        flex-direction: column;
        gap: 18px;
        height: calc(100dvh - 136px);
        min-height: 390px;
        max-width: 1600px;
        margin: 0 auto;
    }
    .monitor-activity {
        flex: 0 0 auto;
        max-height: min(260px, 30dvh);
        overflow: auto;
    }
    .monitor-activity :global(.ai-activity) {
        margin-bottom: 0;
    }
    .monitor-title {
        display: flex;
        align-items: center;
        justify-content: space-between;
        flex: 0 0 auto;
    }
    .monitor-title h1 {
        font-size: 25px;
        margin: 0;
        letter-spacing: -0.7px;
    }
    .monitor-title button {
        display: flex;
        align-items: center;
        gap: 7px;
        font-size: 12px;
    }
    .monitor-workspace {
        min-height: 0;
        flex: 1;
        display: grid;
        grid-template-columns: minmax(0, 1fr);
        gap: 16px;
    }
    .monitor-workspace.inspecting {
        grid-template-columns: minmax(350px, 1fr) minmax(360px, 0.9fr);
    }
    .monitor-inspector {
        min-height: 0;
        min-width: 0;
        border: 1px solid var(--border);
        border-radius: 12px;
        overflow: hidden;
        background: linear-gradient(var(--panel), var(--panel)), var(--canvas);
    }
    @media (max-width: 1100px) {
        .monitor-workspace.inspecting {
            grid-template-columns: minmax(0, 1fr);
        }
        .monitor-inspector {
            position: fixed;
            top: 66px;
            right: 16px;
            bottom: 16px;
            width: min(520px, calc(100vw - 200px));
            z-index: 30;
            box-shadow: var(--shadow-float);
        }
    }
    @media (max-height: 700px) {
        .monitor-page {
            height: calc(100dvh - 112px);
            gap: 12px;
        }
    }
</style>
