//! Processes Jitterbug put in macOS background mode, kept in memory, and the per-sample plan
//! that keeps each app's priority bucket true: Background apps' new processes enter background
//! mode, while the app in front and Protected apps leave it. A process leaves the list when it
//! exits; background mode ends with it.
use crate::{
    automation::{Bucket, Policy},
    types::Snapshot,
};
use std::collections::HashSet;

/// Upper bound on tracked processes; a Background app beyond it gets no more changes.
const LIMIT: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub pid: u32,
    pub start_time: u64,
    pub workload_id: String,
    pub at: u64,
    /// The incident that applied it.
    pub case: Option<u64>,
}

#[derive(Default)]
pub struct Ledger {
    entries: Vec<Change>,
    /// Processes the OS or the permission check refused once; not retried every sample.
    refused: Vec<(u32, u64)>,
}

impl Ledger {
    /// Records a process put in background mode. A process recorded again keeps its first time.
    pub fn record(&mut self, change: Change) {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|c| c.pid == change.pid && c.start_time == change.start_time)
        {
            existing.case = change.case.or(existing.case);
            return;
        }
        if self.entries.len() < LIMIT {
            self.entries.push(change);
        }
    }

    pub fn forget(&mut self, pid: u32, start_time: u64) {
        self.entries
            .retain(|c| c.pid != pid || c.start_time != start_time);
    }

    pub fn refuse(&mut self, pid: u32, start_time: u64) {
        if self.refused.len() < LIMIT {
            self.refused.push((pid, start_time));
        }
    }

    pub fn entries(&self) -> &[Change] {
        &self.entries
    }

    fn holds(&self, pid: u32, start_time: u64) -> bool {
        self.entries
            .iter()
            .any(|c| c.pid == pid && c.start_time == start_time)
            || self.refused.contains(&(pid, start_time))
    }

    /// Processes of one app that Jitterbug keeps in background mode now.
    pub fn count(&self, workload_id: &str) -> usize {
        self.entries
            .iter()
            .filter(|c| c.workload_id == workload_id)
            .count()
    }

    /// Drops processes absent from a full sample: they exited, or their PID now names another
    /// process, which was never changed.
    pub fn retain_live(&mut self, snapshot: &Snapshot) {
        let live: HashSet<(u32, u64)> = snapshot
            .processes
            .iter()
            .map(|p| (p.pid, p.start_time))
            .collect();
        self.entries
            .retain(|c| live.contains(&(c.pid, c.start_time)));
        self.refused.retain(|key| live.contains(key));
    }
}

/// One process to change: identity plus the app it belongs to and its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub pid: u32,
    pub start_time: u64,
    pub workload_id: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Processes of Background apps that are not in background mode yet.
    pub apply: Vec<Target>,
    /// Processes Jitterbug put in background mode that must leave it: their app is in front
    /// or Protected.
    pub clear: Vec<Target>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.apply.is_empty() && self.clear.is_empty()
    }
}

