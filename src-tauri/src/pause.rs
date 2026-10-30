//! Pause and resume one command's whole process tree (an agent session, a build, a script)
//! without ending it. Paused work keeps its memory, but stops using CPU and stops growing,
//! and macOS can compress or swap its pages for other work.
//!
//! A plain SIGSTOP on a terminal's foreground command makes an interactive shell report it as
//! suspended and take the terminal back, so the command could not read input after resuming.
//! The shell that launched the command is therefore stopped first and continued last: it never
//! observes the stop, and the command keeps the terminal.
//!
//! Pauses expire, are resumed when Jitterbug quits, and are recorded privately so a crash
//! cannot leave work frozen: the next launch resumes anything still recorded.
use crate::types::now;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Mutex,
};
use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind};

/// Paused work resumes by itself after this long unless resumed sooner.
pub const LIMIT_SECONDS: u64 = 15 * 60;
const SHELLS: &[&str] = &[
    "zsh", "bash", "fish", "sh", "dash", "nu", "tcsh", "csh", "ksh",
];
const TREE_LIMIT: usize = 512;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Paused {
    pub pid: u32,
    pub start_time: u64,
    pub label: String,
    /// Every stopped process, root first, by PID and start time.
    pub members: Vec<(u32, u64)>,
    /// The launching shell, stopped first so it never sees the command stop.
    pub shell: Option<(u32, u64)>,
    pub paused_at: u64,
    pub until: u64,
}

struct Registry {
    file: Option<PathBuf>,
    paused: Vec<Paused>,
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    file: None,
    paused: Vec::new(),
});

fn registry() -> std::sync::MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(|p| p.into_inner())
}

fn save(registry: &Registry) {
    if let Some(file) = &registry.file {
        let _ = crate::storage::write(file, &registry.paused);
    }
}

fn system() -> System {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_user(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    system
}

fn shell(p: &Process) -> bool {
    let name = p.name().to_string_lossy().to_lowercase();
    SHELLS.contains(&name.trim_start_matches('-'))
}

/// Jitterbug and every process it descends from: never paused, never part of a paused tree.
fn protected(system: &System) -> HashSet<u32> {
    let mut out = HashSet::new();
    let mut current = Some(Pid::from_u32(std::process::id()));
    for _ in 0..64 {
        let Some(pid) = current else { break };
        if !out.insert(pid.as_u32()) {
            break;
        }
        current = system.process(pid).and_then(Process::parent);
    }
    out
}

fn tree(system: &System, root: &Process) -> Vec<(u32, u64)> {
    let mut children: HashMap<Pid, Vec<&Process>> = HashMap::new();
    for p in system.processes().values() {
        if let Some(parent) = p.parent().and_then(|id| system.process(id)) {
            // A reused parent PID must not adopt an unrelated process.
            if parent.start_time() <= p.start_time() && parent.pid() != p.pid() {
                children.entry(parent.pid()).or_default().push(p);
            }
        }
    }
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() && out.len() < TREE_LIMIT {
        for child in children.get(&out[i].pid()).into_iter().flatten() {
            if !out.iter().any(|p| p.pid() == child.pid()) {
                out.push(child);
            }
        }
        i += 1;
    }
    out.iter()
        .map(|p| (p.pid().as_u32(), p.start_time()))
        .collect()
}

/// Why a process cannot be paused, if it cannot. Only your own commands qualify: not apps
/// (a frozen window cannot redraw), not system processes, and never Jitterbug's own lineage.
pub fn refusal(p: &Process, system: &System) -> Option<&'static str> {
    if !cfg!(unix) {
        return Some("Pausing is not implemented on this platform yet.");
    }
    if p.pid().as_u32() <= 100 || !crate::platform::same_user(p) {
        return Some("Only your own commands can be paused.");
    }
    if p.exe()
        .and_then(crate::attribution::application_bundle)
        .is_some()
    {
        return Some("Apps are not paused: a frozen window cannot redraw. Pause a command running inside it instead.");
    }
    let protected = protected(system);
    if tree(system, p)
        .iter()
        .any(|(pid, _)| protected.contains(pid))
    {
        return Some("This command contains Jitterbug itself.");
    }
    None
}

pub fn pausable(p: &Process, system: &System) -> bool {
    refusal(p, system).is_none()
}

#[cfg(unix)]
fn signal(pid: u32, start: u64, system: &System, signal: Signal) -> bool {
    system
        .process(Pid::from_u32(pid))
        .filter(|p| p.start_time() == start)
        .and_then(|p| p.kill_with(signal))
        .unwrap_or(false)
}

