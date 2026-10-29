//! Explicit task wrapper. This executable is opt-in and never launches work on its own.
use jitterbug_lib::workflows::{self, RunManifest};
use std::{ffi::OsString, path::PathBuf, process::Command};

fn run() -> Result<i32, String> {
    let mut args = std::env::args_os().skip(1);
    let mut task_id = None;
    let mut task_title = None;
    let mut agent = None;
    let mut harness = "other".to_string();
    let mut role = "agent".to_string();
    let mut worktree = None;
    let mut command: Vec<OsString> = Vec::new();
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            println!("Usage: jitterbug-run --task-id ID --task-title TITLE --agent LABEL --worktree PATH [--harness claude|codex|other] [--role agent|test|build|typecheck|lint|other] -- COMMAND [ARG ...]\n\nRuns COMMAND in PATH with inherited stdin/stdout/stderr. Registers private task metadata in Jitterbug's application-data directory; never stores command arguments or prompts. Keep Jitterbug open to measure resources. No inference, scheduling or process optimization is performed by this wrapper. JITTERBUG_AGENT_RUNS_DIR can override the private registry directory for isolated tests.");
            return Ok(0);
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
            Some("--agent") => agent = Some(value),
            Some("--harness") => harness = value,
            Some("--role") => role = value,
            _ => return Err("Unknown launcher option; use --help for usage.".into()),
        }
    }
    let executable = command.first().ok_or("Supply a command after --.")?;
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
    };
    let directory = workflows::registry_directory()?;
    workflows::register(&directory, &manifest)?;
    let result = Command::new(executable)
        .args(&command[1..])
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

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("jitterbug-run: {error}");
            std::process::exit(2);
        }
    }
}
