use super::*;
use crate::types::Snapshot;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Clone, Default, Deserialize)]
pub struct Selection {
    pub workload_id: Option<String>,
    pub task_id: Option<String>,
    pub pid: Option<u32>,
    pub start_time: Option<u64>,
    pub worktree: Option<PathBuf>,
}

#[derive(Clone, Serialize)]
pub struct Candidate {
    pub label: String,
    pub worktree: PathBuf,
    pub command: Vec<String>,
    pub memory_mib: u64,
    pub evidence: String,
    pub script: Option<String>,
}

#[derive(Serialize)]
pub struct Discovery {
    pub candidates: Vec<Candidate>,
    pub message: String,
    pub worktree: Option<PathBuf>,
}

pub fn discover(snapshot: &Snapshot, selection: &Selection) -> Result<Discovery, String> {
    let mut roots = Vec::new();
    let mut candidates = Vec::new();
    if let Some(path) = &selection.worktree {
        roots.push(
            path.canonicalize()
                .map_err(|_| "Project folder unavailable.")?,
        );
    }
    if let Some(task) = &selection.task_id {
        let workflow = snapshot
            .agent_workflows
            .iter()
            .find(|w| &w.task_id == task)
            .ok_or("Task no longer available; choose a project folder.")?;
        for agent in &workflow.agents {
            roots.push(PathBuf::from(&agent.worktree));
        }
    }
    // Read arguments only after the user opens this flow, for at most 32 selected processes.
    // Arguments are never added to normal telemetry, history, or model prompts.
    let selected: Vec<_> = snapshot
        .processes
        .iter()
        .filter(|p| {
            selection
                .workload_id
                .as_ref()
                .is_some_and(|id| &p.workload_id == id)
                && selection
                    .pid
                    .is_none_or(|pid| pid == p.pid && selection.start_time == Some(p.start_time))
        })
        .take(32)
        .collect();
    if selection.pid.is_some() && selected.is_empty() {
        return Err("Selected process changed; refresh its details before managing it.".into());
    }
    let ids: Vec<_> = selected
        .iter()
        .map(|p| sysinfo::Pid::from_u32(p.pid))
        .collect();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&ids),
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_cwd(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    for previous in selected {
        let Some(process) = system.process(sysinfo::Pid::from_u32(previous.pid)) else {
            continue;
        };
        if process.start_time() != previous.start_time {
            continue;
        }
        let Some(cwd) = process.cwd() else { continue };
        if !roots.contains(&cwd.to_path_buf()) {
            roots.push(cwd.to_path_buf());
        }
        let command: Option<Vec<String>> = process
            .cmd()
            .iter()
            .map(|v| v.to_str().map(str::to_owned))
            .collect();
        let Some(mut command) = command else { continue };
        if command.is_empty()
            || command.len() > 64
            || command.iter().map(String::len).sum::<usize>() > 16_384
        {
            continue;
        }
        if !finite_command(&command) || sensitive(&command.join(" ")) {
            continue;
        }
        if let Some(exe) = process.exe().and_then(|p| p.to_str()) {
            command[0] = exe.into();
        }
        let observed = snapshot
            .workloads
            .iter()
            .find(|w| w.id == previous.workload_id)
            .map_or(previous.memory_bytes, |w| {
                w.history
                    .iter()
                    .map(|p| p.memory_bytes)
                    .max()
                    .unwrap_or(w.memory_bytes)
            });
        candidates.push(Candidate {
            label: format!("Observed {}", previous.name), worktree: cwd.to_path_buf(), command,
            memory_mib: ((observed / 1024 / 1024).saturating_mul(3) / 2 + 256).clamp(512, 131_072),
            evidence: "Live process identity verified. Estimate uses sampled workload RSS plus headroom; shared processes may inflate it and short peaks may be missed.".into(),
            script: None,
        });
    }
    roots.sort();
    roots.dedup();
    roots.truncate(16);
    let mut projects = Vec::new();
    for root in &roots {
        for ancestor in root.ancestors().take(6) {
            if ancestor.join("package.json").is_file() || ancestor.join("Cargo.toml").is_file() {
                if !projects.contains(&ancestor.to_path_buf()) {
                    projects.push(ancestor.to_path_buf());
                }
                break;
            }
            if ancestor.join(".git").exists() {
                break;
            }
        }
    }
    for project in &projects {
        candidates.extend(project_candidates(project)?);
    }
    candidates.truncate(24);
    Ok(Discovery {
        message: if candidates.is_empty() {
            "No finite build or test command was identified. Regular apps remain observed. Choose a project folder or enter a trusted command that finishes.".into()
        } else {
            "Observed commands and project scripts are suggestions. Review the executable, arguments, and project before approving future runs.".into()
        },
        worktree: projects.first().or(roots.first()).cloned(),
        candidates,
    })
}

