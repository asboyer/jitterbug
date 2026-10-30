//! Explicit task wrapper. This executable is opt-in and never launches work on its own.
use jitterbug_lib::{
    harness,
    workflows::{self, RunManifest},
};
use std::{ffi::OsString, path::PathBuf, process::Command};

fn run() -> Result<i32, String> {
    if jitterbug_lib::queue::worker_entry() {
        return Ok(0);
    }
    let queue_args: Vec<_> = std::env::args_os().skip(1).collect();
    if queue_args
        .first()
        .is_some_and(|a| a == "queue" || a == "mcp")
    {
        let queue_args: Vec<String> = queue_args
            .into_iter()
            .map(|s| {
                s.into_string()
                    .map_err(|_| "Queue arguments must be UTF-8.")
            })
            .collect::<Result<_, _>>()?;
        return if queue_args[0] == "queue" {
            jitterbug_lib::queue::cli::run(&queue_args[1..])
        } else {
            jitterbug_lib::queue::mcp::serve(&queue_args[1..])
        };
    }
    let mut args = std::env::args_os().skip(1).peekable();
    if args.peek().is_some_and(|arg| arg == "control-mcp") {
        return jitterbug_lib::control::mcp::serve();
    }
    if args.peek().is_some_and(|arg| arg == "hook-settings") {
        println!(
            "{}",
            harness::hook_settings(
                &std::env::current_exe().map_err(|_| "Cannot locate launcher.")?
            )?
        );
        return Ok(0);
    }
    if args.peek().is_some_and(|arg| arg == "event") {
        // A hook never blocks an agent or injects stdout into its model context.
        if let Err(error) = event() {
            eprintln!("jitterbug-run: context skipped: {error}");
        }
        return Ok(0);
    }
    let mut capture_context = false;
    let mut description = None;
    let mut task_id = None;
    let mut task_title = None;
    let mut agent = None;
    let mut harness = "other".to_string();
    let mut role = "agent".to_string();
    let mut worktree = None;
    let mut command: Vec<OsString> = Vec::new();
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            println!("Usage: jitterbug-run --task-id ID --task-title TITLE --agent LABEL --worktree PATH [--harness claude|codex|other] [--role agent|test|build|typecheck|lint|other] [--task-description TEXT] [--capture-context] -- COMMAND [ARG ...]\n\nRuns COMMAND in PATH with inherited stdin/stdout/stderr. Registers private task metadata in Jitterbug's application-data directory; never stores command arguments or tool output. --task-description supplies a local task excerpt. --capture-context adds session-only Claude Code hooks that retain the first request and latest follow-up excerpts (600 characters each) and activity counts. No transcript is read. Use hook-settings to print compatible Claude/Codex hook JSON for manual setup. Events outside a verified wrapper are ignored. Keep Jitterbug open to measure resources. No inference, scheduling or process optimization is performed by this wrapper. JITTERBUG_AGENT_RUNS_DIR can override the private registry directory for isolated tests.");
            return Ok(0);
        }
        if flag == "--capture-context" {
            capture_context = true;
            continue;
        }
        if flag == "--" {
            command.extend(args);
            break;
        }
        let value = args
            .next()
            .ok_or("A launcher option is missing its value.")?;
        if flag == "--worktree" {
            worktree = Some(PathBuf::from(value));
            continue;
        }
        let value = value
            .into_string()
            .map_err(|_| "Task labels must be UTF-8 text.")?;
        match flag.to_str() {
            Some("--task-id") => task_id = Some(value),
            Some("--task-title") => task_title = Some(value),
            Some("--task-description") => description = Some(value),
            Some("--agent") => agent = Some(value),
            Some("--harness") => harness = value,
            Some("--role") => role = value,
            _ => return Err("Unknown launcher option; use --help for usage.".into()),
        }
    }
    let executable = command.first().ok_or("Supply a command after --.")?.clone();
    if capture_context {
        if harness != "claude"
            || !matches!(
                PathBuf::from(&executable)
                    .file_stem()
                    .and_then(|s| s.to_str()),
                Some("claude")
            )
        {
            return Err("--capture-context currently installs session-only hooks for the claude executable. For other harnesses use hook-settings and their hook setup.".into());
        }
        if command
            .iter()
            .any(|s| s == "--settings" || s.to_string_lossy().starts_with("--settings="))
        {
            return Err("Merge hook-settings into your supplied settings instead of combining --capture-context with --settings.".into());
        }
        let config = harness::hook_settings(
            &std::env::current_exe().map_err(|_| "Cannot locate launcher.")?,
        )?;
        command.splice(
            1..1,
            [
                OsString::from("--settings"),
                OsString::from(config.to_string()),
            ],
        );
    }
    let worktree = worktree
        .ok_or("--worktree is required.")?
        .canonicalize()
        .map_err(|_| "The requested worktree directory is unavailable.")?;
    if !worktree.is_dir() {
        return Err("--worktree must name a directory.".into());
    }
    let pid = std::process::id();
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    let start = system
        .process(sysinfo::Pid::from_u32(pid))
        .map(|p| p.start_time())
        .filter(|start| *start > 0)
        .ok_or("Cannot verify this wrapper's OS process identity.")?;
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut manifest = RunManifest {
        version: 1,
        run_id: format!("run-{pid:x}-{unique:x}"),
        task_id: task_id.ok_or("--task-id is required.")?,
        task_title: task_title.ok_or("--task-title is required.")?,
        agent: agent.ok_or("--agent is required.")?,
        harness,
        worktree,
        role,
        root_pid: pid,
        root_start_time: start,
        started_at: jitterbug_lib::types::now(),
        ended_at: None,
        status: "active".into(),
        exit_code: None,
        context: description
            .as_deref()
            .map(harness::Context::supplied)
            .unwrap_or_default(),
    };
    let directory = workflows::registry_directory()?;
    workflows::register(&directory, &manifest)?;
    let result = Command::new(executable)
        .args(&command[1..])
        .env("JITTERBUG_RUN_ID", &manifest.run_id)
        .env("JITTERBUG_AGENT_RUNS_DIR", &directory)
        .current_dir(&manifest.worktree)
        .status();
    let exit_code = match result {
        Ok(status) => {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                status
                    .code()
                    .unwrap_or_else(|| 128 + status.signal().unwrap_or(1))
            }
            #[cfg(not(unix))]
            {
                status.code().unwrap_or(1)
            }
        }
        Err(_) => {
            eprintln!("jitterbug-run: The requested command could not be launched.");
            127
        }
    };
    if let Err(error) = workflows::finish(&directory, &mut manifest, exit_code) {
        // Preserve the command's status even if metadata persistence fails. The collector
        // treats the vanished wrapper as interrupted; stale metadata never proves ownership.
        eprintln!("jitterbug-run: {error}");
    }
    Ok(exit_code)
}

fn event() -> Result<(), String> {
    use std::io::Read;
    let Ok(id) = std::env::var("JITTERBUG_RUN_ID") else {
        return Ok(());
    };
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(harness::MAX_EVENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read harness event.")?;
    if bytes.len() as u64 > harness::MAX_EVENT_BYTES {
        return Err("Event exceeds 64 KiB limit.".into());
    }
    let value = serde_json::from_slice(&bytes).map_err(|_| "Invalid harness event JSON.")?;
    workflows::record_event(&workflows::registry_directory()?, &id, &value)
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("jitterbug-run: {error}");
            std::process::exit(2);
        }
    }
}
