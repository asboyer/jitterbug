use super::{Preference, PreferenceValue};
use core_foundation::{
    base::{CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    number::{CFNumber, CFNumberIsFloatType},
    string::{CFString, CFStringRef},
};
use std::ptr;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFPreferencesAnyApplication: CFStringRef;
    static kCFPreferencesCurrentApplication: CFStringRef;
    static kCFPreferencesCurrentUser: CFStringRef;
    static kCFPreferencesAnyHost: CFStringRef;
    fn CFPreferencesCopyValue(
        key: CFStringRef,
        app: CFStringRef,
        user: CFStringRef,
        host: CFStringRef,
    ) -> CFTypeRef;
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

fn encode_value(value: PreferenceValue) -> Option<CFType> {
    match value {
        PreferenceValue::Absent => None,
        PreferenceValue::Boolean(v) => Some(CFBoolean::from(v).as_CFType()),
        PreferenceValue::Integer(v) => Some(CFNumber::from(v).as_CFType()),
        PreferenceValue::Real(v) => Some(CFNumber::from(v).as_CFType()),
    }
}

fn decode_value(object: Option<&CFType>) -> Result<PreferenceValue, String> {
    let Some(object) = object else {
        return Ok(PreferenceValue::Absent);
    };
    if let Some(boolean) = object.downcast::<CFBoolean>() {
        return Ok(PreferenceValue::Boolean(boolean.into()));
    }
    if let Some(number) = object.downcast::<CFNumber>() {
        // SAFETY: type-checked CFNumber, alive for this call.
        return if unsafe { CFNumberIsFloatType(number.as_concrete_TypeRef()) } != 0 {
            number
                .to_f64()
                .map(PreferenceValue::Real)
                .ok_or_else(|| "Preference real number cannot be represented exactly.".into())
        } else {
            number
                .to_i64()
                .map(PreferenceValue::Integer)
                .ok_or_else(|| "Preference integer cannot be represented exactly.".into())
        };
    }
    Err("Unsupported property-list type; not captured or writable.".into())
}

fn domain(key: Preference) -> CFString {
    if key.descriptor().0 == "NSGlobalDomain" {
        // SAFETY: framework-owned constant, retained by the get-rule wrapper.
        unsafe { CFString::wrap_under_get_rule(kCFPreferencesAnyApplication) }
    } else {
        CFString::new(key.descriptor().0)
    }
}

fn synchronize(app: &CFString) -> Result<(), String> {
    // SAFETY: valid CF strings and framework constants, no pending writes during discovery.
    let ok = unsafe {
        CFPreferencesSynchronize(
            app.as_concrete_TypeRef(),
            kCFPreferencesCurrentUser,
            kCFPreferencesAnyHost,
        )
    };
    if ok != 0 {
        Ok(())
    } else {
        Err("macOS denied preference synchronization. Recheck access; no permission is granted by Jitterbug.".into())
    }
}

pub fn read(key: Preference) -> Result<PreferenceValue, String> {
    let major =
        sysinfo::System::os_version().and_then(|v| v.split('.').next()?.parse::<u32>().ok());
    if !super::supported_version() || (key == Preference::MouseLinear && major == Some(13)) {
        return Err("Preference is not validated on this OS version; mouse linear motion requires macOS 14–15.".into());
    }
    let app = domain(key);
    let name = CFString::new(key.descriptor().1);
    synchronize(&app)?;
    // App-level APIs forbid AnyApplication. The current application's search chain
    // includes global managed preferences; primitive exact-domain reads below still use AnyApplication.
    let managed_app = if key.descriptor().0 == "NSGlobalDomain" {
        unsafe { CFString::wrap_under_get_rule(kCFPreferencesCurrentApplication) }
    } else {
        app.clone()
    };
    // SAFETY: initialized immutable strings; managed preferences must never be overridden.
    if unsafe {
        CFPreferencesAppValueIsForced(
            name.as_concrete_TypeRef(),
            managed_app.as_concrete_TypeRef(),
        )
    } != 0
    {
        return Err("Read-only: managed by your organization. Contact your administrator; Jitterbug cannot grant access.".into());
    }
    // SAFETY: CopyValue returns an owned property list or null for this exact domain.
    let raw = unsafe {
        CFPreferencesCopyValue(
            name.as_concrete_TypeRef(),
            app.as_concrete_TypeRef(),
            kCFPreferencesCurrentUser,
            kCFPreferencesAnyHost,
        )
    };
    // SAFETY: non-null CopyValue result is owned; the wrapper releases it exactly once.
    let object = (!raw.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(raw) });
    let value = decode_value(object.as_ref())?;
    key.validate(value)?;
    Ok(value)
}

pub fn write(
    key: Preference,
    expected: PreferenceValue,
    value: PreferenceValue,
) -> Result<(), String> {
    key.validate(value)?;
    if read(key)? != expected {
        return Err("Preference changed immediately before writing; refresh the preview.".into());
    }
    let app = domain(key);
    let name = CFString::new(key.descriptor().1);
    let object = encode_value(value);
    // SAFETY: allowlisted key/domain, current-user any-host only. Null removes the override,
    // rather than writing a guessed factory default. The owned value lives through the call.
    unsafe {
        CFPreferencesSetValue(
            name.as_concrete_TypeRef(),
            object.as_ref().map_or(ptr::null(), TCFType::as_CFTypeRef),
            app.as_concrete_TypeRef(),
            kCFPreferencesCurrentUser,
            kCFPreferencesAnyHost,
        );
    }
    synchronize(&app)?;
    if read(key)? != value {
        return Err("Saved preference readback mismatch; live effect is not verified.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_property_list_conversion_preserves_absence_boolean_integer_and_real() {
        for value in [
            PreferenceValue::Absent,
            PreferenceValue::Boolean(true),
            PreferenceValue::Boolean(false),
            PreferenceValue::Integer(6),
            PreferenceValue::Real(0.6875),
        ] {
            let object = encode_value(value);
            assert_eq!(decode_value(object.as_ref()).unwrap(), value);
        }
        assert!(decode_value(Some(&CFString::new("true").as_CFType())).is_err());
    }
}
