use crate::types::PlatformReadings;

/// Process and memory collection already comes from the shared sysinfo collector.
/// Native pressure/thermal signals and validated process actions are the next Windows increment.
pub fn readings() -> PlatformReadings {
    PlatformReadings {
        os: "windows".into(),
        memory_pressure: None,
        thermal: None,
        process_actions: false,
        memory: None,
    }
}

pub fn frontmost() -> Option<(u32, Option<String>)> {
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };
    let mut pid = 0u32;
    unsafe {
        let window = GetForegroundWindow();
        if window.0.is_null() {
            return None;
        }
        GetWindowThreadProcessId(window, Some(&mut pid as *mut u32));
    }
    // The name comes from the collected inventory, not from the window title.
    (pid != 0).then_some((pid, None))
}
