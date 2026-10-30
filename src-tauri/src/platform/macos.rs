use super::memory::{account, VmPages};
use crate::types::{MemoryBreakdown, PlatformReadings};
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::{
    NSActivityOptions, NSObjectProtocol, NSProcessInfo, NSProcessInfoThermalState, NSString,
};
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

fn statistics() -> Option<libc::vm_statistics64> {
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
    Some(stats)
}

fn memory() -> Option<MemoryBreakdown> {
    let stats = statistics()?;
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

/// Cumulative VM counters for the one-second pulse sampler. Only deltas are meaningful.
pub fn vm_counters() -> Option<super::VmCounters> {
    let stats = statistics()?;
    let page = unsafe { libc::vm_page_size } as u64;
    Some(super::VmCounters {
        swap_in: stats.swapins.checked_mul(page)?,
        swap_out: stats.swapouts.checked_mul(page)?,
        page_in: stats.pageins.checked_mul(page)?,
        compressed: stats.compressions.checked_mul(page)?,
        decompressed: stats.decompressions.checked_mul(page)?,
    })
}

/// Total CPU time of one process in nanoseconds, only while its start time still matches.
/// Two `proc_pidinfo` calls; processes owned by other users are unavailable without privileges.
#[allow(deprecated)] // Stable Mach timebase API; task times are in Mach ticks on Apple silicon.
pub fn process_cpu_ns(pid: u32, start_time: u64) -> Option<u64> {
    static TIMEBASE: OnceLock<(u64, u64)> = OnceLock::new();
    let (numer, denom) = *TIMEBASE.get_or_init(|| {
        let mut info = libc::mach_timebase_info { numer: 0, denom: 0 };
        let ok = unsafe { libc::mach_timebase_info(&mut info) } == libc::KERN_SUCCESS;
        if ok && info.denom > 0 {
            (info.numer as u64, info.denom as u64)
        } else {
            (1, 1)
        }
    });
    let mut bsd: libc::proc_bsdinfo = unsafe { mem::zeroed() };
    let size = mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let read = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut bsd as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if read != size || bsd.pbi_start_tvsec != start_time {
        return None;
    }
    let mut task: libc::proc_taskinfo = unsafe { mem::zeroed() };
    let size = mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    let read = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTASKINFO,
            0,
            (&mut task as *mut libc::proc_taskinfo).cast(),
            size,
        )
    };
    if read != size {
        return None;
    }
    let ticks = task.pti_total_user.saturating_add(task.pti_total_system) as u128;
    u64::try_from(ticks * numer as u128 / denom as u128).ok()
}

/// The app the user is looking at. NSRunningApplication properties are safe to read off-main.
pub fn frontmost() -> Option<(u32, Option<String>)> {
    let app = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = u32::try_from(app.processIdentifier()).ok()?;
    Some((pid, app.localizedName().map(|name| name.to_string())))
}

/// Keeps App Nap from coalescing the pulse timers while sampling. The display and system may
/// still sleep. Ends automatically when dropped on the sampler thread.
pub struct Activity(Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>);

impl Activity {
    pub fn begin() -> Self {
        let reason = NSString::from_str("Measuring short scheduling delays for Jitter Snapshot");
        Self(Some(
            NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
                &reason,
            ),
        ))
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        if let Some(token) = self.0.take() {
            unsafe { NSProcessInfo::processInfo().endActivity(&token) };
        }
    }
}
