//! Exact system scheduling overrides, not backup status or a guessed factory frequency.
use super::HardwareValue;
use core_foundation::{
    base::{CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    number::CFNumber,
    string::{CFString, CFStringRef},
};
use std::{fs::File, io::Read};
const PATH: &str = "/Library/Preferences/com.apple.TimeMachine.plist";
const LIMIT: u64 = 1024 * 1024;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFPreferencesAnyUser: CFStringRef;
    static kCFPreferencesAnyHost: CFStringRef;
    fn CFPreferencesSetValue(
        key: CFStringRef,
        value: CFTypeRef,
        app: CFStringRef,
        user: CFStringRef,
        host: CFStringRef,
    );
    fn CFPreferencesSynchronize(app: CFStringRef, user: CFStringRef, host: CFStringRef) -> u8;
    fn CFPreferencesAppValueIsForced(key: CFStringRef, app: CFStringRef) -> u8;
}

fn decode(d: &plist::Dictionary) -> Result<HardwareValue, String> {
    let automatic = d
        .get("AutoBackup")
        .map(|v| v.as_boolean().ok_or("AutoBackup is not a Boolean"))
        .transpose()?;
    let interval_seconds = d
        .get("AutoBackupInterval")
        .map(|v| {
            v.as_unsigned_integer()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or("AutoBackupInterval is not a supported integer")
        })
        .transpose()?;
    let value = HardwareValue::TimeMachine {
        automatic,
        interval_seconds,
    };
    value.validate(120)?;
    Ok(value)
}

pub(super) fn preflight() -> Result<(), String> {
    if !super::preferences::supported_version() {
        return Err("Time Machine scheduling storage is validated only on macOS 13–15.".into());
    }
    let app = CFString::new("com.apple.TimeMachine");
    for key in ["AutoBackup", "AutoBackupInterval"] {
        // SAFETY: concrete application identifier, valid retained CF strings. Never overwrite MDM policy.
        if unsafe {
            CFPreferencesAppValueIsForced(
                CFString::new(key).as_concrete_TypeRef(),
                app.as_concrete_TypeRef(),
            )
        } != 0
        {
            return Err("Time Machine scheduling is managed by your organization; contact the administrator.".into());
        }
    }
    Ok(())
}

pub(super) fn read() -> Result<HardwareValue, String> {
    preflight()?;
    // Unlike a NULL CFPreferences result, an open failure cannot be mistaken for an absent key.
    let file = File::open(PATH).map_err(|e| format!("Cannot read Time Machine scheduling preferences: {e}. Full Disk Access for Jitterbug may be required; missing configuration is not an automatic/off value."))?;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Time Machine preference read failed: {e}"))?;
    if bytes.len() as u64 > LIMIT {
        return Err("Time Machine preferences exceed capture size bound".into());
    }
    let plist = plist::Value::from_reader(std::io::Cursor::new(bytes))
        .map_err(|_| "Invalid Time Machine preference plist")?;
    decode(
        plist
            .as_dictionary()
            .ok_or("Invalid Time Machine preference dictionary")?,
    )
}

fn write_with(
    before: &HardwareValue,
    target: &HardwareValue,
    mut read: impl FnMut() -> Result<HardwareValue, String>,
    set: impl FnOnce(Option<bool>, Option<u32>) -> Result<(), String>,
) -> Result<(), String> {
    target.validate(120)?;
    if !before.same_identity(target) || read()? != *before {
        return Err("Time Machine schedule changed before write; refresh preview".into());
    }
    let HardwareValue::TimeMachine {
        automatic,
        interval_seconds,
    } = target
    else {
        return Err("Wrong Time Machine target".into());
    };
    set(*automatic, *interval_seconds)?;
    if read()? != *target {
        return Err(
            "Time Machine schedule storage readback mismatch; state may have changed".into(),
        );
    }
    Ok(())
}

