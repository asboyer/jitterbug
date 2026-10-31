//! Agent sessions: every command running inside a terminal, editor, or agent app on the machine,
//! with coding agents recognized and listed first, plus a launcher that opens a new terminal
//! window running an allowlisted agent in a chosen folder. Discovery reuses the Inside view's
//! process-tree sessions, so it needs no setup; a launch is registered with `jitterbug-run`
//! when that wrapper ships next to the app, which makes the session a connected agent task.
use crate::inside::{self, sessions, Lens, Unit};
use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The only commands Jitterbug may start. Never arbitrary commands.
pub const LAUNCHABLE: &[&str] = &["claude", "codex", "aider", "gemini"];

/// Executable names recognized as coding agents, with the name people know them by.
const AGENTS: &[(&str, &str)] = &[
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("aider", "Aider"),
    ("cursor-agent", "Cursor Agent"),
    ("gemini", "Gemini CLI"),
];

/// Runtimes and wrappers that may carry an agent as their direct child (an npm shim's `node`,
/// the `jitterbug-run` wrapper). A bare runtime without a recognized child stays unrecognized.
const CARRIERS: &[&str] = &["node", "bun", "deno", "jitterbug-run"];

const RECENT_FOLDERS: usize = 8;

#[derive(Clone, Debug, Serialize)]
pub struct Session {
    /// `pid-start` of the command's root process.
    pub id: String,
    pub app: String,
    /// Display name of a recognized coding agent.
    pub agent: Option<String>,
    /// The root command's name.
    pub command: String,
    /// How the Inside view names it: a registered task, a Claude Code session's own name, or
    /// the command and folder.
    pub title: String,
    /// A coding agent's own state: `working` or `idle` (waiting for the person).
    pub status: Option<String>,
    /// The registered task title, when the session was started through `jitterbug-run`.
    pub task: Option<String>,
    /// Working directory with the home folder shortened to `~`.
    pub folder: String,
    /// Full working directory, used to start another session there.
    pub path: String,
    pub started_at: u64,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub process_count: usize,
    pub paused_until: Option<u64>,
    /// `pause` and/or `resume`, as currently allowed for this command.
    pub actions: Vec<String>,
    pub connected: bool,
    /// Identity for pause and resume only; not for display.
    pub pid: u32,
    pub start_time: u64,
}

impl Session {
    pub fn key(&self) -> (u32, u64) {
        (self.pid, self.start_time)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Group {
    pub app: String,
    pub sessions: Vec<Session>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Overview {
    pub agents: Vec<Group>,
    pub other: Vec<Group>,
    /// Recent working folders from discovered sessions, agents first.
    pub folders: Vec<String>,
    /// Terminal apps a new session can open in (`terminal`, `iterm`).
    pub terminals: Vec<String>,
    /// New sessions are registered with `jitterbug-run` (connected).
    pub connected_launch: bool,
    pub vscode: bool,
    pub launch_supported: bool,
    pub message: String,
    pub captured_at: u64,
}

fn normalized(name: &str) -> String {
    let name = name.trim_start_matches('-').to_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

/// The agent key a process is, by name or executable. Claude Code's native install runs from
/// `…/claude/versions/<version>`, so a version-named executable there counts too.
pub fn agent_of(p: &ProcessRecord) -> Option<&'static str> {
    let by_name = |name: &str| {
        let name = normalized(name);
        AGENTS.iter().find(|(key, _)| *key == name).map(|(k, _)| *k)
    };
    if let Some(key) = by_name(&p.name) {
        return Some(key);
    }
    let executable = p.executable.as_deref()?;
    if let Some(key) = executable
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(by_name)
    {
        return Some(key);
    }
    let parent = executable.parent()?;
    (parent.file_name()? == "versions" && parent.parent().and_then(Path::file_name)? == "claude")
        .then_some("claude")
}

fn display(key: &str) -> String {
    AGENTS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, d)| d.to_string())
        .unwrap_or_else(|| key.to_string())
}

/// Recognize the agent a session's root runs: the root itself, or the direct child of a
/// runtime or wrapper that only carries it.
pub fn recognize(root: &ProcessRecord, children: &[&ProcessRecord]) -> Option<&'static str> {
    agent_of(root).or_else(|| {
        CARRIERS
            .contains(&normalized(&root.name).as_str())
            .then(|| children.iter().find_map(|c| agent_of(c)))
            .flatten()
    })
}

