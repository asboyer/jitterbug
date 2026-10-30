import { memory, percent } from './format';
import type {
    ActionPopupCase,
    ActionPopupOption,
    AutomationCase,
    AutomationStage,
    CaseOrigin,
} from './types';

/** The fields prompts need; full cases and the trimmed action popup payload both qualify. */
type IncidentSummary = Pick<AutomationCase, 'stage'> & {
    trigger: { kind: string; title: string; target: { label: string } | null };
    report: { fresh: boolean; after: unknown; action?: string } | null;
    verification: { priority_verified: boolean } | null;
    jitter?: { culprit: { label: string } | null; happened: string } | null;
};

export const incidentStageLabels: Record<AutomationStage, string> = {
    detected: 'Detected',
    investigating: 'Investigating',
    awaiting_approval: 'Needs your approval',
    applying: 'Applying',
    verifying: 'Checking the result',
    completed: 'Investigation complete',
    resolved: 'Trigger cleared',
    blocked: 'Needs attention',
    dismissed: 'Dismissed',
};

export function incidentTitle(incident: IncidentSummary): string {
    const subject = incident.trigger.target?.label ?? 'This workload';
    const titles: Record<string, string> = {
        system_cpu: 'Your computer is working hard',
        memory_pressure: 'Memory pressure detected',
        thermal_pressure: 'Your computer is running warm',
        app_cpu: `${subject} is busy`,
        cpu_anomaly: `${subject} became busier than usual`,
        memory_growth: `${subject}’s memory use grew`,
        app_unresponsive: `${subject} is not responding`,
        user_reported: incident.jitter?.culprit
            ? `Jitter Snapshot: ${incident.jitter.culprit.label}`
            : 'Jitter Snapshot',
        short_spike: incident.jitter?.culprit
            ? `Short hitch: ${incident.jitter.culprit.label}`
            : 'Short hitch detected',
    };
    return titles[incident.trigger.kind] ?? incident.trigger.title;
}

export function incidentObservation(incident: IncidentSummary): string {
    const observations: Record<string, string> = {
        system_cpu: 'Total CPU use stayed above your configured limit.',
        memory_pressure: 'Your computer stayed under memory pressure.',
        thermal_pressure: 'The operating system reported elevated thermal pressure.',
        app_cpu: 'This workload stayed above your CPU limit. It may be doing useful work.',
        cpu_anomaly: 'CPU activity rose above this workload’s recent baseline.',
        memory_growth: 'Reported memory rose above its recent baseline. This alone is not a leak.',
        app_unresponsive: 'macOS repeatedly reported that the app was not responding.',
    };
    if (incident.jitter) return incident.jitter.happened;
    const reason = (incident.trigger as { reason?: string | null }).reason;
    if (incident.trigger.kind === 'agent_request' && reason)
        return `An agent asked for this and gave the reason ${reason}`;
    return observations[incident.trigger.kind] ?? 'Resource use crossed a monitoring limit.';
}

export function incidentOutcome(incident: IncidentSummary): string {
    if (incident.verification) {
        if (!incident.report?.fresh)
            return 'The action was sent; a fresh measurement is unavailable.';
        if (incident.report.action === 'terminate')
            return incident.report.after
                ? 'The process is still running; it may be saving work or ignoring the request.'
                : 'The process stopped. A fresh sample no longer shows it.';
        if (!incident.report.after) return 'The original process is no longer in the inventory.';
        return incident.verification.priority_verified
            ? 'The operating system confirmed the lower priority.'
            : 'The priority change could not be confirmed.';
    }
    switch (incident.stage) {
        case 'detected':
            return 'Jitterbug will check the recent activity.';
        case 'investigating':
            return 'Jitterbug is checking what changed.';
        case 'awaiting_approval':
            return 'Review a CPU priority adjustment before it runs.';
        case 'applying':
            return 'Applying the approved priority adjustment.';
        case 'verifying':
            return 'Collecting a fresh reading and checking the operating system’s priority.';
        case 'completed':
            return incident.report
                ? 'The action was sent. Open the result to review what was measured.'
                : 'No automatic change was applied. Review the findings or ask a follow-up.';
        case 'resolved':
            return 'The original trigger or process changed. No optimization was applied.';
        case 'blocked':
            return 'Jitterbug could not finish this step. Open the details for the reason.';
        case 'dismissed':
            return 'This incident was dismissed. Any earlier changes remain in effect.';
    }
}

