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

/// A priority change the OS accepted: the nice value before and after.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lowered {
    pub previous: i32,
    pub now: i32,
}

/// Runs a supported action. A priority change also returns the values it changed, unless the
/// process was already at or below the target and nothing changed.
pub fn execute(
    pid: u32,
    start_time: u64,
    action: &str,
) -> Result<(String, Option<Lowered>), String> {
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
        return match p.kill_with(Signal::Term) { Some(true) => Ok(("Termination requested. A process may handle or ignore this request; watch the next sample.".into(), None)), _ => Err("The operating system rejected the termination request.".into()) };
    }
    #[cfg(target_os = "macos")]
    {
        let current = unsafe { libc::getpriority(libc::PRIO_PROCESS, pid) };
        let target = current.max(10);
        let result = unsafe { libc::setpriority(libc::PRIO_PROCESS, pid, target) };
        if result == 0 {
            Ok((
                "CPU scheduling priority lowered. This does not release memory.".into(),
                (target != current).then_some(Lowered {
                    previous: current,
                    now: target,
                }),
            ))
        } else {
            Err("The operating system rejected the priority change.".into())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Process actions are not implemented for this platform yet.".into())
    }
}

/// Why a priority could not be restored.
#[derive(Debug, PartialEq, Eq)]
pub enum RestoreError {
    /// The process exited, or its PID now belongs to another process: nothing to restore.
    Gone,
    /// The OS refused: macOS lets only an administrator raise a priority again.
    Denied,
    Failed(String),
}

/// Sets a process back to the priority it had before Jitterbug lowered it. Like every action,
/// it rechecks fresh process identity and ownership first and never requests privileges.
pub fn restore(pid: u32, start_time: u64, previous: i32) -> Result<(), RestoreError> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        ProcessRefreshKind::nothing().with_user(UpdateKind::Always),
    );
    let p = system
        .process(Pid::from_u32(pid))
        .filter(|p| p.start_time() == start_time)
        .ok_or(RestoreError::Gone)?;
    if !crate::platform::same_user(p) {
        return Err(RestoreError::Failed(
            "This process belongs to another user. Nothing was changed.".into(),
        ));
    }
    #[cfg(target_os = "macos")]
    unsafe {
        *libc::__error() = 0;
        let current = libc::getpriority(libc::PRIO_PROCESS, pid);
        if *libc::__error() != 0 {
            return Err(RestoreError::Gone);
        }
        // Already back where it was, or above: nothing to undo.
        if current <= previous {
            return Ok(());
        }
        if libc::setpriority(libc::PRIO_PROCESS, pid, previous) == 0 {
            return Ok(());
        }
        match *libc::__error() {
            libc::EACCES | libc::EPERM => Err(RestoreError::Denied),
            libc::ESRCH => Err(RestoreError::Gone),
            _ => Err(RestoreError::Failed(
                "The operating system rejected the priority change.".into(),
            )),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = previous;
        Err(RestoreError::Failed(
            "Process actions are not implemented for this platform yet.".into(),
        ))
    }
}

/// Read OS priority only for the same process identity; never request elevated privileges.
pub fn priority(pid: u32, start_time: u64) -> Result<i32, String> {
    #[cfg(target_os = "macos")]
    {
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
            ProcessRefreshKind::nothing(),
        );
        if system
            .process(Pid::from_u32(pid))
            .is_none_or(|p| p.start_time() != start_time)
        {
            return Err("Process identity changed or exited.".into());
        }
        unsafe {
            *libc::__error() = 0;
            let value = libc::getpriority(libc::PRIO_PROCESS, pid);
            if *libc::__error() != 0 {
                return Err("Cannot read OS scheduling priority.".into());
            }
            Ok(value)
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (pid, start_time);
        Err("OS priority verification is unavailable on this platform.".into())
    }
}

#[cfg(test)]
mod tests {
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

    struct Child(std::process::Child);
    impl Child {
        fn spawn() -> (Self, u32, u64) {
            let child = std::process::Command::new("/bin/cat")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let pid = child.id();
            let mut system = System::new();
            system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
            let start = system.process(Pid::from_u32(pid)).unwrap().start_time();
            (Self(child), pid, start)
        }
    }
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[cfg(target_os = "macos")]
    fn nice(pid: u32) -> i32 {
        unsafe { libc::getpriority(libc::PRIO_PROCESS, pid) }
    }

    #[test]
    fn restore_rechecks_identity_and_skips_exited_processes() {
        let (child, pid, start) = Child::spawn();
        #[cfg(target_os = "macos")]
        unsafe {
            assert_eq!(libc::setpriority(libc::PRIO_PROCESS, pid, 12), 0);
        }
        // A reused PID is another process: it is never touched.
        assert_eq!(restore(pid, start + 1, 15), Err(RestoreError::Gone));
        #[cfg(target_os = "macos")]
        assert_eq!(nice(pid), 12);
        drop(child);
        assert_eq!(restore(pid, start, 15), Err(RestoreError::Gone));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn restore_sets_the_recorded_value_or_reports_the_os_refusal() {
        let (_child, pid, start) = Child::spawn();
        unsafe {
            assert_eq!(libc::setpriority(libc::PRIO_PROCESS, pid, 15), 0);
        }
        // Already back at or above the recorded value: nothing to do, nothing changes.
        assert_eq!(restore(pid, start, 15), Ok(()));
        assert_eq!(restore(pid, start, 18), Ok(()));
        assert_eq!(nice(pid), 15);
        // Raising a priority back needs an administrator unless Jitterbug runs as root.
        let raised = restore(pid, start, 0);
        if unsafe { libc::geteuid() } == 0 {
            assert_eq!(raised, Ok(()));
            assert_eq!(nice(pid), 0);
        } else {
            assert_eq!(raised, Err(RestoreError::Denied));
            assert_eq!(nice(pid), 15);
        }
    }
}
