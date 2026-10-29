//! Opt-in, local-only task registration. A working directory is never proof of ownership.
//! The wrapper PID and its OS start time anchor every attributed descendant.

use crate::types::{ProcessRecord, ResourcePoint};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

const MAX_MANIFESTS: usize = 128;
const MAX_ACTIVE_RUNS: usize = 32;
const MAX_RETAINED_RUNS: usize = 64;
const MAX_WORKFLOWS: usize = 16;
const MAX_MANIFEST_BYTES: u64 = 8192;
const RETAIN_SECONDS: u64 = 24 * 60 * 60;
const ROLES: &[&str] = &["agent", "test", "build", "typecheck", "lint", "other"];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunManifest {
    pub version: u32,
    pub run_id: String,
    pub task_id: String,
    pub task_title: String,
    pub agent: String,
    pub harness: String,
    pub worktree: PathBuf,
    pub role: String,
    pub root_pid: u32,
    pub root_start_time: u64,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub status: String,
    pub exit_code: Option<i32>,
}

impl RunManifest {
    pub fn validate(&self) -> Result<(), String> {
        let bounded = |s: &str, limit| {
            !s.trim().is_empty() && s.chars().count() <= limit && !s.chars().any(char::is_control)
        };
        if self.version != 1
            || !bounded(&self.run_id, 100)
            || !self
                .run_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            || !bounded(&self.task_id, 96)
            || !bounded(&self.task_title, 160)
            || !bounded(&self.agent, 80)
            || !bounded(&self.harness, 40)
            || !ROLES.contains(&self.role.as_str())
            || !self.worktree.is_absolute()
            || self.worktree.as_os_str().len() > 2048
            || self.root_pid == 0
            || self.root_start_time == 0
            || self.started_at < self.root_start_time
            || !matches!(self.status.as_str(), "active" | "completed" | "failed")
            || self.ended_at.is_some_and(|end| end < self.started_at)
            || (self.status == "active" && (self.ended_at.is_some() || self.exit_code.is_some()))
            || (self.status != "active" && (self.ended_at.is_none() || self.exit_code.is_none()))
        {
            return Err("Invalid or oversized agent-run registration.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RoleUsage {
    pub role: String,
    pub process_count: usize,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentSession {
    pub run_id: String,
    pub agent: String,
    pub harness: String,
    pub worktree: String,
    pub role: String,
    pub status: String,
    pub root_pid: u32,
    pub root_start_time: u64,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub process_count: usize,
    pub peak_cpu_percent: f32,
    pub peak_memory_bytes: u64,
    pub peak_process_count: usize,
    pub observed_samples: u64,
    pub roles: Vec<RoleUsage>,
    pub process_ids: Vec<u32>,
    pub workload_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkflowSuggestion {
    pub id: String,
    pub title: String,
    pub recommendation: String,
    pub evidence: Vec<String>,
    pub uncertainty: String,
    pub observed_at: u64,
    pub current: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentWorkflow {
    pub task_id: String,
    pub task_title: String,
    pub status: String,
    pub active_agents: usize,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub process_count: usize,
    pub peak_cpu_percent: f32,
    pub peak_memory_bytes: u64,
    pub peak_process_count: usize,
    pub started_at: u64,
    pub last_seen_at: u64,
    pub observed_samples: u64,
    pub agents: Vec<AgentSession>,
    pub history: Vec<ResourcePoint>,
    pub suggestions: Vec<WorkflowSuggestion>,
    pub evidence: Vec<String>,
}

/// Matches Tauri's app_data_dir on every supported desktop OS. The override is useful for
/// isolated tests and an explicit CLI/app development session; it never enters cloud context.
pub fn registry_directory() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("JITTERBUG_AGENT_RUNS_DIR") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("JITTERBUG_AGENT_RUNS_DIR must be an absolute directory.".into());
        }
        return Ok(path);
    }
    dirs::data_dir()
        .map(|path| path.join("com.jitterbug.desktop").join("agent-runs"))
        .ok_or_else(|| "Cannot locate the private agent-run directory.".into())
}

fn safe_file(path: &Path, metadata: &fs::Metadata) -> bool {
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Do not consume files written by another OS account or readable by other accounts.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return false;
        }
    }
    path.extension()
        .is_some_and(|extension| extension == "json")
}

fn read_manifest(path: &Path, metadata: &fs::Metadata) -> Option<RunManifest> {
    use std::io::Read;
    if !safe_file(path, metadata) {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return None;
    }
    let manifest: RunManifest = serde_json::from_slice(&bytes).ok()?;
    manifest.validate().ok()?;
    if path.file_stem()?.to_str()? != manifest.run_id {
        return None;
    }
    Some(manifest)
}

/// Registration is bounded. Completed entries are disposable launch metadata, never logs.
/// The app retains measured peaks in memory independently of these files.
pub fn register(directory: &Path, manifest: &RunManifest) -> Result<(), String> {
    manifest.validate()?;
    fs::create_dir_all(directory).map_err(|_| "Cannot create the private agent-run directory.")?;
    if fs::symlink_metadata(directory)
        .map_err(|_| "Cannot inspect the agent-run directory.")?
        .file_type()
        .is_symlink()
    {
        return Err("The agent-run directory must not be a symbolic link.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot restrict agent-run directory permissions.")?;
    }
    // Serialize parallel launch registration so the active-run limit cannot be raced.
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(directory.join("registry.lock"))
        .map_err(|_| "Cannot lock agent-run registration.")?;
    fs2::FileExt::lock_exclusive(&lock).map_err(|_| "Cannot lock agent-run registration.")?;
    let entries: Vec<_> = fs::read_dir(directory)
        .map_err(|_| "Cannot read the agent-run directory.")?
        .take(MAX_MANIFESTS + 2)
        .filter_map(Result::ok)
        .collect();
    if entries.len() > MAX_MANIFESTS {
        return Err(
            "Agent-run directory is full; remove old registration files before launching.".into(),
        );
    }
    let mut old = Vec::new();
    let mut active = 0;
    let mut system = sysinfo::System::new();
    for entry in &entries {
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        let Some(run) = read_manifest(&path, &metadata) else {
            continue;
        };
        let pid = sysinfo::Pid::from_u32(run.root_pid);
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            true,
            sysinfo::ProcessRefreshKind::nothing(),
        );
        let live = system
            .process(pid)
            .is_some_and(|process| process.start_time() == run.root_start_time);
        if run.status == "active" && live {
            active += 1;
        } else {
            old.push((run.ended_at.unwrap_or(run.started_at), path));
        }
    }
    if active >= MAX_ACTIVE_RUNS {
        return Err("At most 32 registered agent runs may be active at once.".into());
    }
    old.sort_by_key(|(time, _)| *time);
    let remove = old.len().saturating_sub(MAX_RETAINED_RUNS - active - 1);
    for (_, path) in old.into_iter().take(remove) {
        fs::remove_file(path).map_err(|_| "Cannot prune old agent-run registration.")?;
    }
    let path = directory.join(format!("{}.json", manifest.run_id));
    if path.exists() {
        return Err("This agent-run identifier is already registered.".into());
    }
    crate::storage::write(&path, manifest)
}