pub fn display_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Agents first, everything else under other sessions; each grouped by app with stable order
/// so rows do not jump between five-second refreshes.
pub fn group(sessions: Vec<Session>) -> (Vec<Group>, Vec<Group>) {
    let mut agents: BTreeMap<String, Vec<Session>> = BTreeMap::new();
    let mut other: BTreeMap<String, Vec<Session>> = BTreeMap::new();
    let mut seen = HashSet::new();
    for session in sessions {
        if !seen.insert(session.id.clone()) {
            continue;
        }
        let target = if session.agent.is_some() {
            &mut agents
        } else {
            &mut other
        };
        target.entry(session.app.clone()).or_default().push(session);
    }
    let finish = |map: BTreeMap<String, Vec<Session>>| {
        map.into_iter()
            .map(|(app, mut sessions)| {
                sessions.sort_by(|a, b| (a.started_at, &a.id).cmp(&(b.started_at, &b.id)));
                Group { app, sessions }
            })
            .collect()
    };
    (finish(agents), finish(other))
}

fn session(
    unit: &Unit,
    app: &str,
    by_id: &HashMap<(u32, u64), &ProcessRecord>,
    children: &HashMap<u32, Vec<&ProcessRecord>>,
    registered: &HashMap<(u32, u64), (&str, &str)>,
    home: Option<&Path>,
) -> Option<Session> {
    let (pid, start_time) = (unit.pid?, unit.start_time?);
    let root = by_id.get(&(pid, start_time))?;
    let kids = children.get(&pid).map(Vec::as_slice).unwrap_or_default();
    let registration = registered.get(&(pid, start_time));
    let agent = recognize(root, kids)
        .map(display)
        .or_else(|| registration.map(|(_, agent)| agent.to_string()));
    let path = root.working_directory.clone().unwrap_or_default();
    Some(Session {
        id: unit.id.clone(),
        app: app.to_string(),
        agent,
        command: root.name.clone(),
        title: unit.title.clone(),
        status: unit.agent_status.clone(),
        task: registration.map(|(task, _)| task.to_string()),
        folder: if path.as_os_str().is_empty() {
            String::new()
        } else {
            display_path(&path, home)
        },
        path: path.to_string_lossy().into_owned(),
        started_at: start_time,
        cpu_percent: unit.cpu_percent,
        memory_bytes: unit.memory_bytes,
        process_count: unit.process_count,
        paused_until: unit.paused_until,
        actions: unit
            .actions
            .iter()
            .filter(|a| *a == "pause" || *a == "resume")
            .cloned()
            .collect(),
        connected: registration.is_some(),
        pid,
        start_time,
    })
}

/// Every command session found inside terminal, editor, and agent apps in this snapshot.
pub fn discover(snapshot: &Snapshot) -> Vec<Session> {
    let by_id: HashMap<(u32, u64), &ProcessRecord> = snapshot
        .processes
        .iter()
        .map(|p| ((p.pid, p.start_time), p))
        .collect();
    let by_pid: HashMap<u32, &ProcessRecord> =
        snapshot.processes.iter().map(|p| (p.pid, p)).collect();
    let mut children: HashMap<u32, Vec<&ProcessRecord>> = HashMap::new();
    for p in &snapshot.processes {
        let Some(parent) = p.parent_pid.and_then(|id| by_pid.get(&id)) else {
            continue;
        };
        if parent.pid != p.pid
            && parent.start_time <= p.start_time
            && p.parent_start_time.is_none_or(|s| s == parent.start_time)
        {
            children.entry(parent.pid).or_default().push(p);
        }
    }
    let registered: HashMap<(u32, u64), (&str, &str)> = snapshot
        .agent_workflows
        .iter()
        .flat_map(|w| {
            w.agents
                .iter()
                .filter(|a| a.status == "active")
                .map(move |a| {
                    (
                        (a.root_pid, a.root_start_time),
                        (w.task_title.as_str(), a.agent.as_str()),
                    )
                })
        })
        .collect();
    let home = dirs::home_dir();
    let mut out = Vec::new();
    for workload in &snapshot.workloads {
        let members: Vec<&ProcessRecord> = snapshot
            .processes
            .iter()
            .filter(|p| p.workload_id == workload.id)
            .collect();
        if members.is_empty() || !matches!(inside::lens(&members), Lens::Sessions) {
            continue;
        }
        let view = sessions::inspect(snapshot, &workload.id, &members);
        out.extend(view.units.iter().filter_map(|unit| {
            session(
                unit,
                &workload.label,
                &by_id,
                &children,
                &registered,
                home.as_deref(),
            )
        }));
    }
    out
}

