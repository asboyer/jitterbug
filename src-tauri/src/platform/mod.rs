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
