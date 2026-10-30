//! Pause and resume one command's whole process tree (an agent session, a build, a script)
//! without ending it. Paused work keeps its memory, but stops using CPU and stops growing,
//! and macOS can compress or swap its pages for other work.
//!
//! A plain SIGSTOP on a terminal's foreground command makes an interactive shell report it as
//! suspended and take the terminal back, so the command could not read input after resuming.
//! The shell that launched the command is therefore stopped first and continued last: it never
//! observes the stop, and the command keeps the terminal.
//!
//! Pauses expire, are resumed when Jitterbug quits, and are recorded privately so the next
//! launch resumes anything still recorded. A detached watchdog also resumes them shortly after
//! the limit in case Jitterbug is killed and not started again.
use crate::types::now;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Mutex,
};
#[cfg(unix)]
use sysinfo::Signal;
use sysinfo::{
    Pid, Process, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind,
};

/// Paused work resumes by itself after this long unless resumed sooner.
pub const LIMIT_SECONDS: u64 = 15 * 60;
#[cfg(unix)]
const SHELLS: &[&str] = &[
    "zsh", "bash", "fish", "sh", "dash", "nu", "tcsh", "csh", "ksh",
];
const TREE_LIMIT: usize = 512;
/// The watchdog waits this much past the limit, so the in-app expiry normally runs first.
#[cfg(unix)]
const WATCHDOG_GRACE_SECONDS: u64 = 30;

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
    /// The detached process that resumes this pause if Jitterbug is no longer running.
    #[serde(default)]
    pub watchdog: Option<(u32, u64)>,
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

#[cfg(unix)]
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

/// Whether `parent` runs `pid` as a job: the command leads its own process group, apart from
/// the parent's. Only such a shell watches the command stop and takes the terminal back. A
/// script shell that merely runs the command in its own group is never stopped: stopping it
/// would itself be seen by the interactive shell above it.
#[cfg(unix)]
fn job_control(pid: u32, parent: u32) -> bool {
    let (group, parent_group) =
        unsafe { (libc::getpgid(pid as i32), libc::getpgid(parent as i32)) };
    group > 0 && group == pid as i32 && parent_group > 0 && parent_group != group
}

/// Whether the command owns its terminal (its process group is the terminal's foreground
/// group). Only then does the launching shell react to it stopping. Unknown counts as yes.
#[cfg(unix)]
fn foreground(pid: u32) -> bool {
    let Ok(out) = std::process::Command::new("ps")
        .args(["-o", "pgid=,tpgid=", "-p", &pid.to_string()])
        .output()
    else {
        return true;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut ids = text.split_whitespace().map(str::parse::<i64>);
    match (ids.next(), ids.next()) {
        (Some(Ok(group)), Some(Ok(terminal))) => terminal > 0 && group == terminal,
        _ => true,
    }
}

/// Start a detached process that continues `record` after `seconds`, members first and the
/// shell last, even if Jitterbug has exited by then. Returns its PID and start time.
#[cfg(unix)]
fn watchdog(record: &Paused, seconds: u64) -> Option<(u32, u64)> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let pids = record
        .members
        .iter()
        .chain(record.shell.iter())
        .map(|(pid, _)| pid.to_string());
    // The outer shell exits at once; the timer runs in its own process group, ignores the
    // terminal's hangup and interrupt, and is adopted by the system.
    let out = Command::new("/bin/sh")
        .arg("-c")
        .arg("( trap '' HUP INT TERM; sleep \"$0\"; kill -CONT \"$@\" ) </dev/null >/dev/null 2>&1 & echo $!")
        .arg(seconds.to_string())
        .args(pids)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .output()
        .ok()?;
    let pid: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    let start = system.process(Pid::from_u32(pid))?.start_time();
    Some((pid, start))
}