pub fn finish(directory: &Path, manifest: &mut RunManifest, exit_code: i32) -> Result<(), String> {
    manifest.exit_code = Some(exit_code);
    manifest.ended_at = Some(crate::types::now());
    manifest.status = if exit_code == 0 {
        "completed"
    } else {
        "failed"
    }
    .into();
    manifest.validate()?;
    crate::storage::write(
        &directory.join(format!("{}.json", manifest.run_id)),
        manifest,
    )
}

struct CachedManifest {
    modified: Option<SystemTime>,
    size: u64,
    manifest: RunManifest,
}

pub struct WorkflowTracker {
    directory: Option<PathBuf>,
    cache: HashMap<PathBuf, CachedManifest>,
    sessions: HashMap<String, AgentSession>,
    workflows: HashMap<String, AgentWorkflow>,
    roles: HashMap<(u32, u64), (u64, String)>,
}

impl Default for WorkflowTracker {
    fn default() -> Self {
        Self::new(registry_directory().ok())
    }
}

impl WorkflowTracker {
    pub fn new(directory: Option<PathBuf>) -> Self {
        Self {
            directory,
            cache: HashMap::new(),
            sessions: HashMap::new(),
            workflows: HashMap::new(),
            roles: HashMap::new(),
        }
    }

    fn read_registry(&mut self, at: u64) -> Vec<RunManifest> {
        let mut runs = Vec::new();
        let mut seen = HashSet::new();
        let Some(directory) = &self.directory else {
            return runs;
        };
        let Ok(entries) = fs::read_dir(directory) else {
            self.cache.clear();
            return runs;
        };
        for entry in entries.take(MAX_MANIFESTS).filter_map(Result::ok) {
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !safe_file(&path, &metadata) {
                continue;
            }
            let modified = metadata.modified().ok();
            let cached = self
                .cache
                .get(&path)
                .filter(|cached| cached.modified == modified && cached.size == metadata.len());
            let manifest = match cached {
                Some(cached) => cached.manifest.clone(),
                None => {
                    let Some(manifest) = read_manifest(&path, &metadata) else {
                        continue;
                    };
                    self.cache.insert(
                        path.clone(),
                        CachedManifest {
                            modified,
                            size: metadata.len(),
                            manifest: manifest.clone(),
                        },
                    );
                    manifest
                }
            };
            seen.insert(path);
            if manifest.started_at > at.saturating_add(5)
                || manifest
                    .ended_at
                    .is_some_and(|end| at.saturating_sub(end) > RETAIN_SECONDS)
            {
                continue;
            }
            runs.push(manifest);
        }
        self.cache.retain(|path, _| seen.contains(path));
        runs.sort_by(|a, b| {
            (b.status == "active")
                .cmp(&(a.status == "active"))
                .then_with(|| b.started_at.cmp(&a.started_at))
        });
        runs.truncate(MAX_RETAINED_RUNS);
        runs
    }

    pub fn update(
        &mut self,
        processes: &mut [ProcessRecord],
        at: u64,
        sample_ready: bool,
        available_memory: u64,
        total_memory: u64,
        logical_cpus: usize,
    ) -> Vec<AgentWorkflow> {
        let runs = self.read_registry(at);
        self.classify_registered(&runs, processes, at);
        self.observe(
            &runs,
            processes,
            at,
            sample_ready,
            available_memory,
            total_memory,
            logical_cpus,
        )
    }

