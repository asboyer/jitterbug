//! Exact CoreGraphics mode restore; requests permanent scope without claiming reboot verification.
use super::{
    hardware::{self, DisplayMode as Mode, DisplayState},
    HardwareValue,
};
#[repr(C)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
type ConfigRef = *mut c_void;
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
    fn CGBeginDisplayConfiguration(config: *mut ConfigRef) -> i32;
    fn CGConfigureDisplayWithDisplayMode(
        config: ConfigRef,
        display: u32,
        mode: ModeRef,
        options: CFTypeRef,
    ) -> i32;
    fn CGCompleteDisplayConfiguration(config: ConfigRef, option: u32) -> i32;
    fn CGCancelDisplayConfiguration(config: ConfigRef) -> i32;
    fn CGDisplayBounds(display: u32) -> Rect;
    fn CGDisplayRotation(display: u32) -> f64;
    fn CGDisplayIsMain(display: u32) -> u32;
    fn CGDisplayIsActive(display: u32) -> u32;
    fn CGDisplayMirrorsDisplay(display: u32) -> u32;
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
pub(in crate::system_settings) fn annotate(items: &mut [crate::system_settings::Capability]) {
    if let Some(item) = items.iter_mut().find(|v| v.id == "display_mode") {
        item.status = crate::system_settings::CapabilityStatus::Unavailable;
        item.detail = match collect() {
            Err(error) => format!("Automatic executor cannot capture this configuration: {error}. Recheck hardware support and access; no guessed values are saved."),
            Ok(_) => "Hardware changed during discovery. Recheck before capture.".into(),
        };
        item.verification =
            "Not captured; automatic executor requires readable supported native descriptors"
                .into();
    }
}

