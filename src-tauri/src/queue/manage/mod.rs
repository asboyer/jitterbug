//! User-reviewed enrollment. Discovery never executes commands or changes client settings.
pub mod connections;
pub mod discovery;

use super::{Profile, Stage};
use crate::storage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

#[derive(Clone, Deserialize)]
pub struct Request {
    pub name: String,
    pub worktree: PathBuf,
    pub command: Vec<String>,
    pub memory_mib: u64,
    pub clients: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Change {
    pub path: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct Preview {
    pub token: String,
    pub profile: Profile,
    pub existing: bool,
    pub changes: Vec<Change>,
    pub project_scripts: Option<String>,
}

#[derive(Clone)]
pub struct Edit {
    pub path: PathBuf,
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
    pub preview: String,
    pub visible: bool,
}

struct Plan {
    directory: PathBuf,
    profile: Profile,
    existing: bool,
    edits: Vec<Edit>,
    manifests: Vec<(PathBuf, Option<Vec<u8>>)>,
    created: u64,
}

static PLANS: OnceLock<Mutex<HashMap<String, Plan>>> = OnceLock::new();

fn plans() -> &'static Mutex<HashMap<String, Plan>> {
    PLANS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn discard(token: &str) {
    if let Ok(mut plans) = plans().lock() {
        plans.remove(token);
    }
}

// Reject symbolic links at every existing component, including parent directories. Never
// rewrite a linked client config or a project-controlled target outside its reviewed path.
fn regular_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("Configuration paths must be absolute without parent traversal.".into());
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("Symbolic-link configuration paths require manual setup.".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot inspect the configuration path.".into()),
        }
    }
    Ok(())
}

pub fn read_bounded(path: &Path, limit: u64) -> Result<Option<Vec<u8>>, String> {
    regular_path(path)?;
    match fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Cannot read configuration or project file.".into()),
        Ok(meta) => {
            if !meta.is_file() || meta.len() > limit {
                return Err("Configuration or project file is not a bounded regular file.".into());
            }
            use std::io::Read;
            let mut bytes = Vec::new();
            fs::File::open(path)
                .map_err(|_| "Cannot open configuration.")?
                .take(limit + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "Cannot read configuration.")?;
            if bytes.len() as u64 > limit {
                return Err("Configuration grew beyond its size limit.".into());
            }
            Ok(Some(bytes))
        }
    }
}

