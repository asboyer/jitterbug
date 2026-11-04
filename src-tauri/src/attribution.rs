use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
};
use sysinfo::{Pid, Process, System};

#[derive(Default)]
pub struct Attribution {
    projects: HashMap<PathBuf, Option<PathBuf>>,
    /// Coding-agent identity per process (PID and start time), reused across samples.
    agents: HashMap<(u32, u64), Option<&'static str>>,
    /// Parents the process table could not read (root-owned launchers such as `login`),
    /// filled from the kernel for this sample.
    pub bridges: HashMap<u32, u32>,
}

pub struct Label {
    pub id: String,
    pub label: String,
    pub category: String,
    pub description: String,
    pub evidence: Vec<String>,
    pub confidence: String,
    /// What started this process, as a short sentence ("Started by Claude Code in iTerm").
    /// Names only: never paths or arguments.
    pub origin: Option<String>,
    /// What started the workload this process belongs to, judged from this process.
    pub group_origin: Option<String>,
    /// Whether this process is its workload's root (an app's main executable or the agent
    /// itself), whose `group_origin` outranks other members'.
    pub group_root: bool,
}

const SYSTEM_DESCRIPTION: &str =
    "Part of macOS. Built-in services run in the background and are usually best left alone.";
const APPLICATION_DESCRIPTION: &str =
    "Installed application. One app can run several helper processes.";

/// Word-like tokens: split on punctuation and camel case, so "VTEncoderXPCService" is
/// `vt encoder xpc service` and never matches "code".
fn tokens(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current).to_lowercase());
            }
            continue;
        }
        if let Some(&previous) = (i > 0).then(|| &chars[i - 1]) {
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            let boundary = (previous.is_lowercase() && c.is_uppercase())
                || (previous.is_uppercase() && c.is_uppercase() && next_lower);
            if boundary && !current.is_empty() {
                out.push(std::mem::take(&mut current).to_lowercase());
            }
        }
        current.push(c);
    }
    if !current.is_empty() {
        out.push(current.to_lowercase());
    }
    out
}

pub fn category(name: &str) -> (&'static str, &'static str) {
    let n = name.to_lowercase();
    let words = tokens(name);
    let has = |list: &[&str]| {
        list.contains(&n.trim_end_matches(".exe"))
            || words.iter().any(|w| list.contains(&w.as_str()))
    };
    if n == "jitterbug-worker" || n == "jitterbug-worker.exe" {
        ("test", "Opt-in Jitterbug verification fixture: bounded memory allocation and a short CPU pulse.")
    } else if n.contains("jitterbug") {
        (
            "jitterbug",
            "Jitterbug's own desktop application and resource monitoring.",
        )
    } else if n.contains("ollama") || n.contains("llama") || has(&["lmstudio", "mlx"]) {
        (
            "model",
            "Local model runtime. Loaded model weights and inference can use memory and compute.",
        )
    } else if has(&[
        "rustc", "cargo", "clang", "ninja", "gcc", "cc1", "cc1plus", "cmake", "make", "esbuild",
        "swiftc",
    ]) || n == "swift-frontend"
    {
        (
            "build",
            "Compilation or build tooling. High CPU can be expected during a build.",
        )
    } else if n.starts_with("python") || words.first().is_some_and(|w| w.starts_with("python")) {
        (
            "python",
            "Python workload. Its executable alone does not identify the script or agent task.",
        )
    } else if has(&["node", "bun", "deno"]) {
        (
            "javascript",
            "JavaScript runtime, commonly a development server, build tool, or agent helper.",
        )
    } else if has(&[
        "claude", "codex", "gemini", "aider", "opencode", "goose", "chatgpt", "hermes",
    ]) || matches!(n.trim_end_matches(".exe"), "cursor-agent" | "amp")
    {
        (
            "agent",
            "AI assistant or coding agent. One agent can run several commands, servers and tools.",
        )
    } else if has(&[
        "terminal",
        "iterm",
        "iterm2",
        "ghostty",
        "wezterm",
        "alacritty",
        "kitty",
        "warp",
    ]) {
        (
            "terminal",
            "Terminal application. Its shells can run servers, builds and coding agents.",
        )
    } else if has(&[
        "code",
        "cursor",
        "zed",
        "xcode",
        "windsurf",
        "sublime",
        "vscodium",
        "pycharm",
        "webstorm",
        "rustrover",
        "goland",
        "clion",
        "intellij",
        "nvim",
        "vim",
        "emacs",
        "bbedit",
    ]) {
        (
            "editor",
            "Code editor. Extensions, language servers and terminals run as helper processes.",
        )
    } else if n.contains("webkit")
        || has(&[
            "safari",
            "chrome",
            "chromium",
            "firefox",
            "brave",
            "arc",
            "edge",
            "opera",
            "vivaldi",
            "orion",
            "duckduckgo",
            "browser",
        ])
    {
        ("browser", "Browser or embedded web content. Helper processes may belong to different tabs or apps.")
    } else {
        ("other", "Application or operating-system process. Purpose is not established from its name alone.")
    }
}

