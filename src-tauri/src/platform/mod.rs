use crate::types::PlatformReadings;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", test))]
mod memory;
#[cfg(target_os = "windows")]
mod windows;

pub fn readings() -> PlatformReadings {
    #[cfg(target_os = "macos")]
    {
        macos::readings()
    }
    #[cfg(target_os = "windows")]
    {
        windows::readings()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        PlatformReadings {
            os: std::env::consts::OS.into(),
            ..Default::default()
        }
    }
}

/// Optional macOS window-server signal, not a test of individual web pages or network requests.
pub fn app_unresponsive(pid: u32) -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        macos::app_unresponsive(pid)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        None
    }
}

/// Cumulative kernel paging counters in bytes. Unavailable outside macOS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VmCounters {
    pub swap_in: u64,
    pub swap_out: u64,
    pub page_in: u64,
    pub compressed: u64,
    pub decompressed: u64,
}

pub fn vm_counters() -> Option<VmCounters> {
    #[cfg(target_os = "macos")]
    {
        macos::vm_counters()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

pub fn responsible_pid(pid: u32) -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        macos::responsible_pid(pid)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        None
    }
}

/// Native per-process CPU time for the pulse sampler (nanoseconds), checked against start time.
/// `None` means unavailable; callers fall back to sysinfo outside macOS.
#[cfg(target_os = "macos")]
pub fn process_cpu_ns(pid: u32, start_time: u64) -> Option<u64> {
    macos::process_cpu_ns(pid, start_time)
}

/// A process's physical footprint (resident plus compressed), checked against its start time.
/// `None` where unavailable; callers fall back to resident memory.
pub fn footprint(pid: u32, start_time: u64) -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        macos::footprint(pid, start_time)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (pid, start_time);
        None
    }
}

/// Whether macOS confirms the process is in background mode that another process set: the
/// kernel's flag plus a throttled base priority. `None` when the process exited, its PID was
/// reused, it cannot be read, or the platform has no background mode.
pub fn in_background(pid: u32, start_time: u64) -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        macos::in_background(pid, start_time)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (pid, start_time);
        None
    }
}

/// One process from a native all-process scan.
#[derive(Clone, Debug, Default)]
pub struct NativeProcess {
    pub pid: u32,
    /// Seconds since the epoch, as sysinfo reports it.
    pub start_time: u64,
    /// Microseconds since the epoch, for rates of processes that started this tick.
    pub start_us: u64,
    pub parent_pid: Option<u32>,
    pub cpu_ns: u64,
    /// Shared with the scan's identity cache, so a tick does not copy every name.
    pub name: std::sync::Arc<str>,
}

/// Every readable process with cumulative CPU time. Empty when unsupported (non-macOS).
pub fn all_processes() -> Vec<NativeProcess> {
    #[cfg(target_os = "macos")]
    {
        macos::all_processes()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// Parent PID from the kernel when the process table cannot read it (another user's process).
pub fn parent_pid(pid: u32) -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        macos::parent_pid(pid)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        None
    }
}

/// Outermost application bundle name for a live process, when the OS exposes its path.
pub fn bundle_name(pid: u32) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        macos::bundle_name(pid)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        None
    }
}

/// PID and display name of the foreground application, when the OS exposes one.
pub fn frontmost() -> Option<(u32, Option<String>)> {
    #[cfg(target_os = "macos")]
    {
        macos::frontmost()
    }
    #[cfg(target_os = "windows")]
    {
        windows::frontmost()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// Holds an OS activity assertion while a latency-sensitive sampler runs (macOS App Nap).
pub struct Activity {
    #[cfg(target_os = "macos")]
    _token: macos::Activity,
}

impl Activity {
    pub fn begin() -> Self {
        Self {
            #[cfg(target_os = "macos")]
            _token: macos::Activity::begin(),
        }
    }
}

pub fn same_user(process: &sysinfo::Process) -> bool {
    #[cfg(unix)]
    {
        process
            .user_id()
            .is_some_and(|uid| **uid == unsafe { libc::geteuid() })
    }
    #[cfg(not(unix))]
    {
        let _ = process;
        false
    }
}
