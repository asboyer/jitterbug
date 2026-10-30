//! On-demand local inspection of the full collector inventory, never the truncated dashboard.
use crate::{
    applications::{self, Application},
    profiles::Profile,
    types::*,
};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize)]
pub struct OriginNode {
    pub pid: u32,
    pub start_time: u64,
    pub name: String,
    pub executable: Option<String>,
    pub working_directory: Option<String>,
}

impl From<&ProcessRecord> for OriginNode {
    fn from(p: &ProcessRecord) -> Self {
        Self {
            pid: p.pid,
            start_time: p.start_time,
            name: p.name.clone(),
            executable: p
                .executable
                .as_ref()
                .map(|s| s.to_string_lossy().into_owned()),
            working_directory: p
                .working_directory
                .as_ref()
                .map(|s| s.to_string_lossy().into_owned()),
        }
    }
}

#[derive(Serialize)]
pub struct Details {
    pub captured_at: u64,
    pub workload: Workload,
    pub process: ProcessRecord,
    pub members: Vec<ProcessRecord>,
    pub member_count: usize,
    pub independent_roots: usize,
    pub application: Option<Application>,
    pub profile: Profile,
    pub launch: Option<crate::origins::Launch>,
    pub ancestry: Vec<OriginNode>,
    pub origin_summary: String,
    pub origin_limit: String,
    pub can_generate_profile: bool,
    pub profile_key: Option<String>,
}

/// Search the collector inventory, not just the busiest dashboard groups.
pub fn search(snapshot: &Snapshot, query: &str) -> Result<Vec<Workload>, String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() || query.len() > 128 {
        return Err("Enter a process name or PID, up to 128 bytes.".into());
    }
    let process_groups: HashSet<_> = snapshot
        .processes
        .iter()
        .filter(|p| p.name.to_lowercase().contains(&query) || p.pid.to_string() == query)
        .map(|p| p.workload_id.as_str())
        .collect();
    Ok(snapshot
        .workloads
        .iter()
        .filter(|w| {
            w.label.to_lowercase().contains(&query) || process_groups.contains(w.id.as_str())
        })
        .take(100)
        .cloned()
        .collect())
}

pub fn members<'a>(snapshot: &'a Snapshot, id: &str) -> Result<Vec<&'a ProcessRecord>, String> {
    if snapshot.source != "live" || snapshot.captured_at + 20 < now() {
        return Err("Wait for a fresh telemetry sample before inspecting a workload.".into());
    }
    let members: Vec<_> = snapshot
        .processes
        .iter()
        .filter(|p| p.workload_id == id)
        .collect();
    if members.is_empty() {
        return Err("This workload is no longer in the current process inventory.".into());
    }
    Ok(members)
}

pub fn representative<'a>(members: &[&'a ProcessRecord]) -> &'a ProcessRecord {
    // Prefer the top-level app over a hot helper; for projects, prefer the oldest observed root.
    members
        .iter()
        .copied()
        .min_by_key(|p| {
            let child = p.parent_pid.is_some_and(|id| {
                members
                    .iter()
                    .any(|parent| parent.pid == id && parent.start_time <= p.start_time)
            });
            (child, p.start_time, p.pid)
        })
        .expect("members are non-empty")
}

pub fn ancestry(snapshot: &Snapshot, selected: &ProcessRecord) -> (Vec<OriginNode>, String) {
    let mut chain = vec![OriginNode::from(selected)];
    let mut seen = HashSet::from([selected.pid]);
    let mut process = selected;
    let limit = loop {
        let Some(pid) = process.parent_pid.filter(|pid| *pid != 0) else {
            break "The OS exposes no earlier parent for this process.";
        };
        if seen.contains(&pid) {
            break "The parent chain contains a cycle; tracing stopped.";
        }
        let Some(parent) = snapshot.processes.iter().find(|p| p.pid == pid) else {
            break "An earlier parent has exited, is inaccessible, or is outside the bounded inventory.";
        };
        if parent.start_time > process.start_time
            || process
                .parent_start_time
                .is_some_and(|start| start != parent.start_time)
        {
            break "A parent PID was reused or its start time is inconsistent; tracing stopped.";
        }
        if chain.len() >= 16 {
            break "The trace reached its 16-process depth limit.";
        }
        chain.push(OriginNode::from(parent));
        seen.insert(pid);
        process = parent;
    };
    chain.reverse();
    (chain, format!("{limit} This is the currently observed parent chain, not a historical launch log. Parents can exit and children can be reparented; Dock clicks, login items, and exact agent tasks are not established by this trace."))
}

