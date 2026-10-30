<script lang="ts">
    import { onMount } from 'svelte';
    import { invoke, isTauri } from '@tauri-apps/api/core';
    import { listen, type UnlistenFn } from '@tauri-apps/api/event';
    import { demoSnapshot } from '../lib/demo';
    import { errorMessage, memory, percent } from '../lib/format';
    import type { Snapshot } from '../lib/types';
    import Icon from './Icon.svelte';
    import AppIcon from './AppIcon.svelte';
    import ActivityGraph from './ActivityGraph.svelte';

    const desktop = isTauri();
    let snapshot: Snapshot | null = $state(desktop ? null : demoSnapshot());
    let error = $state('');
    let capture = $state('');
    const shortcut = navigator.userAgent.includes('Mac') ? '⌘⇧J' : 'Ctrl+Shift+J';
    onMount(() => {
        if (!desktop) return;
        let stop: UnlistenFn | undefined;
        let stopCapture: UnlistenFn | undefined;
        let disposed = false;
        async function initialize() {
            try {
                stop = await listen<Snapshot>('telemetry', (event) => {
                    snapshot = event.payload;
                });
                stopCapture = await listen<{ phase: string; message: string }>(
                    'jitter-capture',
                    (event) => {
                        capture = event.payload.phase === 'error' ? '' : event.payload.message;
                        if (event.payload.phase === 'error') error = event.payload.message;
                    }
                );
                if (disposed) {
                    stop();
                    stopCapture();
                    return;
                }
                snapshot = await invoke('get_quick_snapshot');
            } catch (cause) {
                error = errorMessage(cause);
            }
        }
        void initialize();
        return () => {
            disposed = true;
            stop?.();
            stopCapture?.();
        };
    });
    async function snapshotNow() {
        if (!desktop) {
            capture = 'Preview only. No snapshot was recorded.';
            return;
        }
        error = '';
        try {
            capture = await invoke<string>('report_jitter');
        } catch (cause) {
            error = errorMessage(cause);
        }
    }
    async function open(page = 'dashboard', workloadId: string | null = null) {
        if (!desktop) return;
        try {
            await invoke('open_main', { page, workloadId });
        } catch (cause) {
            error = errorMessage(cause);
        }
    }
    async function escape(event: KeyboardEvent) {
        if (event.key === 'Escape' && desktop) await invoke('hide_quick_view');
    }
</script>

<svelte:window onkeydown={escape} />
<div class="quick-view">
    <header class="quick-header">
        <Icon name="companion" size={19} /><strong>Jitterbug</strong>
        <span class="muted">{desktop ? 'Live' : 'Demo'}</span>
        <button class="icon-button" aria-label="Open settings" onclick={() => open('settings')}
            ><Icon name="settings" /></button
        >
    </header>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if snapshot && snapshot.memory_total_bytes > 0}
        <div class="quick-metrics">
            <div>
                <small>CPU</small><strong
                    >{snapshot.sample_ready ? percent(snapshot.cpu_percent) : '…'}</strong
                ><span class="muted">Total capacity</span>
            </div>
            <div>
                <small>Available memory</small><strong
                    >{memory(snapshot.memory_available_bytes)}</strong
                ><span class="muted">of {memory(snapshot.memory_total_bytes)}</span>
            </div>
        </div>
        <div class="quick-pressure">
            <span class="status-dot" class:warning={snapshot.pressure.level !== 'normal'}
            ></span>{!snapshot.sample_ready
                ? 'Collecting samples…'
                : snapshot.pressure.level === 'normal'
                  ? 'System looks healthy'
                  : 'Resource pressure detected'}<span class="muted"
                >{snapshot.platform.thermal ?? ''}</span
            >
        </div>
        <div class="quick-history"><ActivityGraph {snapshot} /></div>
        <div class="quick-snapshot">
            <button onclick={snapshotNow} title="Record what happened around this moment"
                >Jitter Snapshot <kbd>{shortcut}</kbd></button
            >
            <small aria-live="polite"
                >{capture || 'Felt a hitch? Capture the last minute and see why.'}</small
            >
        </div>
        <div class="quick-detail">
            <span>Compressed memory</span><span
                >{snapshot.platform.memory
                    ? memory(snapshot.platform.memory.compressed_bytes)
                    : 'Unavailable'}</span
            >
        </div>
        <div class="quick-detail">
            <span>Swap out · last {snapshot.interval_ms / 1000}s</span><span
                >{snapshot.swap_out_bytes === null
                    ? 'Unavailable'
                    : memory(snapshot.swap_out_bytes)}</span
            >
        </div>
        <div class="quick-list-heading">
            <strong>Top workloads</strong><span>CPU / Memory</span>
        </div>
        <div class="quick-workloads">
            {#each snapshot.workloads.slice(0, 5) as workload (workload.id)}
                <button
                    onclick={() => open('dashboard', workload.id)}
                    title={`Inspect ${workload.label}`}
                >
                    <AppIcon
                        workloadId={workload.id}
                        category={workload.category}
                        size={24}
                        demo={snapshot.source === 'demo'}
                    />
                    <span>{workload.label}</span><small
                        >{percent(workload.cpu_percent)}<br />{memory(workload.memory_bytes)}</small
                    >
                </button>
            {/each}
        </div>
    {:else}<p class="empty">Collecting a fresh sample…</p>{/if}
    <footer>
        <button onclick={() => open('hub')}>Open Jitterbug <span aria-hidden="true">↗</span></button
        ><small>{desktop ? 'Click a workload to investigate' : 'Synthetic preview'}</small>
    </footer>
</div>
