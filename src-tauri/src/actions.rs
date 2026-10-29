use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind};

pub fn allowed(p: &Process, system: &System) -> bool {
    if !cfg!(target_os = "macos") || p.pid().as_u32() <= 100 || !crate::platform::same_user(p) {
        return false;
    }
    let mut parent = Some(Pid::from_u32(std::process::id()));
    for _ in 0..32 {
        if parent == Some(p.pid()) {
            return false;
        }
        parent = parent
            .and_then(|pid| system.process(pid))
            .and_then(Process::parent);
        if parent.is_none() {
            break;
        }
    }
    let name = p.name().to_string_lossy().to_lowercase();
    [
        "python",
        "node",
        "bun",
        "deno",
        "cargo",
        "rustc",
        "jitterbug-worker",
    ]
    .iter()
    .any(|name_prefix| name.starts_with(name_prefix))
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
    if !allowed(p, &system) {
        return Err("Actions are restricted to supported development processes owned by your user. Jitterbug, its ancestors, and system processes are protected.".into());
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unknown_actions_and_stale_identity() {
        assert!(execute(std::process::id(), 0, "shell").is_err());
        assert!(execute(std::process::id(), 0, "terminate").is_err());
        assert!(execute(1, 0, "lower_priority").is_err());
    }
}
