<script lang="ts">
    import { onMount } from 'svelte';
    import HarnessBadge from './HarnessBadge.svelte';
    import Icon from './Icon.svelte';
    import type { AgentWorkflow } from '../lib/types';
    let { workflow, onclose }: { workflow: AgentWorkflow; onclose: () => void } = $props();
    let queued = $state(false);
    let dialog: HTMLDialogElement;
    let close: HTMLButtonElement;
    onMount(() => {
        dialog.showModal();
        close.focus();
        return () => dialog.close();
    });
</script>

<dialog
    bind:this={dialog}
    aria-labelledby="queue-preview-title"
    oncancel={(event) => {
        event.preventDefault();
        onclose();
    }}
>
    <div class="preview-top">
        <span>INTERACTIVE EXAMPLE · NO PROCESSES CHANGED</span><button
            class="icon-button"
            bind:this={close}
            aria-label="Close queue preview"
            onclick={onclose}>×</button
        >
    </div>
    <h2 id="queue-preview-title">Give your work room to finish</h2>
    <p class="task-name">{workflow.task_title}</p>
    <div
        class="schedule"
        class:queued
        aria-label={queued ? 'Example: checks take turns' : 'Example: checks overlap'}
    >
        <div class="scale"><span>Earlier</span><span>Later →</span></div>
        {#each workflow.agents.slice(0, 2) as agent, index}
            <div class="lane">
                <HarnessBadge harness={agent.harness} />
                <div class="track">
                    <div class="check" class:second={index === 1}>
                        <Icon name="code" size={14} /> Heavy check
                    </div>
                </div>
            </div>
        {/each}
    </div>
    <div class="preview-result" aria-live="polite">
        <strong
            >{queued
                ? 'One runs. The next waits for room.'
                : 'Two checks run at the same time.'}</strong
        >
        <p>
            {queued
                ? 'Submitted checks take turns across sessions. The computer is checked again before the next starts.'
                : 'Overlapping checks can compete with the apps you’re using.'}
        </p>
    </div>
    <p class="scope">
        This previews future submitted checks. Existing tests keep running. Actual CPU, memory, and
        completion time will vary.
    </p>
    <div class="actions">
        <button
            onclick={() => {
                queued = !queued;
            }}
            class="primary"
            >{queued ? 'Compare with overlapping' : 'Preview checks taking turns'}</button
        ><button onclick={onclose}>Done</button>
    </div>
</dialog>

<style>
    dialog {
        width: min(520px, calc(100vw - 40px));
        max-height: calc(100vh - 40px);
        padding: 24px;
        border: 1px solid var(--border);
        border-radius: 16px;
        background: var(--panel);
        color: var(--text);
        box-shadow: 0 20px 70px #0005;
    }
    dialog::backdrop {
        background: #0007;
    }
    .preview-top {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 12px;
    }
    .preview-top > span {
        font-size: 9px;
        letter-spacing: 0.5px;
        color: var(--muted);
    }
    h2 {
        font-size: 21px;
        margin: 16px 0 8px;
    }
    .task-name {
        color: var(--muted);
        font-size: 12px;
        margin: 0 0 24px;
    }
    .schedule {
        padding: 16px;
        background: var(--hover);
        border-radius: 10px;
    }
    .scale {
        display: flex;
        justify-content: space-between;
        padding-left: 110px;
        font-size: 9px;
        color: var(--muted);
        margin-bottom: 14px;
    }
    .lane {
        display: grid;
        grid-template-columns: 100px 1fr;
        gap: 10px;
        align-items: center;
        margin: 12px 0;
    }
    .track {
        background: var(--track);
        border-radius: 6px;
    }
    .check {
        width: 49%;
        height: 36px;
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 6px;
        font-size: 10px;
        background: var(--accent-soft);
        color: var(--accent);
        border: 1px solid var(--accent);
        border-radius: 6px;
        transition: transform 250ms ease;
    }
    .queued .second {
        transform: translateX(104%);
    }
    .preview-result {
        margin: 22px 0 14px;
    }
    .preview-result strong {
        font-size: 15px;
    }
    p {
        font-size: 13px;
        line-height: 1.6;
    }
    .scope {
        font-size: 11px;
        color: var(--muted);
    }
    .actions {
        display: flex;
        flex-wrap: wrap;
        gap: 10px;
        margin-top: 22px;
    }
    @media (prefers-reduced-motion: reduce) {
        .check {
            transition: none;
        }
    }
</style>
