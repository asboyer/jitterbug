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
    #[cfg(target_os = "macos")]
    {
        let current = unsafe { libc::getpriority(libc::PRIO_PROCESS, pid) };
        let result = unsafe { libc::setpriority(libc::PRIO_PROCESS, pid, current.max(10)) };
        if result == 0 {
            Ok("CPU scheduling priority lowered. This does not release memory.".into())
        } else {
            Err("The operating system rejected the priority change.".into())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Process actions are not implemented for this platform yet.".into())
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
}