    fn classify_registered(
        &mut self,
        runs: &[RunManifest],
        processes: &mut [ProcessRecord],
        at: u64,
    ) {
        let inventory: HashMap<_, _> = processes.iter().map(|p| (p.pid, p)).collect();
        let roots: HashMap<_, _> = runs
            .iter()
            .filter(|run| {
                run.status == "active"
                    && inventory
                        .get(&run.root_pid)
                        .is_some_and(|p| p.start_time == run.root_start_time)
            })
            .map(|run| ((run.root_pid, run.root_start_time), run.run_id.as_str()))
            .collect();
        let owned: HashSet<_> = if roots.is_empty() {
            HashSet::new()
        } else {
            processes
                .iter()
                .filter(|p| owning_run(p, &inventory, &roots).is_some())
                .map(|p| (p.pid, p.start_time))
                .collect()
        };
        self.roles
            .retain(|id, (time, _)| owned.contains(id) && at.saturating_sub(*time) < 30);
        let pending: Vec<_> = processes
            .iter()
            .filter(|p| {
                owned.contains(&(p.pid, p.start_time))
                    && p.resource_role == "other"
                    && role_uses_arguments(&p.name.to_ascii_lowercase())
                    && !self.roles.contains_key(&(p.pid, p.start_time))
            })
            .take(256)
            .map(|p| sysinfo::Pid::from_u32(p.pid))
            .collect();
        if !pending.is_empty() {
            // Read arguments only for opt-in descendants that need tool classification. This
            // temporary System is dropped here; cache only the allowlisted role, never argv.
            let mut commands = sysinfo::System::new();
            commands.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::Some(&pending),
                true,
                sysinfo::ProcessRefreshKind::nothing().with_cmd(sysinfo::UpdateKind::Always),
            );
            for p in commands.processes().values() {
                let id = (p.pid().as_u32(), p.start_time());
                if owned.contains(&id) && self.roles.len() < 512 {
                    self.roles.insert(
                        id,
                        (
                            at,
                            process_role(&p.name().to_string_lossy(), p.cmd()).into(),
                        ),
                    );
                }
            }
        }
        for process in processes {
            if let Some((_, role)) = self.roles.get(&(process.pid, process.start_time)) {
                process.resource_role = role.clone();
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn observe(
        &mut self,
        runs: &[RunManifest],
        processes: &[ProcessRecord],
        at: u64,
        sample_ready: bool,
        available_memory: u64,
        total_memory: u64,
        logical_cpus: usize,
    ) -> Vec<AgentWorkflow> {
        let inventory: HashMap<_, _> = processes.iter().map(|p| (p.pid, p)).collect();
        let roots: HashMap<_, _> = runs
            .iter()
            .filter(|run| {
                run.status == "active"
                    && inventory.get(&run.root_pid).is_some_and(|root| {
                        root.start_time == run.root_start_time && root.start_time > 0
                    })
            })
            .map(|run| ((run.root_pid, run.root_start_time), run.run_id.as_str()))
            .collect();
        let mut members: HashMap<&str, Vec<&ProcessRecord>> = HashMap::new();
        if !roots.is_empty() {
            for process in processes {
                if let Some(id) = owning_run(process, &inventory, &roots) {
                    members.entry(id).or_default().push(process);
                }
            }
        }
        let mut groups: HashMap<String, AgentWorkflow> = HashMap::new();
        for run in runs.iter().take(MAX_RETAINED_RUNS) {
            let previous = self.sessions.get(&run.run_id);
            let live = roots.contains_key(&(run.root_pid, run.root_start_time));
            let status = if live {
                "active"
            } else if run.status == "active" {
                "interrupted"
            } else {
                run.status.as_str()
            };
            // Do not rediscover an ancient crashed wrapper forever from an abandoned file.
            if !live && at.saturating_sub(run.ended_at.unwrap_or(run.started_at)) > RETAIN_SECONDS {
                continue;
            }
            let mut session = AgentSession {
                run_id: run.run_id.clone(),
                agent: run.agent.clone(),
                harness: run.harness.clone(),
                worktree: run.worktree.to_string_lossy().into_owned(),
                role: run.role.clone(),
                status: status.into(),
                root_pid: run.root_pid,
                root_start_time: run.root_start_time,
                started_at: run.started_at,
                ended_at: run
                    .ended_at
                    .or_else(|| (!live).then(|| previous.and_then(|s| s.ended_at).unwrap_or(at))),
                exit_code: run.exit_code,
                peak_cpu_percent: previous.map_or(0.0, |s| s.peak_cpu_percent),
                peak_memory_bytes: previous.map_or(0, |s| s.peak_memory_bytes),
                peak_process_count: previous.map_or(0, |s| s.peak_process_count),
                observed_samples: previous.map_or(0, |s| s.observed_samples),
                ..Default::default()
            };
            let mut roles: BTreeMap<&str, RoleUsage> = BTreeMap::new();
            if let Some(children) = members.get(run.run_id.as_str()) {
                for process in children {
                    session.cpu_percent += process.cpu_percent;
                    session.memory_bytes =
                        session.memory_bytes.saturating_add(process.memory_bytes);
                    session.process_count += 1;
                    session.process_ids.push(process.pid);
                    if !session.workload_ids.contains(&process.workload_id) {
                        session.workload_ids.push(process.workload_id.clone());
                    }
                    let role = if process.pid == run.root_pid {
                        "other"
                    } else if ROLES.contains(&process.resource_role.as_str())
                        && process.resource_role != "other"
                    {
                        process.resource_role.as_str()
                    } else if run.role != "agent" {
                        run.role.as_str()
                    } else {
                        "other"
                    };
                    let usage = roles.entry(role).or_insert_with(|| RoleUsage {
                        role: role.into(),
                        ..Default::default()
                    });
                    usage.process_count += 1;
                    usage.cpu_percent += process.cpu_percent;
                    usage.memory_bytes = usage.memory_bytes.saturating_add(process.memory_bytes);
                }
                session.peak_memory_bytes = session.peak_memory_bytes.max(session.memory_bytes);
                session.peak_process_count = session.peak_process_count.max(session.process_count);
                if sample_ready {
                    session.observed_samples += 1;
                    session.peak_cpu_percent = session.peak_cpu_percent.max(session.cpu_percent);
                }
            }
            session.process_ids.truncate(256);
            session.workload_ids.truncate(64);
            session.roles = roles.into_values().collect();
            if !live {
                // Preserve the last measured roles for review, without stale current usage.
                session.roles = previous.map_or_else(Vec::new, |s| {
                    s.roles
                        .iter()
                        .map(|r| RoleUsage {
                            role: r.role.clone(),
                            ..Default::default()
                        })
                        .collect()
                });
            }
            self.sessions.insert(run.run_id.clone(), session.clone());
            let workflow = groups.entry(run.task_id.clone()).or_insert_with(|| {
                let old = self.workflows.get(&run.task_id);
                AgentWorkflow {
                    task_id: run.task_id.clone(),
                    task_title: run.task_title.clone(),
                    started_at: run.started_at,
                    last_seen_at: old.map_or(at, |w| w.last_seen_at),
                    peak_cpu_percent: old.map_or(0.0, |w| w.peak_cpu_percent),
                    peak_memory_bytes: old.map_or(0, |w| w.peak_memory_bytes),
                    peak_process_count: old.map_or(0, |w| w.peak_process_count),
                    observed_samples: old.map_or(0, |w| w.observed_samples),
                    history: old.map_or_else(Vec::new, |w| w.history.clone()),
                    suggestions: old.map_or_else(Vec::new, |w| w.suggestions.clone()),
                    ..Default::default()
                }
            });
            workflow.active_agents += usize::from(live);
            workflow.cpu_percent += session.cpu_percent;
            workflow.memory_bytes = workflow.memory_bytes.saturating_add(session.memory_bytes);
            workflow.process_count += session.process_count;
            workflow.started_at = workflow.started_at.min(session.started_at);
            workflow.agents.push(session);
        }
        self.sessions.retain(|id, session| {
            runs.iter().any(|run| &run.run_id == id)
                && session
                    .ended_at
                    .is_none_or(|end| at.saturating_sub(end) <= RETAIN_SECONDS)
        });
        let mut workflows: Vec<_> = groups.into_values().collect();
        for workflow in &mut workflows {
            workflow.status = if workflow.active_agents > 0 {
                "active"
            } else if workflow.agents.iter().any(|s| s.status == "failed") {
                "failed"
            } else if workflow.agents.iter().any(|s| s.status == "interrupted") {
                "interrupted"
            } else {
                "completed"
            }
            .into();
            if workflow.active_agents > 0 {
                workflow.last_seen_at = at;
                workflow.peak_memory_bytes = workflow.peak_memory_bytes.max(workflow.memory_bytes);
                workflow.peak_process_count =
                    workflow.peak_process_count.max(workflow.process_count);
                if sample_ready {
                    workflow.observed_samples += 1;
                    workflow.peak_cpu_percent = workflow.peak_cpu_percent.max(workflow.cpu_percent);
                    workflow.history.push(ResourcePoint {
                        time: at,
                        cpu_percent: workflow.cpu_percent,
                        memory_bytes: workflow.memory_bytes,
                    });
                    if workflow.history.len() > 24 {
                        workflow.history.remove(0);
                    }
                }
                let observed = suggestions(workflow, available_memory, total_memory, logical_cpus);
                for suggestion in &mut workflow.suggestions {
                    suggestion.current = false;
                }
                for mut suggestion in observed {
                    suggestion.observed_at = at;
                    suggestion.current = true;
                    if let Some(previous) = workflow
                        .suggestions
                        .iter_mut()
                        .find(|previous| previous.id == suggestion.id)
                    {
                        *previous = suggestion;
                    } else {
                        workflow.suggestions.push(suggestion);
                    }
                }
            } else {
                for suggestion in &mut workflow.suggestions {
                    suggestion.current = false;
                }
            }
            workflow.evidence = vec![
                "Opt-in launch registration, verified by current wrapper PID, OS start time, and live parent ancestry. Worktree paths and task labels are user-supplied context, not ownership evidence.".into(),
                "CPU is normalized to the whole machine. Summed resident memory can count shared pages more than once. Peaks cover observed samples, not the entire run.".into(),
                "Five-second sampling may miss short-lived commands. Detached or reparented processes lose attribution; command roles are limited tool-name heuristics, not transcript analysis.".into(),
                "Suggestions are advisory. No scheduling, configuration, or process changes have been applied.".into(),
            ];
            workflow.agents.sort_by(|a, b| {
                (b.status == "active")
                    .cmp(&(a.status == "active"))
                    .then_with(|| a.started_at.cmp(&b.started_at))
                    .then_with(|| a.run_id.cmp(&b.run_id))
            });
        }
        workflows.sort_by(|a, b| {
            (b.active_agents > 0)
                .cmp(&(a.active_agents > 0))
                .then_with(|| b.last_seen_at.cmp(&a.last_seen_at))
                .then_with(|| a.task_id.cmp(&b.task_id))
        });
        workflows.truncate(MAX_WORKFLOWS);
        self.workflows = workflows
            .iter()
            .map(|workflow| (workflow.task_id.clone(), workflow.clone()))
            .collect();
        workflows
    }
}

fn owning_run<'a>(
    process: &'a ProcessRecord,
    inventory: &HashMap<u32, &'a ProcessRecord>,
    roots: &HashMap<(u32, u64), &'a str>,
) -> Option<&'a str> {
    let mut current = process;
    let mut seen = HashSet::new();
    for _ in 0..64 {
        if current.start_time == 0 || !seen.insert(current.pid) {
            return None;
        }
        if let Some(id) = roots.get(&(current.pid, current.start_time)) {
            return Some(id);
        }
        let parent = *inventory.get(&current.parent_pid?)?;
        if parent.start_time == 0
            || parent.start_time > current.start_time
            || current.parent_start_time != Some(parent.start_time)
        {
            return None;
        }
        current = parent;
    }
    None
}

