//! Prepared-pane Accessibility adapter. Never opens a pane, grants consent, types, or clicks coordinates.
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
struct Control {
    id: String,
    value: Option<bool>,
    enabled: bool,
    pressable: bool,
}

fn select(nodes: &[Control]) -> Result<BTreeMap<String, bool>, String> {
    let mut counts = BTreeMap::new();
    for n in nodes {
        *counts.entry(&n.id).or_insert(0) += 1;
    }
    let values: BTreeMap<_, _> = nodes
        .iter()
        .filter(|n| {
            n.id.starts_with("background-switch-")
                && n.id.len() <= 512
                && counts[&n.id] == 1
                && n.enabled
                && n.pressable
        })
        .filter_map(|n| n.value.map(|v| (n.id.clone(), v)))
        .collect();
    if values.is_empty() || values.len() > 64 {
        return Err("No unique, enabled, readable background switches in the prepared pane. Duplicate labels, managed/disabled controls and unknown values are never captured.".into());
    }
    Ok(values)
}

fn restore_with(
    before: &BTreeMap<String, bool>,
    target: &BTreeMap<String, bool>,
    mut read: impl FnMut() -> Result<BTreeMap<String, bool>, String>,
    mut press: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    if before.keys().ne(target.keys()) || read()? != *before {
        return Err(
            "Background inventory or approval changed; prepare the pane and refresh preview".into(),
        );
    }
    let mut expected = before.clone();
    for (id, value) in target {
        if expected[id] == *value {
            continue;
        }
        if read()? != expected {
            return Err("Background inventory changed before toggle".into());
        }
        press(id)?;
        expected.insert(id.clone(), *value);
        if read()? != expected {
            return Err("Background switch readback mismatch; state may have changed or macOS may require user authentication. No prompt is clicked.".into());
        }
    }
    Ok(())
}

