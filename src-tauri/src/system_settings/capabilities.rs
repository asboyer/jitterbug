use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    Supported,
    NeedsPermission,
    ReadOnly,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub id: String,
    pub label: String,
    pub group: String,
    pub status: CapabilityStatus,
    #[serde(default)]
    pub included: bool,
    pub detail: String,
    pub verification: String,
    pub pane: Option<String>,
}

pub(super) fn coverage(values: &SystemValues) -> Vec<Capability> {
    let mut result = Vec::new();
    for (id, label, available) in [
        (
            "output_volume",
            "Output volume",
            values.output_volume.is_some(),
        ),
        ("output_muted", "Output mute", values.output_muted.is_some()),
    ] {
        result.push(Capability { included: available, id: id.into(), label: label.into(), group: "Audio".into(), status: if available { CapabilityStatus::Supported } else { CapabilityStatus::Unavailable }, detail: if available { "Current default output device; no microphone or Accessibility permission needed. Restoring volume may make audio louder." } else { "No supported settable master control on the current default output device. Choose a device in Sound, then recheck." }.into(), verification: "Live CoreAudio readback".into(), pane: Some("sound".into()) });
    }
    for key in Preference::ALL {
        let (_, _, label, group) = key.descriptor();
        let captured = values.preferences.contains_key(&key);
        result.push(Capability {
            included: captured,
            id: serde_json::to_value(key).unwrap().as_str().unwrap().into(), label: label.into(), group: group.into(),
            status: if captured { CapabilityStatus::Supported } else { CapabilityStatus::Unavailable },
            detail: if captured { "Current user, any-host preference override only. Absence is preserved, not replaced with a guessed default. No administrator or Accessibility grant needed. A successful read does not prove write permission; any write denial is reported." } else { "Not captured: incompatible OS, managed preference, denied synchronization, or unsupported native type/range. Recheck access; do not grant unrelated privacy permissions." }.into(),
            verification: "Saved preference only — live effect not verified. Application relaunch, sign-out, or a Mac restart may be required; Jitterbug never restarts Dock or SystemUIServer.".into(),
            pane: Some(match group { "Desktop & Dock" => "dock", "Keyboard" => "keyboard", _ if key == Preference::MouseSpeed => "mouse", _ => "trackpad" }.into()),
        });
    }
    for (id, label, group, detail, pane) in [
        ("stage_manager", "Stage Manager", "Window management", "Unavailable: no supported public reversible setting API. Private WindowManager preferences are not treated as a reliable live control. Change this in Desktop & Dock.", "dock"),
        ("reduce_motion", "Reduce motion", "Accessibility display", "Unavailable for restoration: public accessibility APIs report effective behavior but provide no supported setter. Jitterbug will not request Full Disk Access or script protected universalaccess preferences. Change this in Accessibility > Display.", "accessibility_display"),
        ("reduce_transparency", "Reduce transparency", "Accessibility display", "Unavailable for restoration: no public supported setter. Granting Accessibility does not make this preference a supported setter. Change this in Accessibility > Display.", "accessibility_display"),
        ("display_sleep", "Display sleep timeout", "Power & display", "Administrator-managed power preference. Native privileged authorization is not implemented; Jitterbug cannot request or fake a grant. Set the timeout in Lock Screen. No sudo, password prompt, or privileged helper is used.", "lock_screen"),
        ("system_sleep", "System idle sleep", "Power & display", "Administrator-managed and hardware/power-source dependent. No safe privileged write adapter is installed. Review Battery or Energy settings manually; this baseline does not capture system power profiles.", "battery"),
        ("brightness", "Display brightness", "Power & display", "Unavailable: no implemented, verified per-display setter across Apple Silicon, built-in, and external displays. Private DisplayServices/DDC commands are not used. Adjust in Displays.", "displays"),
    ] {
        result.push(Capability { included: false, id: id.into(), label: label.into(), group: group.into(), status: CapabilityStatus::Unavailable, detail: detail.into(), verification: "Not captured; never restored".into(), pane: Some(pane.into()) });
    }
    result
}

/// Closed allowlist; no URL, shell command or argument supplied by callers is executed.
pub fn settings_url(pane: &str) -> Result<&'static str, String> {
    match pane {
        "sound" => Ok("x-apple.systempreferences:com.apple.Sound-Settings.extension"),
        "dock" => Ok("x-apple.systempreferences:com.apple.Desktop-Settings.extension"),
        "keyboard" => Ok("x-apple.systempreferences:com.apple.Keyboard-Settings.extension"),
        "mouse" => Ok("x-apple.systempreferences:com.apple.Mouse-Settings.extension"),
        "trackpad" => Ok("x-apple.systempreferences:com.apple.Trackpad-Settings.extension"),
        "accessibility_display" => {
            Ok("x-apple.systempreferences:com.apple.Accessibility-Settings.extension?Display")
        }
        "lock_screen" => Ok("x-apple.systempreferences:com.apple.Lock-Screen-Settings.extension"),
        "battery" => Ok("x-apple.systempreferences:com.apple.Battery-Settings.extension"),
        "displays" => Ok("x-apple.systempreferences:com.apple.Displays-Settings.extension"),
        _ => Err("Unknown System Settings destination.".into()),
    }
}
