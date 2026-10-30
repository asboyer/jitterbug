use crate::settings::{Mode, Settings, Tier};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const DIRECTORY: &str = "settings-default-history";
const SNAPSHOTS: &str = "snapshots";
const CURRENT: &str = "current.json";
const SCHEMA_VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 64 * 1024;
const MAX_JAVASCRIPT_DATE_MS: u64 = 8_640_000_000_000_000;
pub const MAX_PAGE_SIZE: usize = 50;
static FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportedSettings {
    pub mode: Mode,
    pub decision: Tier,
    pub cheap: Tier,
    pub intelligent: Tier,
    pub decision_enabled: bool,
    pub budget_usd: f64,
}

impl SupportedSettings {
    pub fn from_saved(settings: &Settings) -> Self {
        Self {
            mode: settings.mode.clone(),
            decision: settings.decision.clone(),
            cheap: settings.cheap.clone(),
            intelligent: settings.intelligent.clone(),
            decision_enabled: settings.decision_enabled,
            budget_usd: settings.budget_usd,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        Settings {
            mode: self.mode.clone(),
            decision: self.decision.clone(),
            cheap: self.cheap.clone(),
            intelligent: self.intelligent.clone(),
            decision_enabled: self.decision_enabled,
            budget_usd: self.budget_usd,
            automation: Default::default(),
        }
        .validate()
    }

    pub fn apply_to(&self, current: &Settings) -> Settings {
        Settings {
            mode: self.mode.clone(),
            decision: self.decision.clone(),
            cheap: self.cheap.clone(),
            intelligent: self.intelligent.clone(),
            decision_enabled: self.decision_enabled,
            budget_usd: self.budget_usd,
            automation: current.automation.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotFile {
    schema_version: u32,
    id: String,
    created_at_ms: u64,
    settings: SupportedSettings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentState {
    schema_version: u32,
    pub id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SnapshotView {
    pub id: String,
    pub current: bool,
    pub created_at_ms: Option<u64>,
    pub summary: Option<String>,
    pub settings: Option<SupportedSettings>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HistoryPage {
    pub entries: Vec<SnapshotView>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FieldChange {
    pub field: String,
    pub before: String,
    pub after: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RestorePreview {
    pub token: String,
    pub expires_at_ms: u64,
    pub target: SnapshotView,
    pub changes: Vec<FieldChange>,
}

pub fn capture(directory: &Path, settings: &Settings) -> Result<SnapshotView, String> {
    let supported = SupportedSettings::from_saved(settings);
    supported.validate()?;
    let current = current_state(directory)?;
    let revision = current
        .as_ref()
        .map_or(1, |value| value.revision.saturating_add(1));
    let created_at_ms = now_ms()?;
    let root = history_root(directory);
    let snapshots = root.join(SNAPSHOTS);
    create_private_directory(&snapshots)?;
    let (id, path) = reserve_snapshot_path(&snapshots, created_at_ms)?;
    let snapshot = SnapshotFile {
        schema_version: SCHEMA_VERSION,
        id: id.clone(),
        created_at_ms,
        settings: supported.clone(),
    };
    let bytes = encode(&snapshot)?;
    atomic_create(&path, &bytes)?;
    let pointer = CurrentState {
        schema_version: SCHEMA_VERSION,
        id: id.clone(),
        revision,
    };
    if let Err(error) = atomic_replace(&root.join(CURRENT), &encode(&pointer)?) {
        return Err(format!(
            "The snapshot was preserved, but it could not become the current default: {error}"
        ));
    }
    Ok(valid_view(snapshot, true))
}

pub fn history(
    directory: &Path,
    cursor: Option<&str>,
    page_size: usize,
) -> Result<HistoryPage, String> {
    if page_size == 0 || page_size > MAX_PAGE_SIZE {
        return Err(format!(
            "Choose a history page size from 1 to {MAX_PAGE_SIZE}."
        ));
    }
    if let Some(value) = cursor {
        validate_id(value)?;
    }
    let current = current_state(directory)?;
    let snapshots = history_root(directory).join(SNAPSHOTS);
    let read = match fs::read_dir(&snapshots) {
        Ok(read) => read,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(current) = current.as_ref() {
                return Ok(HistoryPage {
                    entries: vec![inspect_with_current(directory, &current.id, Some(current))?],
                    next_cursor: None,
                });
            }
            return Ok(HistoryPage {
                entries: Vec::new(),
                next_cursor: None,
            });
        }
        Err(_) => return Err("Cannot read saved default history.".into()),
    };
    // Retain only one page plus a sentinel while streaming the directory.
    let mut ids = Vec::with_capacity(page_size + 1);
    for entry in read {
        let entry = entry.map_err(|_| "Cannot read a saved default history entry.")?;
        let file_type = entry
            .file_type()
            .map_err(|_| "Cannot inspect a saved default history entry.")?;
        if !file_type.is_file() && !file_type.is_symlink() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(id) = name.strip_suffix(".json") else {
            continue;
        };
        if validate_id(id).is_err() || cursor.is_some_and(|value| id >= value) {
            continue;
        }
        let position = ids
            .binary_search_by(|probe: &String| probe.as_str().cmp(id).reverse())
            .unwrap_or_else(|position| position);
        ids.insert(position, id.to_owned());
        ids.truncate(page_size + 1);
    }
    if let Some(current) = current.as_ref() {
        let id = current.id.as_str();
        if !ids.iter().any(|known| known == id)
            && cursor.is_none_or(|value| id < value)
            && !history_root(directory)
                .join(SNAPSHOTS)
                .join(format!("{id}.json"))
                .exists()
        {
            let position = ids
                .binary_search_by(|probe: &String| probe.as_str().cmp(id).reverse())
                .unwrap_or_else(|position| position);
            ids.insert(position, id.to_owned());
            ids.truncate(page_size + 1);
        }
    }
    let has_more = ids.len() > page_size;
    ids.truncate(page_size);
    let next_cursor = has_more.then(|| ids.last().cloned()).flatten();
    let entries = ids
        .into_iter()
        .map(|id| inspect_with_current(directory, &id, current.as_ref()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(HistoryPage {
        entries,
        next_cursor,
    })
}

pub fn inspect(directory: &Path, id: &str) -> Result<SnapshotView, String> {
    let current = current_state(directory)?;
    inspect_with_current(directory, id, current.as_ref())
}

pub fn current_state(directory: &Path) -> Result<Option<CurrentState>, String> {
    let path = history_root(directory).join(CURRENT);
    let Some(bytes) = read_bounded(&path)? else {
        return Ok(None);
    };
    let state: CurrentState = serde_json::from_slice(&bytes)
        .map_err(|_| "Current default metadata is corrupt or incompatible.")?;
    if state.schema_version != SCHEMA_VERSION {
        return Err("Current default metadata uses an unsupported future schema.".into());
    }
    validate_id(&state.id)?;
    if state.revision == 0 {
        return Err("Current default metadata has an invalid revision.".into());
    }
    Ok(Some(state))
}

pub fn load_supported(directory: &Path, id: &str) -> Result<SupportedSettings, String> {
    let snapshot = read_snapshot(directory, id)?;
    snapshot.settings.validate()?;
    Ok(snapshot.settings)
}

pub fn content_digest(settings: &SupportedSettings) -> Result<String, String> {
    let bytes = serde_json::to_vec(settings).map_err(|_| "Cannot validate settings content.")?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn summary(settings: &SupportedSettings) -> String {
    format!(
        "{} privacy · decision {} ({}/{}) · cheap {}/{} · intelligent {}/{} · ${:.2} ceiling",
        match settings.mode {
            Mode::Hybrid => "Hybrid",
            Mode::LocalOnly => "Local only",
            Mode::CloudOnly => "Cloud only",
        },
        if settings.decision_enabled {
            "on"
        } else {
            "off"
        },
        settings.decision.provider.name(),
        settings.decision.model,
        settings.cheap.provider.name(),
        settings.cheap.model,
        settings.intelligent.provider.name(),
        settings.intelligent.model,
        settings.budget_usd,
    )
}

pub fn changes(before: &SupportedSettings, after: &SupportedSettings) -> Vec<FieldChange> {
    let rows = [
        (
            "Privacy mode",
            mode_name(&before.mode).to_owned(),
            mode_name(&after.mode).to_owned(),
        ),
        (
            "Decision tier",
            tier_name(&before.decision),
            tier_name(&after.decision),
        ),
        (
            "Cheap tier",
            tier_name(&before.cheap),
            tier_name(&after.cheap),
        ),
        (
            "Intelligent tier",
            tier_name(&before.intelligent),
            tier_name(&after.intelligent),
        ),
        (
            "Decision model",
            if before.decision_enabled {
                "Enabled"
            } else {
                "Disabled"
            }
            .to_owned(),
            if after.decision_enabled {
                "Enabled"
            } else {
                "Disabled"
            }
            .to_owned(),
        ),
        (
            "Budget ceiling",
            format!("${}", before.budget_usd),
            format!("${}", after.budget_usd),
        ),
    ];
    rows.into_iter()
        .filter(|(_, before, after)| before != after)
        .map(|(field, before, after)| FieldChange {
            field: field.to_owned(),
            before,
            after,
        })
        .collect()
}

fn mode_name(mode: &Mode) -> &'static str {
    match mode {
        Mode::Hybrid => "Hybrid",
        Mode::LocalOnly => "Local only",
        Mode::CloudOnly => "Cloud only",
    }
}

fn tier_name(tier: &Tier) -> String {
    format!("{} / {}", tier.provider.name(), tier.model)
}

fn inspect_with_current(
    directory: &Path,
    id: &str,
    current: Option<&CurrentState>,
) -> Result<SnapshotView, String> {
    validate_id(id)?;
    let is_current = current.is_some_and(|value| value.id == id);
    match read_snapshot(directory, id) {
        Ok(snapshot) => {
            if let Err(error) = snapshot.settings.validate() {
                return Ok(error_view(id, is_current, error));
            }
            Ok(valid_view(snapshot, is_current))
        }
        Err(error) => Ok(error_view(id, is_current, error)),
    }
}

fn read_snapshot(directory: &Path, id: &str) -> Result<SnapshotFile, String> {
    validate_id(id)?;
    let path = history_root(directory)
        .join(SNAPSHOTS)
        .join(format!("{id}.json"));
    let bytes = read_bounded(&path)?.ok_or("This saved default no longer exists.")?;
    let snapshot: SnapshotFile = serde_json::from_slice(&bytes)
        .map_err(|_| "This saved default is corrupt or has incompatible fields.")?;
    if snapshot.schema_version != SCHEMA_VERSION {
        return Err("This saved default uses an unsupported future schema.".into());
    }
    if snapshot.id != id {
        return Err("This saved default does not match its immutable file name.".into());
    }
    if snapshot.created_at_ms == 0 || snapshot.created_at_ms > MAX_JAVASCRIPT_DATE_MS {
        return Err("This saved default has an invalid timestamp.".into());
    }
    Ok(snapshot)
}

fn valid_view(snapshot: SnapshotFile, current: bool) -> SnapshotView {
    SnapshotView {
        id: snapshot.id,
        current,
        created_at_ms: Some(snapshot.created_at_ms),
        summary: Some(summary(&snapshot.settings)),
        settings: Some(snapshot.settings),
        error: None,
    }
}

fn error_view(id: &str, current: bool, error: String) -> SnapshotView {
    SnapshotView {
        id: id.to_owned(),
        current,
        created_at_ms: None,
        summary: None,
        settings: None,
        error: Some(error),
    }
}

fn validate_id(id: &str) -> Result<(), String> {
    let bytes = id.as_bytes();
    let valid = bytes.len() == 30
        && bytes[13] == b'-'
        && bytes[..13].iter().all(u8::is_ascii_digit)
        && bytes[14..].iter().all(u8::is_ascii_hexdigit);
    if valid {
        Ok(())
    } else {
        Err("Invalid saved default identifier.".into())
    }
}

fn reserve_snapshot_path(
    directory: &Path,
    created_at_ms: u64,
) -> Result<(String, PathBuf), String> {
    for _ in 0..100 {
        let sequence = FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let id = format!(
            "{created_at_ms:013}-{:08x}{:08x}",
            std::process::id(),
            sequence
        );
        let path = directory.join(format!("{id}.json"));
        if !path.exists() {
            return Ok((id, path));
        }
    }
    Err("Could not allocate a unique saved default identifier.".into())
}

fn history_root(directory: &Path) -> PathBuf {
    directory.join(DIRECTORY)
}

fn now_ms() -> Result<u64, String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System clock is unavailable.")?
        .as_millis();
    u64::try_from(millis).map_err(|_| "System clock is out of range.".into())
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "Cannot serialize saved defaults.")?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("Saved default exceeds the storage size limit.".into());
    }
    Ok(bytes)
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cannot inspect saved default storage.".into()),
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err("Saved default file is unsafe or exceeds the size limit.".into());
    }
    let file = File::open(path).map_err(|_| "Cannot read saved default storage.")?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read saved default storage.")?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("Saved default file exceeds the size limit.".into());
    }
    Ok(Some(bytes))
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|_| "Cannot create saved default storage.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot restrict saved default storage permissions.")?;
        if let Some(parent) = path.parent() {
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .map_err(|_| "Cannot restrict saved default storage permissions.")?;
        }
    }
    Ok(())
}

fn atomic_create(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Missing saved default directory.")?;
    create_private_directory(parent)?;
    let temp = unique_temp_path(parent)?;
    write_new_file(&temp, bytes)?;
    match fs::hard_link(&temp, path) {
        Ok(()) => {
            let _ = fs::remove_file(&temp);
            sync_parent(parent)?;
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(&temp);
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                Err("Saved default identifier collision.".into())
            } else {
                Err("Cannot publish saved default atomically.".into())
            }
        }
    }
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Missing saved default directory.")?;
    create_private_directory(parent)?;
    let temp = unique_temp_path(parent)?;
    write_new_file(&temp, bytes)?;
    if let Err(error) = replace_file(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(format!("Cannot replace saved default metadata: {error}"));
    }
    sync_parent(parent)
}

#[cfg(not(target_os = "windows"))]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

#[cfg(target_os = "windows")]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MOVE_FILE_FLAGS,
    };
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    let flags = MOVE_FILE_FLAGS(MOVEFILE_REPLACE_EXISTING.0 | MOVEFILE_WRITE_THROUGH.0);
    unsafe { MoveFileExW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr()), flags) }
        .map_err(|error| std::io::Error::other(error.to_string()))
}