pub fn inspect(
    snapshot: &Snapshot,
    id: &str,
    selected: Option<(u32, u64)>,
) -> Result<Details, String> {
    if !snapshot.sample_ready {
        return Err("Wait for a second telemetry sample before inspecting resource use.".into());
    }
    let members = members(snapshot, id)?;
    let process = match selected {
        Some((pid, start)) => *members
            .iter()
            .find(|p| p.pid == pid && p.start_time == start)
            .ok_or(
                "The selected process has exited or its PID was reused. Choose a current process.",
            )?,
        None => representative(&members),
    };
    let workload = snapshot
        .workloads
        .iter()
        .find(|w| w.id == id)
        .cloned()
        .ok_or("Workload metadata is no longer available.")?;
    let (ancestry, origin_limit) = ancestry(snapshot, process);
    let origin_summary = if let Some(parent) = ancestry.iter().rev().nth(1) {
        if parent.pid <= 1
            || matches!(
                parent.name.to_lowercase().as_str(),
                "launchd" | "services.exe" | "system"
            )
        {
            format!("{} is currently attached to the system process manager ({}). That does not reveal who originally opened it.", process.name, parent.name)
        } else {
            format!("{} is currently a child of {} (PID {}). The chain below shows its observed ancestors.", process.name, parent.name, parent.pid)
        }
    } else {
        format!(
            "No parent could be verified for {} in this sample.",
            process.name
        )
    };
    let application = process
        .executable
        .as_ref()
        .map(|path| applications::identify(path));
    let profile = crate::profiles::reference(application.as_ref(), &workload);
    let launch = crate::origins::inspect(snapshot, process);
    // A system daemon or command does not need an .app bundle to be explainable.
    // Explanations describe this executable, never infer a script or project task.
    let can_generate_profile = application
        .as_ref()
        .is_some_and(|app| !app.executable_name.is_empty())
        && profile.source != "reference"
        && launch.is_none();
    let profile_key = application.as_ref().map(crate::profiles::key);
    let independent_roots = members
        .iter()
        .filter(|p| {
            p.parent_pid.is_none_or(|id| {
                !members
                    .iter()
                    .any(|parent| parent.pid == id && parent.start_time <= p.start_time)
            })
        })
        .count();
    let member_count = members.len();
    // Ensure a selected process is visible even if it falls outside the first 200 by impact.
    let mut displayed: Vec<_> = members.iter().take(200).map(|p| (*p).clone()).collect();
    if !displayed.iter().any(|p| p.pid == process.pid) {
        displayed.pop();
        displayed.push(process.clone());
    }
    Ok(Details {
        captured_at: snapshot.captured_at,
        workload,
        process: process.clone(),
        members: displayed,
        member_count,
        independent_roots,
        application,
        profile,
        launch,
        ancestry,
        origin_summary,
        origin_limit,
        can_generate_profile,
        profile_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Snapshot {
        let process = |pid, parent, start| ProcessRecord {
            pid,
            parent_pid: parent,
            start_time: start,
            name: format!("process-{pid}"),
            workload_id: "example".into(),
            ..Default::default()
        };
        Snapshot {
            source: "live".into(),
            sample_ready: true,
            captured_at: now(),
            processes: vec![
                process(30, Some(20), 300),
                process(20, Some(10), 200),
                process(10, None, 100),
            ],
            workloads: vec![Workload {
                id: "example".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    #[test]
    fn search_finds_quiet_processes_beyond_the_dashboard() {
        let mut snapshot = Snapshot {
            workloads: (0..80)
                .map(|i| Workload {
                    id: format!("workload-{i}"),
                    label: format!("Example App {i}"),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        snapshot.processes.push(ProcessRecord {
            pid: 9876,
            name: "quiet-helper".into(),
            workload_id: "workload-79".into(),
            ..Default::default()
        });
        assert!(!snapshot
            .dashboard_view()
            .workloads
            .iter()
            .any(|w| w.id == "workload-79"));
        assert_eq!(
            search(&snapshot, "quiet-helper").unwrap()[0].id,
            "workload-79"
        );
        assert_eq!(search(&snapshot, "9876").unwrap()[0].id, "workload-79");
        assert!(search(&snapshot, " ").is_err());
        assert!(search(&snapshot, &"a".repeat(129)).is_err());
    }

    #[test]
    fn traces_oldest_to_selected_and_selects_root() {
        let s = fixture();
        let d = inspect(&s, "example", Some((30, 300))).unwrap();
        assert_eq!(
            d.ancestry.iter().map(|p| p.pid).collect::<Vec<_>>(),
            [10, 20, 30]
        );
        assert_eq!(inspect(&s, "example", None).unwrap().process.pid, 10);
        assert!(inspect(&s, "example", Some((30, 999))).is_err());
    }
    #[test]
    fn icons_can_resolve_during_the_first_cpu_sample() {
        let mut s = fixture();
        s.sample_ready = false;
        assert!(members(&s, "example").is_ok());
        assert!(inspect(&s, "example", None).is_err());
    }
    #[test]
    fn stops_at_missing_reused_and_cyclic_parents() {
        let mut s = fixture();
        s.processes[1].start_time = 400;
        assert!(ancestry(&s, &s.processes[0]).1.contains("reused"));
        s.processes[1].start_time = 200;
        s.processes[0].parent_start_time = Some(199);
        assert_eq!(ancestry(&s, &s.processes[0]).0.len(), 1);
        s.processes[0].parent_start_time = None;
        s.processes[2].parent_pid = Some(30);
        assert!(ancestry(&s, &s.processes[0]).1.contains("cycle"));
        s.processes.pop();
        assert!(ancestry(&s, &s.processes[0]).1.contains("exited"));
    }
    #[test]
    fn paths_are_local_detail_only() {
        let mut s = fixture();
        s.processes[0].executable = Some("/private/example/program".into());
        s.processes[0].working_directory = Some("/private/example/project".into());
        assert!(!serde_json::to_string(&s)
            .unwrap()
            .contains("/private/example"));
        let d = inspect(&s, "example", Some((30, 300))).unwrap();
        assert!(serde_json::to_string(&d)
            .unwrap()
            .contains("/private/example/program"));
        assert!(!crate::providers::model_context(&s, Some("example"))
            .to_string()
            .contains("/private/example"));
    }
    #[test]
    fn standalone_daemons_and_project_runtimes_can_be_explained() {
        let mut s = fixture();
        s.processes[0].executable = Some("/example/bin/service-worker".into());
        let d = inspect(&s, "example", Some((30, 300))).unwrap();
        assert!(d.can_generate_profile);
        assert!(d.profile_key.is_some());
        s.workloads[0].category = "project".into();
        assert!(
            inspect(&s, "example", Some((30, 300)))
                .unwrap()
                .can_generate_profile
        );
        s.processes[0].executable = None;
        let d = inspect(&s, "example", Some((30, 300))).unwrap();
        assert!(!d.can_generate_profile);
        assert!(d.profile_key.is_none());
    }

    #[test]
    fn details_use_full_inventory_and_bound_the_response() {
        let mut s = fixture();
        for pid in 100..500 {
            s.processes.push(ProcessRecord {
                pid,
                workload_id: "example".into(),
                start_time: 400,
                ..Default::default()
            });
        }
        let d = inspect(&s, "example", Some((499, 400))).unwrap();
        assert_eq!(d.member_count, 403);
        assert_eq!(d.members.len(), 200);
        assert!(d.members.iter().any(|p| p.pid == 499));
        s.captured_at = now() - 21;
        assert!(inspect(&s, "example", None).is_err());
    }
}