export type InboxFilter = 'all' | 'snapshot' | 'manual';

export const inboxFilterLabels: Record<InboxFilter, string> = {
    all: 'All',
    snapshot: 'Snapshots',
    manual: 'Manual',
};

/** Every case with captured one-second samples is a snapshot; the badge says who started it. */
export function snapshotBadge(incident: { origin?: CaseOrigin; jitter?: unknown }): string | null {
    if (!incident.jitter) return null;
    return incident.origin === 'snapshot' ? 'Snapshot · Manual' : 'Snapshot · Auto';
}

export function matchesInboxFilter(incident: AutomationCase, filter: InboxFilter): boolean {
    if (filter === 'snapshot') return !!incident.jitter;
    if (filter === 'manual') return incident.origin === 'snapshot';
    return true;
}

/** Friendly first line shared by the in-app notice and the out-of-app popup. */
export function noticeTitle(incident: IncidentSummary): string {
    const subject = incident.trigger.target?.label ?? 'an app';
    switch (incident.trigger.kind) {
        case 'app_unresponsive':
            return `Hey, ${subject} has stopped responding.`;
        case 'system_cpu':
            return 'Hey, your computer has stayed busy.';
        case 'memory_pressure':
            return 'Hey, your computer is short on memory.';
        case 'thermal_pressure':
            return 'Hey, your computer is running warm.';
        case 'memory_growth':
            return `Hey, ${subject}’s memory use has grown.`;
        case 'cpu_anomaly':
            return `Hey, ${subject} is busier than usual.`;
        case 'user_reported':
            return 'Got it. Here’s that moment.';
        case 'short_spike':
            return 'Hey, I caught a short hitch.';
        case 'agent_request':
            return `${incident.trigger.title}.`;
        default:
            return `Hey, ${subject} has stayed busy.`;
    }
}

/** What approving does, in plain words. */
export function proposalSummary(processName: string, victim?: string | null): string {
    return `Lower ${processName}’s CPU priority so ${victim ?? 'your other apps'} get the CPU first when they compete. ${processName} keeps running.`;
}

/** "45% → 12%", or the before value alone when no fresh reading arrived. */
function change(before: string, after: string | null): string {
    return after === null ? `${before} before` : `${before} → ${after}`;
}

/** Current use of the workload an option belongs to. */
export function nowLine(option: ActionPopupOption): string {
    const impact = option.impact;
    if (!impact) {
        return `Now: ${option.process.name} uses ${percent(option.process.cpu_percent)} CPU · ${memory(option.process.memory_bytes)}`;
    }
    const processes =
        impact.process_count === 1 ? '1 process' : `${impact.process_count} processes`;
    return `Now: ${impact.label} uses ${percent(impact.workload_cpu_percent)} CPU · ${memory(impact.workload_memory_bytes)} · ${processes}`;
}

/** A helper's name rarely says what it is; say so instead of implying it can be identified. */
export function helperNote(option: ActionPopupOption): string | null {
    const label = option.impact?.label;
    const name = option.process.name;
    if (!label || !/helper|renderer|gpu|plugin/i.test(name)) return null;
    const manager = /chrome|edge|brave|arc|vivaldi|opera/i.test(label)
        ? ` ${label}’s Task Manager (Window → Task Manager) shows which tab.`
        : '';
    return `Busiest part: ${name}, a tab, extension, or background part of ${label}. macOS doesn’t say which.${manager}`;
}

export function priorityAfter(victim?: string | null): string {
    return `After: same usage; ${victim ?? 'your other apps'} go first when they compete.`;
}

/** A stop frees what ends: the whole app for its main executable, else that one process. */
export function stopAfter(option: ActionPopupOption): string {
    const impact = option.impact;
    if (option.action === 'pause')
        return `Now: ${percent(option.impact?.workload_cpu_percent ?? 0)} CPU · ${memory(option.impact?.workload_memory_bytes ?? 0)}. After: no CPU until resumed; memory stays allocated`;
    if (option.action === 'close_tab')
        return `After: frees about ${percent(impact?.workload_cpu_percent ?? 0)} CPU · ${memory(impact?.workload_memory_bytes ?? 0)}`;
    const [cpu, bytes] = impact?.whole_app
        ? [impact.workload_cpu_percent, impact.workload_memory_bytes]
        : [option.process.cpu_percent, option.process.memory_bytes];
    return `After: frees about ${percent(cpu)} CPU · ${memory(bytes)}`;
}

