//! Terminal, editor, and coding-agent apps: one unit per command started inside the app
//! (an agent session, a dev server, a test run), with its whole process subtree. Built from the
//! current collector inventory; no extra sampling, transcripts, or shell history.
use super::{Inside, Remainder, Unit};
use crate::types::*;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const HOSTS: &[&str] = &[
    "terminal",
    "iterm",
    "iterm2",
    "warp",
    "ghostty",
    "kitty",
    "alacritty",
    "wezterm",
    "hyper",
    "tabby",
    "visual studio code",
    "code",
    "cursor",
    "windsurf",
    "zed",
    "claude",
    "codex",
    "xcode",
    "intellij idea",
    "pycharm",
    "webstorm",
    "rustrover",
    "android studio",
];

/// Launch plumbing between the app and the command a person or agent started.
const WRAPPERS: &[&str] = &[
    "login",
    "zsh",
    "bash",
    "fish",
    "sh",
    "dash",
    "nu",
    "tmux",
    "screen",
    "script",
    "sudo",
    "pwsh",
    "powershell",
    "cmd",
];

const UNIT_LIMIT: usize = 40;

fn is_wrapper(p: &ProcessRecord) -> bool {
    let name = p.name.trim_start_matches('-').to_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    WRAPPERS.contains(&name)
}

fn basename(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().chars().take(60).collect())
        .unwrap_or_default()
}

