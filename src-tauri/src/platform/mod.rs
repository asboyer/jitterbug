use crate::types::PlatformReadings;

#[cfg(target_os = "macos")]
mod macos;

pub fn readings() -> PlatformReadings {
    #[cfg(target_os = "macos")]
    {
        macos::readings()
    }
    #[cfg(not(target_os = "macos"))]
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