export function stopLabel(option: ActionPopupOption): string {
    if (option.action === 'pause') return `Pause ${option.unit?.title ?? option.process.name}`;
    if (option.action === 'close_tab')
        return `Close the “${option.unit?.title ?? option.process.name}” tab`;
    return option.impact?.whole_app ? `Quit ${option.impact.label}` : `Stop ${option.process.name}`;
}

/** What an option ends, without repeating its label: "Pause X: its 7 processes…" → "Its 7 processes…". */
export function stopWarning(option: ActionPopupOption): string {
    const text = option.closes ?? '';
    const label = stopLabel(option);
    if (!text.startsWith(label)) return text;
    const rest = text
        .slice(label.length)
        .replace(/^\s*\([^)]*\)/, '')
        .replace(/^[\s.:;,]+/, '');
    return rest ? rest.charAt(0).toUpperCase() + rest.slice(1) : text;
}

/** The popup's account of a finished action: what, what it means, and before → after. */
export function actionResult(
    report: NonNullable<ActionPopupCase['report']>,
    verification: ActionPopupCase['verification'],
    option: ActionPopupOption | null,
    victim?: string | null
): { title: string; meaning: string; measured: string; note: string } {
    const name = report.process;
    const machine = verification
        ? `whole Mac CPU ${change(
              percent(verification.system_cpu_before),
              verification.system_cpu_after === null ? null : percent(verification.system_cpu_after)
          )}`
        : '';
    if (report.action === 'pause') {
        return {
            title: `Paused ${name}`,
            meaning: `${name} is frozen, not closed: no CPU until you resume it, and it resumes by itself after 15 minutes. Its memory stays allocated; macOS can compress or swap it for your other work.`,
            measured: report.fresh
                ? `${name} ${change(
                      percent(report.cpu_before),
                      report.cpu_after === null ? null : percent(report.cpu_after)
                  )} · ${machine}`
                : 'A fresh measurement was unavailable.',
            note: '',
        };
    }
    if (report.action === 'close_tab') {
        const free = verification
            ? `free memory ${change(
                  memory(verification.available_memory_before),
                  verification.available_memory_after === null
                      ? null
                      : memory(verification.available_memory_after)
              )}`
            : '';
        return {
            title: `Closed the “${name}” tab`,
            meaning: 'The rest of the browser stays open.',
            measured: report.fresh
                ? [machine, free].filter(Boolean).join(' · ')
                : 'A fresh measurement was unavailable.',
            note: 'The browser releases the tab’s memory shortly; other activity may also have changed.',
        };
    }
    if (report.action === 'terminate') {
        const gone = report.fresh && report.after === null;
        const what = option?.impact?.whole_app
            ? `Closed ${option.impact.label}`
            : `Stopped ${name}`;
        const free = verification
            ? `free memory ${change(
                  memory(verification.available_memory_before),
                  verification.available_memory_after === null
                      ? null
                      : memory(verification.available_memory_after)
              )}`
            : '';
        return {
            title: gone ? what : `${name} is still running`,
            meaning: gone
                ? `${name} is gone. ${option?.closes ?? ''}`.trim()
                : 'It may be saving work or ignoring the request. Check it before trying again.',
            measured: report.fresh
                ? [machine, free].filter(Boolean).join(' · ')
                : 'A fresh measurement was unavailable.',
            note: gone ? 'Measured one sample later; other activity may also have changed.' : '',
        };
    }
    const verified = !!verification?.priority_verified;
    const dropped = report.cpu_after !== null && report.cpu_after < report.cpu_before - 1;
    return {
        title: verified
            ? `Lowered ${name}’s priority`
            : `Priority change for ${name} not confirmed`,
        meaning: verified
            ? `${name} keeps running. When apps compete for CPU, macOS now runs ${victim ?? 'your other apps'} first.`
            : 'macOS did not confirm the new priority. Open Review for details.',
        measured: report.fresh
            ? `${name} ${change(
                  percent(report.cpu_before),
                  report.cpu_after === null ? null : percent(report.cpu_after)
              )} · ${machine}`
            : 'A fresh measurement was unavailable.',
        note: !report.fresh
            ? ''
            : dropped
              ? 'Its CPU use dropped; other activity may also have changed.'
              : 'No drop is expected: lower priority doesn’t cap CPU, it only decides who goes first. Lasts until it exits.',
    };
}
