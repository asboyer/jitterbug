//! The most specific action for a workload under pressure, offered in place of quitting the
//! whole app: close its heaviest browser tab, or pause the heaviest command session running
//! inside it. `None` means callers fall back to the existing stop option.
use super::{chrome, lens, sessions, Lens, Unit};
use crate::{
    automation::{Impact, Proposal},
    types::*,
};

/// A session is worth pausing when it carries this share of its app's use...
const SHARE: f32 = 0.3;
/// ...or a real load on its own, so one of several similar sessions still qualifies.
const SESSION_MEMORY: u64 = 1 << 30;
const SESSION_CORES: f32 = 50.0;
/// A tab is offered only above this much memory, or a quarter of one core.
const TAB_MEMORY: u64 = 200 << 20;

fn weight(unit: &Unit, memory: bool) -> f64 {
    if memory {
        unit.memory_bytes as f64
    } else {
        unit.cpu_percent as f64
    }
}

/// Blocking: a connected browser is measured for about one second.
pub fn option(snapshot: &Snapshot, workload_id: &str, memory: bool) -> Option<Proposal> {
    // A Jitter Snapshot culprit can be one command inside an app: `cmd-<pid>-<start>`.
    if let Some(rest) = workload_id.strip_prefix("cmd-") {
        let (pid, start) = rest.split_once('-')?;
        let (pid, start): (u32, u64) = (pid.parse().ok()?, start.parse().ok()?);
        return sessions::session_of(snapshot, pid)
            .filter(|u| u.start_time == Some(start) && u.actions.iter().any(|a| a == "pause"))
            .and_then(|u| pause(snapshot, u));
    }
    let members = crate::details::members(snapshot, workload_id).ok()?;
    let workload = snapshot.workloads.iter().find(|w| w.id == workload_id)?;
    let total = if memory {
        workload.memory_bytes as f64
    } else {
        workload.cpu_percent as f64
    };
    match lens(&members) {
        Lens::Browser => tab(snapshot, workload, &members, memory),
        Lens::Sessions => sessions::inspect(snapshot, workload_id, &members)
            .units
            .into_iter()
            .filter(|u| u.actions.iter().any(|a| a == "pause"))
            .max_by(|a, b| weight(a, memory).total_cmp(&weight(b, memory)))
            .filter(|u| {
                let large = if memory {
                    u.memory_bytes >= SESSION_MEMORY
                } else {
                    u.cpu_percent * snapshot.logical_cpus.max(1) as f32 >= SESSION_CORES
                };
                large || (total > 0.0 && weight(u, memory) >= total * SHARE as f64)
            })
            .and_then(|u| pause(snapshot, u)),
        // A project or command workload: pause the agent or terminal session that owns its
        // heaviest process, when an app launched it.
        Lens::None => members
            .iter()
            .max_by(|a, b| {
                let w = |p: &ProcessRecord| {
                    if memory {
                        p.memory_bytes as f64
                    } else {
                        p.cpu_percent as f64
                    }
                };
                w(a).total_cmp(&w(b))
            })
            .and_then(|hog| sessions::session_of(snapshot, hog.pid))
            .filter(|u| u.actions.iter().any(|a| a == "pause"))
            .and_then(|u| pause(snapshot, u)),
    }
}

fn pause(snapshot: &Snapshot, unit: Unit) -> Option<Proposal> {
    let process = snapshot
        .processes
        .iter()
        .find(|p| Some(p.pid) == unit.pid && Some(p.start_time) == unit.start_time)?
        .clone();
    let count = unit.process_count.max(1);
    Some(Proposal {
        rationale: format!(
            "Pause {}: its {count} process{} stop using CPU until you resume it (automatically after {} minutes). Its memory stays allocated, but stops growing and macOS can compress or swap it for your other work. Nothing is closed.",
            unit.title,
            if count == 1 { "" } else { "es" },
            crate::pause::LIMIT_SECONDS / 60
        ),
        impact: Some(Impact {
            label: unit.title.clone(),
            workload_cpu_percent: unit.cpu_percent,
            workload_memory_bytes: unit.memory_bytes,
            process_count: count,
            whole_app: false,
        }),
        process,
        action: "pause".into(),
        unit: Some(unit),
    })
}