fn unique_temp_path(parent: &Path) -> Result<PathBuf, String> {
    for _ in 0..100 {
        let sequence = FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".settings-default-{:08x}-{sequence:016x}.tmp",
            std::process::id()
        ));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err("Could not allocate a temporary saved default file.".into())
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "Cannot create saved default storage file.")?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(path);
        return Err(format!("Cannot persist saved default storage: {error}"));
    }
    Ok(())
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<(), String> {
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "Cannot durably persist the saved default directory.".into())
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::settings::{Mode, Settings};

    fn test_directory(label: &str) -> std::path::PathBuf {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "jitterbug-{label}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    #[test]
    fn repeated_capture_preserves_history_across_restart_without_automation() {
        let directory = test_directory("settings-history");
        let _ = std::fs::remove_dir_all(&directory);
        let mut first = Settings::default();
        first.automation.enabled = false;
        let saved_first = super::capture(&directory, &first).unwrap();
        let mut second = first.clone();
        second.mode = Mode::CloudOnly;
        second.budget_usd = 7.5;
        second.automation.enabled = true;
        let saved_second = super::capture(&directory, &second).unwrap();

        assert_ne!(saved_first.id, saved_second.id);
        let first_page = super::history(&directory, None, 1).unwrap();
        assert_eq!(first_page.entries.len(), 1);
        assert_eq!(first_page.entries[0].id, saved_second.id);
        assert!(first_page.entries[0].current);
        let second_page = super::history(&directory, first_page.next_cursor.as_deref(), 1).unwrap();
        assert_eq!(second_page.entries[0].id, saved_first.id);
        assert!(!second_page.entries[0].current);
        assert!(second_page.next_cursor.is_none());

        let restarted = super::inspect(&directory, &saved_first.id).unwrap();
        let supported = restarted.settings.unwrap();
        assert_eq!(supported.mode, Mode::Hybrid);
        let value = serde_json::to_value(supported).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 6);
        assert!(value.get("automation").is_none());
        assert!(value.get("credentials").is_none());
        assert!(value.get("spending").is_none());
        assert!(value.get("telemetry").is_none());
        assert!(value.get("process_priorities").is_none());
        assert!(value.get("queue").is_none());
        assert!(value.get("command_authorization").is_none());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn many_defaults_are_paginated_without_pruning() {
        let directory = test_directory("settings-many-defaults");
        let mut expected = Vec::new();
        for index in 0..55 {
            let settings = Settings {
                budget_usd: f64::from(index % 20),
                ..Default::default()
            };
            expected.push(super::capture(&directory, &settings).unwrap().id);
        }
        expected.reverse();
        let mut actual = Vec::new();
        let mut cursor = None;
        loop {
            let page = super::history(&directory, cursor.as_deref(), 10).unwrap();
            actual.extend(page.entries.into_iter().map(|entry| entry.id));
            let Some(next) = page.next_cursor else { break };
            cursor = Some(next);
        }
        assert_eq!(actual, expected);
        assert!(super::history(&directory, None, super::MAX_PAGE_SIZE + 1).is_err());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn corrupt_and_incompatible_entries_are_visible_without_blocking_valid_history() {
        let directory = test_directory("settings-corrupt-history");
        let first = super::capture(&directory, &Settings::default()).unwrap();
        let second = super::capture(&directory, &Settings::default()).unwrap();
        let missing = super::capture(&directory, &Settings::default()).unwrap();
        let first_path = super::history_root(&directory)
            .join(super::SNAPSHOTS)
            .join(format!("{}.json", first.id));
        std::fs::write(&first_path, b"{not json").unwrap();
        let missing_path = super::history_root(&directory)
            .join(super::SNAPSHOTS)
            .join(format!("{}.json", missing.id));
        std::fs::write(&missing_path, br#"{"schema_version":1}"#).unwrap();

        let page = super::history(&directory, None, 10).unwrap();
        assert_eq!(page.entries.len(), 3);
        assert!(page
            .entries
            .iter()
            .any(|entry| entry.id == second.id && entry.error.is_none()));
        assert!(page
            .entries
            .iter()
            .any(|entry| entry.id == first.id && entry.error.is_some()));
        assert!(page.entries.iter().any(|entry| entry.id == missing.id
            && entry.error.as_deref().unwrap().contains("incompatible")));

        let second_path = super::history_root(&directory)
            .join(super::SNAPSHOTS)
            .join(format!("{}.json", second.id));
        let future = std::fs::read_to_string(&second_path).unwrap().replacen(
            "\"schema_version\": 1",
            "\"schema_version\": 2",
            1,
        );
        std::fs::write(&second_path, future).unwrap();
        assert!(super::inspect(&directory, &second.id)
            .unwrap()
            .error
            .unwrap()
            .contains("future schema"));
        assert!(super::inspect(&directory, "../../settings")
            .unwrap_err()
            .contains("identifier"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn timestamp_outside_javascript_date_range_is_corrupt_without_blocking_history() {
        let directory = test_directory("settings-invalid-timestamp");
        let valid = super::capture(&directory, &Settings::default()).unwrap();
        let corrupt = super::capture(&directory, &Settings::default()).unwrap();
        let corrupt_path = super::history_root(&directory)
            .join(super::SNAPSHOTS)
            .join(format!("{}.json", corrupt.id));
        let mut contents: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&corrupt_path).unwrap()).unwrap();
        contents["created_at_ms"] = serde_json::Value::from(u64::MAX);
        std::fs::write(&corrupt_path, serde_json::to_vec_pretty(&contents).unwrap()).unwrap();

        let page = super::history(&directory, None, 10).unwrap();
        assert!(page
            .entries
            .iter()
            .any(|entry| entry.id == valid.id && entry.error.is_none()));
        let invalid = page
            .entries
            .iter()
            .find(|entry| entry.id == corrupt.id)
            .unwrap();
        assert!(invalid.created_at_ms.is_none());
        assert!(invalid
            .error
            .as_deref()
            .unwrap()
            .contains("invalid timestamp"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn preview_does_not_hide_subcent_budget_changes() {
        let before = super::SupportedSettings::from_saved(&Settings {
            budget_usd: 1.001,
            ..Default::default()
        });
        let after = super::SupportedSettings {
            budget_usd: 1.002,
            ..before.clone()
        };
        let changes = super::changes(&before, &after);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].before, "$1.001");
        assert_eq!(changes[0].after, "$1.002");
    }

    #[test]
    fn missing_current_snapshot_is_reported_in_history() {
        let directory = test_directory("settings-missing-current");
        let saved = super::capture(&directory, &Settings::default()).unwrap();
        let snapshots = super::history_root(&directory).join(super::SNAPSHOTS);
        std::fs::remove_dir_all(snapshots).unwrap();

        let page = super::history(&directory, None, 10).unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].id, saved.id);
        assert!(page.entries[0].current);
        assert!(page.entries[0]
            .error
            .as_deref()
            .unwrap()
            .contains("no longer exists"));
        let _ = std::fs::remove_dir_all(directory);
    }
}
