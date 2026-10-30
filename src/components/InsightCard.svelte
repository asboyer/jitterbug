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
                ? 'Findings'
                : reply.source === 'demo'
                  ? 'Demo insight'
                  : reply.unavailable
                    ? 'Local checks · AI unavailable'
                    : 'Local checks'}</span
        >
        {#if reply.investigation}<time
                datetime={new Date(reply.investigation.captured_at * 1000).toISOString()}
                >{new Date(reply.investigation.captured_at * 1000).toLocaleTimeString([], {
                    hour: 'numeric',
                    minute: '2-digit',
                })}</time
            >{/if}
        <InfoTip label="How this insight was made">
            <p>
                {reply.source === 'model'
                    ? `${reply.provider} · ${reply.model}`
                    : reply.unavailable
                      ? 'AI investigation was unavailable. Local checks are shown below.'
                      : 'Measured locally. No model call was needed.'}
            </p>
            {#if reply.source === 'model'}<p>
                    Estimated usage: ${reply.estimated_cost_usd.toFixed(5)}. Based on the latest
                    sample and recent history, not a live app trace.
                </p>{/if}
            {#each reply.route as step}<p>{step}</p>{/each}
        </InfoTip>
    </div>
    {#if reply.unavailable}<p class="unavailable" role="status">{reply.unavailable}</p>{/if}
    {#if reply.insight}
        <h3>{reply.insight.headline}</h3>
        <p>{reply.insight.finding}</p>
        <div class="next-step">
            <span>Try next <span aria-hidden="true">↗</span></span>
            <p>{reply.insight.next_step}</p>
        </div>
    {:else}
        <h3>{preview.length > 160 ? `${preview.slice(0, 157)}…` : preview}</h3>
        {#if reply.text !== preview || preview.length > 160}<details>
                <summary>Read full explanation</summary>
                <p>{reply.text}</p>
            </details>{/if}
    {/if}
    {#if reply.investigation?.responsiveness}<details class="checks">
            <summary>What I checked</summary>
            {#each reply.investigation.stack_checks ?? [] as check}<div class="check-result">
                    <strong
                        >{check.role} · {check.state === 'unavailable'
                            ? 'Unavailable'
                            : 'Sampled'}</strong
                    >
                    <p>{check.summary}</p>
                </div>{/each}
            <p>
                {reply.investigation.responsiveness === 'not_responding'
                    ? 'Window: macOS reports not responding.'
                    : reply.investigation.responsiveness === 'responding'
                      ? 'Window: macOS is not reporting a hang. Individual tabs can still be stuck.'
                      : 'Window responsiveness: unavailable.'}
            </p>
            {#if reply.investigation.recently_unresponsive}<p>
                    A hang was observed in the recent history.
                </p>{/if}
            <p>{reply.investigation.coverage}</p>
        </details>{/if}
    {#if reply.research}<details class="research">
            <summary
                >{reply.research.status === 'checked'
                    ? 'Apple guidance · checked online'
                    : reply.research.status === 'cached'
                      ? 'Apple guidance · cached'
                      : 'Apple guidance · offline reference'}</summary
            >
            <p>{reply.research.summary}</p>
            {#if reply.research.status === 'lookup_unavailable'}<p>
                    Live lookup was unavailable.
                </p>{/if}
            <a href={reply.research.url} target="_blank" rel="noreferrer"
                >Read Apple’s troubleshooting guide ↗</a
            >
        </details>{/if}
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
        font-size: 21px;
        line-height: 1.25;
        letter-spacing: -0.35px;
        margin: 14px 0 10px;
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
        margin-top: 18px;
        padding: 14px;
        border-radius: 12px;
        background: var(--accent-soft);
    }
    .unavailable {
        margin: 12px 0;
        padding: 10px 12px;
        border-radius: 8px;
        background: var(--hover);
        border-left: 3px solid var(--muted);
        font-size: 12px;
    }
    .research a {
        display: inline-block;
        margin-top: 10px;
        color: var(--accent);
    }
    .next-step > span {
        font-size: 11px;
        color: var(--muted);
        font-weight: 500;
    }
    .check-result {
        margin: 14px 0;
    }
    .check-result strong {
        color: var(--text);
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