/// Only a browser connection the person already allowed is reused; a background incident
/// never opens one, so the browser never asks for permission unexpectedly.
fn tab(
    snapshot: &Snapshot,
    workload: &Workload,
    members: &[&ProcessRecord],
    memory: bool,
) -> Option<Proposal> {
    if !chrome::connected() {
        return None;
    }
    let view = chrome::inspect_connected(snapshot, &workload.id, members);
    let unit = view
        .units
        .into_iter()
        .filter(|u| u.kind == "tab")
        .max_by(|a, b| weight(a, memory).total_cmp(&weight(b, memory)))?;
    let large = if memory {
        unit.memory_bytes >= TAB_MEMORY
    } else {
        unit.cpu_percent * snapshot.logical_cpus.max(1) as f32 >= 25.0
    };
    if !large {
        return None;
    }
    // The renderer stands in for identity; the tab itself is rechecked by address on approval.
    let process = members
        .iter()
        .find(|p| Some(p.pid) == unit.pid)
        .or_else(|| {
            members.iter().find(|p| {
                p.parent_pid
                    .is_none_or(|parent| !members.iter().any(|m| m.pid == parent))
            })
        })
        .map(|p| (*p).clone())?;
    let host = unit.host.clone().unwrap_or_default();
    Some(Proposal {
        rationale: format!(
            "Close the “{}” tab{}. Anything running in it ends, such as a call, video, or unsaved form input. The rest of {} stays open.",
            unit.title,
            if host.is_empty() { String::new() } else { format!(" ({host})") },
            workload.label
        ),
        impact: Some(Impact {
            label: format!("{} tab", unit.title),
            workload_cpu_percent: unit.cpu_percent,
            workload_memory_bytes: unit.memory_bytes,
            process_count: 1,
            whole_app: false,
        }),
        process,
        action: "close_tab".into(),
        unit: Some(unit),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: Option<u32>, name: &str, exe: &str, memory_mib: u64) -> ProcessRecord {
        ProcessRecord {
            pid,
            start_time: pid as u64,
            parent_pid: parent,
            name: name.into(),
            executable: Some(exe.into()),
            memory_bytes: memory_mib << 20,
            cpu_percent: 1.0,
            workload_id: "code".into(),
            ..Default::default()
        }
    }

    fn editor(session_mib: u64) -> Snapshot {
        let app = "/A/Visual Studio Code.app/Contents";
        let mut agent = p(5, Some(4), "claude", "/u/.local/bin/claude", session_mib);
        agent.working_directory = Some("/w/repo".into());
        let mut tests = p(6, Some(5), "node", "/usr/local/bin/node", session_mib);
        // A project's node process is grouped by its folder, not under the editor.
        tests.workload_id = "repo".into();
        Snapshot {
            source: "live".into(),
            captured_at: now(),
            sample_ready: true,
            logical_cpus: 8,
            workloads: vec![
                Workload {
                    id: "code".into(),
                    label: "Visual Studio Code".into(),
                    memory_bytes: (400 + session_mib) << 20,
                    cpu_percent: 5.0,
                    ..Default::default()
                },
                Workload {
                    id: "repo".into(),
                    label: "repo".into(),
                    category: "project".into(),
                    memory_bytes: session_mib << 20,
                    cpu_percent: 5.0,
                    ..Default::default()
                },
            ],
            processes: vec![
                p(1, None, "Code", &format!("{app}/MacOS/Code"), 300),
                p(
                    2,
                    Some(1),
                    "Code Helper",
                    &format!("{app}/Frameworks/H.app/Contents/MacOS/h"),
                    100,
                ),
                p(4, Some(2), "zsh", "/bin/zsh", 1),
                agent,
                tests,
            ],
            ..Default::default()
        }
    }

    #[test]
    fn a_heavy_session_inside_the_editor_is_offered_as_a_pause_not_a_quit() {
        let s = editor(2048);
        let option = option(&s, "code", true).unwrap();
        assert_eq!(option.action, "pause");
        assert_eq!(option.process.pid, 5);
        let unit = option.unit.unwrap();
        assert_eq!(unit.title, "claude · repo");
        assert_eq!(unit.process_count, 2);
        assert!(option.rationale.contains("Nothing is closed"));
        assert!(!option.impact.unwrap().whole_app);
    }

    #[test]
    fn one_of_five_similar_agent_sessions_is_still_offered_for_pausing() {
        let mut s = editor(20);
        s.processes.truncate(3);
        // Five sessions of 1.1–1.5 GiB: none carries 30% of the editor, the largest is offered.
        for n in 0..5u32 {
            let shell = 100 + n * 10;
            s.processes.push(p(shell, Some(2), "zsh", "/bin/zsh", 1));
            let mut agent = p(
                shell + 1,
                Some(shell),
                "claude",
                "/u/.local/bin/claude",
                1100 + n as u64 * 100,
            );
            agent.working_directory = Some(format!("/w/task-{n}").into());
            s.processes.push(agent);
        }
        s.workloads[0].memory_bytes = 7 << 30;
        let option = option(&s, "code", true).unwrap();
        assert_eq!(option.action, "pause");
        assert_eq!(option.unit.unwrap().title, "claude · task-4");
    }

    #[test]
    fn a_small_session_is_not_worth_pausing_for_the_editor() {
        assert!(option(&editor(20), "code", true).is_none());
    }

    #[test]
    fn a_project_hog_pauses_the_agent_session_that_launched_it() {
        let s = editor(2048);
        let option = option(&s, "repo", true).unwrap();
        assert_eq!(option.action, "pause");
        assert_eq!(option.process.pid, 5);
        assert_eq!(
            option.unit.unwrap().title,
            "claude · repo in Visual Studio Code"
        );
        // A snapshot culprit names the same session by command ID.
        let by_command = super::option(&s, "cmd-5-5", false).unwrap();
        assert_eq!(by_command.process.pid, 5);
        assert!(super::option(&s, "cmd-5-99", false).is_none());
    }

    #[test]
    fn app_processes_and_unlaunched_work_have_no_session() {
        let s = editor(2048);
        assert!(sessions::session_of(&s, 2).is_none());
        let mut lone = s.clone();
        lone.processes[3].parent_pid = None;
        assert!(sessions::session_of(&lone, 6).is_none());
    }
}