use core_foundation::{
    array::CFArray,
    base::{CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    number::CFNumber,
    string::{CFString, CFStringRef},
};
use objc2_app_kit::NSRunningApplication;
use objc2_foundation::NSString;
use std::time::{Duration, Instant};
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementCopyActionNames(element: CFTypeRef, names: *mut CFTypeRef) -> i32;
    fn AXUIElementPerformAction(element: CFTypeRef, action: CFStringRef) -> i32;
    fn AXUIElementSetMessagingTimeout(element: CFTypeRef, seconds: f32) -> i32;
}
fn attr(element: &CFType, name: &str) -> Option<CFType> {
    let mut value = std::ptr::null();
    // SAFETY: live retained AX element and immutable attribute name. Copy result is owned.
    let status = unsafe {
        AXUIElementCopyAttributeValue(
            element.as_CFTypeRef(),
            CFString::new(name).as_concrete_TypeRef(),
            &mut value,
        )
    };
    (status == 0 && !value.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(value) })
}
fn text(element: &CFType, name: &str) -> Option<String> {
    attr(element, name)?
        .downcast::<CFString>()
        .map(|s| s.to_string())
}
fn children(element: &CFType, name: &str) -> Vec<CFType> {
    attr(element, name)
        .and_then(|v| v.downcast::<CFArray>())
        .map(|a| {
            a.iter()
                .map(|v| unsafe { CFType::wrap_under_get_rule(*v) })
                .collect()
        })
        .unwrap_or_default()
}
fn enabled(element: &CFType) -> bool {
    attr(element, "AXEnabled")
        .and_then(|v| v.downcast::<CFBoolean>())
        .is_some_and(|v| v.into())
}
fn value(element: &CFType) -> Option<bool> {
    let v = attr(element, "AXValue")?;
    if let Some(b) = v.downcast::<CFBoolean>() {
        return Some(b.into());
    }
    match v.downcast::<CFNumber>()?.to_i64()? {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}
fn pressable(element: &CFType) -> bool {
    let mut out = std::ptr::null();
    if unsafe { AXUIElementCopyActionNames(element.as_CFTypeRef(), &mut out) } != 0 || out.is_null()
    {
        return false;
    }
    let names = unsafe { CFType::wrap_under_create_rule(out) };
    names.downcast::<CFArray>().is_some_and(|a| {
        a.iter().any(|s| {
            unsafe { CFType::wrap_under_get_rule(*s) }
                .downcast::<CFString>()
                .is_some_and(|v| v == CFString::from_static_string("AXPress"))
        })
    })
}

fn inspect() -> Result<Vec<(CFType, Control)>, String> {
    if !super::preferences::supported_version() {
        return Err(
            "Prepared-pane adapter is validated only on macOS 13–15, English labels".into(),
        );
    }
    if unsafe { AXIsProcessTrusted() } == 0 {
        return Err("Accessibility must be authorized externally for Jitterbug. Then open General > Login Items & Extensions and leave that pane open. No permission prompt is requested.".into());
    }
    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(
        "com.apple.systempreferences",
    ));
    if apps.len() != 1 {
        return Err("Open System Settings > General > Login Items & Extensions and leave the pane open for capture/preview/restore".into());
    }
    let pid = apps.objectAtIndex(0).processIdentifier();
    let raw = unsafe { AXUIElementCreateApplication(pid) };
    if raw.is_null() {
        return Err("Cannot inspect System Settings".into());
    }
    let app = unsafe { CFType::wrap_under_create_rule(raw) };
    unsafe {
        AXUIElementSetMessagingTimeout(app.as_CFTypeRef(), 0.2);
    }
    let windows = children(&app, "AXWindows");
    let mut matches = windows.into_iter().filter(|w| {
        matches!(
            text(w, "AXTitle").as_deref(),
            Some("Login Items & Extensions" | "Login Items")
        )
    });
    let window = matches
        .next()
        .ok_or("Prepare General > Login Items & Extensions (English UI), then recheck access")?;
    if matches.next().is_some() {
        return Err("Ambiguous System Settings windows".into());
    }
    if !children(&window, "AXSheets").is_empty() {
        return Err("Dismiss the System Settings sheet yourself before capture/restore; no authentication prompt is clicked".into());
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut stack = vec![(window, 0)];
    let mut count = 0;
    let mut controls = Vec::new();
    while let Some((node, depth)) = stack.pop() {
        count += 1;
        if count > 2000 || depth > 32 || Instant::now() > deadline {
            return Err(
                "Accessibility tree exceeded time/size bound; no partial inventory captured".into(),
            );
        }
        if text(&node, "AXRole").as_deref() == Some("AXCheckBox") {
            let id = text(&node, "AXIdentifier")
                .or_else(|| text(&node, "AXDescription"))
                .unwrap_or_default();
            if id.starts_with("background-switch-") {
                controls.push((
                    node.clone(),
                    Control {
                        id,
                        value: value(&node),
                        enabled: enabled(&node),
                        pressable: pressable(&node),
                    },
                ));
            }
        }
        stack.extend(
            children(&node, "AXChildren")
                .into_iter()
                .map(|v| (v, depth + 1)),
        );
    }
    Ok(controls)
}

pub(super) fn read() -> Result<super::HardwareValue, String> {
    let nodes = inspect()?;
    Ok(super::HardwareValue::BackgroundItems {
        values: select(&nodes.into_iter().map(|(_, n)| n).collect::<Vec<_>>())?,
    })
}
pub(super) fn preflight(target: &super::HardwareValue) -> Result<(), String> {
    target.validate(140)?;
    if !read()?.same_identity(target) {
        return Err("Prepared background inventory no longer matches saved unique controls".into());
    }
    Ok(())
}
pub(super) fn write(
    before: &super::HardwareValue,
    target: &super::HardwareValue,
) -> Result<(), String> {
    preflight(target)?;
    let (
        super::HardwareValue::BackgroundItems { values: a },
        super::HardwareValue::BackgroundItems { values: b },
    ) = (before, target)
    else {
        return Err("Wrong background target".into());
    };
    restore_with(
        a,
        b,
        || match read()? {
            super::HardwareValue::BackgroundItems { values } => Ok(values),
            _ => unreachable!(),
        },
        |id| {
            let nodes = inspect()?;
            let matches: Vec<_> = nodes.iter().filter(|(_, n)| n.id == id).collect();
            if matches.len() != 1
                || !matches[0].1.enabled
                || !matches[0].1.pressable
                || matches[0].1.value != a.get(id).copied()
            {
                return Err("Background control changed immediately before press".into());
            }
            let element = &matches[0].0;
            if unsafe {
                AXUIElementPerformAction(
                    element.as_CFTypeRef(),
                    CFString::new("AXPress").as_concrete_TypeRef(),
                )
            } != 0
            {
                return Err("macOS denied background switch action; no approval or authentication is bypassed".into());
            }
            for _ in 0..20 {
                if value(element) == b.get(id).copied() {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err("Background switch did not converge; state may have changed, inspect System Settings yourself".into())
        },
    )
}
pub(super) fn annotate(items: &mut [super::Capability]) {
    if let Some(item) = items.iter_mut().find(|v| v.id == "background_items") {
        item.status = super::CapabilityStatus::Unavailable;
        item.detail = read()
            .err()
            .unwrap_or_else(|| "Prepared background switches not captured; recheck".into());
        item.verification="Implemented bounded AXPress + readback for unique enabled background-switch identifiers. Open and keep the English Login Items pane prepared. Duplicate/managed controls and initial registration approvals are excluded.".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn control(id: &str, value: bool) -> Control {
        Control {
            id: id.into(),
            value: Some(value),
            enabled: true,
            pressable: true,
        }
    }
    #[test]
    #[ignore = "read-only prepared-pane inspection; no AX action or preference setter"]
    fn prepared_background_read_only_probe() {
        match inspect() {
            Ok(nodes) => {
                let selected = select(&nodes.iter().map(|(_, n)| n.clone()).collect::<Vec<_>>());
                println!(
                    "Background AX controls inspected: {}; unique captured: {}",
                    nodes.len(),
                    selected.as_ref().map_or(0, |v| v.len())
                );
                if let Ok(values) = selected {
                    super::super::HardwareValue::BackgroundItems { values }
                        .validate(140)
                        .unwrap();
                }
            }
            Err(e) => println!("Background read-only probe unavailable: {e}"),
        }
    }
    #[test]
    fn only_unique_enabled_background_identifiers_are_captured() {
        let nodes = vec![
            control("background-switch-Example", true),
            control("background-switch-Duplicate", true),
            control("background-switch-Duplicate", false),
            control("Unrelated", true),
        ];
        let values = select(&nodes).unwrap();
        assert_eq!(
            values,
            BTreeMap::from([("background-switch-Example".into(), true)])
        );
        let mut disabled = control("background-switch-Managed", false);
        disabled.enabled = false;
        assert!(select(&[disabled]).is_err());
    }
    #[test]
    fn restore_never_replays_approval_or_mutates_a_changed_inventory() {
        use std::cell::RefCell;
        let old = BTreeMap::from([("background-switch-Example".into(), false)]);
        let target = BTreeMap::from([("background-switch-Example".into(), true)]);
        let live = RefCell::new(old.clone());
        restore_with(
            &old,
            &target,
            || Ok(live.borrow().clone()),
            |id| {
                let mut v = live.borrow_mut();
                *v.get_mut(id).unwrap() = true;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(*live.borrow(), target);
        assert!(restore_with(&old, &target, || Ok(target.clone()), |_| panic!("stale")).is_err());
        assert!(restore_with(&old, &target, || Ok(old.clone()), |_| Ok(()))
            .unwrap_err()
            .contains("readback"));
    }
}
