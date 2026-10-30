//! On-demand launch attribution. Raw argv is inspected locally, then discarded.
use crate::{details, types::*};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, path::Path, sync::Mutex};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Launch {
    pub program: String,
    pub task: String,
    pub summary: String,
    pub launcher: String,
    pub evidence: Vec<String>,
    pub chain: Vec<Step>,
    pub limitation: String,
    pub reference_url: Option<String>,
    // Only recognized, fixed vocabulary crosses the model boundary.
    #[serde(skip)]
    known_role: Option<String>,
    #[serde(skip)]
    known_launchers: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Step {
    pub pid: u32,
    pub role: String,
}

#[derive(Clone)]
struct Command {
    program: String,
    task: String,
    summary: String,
    evidence: Vec<String>,
    known_role: Option<String>,
}

struct Cached {
    pid: u32,
    start: u64,
    executable: Option<std::path::PathBuf>,
    cwd: Option<std::path::PathBuf>,
    expires: u64,
    value: Option<Command>,
}
static CACHE: Mutex<VecDeque<Cached>> = Mutex::new(VecDeque::new());

fn hermes_location(path: Option<&Path>) -> bool {
    path.is_some_and(|path| path.components().any(|p| p.as_os_str() == ".hermes"))
}

fn display_name(value: &str) -> Option<String> {
    let value = value.rsplit(['/', '\\']).next()?;
    (!value.is_empty() && value.len() <= 120 && !value.chars().any(char::is_control))
        .then(|| crate::harness::excerpt(value))
}

/// Read only entrypoint syntax. Never execute code, collect option values, or open a script.
fn classify(exe: Option<&Path>, cwd: Option<&Path>, args: &[String]) -> Option<Command> {
    let binary = exe?.file_name()?.to_string_lossy().to_lowercase();
    let python = binary.starts_with("python");
    let javascript = matches!(binary.as_str(), "node" | "node.exe" | "bun" | "deno");
    if !python && !javascript {
        return None;
    }
    let code_index = args.iter().position(|v| v == "-c");
    let code = code_index.and_then(|i| args.get(i + 1));
    let module = args.windows(2).find_map(|pair| {
        matches!(pair[0].as_str(), "-m" | "--run-module").then_some(pair[1].as_str())
    });
    let hermes = python
        && (hermes_location(exe) || hermes_location(cwd))
        && (module.is_some_and(|m| m.starts_with("hermes_cli."))
            || code
                .is_some_and(|c| c.contains("from hermes_cli") || c.contains("import hermes_cli")));
    if hermes {
        // Ignore inline source text when reading the actual CLI subcommand.
        let tail = code_index.map_or(args, |i| &args[(i + 2).min(args.len())..]);
        let (task, summary, role, command) = if module == Some("hermes_cli.stderr_timestamp") {
            (
                "Gateway log wrapper",
                "This Hermes launcher starts its child command and adds timestamps to its logs.",
                "hermes_log_wrapper",
                "hermes_cli.stderr_timestamp",
            )
        } else if tail.windows(2).any(|p| p == ["gateway", "run"]) {
            ("Background gateway", "Hermes is running its background gateway to handle messaging connections and agent requests. It can stay open while no request is active.", "hermes_gateway", "hermes gateway run")
        } else {
            (
                "Agent command",
                "This process is running the Hermes agent command-line entrypoint.",
                "hermes_cli",
                "hermes_cli",
            )
        };
        return Some(Command {
            program: "Hermes".into(),
            task: task.into(),
            summary: summary.into(),
            evidence: vec![
                format!("Running command: {command}"),
                "Hermes entrypoint matches its installation or working folder.".into(),
            ],
            known_role: Some(role.into()),
        });
    }
    let (task, evidence) = if let Some(module) = module.filter(|m| {
        !m.is_empty()
            && m.len() <= 120
            && m.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._".contains(c))
    }) {
        (
            format!("Module {module}"),
            format!("Python module entrypoint: {module}"),
        )
    } else if code.is_some() || args.iter().any(|v| matches!(v.as_str(), "-e" | "--eval")) {
        (
            "Inline script".into(),
            "The runtime was launched with inline code; its contents stay private.".into(),
        )
    } else {
        let entry = args.iter().skip(1).find(|arg| !arg.starts_with('-'))?;
        let extension = Path::new(entry).extension()?.to_str()?;
        if !(python && extension == "py"
            || javascript && ["js", "cjs", "mjs", "ts"].contains(&extension))
        {
            return None;
        }
        let name = display_name(entry)?;
        (name.clone(), format!("Script entrypoint: {name}"))
    };
    let program = if hermes_location(exe) && python {
        "Hermes Python environment"
    } else if python {
        "Python"
    } else {
        "JavaScript"
    };
    Some(Command {
        program: program.into(), summary: format!("{program} is running {task}. The entrypoint is identified; its current assignment is not recorded in the command."),
        task, evidence: vec![evidence], known_role: None,
    })
}

fn read_command(process: &ProcessRecord) -> Option<Command> {
    if process.pid <= 1 || process.start_time == 0 {
        return None;
    }
    let mut cache = CACHE.lock().ok()?;
    cache.retain(|c| c.expires > now());
    if let Some(c) = cache.iter().find(|c| {
        c.pid == process.pid
            && c.start == process.start_time
            && c.executable == process.executable
            && c.cwd == process.working_directory
    }) {
        return c.value.clone();
    }
    let pid = Pid::from_u32(process.pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::Always)
            .with_cwd(UpdateKind::Always)
            .with_user(UpdateKind::Always),
    );
    let p = system.process(pid)?;
    if p.start_time() != process.start_time
        || p.exe() != process.executable.as_deref()
        || !crate::platform::same_user(p)
    {
        return None;
    }
    // Bound work on argv; no raw command or option values survive this function.
    let value =
        if p.cmd().len() <= 64 && p.cmd().iter().map(|v| v.len()).sum::<usize>() <= 128 * 1024 {
            let args: Vec<_> = p
                .cmd()
                .iter()
                .map(|v| v.to_string_lossy().into_owned())
                .collect();
            classify(p.exe(), p.cwd(), &args)
        } else {
            None
        };
    if cache.len() >= 128 {
        cache.pop_front();
    }
    cache.push_back(Cached {
        pid: process.pid,
        start: process.start_time,
        executable: process.executable.clone(),
        cwd: process.working_directory.clone(),
        expires: now() + 30,
        value: value.clone(),
    });
    value
}

