import type { AutomationCase, AutomationStage } from './types';

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

export function incidentTitle(incident: AutomationCase): string {
    const subject = incident.trigger.target?.label ?? 'This workload';
    const titles: Record<string, string> = {
        system_cpu: 'Your computer is working hard',
        memory_pressure: 'Memory pressure detected',
        thermal_pressure: 'Your computer is running warm',
        app_cpu: `${subject} is busy`,
        cpu_anomaly: `${subject} became busier than usual`,
        memory_growth: `${subject}’s memory use grew`,
        app_unresponsive: `${subject} is not responding`,
    };
    return titles[incident.trigger.kind] ?? incident.trigger.title;
}

export function incidentObservation(incident: AutomationCase): string {
    const observations: Record<string, string> = {
        system_cpu: 'Total CPU use stayed above your configured limit.',
        memory_pressure: 'Your computer stayed under memory pressure.',
        thermal_pressure: 'The operating system reported elevated thermal pressure.',
        app_cpu: 'This workload stayed above your CPU limit. It may be doing useful work.',
        cpu_anomaly: 'CPU activity rose above this workload’s recent baseline.',
        memory_growth: 'Reported memory rose above its recent baseline. This alone is not a leak.',
        app_unresponsive: 'macOS repeatedly reported that the app was not responding.',
    };
    return observations[incident.trigger.kind] ?? 'Resource use crossed a monitoring limit.';
}

export function incidentOutcome(incident: AutomationCase): string {
    if (incident.verification) {
        if (!incident.report?.fresh)
            return 'The action was sent; a fresh measurement is unavailable.';
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
