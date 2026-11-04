//! Process-owned idle-sleep assertion, not a persistent power preference.
//! Public IOKit API: IOPMLib.h. No shell, privileges, or other process assertions.
use std::{ffi::c_void, sync::Mutex};

type CF = *const c_void;
#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPMAssertionCreateWithName(kind: CF, level: u32, name: CF, id: *mut u32) -> i32;
    fn IOPMAssertionCopyProperties(id: u32) -> CF;
    fn IOPMAssertionRelease(id: u32) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(allocator: CF, text: *const i8, encoding: u32) -> CF;
    fn CFRelease(value: CF);
    fn CFDictionaryGetValue(dictionary: CF, key: CF) -> CF;
    fn CFGetTypeID(value: CF) -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(number: CF, kind: i32, value: *mut i32) -> bool;
}
struct OwnedCF(CF);
impl OwnedCF {
    fn string(text: &std::ffi::CStr) -> Result<Self, String> {
        // SAFETY: static NUL-terminated UTF-8 string, default allocator.
        let value =
            unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), 0x08000100) };
        if value.is_null() {
            Err("Could not allocate power assertion text.".into())
        } else {
            Ok(Self(value))
        }
    }
}
impl Drop for OwnedCF {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

#[derive(Default)]
pub(crate) struct IdleSleep(Mutex<Option<u32>>);
impl IdleSleep {
    pub fn read(&self) -> Result<bool, String> {
        let id = self
            .0
            .lock()
            .map_err(|_| "Power assertion lock unavailable.")?;
        match *id {
            Some(id) => Self::active(id),
            None => Ok(false),
        }
    }
    fn active(id: u32) -> Result<bool, String> {
        // Copy queries power management's live assertion, not a defaults preference.
        let properties = unsafe { IOPMAssertionCopyProperties(id) };
        if properties.is_null() {
            return Err("Owned power assertion runtime readback unavailable.".into());
        }
        let properties = OwnedCF(properties);
        let key = OwnedCF::string(c"AssertLevel")?;
        let level = unsafe { CFDictionaryGetValue(properties.0, key.0) };
        let mut value = 0_i32;
        if level.is_null()
            || unsafe {
                CFGetTypeID(level) != CFNumberGetTypeID() || !CFNumberGetValue(level, 3, &mut value)
            }
        {
            return Err("Power assertion level could not be verified.".into());
        }
        Ok(value == 255)
    }
    pub fn apply(&self, enabled: bool) -> Result<bool, String> {
        let mut owned = self
            .0
            .lock()
            .map_err(|_| "Power assertion lock unavailable.")?;
        if enabled {
            if let Some(id) = *owned {
                return Self::active(id);
            }
            let kind = OwnedCF::string(c"PreventUserIdleSystemSleep")?;
            let name = OwnedCF::string(c"Jitterbug explicitly permitted keep-awake session")?;
            let mut id = 0;
            let status = unsafe { IOPMAssertionCreateWithName(kind.0, 255, name.0, &mut id) };
            if status != 0 {
                return Err("macOS rejected the idle-sleep assertion.".into());
            }
            // Retain ownership even when readback fails, so pause/recovery can release it.
            *owned = Some(id);
            Self::active(id)
        } else if let Some(id) = *owned {
            if unsafe { IOPMAssertionRelease(id) } != 0 {
                return Err("macOS did not release the owned idle-sleep assertion.".into());
            }
            let remaining = unsafe { IOPMAssertionCopyProperties(id) };
            if !remaining.is_null() {
                unsafe { CFRelease(remaining) };
                return Err("Released power assertion still has runtime properties.".into());
            }
            *owned = None;
            Ok(false)
        } else {
            Ok(false)
        }
    }
}
impl Drop for IdleSleep {
    fn drop(&mut self) {
        if let Ok(Some(id)) = self.0.get_mut() {
            unsafe { IOPMAssertionRelease(*id) };
        }
        // macOS also releases process-owned assertions on process exit/crash.
    }
}