pub fn sensitive(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "api_key", "api-key", "token=", "--token", "password", "secret=", "bearer ", "llm_",
        "sk-ant-", "sk-proj-",
    ]
    .iter()
    .any(|s| lower.contains(s))
}

fn finite_command(command: &[String]) -> bool {
    if command.iter().any(|a| {
        matches!(
            a.as_str(),
            "--watch" | "-w" | "--watchAll" | "dev" | "serve" | "start"
        )
    }) {
        return false;
    }
    command.iter().any(|a| {
        matches!(
            a.as_str(),
            "test" | "check" | "build" | "lint" | "typecheck" | "pytest" | "--test"
        )
    })
}

fn project_candidates(root: &Path) -> Result<Vec<Candidate>, String> {
    let mut found = Vec::new();
    if let Some(bytes) = read_bounded(&root.join("package.json"), 256 * 1024)? {
        let package: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "Project package.json is invalid.")?;
        if let Some(scripts) = package.get("scripts").and_then(|v| v.as_object()) {
            let runner = if root.join("pnpm-lock.yaml").exists() {
                "pnpm"
            } else if root.join("yarn.lock").exists() {
                "yarn"
            } else if root.join("bun.lock").exists() || root.join("bun.lockb").exists() {
                "bun"
            } else {
                "npm"
            };
            for (name, value) in scripts {
                let base = name.split(':').next().unwrap_or_default();
                if ![
                    "test",
                    "build",
                    "check",
                    "lint",
                    "typecheck",
                    "type-check",
                    "verify",
                    "validate",
                ]
                .contains(&base)
                {
                    continue;
                }
                let Some(body) = value.as_str() else { continue };
                let related = [format!("pre{name}"), name.clone(), format!("post{name}")]
                    .iter()
                    .filter_map(|key| {
                        scripts
                            .get(key)
                            .and_then(|v| v.as_str())
                            .map(|v| format!("{key}: {v}"))
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if related.len() > 8192
                    || sensitive(&related)
                    || body.contains("--watch")
                    || name.contains("watch")
                    || (body.contains("vitest") && !body.contains("vitest run"))
                {
                    continue;
                }
                found.push(Candidate { label: format!("{name} · {}", root.file_name().unwrap_or_default().to_string_lossy()), worktree: root.into(),
                    command: vec![runner.into(), "run".into(), name.clone()], memory_mib: 2048,
                    evidence: "Discovered in package.json. 2 GiB is a starting estimate, not a measured peak or a hard limit.".into(), script: Some(related) });
            }
        }
    }
    if root.join("Cargo.toml").is_file() {
        for task in ["check", "test", "build"] {
            found.push(Candidate { label: format!("cargo {task}"), worktree: root.into(),
                command: vec!["cargo".into(), task.into(), "-j".into(), "2".into()], memory_mib: 2048,
                evidence: "Rust project detected. Two build workers proposed; 2 GiB is an unmeasured starting estimate. Build scripts can execute project code.".into(), script: None });
        }
    }
    Ok(found)
}

pub fn search_path(command: &[String]) -> String {
    let mut paths: Vec<_> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    if let Some(exe) = command.first().map(Path::new).filter(|p| p.is_absolute()) {
        if let Some(parent) = exe.parent() {
            paths.insert(0, parent.to_path_buf());
        }
    }
    if let Some(home) = dirs::home_dir() {
        paths.extend([
            home.join(".local/bin"),
            home.join(".cargo/bin"),
            home.join(".bun/bin"),
        ]);
    }
    #[cfg(unix)]
    paths.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    paths.retain(|p| p.is_absolute());
    paths.dedup();
    std::env::join_paths(paths)
        .unwrap_or_default()
        .to_string_lossy()
        .into()
}

pub fn resolve(name: &str, cwd: &Path, path: &str) -> Result<String, String> {
    let direct = Path::new(name);
    let candidates = if direct.is_absolute() {
        vec![direct.to_path_buf()]
    } else if direct.components().count() > 1 {
        vec![cwd.join(direct)]
    } else {
        std::env::split_paths(path).map(|p| p.join(name)).collect()
    };
    for candidate in candidates {
        #[cfg(windows)]
        let variants = [
            candidate.clone(),
            candidate.with_extension("exe"),
            candidate.with_extension("cmd"),
            candidate.with_extension("bat"),
        ];
        #[cfg(not(windows))]
        let variants = [candidate];
        for file in variants {
            if !file.is_file() {
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if file
                    .metadata()
                    .map_err(|_| "Cannot inspect executable.")?
                    .permissions()
                    .mode()
                    & 0o111
                    == 0
                {
                    continue;
                }
            }
            return file
                .to_str()
                .map(str::to_owned)
                .ok_or("Executable must be UTF-8.".into());
        }
    }
    Err("Executable was not found. Enter its absolute path; terminal-only tool managers may need an explicit executable.".into())
}