/// macOS-owned locations. User-facing apps in /System/Applications are not "system".
fn system_path(path: &Path) -> bool {
    [
        "/System/Library",
        "/usr/libexec",
        "/usr/sbin",
        "/sbin",
        "/Library/Apple",
        // Rapid Security Response cryptexes and Xcode's simulator services.
        "/System/Volumes/Preboot/Cryptexes/OS/System/Library",
        "/System/Cryptexes/App/usr/libexec",
        "/System/Cryptexes/OS/System/Library",
        "/Library/Developer/PrivateFrameworks",
    ]
    .iter()
    .any(|root| path.starts_with(root))
}

fn app_category(label: &str, bundle: &Path) -> (&'static str, &'static str) {
    let (category, description) = category(label);
    if category != "other" {
        return (category, description);
    }
    let finder = bundle.file_stem().is_some_and(|s| s == "Finder");
    if system_path(bundle) && !finder {
        ("system", SYSTEM_DESCRIPTION)
    } else {
        ("application", APPLICATION_DESCRIPTION)
    }
}

fn project_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .take(4)
        .find(|p| {
            [
                ".git",
                "package.json",
                "Cargo.toml",
                "pyproject.toml",
                "go.mod",
            ]
            .iter()
            .any(|m| p.join(m).exists())
        })
        .map(Path::to_path_buf)
}

fn opaque_id(value: &str) -> String {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    format!("w-{:x}", h.finish())
}

pub fn application_bundle(executable: &Path) -> Option<&Path> {
    // Helpers are often nested .app bundles. Attribute them to the outer application.
    executable
        .ancestors()
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
        .last()
}

/// An app a person installed or opened, not a runtime shim such as Python.app inside
/// Python.framework.
fn user_app(executable: &Path) -> Option<&Path> {
    application_bundle(executable).filter(|bundle| {
        !bundle
            .ancestors()
            .skip(1)
            .any(|p| p.extension().is_some_and(|e| e == "framework"))
    })
}

fn bundle_name(bundle: &Path) -> String {
    bundle
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .take(60)
        .collect()
}

fn process_name(p: &Process) -> String {
    p.name().to_string_lossy().chars().take(40).collect()
}

/// The executable the bundle runs when opened (directly in its Contents/MacOS).
fn main_executable(executable: &Path) -> bool {
    application_bundle(executable)
        .is_some_and(|app| executable.parent() == Some(app.join("Contents/MacOS").as_path()))
}

/// A command-line coding agent and the name people know it by. An app's main executable
/// (Claude.app, Codex.app) is the app, not a CLI agent; an agent CLI shipped inside an app
/// (a bundled `codex`) or in its own `claude.app` wrapper is still the agent.
fn agent_of(p: &Process) -> Option<&'static str> {
    let exe = p.exe();
    let name = p.name().to_string_lossy();
    if let Some(executable) = exe {
        // `claude.app` running `claude` wraps the CLI; `Claude.app` is the desktop app.
        let wrapper = application_bundle(executable).is_some_and(|app| {
            let stem = bundle_name(app);
            crate::agent_sessions::agent_key(&stem, None).is_some_and(|key| key == stem)
        });
        if main_executable(executable) && !wrapper {
            return None;
        }
    }
    if name.to_ascii_lowercase().starts_with("python") && crate::origins::hermes_location(exe) {
        return Some("Hermes");
    }
    crate::agent_sessions::agent_key(&name, exe).map(crate::agent_sessions::agent_display)
}

