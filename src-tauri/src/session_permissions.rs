//! Per-user session optimization allowlist. Default off. Availability depends on this PC's OS.
use crate::storage;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

pub const FILE_NAME: &str = "session-permissions.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupId {
    PowerAndSleep,
    Appearance,
    Focus,
    Displays,
    BackgroundWork,
}

pub type ActionId = crate::optimized_sessions::Setting;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Available,
    Unsupported,
    NeedsOsPermission,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Environment {
    pub os: String,
    pub version: String,
    pub arch: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Action {
    pub id: ActionId,
    pub group: GroupId,
    pub label: String,
    pub detail: String,
    pub status: ActionStatus,
    pub session_enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub label: String,
    pub detail: String,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct View {
    pub environment: Environment,
    pub groups: Vec<Group>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    enabled: BTreeMap<ActionId, bool>,
}

pub fn environment() -> Environment {
    Environment {
        os: std::env::consts::OS.into(),
        version: sysinfo::System::os_version().unwrap_or_else(|| "unknown".into()),
        arch: std::env::consts::ARCH.into(),
    }
}

fn catalog(enabled: &BTreeMap<ActionId, bool>) -> Vec<Group> {
    let macos = cfg!(target_os = "macos");
    let mut groups: BTreeMap<GroupId, Vec<Action>> = BTreeMap::new();
    for setting in crate::optimized_sessions::Setting::ALL {
        let status = if macos
            && (setting == crate::optimized_sessions::Setting::KeepAwake
                || setting.preference().is_some())
        {
            ActionStatus::Available
        } else {
            ActionStatus::Unsupported
        };
        let action = Action {
            id: setting,
            group: setting.group(),
            label: setting.label().into(),
            detail: if setting == crate::optimized_sessions::Setting::KeepAwake {
                "Temporary process-owned idle-sleep assertion. Released when the session stops."
                    .into()
            } else {
                "Reversible boolean preference adapter with exact-domain readback. Live windowing effect is not claimed.".into()
            },
            status,
            session_enabled: status == ActionStatus::Available
                && enabled.get(&setting).copied().unwrap_or(false),
        };
        groups.entry(setting.group()).or_default().push(action);
    }
    [
        (
            GroupId::PowerAndSleep,
            "Power & sleep",
            "Idle sleep and energy-related runtime requests.",
        ),
        (
            GroupId::Appearance,
            "Appearance & desktop",
            "Dock and windowing preferences.",
        ),
        (
            GroupId::Focus,
            "Focus & typing",
            "Typing substitutions that interrupt focused work.",
        ),
        (GroupId::Displays, "Pointer", "Pointer motion preferences."),
        (
            GroupId::BackgroundWork,
            "Storage extras",
            "Finder/.DS_Store and iCloud optimize-storage overrides.",
        ),
    ]
    .into_iter()
    .map(|(id, label, detail)| Group {
        id,
        label: label.into(),
        detail: detail.into(),
        actions: groups.remove(&id).unwrap_or_default(),
    })
    .collect()
}

pub fn view(directory: &Path) -> Result<View, String> {
    let saved: Saved = storage::read(&directory.join(FILE_NAME))?;
    Ok(View {
        environment: environment(),
        groups: catalog(&saved.enabled),
    })
}

pub fn save(directory: &Path, enabled: BTreeMap<ActionId, bool>) -> Result<View, String> {
    let mut saved = Saved { enabled };
    saved.enabled.retain(|id, on| {
        *on && catalog(&BTreeMap::from([(*id, true)]))
            .iter()
            .flat_map(|group| group.actions.iter())
            .any(|action| action.id == *id && action.status == ActionStatus::Available)
    });
    storage::write(&directory.join(FILE_NAME), &saved)?;
    view(directory)
}

pub fn allowed_settings(
    directory: &Path,
) -> Result<Vec<crate::optimized_sessions::Setting>, String> {
    Ok(view(directory)?
        .groups
        .into_iter()
        .flat_map(|group| group.actions)
        .filter(|action| action.session_enabled && action.status == ActionStatus::Available)
        .map(|action| action.id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_directory(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "jitterbug-session-permissions-{label}-{}-{}",
            std::process::id(),
            crate::types::now()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn defaults_are_off_and_only_available_actions_can_be_enabled() {
        let directory = temp_directory("defaults");
        let initial = view(&directory).unwrap();
        assert!(!initial
            .groups
            .iter()
            .flat_map(|group| group.actions.iter())
            .any(|action| action.session_enabled));
        assert!(!initial.environment.os.is_empty());
        assert!(!initial.environment.version.is_empty());

        let mut requested = BTreeMap::new();
        requested.insert(ActionId::KeepAwake, true);
        requested.insert(ActionId::StageManager, true);
        let saved = save(&directory, requested).unwrap();
        let enabled: Vec<_> = saved
            .groups
            .iter()
            .flat_map(|group| group.actions.iter())
            .filter(|action| action.session_enabled)
            .map(|action| action.id)
            .collect();
        if cfg!(target_os = "macos") {
            assert!(enabled.contains(&ActionId::KeepAwake));
            assert!(enabled.contains(&ActionId::StageManager));
        } else {
            assert!(enabled.is_empty());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn allowed_settings_are_empty_until_an_available_action_is_enabled() {
        let directory = temp_directory("allowed");
        assert!(allowed_settings(&directory).unwrap().is_empty());
        let mut requested = BTreeMap::new();
        requested.insert(ActionId::KeepAwake, true);
        save(&directory, requested).unwrap();
        let allowed = allowed_settings(&directory).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(allowed, vec![crate::optimized_sessions::Setting::KeepAwake]);
        } else {
            assert!(allowed.is_empty());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
