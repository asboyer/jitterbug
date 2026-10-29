use super::memory::{account, VmPages};
use crate::types::{MemoryBreakdown, PlatformReadings};
use objc2_foundation::{NSProcessInfo, NSProcessInfoThermalState};
use std::{mem, sync::OnceLock};

fn memory() -> Option<MemoryBreakdown> {
    // Keep one host send right for this process, rather than leaking one each sample.
    static HOST: OnceLock<libc::mach_port_t> = OnceLock::new();
    #[allow(deprecated)] // libc exposes the stable Mach host API; no extra runtime is needed.
    let host = *HOST.get_or_init(|| unsafe { libc::mach_host_self() });
    let mut stats: libc::vm_statistics64 = unsafe { mem::zeroed() };
    // The stable revision ends before swapped_count; newer SDKs add fields after it.
    let mut count = (mem::offset_of!(libc::vm_statistics64, swapped_count)
        / mem::size_of::<libc::integer_t>()) as libc::mach_msg_type_number_t;
    let requested = count;
    let result = unsafe {
        libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            (&mut stats as *mut libc::vm_statistics64).cast(),
            &mut count,
        )
    };
    if result != libc::KERN_SUCCESS || count < requested {
        return None;
    }
    account(
        VmPages {
            active: stats.active_count.into(),
            inactive: stats.inactive_count.into(),
            speculative: stats.speculative_count.into(),
            wired: stats.wire_count.into(),
            compressed: stats.compressor_page_count.into(),
            purgeable: stats.purgeable_count.into(),
            external: stats.external_page_count.into(),
            swap_in: stats.swapins,
            swap_out: stats.swapouts,
        },
        unsafe { libc::vm_page_size } as u64,
    )
}

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
        memory: memory(),
    }
}
