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
            pane: match group { "Storage" => Some("icloud".into()), "Finder storage" => None, "Desktop & Dock" => Some("dock".into()), "Keyboard" => Some("keyboard".into()), _ if matches!(key, Preference::MouseSpeed | Preference::MouseLinear) => Some("mouse".into()), _ => Some("trackpad".into()) },
        });
    }
    if let Some(item) = result
        .iter_mut()
        .find(|v| v.id == "icloud_optimize_storage")
    {
        item.detail.push_str(" Only the current user's com.apple.bird optimize-storage override is captured, not the Apple Account, iCloud Drive enablement, Photos, or file residency. No eviction/download command is run. A policy change may make macOS autonomously download or evict local cloud copies later; those file effects cannot be rolled back by this baseline.");
    }
    for (id, label, group, detail, pane) in [
        ("audio_format", "Output sample rate and full stream format", "Audio hardware", "Unavailable: automatic rate/full-ASBD executor exists, but no readable supported settable output configuration was captured. Recheck device support; no microphone permission is needed. Reboot persistence is not verified.", "sound"),
        ("display_mode", "Display resolution and refresh mode", "Displays", "Unavailable: automatic full-mode executor exists, but no stable, supported display inventory was captured. Recheck displays/access. Restore requests permanent scope and verifies current modes, not reboot persistence.", "displays"),
        ("low_power", "Low Power Mode", "Power profiles", "Unavailable: automatic pmset energy-policy executor exists, but no validated Low/High Power value was captured for this source. Recheck Battery settings and external authorization.", "battery"),
        ("high_power", "High Power Mode", "Power profiles", "Unavailable: automatic pmset energy-policy executor exists, but High Power was not reported on this hardware/source. Recheck Battery settings.", "battery"),
        ("login_items", "Login items", "Login & background", "Unavailable: automatic System Events executor exists for user-added application Open at Login items, but none were captured. Approve Jitterbug → System Events Automation if required, then recheck. Background service approvals are separate.", "login_items"),
        ("background_items", "Background items", "Login & background", "Unavailable: automatic Accessibility executor exists for unique enabled background switches, but the Login Items pane was not prepared. Open General > Login Items & Extensions and recheck. Duplicate/managed items are never captured.", "login_items"),
        ("time_machine", "Time Machine automatic backup schedule", "Backup", "Unavailable: automatic AutoBackup/AutoBackupInterval executor exists, but the system schedule plist was not readable. Full Disk Access and external authorization may be required. No backup or deletion is run.", "time_machine"),
        ("microphone_privacy", "Microphone privacy grants", "Privacy", "Manual only: TCC grants cannot be captured and replayed. Discovery does not request microphone access. Grant or revoke access yourself in Privacy & Security > Microphone; Jitterbug never edits the TCC database.", "microphone"),
        ("spotlight_exclusions", "Spotlight privacy exclusions", "Search", "Manual only: no supported exact per-volume exclusion-list restore adapter is implemented. Full Disk Access is not a substitute. Review Spotlight > Search Privacy; no index rebuild or deletion is offered.", "spotlight"),
        ("developer_access", "Developer Tools access", "Privacy", "Manual only: per-app Developer Tools privacy approvals are user-controlled TCC policy, not replayable settings. Review Privacy & Security > Developer Tools. No security controls are disabled.", "developer"),
        ("reduce_motion", "Reduce motion", "Accessibility display", "Unavailable for restoration: public accessibility APIs report effective behavior but provide no supported setter. Jitterbug will not request Full Disk Access or script protected universalaccess preferences. Change this in Accessibility > Display.", "accessibility_display"),
        ("reduce_transparency", "Reduce transparency", "Accessibility display", "Unavailable for restoration: no public supported setter. Granting Accessibility does not make this preference a supported setter. Change this in Accessibility > Display.", "accessibility_display"),
        ("display_sleep", "Display sleep timeout", "Power & display", "Unavailable: automatic per-source displaysleep executor exists, but no validated saved timer was captured. Recheck power-profile discovery. Execution needs external authorization, not a fake grant.", "lock_screen"),
        ("system_sleep", "System idle sleep", "Power & display", "Unavailable: automatic per-source sleep executor exists, but no validated saved timer was captured. Recheck power-profile discovery. Execution needs external authorization.", "battery"),
        ("brightness", "Display brightness", "Power & display", "Unavailable: no implemented, verified per-display setter across Apple Silicon, built-in, and external displays. Private DisplayServices/DDC commands are not used. Adjust in Displays.", "displays"),
    ] {
        result.push(Capability { included: false, id: id.into(), label: label.into(), group: group.into(), status: if detail.starts_with("Manual only:") { CapabilityStatus::Manual } else { CapabilityStatus::Unavailable }, detail: detail.into(), verification: "Not captured; never restored".into(), pane: if id == "audio_format" { None } else { Some(pane.into()) } });
    }
    result.retain(|item| match item.id.as_str() {
        "low_power" | "high_power" => !values
            .hardware
            .values()
            .any(|v| matches!(v, HardwareValue::EnergyPolicy { .. })),
        "icloud_optimize" => !values
            .preferences
            .contains_key(&Preference::IcloudOptimizeStorage),
        "stage_manager" => !values.preferences.contains_key(&Preference::StageManager),
        "pointer_acceleration" => !values.preferences.contains_key(&Preference::MouseLinear),
        "background_items" => !values.hardware.contains_key(&140),
        "login_items" => !values.hardware.contains_key(&130),
        "time_machine" => !values.hardware.contains_key(&120),
        "audio_format" => !values.hardware.contains_key(&0),
        "display_mode" => !values.hardware.contains_key(&1),
        "system_sleep" => !values
            .hardware
            .values()
            .any(|v| matches!(v, HardwareValue::Power {key, ..} if key == "sleep")),
        "display_sleep" => !values
            .hardware
            .values()
            .any(|v| matches!(v, HardwareValue::Power {key, ..} if key == "displaysleep")),
        _ => true,
    });
    for (slot, value) in &values.hardware {
        let power = matches!(
            value,
            HardwareValue::Power { .. }
                | HardwareValue::EnergyPolicy { .. }
                | HardwareValue::TimeMachine { .. }
        );
        result.push(Capability {
            id: format!("hardware_{slot}"), label: match value { HardwareValue::Power {source, key, ..} => format!("{source} / {key}"), _ => value.label().into() },
            group: value.group().into(), status: if power { CapabilityStatus::NeedsPermission } else { CapabilityStatus::Supported }, included: true,
            detail: format!("Automatic rollback executor implemented. Captured: {}. {}", value.description(), if power { "Executable with externally provided system authorization. No password, sudo, helper installation, or automatic grant; a denial is reported per target, not silently treated as success." } else { "Supported native descriptors are re-enumerated before writes. No permission is bypassed. Hardware may reject a busy or unsupported configuration." }),
            verification: value.verification().into(), pane: None,
        });
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