fn supported_mode(id: u32, target: &Mode) -> Result<CFType, String> {
    // SAFETY: copied array owns mode refs until we retain the exact matching descriptor.
    let raw = unsafe { CGDisplayCopyAllDisplayModes(id, ptr::null()) };
    if raw.is_null() {
        return Err("Display mode enumeration unavailable".into());
    }
    let array = unsafe { CFType::wrap_under_create_rule(raw) };
    let count = unsafe { CFArrayGetCount(array.as_CFTypeRef()) };
    if !(1..=4096).contains(&count) {
        return Err("Invalid supported mode count".into());
    }
    for i in 0..count {
        let raw = unsafe { CFArrayGetValueAtIndex(array.as_CFTypeRef(), i) };
        if decode(raw)? == *target {
            return Ok(unsafe { CFType::wrap_under_get_rule(raw) });
        }
    }
    Err("Saved full display mode is no longer enumerated as supported; no guessed refresh/resolution substitution".into())
}
fn inspect() -> Result<(Vec<u32>, HardwareValue), String> {
    let ids = online()?;
    let mut pairs = Vec::new();
    for id in &ids {
        let uuid = identity(*id)?;
        let mode = current(*id)?;
        supported_mode(*id, &mode)?;
        // SAFETY: public read-only display queries; IDs revalidated below.
        let bounds = unsafe { CGDisplayBounds(*id) };
        let mirror = unsafe { CGDisplayMirrorsDisplay(*id) };
        let state = DisplayState {
            uuid: uuid.clone(),
            mode,
            x: bounds.x,
            y: bounds.y,
            rotation: unsafe { CGDisplayRotation(*id) },
            main: unsafe { CGDisplayIsMain(*id) } != 0,
            active: unsafe { CGDisplayIsActive(*id) } != 0,
            mirror: if mirror == 0 {
                None
            } else {
                Some(identity(mirror)?)
            },
        };
        if identity(*id)? != uuid || current(*id)? != state.mode {
            return Err("Display identity/mode changed during inspection".into());
        }
        pairs.push((*id, state));
    }
    if online()? != ids {
        return Err("Display topology changed during inspection".into());
    }
    pairs.sort_by(|a, b| a.1.uuid.cmp(&b.1.uuid));
    let handles = pairs.iter().map(|p| p.0).collect();
    let value = HardwareValue::Displays {
        displays: pairs.into_iter().map(|p| p.1).collect(),
    };
    value.validate(1)?;
    Ok((handles, value))
}
pub(super) fn collect() -> Result<HardwareValue, String> {
    inspect().map(|(_, v)| v)
}
struct Configuration(ConfigRef);
impl Drop for Configuration {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CGCancelDisplayConfiguration(self.0);
            }
        }
    }
}
fn cg_status(status: i32, operation: &str) -> Result<(), String> {
    if status == 0 {
        Ok(())
    } else {
        Err(format!("CoreGraphics could not {operation} (CGError {status}). Configuration may be unsupported, access denied, or another application may own a full-screen display. Recheck and re-preview; no security grant was requested."))
    }
}
pub(super) fn preflight(target: &HardwareValue) -> Result<(), String> {
    target.validate(1)?;
    let (ids, live) = inspect()?;
    if !live.same_identity(target) {
        return Err("Saved display UUIDs/topology unavailable".into());
    }
    let HardwareValue::Displays { displays } = target else {
        return Err("Wrong display target type".into());
    };
    for (id, display) in ids.iter().zip(displays) {
        supported_mode(*id, &display.mode)?;
    }
    Ok(())
}
pub(super) fn write(expected: &HardwareValue, target: &HardwareValue) -> Result<(), String> {
    target.validate(1)?;
    hardware::execute_verified(
        expected,
        target,
        || {
            let (ids, live) = inspect()?;
            if !live.same_identity(target) {
                return Err("Saved display UUIDs/topology no longer match".into());
            }
            let HardwareValue::Displays { displays } = target else {
                return Err("Wrong display target type".into());
            };
            let modes = ids
                .iter()
                .zip(displays)
                .map(|(id, state)| supported_mode(*id, &state.mode))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(((ids, modes), live))
        },
        |(ids, modes)| {
            // Re-read after mode enumeration and before opening a native configuration.
            let (fresh_ids, fresh) = inspect()?;
            if fresh_ids != ids || fresh != *expected {
                return Err("Display identity/topology/mode changed before configuration".into());
            }
            let HardwareValue::Displays { displays: before } = expected else {
                unreachable!()
            };
            let HardwareValue::Displays { displays: after } = target else {
                unreachable!()
            };
            let mut raw = ptr::null_mut();
            cg_status(
                unsafe { CGBeginDisplayConfiguration(&mut raw) },
                "begin display configuration",
            )?;
            if raw.is_null() {
                return Err("CoreGraphics returned no configuration".into());
            }
            let mut config = Configuration(raw);
            for (((id, mode), old), new) in ids.iter().zip(&modes).zip(before).zip(after) {
                if identity(*id)? != new.uuid {
                    return Err("Display replaced before mode configuration".into());
                }
                if old.mode != new.mode {
                    cg_status(
                        unsafe {
                            CGConfigureDisplayWithDisplayMode(
                                config.0,
                                *id,
                                mode.as_CFTypeRef(),
                                ptr::null(),
                            )
                        },
                        "configure saved display mode",
                    )?;
                }
            }
            let (fresh_ids, fresh) = inspect()?;
            if fresh_ids != ids || fresh != *expected {
                return Err("Display changed before commit; configuration canceled".into());
            }
            // SDK: completion consumes config even on failure. 2 = kCGConfigurePermanently;
            // macOS may fall back to login-session scope, so readback never proves reboot persistence.
            let raw = std::mem::replace(&mut config.0, ptr::null_mut());
            cg_status(
                unsafe { CGCompleteDisplayConfiguration(raw, 2) },
                "commit permanent-request display configuration",
            )
        },
        collect,
        || std::thread::sleep(std::time::Duration::from_millis(50)),
        40,
    )
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