fn write_file(path: &Path, bytes: Option<&[u8]>) -> Result<(), String> {
    regular_path(path)?;
    let Some(bytes) = bytes else {
        if path.exists() {
            fs::remove_file(path).map_err(|_| "Cannot roll back the new configuration file.")?;
        }
        return Ok(());
    };
    let parent = path.parent().ok_or("Missing configuration directory.")?;
    fs::create_dir_all(parent).map_err(|_| "Cannot create configuration directory.")?;
    regular_path(path)?;
    let temporary = parent.join(format!(
        ".jitterbug-{}-{}.tmp",
        std::process::id(),
        crate::types::now()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|_| "Cannot stage configuration changes.")?;
        file.write_all(bytes)
            .map_err(|_| "Cannot write configuration changes.")?;
        file.sync_all()
            .map_err(|_| "Cannot persist configuration changes.")?;
        fs::rename(&temporary, path).map_err(|_| "Cannot replace configuration file.")
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result.map_err(str::to_owned)
}

pub fn prepare(
    directory: &Path,
    locations: &connections::Locations,
    executable: &Path,
    request: Request,
) -> Result<Preview, String> {
    if request.clients.len() > 3
        || request
            .clients
            .iter()
            .any(|c| !connections::CLIENTS.contains(&c.as_str()))
    {
        return Err("Choose supported clients only.".into());
    }
    if request.command.is_empty()
        || request
            .command
            .iter()
            .any(|a| a.contains('\0') || a.contains('\n') || a.contains('\r'))
    {
        return Err(
            "Enter one executable and separate arguments, without control characters.".into(),
        );
    }
    if discovery::sensitive(&request.command.join(" ")) {
        return Err("This command appears to contain credentials. Remove secrets from arguments before saving it.".into());
    }
    let worktree = request
        .worktree
        .canonicalize()
        .map_err(|_| "Project folder unavailable.")?;
    let search_path = discovery::search_path(&request.command);
    let mut command = request.command;
    command[0] = discovery::resolve(&command[0], &worktree, &search_path)?;
    let profile = Profile {
        name: request.name,
        worktree,
        command,
        memory_bytes: request
            .memory_mib
            .checked_mul(1024 * 1024)
            .ok_or("Memory estimate too large.")?,
        search_path: Some(search_path),
    };
    profile.validate()?;
    let state = super::inspect(directory)?;
    let existing = state.profiles.iter().find(|p| p.name == profile.name);
    if existing.is_some_and(|p| {
        p.worktree != profile.worktree
            || p.command != profile.command
            || p.memory_bytes != profile.memory_bytes
    }) {
        return Err("That name is already approved for a different command. Choose another name or revoke the old profile.".into());
    }
    let profile = existing.cloned().unwrap_or(profile);
    let mut manifests = Vec::new();
    for name in ["package.json", "Cargo.toml"] {
        let path = profile.worktree.join(name);
        manifests.push((path.clone(), read_bounded(&path, 256 * 1024)?));
    }
    let scripts = manifests
        .iter()
        .find(|(p, _)| p.file_name().is_some_and(|n| n == "package.json"))
        .and_then(|(_, b)| b.as_ref())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok())
        .and_then(|v| v.get("scripts").cloned());
    let project_scripts = scripts.map(|s| serde_json::to_string_pretty(&s).unwrap_or_default());
    if project_scripts
        .as_ref()
        .is_some_and(|s| discovery::sensitive(s))
    {
        return Err("Project scripts appear to contain credentials. Remove them before reviewing this profile.".into());
    }
    let mut edits = Vec::new();
    let mut clients = request.clients;
    clients.sort();
    clients.dedup();
    for client in clients {
        edits.extend(connections::prepare(
            directory,
            locations,
            &client,
            &profile.name,
            executable,
        )?);
    }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let token = format!(
        "{:x}",
        Sha256::digest(format!(
            "{}:{:?}:{}:{sequence}",
            std::process::id(),
            std::time::SystemTime::now(),
            profile.name
        ))
    );
    let preview = Preview {
        token: token.clone(),
        profile: profile.clone(),
        existing: existing.is_some(),
        project_scripts,
        changes: edits
            .iter()
            .filter(|e| e.visible)
            .map(|e| Change {
                path: e.path.to_string_lossy().into(),
                description: e.preview.clone(),
            })
            .collect(),
    };
    let mut plans = plans().lock().map_err(|_| "Setup service unavailable.")?;
    plans.retain(|_, p| crate::types::now().saturating_sub(p.created) < 600);
    if plans.len() >= 8 {
        if let Some(oldest) = plans
            .iter()
            .min_by_key(|(_, p)| p.created)
            .map(|(token, _)| token.clone())
        {
            plans.remove(&oldest);
        }
    }
    plans.insert(
        token,
        Plan {
            directory: directory.into(),
            profile,
            existing: existing.is_some(),
            edits,
            manifests,
            created: crate::types::now(),
        },
    );
    Ok(preview)
}