pub fn overview(snapshot: &Snapshot) -> Overview {
    let mut overview = Overview {
        terminals: terminals(),
        connected_launch: runner().is_some(),
        vscode: vscode_cli().is_some(),
        launch_supported: cfg!(target_os = "macos"),
        captured_at: snapshot.captured_at,
        ..Default::default()
    };
    if snapshot.source != "live" || snapshot.captured_at + 20 < now() {
        overview.message = "Waiting for a fresh process sample…".into();
        return overview;
    }
    let home = dirs::home_dir();
    let (agents, other) = group(discover(snapshot));
    let mut folders = Vec::new();
    for s in agents.iter().chain(&other).flat_map(|g| &g.sessions) {
        let usable = !s.path.is_empty() && home.as_deref() != Some(Path::new(&s.path));
        if usable && !folders.contains(&s.path) && folders.len() < RECENT_FOLDERS {
            folders.push(s.path.clone());
        }
    }
    let count = |groups: &[Group]| groups.iter().map(|g| g.sessions.len()).sum::<usize>();
    overview.message = match (count(&agents), count(&other)) {
        (0, 0) => "No commands are running inside terminals or editors right now.".into(),
        (a, o) => format!(
            "{a} agent session{} and {o} other command{} found.",
            if a == 1 { "" } else { "s" },
            if o == 1 { "" } else { "s" }
        ),
    };
    overview.agents = agents;
    overview.other = other;
    overview.folders = folders;
    overview
}

// ---- Launching ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalApp {
    Terminal,
    ITerm,
}

impl TerminalApp {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "" | "terminal" => Ok(Self::Terminal),
            "iterm" => Ok(Self::ITerm),
            _ => Err("Choose Terminal or iTerm.".into()),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::ITerm => "iTerm",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct LaunchRequest {
    pub folder: String,
    pub agent: String,
    #[serde(default)]
    pub terminal: String,
}

/// The `jitterbug-run` wrapper that registers a launch as a connected task.
#[derive(Clone, Debug)]
pub struct Runner {
    pub path: PathBuf,
    pub task_id: String,
}

/// Single-quote a value for a POSIX shell: nothing inside is interpreted.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// A double-quoted AppleScript string literal.
pub fn applescript_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn has_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

/// An existing, absolute folder as a UTF-8 path without control characters. `~/` is expanded.
pub fn validate_folder(folder: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let folder = folder.trim();
    if folder.is_empty() {
        return Err("Choose a folder.".into());
    }
    if has_control(folder) {
        return Err("The folder path contains control characters.".into());
    }
    let path = match (folder.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ if folder == "~" => home.map(Path::to_path_buf).unwrap_or_default(),
        _ => PathBuf::from(folder),
    };
    if !path.is_absolute() {
        return Err("Use a full folder path, starting with / or ~/.".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| "That folder does not exist.".to_string())?;
    if !canonical.is_dir() {
        return Err("That path is not a folder.".into());
    }
    match canonical.to_str() {
        Some(s) if !has_control(s) => Ok(canonical),
        _ => Err("The folder path must be plain text.".into()),
    }
}

pub fn validate_agent(agent: &str) -> Result<&'static str, String> {
    LAUNCHABLE
        .iter()
        .find(|a| **a == agent)
        .copied()
        .ok_or_else(|| {
            format!(
                "Only these agents can be started: {}.",
                LAUNCHABLE.join(", ")
            )
        })
}

fn harness(agent: &str) -> &'static str {
    match agent {
        "claude" => "claude",
        "codex" => "codex",
        _ => "other",
    }
}

