<script lang="ts">
    import { memory, percent } from '../lib/format';
    import InfoTip from './InfoTip.svelte';
    import Icon from './Icon.svelte';
    import type { AgentWorkflow, AgentRoleUsage } from '../lib/types';

    let { workflow, interval }: { workflow: AgentWorkflow; interval: number } = $props();
    const live = $derived(workflow.status === 'active');
    const cpu = $derived(live ? workflow.cpu_percent : workflow.peak_cpu_percent);
    const measured = $derived(workflow.observed_samples > 0);
    const roles = $derived.by(() => {
        const combined = new Map<string, AgentRoleUsage>();
        const source = live ? workflow.agents.flatMap((agent) => agent.roles) : workflow.peak_roles;
        for (const role of source) {
            const previous = combined.get(role.role);
            combined.set(role.role, {
                role: role.role,
                cpu_percent: (previous?.cpu_percent ?? 0) + role.cpu_percent,
                memory_bytes: (previous?.memory_bytes ?? 0) + role.memory_bytes,
                process_count: (previous?.process_count ?? 0) + role.process_count,
            });
        }
        return [...combined.values()]
            .filter((role) => role.process_count > 0)
            .sort((a, b) => b.cpu_percent - a.cpu_percent || b.memory_bytes - a.memory_bytes);
    });
    const scale = $derived(Math.max(1, Math.ceil(workflow.peak_cpu_percent / 5) * 5));
    const points = $derived(
        workflow.history
            .map((point, index) => {
                const x = (index / Math.max(workflow.history.length - 1, 1)) * 240;
                const y = 40 - Math.min(point.cpu_percent / scale, 1) * 38;
                return `${x},${y}`;
            })
            .join(' ')
    );
    const names: Record<string, string> = {
        agent: 'Agent',
        test: 'Tests',
        build: 'Build',
        typecheck: 'Types',
        lint: 'Lint',
        other: 'Other',
    };
</script>

<div class="resource-heading">
    <span>{live ? 'Live resource use' : 'Recorded resource use'}</span>
    <InfoTip label="About task measurements">
        <p>{workflow.narrative.resources}</p>
        <p>
            CPU is a share of all cores, sampled every {interval / 1000}s. Short bursts may be
            missed. Memory is summed RSS and can include shared pages more than once. Completed
            measurements last for this app session.
        </p>
        <p>
            Process roles are inferred from names and commands. Bars show each role’s share of this
            task’s CPU, not a proven cause of slowness.
        </p>
    </InfoTip>
