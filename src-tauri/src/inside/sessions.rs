//! Terminal, editor, and coding-agent apps: one unit per command started inside the app
//! (an agent session, a dev server, a test run), with its whole process subtree. Built from the
//! current collector inventory; no extra sampling, transcripts, or shell history.
use super::{Inside, ProcessLabel, Remainder, Unit};
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

/// iTerm copies its server outside the app bundle so sessions can survive app upgrades.
/// A process name alone is not enough: restrict this exception to its exact support folder.
/// This is path evidence, not a code-signature or publisher verification.
pub(crate) fn is_iterm_server_executable(executable: &Path) -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    let Some(version) = executable
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("iTermServer-"))
    else {
        return false;
    };
    executable.parent() == Some(home.join("Library/Application Support/iTerm2").as_path())
        && version.starts_with(|c: char| c.is_ascii_digit())
        && version
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c))
}

fn valid_parent(parent: &ProcessRecord, child: &ProcessRecord) -> bool {
    parent.pid != child.pid
        && parent.start_time <= child.start_time
        && child
            .parent_start_time
            .is_none_or(|s| s == parent.start_time)
}

fn is_iterm_server(p: &ProcessRecord, by_pid: &HashMap<u32, &ProcessRecord>) -> bool {
    let Some(executable) = p.executable.as_deref() else {
        return false;
    };
    if !is_iterm_server_executable(executable) {
        return false;
    }
    let Some(parent) = p.parent_pid.and_then(|id| by_pid.get(&id)) else {
        return false;
    };
    let Some(parent_executable) = parent.executable.as_deref() else {
        return false;
    };
    let Some(bundle) = crate::attribution::application_bundle(parent_executable) else {
        return false;
    };
    // Only traverse a server launched directly by iTerm's main executable, with a current
    // parent identity. A renamed command or a helper started by a shell is not app plumbing.
    valid_parent(parent, p)
        && matches!(
            bundle.file_stem().and_then(|s| s.to_str()),
            Some("iTerm" | "iTerm2")
        )
        && parent_executable == bundle.join("Contents/MacOS/iTerm2")
}

fn basename(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().chars().take(60).collect())
        .unwrap_or_default()
}

