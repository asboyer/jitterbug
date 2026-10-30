import type { AgentWorkflow } from './types';

export function currentTip(workflow: AgentWorkflow) {
    if (workflow.status !== 'active') return undefined;
    const priority = (id: string) =>
        id === 'memory-headroom' ? 4 : id.startsWith('overlapping-') ? 3 : 1;
    return workflow.suggestions
        .filter((tip) => tip.current)
        .sort((a, b) => priority(b.id) - priority(a.id))[0];
}

export function taskImpact(workflow: AgentWorkflow): string {
    if (!workflow.observed_samples) return 'I haven’t captured resource samples for this task yet.';
    const active = workflow.status === 'active';
    const cpu = active ? workflow.cpu_percent : workflow.peak_cpu_percent;
    if (cpu === 0)
        return active ? 'No CPU activity in the latest sample.' : 'No CPU activity was sampled.';
    const totals = new Map<string, number>();
    const roles = active ? workflow.agents.flatMap((agent) => agent.roles) : workflow.peak_roles;
    for (const role of roles)
        totals.set(role.role, (totals.get(role.role) ?? 0) + role.cpu_percent);
    const leader = [...totals].sort((a, b) => b[1] - a[1])[0];
    const names: Record<string, string> = {
        test: 'Test runners',
        build: 'Build tools',
        typecheck: 'Type checks',
        lint: 'Lint checks',
        agent: 'Agent tools',
    };
    if (leader && names[leader[0]] && leader[1] > cpu / 2)
        return active
            ? `${names[leader[0]]} are using most of this task’s CPU.`
            : `${names[leader[0]]} led CPU use at the sampled peak.`;
    return active
        ? 'Resource use is spread across this task’s processes.'
        : 'Peak measurements are saved below; this task is no longer running.';
}