</div>
<div class="resources">
    <div
        class="reading"
        title={`Share of total CPU capacity, sampled every ${interval / 1000}s. Short bursts can be missed.`}
    >
        <span><Icon name="cpu" size={14} />{live ? 'CPU now' : 'Sampled CPU peak'}</span>
        <strong class:unavailable={!measured}>
            {measured ? percent(cpu) : '—'}
        </strong>
        <small
            >{!measured
                ? live
                    ? 'Waiting for samples'
                    : 'Not sampled'
                : cpu === 0
                  ? 'No CPU activity recorded'
                  : live
                    ? `peak ${percent(workflow.peak_cpu_percent)}`
                    : 'of total capacity'}</small
        >
    </div>
    <div class="reading">
        <span><Icon name="memory" size={14} />{live ? 'Memory now' : 'Peak memory'}</span>
        <strong class:unavailable={workflow.peak_process_count === 0}>
            {workflow.peak_process_count === 0
                ? '—'
                : memory(live ? workflow.memory_bytes : workflow.peak_memory_bytes)}
        </strong>
        <small
            >{workflow.peak_process_count === 0
                ? 'Not sampled'
                : live
                  ? `peak ${memory(workflow.peak_memory_bytes)}`
                  : 'retained peak'}</small
        >
    </div>
    <div class="history">
        <div><span>CPU history</span><small>{measured ? `0–${scale}%` : 'No samples'}</small></div>
        <svg
            viewBox="0 0 240 42"
            preserveAspectRatio="none"
            role="img"
            aria-label={`Task CPU history. Scale zero to ${scale} percent of total capacity. ${workflow.history.length} samples.`}
        >
            <path d="M0 20H240 M0 40H240" class="grid" />
            {#if points}
                <polygon points={`0,40 ${points} 240,40`} />
                <polyline {points} />
                {#if workflow.history.length === 1}
                    <circle
                        cx="2"
                        cy={40 - Math.min(workflow.history[0].cpu_percent / scale, 1) * 38}
                        r="2"
                        fill="var(--accent)"
                    />
                {/if}
            {/if}
        </svg>
        <small
            >{measured
                ? `${workflow.observed_samples} ${workflow.observed_samples === 1 ? 'sample' : 'samples'} · every ${interval / 1000}s`
                : 'Measurements last for this app session'}</small
        >
    </div>
</div>
{#if measured && roles.length > 0}
    <div
        class="role-strip"
        aria-label={live ? 'Current process roles' : 'Process roles at sampled CPU peak'}
    >
        {#each roles as role}
            <div
                class="role"
                title={`${role.process_count} processes · ${memory(role.memory_bytes)} in ${live ? 'the current' : 'the peak CPU'} sample. Roles inferred from process names and commands.`}
            >
                <span>{names[role.role] ?? role.role}</span>
                <span class="role-meter" aria-hidden="true"
                    ><i
                        style={`width:${cpu > 0 ? Math.min((role.cpu_percent / cpu) * 100, 100) : 0}%`}
                    ></i></span
                >
                <strong>{percent(role.cpu_percent)}</strong>
            </div>
        {/each}
    </div>
{/if}

<style>
    .resource-heading {
        display: flex;
        align-items: center;
        justify-content: space-between;
        color: var(--muted);
        font-size: 11px;
        margin-bottom: 10px;
    }
    .resources {
        display: grid;
        grid-template-columns: 1fr 1fr 1.65fr;
        gap: 12px;
        padding: 0;
    }
    .reading {
        display: grid;
        align-content: start;
        gap: 9px;
        background: var(--hover);
        border-radius: 8px;
        padding: 14px;
    }
    .reading > span {
        display: flex;
        align-items: center;
        gap: 7px;
    }
    .reading > span,
    .history {
        padding: 8px 0 0 8px;
    }
    .history > div {
        font-size: 11px;
        color: var(--muted);
    }
    .reading strong {
        font-size: 26px;
        font-weight: 550;
        letter-spacing: -0.6px;
        font-variant-numeric: tabular-nums;
    }
    .reading .unavailable {
        color: var(--muted);
    }
    small {
        color: var(--muted);
        font-size: 10px;
    }
    .history {
        padding: 8px 0 0 8px;
    }
    .history > div {
        display: flex;
        justify-content: space-between;
    }
    svg {
        width: 100%;
        height: 62px;
        display: block;
        margin: 3px 0;
    }
    .grid {
        stroke: var(--border);
        stroke-width: 0.5;
    }
    polygon {
        fill: var(--accent-soft);
    }
    polyline {
        fill: none;
        stroke: var(--accent);
        stroke-width: 1.5;
        vector-effect: non-scaling-stroke;
    }
    .role-strip {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
        gap: 10px 20px;
        padding: 16px 0 0;
    }
    .role {
        display: grid;
        grid-template-columns: 1fr auto;
        gap: 7px;
        font-size: 11px;
    }
    .role > span:first-child {
        color: var(--muted);
    }
    .role strong {
        font-size: 10px;
        font-weight: 500;
        font-variant-numeric: tabular-nums;
    }
    .role-meter {
        width: 100%;
        height: 5px;
        grid-row: 2;
        grid-column: 1 / -1;
        border-radius: 3px;
        background: var(--track);
        overflow: hidden;
    }
    .role-meter i {
        display: block;
        height: 100%;
        background: var(--accent);
        opacity: 0.7;
    }
    p {
        line-height: 1.6;
        margin: 8px 0;
    }
    @media (max-width: 760px) {
        .resource-heading {
            display: flex;
            align-items: center;
            justify-content: space-between;
            color: var(--muted);
            font-size: 11px;
            margin-bottom: 10px;
        }
        .resources {
            gap: 12px;
            grid-template-columns: 1fr 1fr;
        }
        .history {
            grid-column: 1 / -1;
        }
    }
</style>
