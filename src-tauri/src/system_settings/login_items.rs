//! Bounded predefined System Events program. No supplied script, shell, launch, or TCC writes.
use super::{hardware::LoginItem, HardwareValue};
use std::{
    io::Read,
    os::fd::AsRawFd,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[repr(C)]
struct AeDesc {
    kind: u32,
    data: *mut std::ffi::c_void,
}
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AECreateDesc(
        kind: u32,
        data: *const std::ffi::c_void,
        size: isize,
        result: *mut AeDesc,
    ) -> i32;
    fn AEDisposeDesc(desc: *mut AeDesc) -> i32;
    fn AEDeterminePermissionToAutomateTarget(
        target: *const AeDesc,
        class: u32,
        id: u32,
        ask: u8,
    ) -> i32;
}

fn authorization(write: bool) -> Result<(), String> {
    if !super::preferences::supported_version() {
        return Err("Login inventory is validated only on macOS 13–15".into());
    }
    let bundle = b"com.apple.systemevents";
    let mut desc = AeDesc {
        kind: 0,
        data: std::ptr::null_mut(),
    };
    // SAFETY: bounded bundle bytes and correctly sized owned AEDesc. ask=false never requests consent.
    if unsafe {
        AECreateDesc(
            u32::from_be_bytes(*b"bund"),
            bundle.as_ptr().cast(),
            bundle.len() as isize,
            &mut desc,
        )
    } != 0
    {
        return Err("Cannot address System Events automation".into());
    }
    let events: &[[u8; 4]] = if write {
        &[*b"getd", *b"delo", *b"crel"]
    } else {
        &[*b"getd"]
    };
    let result = events.iter().try_for_each(|event| {
        let status = unsafe { AEDeterminePermissionToAutomateTarget(&desc, u32::from_be_bytes(*b"core"), u32::from_be_bytes(*event), 0) };
        if status == 0 {Ok(())} else {Err(format!("System Events Automation permission is not externally authorized (OS status {status}). Approve Jitterbug → System Events in Privacy & Security > Automation, then recheck. No consent prompt or TCC write was attempted."))}
    });
    unsafe {
        AEDisposeDesc(&mut desc);
    }
    result
}

fn run(args: &[String]) -> Result<Vec<LoginItem>, String> {
    authorization(args.first().is_some_and(|v| v == "write"))?;
    let mut child = Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", include_str!("login_items.js")])
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Cannot start fixed login adapter: {e}"))?;
    let mut out = child.stdout.take().ok_or("Missing login adapter output")?;
    if unsafe { libc::fcntl(out.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } == -1 {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Cannot bound login adapter".into());
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    let result = (|| {
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            match out.read(&mut chunk) {
                Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => (),
                Err(_) => return Err("Login adapter output failed".into()),
            }
            if bytes.len() > 48 * 1024 || Instant::now() >= deadline {
                return Err(
                    "Login adapter size/time bound exceeded; state may have changed".into(),
                );
            }
            if let Some(status) = child
                .try_wait()
                .map_err(|_| "Login adapter process failed")?
            {
                if !status.success() {
                    return Err("System Events rejected login capture/restore: missing Automation permission, an unsupported/non-application item, duplicate identity, or inventory drift. Recheck access and preview; earlier registration changes may have applied. Background service approvals are not part of this adapter.".into());
                }
                out.take(48 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| "Incomplete login adapter output")?;
                if bytes.len() > 48 * 1024 {
                    return Err("Login inventory too large".into());
                }
                let items: Vec<LoginItem> =
                    serde_json::from_slice(&bytes).map_err(|_| "Malformed login adapter output")?;
                HardwareValue::LoginItems {
                    items: items.clone(),
                }
                .validate(130)?;
                return Ok(items);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

pub(super) fn read() -> Result<HardwareValue, String> {
    Ok(HardwareValue::LoginItems {
        items: run(&["read".into()])?,
    })
}

pub(super) fn preflight(target: &HardwareValue) -> Result<(), String> {
    target.validate(130)?;
    let HardwareValue::LoginItems { items } = target else {
        return Err("Wrong login target".into());
    };
    authorization(true)?;
    for item in items {
        if !std::path::Path::new(&item.path)
            .join("Contents/Info.plist")
            .is_file()
        {
            return Err("A saved login application is no longer installed at its captured path; reinstall it yourself and re-preview".into());
        }
    }
    Ok(())
}

pub(super) fn write(before: &HardwareValue, target: &HardwareValue) -> Result<(), String> {
    preflight(target)?;
    let (HardwareValue::LoginItems { items: expected }, HardwareValue::LoginItems { items: saved }) =
        (before, target)
    else {
        return Err("Wrong login target types".into());
    };
    // Script repeats comparison directly before each operation; controller already fsynced applying intent.
    let actual = run(&[
        "write".into(),
        serde_json::to_string(expected).map_err(|e| e.to_string())?,
        serde_json::to_string(saved).map_err(|e| e.to_string())?,
    ])?;
    if actual != *saved || read()? != *target {
        return Err("Login inventory fresh readback mismatch".into());
    }
    Ok(())
}

pub(super) fn annotate(items: &mut [super::Capability]) {
    if let Some(item) = items.iter_mut().find(|v| v.id == "login_items") {
        item.status = super::CapabilityStatus::Unavailable;
        item.detail = read()
            .err()
            .unwrap_or_else(|| "Login inventory was not captured; recheck access".into());
        item.verification = "Implemented fixed System Events reader/writer for user-added application registrations and hidden flags only. No SMAppService approval replay or application launch.".into();
    }
}