/// The shell line typed into the new terminal. Every value is single-quoted; the agent comes
/// from the allowlist.
pub fn shell_command(folder: &Path, agent: &'static str, runner: Option<&Runner>) -> String {
    let dir = folder.to_string_lossy();
    let cd = format!("cd {}", shell_quote(&dir));
    let Some(runner) = runner else {
        return format!("{cd} && {agent}");
    };
    let name: String = folder
        .file_name()
        .map(|n| n.to_string_lossy().chars().take(60).collect())
        .unwrap_or_else(|| dir.chars().take(60).collect());
    let title = format!("{} in {name}", display(agent));
    format!(
        "{cd} && {} --task-id {} --task-title {} --agent {agent} --harness {} --worktree {} -- {agent}",
        shell_quote(&runner.path.to_string_lossy()),
        shell_quote(&runner.task_id),
        shell_quote(&title),
        harness(agent),
        shell_quote(&dir),
    )
}

pub fn applescript(terminal: TerminalApp, command: &str) -> String {
    let command = applescript_string(command);
    match terminal {
        TerminalApp::Terminal => format!(
            "tell application \"Terminal\"\n    activate\n    do script {command}\nend tell"
        ),
        TerminalApp::ITerm => format!(
            "tell application \"iTerm\"\n    activate\n    set newWindow to (create window with default profile)\n    tell current session of newWindow to write text {command}\nend tell"
        ),
    }
}

/// Validate a request and build the AppleScript that opens the session. Nothing is run.
pub fn plan(
    request: &LaunchRequest,
    home: Option<&Path>,
    runner: Option<&Runner>,
) -> Result<(TerminalApp, String), String> {
    let agent = validate_agent(request.agent.trim())?;
    let folder = validate_folder(&request.folder, home)?;
    let terminal = TerminalApp::parse(&request.terminal)?;
    if let Some(runner) = runner {
        let path = runner.path.to_str().unwrap_or("\n");
        if has_control(path) || has_control(&runner.task_id) {
            return Err("The session wrapper's path is not usable.".into());
        }
    }
    Ok((
        terminal,
        applescript(terminal, &shell_command(&folder, agent, runner)),
    ))
}

/// `jitterbug-run` next to the running app, if it was built or bundled there.
pub fn runner() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) {
        "jitterbug-run.exe"
    } else {
        "jitterbug-run"
    };
    let path = exe.with_file_name(name);
    path.is_file().then_some(path)
}

fn iterm_installed() -> bool {
    let mut places = vec![PathBuf::from("/Applications/iTerm.app")];
    if let Some(home) = dirs::home_dir() {
        places.push(home.join("Applications/iTerm.app"));
    }
    places.iter().any(|p| p.is_dir())
}

fn terminals() -> Vec<String> {
    if !cfg!(target_os = "macos") {
        return vec![];
    }
    let mut out = vec!["terminal".to_string()];
    if iterm_installed() {
        out.push("iterm".into());
    }
    out
}

/// Open a new terminal window running an allowlisted agent in a folder.
pub fn launch(request: &LaunchRequest) -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err(
            "Starting agent sessions is not supported on this platform yet. Start the agent in a terminal; it will appear here automatically.".into(),
        );
    }
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let runner = runner().map(|path| Runner {
        path,
        task_id: format!("session-{unique:x}"),
    });
    let home = dirs::home_dir();
    let (terminal, script) = plan(request, home.as_deref(), runner.as_ref())?;
    if terminal == TerminalApp::ITerm && !iterm_installed() {
        return Err("iTerm is not installed in Applications.".into());
    }
    let output = std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|_| "Cannot run the macOS script launcher.".to_string())?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(if error.contains("-1743") {
            format!(
                "macOS blocked Jitterbug from controlling {0}. Allow it in System Settings > Privacy & Security > Automation, then try again.",
                terminal.label()
            )
        } else {
            format!("{} could not open a new window.", terminal.label())
        });
    }
    Ok(format!(
        "Started {} in a new {} window{}.",
        display(validate_agent(request.agent.trim())?),
        terminal.label(),
        if runner.is_some() {
            ", connected as an agent task"
        } else {
            ""
        }
    ))
}

