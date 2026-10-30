//! Read-only full-mode inventory. Never offers a transient display write as persistent rollback.
#[derive(Clone, Debug, PartialEq)]
struct Mode {
    id: i32,
    width: usize,
    height: usize,
    pixel_width: usize,
    pixel_height: usize,
    refresh_hz: f64,
    flags: u32,
    desktop_usable: bool,
}
fn validate_mode(mode: &Mode) -> Result<(), String> {
    if mode.width == 0
        || mode.height == 0
        || mode.pixel_width == 0
        || mode.pixel_height == 0
        || !mode.refresh_hz.is_finite()
        || mode.refresh_hz < 0.0
    {
        return Err("Invalid full display mode; observation unavailable.".into());
    }
    Ok(())
}

use core_foundation::{
    base::{CFType, CFTypeRef, TCFType},
    string::{CFString, CFStringRef},
};
use std::{ffi::c_void, ptr};
type ModeRef = *const c_void;
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetOnlineDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayCreateUUIDFromDisplayID(display: u32) -> CFTypeRef;
    fn CGDisplayCopyDisplayMode(display: u32) -> ModeRef;
    fn CGDisplayCopyAllDisplayModes(display: u32, options: CFTypeRef) -> CFTypeRef;
    fn CGDisplayModeGetWidth(mode: ModeRef) -> usize;
    fn CGDisplayModeGetHeight(mode: ModeRef) -> usize;
    fn CGDisplayModeGetPixelWidth(mode: ModeRef) -> usize;
    fn CGDisplayModeGetPixelHeight(mode: ModeRef) -> usize;
    fn CGDisplayModeGetRefreshRate(mode: ModeRef) -> f64;
    fn CGDisplayModeGetIOFlags(mode: ModeRef) -> u32;
    fn CGDisplayModeGetIODisplayModeID(mode: ModeRef) -> i32;
    fn CGDisplayModeIsUsableForDesktopGUI(mode: ModeRef) -> bool;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFUUIDCreateString(allocator: CFTypeRef, uuid: CFTypeRef) -> CFStringRef;
    fn CFArrayGetCount(array: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
}
fn online() -> Result<Vec<u32>, String> {
    let mut ids = [0; 32];
    let mut count = 0;
    // SAFETY: bounded initialized UInt32 buffer and valid count pointer.
    if unsafe { CGGetOnlineDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) } != 0
        || count == 0
        || count as usize >= ids.len()
    {
        return Err("Display inventory unavailable or exceeds safe bound.".into());
    }
    let mut ids = ids[..count as usize].to_vec();
    ids.sort_unstable();
    Ok(ids)
}
fn identity(id: u32) -> Result<String, String> {
    // SAFETY: Create returns an owned UUID or null, RAII releases it.
    let raw = unsafe { CGDisplayCreateUUIDFromDisplayID(id) };
    if raw.is_null() {
        return Err("No stable display UUID.".into());
    }
    let uuid = unsafe { CFType::wrap_under_create_rule(raw) };
    let raw = unsafe { CFUUIDCreateString(ptr::null(), uuid.as_CFTypeRef()) };
    if raw.is_null() {
        return Err("Display UUID cannot be represented.".into());
    }
    Ok(unsafe { CFString::wrap_under_create_rule(raw) }.to_string())
}
fn decode(raw: ModeRef) -> Result<Mode, String> {
    if raw.is_null() {
        return Err("Display mode unavailable.".into());
    }
    // SAFETY: callers hold the copied mode or its containing copied array alive.
    let mode = unsafe {
        Mode {
            id: CGDisplayModeGetIODisplayModeID(raw),
            width: CGDisplayModeGetWidth(raw),
            height: CGDisplayModeGetHeight(raw),
            pixel_width: CGDisplayModeGetPixelWidth(raw),
            pixel_height: CGDisplayModeGetPixelHeight(raw),
            refresh_hz: CGDisplayModeGetRefreshRate(raw),
            flags: CGDisplayModeGetIOFlags(raw),
            desktop_usable: CGDisplayModeIsUsableForDesktopGUI(raw),
        }
    };
    validate_mode(&mode)?;
    Ok(mode)
}
fn current(id: u32) -> Result<Mode, String> {
    let raw = unsafe { CGDisplayCopyDisplayMode(id) };
    if raw.is_null() {
        return Err("Current display mode unavailable.".into());
    }
    let owned = unsafe { CFType::wrap_under_create_rule(raw) };
    decode(owned.as_CFTypeRef())
}
fn observe() -> Result<String, String> {
    let ids = online()?;
    let mut identities = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        let uuid = identity(*id)?;
        if !identities.insert(uuid.clone()) {
            return Err("Ambiguous display identity.".into());
        }
        let mode = current(*id)?;
        let raw = unsafe { CGDisplayCopyAllDisplayModes(*id, ptr::null()) };
        if raw.is_null() {
            return Err("Supported display modes unavailable.".into());
        }
        let array = unsafe { CFType::wrap_under_create_rule(raw) };
        let count = unsafe { CFArrayGetCount(array.as_CFTypeRef()) };
        if !(1..=4096).contains(&count) {
            return Err("Invalid supported mode count.".into());
        }
        let mut supported = false;
        for i in 0..count {
            let candidate = decode(unsafe { CFArrayGetValueAtIndex(array.as_CFTypeRef(), i) })?;
            supported |= candidate == mode;
        }
        if identity(*id)? != uuid || current(*id)? != mode {
            return Err("Display identity or full mode changed during discovery.".into());
        }
        // Identity is checked but never exposed or written into coverage details.
        let rate = if mode.refresh_hz == 0.0 {
            "unknown/variable refresh (API reports 0)".into()
        } else {
            format!("{} Hz", mode.refresh_hz)
        };
        result.push(format!("Display {}: {}×{} points, {}×{} pixels, {rate}, mode ID {}, flags {:#x}, desktop usable {}, exact mode in {} supported modes: {}; stable UUID checked", index + 1, mode.width, mode.height, mode.pixel_width, mode.pixel_height, mode.id, mode.flags, mode.desktop_usable, count, supported));
    }
    if online()? != ids {
        return Err("Display inventory changed during discovery.".into());
    }
    Ok(result.join("; "))
}
pub(super) fn annotate(items: &mut [super::Capability]) {
    let Some(item) = items.iter_mut().find(|v| v.id == "display_mode") else {
        return;
    };
    match observe() {
        Ok(detail) => {
            item.status = super::CapabilityStatus::ReadOnly;
            item.detail = format!("Live observation only: {detail}. {}", item.detail);
            item.verification =
                "Full CoreGraphics mode observation; persistent restore gated, never included"
                    .into();
        }
        Err(error) => {
            item.status = super::CapabilityStatus::Unavailable;
            item.detail = format!("Unknown: {error} {}", item.detail);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_mode_keeps_identity_resolution_flags_and_unknown_refresh() {
        let mode = Mode {
            id: 1,
            width: 1000,
            height: 800,
            pixel_width: 2000,
            pixel_height: 1600,
            refresh_hz: 0.0,
            flags: 3,
            desktop_usable: true,
        };
        assert!(validate_mode(&mode).is_ok());
        for changed in [
            Mode {
                id: 2,
                ..mode.clone()
            },
            Mode {
                pixel_width: 1000,
                ..mode.clone()
            },
            Mode {
                flags: 1,
                ..mode.clone()
            },
        ] {
            assert_ne!(mode, changed);
        }
        assert!(validate_mode(&Mode {
            refresh_hz: f64::NAN,
            ..mode.clone()
        })
        .is_err());
        assert!(validate_mode(&Mode { width: 0, ..mode }).is_err());
    }
}