pub fn inspect(snapshot: &Snapshot, process: &ProcessRecord) -> Option<Launch> {
    if snapshot.source != "live" || snapshot.captured_at + 20 < now() {
        return None;
    }
    let command = read_command(process)?;
    let (ancestry, _) = details::ancestry(snapshot, process);
    let known_launchers = ancestry
        .iter()
        .filter(|node| node.pid != process.pid)
        .filter_map(|node| match node.name.as_str() {
            "osascript" => Some("macos_script_launcher".into()),
            "launchd" => Some("macos_process_manager".into()),
            _ => snapshot
                .processes
                .iter()
                .find(|p| p.pid == node.pid && p.start_time == node.start_time)
                .and_then(read_command)
                .and_then(|c| c.known_role),
        })
        .collect();
    let chain: Vec<_> = ancestry
        .iter()
        .map(|node| {
            let role = if node.pid == process.pid {
                command.task.clone()
            } else {
                snapshot
                    .processes
                    .iter()
                    .find(|p| p.pid == node.pid && p.start_time == node.start_time)
                    .and_then(read_command)
                    .map(|c| format!("{} · {}", c.program, c.task))
                    .unwrap_or_else(|| match node.name.as_str() {
                        "osascript" => "macOS script launcher".into(),
                        "launchd" => "macOS process manager".into(),
                        _ => node
                            .executable
                            .as_deref()
                            .map(Path::new)
                            .and_then(crate::attribution::application_bundle)
                            .and_then(|p| p.file_stem())
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| node.name.clone()),
                    })
            };
            Step {
                pid: node.pid,
                role,
            }
        })
        .collect();
    let launcher = chain
        .iter()
        .rev()
        .nth(1)
        .map(|s| s.role.clone())
        .unwrap_or_else(|| "Parent no longer available".into());
    let reference_url = command
        .known_role
        .as_deref()
        .filter(|r| r.starts_with("hermes_"))
        .map(|_| "https://hermes-agent.nousresearch.com/docs/user-guide/messaging".into());
    Some(Launch { program: command.program, task: command.task, summary: command.summary,
        launcher, evidence: command.evidence, chain, known_role: command.known_role, known_launchers, reference_url,
        limitation: "The command and current parent chain identify this launch. They do not establish who clicked Start, a past user request, or the current message being handled.".into(),
    })
}

impl Launch {
    pub fn insight(&self) -> Option<crate::providers::Insight> {
        let (headline, purpose, next) = match self.known_role.as_deref()? {
            "hermes_gateway" => ("Hermes’s background messaging gateway", "It keeps Hermes messaging connections and agent requests available in the background.", "Leave it running if you use Hermes messaging. Stopping it interrupts those connections."),
            "hermes_log_wrapper" => ("Hermes’s gateway launcher and log wrapper", "It starts the Hermes child command and adds timestamps to its output.", "This supports the gateway. Stopping the wrapper can interrupt its child service."),
            "hermes_cli" => ("The Hermes agent is running this command", "The command enters Hermes through its Python launcher.", "The entrypoint is known; this command does not identify a particular user assignment."),
            _ => return None,
        };
        Some(crate::providers::Insight {
            headline: headline.into(),
            finding: format!("{purpose} Its observed parent is {}.", self.launcher),
            next_step: next.into(),
        })
    }

