use crate::types::PlatformReadings;

#[cfg(target_os = "macos")]
mod macos;
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
