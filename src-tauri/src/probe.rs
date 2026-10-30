//! Short, on-demand stack checks. Raw samples never enter model context or the UI.
use crate::{diagnostics::Target, types::Snapshot};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StackCheck {
    pub role: String,
    pub state: String,
    pub summary: String,
    pub captured_at: u64,
}

fn unavailable(role: &str, reason: &str) -> StackCheck {
    StackCheck {
        role: role.into(),
        state: "unavailable".into(),
        summary: reason.into(),
        captured_at: crate::types::now(),
    }
}

/// Analyze only the main thread. A normal run-loop wait is not a deadlock.
/// Only fixed categories leave this parser; symbols, paths and report headers do not.
#[cfg(any(target_os = "macos", test))]
fn classify(report: &str, role: &str) -> StackCheck {
    let mut main = false;
    let mut total = 0u32;
    let mut counts = [0u32; 5];
    for line in report.lines() {
        if line.contains("Thread_") {
            if main {
                break;
            }
            main = line.contains("com.apple.main-thread");
            if main {
                total = line
                    .split_whitespace()
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
            }
            continue;
        }
        if !main {
            continue;
        }
        let frame = line.trim_start_matches(|c: char| c.is_whitespace() || "+!:|".contains(c));
        let count = frame
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        for (index, matches) in [
            frame.contains("IPC::Connection::waitFor")
                || frame.contains("IPC::Connection::sendSync"),
            [
                "__psynch_mutexwait",
                "__psynch_cvwait",
                "__ulock_wait",
                "_dispatch_semaphore_wait_slow",
                "_dispatch_sync_f_slow",
            ]
            .iter()
            .any(|s| frame.contains(s)),
            frame.contains("nanosleep") || frame.contains("sleepForTimeInterval"),
            ["__read_nocancel", "__connect", "__recvfrom", "__sendto"]
                .iter()
                .any(|s| frame.contains(s)),
            frame.contains("__CFRunLoopServiceMachPort"),
        ]
        .iter()
        .enumerate()
        {
            if *matches {
                counts[index] = counts[index].max(count);
            }
        }
    }
    if total == 0 {
        return unavailable(
            role,
            "The sample did not expose a recognizable main thread.",
        );
    }
    let result = counts.iter().position(|count| *count >= total.div_ceil(2));
    let (state, summary) = match result {
        Some(0) => ("synchronous_ipc", "The main thread spent most of the one-second sample waiting for another process to reply. The dependency, not CPU load alone, needs investigation."),
        Some(1) => ("lock_wait", "The main thread spent most of the one-second sample waiting on a lock or synchronization. This is a waiting pattern, not proof of a deadlock."),
        Some(2) => ("sleep", "The main thread spent most of the one-second sample sleeping. A sleeping UI thread cannot handle input during that wait."),
        Some(3) => ("io_wait", "The main thread spent most of the one-second sample in an input/output call. This does not identify the file, server, or reason for the wait."),
        Some(4) => ("event_loop_wait", "The main thread was mostly waiting for app events, a normal idle pattern. No persistent main-thread block was captured; this does not rule out a stuck tab or an earlier freeze."),
        _ => ("unclassified", "The one-second sample did not show a dominant known wait pattern. It does not establish why the app felt slow."),
    };
    StackCheck {
        role: role.into(),
        state: state.into(),
        summary: summary.into(),
        captured_at: crate::types::now(),
    }
}