/// Pause a command's tree. Returns the record, including when it resumes by itself.
pub fn pause(pid: u32, start_time: u64, label: &str) -> Result<Paused, String> {
    if is_paused(pid, start_time) {
        return Err("This command is already paused.".into());
    }
    let system = system();
    let root = system
        .process(Pid::from_u32(pid))
        .filter(|p| p.start_time() == start_time)
        .ok_or("The command has exited or changed identity. Nothing was paused.")?;
    if let Some(reason) = refusal(root, &system) {
        return Err(reason.into());
    }
    #[cfg(unix)]
    {
        let protected = protected(&system);
        let shell = root
            .parent()
            .and_then(|id| system.process(id))
            .filter(|parent| shell(parent) && crate::platform::same_user(parent))
            .filter(|parent| !protected.contains(&parent.pid().as_u32()))
            .map(|parent| (parent.pid().as_u32(), parent.start_time()));
        if let Some((spid, sstart)) = shell {
            if !signal(spid, sstart, &system, Signal::Stop) {
                return Err("The operating system rejected the pause.".into());
            }
        }
        // Stop the root first so it cannot start new children, then catch any it just started.
        let mut members: Vec<(u32, u64)> = vec![];
        for _ in 0..3 {
            let current = system_tree(pid, start_time);
            let fresh: Vec<_> = current
                .into_iter()
                .filter(|m| !members.contains(m) && !protected.contains(&m.0))
                .collect();
            if fresh.is_empty() {
                break;
            }
            let check = self::system();
            for (mpid, mstart) in fresh {
                if signal(mpid, mstart, &check, Signal::Stop) {
                    members.push((mpid, mstart));
                }
            }
        }
        if !members.contains(&(pid, start_time)) {
            // Undo the shell stop: nothing of the command was paused.
            if let Some((spid, sstart)) = shell {
                signal(spid, sstart, &self::system(), Signal::Continue);
            }
            return Err("The operating system rejected the pause.".into());
        }
        let at = now();
        let record = Paused {
            pid,
            start_time,
            label: label.chars().take(120).collect(),
            members,
            shell,
            paused_at: at,
            until: at + LIMIT_SECONDS,
        };
        let mut registry = registry();
        registry.paused.push(record.clone());
        save(&registry);
        Ok(record)
    }
    #[cfg(not(unix))]
    {
        let _ = (label, root);
        Err("Pausing is not implemented on this platform yet.".into())
    }
}

fn system_tree(pid: u32, start: u64) -> Vec<(u32, u64)> {
    let system = system();
    system
        .process(Pid::from_u32(pid))
        .filter(|p| p.start_time() == start)
        .map(|root| tree(&system, root))
        .unwrap_or_default()
}

fn continue_record(record: &Paused) -> usize {
    #[cfg(unix)]
    {
        let system = system();
        let resumed = record
            .members
            .iter()
            .filter(|(pid, start)| signal(*pid, *start, &system, Signal::Continue))
            .count();
        // The shell resumes last, after its command is running again.
        if let Some((pid, start)) = record.shell {
            signal(pid, start, &system, Signal::Continue);
        }
        resumed
    }
    #[cfg(not(unix))]
    {
        let _ = record;
        0
    }
}

pub fn resume(pid: u32, start_time: u64) -> Result<String, String> {
    let record = {
        let mut registry = registry();
        let index = registry
            .paused
            .iter()
            .position(|p| p.pid == pid && p.start_time == start_time)
            .ok_or("This command is not paused by Jitterbug.")?;
        let record = registry.paused.remove(index);
        save(&registry);
        record
    };
    let resumed = continue_record(&record);
    Ok(if resumed == 0 {
        format!("{} had already exited.", record.label)
    } else {
        format!("{} resumed.", record.label)
    })
}

pub fn is_paused(pid: u32, start_time: u64) -> bool {
    registry()
        .paused
        .iter()
        .any(|p| p.pid == pid && p.start_time == start_time)
}

pub fn list() -> Vec<Paused> {
    registry().paused.clone()
}

/// Resume pauses that reached their limit. Returns their labels.
pub fn expire() -> Vec<String> {
    let due: Vec<Paused> = {
        let mut registry = registry();
        let at = now();
        let (due, keep): (Vec<_>, Vec<_>) = registry.paused.drain(..).partition(|p| p.until <= at);
        registry.paused = keep;
        if !due.is_empty() {
            save(&registry);
        }
        due
    };
    due.iter()
        .map(|p| {
            continue_record(p);
            p.label.clone()
        })
        .collect()
}

/// Resume everything; called when Jitterbug quits.
pub fn resume_all() {
    let all: Vec<Paused> = {
        let mut registry = registry();
        let all = std::mem::take(&mut registry.paused);
        save(&registry);
        all
    };
    for p in &all {
        continue_record(p);
    }
}

/// Load the private record and resume anything a previous run left paused.
pub fn init(directory: &Path) {
    let file = directory.join("paused.json");
    let left: Vec<Paused> = crate::storage::read(&file).unwrap_or_default();
    {
        let mut registry = registry();
        registry.file = Some(file);
        registry.paused.clear();
        save(&registry);
    }
    for p in &left {
        continue_record(p);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    fn state(pid: u32) -> String {
        let out = Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn a_tree_pauses_and_resumes_together_and_jitterbug_is_protected() {
        // sh → sleep: the root and its child must both stop and both continue.
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30 & wait"])
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let pid = child.id();
        let system = system();
        let root = system.process(Pid::from_u32(pid)).unwrap();
        let start = root.start_time();
        let record = pause(pid, start, "test tree").unwrap();
        assert!(record.members.len() >= 2, "{:?}", record.members);
        assert!(record.until > record.paused_at);
        // The test runner is the parent and an ancestor of nothing paused; it is not a shell.
        assert_eq!(record.shell, None);
        assert!(record
            .members
            .iter()
            .all(|(p, _)| state(*p).starts_with('T')));
        assert!(is_paused(pid, start));
        assert!(pause(pid, start, "again").is_err());
        resume(pid, start).unwrap();
        assert!(record
            .members
            .iter()
            .all(|(p, _)| !state(*p).starts_with('T')));
        assert!(!is_paused(pid, start));
        let _ = child.kill();
        let _ = child.wait();
        // This test process is Jitterbug's stand-in: it can never be paused.
        let me = system.process(Pid::from_u32(std::process::id())).unwrap();
        assert!(refusal(me, &system).is_some());
    }
}
