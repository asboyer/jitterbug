use super::*;
use std::io::Read;

#[derive(Default, Serialize, Deserialize)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

fn drain(mut reader: impl Read) -> (String, bool) {
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut buffer = [0u8; 4096];
    while let Ok(count) = reader.read(&mut buffer) {
        if count == 0 {
            break;
        }
        let retain = count.min(32_768usize.saturating_sub(kept.len()));
        kept.extend_from_slice(&buffer[..retain]);
        truncated |= retain < count;
    }
    (String::from_utf8_lossy(&kept).into_owned(), truncated)
}

pub fn run(directory: &Path, id: &str) -> Result<(), String> {
    identifier(id)?;
    let own = identity(std::process::id()).ok_or("Cannot verify worker identity.")?;
    let state = inspect(directory)?;
    let original = state
        .jobs
        .iter()
        .find(|j| j.id == id)
        .ok_or("Job not found.")?
        .clone();
    if original.stage != Stage::Starting
        || original
            .worker
            .as_ref()
            .is_none_or(|w| w.pid != own.pid || w.start != own.start)
    {
        return Err("Worker does not own this job.".into());
    }
    let inputs = inputs::capture(
        &original.profile.worktree,
        original.profile.search_path.as_deref(),
    );
    let allowed = transaction(directory, |s| {
        let reason = s
            .resources
            .block_reason(original.profile.memory_bytes)
            .or_else(|| s.goal_block())
            .or_else(|| {
                s.jobs
                    .iter()
                    .find(|j| j.id == id)
                    .and_then(|j| s.job_hold(j))
            });
        let approved = s.profiles.contains(&original.profile);
        let job = s
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or("Job vanished.")?;
        if job.stage != Stage::Starting {
            return Ok(false);
        }
        if !approved || inputs.as_ref().is_err() || inputs.as_ref().is_ok_and(|i| *i != job.inputs)
        {
            job.stage = Stage::Stale;
            job.ended_at = Some(now());
            job.message = if !approved {
                "Approved profile changed or was removed; submit again.".into()
            } else {
                "Source inputs changed or could not be verified while waiting; submit a fresh job."
                    .into()
            };
            return Ok(false);
        }
        if s.paused || reason.is_some() {
            job.stage = Stage::Queued;
            job.worker = None;
            job.message = reason.unwrap_or_else(|| "New jobs are paused.".into());
            return Ok(false);
        }
        job.stage = Stage::Running;
        job.started_at = Some(now());
        job.message = "Running the approved validation command.".into();
        Ok(true)
    })?;
    if !allowed {
        return Ok(());
    }
    let registry = crate::workflows::registry_directory()?;
    let mut manifest = crate::workflows::RunManifest {
        version: 1,
        run_id: id.into(),
        // Reusing a profile does not prove that two requests belong to one task.
        task_id: id.into(),
        task_title: original.task.clone(),
        agent: original.session.chars().take(80).collect(),
        harness: original.harness.clone(),
        worktree: original.profile.worktree.clone(),
        role: "test".into(),
        root_pid: own.pid,
        root_start_time: own.start,
        started_at: now(),
        ended_at: None,
        status: "active".into(),
        exit_code: None,
        context: Default::default(),
    };
    // Attribution registration must succeed before executing anything.
    crate::workflows::register(&registry, &manifest)?;
    let mut command = Command::new(&original.profile.command[0]);
    if let Some(value) = &original.profile.search_path {
        command.env("PATH", value);
    }
    command
        .args(&original.profile.command[1..])
        .current_dir(&original.profile.worktree)
        .env("JITTERBUG_QUEUE_JOB_ID", id)
        .env("JITTERBUG_RUN_ID", id)
        .env("JITTERBUG_AGENT_RUNS_DIR", &registry)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let (exit_code, output) = match command.spawn() {
        Ok(mut child) => {
            let child_identity = identity(child.id());
            // If persistence fails, still wait for our child; never silently orphan it.
            let _ = transaction(directory, |s| {
                s.jobs
                    .iter_mut()
                    .find(|j| j.id == id)
                    .ok_or("Job vanished.")?
                    .child = child_identity;
                Ok(())
            });
            let stdout = child.stdout.take().ok_or("Missing stdout pipe.")?;
            let stderr = child.stderr.take().ok_or("Missing stderr pipe.")?;
            let out = std::thread::spawn(move || drain(stdout));
            let err = std::thread::spawn(move || drain(stderr));
            let code = child
                .wait()
                .map(|status| {
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
                })
                .unwrap_or(127);
            let (stdout, a) = out.join().map_err(|_| "Output reader failed.")?;
            let (stderr, b) = err.join().map_err(|_| "Output reader failed.")?;
            (
                code,
                Output {
                    stdout,
                    stderr,
                    truncated: a || b,
                },
            )
        }
        Err(_) => (
            127,
            Output {
                stderr: "Approved command could not be launched.".into(),
                ..Default::default()
            },
        ),
    };
    let after = inputs::capture(
        &original.profile.worktree,
        original.profile.search_path.as_deref(),
    );
    let _ = crate::workflows::finish(&registry, &mut manifest, exit_code);
    storage::write(&directory.join(format!("{id}.output.json")), &output)?;
    transaction(directory, |s| {
        let job = s
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or("Job vanished.")?;
        job.exit_code = Some(exit_code);
        job.ended_at = Some(now());
        let changed = after.as_ref().is_err() || after.as_ref().is_ok_and(|a| *a != job.inputs);
        job.stage = if changed {
            Stage::Stale
        } else if exit_code == 0 {
            Stage::Passed
        } else {
            Stage::Failed
        };
        job.message = if changed {
            "Inputs changed during validation; result is stale. Submit again.".into()
        } else if job.inputs.fingerprint.is_none() {
            format!(
                "Command exited {exit_code}. Not a Git worktree: source inputs were not verified."
            )
        } else {
            format!("Command exited {exit_code}. Git source fingerprint matched before and after; ignored/external inputs are not covered.")
        };
        Ok(())
    })
}

pub fn output(directory: &Path, id: &str) -> Result<Output, String> {
    identifier(id)?;
    let path = directory.join(format!("{id}.output.json"));
    if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file() || m.len() > 512 * 1024) {
        return Err("Invalid job output.".into());
    }
    storage::read(&path)
}
