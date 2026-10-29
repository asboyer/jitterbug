use crate::types::PlatformReadings;
use objc2_foundation::{NSProcessInfo, NSProcessInfoThermalState};

pub fn readings() -> PlatformReadings {
    let mut value: i32 = 0;
    let mut size = std::mem::size_of::<i32>();
    let result = unsafe {
        libc::sysctlbyname(
            c"kern.memorystatus_vm_pressure_level".as_ptr(),
            (&mut value as *mut i32).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    let memory_pressure = if result == 0 {
        match value {
            1 => Some("normal"),
            2 => Some("warning"),
            4 => Some("critical"),
            _ => None,
        }
        .map(str::to_string)
    } else {
        None
    };
    let state = NSProcessInfo::processInfo().thermalState();
    let thermal = if state == NSProcessInfoThermalState::Nominal {
        "nominal"
    } else if state == NSProcessInfoThermalState::Fair {
        "fair"
    } else if state == NSProcessInfoThermalState::Serious {
        "serious"
    } else {
        "critical"
    };
    PlatformReadings {
        os: "macos".into(),
        memory_pressure,
        thermal: Some(thermal.into()),
        process_actions: true,
    }
}