pub fn apply(token: &str) -> Result<String, String> {
    let plan = plans()
        .lock()
        .map_err(|_| "Setup service unavailable.")?
        .remove(token)
        .ok_or("Preview expired. Review the changes again.")?;
    if crate::types::now().saturating_sub(plan.created) >= 600 {
        return Err("Preview expired. Review the changes again.".into());
    }
    let mut written: Vec<&Edit> = Vec::new();
    let result = super::transaction(&plan.directory, |state| {
        plan.profile.validate()?;
        if !Path::new(&plan.profile.command[0]).is_file() {
            return Err("Executable moved. Review again.".into());
        }
        let current = state.profiles.iter().find(|p| p.name == plan.profile.name);
        if (plan.existing && current != Some(&plan.profile))
            || (!plan.existing && current.is_some())
        {
            return Err("Profile changed since preview. Review again.".into());
        }
        if !plan.existing && state.profiles.len() >= 32 {
            return Err("At most 32 approved profiles.".into());
        }
        for (path, before) in &plan.manifests {
            if read_bounded(path, 256 * 1024)? != *before {
                return Err("Project scripts changed since preview. Review again.".into());
            }
        }
        for edit in &plan.edits {
            if read_bounded(&edit.path, 2 * 1024 * 1024)? != edit.before {
                return Err(
                    "Client settings changed since preview. Review again; no changes applied."
                        .into(),
                );
            }
        }
        // Keep recovery copies private. Settings can contain unrelated provider credentials.
        for (index, edit) in plan.edits.iter().enumerate() {
            if let Some(before) = &edit.before {
                let backup = plan
                    .directory
                    .join("setup-backup")
                    .join(format!("{index}.json"));
                storage::write(
                    &backup,
                    &serde_json::json!({"path":edit.path,"bytes":before}),
                )?;
            }
            // Recheck each file immediately before replacement as other apps can write settings.
            if read_bounded(&edit.path, 2 * 1024 * 1024)? != edit.before {
                return Err("Client settings changed during setup. Retry the preview.".into());
            }
            write_file(&edit.path, edit.after.as_deref())?;
            written.push(edit);
        }
        if !plan.existing {
            state.profiles.push(plan.profile.clone());
        }
        Ok(())
    });
    if let Err(error) = result {
        let mut restored = true;
        for edit in written.into_iter().rev() {
            if read_bounded(&edit.path, 2 * 1024 * 1024).ok().as_ref() == Some(&edit.after) {
                restored &= write_file(&edit.path, edit.before.as_deref()).is_ok();
            } else {
                restored = false;
            }
        }
        return Err(if restored {
            error
        } else {
            format!("{error} Some settings changed concurrently and could not be restored. Private recovery copies are in the queue's setup-backup directory; review client settings before retrying.")
        });
    }
    Ok(plan.profile.name)
}

#[derive(Serialize)]
pub struct ManagedProfile {
    pub profile: Profile,
    pub state: String,
    pub last_result: Option<String>,
    pub connections: Vec<connections::ConnectionStatus>,
}

pub fn overview(
    directory: &Path,
    locations: &connections::Locations,
) -> Result<Vec<ManagedProfile>, String> {
    let store = super::inspect(directory)?;
    let mut connections = connections::statuses(
        directory,
        locations,
        &store
            .profiles
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>(),
    );
    Ok(store
        .profiles
        .iter()
        .map(|profile| {
            let active = store.jobs.iter().rev().find(|j| {
                j.profile.name == profile.name
                    && (j.stage.active() || j.stage == Stage::Interrupted)
            });
            ManagedProfile {
                profile: profile.clone(),
                state: match active.map(|j| &j.stage) {
                    Some(Stage::Queued) => "waiting",
                    Some(Stage::Starting | Stage::Running) => "running",
                    Some(Stage::Interrupted) => "needs_review",
                    _ => "managed",
                }
                .into(),
                last_result: store
                    .jobs
                    .iter()
                    .rev()
                    .find(|j| j.profile.name == profile.name)
                    .map(|j| {
                        format!(
                            "{} · {}",
                            serde_json::to_value(&j.stage)
                                .unwrap_or_default()
                                .as_str()
                                .unwrap_or("unknown"),
                            j.message
                        )
                    }),
                connections: connections.remove(&profile.name).unwrap_or_default(),
            }
        })
        .collect())
}

pub fn client_entry() -> Option<Result<i32, String>> {
    let raw: Vec<_> = std::env::args_os().skip(1).collect();
    if !raw.first().is_some_and(|a| a == "mcp" || a == "queue") {
        return None;
    }
    Some((|| {
        let args: Vec<String> = raw
            .into_iter()
            .map(|a| {
                a.into_string()
                    .map_err(|_| "Queue arguments must be UTF-8.".to_owned())
            })
            .collect::<Result<_, _>>()?;
        if args[0] == "mcp" {
            super::mcp::serve(&args[1..])
        } else {
            super::cli::run(&args[1..])
        }
    })())
}

pub fn submit_profile(name: &str) -> Result<super::JobView, String> {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    super::submit(
        &super::directory()?,
        name,
        &format!("desktop-{unique:x}"),
        &format!("Validate {name}"),
        "manual",
        "desktop",
    )
}

pub fn revoke_profile(directory: &Path, name: &str) -> Result<(), String> {
    super::identifier(name)?;
    super::transaction(directory, |s| {
        s.profiles.retain(|p| p.name != name);
        for job in s
            .jobs
            .iter_mut()
            .filter(|j| j.profile.name == name && j.stage == Stage::Queued)
        {
            job.stage = Stage::Cancelled;
            job.ended_at = Some(crate::types::now());
            job.message = "Profile revoked before launch.".into();
        }
        Ok(())
    })
}
