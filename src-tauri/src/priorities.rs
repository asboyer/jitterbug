//! Priority changes Jitterbug made, kept in memory like incident history, so the person can see
//! what is lowered and undo it. A lowered process returns to normal by itself when it exits.
use serde::Serialize;
use std::collections::VecDeque;

/// More lowered processes than this at once is unlikely; the oldest entry is dropped first.
const LIMIT: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The person approved it, in the popup, an incident, or a process view.
    Approval,
    /// A saved "always lower" permission applied it without asking.
    Rule,
    /// It answered an agent's request.
    Agent,
}

#[derive(Clone, Debug)]
pub struct Change {
    pub id: u64,
    pub pid: u32,
    pub start_time: u64,
    pub workload_id: String,
    pub label: String,
    pub process: String,
    /// The OS nice value before Jitterbug changed it, and the value it set.
    pub previous: i32,
    pub lowered_to: i32,
    pub at: u64,
    pub source: Source,
    /// The incident that applied it, so its result can offer Undo.
    pub case: Option<u64>,
}

/// What the UI sees: names and values only, no process identity or paths.
#[derive(Clone, Debug, Serialize)]
pub struct View {
    pub id: u64,
    pub label: String,
    pub process: String,
    pub previous: i32,
    pub lowered_to: i32,
    pub at: u64,
    pub source: Source,
}

#[derive(Default)]
pub struct Ledger {
    entries: VecDeque<Change>,
    sequence: u64,
}

impl Ledger {
    /// Records a change and returns its id. A process lowered again keeps its first original
    /// value, so restoring always returns to where it was before Jitterbug touched it.
    pub fn record(&mut self, mut change: Change) -> u64 {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|c| c.pid == change.pid && c.start_time == change.start_time)
        {
            existing.lowered_to = change.lowered_to;
            existing.case = change.case.or(existing.case);
            return existing.id;
        }
        self.sequence += 1;
        change.id = self.sequence;
        self.entries.push_front(change);
        self.entries.truncate(LIMIT);
        self.sequence
    }

    pub fn entries(&self) -> Vec<Change> {
        self.entries.iter().cloned().collect()
    }

    pub fn get(&self, id: u64) -> Option<Change> {
        self.entries.iter().find(|c| c.id == id).cloned()
    }

    pub fn remove(&mut self, id: u64) -> Option<Change> {
        let position = self.entries.iter().position(|c| c.id == id)?;
        self.entries.remove(position)
    }

    /// Keeps only the listed ids: the caller checked the others exited or are back to normal.
    pub fn keep(&mut self, ids: &[u64]) {
        self.entries.retain(|c| ids.contains(&c.id));
    }

    pub fn for_case(&self, case: u64) -> Option<u64> {
        self.entries
            .iter()
            .find(|c| c.case == Some(case))
            .map(|c| c.id)
    }

    pub fn views(&self) -> Vec<View> {
        self.entries
            .iter()
            .map(|c| View {
                id: c.id,
                label: c.label.clone(),
                process: c.process.clone(),
                previous: c.previous,
                lowered_to: c.lowered_to,
                at: c.at,
                source: c.source,
            })
            .collect()
    }
}

/// Whether a recorded change still holds: the same process is alive and still above its
/// original value. Anything else is gone or already back to normal, and leaves the list.
pub fn still_lowered(change: &Change) -> bool {
    crate::actions::priority(change.pid, change.start_time).is_ok_and(|n| n > change.previous)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(pid: u32, start_time: u64) -> Change {
        Change {
            id: 0,
            pid,
            start_time,
            workload_id: "w-1".into(),
            label: "Example".into(),
            process: "example".into(),
            previous: 0,
            lowered_to: 10,
            at: 100,
            source: Source::Approval,
            case: None,
        }
    }

    #[test]
    fn records_once_per_process_and_keeps_the_original_value() {
        let mut ledger = Ledger::default();
        let first = ledger.record(change(10, 1));
        let again = ledger.record(Change {
            previous: 10,
            lowered_to: 12,
            case: Some(7),
            ..change(10, 1)
        });
        assert_eq!(first, again);
        let entry = ledger.get(first).unwrap();
        assert_eq!((entry.previous, entry.lowered_to), (0, 12));
        assert_eq!(ledger.for_case(7), Some(first));
        // A reused PID is a different process.
        let other = ledger.record(change(10, 2));
        assert_ne!(other, first);
        assert_eq!(ledger.views().len(), 2);
    }

    #[test]
    fn is_bounded_and_views_carry_no_process_identity() {
        let mut ledger = Ledger::default();
        for pid in 0..(LIMIT as u32 + 5) {
            ledger.record(change(pid + 100, 1));
        }
        assert_eq!(ledger.entries().len(), LIMIT);
        // The newest entries stay.
        assert!(ledger.entries().iter().any(|c| c.pid == LIMIT as u32 + 104));
        assert!(!ledger.entries().iter().any(|c| c.pid == 100));
        let text = serde_json::to_string(&ledger.views()).unwrap();
        for private in ["pid", "start_time", "workload_id"] {
            assert!(!text.contains(private), "{private} leaked");
        }
        ledger.keep(&[1]);
        assert!(ledger.entries().is_empty());
    }

    #[test]
    fn an_exited_process_leaves_the_list() {
        let mut child = std::process::Command::new("/bin/cat")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!still_lowered(&change(pid, 1)));
    }
}
