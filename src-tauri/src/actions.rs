use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind};

pub fn permitted(p: &Process, system: &System) -> Vec<String> {
    if !cfg!(target_os = "macos") || p.pid().as_u32() <= 100 || !crate::platform::same_user(p) {
        return vec![];
    }
    let mut parent = Some(Pid::from_u32(std::process::id()));
    for _ in 0..32 {
        if parent == Some(p.pid()) {
            return vec![];
        }
        parent = parent
            .and_then(|pid| system.process(pid))
            .and_then(Process::parent);
        if parent.is_none() {
            break;
        }
    }
    if let Some(exe) = p.exe().filter(|exe| {
        exe.ancestors()
            .any(|p| p.extension().is_some_and(|e| e == "app"))
    }) {
        return application_actions(exe);
    }
    let name = p.name().to_string_lossy().to_lowercase();
    let development = [
        "python",
        "node",
        "bun",
        "deno",
        "cargo",
        "rustc",
        "jitterbug-worker",
    ]
    .iter()
    .any(|name_prefix| name.starts_with(name_prefix));
    if development {
        return vec!["lower_priority".into(), "terminate".into()];
    }
    p.exe().map(application_actions).unwrap_or_default()
}

fn application_actions(exe: &std::path::Path) -> Vec<String> {
    if ["/System", "/usr", "/bin", "/sbin", "/Library"]
        .iter()
        .any(|prefix| exe.starts_with(prefix))
    {
        return vec![];
    }
    let Some(bundle) = exe
        .ancestors()
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
        .last()
    else {
        return vec![];
    };
    if bundle
        .file_stem()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("jitterbug"))
    {
        return vec![];
    }
    let mut actions = vec!["lower_priority".into()];
    // Helpers may be reprioritized, but stopping arbitrary app helpers is not a useful repair.
    if exe.parent() == Some(bundle.join("Contents/MacOS").as_path()) {
        actions.push("terminate".into());
    }
    actions
}

/// Runs a supported action after rechecking fresh process identity and permission. On macOS,
/// lowering priority puts the process in background mode: lower CPU priority, throttled disk
/// and network I/O, and efficiency cores. Unlike a nice value, it can be undone without
/// administrator access.
pub fn execute(pid: u32, start_time: u64, action: &str) -> Result<String, String> {
    if !matches!(action, "terminate" | "lower_priority") {
        return Err("Unsupported process action.".into());
    }
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_user(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    let p = system
        .process(Pid::from_u32(pid))
        .ok_or("Process has already exited.")?;
    if p.start_time() != start_time {
        return Err("Process identity changed. Refresh before taking an action.".into());
    }
    if !permitted(p, &system).iter().any(|a| a == action) {
        return Err("This action is unavailable for this process. Actions require a supported development process or application owned by your user. Jitterbug, its ancestors, and system processes are protected.".into());
    }
    if action == "terminate" {
        return match p.kill_with(Signal::Term) { Some(true) => Ok("Termination requested. A process may handle or ignore this request; watch the next sample.".into()), _ => Err("The operating system rejected the termination request.".into()) };
    }
    match set(pid, true) {
        Ok(()) => Ok("Moved to background mode: it gets CPU, disk, and network after your other apps. This does not release memory.".into()),
        Err(Refusal::Gone) => Err("Process has already exited.".into()),
        Err(_) => Err("The operating system rejected the priority change.".into()),
    }
}

/// Why a background-mode change was not made. Nothing was changed in every case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The process exited, or its PID now belongs to another process.
    Gone,
    /// Another user's process, or (when entering) one Jitterbug never changes: itself, its
    /// ancestors, system processes, and unsupported executables.
    NotAllowed,
    /// The OS refused, or the platform has no background mode.
    Failed,
}