/// Inspect only a small allowlist of executable/tool tokens. Never retain command arguments.
pub fn process_role(name: &str, command: &[OsString]) -> &'static str {
    fn token_role(token: &str) -> Option<&'static str> {
        match token.trim_end_matches(".exe") {
            "claude" | "codex" | "claude-code" => Some("agent"),
            "pytest" | "pytest.py" | "vitest" | "vitest.mjs" | "jest" | "jest.js" | "mocha"
            | "playwright" | "ava" => Some("test"),
            "tsc" | "tsc.js" | "vue-tsc" | "svelte-check" | "pyright" | "mypy" => Some("typecheck"),
            "eslint" | "eslint.js" | "ruff" | "clippy-driver" => Some("lint"),
            "rustc" | "clang" | "clang++" | "gcc" | "g++" | "ninja" | "esbuild" => Some("build"),
            _ => None,
        }
    }
    let basename = |value: &str| {
        value
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(value)
            .to_ascii_lowercase()
    };
    let program = basename(name);
    if let Some(role) = token_role(&program) {
        return role;
    }
    // Only inspect arguments for known runtimes/tool launchers. An arbitrary prompt containing
    // the word "pytest" must not make an agent look like a test runner.
    let program = program.trim_end_matches(".exe");
    if !role_uses_arguments(program) {
        return "other";
    }
    for token in command.iter().skip(1).take(6) {
        let Some(token) = token.to_str().filter(|value| value.len() <= 256) else {
            continue;
        };
        if matches!(token, "-e" | "--eval" | "-p" | "--print" | "-c") {
            // The next token is arbitrary source text, not a tool name or subcommand.
            return "other";
        }
        if token == "--test" && matches!(program, "node" | "bun") {
            return "test";
        }
        if token.starts_with('-') {
            continue;
        }
        if let Some(role) = token_role(&basename(token)) {
            return role;
        }
        match token {
            "test" | "test:unit" | "test:integration" | "test:e2e" => return "test",
            "build" | "check" if program == "cargo" => return "build",
            "check" if program == "deno" => return "typecheck",
            "build" => return "build",
            "typecheck" | "type-check" | "check:types" => return "typecheck",
            "lint" | "clippy" => return "lint",
            script
                if matches!(program, "npm" | "pnpm" | "yarn" | "bun")
                    && script.starts_with("test:")
                    && script
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_')) =>
            {
                return "test"
            }
            _ => (),
        }
    }
    "other"
}