pub async fn inspect(snapshot: &Snapshot, target: &Target, directory: &Path) -> Vec<StackCheck> {
    if snapshot.source != "live"
        || !snapshot.sample_ready
        || crate::types::now().saturating_sub(snapshot.captured_at) > 20
    {
        return vec![];
    }
    #[cfg(target_os = "macos")]
    {
        if snapshot.platform.thermal.as_deref() == Some("critical") {
            return vec![unavailable(
                "Application",
                "Stack sampling was skipped while thermal pressure is critical.",
            )];
        }
        let members: Vec<_> = snapshot
            .processes
            .iter()
            .filter(|p| {
                p.workload_id == target.workload_id && target.pid.is_none_or(|id| p.pid == id)
            })
            .collect();
        if members.is_empty() {
            return vec![];
        }
        let main = crate::details::representative(&members);
        let mut selected = vec![(main.clone(), "Application")];
        if target.pid.is_none() {
            if let Some(helper) = members.iter().filter(|p| p.pid != main.pid).max_by(|a, b| {
                a.cpu_percent
                    .total_cmp(&b.cpu_percent)
                    .then(a.memory_bytes.cmp(&b.memory_bytes))
            }) {
                selected.push(((*helper).clone(), "Busiest helper"));
            }
        }
        let directory = directory.to_path_buf();
        tokio::task::spawn_blocking(move || {
            static SINGLE: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let Ok(_guard) = SINGLE.try_lock() else {
                return vec![unavailable(
                    "Application",
                    "Another stack check is running. Try again shortly.",
                )];
            };
            selected
                .iter()
                .map(|(process, role)| sample(process, role, &directory))
                .collect()
        })
        .await
        .unwrap_or_else(|_| {
            vec![unavailable(
                "Application",
                "The stack check could not finish.",
            )]
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (target, directory);
        vec![unavailable(
            "Application",
            "Stack checks are not available on this platform yet.",
        )]
    }
}

#[cfg(target_os = "macos")]
fn sample(process: &crate::types::ProcessRecord, role: &str, directory: &Path) -> StackCheck {
    use std::{
        fs::OpenOptions,
        io::Read,
        os::unix::fs::OpenOptionsExt,
        process::{Command, Stdio},
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    fn same_identity(expected: &crate::types::ProcessRecord) -> bool {
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(expected.pid)]),
            true,
            ProcessRefreshKind::nothing()
                .with_user(UpdateKind::Always)
                .with_exe(UpdateKind::Always),
        );
        system
            .process(Pid::from_u32(expected.pid))
            .is_some_and(|p| {
                p.start_time() == expected.start_time
                    && p.exe() == expected.executable.as_deref()
                    && crate::platform::same_user(p)
            })
    }
    if process.pid <= 100 || process.pid == std::process::id() || !same_identity(process) {
        return unavailable(
            role,
            "The process exited, changed identity, or cannot be sampled by this user.",
        );
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = directory.join(format!(
        "stack-check-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let Ok(mut file) = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
    else {
        return unavailable(
            role,
            "A private temporary sample file could not be created.",
        );
    };
    let _cleanup = Cleanup(path.clone());
    let Ok(mut child) = Command::new("/usr/bin/sample")
        .args([
            process.pid.to_string(),
            "1".into(),
            "10".into(),
            "-file".into(),
        ])
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return unavailable(role, "macOS sampling is unavailable.");
    };
    let start = Instant::now();
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
            Ok(None) => (),
        }
        if start.elapsed() > Duration::from_secs(5)
            || file.metadata().is_ok_and(|m| m.len() > 2 * 1024 * 1024)
        {
            let _ = child.kill();
            let _ = child.wait();
            return unavailable(role, "The stack check reached its time or size limit.");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    if !success || !same_identity(process) {
        return unavailable(
            role,
            "macOS denied sampling, or the process exited during the check.",
        );
    }
    let mut report = String::new();
    if file
        .by_ref()
        .take(2 * 1024 * 1024 + 1)
        .read_to_string(&mut report)
        .is_err()
        || report.len() > 2 * 1024 * 1024
    {
        return unavailable(role, "The stack report was unreadable or too large.");
    }
    classify(&report, role)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_run_loop_is_not_a_hang_and_worker_locks_are_not_ui_locks() {
        let result = classify("100 Thread_1 DispatchQueue_1: com.apple.main-thread\n+ 93 __CFRunLoopServiceMachPort (in CoreFoundation)\n100 Thread_2\n+ 100 __psynch_mutexwait", "Application");
        assert_eq!(result.state, "event_loop_wait");
        assert!(!result.summary.contains("Thread_"));
    }
    #[test]
    fn requires_majority_and_does_not_export_private_report_data() {
        let result = classify("Path: /private/example\n100 Thread_1 com.apple.main-thread\n+ 10 IPC::Connection::waitForSyncReply\n+ 90 nanosleep", "Application");
        assert_eq!(result.state, "sleep");
        assert!(!serde_json::to_string(&result).unwrap().contains("/private"));
        assert_eq!(
            classify("unrecognized report", "Application").state,
            "unavailable"
        );
    }
    #[test]
    fn synchronous_dependency_wait_takes_precedence_over_its_nested_run_loop() {
        let result = classify("100 Thread_1 com.apple.main-thread\n+ 80 IPC::Connection::waitForSyncReply\n+ 80 __CFRunLoopServiceMachPort", "Application");
        assert_eq!(result.state, "synchronous_ipc");
    }
}