pub fn inspect(snapshot: &Snapshot, workload_id: &str, members: &[&ProcessRecord]) -> Inside {
    let by_pid: HashMap<u32, &ProcessRecord> =
        snapshot.processes.iter().map(|p| (p.pid, p)).collect();
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
    // An app's own processes live inside its bundle, except iTerm's copied session server.
    let own = |p: &ProcessRecord| {
        p.executable
            .as_deref()
            .and_then(crate::attribution::application_bundle)
            .is_some_and(|b| bundles.contains(b))
            || is_iterm_server(p, &by_pid)
    };
    let mut children: HashMap<u32, Vec<&ProcessRecord>> = HashMap::new();
    for p in &snapshot.processes {
        let Some(parent) = p.parent_pid.and_then(|id| by_pid.get(&id)) else {
            continue;
        };
        // A reused parent PID must not adopt an unrelated process.
        if !valid_parent(parent, p) {
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
    let mut wrappers = Vec::new();
    let mut labels: Vec<ProcessLabel> = Vec::new();
    while let Some(p) = queue.pop() {
        if is_iterm_server(p, &by_pid) {
            labels.push(ProcessLabel::new(
                p,
                "Terminal session server",
                "Keeps iTerm sessions running",
            ));
        }
        if own(p) {
            own_total.cpu_percent += p.cpu_percent;
            own_total.memory_bytes += p.memory_bytes;
            own_total.process_count += 1;
        } else {
            wrappers.push(p);
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
            let unit = unit(root, &tree, &app_name, &registered);
            let (title, subtitle) = session_label(&unit, root);
            // The shells that launched the command belong to its session, not to idle ones.
            let mut launchers = Vec::new();
            let mut current = root;
            while let Some(parent) = current.parent_pid.and_then(|id| by_pid.get(&id)) {
                if own(parent) || !is_wrapper(parent) || !valid_parent(parent, current) {
                    break;
                }
                launchers.push(*parent);
                current = parent;
            }
            for p in tree.iter().chain(&launchers) {
                if !labels.iter().any(|l| l.pid == p.pid) {
                    labels.push(ProcessLabel::new(p, &title, &subtitle));
                }
            }
            unit
        })
        .collect();
    for p in wrappers {
        if !labels.iter().any(|l| l.pid == p.pid) {
            labels.push(ProcessLabel::new(
                p,
                "Idle shell",
                "A terminal shell with nothing running in it",
            ));
        }
    }
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
    inside.labels = labels;
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
    // A home folder's name is the account name, not a project: leave it out of the title.
    let folder = root
        .working_directory
        .as_deref()
        .filter(|dir| dirs::home_dir().as_deref() != Some(*dir))
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
    // A Claude Code session's own name (set with /rename) and whether it is working.
    let claude = tree
        .iter()
        .find_map(|p| crate::claude_sessions::lookup(p.pid, p.start_time));
    if let Some(status) = claude.as_ref().and_then(|c| c.status.as_deref()) {
        evidence.push(format!("Claude Code reports this session as {status}."));
    }
    let named = claude.as_ref().and_then(|c| c.name.clone());
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
        None if named.is_some() => {
            let name = named.clone().unwrap_or_default();
            (
                if folder.is_empty() {
                    format!("{name} · Claude Code")
                } else {
                    format!("{name} · Claude Code · {folder}")
                },
                "Named Claude Code session".to_string(),
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
        agent_status: claude.and_then(|c| c.status),
    }
}

/// Coding agents whose command name is not what people call them.
const AGENTS: &[(&str, &str)] = &[
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("gemini", "Gemini CLI"),
    ("aider", "Aider"),
    ("opencode", "OpenCode"),
    ("cursor-agent", "Cursor Agent"),
];

/// A session's process title: a registered task as registered, a known agent by its product
/// name ("Claude Code session · repo"), anything else as its command and folder.
fn session_label(unit: &Unit, root: &ProcessRecord) -> (String, String) {
    if unit.confidence == "Registered task" || unit.confidence == "Named Claude Code session" {
        return (unit.title.clone(), "Registered agent task".into());
    }
    let name = root.name.to_lowercase();
    match AGENTS.iter().find(|(command, _)| *command == name) {
        Some((_, agent)) => (
            unit.title
                .replacen(&root.name, &format!("{agent} session"), 1),
            "Coding agent session".into(),
        ),
        None => (unit.title.clone(), "Command session".into()),
    }
}

/// Existing process actions, plus pause or resume for the whole command. The pause policy is
/// rechecked against live ownership when it runs.
fn session_actions(root: &ProcessRecord) -> Vec<String> {
    if crate::pause::is_paused(root.pid, root.start_time) {
        return vec!["resume".into()];
    }
    if root
        .executable
        .as_deref()
        .is_some_and(is_iterm_server_executable)
    {
        return vec![];
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
    if bundle(start).is_some()
        || start
            .executable
            .as_deref()
            .is_some_and(is_iterm_server_executable)
    {
        return None;
    }
    let (mut root, mut current, mut app) = (*start, *start, None);
    for _ in 0..32 {
        let Some(parent) = current.parent_pid.and_then(|id| by_pid.get(&id)) else {
            break;
        };
        if !valid_parent(parent, current) {
            break;
        }
        if let Some(name) = bundle(parent) {
            app = Some(name);
            break;
        }
        if !is_wrapper(parent) && !is_iterm_server(parent, &by_pid) {
            root = parent;
        }
        current = parent;
    }
    let app = app?;
    let mut children: HashMap<u32, Vec<&ProcessRecord>> = HashMap::new();
    for p in &snapshot.processes {
        if let Some(parent) = p.parent_pid.and_then(|id| by_pid.get(&id)) {
            if valid_parent(parent, p) {
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

    fn iterm() -> Snapshot {
        let server = dirs::home_dir()
            .unwrap()
            .join("Library/Application Support/iTerm2/iTermServer-3.7.6");
        Snapshot {
            source: "live".into(),
            sample_ready: true,
            processes: vec![
                p(
                    1,
                    None,
                    "iTerm2",
                    "/Applications/iTerm.app/Contents/MacOS/iTerm2",
                    0.5,
                ),
                p(
                    2,
                    Some(1),
                    "iTermServer-3.7.6",
                    server.to_str().unwrap(),
                    0.5,
                ),
                p(3, Some(2), "login", "/usr/bin/login", 0.0),
                p(4, Some(3), "-zsh", "/bin/zsh", 0.0),
                p(5, Some(4), "node", "/usr/local/bin/node", 8.0),
                p(6, Some(5), "worker", "/opt/tools/worker", 4.0),
                p(7, Some(2), "zsh", "/bin/zsh", 0.0),
                p(8, Some(7), "python3", "/usr/local/bin/python3", 3.0),
                p(9, Some(2), "login", "/usr/bin/login", 0.0),
                p(10, Some(9), "zsh", "/bin/zsh", 0.0),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn iterm_server_is_app_plumbing_and_commands_have_separate_resources() {
        let snapshot = iterm();
        let members = snapshot.processes.iter().collect::<Vec<_>>();
        let inside = inspect(&snapshot, "iterm", &members);
        assert_eq!(inside.units.len(), 2);
        let first = &inside.units[0];
        assert_eq!((first.pid, first.process_count), (Some(5), 2));
        assert_eq!(first.cpu_percent, 12.0);
        assert_eq!(first.memory_bytes, 2 << 20);
        assert!(first.actions.iter().any(|a| a == "pause"));
        let second = &inside.units[1];
        assert_eq!((second.pid, second.process_count), (Some(8), 1));
        assert_eq!(second.cpu_percent, 3.0);
        assert_eq!(inside.remainder[0].process_count, 2);
        assert_eq!(inside.remainder[0].cpu_percent, 1.0);
        assert_eq!(inside.remainder[1].process_count, 5);
    }

    #[test]
    fn the_iterm_server_and_commands_are_named() {
        let snapshot = iterm();
        let members = snapshot.processes.iter().collect::<Vec<_>>();
        let inside = inspect(&snapshot, "iterm", &members);
        let title = |pid: u32| {
            inside
                .labels
                .iter()
                .find(|l| l.pid == pid)
                .map(|l| l.title.as_str())
        };
        assert_eq!(title(2), Some("Terminal session server"));
        assert_eq!(title(6), Some("node"));
        assert_eq!(title(3), Some("node"));
        assert_eq!(title(8), Some("python3"));
        assert_eq!(title(10), Some("Idle shell"));
    }

    #[test]
    fn incident_attribution_targets_the_command_not_all_iterm_sessions() {
        let snapshot = iterm();
        let first = session_of(&snapshot, 6).unwrap();
        assert_eq!((first.pid, first.process_count), (Some(5), 2));
        assert_eq!(first.cpu_percent, 12.0);
        let second = session_of(&snapshot, 8).unwrap();
        assert_eq!((second.pid, second.process_count), (Some(8), 1));
        assert!(session_of(&snapshot, 2).is_none());
        assert!(session_actions(&snapshot.processes[1]).is_empty());
    }

    #[test]
    fn a_similarly_named_command_outside_iterms_support_folder_stays_a_command() {
        let mut snapshot = iterm();
        for path in [
            "/tmp/iTermServer-3.7.6",
            "/tmp/Library/Application Support/iTerm2/iTermServer-3.7.6",
        ] {
            snapshot.processes[1].executable = Some(path.into());
            let members = snapshot.processes.iter().collect::<Vec<_>>();
            let inside = inspect(&snapshot, "iterm", &members);
            assert_eq!(inside.units.len(), 1);
            assert_eq!(inside.units[0].pid, Some(2));
            assert_eq!(session_of(&snapshot, 6).unwrap().pid, Some(2));
        }
    }

    #[test]
    fn the_server_exception_requires_iterms_main_executable_as_its_parent() {
        let mut snapshot = iterm();
        for parent in [
            "/Applications/Other.app/Contents/MacOS/iTerm2",
            "/Applications/iTerm.app/Contents/MacOS/other",
        ] {
            snapshot.processes[0].executable = Some(parent.into());
            let members = snapshot.processes.iter().collect::<Vec<_>>();
            let inside = inspect(&snapshot, "iterm", &members);
            assert_eq!(inside.units.len(), 1);
            assert_eq!(inside.units[0].pid, Some(2));
            // Do not offer pausing a possible server, even without enough evidence to
            // split its sessions (e.g. a detached server or an unexpected parent).
            assert!(inside.units[0].actions.is_empty());
        }
    }

    #[test]
    fn changed_parent_identity_cannot_attach_a_server_or_an_extra_command_child() {
        let mut snapshot = iterm();
        snapshot.processes[1].parent_start_time = Some(99);
        let members = snapshot.processes.iter().collect::<Vec<_>>();
        assert!(inspect(&snapshot, "iterm", &members).units.is_empty());
        assert!(session_of(&snapshot, 6).is_none());

        let mut snapshot = iterm();
        snapshot.processes[5].parent_start_time = Some(99);
        let members = snapshot.processes.iter().collect::<Vec<_>>();
        let inside = inspect(&snapshot, "iterm", &members);
        assert_eq!(inside.units[0].process_count, 1);
        assert_eq!(session_of(&snapshot, 5).unwrap().process_count, 1);
        assert!(session_of(&snapshot, 6).is_none());
    }

    #[test]
    fn a_server_without_its_original_parent_never_becomes_a_pause_target() {
        let mut snapshot = iterm();
        snapshot.processes[1].parent_pid = None;
        assert!(session_of(&snapshot, 2).is_none());
        assert!(session_actions(&snapshot.processes[1]).is_empty());
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
        // Every process of the session, and the shells that launched it, carry its name.
        let title = |pid: u32| {
            inside
                .labels
                .iter()
                .find(|l| l.pid == pid)
                .map(|l| l.title.as_str())
        };
        for pid in [3, 4, 5, 6] {
            assert_eq!(title(pid), Some("Claude Code session · feat-x"));
        }
        assert_eq!(title(7), Some("Idle shell"));
        // The app's own processes are named by role in `inspect_with`, not here.
        assert_eq!(title(1), None);
        assert_eq!(title(8), None);
    }

    #[test]
    fn a_session_in_the_home_folder_is_not_titled_by_the_account_name() {
        let Some(home) = dirs::home_dir() else { return };
        let app = "/Applications/Terminal.app/Contents/MacOS/Terminal";
        let mut perl = p(3, Some(2), "perl", "/usr/bin/perl", 5.0);
        perl.working_directory = Some(home);
        let snapshot = Snapshot {
            source: "live".into(),
            captured_at: now(),
            sample_ready: true,
            processes: vec![
                p(1, None, "Terminal", app, 0.0),
                p(2, Some(1), "zsh", "/bin/zsh", 0.0),
                perl,
            ],
            ..Default::default()
        };
        let members: Vec<_> = snapshot.processes.iter().take(2).collect();
        assert_eq!(inspect(&snapshot, "t", &members).units[0].title, "perl");
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
