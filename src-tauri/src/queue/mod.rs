//! One cooperative validation slot shared by every session of this OS user.
//! The desktop publishes headroom and schedules. Clients can request only saved profiles.
pub mod cli;
pub mod inputs;
pub mod mcp;
pub mod resources;
mod worker;

use crate::{
    storage,
    types::{now, Snapshot},
};
use resources::{Resources, GIB};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

const MAX_JOBS: usize = 48;
const MAX_PENDING: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub worktree: PathBuf,
    pub command: Vec<String>,
    pub memory_bytes: u64,
    #[serde(default)]
    pub search_path: Option<String>,
}

impl Profile {
    pub fn validate(&self) -> Result<(), String> {
        identifier(&self.name)?;
        if !self.worktree.is_absolute()
            || !self.worktree.is_dir()
            || self.command.is_empty()
            || self.command.len() > 64
            || self.command.iter().map(String::len).sum::<usize>() > 16_384
            || !Path::new(&self.command[0]).is_absolute()
            || self.memory_bytes > 128 * GIB
            || self
                .search_path
                .as_ref()
                .is_some_and(|p| p.len() > 8192 || p.contains('\0'))
        {
            return Err("Profile needs an existing absolute worktree, an absolute executable, and bounded arguments and memory estimate.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Identity {
    pub pid: u32,
    pub start: u64,
}

pub fn identity(pid: u32) -> Option<Identity> {
    let mut system = sysinfo::System::new();
    let id = sysinfo::Pid::from_u32(pid);
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[id]),
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    system
        .process(id)
        .filter(|p| p.start_time() > 0)
        .map(|p| Identity {
            pid,
            start: p.start_time(),
        })
}

impl Identity {
    fn alive(&self) -> bool {
        identity(self.pid).is_some_and(|p| p.start == self.start)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Queued,
    Starting,
    Running,
    Passed,
    Failed,
    Stale,
    Cancelled,
    Interrupted,
}

impl Stage {
    pub fn active(&self) -> bool {
        matches!(self, Self::Queued | Self::Starting | Self::Running)
    }
    fn occupies_slot(&self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Interrupted)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub request_id: String,
    pub task: String,
    pub harness: String,
    pub session: String,
    pub profile: Profile,
    pub inputs: inputs::Inputs,
    pub stage: Stage,
    pub submitted_at: u64,
    pub started_at: Option<u64>,
    pub ended_at: Option<u64>,
    pub worker: Option<Identity>,
    pub child: Option<Identity>,
    pub exit_code: Option<i32>,
    pub message: String,
    pub peak_cpu_percent: f32,
    pub peak_memory_bytes: u64,
    pub observed_samples: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Store {
    pub paused: bool,
    pub profiles: Vec<Profile>,
    pub jobs: Vec<Job>,
    #[serde(skip)]
    pub resources: Resources,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobView {
    pub id: String,
    pub task: String,
    pub harness: String,
    pub session: String,
    pub profile: String,
    pub worktree: String,
    pub revision: Option<String>,
    pub fingerprinted: bool,
    pub stage: Stage,
    pub submitted_at: u64,
    pub started_at: Option<u64>,
    pub ended_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub message: String,
    pub peak_cpu_percent: f32,
    pub peak_memory_bytes: u64,
    pub observed_samples: u64,
}

impl From<&Job> for JobView {
    fn from(j: &Job) -> Self {
        Self {
            id: j.id.clone(),
            task: j.task.clone(),
            harness: j.harness.clone(),
            session: j.session.clone(),
            profile: j.profile.name.clone(),
            worktree: j
                .profile
                .worktree
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            revision: j.inputs.revision.clone(),
            fingerprinted: j.inputs.fingerprint.is_some(),
            stage: j.stage.clone(),
            submitted_at: j.submitted_at,
            started_at: j.started_at,
            ended_at: j.ended_at,
            exit_code: j.exit_code,
            message: j.message.clone(),
            peak_cpu_percent: j.peak_cpu_percent,
            peak_memory_bytes: j.peak_memory_bytes,
            observed_samples: j.observed_samples,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct View {
    pub paused: bool,
    pub capacity: usize,
    pub reason: String,
    pub resources: Resources,
    pub profiles: Vec<String>,
    pub jobs: Vec<JobView>,
}

impl Store {
    fn reason(&self, memory: u64) -> Option<String> {
        if self.paused {
            return Some("New jobs are paused.".into());
        }
        if self.jobs.iter().any(|j| j.stage == Stage::Interrupted) {
            return Some(
                "A worker was interrupted. Review its processes before releasing the slot.".into(),
            );
        }
        if self.jobs.iter().any(|j| j.stage.occupies_slot()) {
            return Some("One validation is running. Other sessions wait their turn.".into());
        }
        if let Some(reason) = self.resources.block_reason(memory) {
            return Some(reason);
        }
        if self.resources.consecutive_good < 2 {
            return Some("Waiting for two healthy resource samples.".into());
        }
        None
    }
    pub fn view(&self) -> View {
        let memory = self
            .jobs
            .iter()
            .find(|j| j.stage == Stage::Queued)
            .map_or(0, |j| j.profile.memory_bytes);
        View {
            paused: self.paused,
            capacity: 1,
            reason: self
                .reason(memory)
                .unwrap_or_else(|| "Ready for the next validation.".into()),
            resources: self.resources.clone(),
            profiles: self.profiles.iter().map(|p| p.name.clone()).collect(),
            jobs: self.jobs.iter().rev().map(JobView::from).collect(),
        }
    }
}

pub fn directory() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("JITTERBUG_QUEUE_DIR") {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err("JITTERBUG_QUEUE_DIR must be absolute.".into());
        }
        return Ok(path);
    }
    dirs::data_dir()
        .map(|p| p.join("com.jitterbug.desktop").join("validation-queue"))
        .ok_or_else(|| "Cannot locate the private queue directory.".into())
}

fn lock(directory: &Path) -> Result<File, String> {
    fs::create_dir_all(directory).map_err(|_| "Cannot create the private queue directory.")?;
    if fs::symlink_metadata(directory)
        .map_err(|_| "Cannot inspect queue directory.")?
        .file_type()
        .is_symlink()
    {
        return Err("Queue directory must not be a symbolic link.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot restrict queue permissions.")?;
    }
    let path = directory.join("queue.lock");
    if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
        return Err("Invalid queue lock.".into());
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(path).map_err(|_| "Cannot open queue lock.")?;
    for _ in 0..100 {
        match fs2::FileExt::try_lock_exclusive(&lock) {
            Ok(()) => return Ok(lock),
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(_) => return Err("Cannot lock queue state.".into()),
        }
    }
    Err("Queue busy; retry shortly.".into())
}

fn read(directory: &Path) -> Result<Store, String> {
    let path = directory.join("queue.json");
    if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file() || m.len() > 2 * 1024 * 1024) {
        return Err("Invalid or oversized queue state; refusing to reset it.".into());
    }
    let mut state: Store = storage::read(&path)?;
    state.resources = storage::read(&directory.join("headroom.json"))?;
    Ok(state)
}

pub fn transaction<T>(
    directory: &Path,
    operation: impl FnOnce(&mut Store) -> Result<T, String>,
) -> Result<T, String> {
    let _lock = lock(directory)?;
    let mut state = read(directory)?;
    let before = serde_json::to_vec(&state).map_err(|_| "Cannot encode queue state.")?;
    let result = operation(&mut state)?;
    // Do not rewrite commands and history on every five-second resource sample.
    let encoded = serde_json::to_vec(&state).map_err(|_| "Cannot encode queue state.")?;
    if encoded.len() > 1024 * 1024 {
        return Err("Queue storage is full; remove unused profiles or shorten arguments.".into());
    }
    if encoded != before {
        storage::write(&directory.join("queue.json"), &state)?;
    }
    Ok(result)
}

pub fn inspect(directory: &Path) -> Result<Store, String> {
    let _lock = lock(directory)?;
    read(directory)
}

pub fn identifier(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 96
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err(
            "Identifiers must be 1–96 letters, digits, hyphens, underscores or dots.".into(),
        );
    }
    Ok(())
}

pub fn label(value: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.chars().count() > max || value.chars().any(char::is_control)
    {
        return Err("Invalid or oversized task label.".into());
    }
    Ok(())
}

pub fn add_profile(directory: &Path, profile: Profile) -> Result<(), String> {
    profile.validate()?;
    transaction(directory, |s| {
        if s.profiles.iter().any(|p| p.name == profile.name) {
            return Err("Profile already exists; remove it explicitly before replacing.".into());
        }
        if s.profiles.len() >= 32 {
            return Err("At most 32 saved validation profiles.".into());
        }
        s.profiles.push(profile);
        Ok(())
    })
}

pub fn submit(
    directory: &Path,
    profile: &str,
    request_id: &str,
    task: &str,
    harness: &str,
    session: &str,
) -> Result<JobView, String> {
    identifier(request_id)?;
    label(task, 160)?;
    label(harness, 40)?;
    label(session, 96)?;
    let approved = inspect(directory)?
        .profiles
        .into_iter()
        .find(|p| p.name == profile)
        .ok_or("Unknown approved validation profile.")?;
    approved.validate()?;
    let inputs = inputs::capture(&approved.worktree, approved.search_path.as_deref())?;
    transaction(directory, |s| {
        if let Some(existing) = s.jobs.iter().find(|j| j.request_id == request_id) {
            if existing.profile.name != profile
                || existing.task != task
                || existing.harness != harness
                || existing.session != session
            {
                return Err("Request ID belongs to another job; use a unique request ID.".into());
            }
            return Ok(JobView::from(existing));
        }
        if !s.profiles.contains(&approved) {
            return Err("Profile changed; review and retry.".into());
        }
        if s.jobs
            .iter()
            .filter(|j| j.stage.active() || j.stage == Stage::Interrupted)
            .count()
            >= MAX_PENDING
        {
            return Err("Queue is full (16 pending jobs).".into());
        }
        while s.jobs.len() >= MAX_JOBS {
            let index = s
                .jobs
                .iter()
                .position(|j| !j.stage.active() && j.stage != Stage::Interrupted)
                .ok_or("Queue history is full.")?;
            let removed = s.jobs.remove(index);
            let _ = fs::remove_file(directory.join(format!("{}.output.json", removed.id)));
        }
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let job = Job {
            id: format!("validation-{}-{unique:x}", std::process::id()),
            request_id: request_id.into(),
            task: task.into(),
            harness: harness.into(),
            session: session.into(),
            profile: approved,
            inputs,
            stage: Stage::Queued,
            submitted_at: now(),
            started_at: None,
            ended_at: None,
            worker: None,
            child: None,
            exit_code: None,
            message: "Waiting for admission.".into(),
            peak_cpu_percent: 0.0,
            peak_memory_bytes: 0,
            observed_samples: 0,
        };
        let view = JobView::from(&job);
        s.jobs.push(job);
        Ok(view)
    })
}

pub fn cancel(directory: &Path, id: &str) -> Result<(), String> {
    transaction(directory, |s| {
        let job = s
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or("Job not found.")?;
        if job.stage != Stage::Queued {
            return Err("Only queued jobs can be cancelled. Running jobs finish normally.".into());
        }
        job.stage = Stage::Cancelled;
        job.ended_at = Some(now());
        job.message = "Cancelled before launch.".into();
        Ok(())
    })
}

pub fn acknowledge(directory: &Path, id: &str) -> Result<(), String> {
    transaction(directory, |s| {
        let job = s
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or("Job not found.")?;
        if job.stage != Stage::Interrupted
            || job.worker.as_ref().is_some_and(Identity::alive)
            || job.child.as_ref().is_some_and(Identity::alive)
        {
            return Err(
                "Cannot release this slot: worker or direct child may still be running.".into(),
            );
        }
        job.stage = Stage::Failed;
        job.ended_at = Some(now());
        job.message =
            "Interrupted run reviewed and slot released by user; no validation result.".into();
        Ok(())
    })
}

pub fn tick(directory: &Path, snapshot: &Snapshot, executable: &Path) -> Result<(), String> {
    transaction(directory, |s| {
        let new_sample = s.resources.captured_at != snapshot.captured_at;
        s.resources = Resources::from_snapshot(snapshot, &s.resources);
        storage::write(&directory.join("headroom.json"), &s.resources)?;
        for job in &mut s.jobs {
            if matches!(job.stage, Stage::Starting | Stage::Running) {
                if !job.worker.as_ref().is_some_and(Identity::alive) {
                    job.stage = Stage::Interrupted;
                    job.message = "Worker disappeared. Its descendants might still be running; inspect them before releasing the slot.".into();
                }
                if new_sample {
                    if let Some(session) = snapshot
                        .agent_workflows
                        .iter()
                        .flat_map(|w| &w.agents)
                        .find(|a| a.run_id == job.id)
                    {
                        job.peak_cpu_percent = job.peak_cpu_percent.max(session.peak_cpu_percent);
                        job.peak_memory_bytes =
                            job.peak_memory_bytes.max(session.peak_memory_bytes);
                        job.observed_samples = job.observed_samples.max(session.observed_samples);
                    }
                }
            }
            if job.stage == Stage::Queued && job.submitted_at + 3600 < now() {
                job.stage = Stage::Cancelled;
                job.ended_at = Some(now());
                job.message =
                    "Expired after one hour in the queue; submit again when ready.".into();
            }
        }
        let Some(index) = s.jobs.iter().position(|j| j.stage == Stage::Queued) else {
            return Ok(());
        };
        if let Some(reason) = s.reason(s.jobs[index].profile.memory_bytes) {
            s.jobs[index].message = reason;
            return Ok(());
        }
        let job = &mut s.jobs[index];
        let mut command = Command::new(executable);
        command
            .args(["--queue-worker", &job.id])
            .env("JITTERBUG_QUEUE_DIR", directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // The desktop may have loaded its model credentials from .env. They are
        // not credentials for a validation command and must not travel with it.
        for key in [
            "META_API_KEY",
            "MODEL_API_KEY",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "OPENAI_API_KEY",
            "JEV_API_KEY",
        ] {
            command.env_remove(key);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        match command.spawn() {
            Ok(mut process) => {
                job.stage = Stage::Starting;
                job.worker = identity(process.id());
                job.message = "Checking source inputs and current headroom.".into();
                // Reap owned children without blocking the collector. A detached worker can
                // finish if the desktop closes; no new jobs start without fresh telemetry.
                std::thread::spawn(move || {
                    let _ = process.wait();
                });
            }
            Err(_) => {
                job.stage = Stage::Failed;
                job.ended_at = Some(now());
                job.message = "Could not start validation worker.".into();
            }
        }
        Ok(())
    })
}

pub fn worker_entry() -> bool {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_none_or(|a| a != "--queue-worker") {
        return false;
    }
    let result = args
        .get(2)
        .ok_or_else(|| "Missing job ID.".into())
        .and_then(|id| id.to_str().ok_or_else(|| "Invalid job ID.".into()))
        .and_then(|id| directory().and_then(|d| worker::run(&d, id)));
    if let Err(error) = result {
        eprintln!("jitterbug: {error}");
    }
    true
}