pub fn inspect(snapshot: &Snapshot, workload_id: &str, members: &[&ProcessRecord]) -> Inside {
    let bundles: HashSet<&Path> = members
        .iter()
        .filter_map(|p| p.executable.as_deref())
        .filter_map(crate::attribution::application_bundle)
        .collect();
    let app_name = bundles
        .iter()
        .next()
        .map(|b| {
            b.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_else(|| "the app".into());
    // An app's own processes live inside its bundle; commands it launched do not.
    let own = |p: &ProcessRecord| {
        p.executable
            .as_deref()
            .and_then(crate::attribution::application_bundle)
            .is_some_and(|b| bundles.contains(b))
    };
    let mut children: HashMap<u32, Vec<&ProcessRecord>> = HashMap::new();
    let by_pid: HashMap<u32, &ProcessRecord> =
        snapshot.processes.iter().map(|p| (p.pid, p)).collect();
    for p in &snapshot.processes {
        let Some(parent) = p.parent_pid.and_then(|id| by_pid.get(&id)) else {
            continue;
        };
        // A reused parent PID must not adopt an unrelated process.
        if parent.pid == p.pid
            || parent.start_time > p.start_time
            || p.parent_start_time.is_some_and(|s| s != parent.start_time)
        {
            continue;
        }
        children.entry(parent.pid).or_default().push(p);
    }
    let mut queue: Vec<&ProcessRecord> = members
        .iter()
        .copied()
        .filter(|p| own(p))
        .filter(|p| {
            !p.parent_pid
                .and_then(|id| by_pid.get(&id))
                .is_some_and(|parent| own(parent))
        })
        .collect();
    let mut seen: HashSet<u32> = queue.iter().map(|p| p.pid).collect();
    let (mut own_total, mut shells) = (Remainder::default(), Remainder::default());
    let mut roots = Vec::new();
    while let Some(p) = queue.pop() {
        if own(p) {
            own_total.cpu_percent += p.cpu_percent;
            own_total.memory_bytes += p.memory_bytes;
            own_total.process_count += 1;
        } else {
            shells.cpu_percent += p.cpu_percent;
            shells.memory_bytes += p.memory_bytes;
            shells.process_count += 1;
        }
        for child in children.get(&p.pid).into_iter().flatten() {
            if !seen.insert(child.pid) {
                continue;
            }
            if own(child) || is_wrapper(child) {
                queue.push(child);
            } else {
                roots.push(*child);
            }
        }
    }
    let registered: Vec<(
        &crate::workflows::AgentWorkflow,
        &crate::workflows::AgentSession,
    )> = snapshot
        .agent_workflows
        .iter()
        .flat_map(|w| w.agents.iter().map(move |a| (w, a)))
        .collect();
    let mut units: Vec<Unit> = roots
        .into_iter()
        .map(|root| {
            let mut tree = vec![root];
            let mut i = 0;
            while i < tree.len() && tree.len() < 512 {
                for child in children.get(&tree[i].pid).into_iter().flatten() {
                    if seen.insert(child.pid) {
                        tree.push(child);
                    }
                }
                i += 1;
            }
            unit(root, &tree, &app_name, &registered)
        })
        .collect();
    units.sort_by(|a, b| {
        (b.cpu_percent + b.memory_bytes as f32 / 1e9)
            .total_cmp(&(a.cpu_percent + a.memory_bytes as f32 / 1e9))
    });
    units.truncate(UNIT_LIMIT);
    own_total.label = format!("{app_name} itself (window, renderer, extensions, helpers)");
    shells.label = "Idle shells and launchers".into();
    let mut inside = Inside::status(
        workload_id,
        "Process tree",
        "ready",
        if units.is_empty() {
            format!("Nothing is running inside {app_name} beyond the app itself.")
        } else {
            format!(
                "{} command{} running inside {app_name}.",
                units.len(),
                if units.len() == 1 { "" } else { "s" }
            )
        },
    );
    inside.units = units;
    inside.remainder = [own_total, shells]
        .into_iter()
        .filter(|r| r.process_count > 0)
        .collect();
    inside.captured_at = snapshot.captured_at;
    inside.limits = "Built from the current parent chain. Detached or reparented commands (tmux servers, daemons, background jobs after their shell exits) are not shown here. CPU and memory are the last five-second sample; summed memory may double-count shared pages. A command's name and folder do not prove which task it is doing unless it was registered with jitterbug-run.".into();
    inside
}

fn unit(
    root: &ProcessRecord,
    tree: &[&ProcessRecord],
    app: &str,
    registered: &[(
        &crate::workflows::AgentWorkflow,
        &crate::workflows::AgentSession,
    )],
) -> Unit {
    let cpu: f32 = tree.iter().map(|p| p.cpu_percent).sum();
    let memory: u64 = tree.iter().map(|p| p.memory_bytes).sum();
    let folder = root
        .working_directory
        .as_deref()
        .map(basename)
        .unwrap_or_default();
    let mut evidence = vec![format!(
        "Started inside {app}: {} process{} in this command's tree.",
        tree.len(),
        if tree.len() == 1 { "" } else { "es" }
    )];
    if let Some(hot) = tree
        .iter()
        .skip(1)
        .max_by(|a, b| a.cpu_percent.total_cmp(&b.cpu_percent))
        .filter(|p| p.cpu_percent >= 1.0)
    {
        evidence.push(format!(
            "Busiest child: {} at {:.1}% of total CPU.",
            hot.name, hot.cpu_percent
        ));
    }
    let session = registered.iter().find(|(_, a)| {
        tree.iter()
            .any(|p| a.root_pid == p.pid && a.root_start_time == p.start_time)
    });
    let (title, confidence) = match session {
        Some((task, agent)) => {
            evidence.push(format!(
                "Registered with jitterbug-run as agent “{}” ({}).",
                agent.agent, agent.harness
            ));
            (
                format!("{} · {}", task.task_title, agent.agent),
                "Registered task".to_string(),
            )
        }
        None if folder.is_empty() => (root.name.clone(), "Process tree".to_string()),
        None => (
            format!("{} · {folder}", root.name),
            "Process tree and working directory".to_string(),
        ),
    };
    Unit {
        id: format!("{}-{}", root.pid, root.start_time),
        kind: "session".into(),
        title,
        detail: root
            .working_directory
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        host: Some(root.name.clone()),
        cpu_percent: cpu,
        memory_bytes: memory,
        process_count: tree.len(),
        shared_with: 0,
        pid: Some(root.pid),
        start_time: Some(root.start_time),
        actions: session_actions(root),
        evidence,
        confidence,
        paused_until: crate::pause::list()
            .into_iter()
            .find(|p| p.pid == root.pid && p.start_time == root.start_time)
            .map(|p| p.until),
    }
}

/// Existing process actions, plus pause or resume for the whole command. The pause policy is
/// rechecked against live ownership when it runs.
fn session_actions(root: &ProcessRecord) -> Vec<String> {
    if crate::pause::is_paused(root.pid, root.start_time) {
        return vec!["resume".into()];
    }
    let mut actions = vec!["pause".into()];
    actions.extend(root.actions.iter().cloned());
    actions
}

/// The command inside a terminal, editor, or agent app that contains `pid`, as one unit with
/// its whole tree. `None` when the process is an app's own or no app launched it.
pub fn session_of(snapshot: &Snapshot, pid: u32) -> Option<Unit> {
    let by_pid: HashMap<u32, &ProcessRecord> =
        snapshot.processes.iter().map(|p| (p.pid, p)).collect();
    let bundle = |p: &ProcessRecord| {
        p.executable
            .as_deref()
            .and_then(crate::attribution::application_bundle)
            .and_then(|b| b.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
    };
    let start = by_pid.get(&pid)?;
    if bundle(start).is_some() {
        return None;
    }
    let (mut root, mut current, mut app) = (*start, *start, None);
    for _ in 0..32 {
        let Some(parent) = current.parent_pid.and_then(|id| by_pid.get(&id)) else {
            break;
        };
        if parent.start_time > current.start_time
            || current
                .parent_start_time
                .is_some_and(|s| s != parent.start_time)
        {
            break;
        }
        if let Some(name) = bundle(parent) {
            app = Some(name);
            break;
        }
        if !is_wrapper(parent) {
            root = parent;
        }
        current = parent;
    }
    let app = app?;
    let mut children: HashMap<u32, Vec<&ProcessRecord>> = HashMap::new();
    for p in &snapshot.processes {
        if let Some(parent) = p.parent_pid.and_then(|id| by_pid.get(&id)) {
            if parent.pid != p.pid && parent.start_time <= p.start_time {
                children.entry(parent.pid).or_default().push(p);
            }
        }
    }
    let mut tree = vec![root];
    let mut i = 0;
    while i < tree.len() && tree.len() < 512 {
        for child in children.get(&tree[i].pid).into_iter().flatten() {
            if !tree.iter().any(|t| t.pid == child.pid) {
                tree.push(child);
            }
        }
        i += 1;
    }
    let registered: Vec<_> = snapshot
        .agent_workflows
        .iter()
        .flat_map(|w| w.agents.iter().map(move |a| (w, a)))
        .collect();
    let mut unit = unit(root, &tree, &app, &registered);
    unit.title = format!("{} in {app}", unit.title);
    Some(unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: Option<u32>, name: &str, exe: &str, cpu: f32) -> ProcessRecord {
        ProcessRecord {
            pid,
            start_time: pid as u64,
            parent_pid: parent,
            name: name.into(),
            executable: Some(exe.into()),
            cpu_percent: cpu,
            memory_bytes: 1 << 20,
            workload_id: "code".into(),
            ..Default::default()
        }
    }

    #[test]
    fn commands_under_shells_become_units_with_their_subtree() {
        let app = "/A/Visual Studio Code.app/Contents";
        let mut agent = p(5, Some(4), "claude", "/u/.local/bin/claude", 4.0);
        agent.working_directory = Some("/w/repo.worktrees/feat-x".into());
        let snapshot = Snapshot {
            source: "live".into(),
            captured_at: now(),
            sample_ready: true,
            processes: vec![
                p(1, None, "Code", &format!("{app}/MacOS/Code"), 1.0),
                p(
                    2,
                    Some(1),
                    "Code Helper",
                    &format!("{app}/Frameworks/H.app/Contents/MacOS/h"),
                    0.5,
                ),
                p(3, Some(2), "zsh", "/bin/zsh", 0.0),
                p(4, Some(3), "zsh", "/bin/zsh", 0.0),
                agent,
                p(6, Some(5), "node", "/usr/local/bin/node", 6.0),
                p(7, Some(2), "zsh", "/bin/zsh", 0.0),
                p(8, None, "unrelated", "/usr/bin/unrelated", 50.0),
            ],
            ..Default::default()
        };
        let members: Vec<_> = snapshot.processes.iter().filter(|p| p.pid <= 7).collect();
        let inside = inspect(&snapshot, "code", &members);
        assert_eq!(inside.units.len(), 1);
        let unit = &inside.units[0];
        assert_eq!(unit.title, "claude · feat-x");
        assert!((unit.cpu_percent - 10.0).abs() < 0.01);
        assert_eq!(unit.pid, Some(5));
        assert!(unit.evidence.iter().any(|e| e.contains("node")));
        let own = &inside.remainder[0];
        assert_eq!(own.process_count, 2);
        assert_eq!(inside.remainder[1].process_count, 3);
    }

    #[test]
    fn reused_parent_ids_do_not_adopt_processes() {
        let app = "/Applications/Terminal.app/Contents/MacOS/Terminal";
        let mut late_parent = p(10, None, "Terminal", app, 0.0);
        late_parent.start_time = 100;
        let snapshot = Snapshot {
            source: "live".into(),
            captured_at: now(),
            sample_ready: true,
            processes: vec![late_parent, p(11, Some(10), "cargo", "/c/cargo", 9.0)],
            ..Default::default()
        };
        let members: Vec<_> = snapshot.processes.iter().take(1).collect();
        assert!(inspect(&snapshot, "t", &members).units.is_empty());
    }
}