    pub fn model_context(&self) -> serde_json::Value {
        // No free-form script/module names, arguments, PIDs, paths, or raw ancestry.
        serde_json::json!({"recognized_role": self.known_role, "observed_ancestor_roles": self.known_launchers, "basis": "fresh process identity and locally parsed command; current ancestors are not a historical launch log; no task transcript inspected"})
    }
}

pub fn origin_question(question: &str) -> bool {
    let q = question.to_lowercase();
    !crate::research::slowdown(&q)
        && !q
            .split(|c: char| !c.is_alphabetic())
            .any(|word| matches!(word, "cpu" | "memory" | "ram") || word.starts_with("optimiz"))
        && [
            "what program",
            "what spawned",
            "who started",
            "why is it running",
            "why is this running",
            "where did",
            "what is this process",
            "what is it doing",
        ]
        .iter()
        .any(|phrase| q.contains(phrase))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_origins_answer_directly_and_only_fixed_roles_enter_model_context() {
        let launch = Launch {
            program: "private-program".into(),
            task: "private-task".into(),
            summary: "private-summary".into(),
            launcher: "Hermes · Gateway log wrapper".into(),
            evidence: vec!["private-command".into()],
            chain: vec![Step {
                pid: 12345,
                role: "private-script".into(),
            }],
            limitation: String::new(),
            reference_url: None,
            known_role: Some("hermes_gateway".into()),
            known_launchers: vec!["hermes_log_wrapper".into()],
        };
        let answer = launch.insight().unwrap();
        assert!(answer.headline.contains("Hermes"));
        assert!(answer.finding.contains("messaging connections"));
        assert!(answer.finding.contains("Gateway log wrapper"));
        let context = launch.model_context().to_string();
        assert!(context.contains("hermes_log_wrapper"));
        assert!(!context.contains("private-"));
        assert!(!context.contains("12345"));
        assert!(origin_question(
            "What program is running this, what spawned it, and why is it running?"
        ));
        assert!(!origin_question("What is it doing with so much CPU?"));
        assert!(!origin_question("Where did this lag come from?"));
    }
    fn command(args: &[&str]) -> Option<Command> {
        classify(
            Some(Path::new("/example/.hermes/tools/bin/python3")),
            Some(Path::new("/example/.hermes")),
            &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
    }
    #[test]
    fn identifies_gateway_and_wrapper_without_exposing_arguments() {
        let c = command(&[
            "python3",
            "-I",
            "-c",
            "from hermes_cli.main import main; main()",
            "gateway",
            "run",
            "--token",
            "private-value",
        ])
        .unwrap();
        assert_eq!(c.known_role.as_deref(), Some("hermes_gateway"));
        assert!(!c.summary.contains("private-value"));
        assert!(!c.evidence.join(" ").contains("private-value"));
        assert_eq!(
            command(&[
                "python3",
                "-m",
                "hermes_cli.stderr_timestamp",
                "--",
                "hermes",
                "gateway",
                "run"
            ])
            .unwrap()
            .known_role
            .as_deref(),
            Some("hermes_log_wrapper")
        );
    }
    #[test]
    fn folder_or_inline_mentions_alone_do_not_claim_gateway_ownership() {
        assert!(command(&["python3"]).is_none());
        assert!(command(&["python3", "other.py"])
            .unwrap()
            .known_role
            .is_none());
        let c = command(&[
            "python3",
            "-c",
            "from hermes_cli import main; print('gateway run')",
        ])
        .unwrap();
        assert_eq!(c.known_role.as_deref(), Some("hermes_cli"));
        assert!(classify(
            Some(Path::new("/tmp/python3")),
            None,
            &[
                "python3".into(),
                "-m".into(),
                "hermes_cli.main".into(),
                "gateway".into(),
                "run".into()
            ]
        )
        .unwrap()
        .known_role
        .is_none());
    }
    #[test]
    fn generic_entrypoints_are_local_and_stale_processes_are_not_inspected() {
        let c = command(&[
            "python3",
            "/private/project/worker.py",
            "--token",
            "private-value",
        ])
        .unwrap();
        assert_eq!(c.task, "worker.py");
        assert!(!c.evidence.join(" ").contains("/private"));
        assert!(inspect(&Snapshot::default(), &ProcessRecord::default()).is_none());
    }
}