/// Stop a watchdog that is no longer needed, with its timer.
#[cfg(unix)]
fn cancel(watchdog: Option<(u32, u64)>, system: &System) {
    let Some((pid, start)) = watchdog else { return };
    if system
        .process(Pid::from_u32(pid))
        .is_none_or(|p| p.start_time() != start)
    {
        return;
    }
    // Its own process group holds only the watchdog and its timer; never Jitterbug's.
    let group = unsafe { libc::getpgid(pid as i32) };
    if group > 1 && group != unsafe { libc::getpgrp() } {
        unsafe { libc::killpg(group, libc::SIGKILL) };
    }
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
    // Resuming must never continue part of another pause, or a job the person suspended.
    let held: HashSet<(u32, u64)> = registry()
        .paused
        .iter()
        .flat_map(|p| p.members.iter().copied())
        .collect();
    if tree(&system, root).iter().any(|m| held.contains(m)) {
        return Err("Part of this command is already paused.".into());
    }
    if root.status() == ProcessStatus::Stop {
        return Err("This command is already stopped. Nothing was paused.".into());
    }
    #[cfg(unix)]
    {
        let protected = protected(&system);
        let shell = root
            .parent()
            .and_then(|id| system.process(id))
            .filter(|parent| shell(parent) && crate::platform::same_user(parent))
            .filter(|parent| !protected.contains(&parent.pid().as_u32()))
            .filter(|parent| job_control(pid, parent.pid().as_u32()))
            .filter(|_| foreground(pid))
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
            // Undo everything stopped so far, shell last: the command itself was not paused.
            let check = self::system();
            for (mpid, mstart) in &members {
                signal(*mpid, *mstart, &check, Signal::Continue);
            }
            if let Some((spid, sstart)) = shell {
                if !held_shell(shell) {
                    signal(spid, sstart, &check, Signal::Continue);
                }
            }
            return Err("The operating system rejected the pause.".into());
        }
        let at = now();
        let mut record = Paused {
            pid,
            start_time,
            label: label.chars().take(120).collect(),
            members,
            shell,
            paused_at: at,
            until: at + LIMIT_SECONDS,
            watchdog: None,
        };
        record.watchdog = watchdog(&record, LIMIT_SECONDS + WATCHDOG_GRACE_SECONDS);
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

#[cfg(unix)]
fn system_tree(pid: u32, start: u64) -> Vec<(u32, u64)> {
    let system = system();
    system
        .process(Pid::from_u32(pid))
        .filter(|p| p.start_time() == start)
        .map(|root| tree(&system, root))
        .unwrap_or_default()
}

#[cfg(unix)]
/// Whether a pause still recorded keeps this shell stopped (two commands of one pipeline).
fn held_shell(shell: Option<(u32, u64)>) -> bool {
    shell.is_some() && registry().paused.iter().any(|p| p.shell == shell)
}

/// Continue records already removed from the registry. Returns how many processes resumed.
fn continue_records(records: &[Paused]) -> usize {
    #[cfg(unix)]
    {
        let system = system();
        let resumed = records
            .iter()
            .flat_map(|r| r.members.iter())
            .filter(|(pid, start)| signal(*pid, *start, &system, Signal::Continue))
            .count();
        // Shells resume last, after their commands are running again, and only once no other
        // pause still holds them.
        for record in records {
            if let Some((pid, start)) = record.shell.filter(|_| !held_shell(record.shell)) {
                signal(pid, start, &system, Signal::Continue);
            }
            cancel(record.watchdog, &system);
        }
        resumed
    }
    #[cfg(not(unix))]
    {
        let _ = records;
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
    let resumed = continue_records(std::slice::from_ref(&record));
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
    continue_records(&due);
    due.into_iter().map(|p| p.label).collect()
}

/// Resume everything; called when Jitterbug quits.
pub fn resume_all() {
    let all: Vec<Paused> = {
        let mut registry = registry();
        let all = std::mem::take(&mut registry.paused);
        save(&registry);
        all
    };
    continue_records(&all);
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
    continue_records(&left);
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn only_a_shell_running_the_command_as_its_own_job_is_held() {
        use std::os::unix::process::CommandExt;
        let me = std::process::id();
        // A job: the command leads its own group, apart from its parent's.
        let mut job = Command::new("sleep")
            .arg("5")
            .process_group(0)
            .spawn()
            .unwrap();
        assert!(job_control(job.id(), me));
        // A script shell's command shares the shell's group; that shell is never stopped.
        let mut plain = Command::new("sleep").arg("5").spawn().unwrap();
        assert!(!job_control(plain.id(), me));
        for child in [&mut job, &mut plain] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
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

    fn sleeper() -> (std::process::Child, u32, u64) {
        let child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        std::thread::sleep(Duration::from_millis(200));
        let start = system().process(Pid::from_u32(pid)).unwrap().start_time();
        (child, pid, start)
    }

    fn record(member: (u32, u64), shell: Option<(u32, u64)>) -> Paused {
        Paused {
            pid: member.0,
            start_time: member.1,
            label: "test".into(),
            members: vec![member],
            shell,
            paused_at: now(),
            until: now(),
            watchdog: None,
        }
    }

    #[test]
    fn stopped_jobs_and_parts_of_a_pause_are_not_paused_again() {
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30 & wait"])
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let pid = child.id();
        let start = system().process(Pid::from_u32(pid)).unwrap().start_time();
        let paused = pause(pid, start, "tree").unwrap();
        let (cpid, cstart) = paused.members[1];
        assert!(pause(cpid, cstart, "child")
            .unwrap_err()
            .contains("already paused"));
        resume(pid, start).unwrap();
        // A job the person suspended stays theirs: resuming it later would surprise them.
        let (mut job, jpid, jstart) = sleeper();
        Command::new("kill")
            .args(["-STOP", &jpid.to_string()])
            .status()
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(pause(jpid, jstart, "job")
            .unwrap_err()
            .contains("already stopped"));
        assert!(state(jpid).starts_with('T'));
        for c in [&mut child, &mut job] {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    #[test]
    fn a_shell_shared_by_two_pauses_resumes_only_after_both() {
        let (mut a, apid, astart) = sleeper();
        let (mut b, bpid, bstart) = sleeper();
        let (mut sh, spid, sstart) = sleeper();
        for p in [apid, bpid, spid] {
            Command::new("kill")
                .args(["-STOP", &p.to_string()])
                .status()
                .unwrap();
        }
        let shell = Some((spid, sstart));
        let other = record((bpid, bstart), shell);
        registry().paused.push(other.clone());
        continue_records(&[record((apid, astart), shell)]);
        std::thread::sleep(Duration::from_millis(100));
        assert!(!state(apid).starts_with('T'));
        assert!(
            state(spid).starts_with('T'),
            "the other pause still holds it"
        );
        registry().paused.retain(|p| p != &other);
        continue_records(&[other]);
        std::thread::sleep(Duration::from_millis(100));
        assert!(!state(bpid).starts_with('T') && !state(spid).starts_with('T'));
        for c in [&mut a, &mut b, &mut sh] {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    #[test]
    fn a_watchdog_resumes_work_on_its_own_and_cancels_cleanly() {
        let (mut a, apid, astart) = sleeper();
        let (mut b, bpid, bstart) = sleeper();
        for p in [apid, bpid] {
            Command::new("kill")
                .args(["-STOP", &p.to_string()])
                .status()
                .unwrap();
        }
        // Fires without Jitterbug's help, as after a crash.
        assert!(watchdog(&record((apid, astart), None), 1).is_some());
        // Cancelled, as when Jitterbug resumes the pause itself.
        let mut cancelled = record((bpid, bstart), None);
        cancelled.watchdog = watchdog(&cancelled, 1);
        let (wpid, _) = cancelled.watchdog.unwrap();
        cancel(cancelled.watchdog, &system());
        std::thread::sleep(Duration::from_millis(2500));
        assert!(!state(apid).starts_with('T'));
        assert!(state(bpid).starts_with('T'));
        assert!(state(wpid).is_empty(), "the cancelled watchdog is gone");
        for c in [&mut a, &mut b] {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
