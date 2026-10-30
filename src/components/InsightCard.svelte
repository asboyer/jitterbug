<script lang="ts">
    import type { Reply } from '../lib/types';
    import InfoTip from './InfoTip.svelte';
    let { reply }: { reply: Reply } = $props();
    const preview = $derived(reply.text.split('\n').find((line) => line.trim()) ?? reply.text);
</script>

<section class="insight-card" aria-label="Activity insight">
    <div class="insight-label">
        <span
            >{reply.source === 'model'
                ? 'Jitterbug insight'
                : reply.source === 'demo'
                  ? 'Demo insight'
                  : 'Current observations'}</span
        >
        {#if reply.investigation}<time
                datetime={new Date(reply.investigation.captured_at * 1000).toISOString()}
                >Checked {new Date(reply.investigation.captured_at * 1000).toLocaleTimeString([], {
                    hour: 'numeric',
                    minute: '2-digit',
                })}</time
            >{/if}
        <InfoTip label="How this insight was made">
            <p>
                {reply.source === 'model'
                    ? `${reply.provider} · ${reply.model}`
                    : 'No model was called.'}
            </p>
            {#if reply.source === 'model'}<p>
                    Estimated usage: ${reply.estimated_cost_usd.toFixed(5)}. Based on the latest
                    sample and recent history, not a live app trace.
                </p>{/if}
            {#each reply.route as step}<p>{step}</p>{/each}
        </InfoTip>
    </div>
    {#if reply.insight}
        <h3>{reply.insight.headline}</h3>
        <p>{reply.insight.finding}</p>
        <div class="next-step">
            <span>Next step</span>
            <p>{reply.insight.next_step}</p>
        </div>
    {:else}
        <h3>{preview.length > 160 ? `${preview.slice(0, 157)}…` : preview}</h3>
        {#if reply.text !== preview || preview.length > 160}<details>
                <summary>Read full explanation</summary>
                <p>{reply.text}</p>
            </details>{/if}
    {/if}
</section>

<style>
    .insight-card {
        min-width: 0;
    }
    .insight-label {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        color: var(--muted);
        font-size: 11px;
    }
    time {
        margin-left: auto;
        font-size: 10px;
        white-space: nowrap;
    }
    h3 {
        font-size: 19px;
        line-height: 1.3;
        letter-spacing: -0.35px;
        margin: 10px 0;
        font-weight: 600;
    }
    p {
        font-size: 13px;
        line-height: 1.6;
        margin: 0;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
    }
    .next-step {
        display: grid;
        gap: 6px;
        border-top: 1px solid var(--border);
        margin-top: 16px;
        padding-top: 14px;
    }
    .next-step > span {
        font-size: 11px;
        color: var(--muted);
        font-weight: 500;
    }
    details {
        margin-top: 12px;
        font-size: 11px;
        color: var(--muted);
    }
    details p {
        margin-top: 12px;
        color: var(--text);
    }
</style>