/// Shells and pass-through launchers say nothing about what started a command.
fn pass_through(p: &Process) -> bool {
    let name = p.name().to_string_lossy().to_ascii_lowercase();
    matches!(
        name.trim_start_matches('-'),
        "zsh"
            | "bash"
            | "sh"
            | "fish"
            | "dash"
            | "tcsh"
            | "csh"
            | "nu"
            | "login"
            | "sudo"
            | "env"
            | "nohup"
            | "timeout"
            | "xargs"
            | "script"
            | "jitterbug-run"
    ) || name.starts_with("itermserver")
}

fn launchd(p: &Process) -> bool {
    p.pid().as_u32() == 1
}

/// Where an agent is running: the app hosting it, or a non-shell host such as tmux.
fn host_of(chain: &[&Process], agent: usize) -> Option<String> {
    let mut fallback = None;
    for ancestor in chain.iter().skip(agent + 1) {
        if let Some(app) = ancestor.exe().and_then(user_app) {
            return Some(bundle_name(app));
        }
        if launchd(ancestor) {
            break;
        }
        if fallback.is_none() {
            fallback = match process_name(ancestor).as_str() {
                "sshd" | "sshd-session" => Some("an SSH session".to_string()),
                name @ ("tmux" | "screen" | "zellij" | "mosh-server") => Some(name.to_string()),
                _ => None,
            };
        }
    }
    fallback
}

fn launchd_origin(subject: &Process) -> String {
    let Some(exe) = subject.exe() else {
        return "Started by launchd".into();
    };
    match application_bundle(exe) {
        Some(app) if system_path(app) => "Started by macOS".into(),
        // The main executable lives directly in Contents/MacOS of the outer bundle.
        Some(app) if exe.parent() == Some(&app.join("Contents/MacOS")) => {
            "Opened by you or at login".into()
        }
        Some(app) => format!("Started by macOS for {}", bundle_name(app)),
        None if system_path(exe) => "Started by macOS".into(),
        None => "Started in the background by launchd".into(),
    }
}

