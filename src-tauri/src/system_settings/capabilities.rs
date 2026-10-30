use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    Supported,
    NeedsPermission,
    ReadOnly,
    Manual,
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
            pane: match group { "Finder storage" => None, "Desktop & Dock" => Some("dock".into()), "Keyboard" => Some("keyboard".into()), _ if key == Preference::MouseSpeed => Some("mouse".into()), _ => Some("trackpad".into()) },
        });
    }
    for (id, label, group, detail, pane) in [
        ("audio_format", "Output sample rate and full stream format", "Audio hardware", "Manual only: CoreAudio nominal sample rate and physical stream format describe live driver state. A settable property is not a persistence guarantee across reconnect/reboot. No device-specific persistence contract has been validated; use Audio MIDI Setup. No microphone permission is required for discovery.", "sound"),
        ("display_mode", "Display resolution and refresh mode", "Displays", "Manual only: full CoreGraphics mode, not Hz alone. CGDisplaySetDisplayMode does not establish persisted configuration; even CGCompleteDisplayConfiguration(kCGConfigurePermanently) may fall back to login-session scope when the UI cannot support a mode. No verified persistent-write adapter is enabled. Use Displays.", "displays"),
        ("low_power", "Low Power Mode", "Power profiles", "Manual only: hardware, macOS, and power-source dependent. Power-profile reads do not authorize writes; no native privileged authorization adapter is installed. Use Battery or Energy settings.", "battery"),
        ("high_power", "High Power Mode", "Power profiles", "Manual only: available on selected Mac models; do not infer support from a numeric power mode alone. No privileged writer is installed. Use Battery or Energy settings.", "battery"),
        ("login_items", "Login items", "Login & background", "Manual only: ServiceManagement controls an app's own registered helpers, not a safe reversible inventory of arbitrary user login items. No other apps' entries are captured or changed. Review General > Login Items & Extensions.", "login_items"),
        ("background_items", "Background items", "Login & background", "Manual only: user approvals and managed background-task registrations are not replayable preferences. No background-item database is modified. Review General > Login Items & Extensions.", "login_items"),
        ("time_machine", "Time Machine automatic backup schedule", "Backup", "Manual only: no typed authorized backup-schedule adapter is installed. A backup run or deletion is not a setting; neither is offered. Review General > Time Machine.", "time_machine"),
        ("pointer_acceleration", "Pointer acceleration (not tracking speed)", "Pointer", "Manual only: tracking-speed overrides captured above are not a separate acceleration toggle or per-device curve. No verified reversible acceleration adapter is implemented. Review Mouse > Advanced where available.", "mouse"),
        ("icloud_optimize", "iCloud Optimize Mac Storage", "Storage", "Manual only: account-managed cloud storage policy is not a reliable public reversible preference API. Cloud files are never evicted, downloaded, or deleted by this baseline. Review Apple Account > iCloud.", "icloud"),
        ("microphone_privacy", "Microphone privacy grants", "Privacy", "Manual only: TCC grants cannot be captured and replayed. Discovery does not request microphone access. Grant or revoke access yourself in Privacy & Security > Microphone; Jitterbug never edits the TCC database.", "microphone"),
        ("spotlight_exclusions", "Spotlight privacy exclusions", "Search", "Manual only: no supported exact per-volume exclusion-list restore adapter is implemented. Full Disk Access is not a substitute. Review Spotlight > Search Privacy; no index rebuild or deletion is offered.", "spotlight"),
        ("developer_access", "Developer Tools access", "Privacy", "Manual only: per-app Developer Tools privacy approvals are user-controlled TCC policy, not replayable settings. Review Privacy & Security > Developer Tools. No security controls are disabled.", "developer"),
        ("stage_manager", "Stage Manager", "Window management", "Unavailable: no supported public reversible setting API. Private WindowManager preferences are not treated as a reliable live control. Change this in Desktop & Dock.", "dock"),
        ("reduce_motion", "Reduce motion", "Accessibility display", "Unavailable for restoration: public accessibility APIs report effective behavior but provide no supported setter. Jitterbug will not request Full Disk Access or script protected universalaccess preferences. Change this in Accessibility > Display.", "accessibility_display"),
        ("reduce_transparency", "Reduce transparency", "Accessibility display", "Unavailable for restoration: no public supported setter. Granting Accessibility does not make this preference a supported setter. Change this in Accessibility > Display.", "accessibility_display"),
        ("display_sleep", "Display sleep timeout", "Power & display", "Administrator-managed power preference. Native privileged authorization is not implemented; Jitterbug cannot request or fake a grant. Set the timeout in Lock Screen. No sudo, password prompt, or privileged helper is used.", "lock_screen"),
        ("system_sleep", "System idle sleep", "Power & display", "Administrator-managed and hardware/power-source dependent. No safe privileged write adapter is installed. Review Battery or Energy settings manually; this baseline does not capture system power profiles.", "battery"),
        ("brightness", "Display brightness", "Power & display", "Unavailable: no implemented, verified per-display setter across Apple Silicon, built-in, and external displays. Private DisplayServices/DDC commands are not used. Adjust in Displays.", "displays"),
    ] {
        result.push(Capability { included: false, id: id.into(), label: label.into(), group: group.into(), status: if detail.starts_with("Manual only:") { CapabilityStatus::Manual } else { CapabilityStatus::Unavailable }, detail: detail.into(), verification: "Not captured; never restored".into(), pane: if id == "audio_format" { None } else { Some(pane.into()) } });
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
        "login_items" => Ok("x-apple.systempreferences:com.apple.LoginItems-Settings.extension"),
        "time_machine" => Ok("x-apple.systempreferences:com.apple.Time-Machine-Settings.extension"),
        "icloud" => Ok("x-apple.systempreferences:com.apple.preferences.AppleIDPrefPane"),
        "microphone" => {
            Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        }
        "spotlight" => Ok("x-apple.systempreferences:com.apple.Spotlight-Settings.extension"),
        "developer" => {
            Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_DevTools")
        }
        _ => Err("Unknown System Settings destination.".into()),
    }
}
