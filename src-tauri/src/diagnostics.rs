//! Resolve a user's app/PID locally, then assemble a small, evidence-based investigation.
use crate::types::{ProcessRecord, ResourcePoint, Snapshot};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Target {
    pub workload_id: String,
    pub label: String,
    pub pid: Option<u32>,
}

pub enum Resolution {
    Found(Target),
    Clarify(String, Vec<Target>),
    System,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Investigation {
    #[serde(default)]
    pub stack_checks: Vec<crate::probe::StackCheck>,
    pub target: Target,
    pub captured_at: u64,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub process_count: usize,
    pub history: Vec<ResourcePoint>,
    pub processes: Vec<ProcessRecord>,
    pub limits: String,
    #[serde(default)]
    pub responsiveness: String,
    #[serde(default)]
    pub coverage: String,
    #[serde(default)]
    pub recently_unresponsive: bool,
}

fn words(text: &str) -> String {
    format!(
        " {} ",
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    )
}

fn mentions(question: &str, name: &str) -> bool {
    let n = words(name);
    n.trim().len() >= 3 && words(question).contains(&n)
}

fn explicit_pid(question: &str) -> Option<u32> {
    let lower = question.to_lowercase();
    let parts: Vec<_> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    parts.windows(2).find_map(|pair| {
        matches!(pair[0], "pid" | "process")
            .then(|| pair[1].parse().ok())
            .flatten()
    })
}

fn matches(snapshot: &Snapshot, question: &str) -> Option<Resolution> {
    if let Some(pid) = explicit_pid(question) {
        return Some(match snapshot.processes.iter().find(|p| p.pid == pid) {
            Some(p) => Resolution::Found(Target { workload_id: p.workload_id.clone(), label: p.name.clone(), pid: Some(pid) }),
            None => Resolution::Clarify(format!("PID {pid} is not in the current inventory. It may have exited or be outside the inventory limit. Choose a running workload or enter a current PID."), vec![]),
        });
    }
    let mut found: Vec<Target> = snapshot
        .workloads
        .iter()
        .filter(|w| mentions(question, &w.label))
        .map(|w| Target {
            workload_id: w.id.clone(),
            label: w.label.clone(),
            pid: None,
        })
        .collect();
    // Prefer an exact app/project name; runtime names are intentionally ambiguous across projects.
    if found.is_empty() {
        for p in &snapshot.processes {
            let runtime = ["python", "node", "rustc", "cargo"]
                .iter()
                .any(|r| p.name.to_lowercase().starts_with(r) && mentions(question, r));
            if (mentions(question, &p.name) || runtime)
                && !found.iter().any(|t| t.workload_id == p.workload_id)
            {
                if let Some(w) = snapshot.workloads.iter().find(|w| w.id == p.workload_id) {
                    found.push(Target {
                        workload_id: w.id.clone(),
                        label: w.label.clone(),
                        pid: None,
                    });
                }
            }
        }
    }
    match found.len() {
        0 => None,
        1 => Some(Resolution::Found(found.remove(0))),
        _ => Some(Resolution::Clarify("More than one workload matches. Choose the app or project to investigate, or enter a PID for a single process.".into(), found.into_iter().take(8).collect())),
    }
}

pub fn resolve(snapshot: &Snapshot, questions: &[&str], selected: Option<&str>) -> Resolution {
    let question = questions.last().copied().unwrap_or_default();
    if let Some(result) = matches(snapshot, question) {
        // The selection is stronger evidence than a duplicate app name (for example
        // an installed Jitterbug and its development checkout). No model is needed.
        if let Resolution::Clarify(_, candidates) = &result {
            let same_name = candidates.first().is_some_and(|first| {
                candidates
                    .iter()
                    .all(|target| words(&target.label) == words(&first.label))
            });
            if same_name {
                if let Some(target) = candidates
                    .iter()
                    .find(|t| Some(t.workload_id.as_str()) == selected)
                {
                    return Resolution::Found(target.clone());
                }
            }
        }
        return result;
    }
    let q = question.to_lowercase();
    if [
        "my computer",
        "whole computer",
        "system-wide",
        "overall",
        "what does cpu",
        "what is ram",
        "what is swap",
    ]
    .iter()
    .any(|term| q.contains(term))
    {
        return Resolution::System;
    }
    // Do not substitute an old target when a new named app/PID cannot be resolved.
    let unresolved_name = [
        "why is ",
        "diagnose ",
        "investigate ",
        "look at ",
        "what about ",
        "how about ",
    ]
    .iter()
    .any(|prefix| {
        q.split_once(prefix).is_some_and(|(_, rest)| {
            ![
                "it",
                "this",
                "that",
                "cpu",
                "memory",
                "ram",
                "swap",
                "my computer",
                "the computer",
                "the system",
            ]
            .iter()
            .any(|word| words(rest).starts_with(&words(word)))
        })
    });
    if unresolved_name {
        return Resolution::Clarify("I couldn't match that name to a running workload. Choose one below or enter its PID; I won't guess which process you mean.".into(), choices(snapshot));
    }
    if selected.is_some_and(|id| !snapshot.workloads.iter().any(|w| w.id == id)) {
        return Resolution::Clarify("The selected workload is no longer in the current inventory. It may have exited. Choose another running workload or enter a current PID.".into(), choices(snapshot));
    }
    if let Some(w) = selected.and_then(|id| snapshot.workloads.iter().find(|w| w.id == id)) {
        // Preserve a PID investigation for follow-ups, but respect an explicit workload choice.
        if !q.contains("selected workload") {
            if let Some(previous) = questions
                .iter()
                .rev()
                .skip(1)
                .find_map(|q| matches(snapshot, q))
            {
                match previous {
                    Resolution::Found(t) if t.workload_id == w.id => return Resolution::Found(t),
                    Resolution::Clarify(..) => return previous,
                    _ => (),
                }
            }
        }
        return Resolution::Found(Target {
            workload_id: w.id.clone(),
            label: w.label.clone(),
            pid: None,
        });
    }
    for previous in questions.iter().rev().skip(1) {
        if let Some(result) = matches(snapshot, previous) {
            return result;
        }
    }
    Resolution::System
}

pub fn resolve_with_process(
    snapshot: &Snapshot,
    questions: &[&str],
    selected: Option<&str>,
    process: Option<(u32, u64)>,
) -> Resolution {
    let resolution = resolve(snapshot, questions, selected);
    if let (Resolution::Found(target), Some((pid, start))) = (&resolution, process) {
        if target.pid.is_none() && Some(target.workload_id.as_str()) == selected {
            return match snapshot.processes.iter().find(|p| {
                p.pid == pid && p.start_time == start && p.workload_id == target.workload_id
            }) {
                Some(process) => Resolution::Found(Target {
                    workload_id: target.workload_id.clone(),
                    label: process.name.clone(),
                    pid: Some(pid),
                }),
                None => Resolution::Clarify(
                    "The selected process has exited. Choose a current process.".into(),
                    vec![],
                ),
            };
        }
    }
    resolution
}

pub fn choices(snapshot: &Snapshot) -> Vec<Target> {
    snapshot
        .workloads
        .iter()
        .take(6)
        .map(|w| Target {
            workload_id: w.id.clone(),
            label: w.label.clone(),
            pid: None,
        })
        .collect()
}

pub fn investigate(snapshot: &Snapshot, target: Target) -> Investigation {
    let workload = snapshot
        .workloads
        .iter()
        .find(|w| w.id == target.workload_id);
    let mut processes: Vec<_> = snapshot
        .processes
        .iter()
        .filter(|p| {
            p.workload_id == target.workload_id && target.pid.is_none_or(|pid| p.pid == pid)
        })
        .cloned()
        .collect();
    processes.sort_by(|a, b| {
        b.cpu_percent
            .total_cmp(&a.cpu_percent)
            .then(b.memory_bytes.cmp(&a.memory_bytes))
    });
    let cpu_percent = if target.pid.is_some() {
        processes.iter().map(|p| p.cpu_percent).sum()
    } else {
        workload.map(|w| w.cpu_percent).unwrap_or_default()
    };
    let memory_bytes = if target.pid.is_some() {
        processes.iter().map(|p| p.memory_bytes).sum()
    } else {
        workload.map(|w| w.memory_bytes).unwrap_or_default()
    };
    let process_count = if target.pid.is_some() {
        processes.len()
    } else {
        workload.map(|w| w.process_count).unwrap_or_default()
    };
    let responsiveness = match if target.pid.is_some() {
        processes.first().and_then(|p| p.unresponsive)
    } else {
        workload.and_then(|w| w.unresponsive)
    } {
        Some(true) => "not_responding",
        Some(false) => "responding",
        None => "unknown",
    }
    .into();
    let helpers = processes
        .iter()
        .filter(|p| p.name.starts_with("com.apple.WebKit."))
        .count();
    let coverage = if workload.is_some_and(|w| w.category == "browser") {
        format!("{helpers} WebKit helpers included through observed ownership. Tab contents, extensions, and page/network responsiveness are not measured. Low CPU cannot rule out a stuck page.")
    } else {
        "Includes observed workload members. Native window responsiveness does not establish whether every task inside the app is progressing.".into()
    };
    let recently_unresponsive =
        workload.is_some_and(|w| w.history.iter().any(|p| p.unresponsive == Some(true)));
    processes.truncate(8);
    Investigation {
        stack_checks: vec![],
        responsiveness, coverage, recently_unresponsive,
        target, captured_at: snapshot.captured_at, cpu_percent, memory_bytes, process_count,
        history: workload.map(|w| w.history.clone()).unwrap_or_default(), processes,
        limits: "CPU is % of total machine capacity. History covers the whole workload, including for a PID question. Memory is summed RSS and can count shared pages twice. Disk bytes cover the last collection interval. App settings, GPU usage, network traffic, and agent transcripts were not inspected. Any short stack checks are reported separately. Helper roles inferred from names are hypotheses; resource use alone cannot prove a root cause.".into(),
    }
}

impl Investigation {
    /// Selected app/process names are useful evidence; paths, project labels, identifiers and unrelated apps stay local.
    pub fn model_context(&self, snapshot: &Snapshot) -> Value {
        let project = snapshot
            .workloads
            .iter()
            .find(|w| w.id == self.target.workload_id)
            .is_some_and(|w| w.category == "project");
        let history: Vec<_> = self.history.iter().map(|p| json!({"seconds_ago":self.captured_at.saturating_sub(p.time),"cpu_percent":p.cpu_percent,"resident_mib":p.memory_bytes/1048576})).collect();
        let processes: Vec<_> = self.processes.iter().map(|p| json!({"name":p.name,"cpu_percent":p.cpu_percent,"cpu_percent_one_core_scale":p.cpu_percent*snapshot.logical_cpus as f32,"resident_mib":p.memory_bytes/1048576,"disk_read_bytes":p.disk_read_bytes,"disk_written_bytes":p.disk_written_bytes,"available_actions":p.actions})).collect();
        let members: Vec<_> = snapshot
            .processes
            .iter()
            .filter(|p| {
                p.workload_id == self.target.workload_id
                    && self.target.pid.is_none_or(|pid| p.pid == pid)
            })
            .collect();
        let identity = (!members.is_empty())
            .then(|| crate::details::representative(&members))
            .and_then(|process| {
                process.executable.as_deref().map(|path| {
                    crate::profiles::model_payload(&crate::applications::identify(path))
                })
            });
        let launch = (!members.is_empty())
            .then(|| crate::details::representative(&members))
            .and_then(|p| crate::origins::inspect(snapshot, p))
            .map(|origin| origin.model_context());
        json!({"launch_origin":launch,"stack_checks":self.stack_checks,"executable_identity":identity,"target":if project {"Selected development workload"} else {&self.target.label},"scope":if self.target.pid.is_some(){"single PID; history is its containing workload"}else{"application/workload"},"cpu_percent":self.cpu_percent,"resident_mib":self.memory_bytes/1048576,"process_count":self.process_count,"history":history,"busiest_processes":processes,"limits":self.limits,"window_responsiveness":self.responsiveness,"recently_unresponsive":self.recently_unresponsive,"coverage":self.coverage})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Workload;
    fn fixture() -> Snapshot {
        Snapshot {
            workloads: vec![
                Workload {
                    id: "music".into(),
                    label: "Spotify".into(),
                    ..Default::default()
                },
                Workload {
                    id: "one".into(),
                    label: "Project One".into(),
                    category: "project".into(),
                    ..Default::default()
                },
                Workload {
                    id: "two".into(),
                    label: "Project Two".into(),
                    category: "project".into(),
                    ..Default::default()
                },
            ],
            processes: vec![
                ProcessRecord {
                    pid: 123,
                    start_time: 10,
                    name: "Spotify Helper (Renderer)".into(),
                    workload_id: "music".into(),
                    ..Default::default()
                },
                ProcessRecord {
                    pid: 124,
                    name: "python3".into(),
                    workload_id: "one".into(),
                    ..Default::default()
                },
                ProcessRecord {
                    pid: 125,
                    name: "python3".into(),
                    workload_id: "two".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }
    fn target(resolution: Resolution) -> Target {
        match resolution {
            Resolution::Found(t) => t,
            _ => panic!("Expected resolved target"),
        }
    }
    #[test]
    fn name_pid_followups_and_target_changes() {
        let s = fixture();
        assert_eq!(
            target(resolve(&s, &["Why is SPOTIFY using so much CPU?"], None)).workload_id,
            "music"
        );
        assert_eq!(
            target(resolve(
                &s,
                &["Look at Spotify", "What can I do about it?"],
                None
            ))
            .workload_id,
            "music"
        );
        assert_eq!(
            target(resolve(&s, &["Investigate PID 123"], Some("one"))).pid,
            Some(123)
        );
        assert_eq!(
            target(resolve(&s, &["Explain Spotify"], Some("one"))).workload_id,
            "music"
        );
        assert!(matches!(
            resolve(&s, &["Why is UnknownApp using so much CPU?"], Some("music")),
            Resolution::Clarify(..)
        ));
        assert!(matches!(
            resolve(&s, &["Investigate PID 999"], Some("music")),
            Resolution::Clarify(..)
        ));
        assert!(
            matches!(resolve(&s,&["Why is python using so much CPU?"],None),Resolution::Clarify(_,c) if c.len()==2)
        );
        assert!(matches!(
            resolve(&s, &["Spotify", "Why is my computer struggling?"], None),
            Resolution::System
        ));
        assert!(matches!(
            resolve(&s, &["Explain Spotifying"], None),
            Resolution::System
        ));
        assert_eq!(
            target(resolve(
                &s,
                &["Investigate PID 123", "What can we improve?"],
                Some("music")
            ))
            .pid,
            Some(123)
        );
        assert!(matches!(
            resolve(&s, &["Investigate UnknownApp"], Some("music")),
            Resolution::Clarify(..)
        ));
        assert_eq!(
            target(resolve(
                &s,
                &["Investigate this selected workload"],
                Some("music")
            ))
            .workload_id,
            "music"
        );
    }
    #[test]
    fn process_selection_is_preserved_for_plain_language_followups() {
        let s = fixture();
        assert_eq!(
            target(resolve_with_process(
                &s,
                &["Does this need attention?"],
                Some("music"),
                Some((123, 10))
            ))
            .pid,
            Some(123)
        );
        assert!(matches!(
            resolve_with_process(&s, &["What is it doing?"], Some("music"), Some((123, 11))),
            Resolution::Clarify(..)
        ));
        assert_eq!(
            target(resolve_with_process(
                &s,
                &["Explain Project Two"],
                Some("music"),
                Some((123, 10))
            ))
            .workload_id,
            "two"
        );
        assert_eq!(
            target(resolve_with_process(
                &s,
                &["Investigate PID 125"],
                Some("music"),
                Some((123, 10))
            ))
            .pid,
            Some(125)
        );
    }

    #[test]
    fn selected_workload_resolves_duplicate_names_without_a_model() {
        let mut s = fixture();
        s.workloads[1].label = "Spotify".into();
        assert_eq!(
            target(resolve(&s, &["Why is Spotify busy?"], Some("music"))).workload_id,
            "music"
        );
        assert_eq!(
            target(resolve(&s, &["Why is Spotify busy?"], Some("one"))).workload_id,
            "one"
        );
        assert!(matches!(
            resolve(&s, &["Why is Spotify busy?"], None),
            Resolution::Clarify(..)
        ));
        // An explicit PID still overrides a selection; genuinely different targets are not guessed.
        assert_eq!(
            target(resolve(&s, &["Investigate PID 125"], Some("music"))).workload_id,
            "two"
        );
        assert!(matches!(
            resolve(&s, &["Spotify and Project Two"], Some("music")),
            Resolution::Clarify(..)
        ));
    }

    #[test]
    fn exited_targets_are_not_silently_replaced_with_other_processes() {
        let s = fixture();
        assert!(matches!(
            resolve(&s, &["Investigate PID 999", "What next?"], Some("music")),
            Resolution::Clarify(..)
        ));
        assert!(matches!(
            resolve(&s, &["What next?"], Some("exited")),
            Resolution::Clarify(..)
        ));
        assert_eq!(
            target(resolve(&s, &["Spotify"], Some("exited"))).workload_id,
            "music"
        );
    }

    #[test]
    fn context_contains_selected_evidence_without_other_apps_or_identity() {
        let s = fixture();
        let investigation = investigate(&s, target(resolve(&s, &["Spotify"], None)));
        let context = investigation.model_context(&s).to_string();
        assert!(context.contains("Spotify Helper"));
        for private in [
            "Project One",
            "Project Two",
            "python3",
            "start_time",
            "workload_id",
        ] {
            assert!(!context.contains(private));
        }
    }
    #[test]
    fn resolution_uses_inventory_beyond_dashboard() {
        let mut s = fixture();
        s.workloads.splice(
            0..0,
            (0..45).map(|n| Workload {
                id: format!("w{n}"),
                label: format!("App {n}"),
                ..Default::default()
            }),
        );
        assert!(!s.dashboard_view().workloads.iter().any(|w| w.id == "music"));
        assert_eq!(target(resolve(&s, &["Spotify"], None)).workload_id, "music");
    }
}
