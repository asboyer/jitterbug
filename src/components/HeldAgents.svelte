<script lang="ts">
    // Agent sessions the governor holds to keep the computer responsive, with a way to resume
    // one now. Hidden while the governor is off and holds nothing.
    import { invoke } from '@tauri-apps/api/core';
    import { errorMessage } from '../lib/format';
    import type { Governor } from '../lib/types';

    let {
        governor,
        demo = false,
        compact = false,
    }: { governor: Governor; demo?: boolean; compact?: boolean } = $props();

    let busy = $state(false);
    let notice = $state('');

    const clock = (time: number) =>
        new Date(time * 1000).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

    async function resume(pid: number, startTime: number) {
        busy = true;
        try {
            notice = await invoke<string>('session_control', {
                pid,
                startTime,
                action: 'resume',
                label: '',
            });
        } catch (cause) {
            notice = errorMessage(cause);
        } finally {
            busy = false;
        }
    }
</script>

{#if governor.enabled || governor.held.length}
    <section class="held" class:compact aria-label="Agents held for room">
        <p class="summary">
            <span class="dot" class:holding={governor.held.length > 0}></span>
            {governor.held.length
                ? `Holding ${governor.held.length} agent session${governor.held.length === 1 ? '' : 's'} so this computer stays responsive`
                : 'Pausing agents if the computer gets strained · every agent is running'}
        </p>
        {#if governor.held.length}
            <ul>
                {#each governor.held as held (`${held.pid}-${held.start_time}`)}
                    <li>
                        <div>
                            <strong>{held.title}</strong>
                            {#if !compact}<small
                                    >Paused at {clock(held.paused_at)} because {held.reason}.
                                    Resumes when there is room, by {clock(held.until)} at the latest.</small
                                >{/if}
                        </div>
                        <button
                            disabled={busy || demo}
                            onclick={() => resume(held.pid, held.start_time)}>Resume</button
                        >
                    </li>
                {/each}
            </ul>
        {:else if governor.note && !compact}
            <small class="note">{governor.note}</small>
        {/if}
        {#if notice}<small class="note" role="status">{notice}</small>{/if}
    </section>
{/if}

<style>
    .held {
        display: grid;
        gap: 8px;
        padding: 12px 14px;
        margin: 0 0 18px;
        border: 1px solid var(--border);
        border-radius: 12px;
        background: var(--panel);
    }
    .held.compact {
        flex-shrink: 0;
        gap: 6px;
        margin: 10px 16px 0;
        padding: 8px 10px;
        font-size: 11px;
    }
    .compact .summary {
        font-weight: 550;
    }
    .compact button {
        font-size: 11px;
    }
    .summary {
        display: flex;
        align-items: center;
        gap: 8px;
        margin: 0;
        font-weight: 600;
    }
    .dot {
        flex: none;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: var(--success);
    }
    .dot.holding {
        background: var(--warning);
    }
    ul {
        display: grid;
        gap: 6px;
        margin: 0;
        padding: 0;
        list-style: none;
    }
    li {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
    }
    li div {
        display: grid;
        gap: 2px;
        min-width: 0;
    }
    strong {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    small {
        color: var(--muted);
        line-height: 1.4;
    }
</style>