/// What started `chain[index]`, from its ancestors. A coding agent anywhere above wins; a
/// runtime between it and an app is named ("Started by node in iTerm").
fn started_by(chain: &[&Process], agents: &[Option<&'static str>], index: usize) -> Option<String> {
    let subject = chain.get(index)?;
    match subject.pid().as_u32() {
        0 => return Some("Part of the macOS kernel".into()),
        1 => return Some("The first process macOS starts at boot".into()),
        _ => (),
    }
    let subject_is_agent = agents[index].is_some();
    let mut runtime: Option<String> = None;
    for (j, ancestor) in chain.iter().enumerate().skip(index + 1) {
        if let Some(agent) = agents[j] {
            let host = host_of(chain, j)
                .map(|h| format!(" in {h}"))
                .unwrap_or_default();
            return Some(format!("Started by {agent}{host}"));
        }
        if let Some(app) = ancestor.exe().and_then(user_app) {
            let app = bundle_name(app);
            return Some(match (&runtime, subject_is_agent) {
                (None, true) => format!("Started in {app}"),
                (None, false) => format!("Started by {app}"),
                (Some(runtime), _) => format!("Started by {runtime} in {app}"),
            });
        }
        if launchd(ancestor) {
            return Some(match runtime {
                Some(runtime) => format!("Started by {runtime}"),
                None => launchd_origin(subject),
            });
        }
        if runtime.is_none() && !pass_through(ancestor) {
            runtime = Some(process_name(ancestor));
        }
    }
    runtime.map(|r| format!("Started by {r}"))
}

impl Attribution {
    fn agent(&mut self, p: &Process) -> Option<&'static str> {
        if self.agents.len() > 8192 {
            self.agents.clear();
        }
        *self
            .agents
            .entry((p.pid().as_u32(), p.start_time()))
            .or_insert_with(|| agent_of(p))
    }

    fn project(&mut self, cwd: &Path) -> Option<PathBuf> {
        if self.projects.len() > 512 {
            self.projects.clear();
        }
        self.projects
            .entry(cwd.to_path_buf())
            .or_insert_with(|| project_root(cwd))
            .clone()
    }

    /// The process and its readable ancestors, nearest first. Stops at reused or unreadable
    /// parents: a parent that started after its child is not its launcher.
    fn chain<'a>(&self, p: &'a Process, system: &'a System) -> Vec<&'a Process> {
        let mut chain = vec![p];
        let mut ancestor = p;
        for _ in 0..16 {
            let Some(parent) = ancestor
                .parent()
                .or_else(|| {
                    self.bridges
                        .get(&ancestor.pid().as_u32())
                        .map(|id| Pid::from_u32(*id))
                })
                .and_then(|id| system.process(id))
            else {
                break;
            };
            // A start time of zero is unreadable (another user's process), not "older".
            if parent.pid() == ancestor.pid()
                || (ancestor.start_time() != 0 && parent.start_time() > ancestor.start_time())
            {
                break;
            }
            chain.push(parent);
            ancestor = parent;
        }
        chain
    }

    pub fn label(&mut self, p: &Process, system: &System) -> Label {
        let name = p.name().to_string_lossy();
        let (cat, description) = category(&name);
        let chain = self.chain(p, system);
        let agents: Vec<_> = chain.iter().map(|a| self.agent(a)).collect();
        let origin = started_by(&chain, &agents, 0);
        // A command-line agent and everything it started form one workload, unless the
        // process belongs to another app (an app's helpers stay with their app, even when an
        // agent opened it). An agent bundled in an app keeps that app's runtimes it started.
        let own_app = p.exe().and_then(user_app);
        if let Some((index, agent)) = chain
            .iter()
            .enumerate()
            .find_map(|(i, _)| agents[i].map(|agent| (i, agent)))
            .filter(|(i, _)| {
                own_app.is_none() || *i == 0 || chain[*i].exe().and_then(user_app) == own_app
            })
        {
            let root = chain[index];
            // The agent's own folder, else this command's (an app-hosted agent server).
            let project = root
                .cwd()
                .and_then(|cwd| self.project(cwd))
                .or_else(|| p.cwd().and_then(|cwd| self.project(cwd)));
            let folder: Option<String> = project
                .as_ref()
                .and_then(|r| r.file_name())
                .map(|f| f.to_string_lossy().chars().take(60).collect());
            let host = host_of(&chain, index)
                .map(|h| format!(" in {h}"))
                .unwrap_or_default();
            let started = format!("Started by {agent}{host}");
            return Label {
                    id: opaque_id(&format!(
                        "agent:{agent}:{}",
                        project.as_deref().unwrap_or(Path::new("")).display()
                    )),
                    label: folder.map_or_else(|| agent.to_string(), |f| format!("{agent} · {f}")),
                    category: "agent".into(),
                    description: format!("{agent} and the commands, servers and tools it started. Ancestry shows who started them, not which task each serves."),
                    evidence: vec![
                        format!("{agent} is this process or one of its ancestors."),
                        format!("Executable category: {cat}."),
                    ],
                    confidence: started.clone(),
                    origin,
                    group_origin: Some(started),
                    group_root: index == 0,
                };
        }
        let project = p.cwd().and_then(|cwd| self.project(cwd));
        if let Some(root) =
            project.filter(|_| matches!(cat, "build" | "python" | "javascript" | "test"))
        {
            let label = root
                .file_name()
                .map(|x| x.to_string_lossy().chars().take(60).collect())
                .unwrap_or_else(|| "Development project".into());
            return Label { id: opaque_id(&format!("project:{}", root.display())), label, category: "project".into(), description: "Development processes sharing a project working directory. This does not prove they belong to the same agent task.".into(), evidence: vec!["Working directory contains a project manifest or .git directory.".into(), format!("Executable category: {cat}.")], confidence: "Project-directory match".into(), group_origin: origin.clone(), origin, group_root: false };
        }
        // An app bundle is stronger evidence than a shared runtime name.
        if let Some((index, app)) = chain
            .iter()
            .take(8)
            .enumerate()
            .find_map(|(i, a)| a.exe().and_then(application_bundle).map(|app| (i, app)))
        {
            let label = bundle_name(app);
            let (app_category, app_description) = app_category(&label, app);
            // The app's root: the highest ancestor still inside the same bundle.
            let root = chain
                .iter()
                .enumerate()
                .skip(index)
                .take_while(|(_, a)| a.exe().and_then(application_bundle) == Some(app))
                .last()
                .map_or(index, |(i, _)| i);
            return Label {
                id: opaque_id(&format!("app:{}", app.display())),
                label,
                category: app_category.into(),
                description: app_description.into(),
                evidence: vec!["Executable or ancestor belongs to this application bundle.".into()],
                confidence: "Application ancestry".into(),
                origin,
                group_origin: started_by(&chain, &agents, root),
                group_root: p.exe().is_some_and(main_executable),
            };
        }
        // WebKit's XPC services can have launchd as their parent. The kernel's
        // responsibility relationship distinguishes Safari from other WebKit clients.
        // Never group all WebKit processes by name or assume the only browser owns them.
        if name.starts_with("com.apple.WebKit.") && crate::platform::same_user(p) {
            if let Some(owner) = crate::platform::responsible_pid(p.pid().as_u32())
                .and_then(|pid| system.process(Pid::from_u32(pid)))
                .filter(|owner| {
                    owner.start_time() <= p.start_time() && owner.user_id() == p.user_id()
                })
            {
                if let Some(app) = owner.exe().and_then(|exe| application_bundle(exe)) {
                    let label = bundle_name(app);
                    let (category, description) = app_category(&label, app);
                    let started = format!("Started by macOS for {label}");
                    return Label {
                        id: opaque_id(&format!("app:{}", app.display())), label,
                        category: category.into(), description: description.into(),
                        evidence: vec!["macOS reports this application as responsible for the WebKit helper. This does not identify a tab.".into()],
                        confidence: "macOS responsible application".into(),
                        origin: Some(started),
                        group_origin: None,
                        group_root: false,
                    };
                }
            }
        }
        let system_process = p.exe().is_some_and(system_path) || {
            #[cfg(unix)]
            {
                p.exe().is_none() && p.user_id().is_some_and(|uid| **uid == 0)
            }
            #[cfg(not(unix))]
            {
                false
            }
        };
        let (cat, description) = if cat == "other" && system_process {
            ("system", SYSTEM_DESCRIPTION)
        } else {
            (cat, description)
        };
        Label {
            id: format!("p-{}-{}", p.pid(), p.start_time()),
            label: name.chars().take(60).collect(),
            category: cat.into(),
            description: description.into(),
            evidence: vec!["Executable name only; task ownership is unknown.".into()],
            confidence: "Name-based inference".into(),
            group_origin: origin.clone(),
            origin,
            group_root: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_is_not_a_task_claim() {
        assert!(category("python3").1.contains("does not identify"));
        assert_eq!(category("ollama").0, "model");
    }
    #[test]
    fn categories_match_words_not_substrings() {
        assert_eq!(
            tokens("VTEncoderXPCService"),
            ["vt", "encoder", "xpc", "service"]
        );
        assert_eq!(category("VTEncoderXPCService").0, "other");
        assert_eq!(category("Code Helper (Renderer)").0, "editor");
        assert_eq!(category("Visual Studio Code").0, "editor");
        assert_eq!(category("Xcode").0, "editor");
        assert_eq!(category("Codex").0, "agent");
        assert_eq!(category("claude").0, "agent");
        assert_eq!(category("cursor-agent").0, "agent");
        assert_eq!(category("Cursor").0, "editor");
        assert_eq!(category("nodemon-like").0, "other");
        assert_eq!(category("node").0, "javascript");
        assert_eq!(category("python3.14").0, "python");
        assert_eq!(category("Google Chrome Helper (GPU)").0, "browser");
        assert_eq!(category("com.apple.WebKit.WebContent").0, "browser");
        assert_eq!(category("iTerm2").0, "terminal");
        assert_eq!(category("Example Encoder").0, "other");
    }
    #[test]
    fn installed_apps_are_applications_and_macos_components_are_system() {
        assert_eq!(
            app_category("Example", Path::new("/Applications/Example.app")).0,
            "application"
        );
        assert_eq!(
            app_category(
                "ExampleCenter",
                Path::new("/System/Library/CoreServices/ExampleCenter.app")
            )
            .0,
            "system"
        );
        assert_eq!(
            app_category(
                "Finder",
                Path::new("/System/Library/CoreServices/Finder.app")
            )
            .0,
            "application"
        );
        assert_eq!(
            app_category("Example", Path::new("/System/Applications/Example.app")).0,
            "application"
        );
        assert!(system_path(Path::new("/usr/libexec/exampled")));
        assert!(!system_path(Path::new("/usr/bin/example")));
    }
    #[test]
    fn framework_runtime_shims_are_not_user_apps() {
        let python = Path::new(
            "/Library/Frameworks/Example.framework/Versions/3/Resources/Example.app/Contents/MacOS/Example",
        );
        assert!(application_bundle(python).is_some());
        assert!(user_app(python).is_none());
        assert!(user_app(Path::new(
            "/Applications/Example.app/Contents/MacOS/Example"
        ))
        .is_some());
    }
    #[test]
    fn project_identifiers_do_not_contain_paths() {
        assert!(!opaque_id("/private/example/project").contains("private"));
    }
    #[test]
    fn nested_helpers_share_the_outer_application() {
        let executable = Path::new(
            "/Applications/Example.app/Contents/Frameworks/Helper.app/Contents/MacOS/helper",
        );
        assert_eq!(
            application_bundle(executable),
            Some(Path::new("/Applications/Example.app"))
        );
    }

    /// Live, same-user process tree: an agent-named launcher and its child form one agent
    /// workload with an origin sentence, separate from whatever started the launcher.
    #[test]
    #[ignore = "helper process for agent_descendants_get_their_own_workload_and_origin"]
    fn synthetic_agent_spawns_a_child() {
        if std::env::var_os("JITTERBUG_SYNTHETIC_AGENT").is_some() {
            let _ = std::process::Command::new("/bin/sleep").arg("30").status();
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn agent_descendants_get_their_own_workload_and_origin() {
        let directory = std::env::temp_dir().join(format!("jitterbug-attr-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        // This test binary, under an agent's executable name, stands in for the agent CLI
        // (copies of signed system binaries are not allowed to run).
        let agent = directory.join("codex");
        let _ = std::fs::remove_file(&agent);
        let exe = std::env::current_exe().unwrap();
        std::fs::hard_link(&exe, &agent)
            .or_else(|_| std::fs::copy(&exe, &agent).map(|_| ()))
            .unwrap();
        let mut launcher = std::process::Command::new(&agent)
            .args([
                "--exact",
                "attribution::tests::synthetic_agent_spawns_a_child",
                "--ignored",
                "--test-threads=1",
            ])
            .env("JITTERBUG_SYNTHETIC_AGENT", "1")
            .current_dir(&directory)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let root = Pid::from_u32(launcher.id());
        let mut system = System::new();
        let mut child = None;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            system.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::All,
                true,
                sysinfo::ProcessRefreshKind::nothing()
                    .with_exe(sysinfo::UpdateKind::Always)
                    .with_cwd(sysinfo::UpdateKind::Always),
            );
            child = system
                .processes()
                .values()
                .find(|p| p.parent() == Some(root))
                .map(|p| p.pid());
            if child.is_some() {
                break;
            }
        }
        let mut attribution = Attribution::default();
        let child = system.process(child.unwrap()).unwrap();
        let label = attribution.label(child, &system);
        child.kill();
        let _ = launcher.kill();
        let _ = launcher.wait();
        let _ = std::fs::remove_dir_all(&directory);
        assert_eq!(label.category, "agent");
        assert_eq!(label.label, "Codex");
        assert!(label.origin.unwrap().starts_with("Started by Codex"));
        assert!(label.group_origin.unwrap().starts_with("Started by Codex"));
        assert!(!label.id.contains("jitterbug-attr"));
    }
}