fn role_uses_arguments(program: &str) -> bool {
    matches!(
        program.trim_end_matches(".exe"),
        "node" | "bun" | "deno" | "python" | "python3" | "npm" | "npx" | "pnpm" | "yarn" | "cargo"
    )
}

fn suggestions(
    workflow: &AgentWorkflow,
    available: u64,
    total: u64,
    cores: usize,
) -> Vec<WorkflowSuggestion> {
    let active: Vec<_> = workflow
        .agents
        .iter()
        .filter(|s| s.status == "active")
        .collect();
    let mut advice = Vec::new();
    if active.len() > 1 {
        let worktrees: HashSet<_> = active.iter().map(|s| &s.worktree).collect();
        advice.push(WorkflowSuggestion {
            id: "parallel-sessions".into(),
            title: "Coordinate parallel agents before full validation".into(),
            recommendation: "Give each agent a separate area to change, let it run focused checks, and run the combined full suite once after integration. Keep independent implementation in parallel when headroom allows.".into(),
            evidence: vec![format!("{} live registered sessions for the same task across {} worktree paths.", active.len(), worktrees.len())],
            uncertainty: "Registration proves task association; it does not reveal whether edits or tests are duplicated. Review the agents' assignments before combining validation.".into(),
            ..Default::default()
        });
    }
    for role in ["test", "build", "typecheck", "lint"] {
        let sessions = active
            .iter()
            .filter(|s| {
                s.roles
                    .iter()
                    .any(|r| r.role == role && r.process_count > 0)
            })
            .count();
        if sessions > 1 {
            advice.push(WorkflowSuggestion {
                id: format!("overlapping-{role}"),
                title: format!("Stagger overlapping {role} work"),
                recommendation: format!("Use one shared validation slot for full {role} runs; keep small targeted checks inside each agent. Reuse the package manager's read-only download cache and tooling's supported caches, while keeping mutable build outputs isolated per worktree."),
                evidence: vec![format!("Processes identified as {role} tooling are running in {sessions} separate sessions in the same sample.")],
                uncertainty: "Tool roles are inferred from a small executable/argument allowlist. Overlap is observed; identical commands or duplicated coverage are not established.".into(),
                ..Default::default()
            });
        }
    }
    let validation: usize = active
        .iter()
        .flat_map(|s| &s.roles)
        .filter(|r| matches!(r.role.as_str(), "test" | "build" | "typecheck" | "lint"))
        .map(|r| r.process_count)
        .sum();
    if validation > cores.max(4) || workflow.process_count >= active.len().max(1) * 8 {
        advice.push(WorkflowSuggestion {
            id: "worker-amplification".into(),
            title: "Review combined worker limits".into(),
            recommendation: "Check the existing worker/job limits in each test runner or compiler against a machine-wide budget. One or two workers per concurrent validation run is a conservative starting point. If limits are already low and headroom is healthy, keep them and measure elapsed time before changing anything.".into(),
            evidence: vec![format!("{} attributed processes, including {validation} validation-tool processes, across {} active session(s) on {cores} logical CPUs.", workflow.process_count, active.len())],
            uncertainty: "Process count is an indicator, not a CPU-thread measurement. Idle helpers and shared memory can inflate totals; this is not proof that every process is a competing worker.".into(),
            ..Default::default()
        });
    }
    if total > 0
        && (available < 2 * 1024 * 1024 * 1024 || available < total / 10)
        && !active.is_empty()
    {
        advice.push(WorkflowSuggestion {
            id: "memory-headroom".into(),
            title: "Leave memory headroom before another agent or build".into(),
            recommendation: "Wait for one validation run to finish before starting another. Prefer cloud reasoning, avoid starting a local model, and lower test/build worker limits. Consider running the full integration suite on CI after focused local checks.".into(),
            evidence: vec![format!("{} MiB of system memory headroom; the task's attributed processes sum to {} MiB resident memory.", available / 1048576, workflow.memory_bytes / 1048576)],
            uncertainty: "Headroom is a system-wide observation; this task is not necessarily its sole cause. No process has been paused or moved to cloud compute.".into(),
            ..Default::default()
        });
    }
    advice
}