// ---- VS Code -----------------------------------------------------------------------------

fn vscode_cli() -> Option<PathBuf> {
    let mut places: Vec<PathBuf> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|d| d.join("code"))
            .collect();
    // An app opened from Finder gets a minimal PATH; check the usual install places too.
    places.extend(
        [
            "/usr/local/bin/code",
            "/opt/homebrew/bin/code",
            "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
        ]
        .map(PathBuf::from),
    );
    places.into_iter().find(|p| p.is_file())
}

/// Open a folder in VS Code with its `code` command. No terminal is started inside VS Code:
/// that needs a VS Code extension.
pub fn open_in_vscode(folder: &str) -> Result<String, String> {
    let folder = validate_folder(folder, dirs::home_dir().as_deref())?;
    let code = vscode_cli().ok_or("The VS Code `code` command was not found.")?;
    let status = std::process::Command::new(code)
        .arg(&folder)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|_| "Cannot start VS Code.".to_string())?;
    if !status.success() {
        return Err("VS Code could not open the folder.".into());
    }
    Ok("Opened the folder in VS Code. Start the agent in its terminal; it appears here automatically.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pid: u32, parent: Option<u32>, name: &str, exe: &str) -> ProcessRecord {
        ProcessRecord {
            pid,
            start_time: pid as u64,
            parent_pid: parent,
            name: name.into(),
            executable: Some(exe.into()),
            cpu_percent: 1.0,
            memory_bytes: 1 << 20,
            ..Default::default()
        }
    }

    fn temp_folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("jitterbug-agent-sessions-{}", std::process::id()))
            .join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn request(folder: &str, agent: &str) -> LaunchRequest {
        LaunchRequest {
            folder: folder.into(),
            agent: agent.into(),
            terminal: "terminal".into(),
        }
    }

    /// Undo `shell_quote` the way a POSIX shell reads a single word.
    fn shell_unquote(word: &str) -> String {
        let (mut out, mut quoted, mut chars) = (String::new(), false, word.chars().peekable());
        while let Some(c) = chars.next() {
            match c {
                '\'' => quoted = !quoted,
                '\\' if !quoted => out.extend(chars.next()),
                c => {
                    assert!(quoted, "unquoted character {c:?} in {word}");
                    out.push(c);
                }
            }
        }
        assert!(!quoted);
        out
    }

    /// Read an AppleScript string literal; every quote inside must be escaped.
    fn applescript_unquote(literal: &str) -> String {
        let inner = literal
            .strip_prefix('"')
            .unwrap()
            .strip_suffix('"')
            .unwrap();
        let (mut out, mut chars) = (String::new(), inner.chars());
        while let Some(c) = chars.next() {
            match c {
                '\\' => out.push(chars.next().unwrap()),
                '"' => panic!("unescaped quote in {literal}"),
                c => out.push(c),
            }
        }
        out
    }

    #[test]
    fn quoting_keeps_hostile_text_literal() {
        for value in [
            "plain",
            "it's",
            "'; rm -rf ~",
            "$(whoami) `id` \"x\" \\ ;&|",
        ] {
            let quoted = shell_quote(value);
            assert_eq!(shell_unquote(&quoted), value);
        }
        assert_eq!(shell_quote("a'b"), r"'a'\''b'");
        assert_eq!(applescript_string(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(
            applescript_unquote(&applescript_string(r#"x\" & y"#)),
            r#"x\" & y"#
        );
    }

    // Windows forbids quotes and backslashes in folder names, and the launch is macOS-only.
    #[cfg(unix)]
    #[test]
    fn the_launch_command_quotes_an_injection_attempt_in_the_folder_name() {
        let folder = temp_folder("x'; rm -rf ~; echo '\"$(id)\\");
        let (terminal, script) =
            plan(&request(folder.to_str().unwrap(), "claude"), None, None).unwrap();
        assert_eq!(terminal, TerminalApp::Terminal);
        let command = shell_command(&folder, "claude", None);
        assert_eq!(
            command,
            format!("cd {} && claude", shell_quote(folder.to_str().unwrap()))
        );
        assert!(command.contains(r"'\''; rm -rf ~; echo '\''"));
        // The AppleScript literal decodes back to exactly the shell command.
        let literal = script
            .lines()
            .find_map(|l| l.trim().strip_prefix("do script "))
            .unwrap();
        assert_eq!(applescript_unquote(literal), command);
        assert!(script.starts_with("tell application \"Terminal\""));
    }

    #[test]
    fn only_allowlisted_agents_can_be_started() {
        let folder = temp_folder("allow");
        let folder = folder.to_str().unwrap();
        for agent in LAUNCHABLE {
            assert!(plan(&request(folder, agent), None, None).is_ok());
        }
        for agent in [
            "",
            "bash",
            "rm",
            "claude; rm -rf ~",
            "claude --dangerously-skip-permissions",
            "/usr/local/bin/claude",
            "Claude",
            "cursor-agent",
        ] {
            assert!(
                plan(&request(folder, agent), None, None).is_err(),
                "{agent}"
            );
        }
    }

    #[test]
    fn folders_must_be_existing_absolute_directories_without_control_characters() {
        let file = temp_folder("files").join("file.txt");
        std::fs::write(&file, "x").unwrap();
        for folder in [
            "",
            "relative/path",
            "./here",
            "~user/elsewhere",
            "/definitely/not/a/real/folder/for/jitterbug",
            file.to_str().unwrap(),
            "/tmp\n; rm -rf ~",
            "/tmp\u{0}",
        ] {
            assert!(validate_folder(folder, None).is_err(), "{folder:?}");
        }
        let home = temp_folder("home");
        std::fs::create_dir_all(home.join("repo")).unwrap();
        assert_eq!(
            validate_folder("~/repo", Some(&home)).unwrap(),
            home.join("repo")
        );
        assert!(validate_folder("~/repo", None).is_err());
    }

    #[test]
    fn a_connected_launch_goes_through_jitterbug_run_with_quoted_values() {
        let folder = temp_folder("it's mine");
        let runner = Runner {
            path: PathBuf::from("/Apps/Jitter Bug/jitterbug-run"),
            task_id: "session-1f".into(),
        };
        let command = shell_command(&folder, "codex", Some(&runner));
        let dir = shell_quote(folder.to_str().unwrap());
        assert_eq!(
            command,
            format!("cd {dir} && '/Apps/Jitter Bug/jitterbug-run' --task-id 'session-1f' --task-title 'Codex in it'\\''s mine' --agent codex --harness codex --worktree {dir} -- codex")
        );
        let iterm = applescript(TerminalApp::ITerm, &command);
        assert!(iterm.starts_with("tell application \"iTerm\""));
        assert!(iterm.contains("write text \"cd "));
        let bad = Runner {
            path: PathBuf::from("/x\n/jitterbug-run"),
            task_id: "t".into(),
        };
        let request = request(folder.to_str().unwrap(), "codex");
        assert!(plan(&request, None, Some(&bad)).is_err());
        assert!(TerminalApp::parse("xterm").is_err());
    }

    #[test]
    fn agents_are_recognized_by_name_install_path_or_carrier_child() {
        let claude = record(1, None, "claude", "/u/.local/bin/claude");
        assert_eq!(agent_of(&claude), Some("claude"));
        let native = record(2, None, "2.1.4", "/u/.local/share/claude/versions/2.1.4");
        assert_eq!(agent_of(&native), Some("claude"));
        assert_eq!(
            agent_of(&record(3, None, "cursor-agent", "/x/cursor-agent")),
            Some("cursor-agent")
        );
        assert_eq!(agent_of(&record(4, None, "cargo", "/x/cargo")), None);
        assert_eq!(
            agent_of(&record(5, None, "claudette", "/x/claudette")),
            None
        );
        let node = record(6, None, "node", "/usr/local/bin/node");
        let codex = record(7, Some(6), "codex", "/n/codex/bin/codex");
        assert_eq!(recognize(&node, &[&codex]), Some("codex"));
        assert_eq!(recognize(&node, &[]), None);
        let python = record(8, None, "python3", "/usr/bin/python3");
        assert_eq!(recognize(&python, &[&codex]), None);
    }

    fn sample(id: &str, app: &str, agent: Option<&str>, started_at: u64) -> Session {
        Session {
            id: id.into(),
            app: app.into(),
            agent: agent.map(String::from),
            command: "x".into(),
            title: String::new(),
            status: None,
            task: None,
            folder: String::new(),
            path: String::new(),
            started_at,
            cpu_percent: 0.0,
            memory_bytes: 0,
            process_count: 1,
            paused_until: None,
            actions: vec![],
            connected: false,
            pid: 1,
            start_time: started_at,
        }
    }

    #[test]
    fn agents_come_first_grouped_by_app_in_a_stable_order() {
        let (agents, other) = group(vec![
            sample("a", "Terminal", Some("Claude Code"), 30),
            sample("b", "Visual Studio Code", Some("Codex"), 20),
            sample("c", "Terminal", None, 10),
            sample("d", "Terminal", Some("Claude Code"), 10),
            sample("d", "iTerm2", Some("Claude Code"), 10),
        ]);
        let ids = |groups: &[Group]| {
            groups
                .iter()
                .map(|g| {
                    let ids: Vec<_> = g.sessions.iter().map(|s| s.id.as_str()).collect();
                    format!("{}:{}", g.app, ids.join(","))
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&agents), ["Terminal:d,a", "Visual Studio Code:b"]);
        assert_eq!(ids(&other), ["Terminal:c"]);
    }

    #[test]
    fn discovery_lists_terminal_commands_with_recognized_agents() {
        let terminal = "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal";
        let mut processes = vec![
            record(10, None, "Terminal", terminal),
            record(11, Some(10), "login", "/usr/bin/login"),
            record(12, Some(11), "-zsh", "/bin/zsh"),
            record(13, Some(12), "claude", "/u/.local/bin/claude"),
            record(14, Some(13), "node", "/usr/local/bin/node"),
            record(15, Some(11), "zsh", "/bin/zsh"),
            record(16, Some(15), "cargo", "/u/.cargo/bin/cargo"),
        ];
        for p in &mut processes {
            p.workload_id = if p.pid == 10 { "terminal" } else { "other" }.into();
        }
        processes[3].working_directory = Some("/w/project".into());
        let snapshot = Snapshot {
            source: "live".into(),
            captured_at: now(),
            sample_ready: true,
            processes,
            workloads: vec![Workload {
                id: "terminal".into(),
                label: "Terminal".into(),
                category: "app".into(),
                description: String::new(),
                attribution: String::new(),
                cpu_percent: 0.0,
                memory_bytes: 0,
                process_count: 1,
                unresponsive: None,
                history: vec![],
            }],
            ..Default::default()
        };
        let overview = overview(&snapshot);
        assert_eq!(overview.agents.len(), 1);
        let claude = &overview.agents[0].sessions[0];
        assert_eq!(claude.app, "Terminal");
        assert_eq!(claude.agent.as_deref(), Some("Claude Code"));
        assert_eq!(claude.process_count, 2);
        assert_eq!(claude.folder, "/w/project");
        assert!(claude.actions.iter().any(|a| a == "pause"));
        assert!(!claude.connected);
        assert_eq!(overview.other[0].sessions[0].command, "cargo");
        assert_eq!(overview.folders, ["/w/project"]);
    }

    #[test]
    fn paths_under_home_are_shortened() {
        let home = Path::new("/Users/me");
        assert_eq!(
            display_path(Path::new("/Users/me/code/x"), Some(home)),
            "~/code/x"
        );
        assert_eq!(display_path(home, Some(home)), "~");
        assert_eq!(display_path(Path::new("/opt/x"), Some(home)), "/opt/x");
    }
}