/// What one sample needs. Only processes the collector already marked as allowed to lower
/// (this user's, never Jitterbug, its ancestors, or system processes) are planned; the action
/// rechecks fresh identity before changing anything. Call `Ledger::retain_live` first.
pub fn plan(snapshot: &Snapshot, policy: &Policy, ledger: &Ledger) -> Plan {
    let front = snapshot.frontmost_workload.as_deref();
    let clear = ledger
        .entries
        .iter()
        .filter(|c| Some(c.workload_id.as_str()) == front || policy.protected(&c.workload_id))
        .map(|c| Target {
            pid: c.pid,
            start_time: c.start_time,
            workload_id: c.workload_id.clone(),
        })
        .collect();
    let own = std::process::id();
    let apply = snapshot
        .processes
        .iter()
        .filter(|p| {
            p.pid != own
                && Some(p.workload_id.as_str()) != front
                && policy.bucket(&p.workload_id) == Bucket::Background
                && p.actions.iter().any(|a| a == "lower_priority")
                && !ledger.holds(p.pid, p.start_time)
        })
        .take(LIMIT.saturating_sub(ledger.entries.len()))
        .map(|p| Target {
            pid: p.pid,
            start_time: p.start_time,
            workload_id: p.workload_id.clone(),
        })
        .collect();
    Plan { apply, clear }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::{AssignedBy, Assignment};
    use crate::types::ProcessRecord;

    fn process(pid: u32, start_time: u64, workload: &str) -> ProcessRecord {
        ProcessRecord {
            pid,
            start_time,
            name: format!("p{pid}"),
            workload_id: workload.into(),
            actions: vec!["lower_priority".into()],
            ..Default::default()
        }
    }

    fn policy(entries: &[(&str, Bucket)]) -> Policy {
        let mut policy = Policy::default();
        for (id, bucket) in entries {
            policy.assign(Assignment {
                workload_id: id.to_string(),
                label: id.to_string(),
                bucket: *bucket,
                since: 1,
                source: AssignedBy::You,
            });
        }
        policy
    }

    fn change(pid: u32, start_time: u64, workload: &str) -> Change {
        Change {
            pid,
            start_time,
            workload_id: workload.into(),
            at: 1,
            case: None,
        }
    }

    fn pids(targets: &[Target]) -> Vec<u32> {
        targets.iter().map(|t| t.pid).collect()
    }

    #[test]
    fn background_apps_get_new_processes_and_nothing_else_is_touched() {
        let mut snapshot = Snapshot {
            processes: vec![
                process(10, 1, "bg"),
                process(11, 1, "bg"),
                process(20, 1, "normal"),
                process(30, 1, "guarded"),
            ],
            ..Default::default()
        };
        // A system or another user's process carries no lower_priority action.
        snapshot.processes[1].actions.clear();
        let policy = policy(&[("bg", Bucket::Background), ("guarded", Bucket::Protected)]);
        let mut ledger = Ledger::default();
        assert_eq!(pids(&plan(&snapshot, &policy, &ledger).apply), [10]);
        ledger.record(change(10, 1, "bg"));
        assert!(plan(&snapshot, &policy, &ledger).is_empty());
        // A helper the app starts later is moved on the next sample.
        snapshot.processes.push(process(12, 5, "bg"));
        assert_eq!(pids(&plan(&snapshot, &policy, &ledger).apply), [12]);
        // A refusal is not retried every sample.
        ledger.refuse(12, 5);
        assert!(plan(&snapshot, &policy, &ledger).is_empty());
        // Jitterbug itself is never planned, whatever its bucket.
        snapshot
            .processes
            .push(process(std::process::id(), 1, "bg"));
        assert!(plan(&snapshot, &policy, &ledger).apply.is_empty());
    }

    #[test]
    fn the_app_in_front_always_wins_and_is_moved_again_once_it_leaves() {
        let mut snapshot = Snapshot {
            processes: vec![process(10, 1, "bg"), process(11, 1, "bg")],
            frontmost_workload: Some("bg".into()),
            ..Default::default()
        };
        let policy = policy(&[("bg", Bucket::Background)]);
        let mut ledger = Ledger::default();
        ledger.record(change(10, 1, "bg"));
        let front = plan(&snapshot, &policy, &ledger);
        assert!(front.apply.is_empty());
        assert_eq!(pids(&front.clear), [10]);
        ledger.forget(10, 1);
        snapshot.frontmost_workload = Some("editor".into());
        assert_eq!(pids(&plan(&snapshot, &policy, &ledger).apply), [10, 11]);
    }

    #[test]
    fn protected_apps_are_never_targeted_and_leave_background_mode() {
        let snapshot = Snapshot {
            processes: vec![process(10, 1, "app")],
            ..Default::default()
        };
        let policy = policy(&[("app", Bucket::Protected)]);
        let mut ledger = Ledger::default();
        assert!(plan(&snapshot, &policy, &ledger).is_empty());
        // Something Jitterbug moved earlier, e.g. a one-off process action, is cleared.
        ledger.record(change(10, 1, "app"));
        let plan = plan(&snapshot, &policy, &ledger);
        assert!(plan.apply.is_empty());
        assert_eq!(pids(&plan.clear), [10]);
    }

    #[test]
    fn a_reused_pid_is_another_process_and_is_never_touched() {
        let snapshot = Snapshot {
            processes: vec![process(10, 2, "normal")],
            frontmost_workload: Some("bg".into()),
            ..Default::default()
        };
        let policy = policy(&[("bg", Bucket::Background)]);
        let mut ledger = Ledger::default();
        // Recorded for the old process with this PID, which exited.
        ledger.record(change(10, 1, "bg"));
        ledger.refuse(10, 1);
        ledger.retain_live(&snapshot);
        assert!(ledger.entries().is_empty());
        assert!(plan(&snapshot, &policy, &ledger).is_empty());
    }

    #[test]
    fn records_once_per_process_and_counts_per_app() {
        let mut ledger = Ledger::default();
        ledger.record(change(10, 1, "bg"));
        ledger.record(Change {
            case: Some(7),
            at: 9,
            ..change(10, 1, "bg")
        });
        ledger.record(change(10, 2, "bg"));
        assert_eq!(ledger.count("bg"), 2);
        assert_eq!(ledger.entries()[0].at, 1);
        assert_eq!(ledger.entries()[0].case, Some(7));
        for pid in 0..(LIMIT as u32 + 5) {
            ledger.record(change(pid + 100, 1, "many"));
        }
        assert_eq!(ledger.entries().len(), LIMIT);
    }
}