/// Cloud-safe summaries intentionally omit task IDs/titles, run IDs, PIDs, worktree paths,
/// harness/agent labels, and raw suggestions. This opt-in metadata stays on this computer.
pub fn model_context(workflows: &[AgentWorkflow]) -> serde_json::Value {
    serde_json::Value::Array(workflows.iter().filter(|w| w.active_agents > 0).take(4).enumerate().map(|(i, w)| {
        let mut roles: BTreeMap<&str, usize> = BTreeMap::new();
        for role in w.agents.iter().flat_map(|a| &a.roles) {
            *roles.entry(&role.role).or_default() += role.process_count;
        }
        serde_json::json!({
            "alias": format!("Registered workflow {}", i + 1),
            "active_sessions": w.active_agents,
            "cpu_percent": (w.cpu_percent * 10.0).round() / 10.0,
            "resident_mib": w.memory_bytes / 1048576,
            "process_count": w.process_count,
            "observed_samples": w.observed_samples,
            "roles": roles,
            "current_advisory_ids": w.suggestions.iter().filter(|s| s.current).map(|s| &s.id).collect::<Vec<_>>(),
            "ownership": "Opt-in wrapper PID/start-time and observed live ancestry; not cwd matching",
        })
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: u32, worktree: &str) -> RunManifest {
        #[cfg(target_os = "windows")]
        let worktree = PathBuf::from(r"C:\example").join(worktree.trim_start_matches('/'));
        #[cfg(not(target_os = "windows"))]
        let worktree = PathBuf::from(worktree);
        RunManifest {
            version: 1,
            run_id: format!("example-{id}"),
            task_id: "EXAMPLE-1".into(),
            task_title: "Private example title".into(),
            agent: format!("Private agent {id}"),
            harness: "example-harness".into(),
            worktree,
            role: "agent".into(),
            root_pid: id,
            root_start_time: 100,
            started_at: 100,
            ended_at: None,
            status: "active".into(),
            exit_code: None,
        }
    }

    fn process(pid: u32, parent: Option<u32>, role: &str) -> ProcessRecord {
        ProcessRecord {
            pid,
            start_time: 100,
            parent_pid: parent,
            parent_start_time: parent.map(|_| 100),
            cpu_percent: 2.0,
            memory_bytes: 1024 * 1024,
            workload_id: format!("w-{pid}"),
            resource_role: role.into(),
            ..Default::default()
        }
    }

    fn observe(
        tracker: &mut WorkflowTracker,
        runs: &[RunManifest],
        processes: &[ProcessRecord],
        at: u64,
    ) -> Vec<AgentWorkflow> {
        tracker.observe(runs, processes, at, true, 8 << 30, 16 << 30, 8)
    }

    #[test]
    fn live_ancestry_combines_parallel_worktrees_without_changing_workloads() {
        let mut tracker = WorkflowTracker::new(None);
        let runs = [
            run(10, "/example/worktree-a"),
            run(20, "/example/worktree-b"),
        ];
        let mut processes = vec![
            process(10, None, "other"),
            process(11, Some(10), "agent"),
            process(12, Some(11), "test"),
            process(20, None, "other"),
            process(21, Some(20), "agent"),
            process(22, Some(21), "test"),
            process(30, None, "test"),
        ];
        // An unrelated process in the same worktree is not owned by either agent.
        processes[6].working_directory = Some(runs[0].worktree.clone());
        let workflows = observe(&mut tracker, &runs, &processes, 110);
        assert_eq!(workflows.len(), 1);
        let workflow = &workflows[0];
        assert_eq!(workflow.active_agents, 2);
        assert_eq!(workflow.process_count, 6);
        assert_eq!(workflow.memory_bytes, 6 << 20);
        assert_eq!(workflow.cpu_percent, 12.0);
        assert_eq!(workflow.agents.len(), 2);
        assert!(workflow
            .suggestions
            .iter()
            .any(|s| s.id == "overlapping-test" && s.current));
        assert!(workflow
            .suggestions
            .iter()
            .any(|s| s.id == "parallel-sessions"));
        assert!(!workflow.agents.iter().any(|s| s.process_ids.contains(&30)));
        assert_eq!(processes[2].workload_id, "w-12");
    }

    #[test]
    fn reused_wrapper_parent_mismatch_missing_parent_and_cycles_are_not_ownership() {
        let mut tracker = WorkflowTracker::new(None);
        let run = run(10, "/example/worktree");
        let mut processes = vec![process(10, None, "other"), process(11, Some(10), "agent")];
        processes[0].start_time = 101; // The registered wrapper PID has been reused.
        assert_eq!(
            observe(&mut tracker, std::slice::from_ref(&run), &processes, 110)[0].process_count,
            0
        );
        processes[0].start_time = 100;
        processes[1].parent_start_time = Some(99); // Not the collected parent identity.
        assert_eq!(
            observe(&mut tracker, std::slice::from_ref(&run), &processes, 115)[0].process_count,
            1
        );
        processes[1].parent_pid = Some(99);
        assert_eq!(
            observe(&mut tracker, std::slice::from_ref(&run), &processes, 120)[0].process_count,
            1
        );
        processes[1].parent_pid = Some(12);
        processes[1].parent_start_time = Some(100);
        processes.push(process(12, Some(11), "test"));
        assert_eq!(
            observe(&mut tracker, std::slice::from_ref(&run), &processes, 125)[0].process_count,
            1
        );
        processes[1].parent_pid = Some(10);
        processes[1].start_time = 99; // A parent cannot start after its child.
        assert_eq!(
            observe(&mut tracker, &[run], &processes, 130)[0].process_count,
            1
        );
    }

    #[test]
    fn nearest_registered_wrapper_wins_without_double_counting_nested_sessions() {
        let mut tracker = WorkflowTracker::new(None);
        let runs = [run(10, "/example/a"), run(20, "/example/b")];
        let processes = [
            process(10, None, "other"),
            process(20, Some(10), "other"),
            process(21, Some(20), "test"),
        ];
        let workflow = observe(&mut tracker, &runs, &processes, 110).remove(0);
        assert_eq!(workflow.process_count, 3);
        assert_eq!(workflow.agents[0].process_count, 1);
        assert_eq!(workflow.agents[1].process_count, 2);
    }

    #[test]
    fn completed_runs_keep_observed_peaks_and_advice_without_stale_usage() {
        let mut tracker = WorkflowTracker::new(None);
        let mut runs = [run(10, "/example/a"), run(20, "/example/b")];
        let processes = [
            process(10, None, "other"),
            process(11, Some(10), "test"),
            process(20, None, "other"),
            process(21, Some(20), "test"),
        ];
        observe(&mut tracker, &runs, &processes, 110);
        for run in &mut runs {
            run.status = "completed".into();
            run.ended_at = Some(112);
            run.exit_code = Some(0);
        }
        let workflow = observe(&mut tracker, &runs, &processes, 115).remove(0);
        assert_eq!(workflow.status, "completed");
        assert_eq!(workflow.active_agents, 0);
        assert_eq!(workflow.cpu_percent, 0.0);
        assert_eq!(workflow.memory_bytes, 0);
        assert_eq!(workflow.process_count, 0);
        assert_eq!(workflow.peak_cpu_percent, 8.0);
        assert_eq!(workflow.peak_memory_bytes, 4 << 20);
        assert_eq!(workflow.observed_samples, 1);
        assert_eq!(workflow.last_seen_at, 110);
        assert!(!workflow.suggestions.is_empty());
        assert!(workflow
            .suggestions
            .iter()
            .all(|s| !s.current && s.observed_at == 110));
        assert_eq!(workflow.agents[0].peak_process_count, 2);
        assert_eq!(workflow.agents[0].ended_at, Some(112));
    }

    #[test]
    fn missing_wrapper_marks_interrupted_and_expired_registrations_disappear() {
        let mut tracker = WorkflowTracker::new(None);
        let runs = [run(10, "/example/a")];
        observe(&mut tracker, &runs, &[process(10, None, "other")], 110);
        let workflow = observe(&mut tracker, &runs, &[], 115).remove(0);
        assert_eq!(workflow.status, "interrupted");
        assert_eq!(workflow.agents[0].ended_at, Some(115));
        let next = observe(&mut tracker, &runs, &[], 120).remove(0);
        assert_eq!(next.agents[0].ended_at, Some(115));
        assert!(observe(&mut tracker, &runs, &[], 100 + RETAIN_SECONDS + 1).is_empty());
    }

    #[test]
    fn history_sessions_and_workflows_are_bounded() {
        let mut tracker = WorkflowTracker::new(None);
        let runs: Vec<_> = (10..110)
            .map(|id| {
                let mut registration = run(id, "/example/worktree");
                registration.task_id = format!("EXAMPLE-{id}");
                registration
            })
            .collect();
        let processes: Vec<_> = (10..110).map(|id| process(id, None, "other")).collect();
        for at in 110..160 {
            let workflows = observe(&mut tracker, &runs, &processes, at);
            assert!(workflows.len() <= MAX_WORKFLOWS);
            assert!(tracker.sessions.len() <= MAX_RETAINED_RUNS);
            assert!(workflows.iter().all(|w| w.history.len() <= 24));
        }
        assert!(tracker.workflows.values().all(|w| w.history.len() == 24));
    }

    #[test]
    fn model_summary_excludes_private_registration_and_process_identity() {
        let mut tracker = WorkflowTracker::new(None);
        let runs = [
            run(65432, "/example/private-worktree"),
            run(65433, "/example/private-worktree-b"),
        ];
        let processes = [process(65432, None, "other"), process(65433, None, "other")];
        let workflows = observe(&mut tracker, &runs, &processes, 110);
        let snapshot = crate::types::Snapshot {
            agent_workflows: workflows,
            ..Default::default()
        };
        let value = crate::providers::model_context(&snapshot, None).to_string();
        for secret in [
            "EXAMPLE-1",
            "Private example",
            "Private agent",
            "example-harness",
            "private-worktree",
            "example-65432",
            "65432",
            "65433",
        ] {
            assert!(!value.contains(secret), "Private field leaked: {secret}");
        }
        assert!(value.contains("active_sessions"));
        assert!(value.contains("parallel-sessions"));
    }

    #[test]
    fn allowlisted_roles_do_not_interpret_freeform_agent_prompts() {
        let args = |items: &[&str]| items.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            process_role("claude", &args(&["claude", "--print", "test"])),
            "agent"
        );
        assert_eq!(
            process_role("unknown", &args(&["unknown", "test"])),
            "other"
        );
        assert_eq!(
            process_role(
                "node",
                &args(&["node", "/example/node_modules/vitest/vitest.mjs"])
            ),
            "test"
        );
        assert_eq!(
            process_role("node.exe", &args(&["node.exe", "C:\\example\\tsc.js"])),
            "typecheck"
        );
        assert_eq!(process_role("cargo", &args(&["cargo", "clippy"])), "lint");
        assert_eq!(
            process_role("python3", &args(&["python3", "-m", "pytest"])),
            "test"
        );
        assert_eq!(
            process_role("node", &args(&["node", "-e", "console.log('test')"])),
            "other"
        );
        assert_eq!(
            process_role("node", &args(&["node", "-e", "test"])),
            "other"
        );
        assert_eq!(
            process_role("node", &args(&["node", "--test", "example.test.js"])),
            "test"
        );
        assert_eq!(process_role("bun", &args(&["bun", "--test"])), "test");
        assert_eq!(process_role("deno", &args(&["deno", "test"])), "test");
        assert_eq!(
            process_role("deno", &args(&["deno", "check", "example.ts"])),
            "typecheck"
        );
        assert_eq!(
            process_role("npm", &args(&["npm", "run", "test:browser"])),
            "test"
        );
        assert_eq!(
            process_role("claude", &args(&["claude", "--print", "test:browser"])),
            "agent"
        );
    }

    #[test]
    fn failed_sessions_fail_the_workflow_and_unknown_helpers_stay_other() {
        let mut tracker = WorkflowTracker::new(None);
        let mut runs = [run(10, "/example/a")];
        let processes = [process(10, None, "other"), process(11, Some(10), "other")];
        let workflow = observe(&mut tracker, &runs, &processes, 110).remove(0);
        assert_eq!(workflow.agents[0].roles.len(), 1);
        assert_eq!(workflow.agents[0].roles[0].role, "other");
        runs[0].status = "failed".into();
        runs[0].exit_code = Some(2);
        runs[0].ended_at = Some(112);
        let completed = observe(&mut tracker, &runs, &[], 115).remove(0);
        assert_eq!(completed.status, "failed");
        assert_eq!(completed.agents[0].exit_code, Some(2));
        let mut validation = run(20, "/example/b");
        validation.role = "typecheck".into();
        let explicit = observe(
            &mut tracker,
            &[validation],
            &[process(20, None, "other"), process(21, Some(20), "other")],
            120,
        )
        .remove(0);
        assert!(explicit.agents[0]
            .roles
            .iter()
            .any(|r| r.role == "typecheck" && r.process_count == 1));
    }

    #[test]
    fn resource_advice_requires_measured_conditions() {
        let mut tracker = WorkflowTracker::new(None);
        let runs = [run(10, "/example/a")];
        let mut processes = vec![process(10, None, "other")];
        processes.extend((11..23).map(|pid| process(pid, Some(10), "test")));
        let workflow = tracker
            .observe(&runs, &processes, 110, true, 1 << 30, 16 << 30, 8)
            .remove(0);
        assert!(workflow
            .suggestions
            .iter()
            .any(|s| s.id == "worker-amplification"));
        assert!(workflow
            .suggestions
            .iter()
            .any(|s| s.id == "memory-headroom"));
        assert!(!workflow
            .suggestions
            .iter()
            .any(|s| s.id == "overlapping-test"));
        let next = tracker
            .observe(&runs, &processes[..1], 115, true, 8 << 30, 16 << 30, 8)
            .remove(0);
        assert!(next.suggestions.iter().all(|s| !s.current));
    }

    #[test]
    fn registry_cache_reloads_completion_and_rejects_oversized_or_mismatched_files() {
        let directory = std::env::temp_dir().join(format!(
            "jitterbug-workflow-registry-{}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let mut run = run(10, "/example/a");
        let path = directory.join(format!("{}.json", run.run_id));
        crate::storage::write(&path, &run).unwrap();
        let mut tracker = WorkflowTracker::new(Some(directory.clone()));
        assert_eq!(tracker.read_registry(110)[0].status, "active");
        run.status = "completed".into();
        run.ended_at = Some(112);
        run.exit_code = Some(0);
        crate::storage::write(&path, &run).unwrap();
        assert_eq!(tracker.read_registry(115)[0].status, "completed");
        crate::storage::write(&directory.join("wrong-id.json"), &run).unwrap();
        crate::storage::write(
            &directory.join("oversized.json"),
            &"x".repeat(MAX_MANIFEST_BYTES as usize),
        )
        .unwrap();
        assert_eq!(tracker.read_registry(115).len(), 1);
        assert!(tracker.read_registry(112 + RETAIN_SECONDS + 1).is_empty());
        fs::remove_dir_all(directory).unwrap();
    }
}