/// Puts processes in macOS background mode (`on`) or takes them out of it. Every target is
/// rechecked first with one fresh process scan: same PID and start time, owned by this user,
/// and, to enter background mode, permitted to lower priority. Returns one result per target.
pub fn set_background(targets: &[(u32, u64)], on: bool) -> Vec<Result<(), Refusal>> {
    if targets.is_empty() {
        return vec![];
    }
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_user(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    targets
        .iter()
        .map(|&(pid, start_time)| {
            let p = system
                .process(Pid::from_u32(pid))
                .filter(|p| p.start_time() == start_time)
                .ok_or(Refusal::Gone)?;
            let allowed = if on {
                permitted(p, &system).iter().any(|a| a == "lower_priority")
            } else {
                // Leaving background mode only restores the default, but still never touches
                // another user's process.
                crate::platform::same_user(p)
            };
            if !allowed {
                return Err(Refusal::NotAllowed);
            }
            set(pid, on)
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn set(pid: u32, on: bool) -> Result<(), Refusal> {
    let value = if on { libc::PRIO_DARWIN_BG } else { 0 };
    unsafe {
        if libc::setpriority(libc::PRIO_DARWIN_PROCESS, pid, value) == 0 {
            return Ok(());
        }
        match *libc::__error() {
            libc::ESRCH => Err(Refusal::Gone),
            _ => Err(Refusal::Failed),
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn set(pid: u32, on: bool) -> Result<(), Refusal> {
    // Process actions are macOS-only for now: `permitted` offers none elsewhere.
    let _ = (pid, on);
    Err(Refusal::Failed)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[test]
    fn application_helpers_only_offer_priority_changes() {
        use std::path::Path;
        assert_eq!(
            application_actions(Path::new(
                "/Applications/Example.app/Contents/MacOS/Example"
            ))
            .len(),
            2
        );
        assert_eq!(
            application_actions(Path::new(
                "/Applications/Example.app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper"
            )),
            ["lower_priority"]
        );
        assert!(application_actions(Path::new(
            "/System/Applications/Example.app/Contents/MacOS/Example"
        ))
        .is_empty());
        assert!(application_actions(Path::new(
            "/Applications/Jitterbug.app/Contents/MacOS/jitterbug"
        ))
        .is_empty());
    }
    #[test]
    fn rejects_unknown_actions_and_stale_identity() {
        assert!(execute(std::process::id(), 0, "shell").is_err());
        assert!(execute(std::process::id(), 0, "terminate").is_err());
        assert!(execute(1, 0, "lower_priority").is_err());
    }

    /// A process this test starts and kills, never anyone else's: this test binary, linked
    /// under a development tool's name so the same permission check as real targets allows it,
    /// running only `stand_in` below. (A copied system binary would be killed by macOS.)
    pub(crate) struct Child(std::process::Child, std::path::PathBuf);
    impl Child {
        pub(crate) fn spawn() -> (Self, u32, u64) {
            use std::sync::atomic::{AtomicU32, Ordering};
            static COUNT: AtomicU32 = AtomicU32::new(0);
            let directory = std::env::temp_dir().join(format!(
                "jitterbug-bg-test-{}-{}",
                std::process::id(),
                COUNT.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir_all(&directory).unwrap();
            let program = directory.join("python-bg-test");
            let current = std::env::current_exe().unwrap();
            if std::fs::hard_link(&current, &program).is_err() {
                std::fs::copy(&current, &program).unwrap();
            }
            let child = std::process::Command::new(&program)
                .args(["--exact", "actions::tests::stand_in", "--test-threads=1"])
                .env("JITTERBUG_STAND_IN", "1")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let pid = child.id();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let start = loop {
                let mut system = System::new();
                system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
                if let Some(process) = system.process(Pid::from_u32(pid)) {
                    break process.start_time();
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "stand-in process did not start"
                );
                std::thread::sleep(std::time::Duration::from_millis(20));
            };
            (Self(child, directory), pid, start)
        }
    }
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    /// Idle stand-in process for `Child`; returns at once in a normal test run.
    #[test]
    fn stand_in() {
        if std::env::var_os("JITTERBUG_STAND_IN").is_some() {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    }

    #[test]
    fn background_mode_rechecks_identity_and_skips_exited_processes() {
        let (child, pid, start) = Child::spawn();
        // A reused PID is another process: it is never touched.
        assert_eq!(
            set_background(&[(pid, start + 1)], true),
            [Err(Refusal::Gone)]
        );
        assert_eq!(
            crate::platform::in_background(pid, start),
            cfg!(target_os = "macos").then_some(false)
        );
        // Jitterbug itself is never put in background mode.
        let own = std::process::id();
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(own)]), true);
        let own_start = system.process(Pid::from_u32(own)).unwrap().start_time();
        assert_eq!(
            set_background(&[(own, own_start)], true),
            [Err(Refusal::NotAllowed)]
        );
        drop(child);
        assert_eq!(set_background(&[(pid, start)], false), [Err(Refusal::Gone)]);
        assert_eq!(crate::platform::in_background(pid, start), None);
    }

    /// The live check: background mode through Jitterbug's own path on a process this test
    /// started, confirmed by the readback and by `ps`, then cleared without privileges.
    #[cfg(target_os = "macos")]
    #[test]
    fn background_mode_is_applied_confirmed_and_cleared_without_privileges() {
        let (_child, pid, start) = Child::spawn();
        let ps = || {
            let out = std::process::Command::new("/bin/ps")
                .args(["-o", "pri=", "-p", &pid.to_string()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse::<i32>()
                .unwrap()
        };
        let wait_for_priority = |expected| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                let actual = ps();
                if actual == expected {
                    return;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "priority stayed at {actual}, expected {expected}"
                );
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        let before = ps();
        assert!(before > 4, "{before}");
        assert_eq!(set_background(&[(pid, start)], true), [Ok(())]);
        assert_eq!(crate::platform::in_background(pid, start), Some(true));
        wait_for_priority(4);
        assert_eq!(set_background(&[(pid, start)], false), [Ok(())]);
        assert_eq!(crate::platform::in_background(pid, start), Some(false));
        wait_for_priority(before);
        // The single-process action uses the same mechanism.
        execute(pid, start, "lower_priority").unwrap();
        assert_eq!(crate::platform::in_background(pid, start), Some(true));
    }
}
