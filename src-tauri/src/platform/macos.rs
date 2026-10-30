use super::memory::{account, VmPages};
use crate::types::{MemoryBreakdown, PlatformReadings};
use objc2_foundation::{NSProcessInfo, NSProcessInfoThermalState};
use std::{mem, sync::OnceLock};

// Optional private, read-only OS signals. Resolve at runtime so an OS change yields
// "unavailable", never a guessed healthy state or a launch-time linker failure.
// ABI references: NUIKit/CGSInternal (CGSEvent.h), Objective-See FileMonitor.
#[repr(C)]
struct ProcessSerialNumber {
    high: u32,
    low: u32,
}
type ProcessSerial = unsafe extern "C" fn(i32, *mut ProcessSerialNumber) -> i32;
type Connection = unsafe extern "C" fn() -> i32;
type Unresponsive = unsafe extern "C" fn(i32, *const ProcessSerialNumber) -> bool;

pub fn app_unresponsive(pid: u32) -> Option<bool> {
    static API: OnceLock<Option<(ProcessSerial, Connection, Unresponsive)>> = OnceLock::new();
    let (serial, connection, unresponsive) = API.get_or_init(|| unsafe {
        // Retain the system framework handle for the lifetime of its function pointers.
        let handle = libc::dlopen(c"/System/Library/Frameworks/ApplicationServices.framework/Frameworks/HIServices.framework/HIServices".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if handle.is_null() { return None; }
        let serial = libc::dlsym(handle, c"GetProcessForPID".as_ptr());
        let connection = libc::dlsym(handle, c"CGSMainConnectionID".as_ptr());
        let unresponsive = libc::dlsym(handle, c"CGSEventIsAppUnresponsive".as_ptr());
        if serial.is_null() || connection.is_null() || unresponsive.is_null() { return None; }
        Some((mem::transmute::<*mut libc::c_void, ProcessSerial>(serial),
            mem::transmute::<*mut libc::c_void, Connection>(connection),
            mem::transmute::<*mut libc::c_void, Unresponsive>(unresponsive)))
    }).as_ref()?;
    let mut psn = ProcessSerialNumber { high: 0, low: 0 };
    let cid = unsafe { connection() };
    if cid == 0 || unsafe { serial(i32::try_from(pid).ok()?, &mut psn) } != 0 {
        return None;
    }
    Some(unsafe { unresponsive(cid, &psn) })
}

pub fn responsible_pid(pid: u32) -> Option<u32> {
    type Responsible = unsafe extern "C" fn(i32) -> i32;
    static FUNCTION: OnceLock<Option<Responsible>> = OnceLock::new();
    let function = FUNCTION
        .get_or_init(|| unsafe {
            let ptr = libc::dlsym(
                libc::RTLD_DEFAULT,
                c"responsibility_get_pid_responsible_for_pid".as_ptr(),
            );
            if ptr.is_null() {
                None
            } else {
                Some(mem::transmute::<*mut libc::c_void, Responsible>(ptr))
            }
        })
        .as_ref()?;
    let owner = unsafe { function(i32::try_from(pid).ok()?) };
    (owner > 1 && owner as u32 != pid).then_some(owner as u32)
}

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