pub(super) fn write(before: &HardwareValue, target: &HardwareValue) -> Result<(), String> {
    preflight()?;
    write_with(before, target, read, |automatic, interval| {
        let app = CFString::new("com.apple.TimeMachine");
        // Synchronize the exact system scope before staging this finite pair. No system file replacement.
        let sync = || unsafe { CFPreferencesSynchronize(app.as_concrete_TypeRef(), kCFPreferencesAnyUser, kCFPreferencesAnyHost) } != 0;
        if !sync() {
            return Err("Time Machine synchronization denied; external system authorization and Full Disk Access are required".into());
        }
        for (key, value) in [
            (
                "AutoBackup",
                automatic.map(|v| CFBoolean::from(v).as_CFType()),
            ),
            (
                "AutoBackupInterval",
                interval.map(|v| CFNumber::from(i64::from(v)).as_CFType()),
            ),
        ] {
            // SAFETY: two fixed keys, exact system/any-host scope; NULL restores only the absent override.
            unsafe {
                CFPreferencesSetValue(
                    CFString::new(key).as_concrete_TypeRef(),
                    value
                        .as_ref()
                        .map_or(std::ptr::null(), CFType::as_CFTypeRef),
                    app.as_concrete_TypeRef(),
                    kCFPreferencesAnyUser,
                    kCFPreferencesAnyHost,
                );
            }
        }
        if !sync() {
            return Err("Time Machine save denied or incomplete; state may have changed. Recheck external system authorization and Full Disk Access.".into());
        }
        Ok(())
    })
}

pub(super) fn annotate(items: &mut [super::Capability]) {
    if let Some(item) = items.iter_mut().find(|v| v.id == "time_machine") {
        item.status = super::CapabilityStatus::Unavailable;
        item.detail = read()
            .err()
            .unwrap_or_else(|| "No typed schedule captured; recheck access".into());
        item.verification = "Exact AutoBackup + AutoBackupInterval storage only; not backup-running status. An implemented writer requires a captured typed original and external system authorization; no backup or deletion is run.".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decode_preserves_typed_absence_and_rejects_strings() {
        let mut d = plist::Dictionary::new();
        d.insert("AutoBackup".into(), plist::Value::Boolean(false));
        d.insert(
            "AutoBackupInterval".into(),
            plist::Value::Integer(604800.into()),
        );
        assert_eq!(
            decode(&d).unwrap(),
            HardwareValue::TimeMachine {
                automatic: Some(false),
                interval_seconds: Some(604800)
            }
        );
        d.remove("AutoBackupInterval");
        assert_eq!(
            decode(&d).unwrap(),
            HardwareValue::TimeMachine {
                automatic: Some(false),
                interval_seconds: None
            }
        );
        d.insert("AutoBackup".into(), plist::Value::String("false".into()));
        assert!(decode(&d).is_err());
    }
    #[test]
    fn coupled_schedule_write_rechecks_and_verifies_without_running_a_backup() {
        use std::cell::RefCell;
        let before = HardwareValue::TimeMachine {
            automatic: Some(false),
            interval_seconds: Some(86400),
        };
        let target = HardwareValue::TimeMachine {
            automatic: None,
            interval_seconds: Some(604800),
        };
        let state = RefCell::new(before.clone());
        write_with(
            &before,
            &target,
            || Ok(state.borrow().clone()),
            |a, i| {
                *state.borrow_mut() = HardwareValue::TimeMachine {
                    automatic: a,
                    interval_seconds: i,
                };
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*state.borrow(), target);
        assert!(write_with(
            &before,
            &target,
            || Ok(target.clone()),
            |_, _| panic!("stale write")
        )
        .is_err());
        assert!(
            write_with(&before, &target, || Ok(before.clone()), |_, _| Ok(()))
                .unwrap_err()
                .contains("readback")
        );
        assert!(write_with(
            &before,
            &target,
            || Ok(before.clone()),
            |_, _| Err("denied".into())
        )
        .unwrap_err()
        .contains("denied"));
    }
}
