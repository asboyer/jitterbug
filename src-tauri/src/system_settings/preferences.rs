//! Allowlisted preference storage, not a claim that macOS has applied a live effect.
use serde::{Deserialize, Serialize};
#[cfg(target_os = "macos")]
pub(super) mod macos;

#[cfg(target_os = "macos")]
pub fn collect() -> std::collections::BTreeMap<Preference, PreferenceValue> {
    if !supported_version() {
        return Default::default();
    }
    Preference::ALL
        .into_iter()
        .filter_map(|key| macos::read(key).ok().map(|value| (key, value)))
        .collect()
}

#[cfg(target_os = "macos")]
pub fn supported_version() -> bool {
    sysinfo::System::os_version()
        .and_then(|v| v.split('.').next().and_then(|v| v.parse::<u32>().ok()))
        .is_some_and(|v| (13..=15).contains(&v))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preference {
    DockAutohide,
    DockMagnification,
    DockAnimate,
    DockMinimizeToApplication,
    DockShowRecents,
    DockSize,
    KeyRepeat,
    InitialKeyRepeat,
    PressAndHold,
    NaturalScroll,
    MouseSpeed,
    TrackpadSpeed,
    AutomaticSpelling,
    AutomaticCapitalization,
    AutomaticPeriod,
    AutomaticDash,
    AutomaticQuote,
    DsStoreNetwork,
    DsStoreUsb,
    MouseLinear,
    IcloudOptimizeStorage,
    StageManager,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PreferenceValue {
    Absent,
    Boolean(bool),
    Integer(i64),
    Real(f64),
}

impl Preference {
    pub const ALL: [Self; 22] = [
        Self::DockAutohide,
        Self::DockMagnification,
        Self::DockAnimate,
        Self::DockMinimizeToApplication,
        Self::DockShowRecents,
        Self::DockSize,
        Self::KeyRepeat,
        Self::InitialKeyRepeat,
        Self::PressAndHold,
        Self::NaturalScroll,
        Self::MouseSpeed,
        Self::TrackpadSpeed,
        Self::AutomaticSpelling,
        Self::AutomaticCapitalization,
        Self::AutomaticPeriod,
        Self::AutomaticDash,
        Self::AutomaticQuote,
        Self::DsStoreNetwork,
        Self::DsStoreUsb,
        Self::MouseLinear,
        Self::IcloudOptimizeStorage,
        Self::StageManager,
    ];
    pub fn descriptor(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Self::IcloudOptimizeStorage => (
                "com.apple.bird",
                "optimize-storage",
                "iCloud Optimize Mac Storage override",
                "Storage",
            ),
            Self::StageManager => (
                "com.apple.WindowManager",
                "GloballyEnabled",
                "Stage Manager override",
                "Desktop & Dock",
            ),
            Self::MouseLinear => (
                "NSGlobalDomain",
                "com.apple.mouse.linear",
                "Mouse linear motion (acceleration off)",
                "Pointer",
            ),
            Self::AutomaticSpelling => (
                "NSGlobalDomain",
                "NSAutomaticSpellingCorrectionEnabled",
                "Automatic spelling correction",
                "Keyboard",
            ),
            Self::AutomaticCapitalization => (
                "NSGlobalDomain",
                "NSAutomaticCapitalizationEnabled",
                "Automatic capitalization",
                "Keyboard",
            ),
            Self::AutomaticPeriod => (
                "NSGlobalDomain",
                "NSAutomaticPeriodSubstitutionEnabled",
                "Automatic period substitution",
                "Keyboard",
            ),
            Self::AutomaticDash => (
                "NSGlobalDomain",
                "NSAutomaticDashSubstitutionEnabled",
                "Smart dashes",
                "Keyboard",
            ),
            Self::AutomaticQuote => (
                "NSGlobalDomain",
                "NSAutomaticQuoteSubstitutionEnabled",
                "Smart quotes",
                "Keyboard",
            ),
            Self::DsStoreNetwork => (
                "com.apple.desktopservices",
                "DSDontWriteNetworkStores",
                "Avoid .DS_Store on network volumes",
                "Finder storage",
            ),
            Self::DsStoreUsb => (
                "com.apple.desktopservices",
                "DSDontWriteUSBStores",
                "Avoid .DS_Store on USB volumes",
                "Finder storage",
            ),
            Self::DockAutohide => (
                "com.apple.dock",
                "autohide",
                "Dock auto-hide",
                "Desktop & Dock",
            ),
            Self::DockMagnification => (
                "com.apple.dock",
                "magnification",
                "Dock magnification",
                "Desktop & Dock",
            ),
            Self::DockAnimate => (
                "com.apple.dock",
                "launchanim",
                "Animate opening applications",
                "Desktop & Dock",
            ),
            Self::DockMinimizeToApplication => (
                "com.apple.dock",
                "minimize-to-application",
                "Minimize into application icon",
                "Desktop & Dock",
            ),
            Self::DockShowRecents => (
                "com.apple.dock",
                "show-recents",
                "Show recent applications",
                "Desktop & Dock",
            ),
            Self::DockSize => (
                "com.apple.dock",
                "tilesize",
                "Dock icon size",
                "Desktop & Dock",
            ),
            Self::KeyRepeat => (
                "NSGlobalDomain",
                "KeyRepeat",
                "Keyboard repeat interval",
                "Keyboard",
            ),
            Self::InitialKeyRepeat => (
                "NSGlobalDomain",
                "InitialKeyRepeat",
                "Keyboard repeat delay",
                "Keyboard",
            ),
            Self::PressAndHold => (
                "NSGlobalDomain",
                "ApplePressAndHoldEnabled",
                "Press-and-hold accent menu",
                "Keyboard",
            ),
            Self::NaturalScroll => (
                "NSGlobalDomain",
                "com.apple.swipescrolldirection",
                "Natural scrolling",
                "Pointer",
            ),
            Self::MouseSpeed => (
                "NSGlobalDomain",
                "com.apple.mouse.scaling",
                "Mouse tracking speed",
                "Pointer",
            ),
            Self::TrackpadSpeed => (
                "NSGlobalDomain",
                "com.apple.trackpad.scaling",
                "Trackpad tracking speed",
                "Pointer",
            ),
        }
    }
    pub fn validate(self, value: PreferenceValue) -> Result<(), String> {
        let valid = match value {
            PreferenceValue::Absent => true,
            PreferenceValue::Boolean(_) => !matches!(
                self,
                Self::DockSize
                    | Self::KeyRepeat
                    | Self::InitialKeyRepeat
                    | Self::MouseSpeed
                    | Self::TrackpadSpeed
            ),
            PreferenceValue::Integer(v) => match self {
                Self::KeyRepeat | Self::InitialKeyRepeat => (0..=10000).contains(&v),
                Self::DockSize => (16..=256).contains(&v),
                _ => false,
            },
            PreferenceValue::Real(v) => {
                v.is_finite()
                    && match self {
                        Self::MouseSpeed | Self::TrackpadSpeed => (-1.0..=10.0).contains(&v),
                        Self::DockSize => (16.0..=256.0).contains(&v),
                        _ => false,
                    }
            }
        };
        if valid {
            Ok(())
        } else {
            Err(format!(
                "{} has an unsupported native type or range; not captured.",
                self.descriptor().2
            ))
        }
    }
}

#[cfg(test)]
mod modern_tests {
    use super::*;
    #[test]
    fn icloud_and_stage_manager_have_exact_scopes_not_cloud_file_actions() {
        for (encoded, domain, key) in [
            (
                "icloud_optimize_storage",
                "com.apple.bird",
                "optimize-storage",
            ),
            (
                "stage_manager",
                "com.apple.WindowManager",
                "GloballyEnabled",
            ),
        ] {
            let p: Preference = serde_json::from_str(&format!("\"{encoded}\""))
                .expect("persistent policy override");
            assert_eq!((p.descriptor().0, p.descriptor().1), (domain, key));
            assert!(p.validate(PreferenceValue::Boolean(false)).is_ok());
            assert!(p.validate(PreferenceValue::Absent).is_ok());
            assert!(p.validate(PreferenceValue::Integer(1)).is_err());
        }
    }
    #[test]
    fn modern_acceleration_is_a_distinct_boolean_not_tracking_speed() {
        let key: Preference =
            serde_json::from_str("\"mouse_linear\"").expect("modern acceleration adapter");
        assert_eq!(
            key.descriptor(),
            (
                "NSGlobalDomain",
                "com.apple.mouse.linear",
                "Mouse linear motion (acceleration off)",
                "Pointer"
            )
        );
        assert!(key.validate(PreferenceValue::Boolean(true)).is_ok());
        assert!(key.validate(PreferenceValue::Absent).is_ok());
        assert!(key.validate(PreferenceValue::Real(-1.0)).is_err());
    }
}
