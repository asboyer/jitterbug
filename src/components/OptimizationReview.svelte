<script lang="ts">
    import { onMount } from 'svelte';
    import type { AutomationCase } from '../lib/types';
    import { percent } from '../lib/format';

    let {
        incident,
        current,
        busy,
        error,
        canApply,
        onapply,
        onclose,
    }: {
        incident: AutomationCase;
        current: boolean;
        busy: boolean;
        error: string;
        canApply: boolean;
        onapply: () => void;
        onclose: () => void;
    } = $props();
    let dialog: HTMLDialogElement;
    let cancel: HTMLButtonElement;
    const label = $derived(
        incident.trigger.target?.label ?? incident.proposal?.process.name ?? 'this workload'
    );
    onMount(() => {
        dialog.showModal();
        cancel.focus();
        return () => dialog.close();
    });
</script>

<dialog
    bind:this={dialog}
    class="optimization-review"
    aria-labelledby="optimization-review-title"
    oncancel={(event) => {
        event.preventDefault();
        if (!busy) onclose();
    }}
>
    <h2 id="optimization-review-title">Review priority change</h2>
    <p>
        Move <strong>{label}</strong> to Background: it keeps running while other apps get CPU and disk
        first.
    </p>
    <ul class="effects">
        <li>This does not free memory or guarantee lower CPU use.</li>
        <li>
            It stays in Background, including processes it starts later, until you move it back to
            Normal. It runs at full speed while it is in front.
        </li>
    </ul>
    <details class="process-details">
        <summary>Process details</summary>
        <p>
            {incident.proposal?.process.name} · PID {incident.proposal?.process.pid} · {percent(
                incident.proposal?.process.cpu_percent ?? 0
            )} CPU when proposed
        </p>
        <p>{incident.proposal?.rationale}</p>
    </details>
    {#if incident.source !== 'live'}<p class="demo-note">
            Example only. Nothing will be applied.
        </p>{/if}
    {#if !current}<p role="status" class="error">
            This suggestion changed. Close this review and check its latest state.
        </p>{/if}
    {#if error}<p role="alert" class="error">{error}</p>{/if}
    <div class="review-actions">
        <button bind:this={cancel} disabled={busy} onclick={onclose}>Cancel</button>
        <button class="primary" disabled={!canApply || !current || busy} onclick={() => onapply()}
            >{busy ? 'Moving…' : 'Move to Background'}</button
        >
    </div>
</dialog>

<style>
    .optimization-review {
        background: var(--panel);
        color: var(--text);
        width: min(420px, calc(100vw - 40px));
        max-height: calc(100vh - 40px);
        padding: 24px;
        border: 1px solid var(--border);
        border-radius: 14px;
        box-shadow: 0 20px 70px #0005;
    }
    .optimization-review::backdrop {
        background: #0007;
    }
    h2 {
        margin: 0 0 16px;
        font-size: 18px;
    }
    p,
    li {
        font-size: 13px;
        line-height: 1.6;
    }
    .effects {
        margin: 14px 0 18px;
        padding-left: 18px;
        color: var(--muted);
    }
    .effects li + li {
        margin-top: 8px;
    }
    .process-details {
        font-size: 12px;
        color: var(--muted);
        margin-bottom: 18px;
    }
    summary {
        cursor: pointer;
    }
    .demo-note {
        color: var(--muted);
        font-size: 12px;
    }
    .review-actions {
        display: flex;
        justify-content: flex-end;
        gap: 10px;
        margin-top: 24px;
    }
</style>
