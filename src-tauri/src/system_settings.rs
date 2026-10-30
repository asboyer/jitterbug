mod capabilities;
mod hardware;
pub use hardware::HardwareValue;
#[cfg(target_os = "macos")]
mod background_items;
#[cfg(target_os = "macos")]
mod displays;
#[cfg(target_os = "macos")]
mod login_items;
#[cfg(target_os = "macos")]
mod power;
mod preferences;
#[cfg(target_os = "macos")]
mod spotlight;
#[cfg(target_os = "macos")]
mod time_machine;
pub use capabilities::{settings_url, Capability, CapabilityStatus};
pub use preferences::{Preference, PreferenceValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DIRECTORY: &str = "system-settings-defaults-v1";
const SNAPSHOTS: &str = "snapshots";
const CURRENT: &str = "current.json";
const BINDING_KEY: &str = "binding-key";
const JOURNALS: &str = "rollback-journals";
const SCHEMA_VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 64 * 1024;
const PREVIEW_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_PREVIEWS: usize = 32;
pub const MAX_PAGE_SIZE: usize = 50;
static FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemValues {
    pub output_volume: Option<f32>,
    pub output_muted: Option<bool>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub preferences: BTreeMap<Preference, PreferenceValue>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hardware: BTreeMap<u16, HardwareValue>,
}

impl SystemValues {
    fn validate(&self) -> Result<(), String> {
        if self.hardware.len() > 64 {
            return Err("Hardware inventory exceeds safe bound".into());
        }
        for (slot, value) in &self.hardware {
            value.validate(*slot)?;
        }
        for (key, value) in &self.preferences {
            key.validate(*value)?;
        }
        if self.output_volume.is_none()
            && self.output_muted.is_none()
            && self.preferences.is_empty()
            && self.hardware.is_empty()
        {
            return Err(
                "No supported settable controls or selected preferences are available; recheck coverage and category selection.".into(),
            );
        }
        if self
            .output_volume
            .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err("The audio output volume is outside its supported range.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeIdentity {
    pub platform: String,
    pub os_version: String,
    pub user: String,
    pub machine: String,
    pub device: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeState {
    pub identity: NativeIdentity,
    pub values: SystemValues,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Setting {
    OutputVolume,
    OutputMuted,
    Preference(Preference),
    Hardware(u16),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SettingValue {
    Volume(f32),
    Muted(bool),
    Preference(PreferenceValue),
    Hardware(HardwareValue),
}

pub trait NativeSettings: Send + Sync {
    fn validate_target(&self, values: &SystemValues) -> Result<(), String> {
        values.validate()
    }
    fn read(&self) -> Result<NativeState, String>;
    fn coverage(&self, values: &SystemValues) -> Vec<Capability> {
        capabilities::coverage(values)
    }
    fn write(
        &self,
        expected: &NativeState,
        setting: Setting,
        value: SettingValue,
    ) -> Result<(), String>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompatibilityContext {
    platform: String,
    os_version: String,
    user_binding: String,
    machine_binding: String,
    device_binding: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotFile {
    schema_version: u32,
    id: String,
    created_at_ms: u64,
    context: CompatibilityContext,
    values: SystemValues,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    coverage: Vec<Capability>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentState {
    schema_version: u32,
    id: String,
    revision: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SnapshotView {
    pub id: String,
    pub current: bool,
    pub created_at_ms: Option<u64>,
    pub summary: Option<String>,
    pub values: Option<SystemValues>,
    pub compatible: bool,
    pub error: Option<String>,
    pub coverage: Vec<Capability>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HistoryPage {
    pub entries: Vec<SnapshotView>,
    pub next_cursor: Option<String>,
    pub interrupted_operations: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct FieldChange {
    pub group: String,
    pub verification: String,
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
    pub coverage: Vec<Capability>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Pending,
    Applying,
    Verified,
    WriteFailed,
    ReadbackMismatch,
    ExternalDrift,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingOutcome {
    pub setting: Setting,
    pub field: String,
    pub before: String,
    pub target: String,
    pub status: OutcomeStatus,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackStatus {
    NoChanges,
    Verified,
    PartialFailure,
}

#[derive(Clone, Debug, Serialize)]
pub struct RollbackResult {
    pub status: RollbackStatus,
    pub outcomes: Vec<SettingOutcome>,
    pub current: Option<SystemValues>,
    pub journal_id: Option<String>,
    pub message: String,
}

struct PendingPreview {
    token: String,
    target_id: String,
    target_digest: String,
    expected_context: CompatibilityContext,
    expected_values: SystemValues,
    expires_at_ms: u64,
}

#[derive(Default)]
struct PendingPreviews {
    entries: VecDeque<PendingPreview>,
}

impl PendingPreviews {
    fn issue(&mut self, preview: PendingPreview) {
        self.entries
            .retain(|entry| entry.expires_at_ms > now_ms().unwrap_or(u64::MAX));
        self.entries.push_back(preview);
        while self.entries.len() > MAX_PREVIEWS {
            self.entries.pop_front();
        }
    }

    fn take(&mut self, token: &str) -> Option<PendingPreview> {
        let position = self.entries.iter().position(|entry| entry.token == token)?;
        self.entries.remove(position)
    }
}

pub struct Controller {
    directory: PathBuf,
    native: Arc<dyn NativeSettings>,
    persistence: Mutex<()>,
    previews: Mutex<PendingPreviews>,
    preview_ttl: Duration,
}

impl Controller {
    pub fn new(directory: PathBuf, native: Arc<dyn NativeSettings>) -> Self {
        Self {
            directory,
            native,
            persistence: Mutex::new(()),
            previews: Mutex::new(PendingPreviews::default()),
            preview_ttl: PREVIEW_TTL,
        }
    }

    pub fn inventory(&self) -> Vec<Capability> {
        match self.native.read() {
            Ok(state) => self.native.coverage(&state.values),
            Err(error) => {
                let mut items = capabilities::coverage(&SystemValues {
                    output_volume: None,
                    output_muted: None,
                    preferences: BTreeMap::new(),
                    hardware: BTreeMap::new(),
                });
                for item in &mut items {
                    item.status = CapabilityStatus::Unavailable;
                    item.detail = error.clone();
                    item.pane = None;
                }
                items
            }
        }
    }

    pub fn capture(&self) -> Result<SnapshotView, String> {
        self.capture_selected(None)
    }

    pub fn capture_selected(&self, groups: Option<&[String]>) -> Result<SnapshotView, String> {
        if let Some(groups) = groups {
            if groups.is_empty()
                || groups.len() > 11
                || groups.iter().any(|group| {
                    ![
                        "Audio",
                        "Desktop & Dock",
                        "Keyboard",
                        "Pointer",
                        "Finder storage",
                        "Power profiles",
                        "Displays",
                        "Audio hardware",
                        "Storage",
                        "Backup",
                        "Login & background",
                    ]
                    .contains(&group.as_str())
                })
            {
                return Err("Choose at least one supported category: Audio, Audio hardware, Displays, Power profiles, Desktop & Dock, Keyboard, Pointer, or Finder storage.".into());
            }
        }
        let _guard = self
            .persistence
            .lock()
            .map_err(|_| "System settings history is unavailable.")?;
        let mut native = self.native.read()?;
        let mut coverage = self.native.coverage(&native.values);
        if let Some(groups) = groups {
            let selected = |group: &str| groups.iter().any(|item| item == group);
            if !selected("Audio") {
                native.values.output_volume = None;
                native.values.output_muted = None;
            }
            native
                .values
                .preferences
                .retain(|key, _| selected(key.descriptor().3));
            native
                .values
                .hardware
                .retain(|_, value| selected(value.group()));
            for item in &mut coverage {
                if item.included && !selected(&item.group) {
                    item.included = false;
                    item.detail = format!("Not selected for this capture. {}", item.detail);
                }
            }
        }
        native.values.validate()?;
        let context = self.bind(&native.identity)?;
        let current = self.current_state()?;
        let revision = current
            .as_ref()
            .map_or(1, |state| state.revision.saturating_add(1));
        let created_at_ms = now_ms()?;
        let snapshots = self.root().join(SNAPSHOTS);
        create_private_directory(&snapshots)?;
        let (id, path) = reserve_snapshot_path(&snapshots, created_at_ms)?;
        let snapshot = SnapshotFile {
            schema_version: SCHEMA_VERSION,
            id: id.clone(),
            created_at_ms,
            context,
            coverage,
            values: native.values,
        };
        atomic_create(&path, &encode(&snapshot)?)?;
        let pointer = CurrentState {
            schema_version: SCHEMA_VERSION,
            id,
            revision,
        };
        atomic_replace(&self.root().join(CURRENT), &encode(&pointer)?)?;
        Ok(snapshot_view(snapshot, true, true))
    }

    pub fn history(&self, cursor: Option<&str>, page_size: usize) -> Result<HistoryPage, String> {
        if page_size == 0 || page_size > MAX_PAGE_SIZE {
            return Err(format!(
                "Choose a history page size from 1 to {MAX_PAGE_SIZE}."
            ));
        }
        if let Some(cursor) = cursor {
            validate_id(cursor)?;
        }
        let _guard = self
            .persistence
            .lock()
            .map_err(|_| "System settings history is unavailable.")?;
        let current = self.current_state()?;
        let live_context = self
            .native
            .read()
            .and_then(|state| self.bind(&state.identity))
            .ok();
        let snapshots = self.root().join(SNAPSHOTS);
        let read = match fs::read_dir(&snapshots) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(HistoryPage {
                    entries: Vec::new(),
                    next_cursor: None,
                    interrupted_operations: self.interrupted_operations()?,
                });
            }
            Err(_) => return Err("Cannot read system settings history.".into()),
        };
        let mut ids = Vec::with_capacity(page_size + 1);
        for entry in read {
            let entry = entry.map_err(|_| "Cannot read a system settings history entry.")?;
            let kind = entry
                .file_type()
                .map_err(|_| "Cannot inspect a system settings history entry.")?;
            if !kind.is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            if validate_id(id).is_err() || cursor.is_some_and(|cursor| id >= cursor) {
                continue;
            }
            let position = ids
                .binary_search_by(|probe: &String| probe.as_str().cmp(id).reverse())
                .unwrap_or_else(|position| position);
            ids.insert(position, id.to_owned());
            ids.truncate(page_size + 1);
        }
        let has_more = ids.len() > page_size;
        ids.truncate(page_size);
        let next_cursor = has_more.then(|| ids.last().cloned()).flatten();
        let entries = ids
            .into_iter()
            .map(|id| self.inspect_inner(&id, current.as_ref(), live_context.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(HistoryPage {
            entries,
            next_cursor,
            interrupted_operations: self.interrupted_operations()?,
        })
    }

    pub fn inspect(&self, id: &str) -> Result<SnapshotView, String> {
        let _guard = self
            .persistence
            .lock()
            .map_err(|_| "System settings history is unavailable.")?;
        let current = self.current_state()?;
        let live = self
            .native
            .read()
            .and_then(|state| self.bind(&state.identity))
            .ok();
        self.inspect_inner(id, current.as_ref(), live.as_ref())
    }

    pub fn preview(&self, id: &str) -> Result<RestorePreview, String> {
        let _guard = self
            .persistence
            .lock()
            .map_err(|_| "System settings history is unavailable.")?;
        let current = self.native.read()?;
        current.values.validate()?;
        let current_context = self.bind(&current.identity)?;
        let target = self.read_snapshot(id)?;
        ensure_compatible(
            &current_context,
            &target.context,
            &current.values,
            &target.values,
        )?;
        self.native.validate_target(&target.values)?;
        let changes = changes(&current.values, &target.values);
        let mut coverage = self.native.coverage(&current.values);
        for item in &mut coverage {
            item.included = match item.id.as_str() {
                "output_volume" => target.values.output_volume.is_some(),
                "output_muted" => target.values.output_muted.is_some(),
                _ if item.id.starts_with("hardware_") => item.id[9..]
                    .parse::<u16>()
                    .is_ok_and(|slot| target.values.hardware.contains_key(&slot)),
                _ => target
                    .values
                    .preferences
                    .keys()
                    .any(|key| key.descriptor().2 == item.label),
            };
        }
        let token = secure_token()?;
        let expires_at_ms = now_ms()?
            .saturating_add(u64::try_from(self.preview_ttl.as_millis()).unwrap_or(u64::MAX));
        self.previews
            .lock()
            .map_err(|_| "System settings preview state is unavailable.")?
            .issue(PendingPreview {
                token: token.clone(),
                target_id: id.to_owned(),
                target_digest: content_digest(&target)?,
                expected_context: current_context.clone(),
                expected_values: current.values,
                expires_at_ms,
            });
        Ok(RestorePreview {
            token,
            expires_at_ms,
            target: snapshot_view(target, false, true),
            changes,
            coverage,
        })
    }

    pub fn cancel(&self, token: &str) -> Result<(), String> {
        self.previews
            .lock()
            .map_err(|_| "System settings preview state is unavailable.")?
            .take(token)
            .map(|_| ())
            .ok_or_else(|| "Rollback preview is unavailable or already used.".into())
    }

    pub fn confirm(&self, token: &str) -> Result<RollbackResult, String> {
        // Consume before validation: rejected, expired, and stale confirmations are one-use.
        let pending = self
            .previews
            .lock()
            .map_err(|_| "System settings preview state is unavailable.")?
            .take(token)
            .ok_or("Rollback preview is unavailable or already used.")?;
        if pending.expires_at_ms <= now_ms()? {
            return Err("Rollback preview expired. Review current values again.".into());
        }
        let _guard = self
            .persistence
            .lock()
            .map_err(|_| "System settings history is unavailable.")?;
        let current = self.native.read()?;
        current.values.validate()?;
        let current_context = self.bind(&current.identity)?;
        let target = self.read_snapshot(&pending.target_id)?;
        let stale = current_context != pending.expected_context
            || !values_equal(&current.values, &pending.expected_values)
            || content_digest(&target)? != pending.target_digest;
        if stale {
            return Err(
                "This rollback preview is stale because actual system settings or compatibility changed."
                    .into(),
            );
        }
        ensure_compatible(
            &current_context,
            &target.context,
            &current.values,
            &target.values,
        )?;
        self.native.validate_target(&target.values)?;
        let plan = planned_settings(&current.values, &target.values);
        if plan.is_empty() {
            return Ok(RollbackResult {
                status: RollbackStatus::NoChanges,
                outcomes: Vec::new(),
                current: Some(current.values),
                journal_id: None,
                message: "Actual system settings already match this baseline; no writes were made."
                    .into(),
            });
        }

        let started_at_ms = now_ms()?;
        let journals = self.root().join(JOURNALS);
        create_private_directory(&journals)?;
        let (journal_id, journal_path) = reserve_snapshot_path(&journals, started_at_ms)?;
        let mut journal = JournalFile {
            schema_version: SCHEMA_VERSION,
            id: journal_id.clone(),
            target_id: pending.target_id,
            started_at_ms,
            status: JournalStatus::Intent,
            expected_context: current_context.clone(),
            expected_values: current.values.clone(),
            target_values: target.values.clone(),
            outcomes: plan
                .iter()
                .map(|planned| SettingOutcome {
                    setting: planned.setting,
                    field: planned.field.to_owned(),
                    before: display_value(planned.before.clone()),
                    target: display_value(planned.target.clone()),
                    status: OutcomeStatus::Pending,
                    error: None,
                })
                .collect(),
        };
        // This create-new + fsync is the durable intent boundary. No native write occurs before it.
        atomic_create(&journal_path, &encode(&journal)?)?;

        let mut expected = current.values;
        for (index, planned) in plan.iter().enumerate() {
            let fresh = match self.native.read() {
                Ok(fresh) => fresh,
                Err(error) => {
                    journal.outcomes[index].status = OutcomeStatus::ExternalDrift;
                    journal.outcomes[index].error = Some(format!(
                        "Actual settings could not be re-read before the write: {error}"
                    ));
                    return self.finish_partial(journal_path, journal, None);
                }
            };
            let fresh_context = self.bind(&fresh.identity)?;
            if fresh_context != current_context || !values_equal(&fresh.values, &expected) {
                journal.outcomes[index].status = OutcomeStatus::ExternalDrift;
                journal.outcomes[index].error = Some(
                    "Actual settings changed after confirmation; no further writes were made."
                        .into(),
                );
                return self.finish_partial(journal_path, journal, Some(fresh.values));
            }
            journal.status = JournalStatus::Applying;
            journal.outcomes[index].status = OutcomeStatus::Applying;
            atomic_replace(&journal_path, &encode(&journal)?)?;
            if let Err(error) = self
                .native
                .write(&fresh, planned.setting, planned.target.clone())
            {
                journal.outcomes[index].status = OutcomeStatus::WriteFailed;
                journal.outcomes[index].error = Some(error);
                let actual = self.native.read().ok().map(|state| state.values);
                return self.finish_partial(journal_path, journal, actual);
            }
            set_value(&mut expected, planned.setting, planned.target.clone());
            let readback = match self.native.read() {
                Ok(readback) => readback,
                Err(error) => {
                    journal.outcomes[index].status = OutcomeStatus::ReadbackMismatch;
                    journal.outcomes[index].error =
                        Some(format!("Native readback failed after the write: {error}"));
                    return self.finish_partial(journal_path, journal, None);
                }
            };
            let readback_context = match self.bind(&readback.identity) {
                Ok(context) => context,
                Err(error) => {
                    journal.outcomes[index].status = OutcomeStatus::ReadbackMismatch;
                    journal.outcomes[index].error = Some(format!(
                        "Native identity validation failed after the write: {error}"
                    ));
                    return self.finish_partial(journal_path, journal, Some(readback.values));
                }
            };
            if readback_context != current_context || !values_equal(&readback.values, &expected) {
                journal.outcomes[index].status = OutcomeStatus::ReadbackMismatch;
                journal.outcomes[index].error = Some(
                    "Native readback did not exactly match the requested setting while preserving unrelated values."
                        .into(),
                );
                return self.finish_partial(journal_path, journal, Some(readback.values));
            }
            journal.outcomes[index].status = OutcomeStatus::Verified;
            atomic_replace(&journal_path, &encode(&journal)?)?;
        }
        let final_state = match self.native.read() {
            Ok(state) => state,
            Err(error) => {
                if let Some(last) = journal.outcomes.last_mut() {
                    last.status = OutcomeStatus::ReadbackMismatch;
                    last.error = Some(format!("Final native readback failed: {error}"));
                }
                return self.finish_partial(journal_path, journal, None);
            }
        };
        let final_context = match self.bind(&final_state.identity) {
            Ok(context) => context,
            Err(error) => {
                if let Some(last) = journal.outcomes.last_mut() {
                    last.status = OutcomeStatus::ReadbackMismatch;
                    last.error = Some(format!("Final native identity validation failed: {error}"));
                }
                return self.finish_partial(journal_path, journal, Some(final_state.values));
            }
        };
        if final_context != target.context || !values_equal(&final_state.values, &expected) {
            if let Some(last) = journal.outcomes.last_mut() {
                last.status = OutcomeStatus::ReadbackMismatch;
                last.error = Some("Final native readback no longer matches the baseline.".into());
            }
            return self.finish_partial(journal_path, journal, Some(final_state.values));
        }
        journal.status = JournalStatus::Verified;
        atomic_replace(&journal_path, &encode(&journal)?)?;
        Ok(RollbackResult {
            status: RollbackStatus::Verified,
            outcomes: journal.outcomes,
            current: Some(final_state.values),
            journal_id: Some(journal_id),
            message: "Requested values verified by native readback. Current audio configuration and display modes restored where selected; reboot/reconnect persistence not verified. For display changes, permanent scope is requested but macOS can fall back to login-session scope. Power verification covers saved per-source timers, not effective sleep. Preferences verify saved storage only; applications may require relaunch or sign-out. No processes restarted; performance not verified.".into(),
        })
    }

    fn finish_partial(
        &self,
        path: PathBuf,
        mut journal: JournalFile,
        current: Option<SystemValues>,
    ) -> Result<RollbackResult, String> {
        journal.status = JournalStatus::PartialFailure;
        atomic_replace(&path, &encode(&journal)?)?;
        Ok(RollbackResult {
            status: RollbackStatus::PartialFailure,
            outcomes: journal.outcomes,
            current,
            journal_id: Some(journal.id),
            message: "Rollback stopped after a failed write or readback. Verified earlier changes were not automatically replayed or undone.".into(),
        })
    }

    fn inspect_inner(
        &self,
        id: &str,
        current: Option<&CurrentState>,
        live: Option<&CompatibilityContext>,
    ) -> Result<SnapshotView, String> {
        validate_id(id)?;
        let is_current = current.is_some_and(|state| state.id == id);
        match self.read_snapshot(id) {
            Ok(snapshot) => {
                let compatible = live.is_some_and(|context| *context == snapshot.context);
                Ok(snapshot_view(snapshot, is_current, compatible))
            }
            Err(error) => Ok(SnapshotView {
                id: id.to_owned(),
                current: is_current,
                created_at_ms: None,
                summary: None,
                values: None,
                compatible: false,
                error: Some(error),
                coverage: Vec::new(),
            }),
        }
    }

    fn read_snapshot(&self, id: &str) -> Result<SnapshotFile, String> {
        validate_id(id)?;
        let path = self.root().join(SNAPSHOTS).join(format!("{id}.json"));
        let bytes =
            read_bounded(&path)?.ok_or("This system settings baseline no longer exists.")?;
        let snapshot: SnapshotFile = serde_json::from_slice(&bytes)
            .map_err(|_| "This system settings baseline is corrupt or incompatible.")?;
        if snapshot.schema_version != SCHEMA_VERSION || snapshot.id != id {
            return Err("This system settings baseline has incompatible metadata.".into());
        }
        snapshot.values.validate()?;
        Ok(snapshot)
    }

    fn current_state(&self) -> Result<Option<CurrentState>, String> {
        let Some(bytes) = read_bounded(&self.root().join(CURRENT))? else {
            return Ok(None);
        };
        let state: CurrentState = serde_json::from_slice(&bytes)
            .map_err(|_| "Current system settings baseline metadata is corrupt.")?;
        if state.schema_version != SCHEMA_VERSION || state.revision == 0 {
            return Err("Current system settings baseline metadata is incompatible.".into());
        }
        validate_id(&state.id)?;
        Ok(Some(state))
    }

    fn bind(&self, identity: &NativeIdentity) -> Result<CompatibilityContext, String> {
        if identity.platform.is_empty()
            || identity.os_version.is_empty()
            || identity.user.is_empty()
            || identity.machine.is_empty()
            || identity.device.is_empty()
        {
            return Err("Native compatibility identity is incomplete.".into());
        }
        let key = self.binding_key()?;
        let binding = |label: &str, value: &str| {
            let mut digest = Sha256::new();
            digest.update(&key);
            digest.update(label.as_bytes());
            digest.update(value.as_bytes());
            format!("{:x}", digest.finalize())
        };
        Ok(CompatibilityContext {
            platform: identity.platform.clone(),
            os_version: identity.os_version.clone(),
            user_binding: binding("user", &identity.user),
            machine_binding: binding("machine", &identity.machine),
            device_binding: binding("device", &identity.device),
        })
    }

    fn binding_key(&self) -> Result<Vec<u8>, String> {
        let path = self.root().join(BINDING_KEY);
        if let Some(key) = read_bounded(&path)? {
            if key.len() == 32 {
                return Ok(key);
            }
            return Err("System settings compatibility key is corrupt.".into());
        }
        create_private_directory(&self.root())?;
        let mut key = vec![0_u8; 32];
        getrandom::fill(&mut key)
            .map_err(|_| "Cannot create a system settings compatibility key.")?;
        match atomic_create(&path, &key) {
            Ok(()) => Ok(key),
            Err(_) => read_bounded(&path)?
                .filter(|saved| saved.len() == 32)
                .ok_or_else(|| "Cannot create a system settings compatibility key.".into()),
        }
    }

    fn interrupted_operations(&self) -> Result<usize, String> {
        let read = match fs::read_dir(self.root().join(JOURNALS)) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(_) => return Err("Cannot inspect rollback journals.".into()),
        };
        let mut interrupted = 0;
        for entry in read {
            let entry = entry.map_err(|_| "Cannot inspect a rollback journal.")?;
            if !entry
                .file_type()
                .map_err(|_| "Cannot inspect a rollback journal.")?
                .is_file()
            {
                continue;
            }
            let Some(bytes) = read_bounded(&entry.path())? else {
                continue;
            };
            match serde_json::from_slice::<JournalFile>(&bytes) {
                Ok(journal)
                    if matches!(
                        journal.status,
                        JournalStatus::Intent | JournalStatus::Applying
                    ) =>
                {
                    interrupted += 1;
                }
                Err(_) => interrupted += 1,
                _ => {}
            }
        }
        Ok(interrupted)
    }

    fn root(&self) -> PathBuf {
        self.directory.join(DIRECTORY)
    }
}

#[derive(Clone)]
struct PlannedSetting {
    setting: Setting,
    field: &'static str,
    before: SettingValue,
    target: SettingValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JournalStatus {
    Intent,
    Applying,
    Verified,
    PartialFailure,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalFile {
    schema_version: u32,
    id: String,
    target_id: String,
    started_at_ms: u64,
    status: JournalStatus,
    expected_context: CompatibilityContext,
    expected_values: SystemValues,
    target_values: SystemValues,
    outcomes: Vec<SettingOutcome>,
}

fn planned_settings(before: &SystemValues, target: &SystemValues) -> Vec<PlannedSetting> {
    let mut result = Vec::new();
    if let (Some(before), Some(target)) = (before.output_volume, target.output_volume) {
        if (before - target).abs() > f32::EPSILON {
            result.push(PlannedSetting {
                setting: Setting::OutputVolume,
                field: "Output volume",
                before: SettingValue::Volume(before),
                target: SettingValue::Volume(target),
            });
        }
    }
    if let (Some(before), Some(target)) = (before.output_muted, target.output_muted) {
        if before != target {
            result.push(PlannedSetting {
                setting: Setting::OutputMuted,
                field: "Output mute",
                before: SettingValue::Muted(before),
                target: SettingValue::Muted(target),
            });
        }
    }
    for (key, target) in &target.preferences {
        if let Some(before) = before.preferences.get(key) {
            if before != target {
                result.push(PlannedSetting {
                    setting: Setting::Preference(*key),
                    field: key.descriptor().2,
                    before: SettingValue::Preference(*before),
                    target: SettingValue::Preference(*target),
                });
            }
        }
    }
    for (slot, target) in &target.hardware {
        if let Some(before) = before.hardware.get(slot) {
            if before != target {
                result.push(PlannedSetting {
                    setting: Setting::Hardware(*slot),
                    field: target.label(),
                    before: SettingValue::Hardware(before.clone()),
                    target: SettingValue::Hardware(target.clone()),
                });
            }
        }
    }
    result
}

fn changes(before: &SystemValues, target: &SystemValues) -> Vec<FieldChange> {
    planned_settings(before, target)
        .into_iter()
        .map(|planned| FieldChange {
            group: match planned.setting {
                Setting::Hardware(_) => match &planned.target {
                    SettingValue::Hardware(v) => v.group(),
                    _ => unreachable!(),
                },
                Setting::Preference(key) => key.descriptor().3,
                _ => "Audio",
            }
            .into(),
            verification: match planned.setting {
                Setting::Hardware(_) => match &planned.target {
                    SettingValue::Hardware(v) => v.verification(),
                    _ => unreachable!(),
                },
                Setting::Preference(_) => "Saved preference only; live effect not verified",
                _ => "Live CoreAudio readback",
            }
            .into(),
            field: planned.field.into(),
            before: display_value(planned.before.clone()),
            after: display_value(planned.target.clone()),
        })
        .collect()
}

fn display_value(value: SettingValue) -> String {
    match value {
        SettingValue::Hardware(value) => value.description(),
        SettingValue::Volume(value) => format!("{:.1}%", value * 100.0),
        SettingValue::Muted(true) => "Muted".into(),
        SettingValue::Muted(false) => "Unmuted".into(),
        SettingValue::Preference(value) => match value {
            PreferenceValue::Absent => "Absent (no override in this scope)".into(),
            PreferenceValue::Boolean(v) => format!("Boolean: {v}"),
            PreferenceValue::Integer(v) => format!("Integer: {v}"),
            PreferenceValue::Real(v) => format!("Real: {v}"),
        },
    }
}

fn set_value(values: &mut SystemValues, setting: Setting, value: SettingValue) {
    match (setting, value) {
        (Setting::Hardware(slot), SettingValue::Hardware(value)) => {
            values.hardware.insert(slot, value);
        }
        (Setting::OutputVolume, SettingValue::Volume(value)) => {
            values.output_volume = Some(value);
        }
        (Setting::OutputMuted, SettingValue::Muted(value)) => {
            values.output_muted = Some(value);
        }
        (Setting::Preference(key), SettingValue::Preference(value)) => {
            values.preferences.insert(key, value);
        }
        _ => {}
    }
}

fn values_equal(left: &SystemValues, right: &SystemValues) -> bool {
    let volume_equal = match (left.output_volume, right.output_volume) {
        (Some(left), Some(right)) => (left - right).abs() <= 0.000_1,
        (None, None) => true,
        _ => false,
    };
    volume_equal
        && left.output_muted == right.output_muted
        && left.preferences == right.preferences
        && left.hardware == right.hardware
}

fn ensure_compatible(
    current: &CompatibilityContext,
    target: &CompatibilityContext,
    current_values: &SystemValues,
    target_values: &SystemValues,
) -> Result<(), String> {
    if current != target {
        return Err(
            "This baseline belongs to a different OS, user, machine, or default output device."
                .into(),
        );
    }
    for (slot, target) in &target_values.hardware {
        if !current_values
            .hardware
            .get(slot)
            .is_some_and(|live| live.same_identity(target))
        {
            return Err("Hardware or power source identity/topology changed or control unavailable; capture/review again.".into());
        }
    }
    for key in target_values.preferences.keys() {
        if !current_values.preferences.contains_key(key) {
            return Err(format!(
                "{} is unavailable; recheck access and capture a new baseline.",
                key.descriptor().2
            ));
        }
    }
    if (target_values.output_volume.is_some() && current_values.output_volume.is_none())
        || (target_values.output_muted.is_some() && current_values.output_muted.is_none())
    {
        return Err("The default output device's supported controls changed.".into());
    }
    Ok(())
}

fn content_digest(snapshot: &SnapshotFile) -> Result<String, String> {
    let bytes = serde_json::to_vec(snapshot)
        .map_err(|_| "Cannot validate system settings baseline content.")?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn secure_token() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "Cannot create a secure rollback token.")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn snapshot_view(snapshot: SnapshotFile, current: bool, compatible: bool) -> SnapshotView {
    SnapshotView {
        id: snapshot.id,
        current,
        created_at_ms: Some(snapshot.created_at_ms),
        summary: Some(summary(&snapshot.values)),
        values: Some(snapshot.values),
        coverage: snapshot.coverage,
        compatible,
        error: None,
    }
}

fn summary(values: &SystemValues) -> String {
    let volume = values
        .output_volume
        .map(|value| format!("volume {:.0}%", value * 100.0));
    let muted = values.output_muted.map(|value| {
        if value {
            "muted".to_owned()
        } else {
            "unmuted".to_owned()
        }
    });
    let preferences = (!values.preferences.is_empty()).then(|| {
        format!(
            "{} saved preferences (live effect not verified)",
            values.preferences.len()
        )
    });
    let hardware = (!values.hardware.is_empty()).then(|| {
        format!(
            "{} hardware/power targets (reboot persistence not verified)",
            values.hardware.len()
        )
    });
    [volume, muted, preferences, hardware]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

fn now_ms() -> Result<u64, String> {
    let value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System clock is unavailable.")?
        .as_millis();
    u64::try_from(value).map_err(|_| "System clock is out of range.".into())
}

fn validate_id(id: &str) -> Result<(), String> {
    let bytes = id.as_bytes();
    let valid = bytes.len() == 30
        && bytes[13] == b'-'
        && bytes.iter().enumerate().all(|(index, byte)| {
            index == 13 || byte.is_ascii_digit() || (index > 13 && byte.is_ascii_lowercase())
        });
    if valid {
        Ok(())
    } else {
        Err("Invalid system settings baseline identifier.".into())
    }
}

fn reserve_snapshot_path(directory: &Path, timestamp: u64) -> Result<(String, PathBuf), String> {
    for _ in 0..100 {
        let sequence = FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let random =
            getrandom::u64().map_err(|_| "Cannot create a system settings baseline identifier.")?;
        let id = format!("{timestamp:013}-{sequence:08x}{:08x}", random as u32);
        let path = directory.join(format!("{id}.json"));
        if !path.exists() {
            return Ok((id, path));
        }
    }
    Err("Cannot reserve a unique system settings baseline file.".into())
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|_| "Cannot encode system settings data.")?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(
            "System settings data exceeds the bounded storage size; no further writes made.".into(),
        );
    }
    Ok(bytes)
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|_| "Cannot create private system settings storage.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot secure system settings storage.")?;
    }
    Ok(())
}

fn atomic_create(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("System settings storage path is invalid.")?;
    create_private_directory(parent)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "Cannot create immutable system settings data.")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Cannot durably save system settings data.")?;
    sync_directory(parent)
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("System settings storage path is invalid.")?;
    create_private_directory(parent)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("System settings storage path is invalid.")?;
    let temporary = parent.join(format!(
        ".{name}.{:016x}.tmp",
        getrandom::u64().map_err(|_| "Cannot prepare system settings data.")?
    ));
    atomic_create(&temporary, bytes)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Cannot commit system settings data: {error}"));
    }
    sync_directory(parent)
}

fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "Cannot durably commit system settings data.".into())
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cannot read system settings data.".into()),
    };
    let metadata = file
        .metadata()
        .map_err(|_| "Cannot inspect system settings data.")?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err("System settings data is invalid or too large.".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|_| "Cannot read system settings data.")?;
    Ok(Some(bytes))
}

// Inspect and mutate the same native object. There is no CoreAudio atomic compare-and-set;
// this closes our own re-resolution/fsync gap, not races inside the OS call itself.
#[cfg(any(target_os = "macos", test))]
fn guarded_write<T>(
    expected: &NativeState,
    inspect: impl FnOnce() -> Result<(T, NativeState), String>,
    setter: impl FnOnce(T) -> Result<(), String>,
) -> Result<(), String> {
    let (object, actual) = inspect()?;
    if actual.identity != expected.identity || !values_equal(&actual.values, &expected.values) {
        return Err(
            "Actual system identity or values changed immediately before the write.".into(),
        );
    }
    setter(object)
}

pub fn native_adapter() -> Arc<dyn NativeSettings> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(MacOsNativeSettings)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(UnsupportedNativeSettings)
    }
}

#[cfg(not(target_os = "macos"))]
struct UnsupportedNativeSettings;

#[cfg(not(target_os = "macos"))]
impl NativeSettings for UnsupportedNativeSettings {
    fn read(&self) -> Result<NativeState, String> {
        Err("System settings baselines are currently supported on macOS only.".into())
    }

    fn write(
        &self,
        _expected: &NativeState,
        _setting: Setting,
        _value: SettingValue,
    ) -> Result<(), String> {
        Err("System settings rollback is currently supported on macOS only.".into())
    }
}

#[cfg(target_os = "macos")]
struct MacOsNativeSettings;

#[cfg(target_os = "macos")]
mod core_audio {
    pub(super) mod formats;
    use super::*;
    use core_foundation::{base::TCFType, string::CFString, string::CFStringRef};
    use std::{ffi::c_void, mem::size_of, ptr};

    type AudioObjectId = u32;
    type OsStatus = i32;
    type Boolean = u8;

    #[repr(C)]
    struct PropertyAddress {
        selector: u32,
        scope: u32,
        element: u32,
    }

    const fn fourcc(bytes: &[u8; 4]) -> u32 {
        ((bytes[0] as u32) << 24)
            | ((bytes[1] as u32) << 16)
            | ((bytes[2] as u32) << 8)
            | bytes[3] as u32
    }

    const SYSTEM_OBJECT: AudioObjectId = 1;
    const DEFAULT_OUTPUT: u32 = fourcc(b"dOut");
    const DEVICE_UID: u32 = fourcc(b"uid ");
    const VOLUME_SCALAR: u32 = fourcc(b"volm");
    const MUTE: u32 = fourcc(b"mute");
    const GLOBAL: u32 = fourcc(b"glob");
    const OUTPUT: u32 = fourcc(b"outp");
    const MAIN: u32 = 0;

    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        fn AudioObjectHasProperty(
            object: AudioObjectId,
            address: *const PropertyAddress,
        ) -> Boolean;
        fn AudioObjectIsPropertySettable(
            object: AudioObjectId,
            address: *const PropertyAddress,
            settable: *mut Boolean,
        ) -> OsStatus;
        fn AudioObjectGetPropertyData(
            object: AudioObjectId,
            address: *const PropertyAddress,
            qualifier_size: u32,
            qualifier_data: *const c_void,
            data_size: *mut u32,
            data: *mut c_void,
        ) -> OsStatus;
        fn AudioObjectSetPropertyData(
            object: AudioObjectId,
            address: *const PropertyAddress,
            qualifier_size: u32,
            qualifier_data: *const c_void,
            data_size: u32,
            data: *const c_void,
        ) -> OsStatus;
    }

    fn address(selector: u32, scope: u32) -> PropertyAddress {
        PropertyAddress {
            selector,
            scope,
            element: MAIN,
        }
    }

    fn status(result: OsStatus, action: &str) -> Result<(), String> {
        if result == 0 {
            Ok(())
        } else {
            Err(format!("CoreAudio could not {action} (OSStatus {result})."))
        }
    }

    fn default_device() -> Result<AudioObjectId, String> {
        let address = address(DEFAULT_OUTPUT, GLOBAL);
        let mut device = 0_u32;
        let mut size = size_of::<AudioObjectId>() as u32;
        // SAFETY: CoreAudio receives correctly sized pointers for a public UInt32 property.
        status(
            unsafe {
                AudioObjectGetPropertyData(
                    SYSTEM_OBJECT,
                    &address,
                    0,
                    ptr::null(),
                    &mut size,
                    (&mut device as *mut AudioObjectId).cast(),
                )
            },
            "read the default output device",
        )?;
        if device == 0 {
            Err("CoreAudio did not report a default output device.".into())
        } else {
            Ok(device)
        }
    }

    fn settable(device: AudioObjectId, address: &PropertyAddress) -> Result<bool, String> {
        // SAFETY: The address is initialized and CoreAudio writes one Boolean to `settable`.
        if unsafe { AudioObjectHasProperty(device, address) } == 0 {
            return Ok(false);
        }
        let mut value = 0_u8;
        // SAFETY: The public API writes a Boolean to a valid pointer.
        status(
            unsafe { AudioObjectIsPropertySettable(device, address, &mut value) },
            "inspect an output control",
        )?;
        Ok(value != 0)
    }

    fn read_value<T: Copy>(
        device: AudioObjectId,
        address: &PropertyAddress,
        action: &str,
        initial: T,
    ) -> Result<T, String> {
        let mut value = initial;
        let mut size = size_of::<T>() as u32;
        // SAFETY: `value` is valid for `size` bytes and matches the documented property type.
        status(
            unsafe {
                AudioObjectGetPropertyData(
                    device,
                    address,
                    0,
                    ptr::null(),
                    &mut size,
                    (&mut value as *mut T).cast(),
                )
            },
            action,
        )?;
        if size != size_of::<T>() as u32 {
            return Err(format!(
                "CoreAudio returned an invalid value while trying to {action}."
            ));
        }
        Ok(value)
    }

    fn write_value<T>(
        device: AudioObjectId,
        address: &PropertyAddress,
        action: &str,
        value: &T,
    ) -> Result<(), String> {
        // SAFETY: `value` is valid for the documented property size and lives through the call.
        status(
            unsafe {
                AudioObjectSetPropertyData(
                    device,
                    address,
                    0,
                    ptr::null(),
                    size_of::<T>() as u32,
                    (value as *const T).cast(),
                )
            },
            action,
        )
    }

    fn device_uid(device: AudioObjectId) -> Result<String, String> {
        let address = address(DEVICE_UID, GLOBAL);
        let reference: CFStringRef = read_value(
            device,
            &address,
            "read the output device identity",
            ptr::null(),
        )?;
        if reference.is_null() {
            return Err("CoreAudio returned an empty output device identity.".into());
        }
        // SAFETY: The CoreAudio declaration states that callers own and must release this CFString.
        let value = unsafe { CFString::wrap_under_create_rule(reference) }.to_string();
        if value.is_empty() {
            Err("CoreAudio returned an empty output device identity.".into())
        } else {
            Ok(value)
        }
    }

    fn host_identity() -> Result<String, String> {
        extern "C" {
            fn gethostuuid(uuid: *mut u8, timeout: *const libc::timespec) -> i32;
        }
        let mut uuid = [0_u8; 16];
        let timeout = libc::timespec {
            tv_sec: 5,
            tv_nsec: 0,
        };
        // SAFETY: `uuid` has the 16-byte uuid_t layout and timeout is a valid timespec.
        if unsafe { gethostuuid(uuid.as_mut_ptr(), &timeout) } != 0 {
            return Err("macOS could not provide a stable machine identity.".into());
        }
        Ok(uuid.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    pub(super) fn read() -> Result<NativeState, String> {
        match inspect_device() {
            Ok((_, state)) => Ok(state),
            Err(_) => {
                // Audio availability must not prevent independent user preference capture.
                // The sentinel changes compatibility if an output device later becomes available.
                let state = NativeState {
                    identity: NativeIdentity {
                        platform: "macos".into(),
                        os_version: sysinfo::System::os_version()
                            .ok_or("macOS version is unavailable.")?,
                        user: unsafe { libc::geteuid() }.to_string(),
                        machine: host_identity()?,
                        device: "audio-unavailable".into(),
                    },
                    values: SystemValues {
                        output_volume: None,
                        output_muted: None,
                        preferences: preferences::collect(),
                        hardware: collect_hardware(),
                    },
                };
                state.values.validate()?;
                Ok(state)
            }
        }
    }

    fn inspect_device() -> Result<(AudioObjectId, NativeState), String> {
        let device = default_device()?;
        let volume_address = address(VOLUME_SCALAR, OUTPUT);
        let mute_address = address(MUTE, OUTPUT);
        let output_volume = if settable(device, &volume_address)? {
            Some(read_value(
                device,
                &volume_address,
                "read output volume",
                0.0_f32,
            )?)
        } else {
            None
        };
        let output_muted = if settable(device, &mute_address)? {
            Some(read_value(device, &mute_address, "read output mute", 0_u32)? != 0)
        } else {
            None
        };
        let state = NativeState {
            identity: NativeIdentity {
                platform: "macos".into(),
                os_version: sysinfo::System::os_version().ok_or("macOS version is unavailable.")?,
                // SAFETY: geteuid is a read-only POSIX identity query with no preconditions.
                user: unsafe { libc::geteuid() }.to_string(),
                machine: host_identity()?,
                device: device_uid(device)?,
            },
            values: SystemValues {
                output_volume,
                output_muted,
                preferences: preferences::collect(),
                hardware: collect_hardware(),
            },
        };
        state.values.validate()?;
        if default_device()? != device || device_uid(device)? != state.identity.device {
            return Err("The default output device changed during inspection.".into());
        }
        Ok((device, state))
    }

    pub(super) fn write(
        expected: &NativeState,
        setting: Setting,
        value: SettingValue,
    ) -> Result<(), String> {
        // Called only after the applying journal is fsynced. Never resolve a new device
        // between inspecting its UID/values and passing that exact ID to the setter.
        guarded_write(expected, inspect_device, |device| match (setting, value) {
            (Setting::OutputVolume, SettingValue::Volume(value)) => {
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err("Output volume is outside the CoreAudio scalar range.".into());
                }
                let address = address(VOLUME_SCALAR, OUTPUT);
                if !settable(device, &address)? {
                    return Err("The default output device volume is no longer settable.".into());
                }
                write_value(device, &address, "set output volume", &value)
            }
            (Setting::OutputMuted, SettingValue::Muted(value)) => {
                let address = address(MUTE, OUTPUT);
                if !settable(device, &address)? {
                    return Err(
                        "The default output device mute control is no longer settable.".into(),
                    );
                }
                let value = u32::from(value);
                write_value(device, &address, "set output mute", &value)
            }
            _ => Err("The requested setting value has the wrong native type.".into()),
        })
    }
}

#[cfg(target_os = "macos")]
fn collect_hardware() -> BTreeMap<u16, HardwareValue> {
    let mut values = power::collect().unwrap_or_default();
    if let Ok(value) = time_machine::read() {
        values.insert(120, value);
    }
    if let Ok(value) = login_items::read() {
        values.insert(130, value);
    }
    if let Ok(value) = background_items::read() {
        values.insert(140, value);
    }
    if let Ok(value) = core_audio::formats::collect() {
        values.insert(0, value);
    }
    if let Ok(value) = displays::collect() {
        values.insert(1, value);
    }
    values
}

#[cfg(target_os = "macos")]
impl NativeSettings for MacOsNativeSettings {
    fn validate_target(&self, values: &SystemValues) -> Result<(), String> {
        values.validate()?;
        for target in values.hardware.values() {
            match target {
                HardwareValue::BackgroundItems { .. } => background_items::preflight(target)?,
                HardwareValue::LoginItems { .. } => login_items::preflight(target)?,
                HardwareValue::TimeMachine { .. } => time_machine::preflight()?,
                HardwareValue::Audio { .. } => core_audio::formats::preflight(target)?,
                HardwareValue::Displays { .. } => displays::preflight(target)?,
                HardwareValue::Power { .. } | HardwareValue::EnergyPolicy { .. } => (), // Shape/identity checked; authorization is tested only by the confirmed executor.
            }
        }
        Ok(())
    }
    fn coverage(&self, values: &SystemValues) -> Vec<Capability> {
        let mut items = capabilities::coverage(values);
        power::annotate(&mut items);
        time_machine::annotate(&mut items);
        login_items::annotate(&mut items);
        background_items::annotate(&mut items);
        spotlight::annotate(&mut items);
        displays::annotate(&mut items);
        core_audio::formats::annotate(&mut items);
        for (index, key) in Preference::ALL.into_iter().enumerate() {
            if !values.preferences.contains_key(&key) {
                if !preferences::supported_version() {
                    items[index + 2].detail = "Preference adapter is limited to macOS 13–15; this OS version is unvalidated. Audio remains independently supported.".into();
                } else if let Err(error) = preferences::macos::read(key) {
                    items[index + 2].status = if error.starts_with("Read-only:") {
                        CapabilityStatus::ReadOnly
                    } else {
                        CapabilityStatus::Unavailable
                    };
                    items[index + 2].detail = error;
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
            for item in &mut items {
                let effective = match item.id.as_str() {
                    "reduce_motion" => Some(workspace.accessibilityDisplayShouldReduceMotion()),
                    "reduce_transparency" => {
                        Some(workspace.accessibilityDisplayShouldReduceTransparency())
                    }
                    _ => None,
                };
                if let Some(value) = effective {
                    item.status = CapabilityStatus::ReadOnly;
                    item.detail = format!(
                        "Effective value reported by NSWorkspace: {value}. {}",
                        item.detail
                    );
                }
            }
        }
        items
    }
    fn read(&self) -> Result<NativeState, String> {
        core_audio::read()
    }

    fn write(
        &self,
        expected: &NativeState,
        setting: Setting,
        value: SettingValue,
    ) -> Result<(), String> {
        if let (Setting::Hardware(slot), SettingValue::Hardware(ref target)) =
            (setting, value.clone())
        {
            return guarded_write(
                expected,
                || self.read().map(|state| ((), state)),
                |_| {
                    let before = expected
                        .values
                        .hardware
                        .get(&slot)
                        .ok_or("Hardware target unavailable")?;
                    match target {
                        HardwareValue::BackgroundItems { .. } => {
                            background_items::write(before, target)
                        }
                        HardwareValue::LoginItems { .. } => login_items::write(before, target),
                        HardwareValue::TimeMachine { .. } => time_machine::write(before, target),
                        HardwareValue::Audio { .. } => core_audio::formats::write(before, target),
                        HardwareValue::Displays { .. } => displays::write(before, target),
                        HardwareValue::Power { .. } | HardwareValue::EnergyPolicy { .. } => {
                            power::write(slot, before, target)
                        }
                    }
                },
            );
        }
        if let (Setting::Preference(key), SettingValue::Preference(value)) =
            (setting, value.clone())
        {
            guarded_write(
                expected,
                || self.read().map(|state| ((), state)),
                |_| {
                    let before = expected
                        .values
                        .preferences
                        .get(&key)
                        .ok_or("Preference is unavailable; recheck access.")?;
                    preferences::macos::write(key, *before, value)
                },
            )
        } else {
            core_audio::write(expected, setting, value)
        }
    }
}

#[cfg(test)]
mod tests {
    mod hardware_tests;
    mod preference_tests;
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::{Arc, Mutex},
        time::Duration,
    };

    #[derive(Clone)]
    struct FakeNative {
        state: Arc<Mutex<NativeState>>,
    }

    impl NativeSettings for FakeNative {
        fn read(&self) -> Result<NativeState, String> {
            Ok(self.state.lock().unwrap().clone())
        }

        fn write(
            &self,
            _expected: &NativeState,
            _setting: Setting,
            _value: SettingValue,
        ) -> Result<(), String> {
            panic!("capture and history must never write native settings")
        }
    }

    fn directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "jitterbug-system-settings-{name}-{}",
            getrandom::u64().unwrap()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn state(device: &str) -> NativeState {
        NativeState {
            identity: NativeIdentity {
                platform: "macos".into(),
                os_version: "15.7.1".into(),
                user: "501".into(),
                machine: "machine-a".into(),
                device: device.into(),
            },
            values: SystemValues {
                output_volume: Some(0.42),
                output_muted: Some(false),
                preferences: BTreeMap::new(),
                hardware: BTreeMap::new(),
            },
        }
    }

    #[test]
    fn power_timer_is_an_automatic_journaled_restore_target() {
        let mut initial = state("speakers");
        let json = serde_json::json!({"output_volume": null, "output_muted": null,
            "hardware": {"100": {"kind":"power", "source":"AC Power", "key":"sleep", "minutes":12}}});
        initial.values = serde_json::from_value(json).expect("typed power capture");
        let directory = directory("automatic-power");
        let (controller, live, writes) = scripted(&directory, initial, None, None);
        let baseline = controller
            .capture_selected(Some(&["Power profiles".into()]))
            .unwrap();
        let changed = serde_json::json!({"output_volume": null, "output_muted": null,
            "hardware": {"100": {"kind":"power", "source":"AC Power", "key":"sleep", "minutes":0}}});
        live.lock().unwrap().values = serde_json::from_value(changed).unwrap();
        let preview = controller.preview(&baseline.id).unwrap();
        assert_eq!(preview.changes.len(), 1);
        assert_eq!(preview.changes[0].group, "Power profiles");
        assert!(preview.coverage.iter().any(|item| item.id == "hardware_100"
            && item.included
            && item.status == CapabilityStatus::NeedsPermission));
        let result = controller.confirm(&preview.token).unwrap();
        assert_eq!(result.status, RollbackStatus::Verified);
        assert_eq!(writes.lock().unwrap().len(), 1);
        assert_eq!(result.current, baseline.values);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn audio_configuration_is_captured_and_restored_as_a_coupled_target() {
        let json = serde_json::json!({"output_volume": null, "output_muted": null,
            "hardware": {"0": {"kind":"audio", "uid":"test-device", "rate":48000.0, "streams":[]}}});
        let mut initial = state("speakers");
        initial.values = serde_json::from_value(json).expect("typed audio format capture");
        let directory = directory("automatic-format");
        let (controller, live, writes) = scripted(&directory, initial.clone(), None, None);
        let baseline = controller
            .capture_selected(Some(&["Audio hardware".into()]))
            .unwrap();
        let mut changed = serde_json::to_value(&initial.values).unwrap();
        changed["hardware"]["0"]["rate"] = 44100.0.into();
        live.lock().unwrap().values = serde_json::from_value(changed).unwrap();
        let preview = controller.preview(&baseline.id).unwrap();
        assert!(preview.changes[0].verification.contains("interrupt"));
        assert_eq!(
            controller.confirm(&preview.token).unwrap().status,
            RollbackStatus::Verified
        );
        assert_eq!(writes.lock().unwrap().len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn display_modes_restore_and_reject_topology_rebinding() {
        let json = serde_json::json!({"output_volume":null,"output_muted":null,"hardware":{"1":{"kind":"displays","displays":[{"uuid":"test-display", "x":0.0,"y":0.0,"rotation":0.0,"main":true,"active":true,"mirror":null,"mode":{"id":1,"width":1920,"height":1080,"pixel_width":1920,"pixel_height":1080,"refresh_hz":60.0,"flags":0,"desktop_usable":true}}]}}});
        let mut initial = state("speakers");
        initial.values = serde_json::from_value(json).expect("typed display capture");
        let directory = directory("automatic-display");
        let (controller, live, _) = scripted(&directory, initial.clone(), None, None);
        let baseline = controller
            .capture_selected(Some(&["Displays".into()]))
            .unwrap();
        let mut changed = serde_json::to_value(&initial.values).unwrap();
        changed["hardware"]["1"]["displays"][0]["mode"]["id"] = 2.into();
        live.lock().unwrap().values = serde_json::from_value(changed.clone()).unwrap();
        let preview = controller.preview(&baseline.id).unwrap();
        assert!(preview.changes[0]
            .verification
            .contains("reboot persistence not verified"));
        assert_eq!(
            controller.confirm(&preview.token).unwrap().status,
            RollbackStatus::Verified
        );
        changed["hardware"]["1"]["displays"][0]["uuid"] = "other-display".into();
        live.lock().unwrap().values = serde_json::from_value(changed).unwrap();
        assert!(controller.preview(&baseline.id).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn oversized_journal_encoding_is_rejected_before_any_write() {
        assert!(encode(&"x".repeat(MAX_FILE_BYTES as usize)).is_err());
    }

    #[test]
    fn adapter_preflight_rejects_unsupported_saved_targets_before_confirmation() {
        struct RejectTarget(FakeNative);
        impl NativeSettings for RejectTarget {
            fn read(&self) -> Result<NativeState, String> {
                self.0.read()
            }
            fn validate_target(&self, _: &SystemValues) -> Result<(), String> {
                Err("Saved mode is no longer supported".into())
            }
            fn write(&self, _: &NativeState, _: Setting, _: SettingValue) -> Result<(), String> {
                panic!("unsupported target")
            }
        }
        let directory = directory("unsupported-preflight");
        let controller = Controller::new(
            directory.clone(),
            Arc::new(RejectTarget(FakeNative {
                state: Arc::new(Mutex::new(state("speakers"))),
            })),
        );
        let saved = controller.capture().unwrap();
        assert!(controller
            .preview(&saved.id)
            .unwrap_err()
            .contains("no longer supported"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn preference_values_preserve_absence_and_native_types() {
        let json = serde_json::json!({"output_volume":null,"output_muted":null,
            "preferences":{"dock_autohide":{"kind":"absent"},
            "key_repeat":{"kind":"integer","value":6},
            "mouse_speed":{"kind":"real","value":0.5}}});
        let values: SystemValues = serde_json::from_value(json.clone()).unwrap();
        values.validate().unwrap();
        assert_eq!(serde_json::to_value(values).unwrap(), json);
    }

    #[test]
    fn selected_categories_preserve_unselected_live_values() {
        let directory = directory("selected");
        let mut state = state("speakers");
        state
            .values
            .preferences
            .insert(Preference::KeyRepeat, PreferenceValue::Integer(6));
        state
            .values
            .preferences
            .insert(Preference::DockAutohide, PreferenceValue::Absent);
        let (controller, live, writes) = scripted(&directory, state, None, None);
        let baseline = controller
            .capture_selected(Some(&["Keyboard".into()]))
            .unwrap();
        assert!(baseline.values.as_ref().unwrap().output_volume.is_none());
        assert_eq!(baseline.values.as_ref().unwrap().preferences.len(), 1);
        live.lock()
            .unwrap()
            .values
            .preferences
            .insert(Preference::KeyRepeat, PreferenceValue::Integer(2));
        live.lock().unwrap().values.output_volume = Some(0.9);
        let preview = controller.preview(&baseline.id).unwrap();
        assert_eq!(
            controller.confirm(&preview.token).unwrap().status,
            RollbackStatus::Verified
        );
        assert_eq!(
            *writes.lock().unwrap(),
            vec![Setting::Preference(Preference::KeyRepeat)]
        );
        assert_eq!(live.lock().unwrap().values.output_volume, Some(0.9));
        assert!(controller.capture_selected(Some(&[])).is_err());
        assert!(controller
            .capture_selected(Some(&["Security".into()]))
            .is_err());
    }

    #[test]
    fn preference_preview_distinguishes_saved_verification_from_audio() {
        let mut before = state("speakers").values;
        before
            .preferences
            .insert(Preference::KeyRepeat, PreferenceValue::Integer(6));
        let mut target = before.clone();
        target
            .preferences
            .insert(Preference::KeyRepeat, PreferenceValue::Absent);
        let change = serde_json::to_value(&changes(&before, &target)[0]).unwrap();
        assert_eq!(change["group"], "Keyboard");
        assert_eq!(
            change["verification"],
            "Saved preference only; live effect not verified"
        );
    }

    #[test]
    fn capture_reports_every_available_and_excluded_setting() {
        let directory = directory("coverage");
        let (controller, _, _) = scripted(&directory, state("speakers"), None, None);
        let baseline = serde_json::to_value(controller.capture().unwrap()).unwrap();
        assert!(
            baseline.get("coverage").is_some(),
            "capture must explain omissions individually"
        );
    }

    #[test]
    fn restoring_absent_preferences_removes_only_the_saved_override() {
        let directory = directory("preference-absence");
        let mut initial = state("speakers");
        initial
            .values
            .preferences
            .insert(Preference::DockAutohide, PreferenceValue::Absent);
        let (controller, live, _) = scripted(&directory, initial.clone(), None, None);
        let baseline = controller.capture().unwrap();
        live.lock()
            .unwrap()
            .values
            .preferences
            .insert(Preference::DockAutohide, PreferenceValue::Boolean(true));
        let preview = controller.preview(&baseline.id).unwrap();
        assert_eq!(preview.changes.len(), 1);
        let result = controller.confirm(&preview.token).unwrap();
        assert_eq!(result.status, RollbackStatus::Verified);
        assert_eq!(live.lock().unwrap().values, initial.values);
    }

    #[test]
    fn last_moment_guard_rejects_value_and_identity_drift() {
        let expected = state("speakers");
        for identity_drift in [false, true] {
            let mut actual = expected.clone();
            if identity_drift {
                actual.identity.device = "headphones".into();
            } else {
                actual.values.output_volume = Some(0.91);
            }
            let result = guarded_write(
                &expected,
                || Ok((17_u32, actual)),
                |_| panic!("drift must never reach the setter"),
            );
            assert!(result.is_err());
        }
    }

    #[test]
    fn last_moment_guard_passes_the_exact_inspected_device_to_setter() {
        let expected = state("speakers");
        let mut written = None;
        guarded_write(
            &expected,
            || Ok((17_u32, expected.clone())),
            |device| {
                written = Some(device);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(written, Some(17));
    }

    #[test]
    fn repeated_capture_preserves_all_snapshots_and_legacy_provider_history() {
        let directory = directory("capture");
        let legacy = directory.join("settings-default-history/snapshots/legacy.json");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, b"legacy-provider-history").unwrap();
        let native = FakeNative {
            state: Arc::new(Mutex::new(state("speakers"))),
        };
        let controller = Controller::new(directory.clone(), Arc::new(native));

        let first = controller.capture().unwrap();
        let second = controller.capture().unwrap();

        assert_ne!(first.id, second.id);
        assert_eq!(fs::read(&legacy).unwrap(), b"legacy-provider-history");
        let snapshots = fs::read_dir(directory.join("system-settings-defaults-v1/snapshots"))
            .unwrap()
            .count();
        assert_eq!(snapshots, 2);
        let page = controller.history(None, 1).unwrap();
        assert_eq!(page.entries.len(), 1);
        assert!(page.entries[0].current);
        assert!(page.entries[0].compatible);
        let older = controller.history(page.next_cursor.as_deref(), 1).unwrap();
        assert_eq!(older.entries.len(), 1);
        assert_eq!(older.entries[0].id, first.id);
        assert!(older.next_cursor.is_none());
    }

    #[test]
    fn history_marks_a_different_device_context_incompatible() {
        let directory = directory("compatibility");
        let state = Arc::new(Mutex::new(state("speakers")));
        let controller = Controller::new(
            directory,
            Arc::new(FakeNative {
                state: state.clone(),
            }),
        );
        controller.capture().unwrap();
        state.lock().unwrap().identity.device = "headphones".into();

        let page = controller.history(None, 20).unwrap();

        assert!(!page.entries[0].compatible);
    }

    #[test]
    fn pagination_retains_every_immutable_snapshot() {
        let directory = directory("pagination-retention");
        let native = FakeNative {
            state: Arc::new(Mutex::new(state("speakers"))),
        };
        let controller = Controller::new(directory.clone(), Arc::new(native));
        for _ in 0..55 {
            controller.capture().unwrap();
        }

        let mut cursor = None;
        let mut count = 0;
        loop {
            let page = controller.history(cursor.as_deref(), 20).unwrap();
            count += page.entries.len();
            let Some(next) = page.next_cursor else { break };
            cursor = Some(next);
        }

        assert_eq!(count, 55);
        assert_eq!(
            fs::read_dir(directory.join("system-settings-defaults-v1/snapshots"))
                .unwrap()
                .count(),
            55
        );
    }

    #[derive(Clone)]
    struct ScriptedNative {
        state: Arc<Mutex<NativeState>>,
        writes: Arc<Mutex<Vec<Setting>>>,
        fail_on: Option<Setting>,
        ignore_on: Option<Setting>,
        journal_root: Option<PathBuf>,
    }
    type ScriptedFixture = (
        Controller,
        Arc<Mutex<NativeState>>,
        Arc<Mutex<Vec<Setting>>>,
    );

    impl NativeSettings for ScriptedNative {
        fn read(&self) -> Result<NativeState, String> {
            Ok(self.state.lock().unwrap().clone())
        }

        fn write(
            &self,
            _expected: &NativeState,
            setting: Setting,
            value: SettingValue,
        ) -> Result<(), String> {
            if let Some(root) = &self.journal_root {
                assert!(fs::read_dir(root).unwrap().next().is_some());
            }
            self.writes.lock().unwrap().push(setting);
            if self.fail_on == Some(setting) {
                return Err("injected native write failure".into());
            }
            if self.ignore_on == Some(setting) {
                return Ok(());
            }
            let mut state = self.state.lock().unwrap();
            match (setting, value) {
                (Setting::Hardware(slot), SettingValue::Hardware(value)) => {
                    state.values.hardware.insert(slot, value);
                }
                (Setting::OutputVolume, SettingValue::Volume(value)) => {
                    state.values.output_volume = Some(value);
                }
                (Setting::OutputMuted, SettingValue::Muted(value)) => {
                    state.values.output_muted = Some(value);
                }
                (Setting::Preference(key), SettingValue::Preference(value)) => {
                    state.values.preferences.insert(key, value);
                }
                _ => panic!("setting/value mismatch"),
            }
            Ok(())
        }
    }

    fn scripted(
        directory: &Path,
        current: NativeState,
        fail_on: Option<Setting>,
        ignore_on: Option<Setting>,
    ) -> ScriptedFixture {
        let state = Arc::new(Mutex::new(current));
        let writes = Arc::new(Mutex::new(Vec::new()));
        let native = ScriptedNative {
            state: state.clone(),
            writes: writes.clone(),
            fail_on,
            ignore_on,
            journal_root: Some(directory.join("system-settings-defaults-v1/rollback-journals")),
        };
        (
            Controller::new(directory.to_owned(), Arc::new(native)),
            state,
            writes,
        )
    }

    struct LastMomentDrift {
        inner: ScriptedNative,
    }

    impl NativeSettings for LastMomentDrift {
        fn read(&self) -> Result<NativeState, String> {
            self.inner.read()
        }

        fn write(
            &self,
            expected: &NativeState,
            setting: Setting,
            value: SettingValue,
        ) -> Result<(), String> {
            let root = self.inner.journal_root.as_ref().unwrap();
            let path = fs::read_dir(root).unwrap().next().unwrap().unwrap().path();
            let journal: JournalFile = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            assert_eq!(journal.status, JournalStatus::Applying);
            // Simulate a volume key press after the durable applying boundary.
            self.inner.state.lock().unwrap().values.output_volume = Some(0.73);
            guarded_write(
                expected,
                || self.inner.read().map(|state| ((), state)),
                |_| self.inner.write(expected, setting, value),
            )
        }
    }

    #[test]
    fn drift_after_applying_journal_never_reaches_the_setter() {
        let directory = directory("last-moment-drift");
        let live = Arc::new(Mutex::new(state("speakers")));
        let writes = Arc::new(Mutex::new(Vec::new()));
        let controller = Controller::new(
            directory.clone(),
            Arc::new(LastMomentDrift {
                inner: ScriptedNative {
                    state: live.clone(),
                    writes: writes.clone(),
                    fail_on: None,
                    ignore_on: None,
                    journal_root: Some(directory.join(DIRECTORY).join(JOURNALS)),
                },
            }),
        );
        let baseline = controller.capture().unwrap();
        live.lock().unwrap().values.output_volume = Some(0.9);
        let preview = controller.preview(&baseline.id).unwrap();
        let result = controller.confirm(&preview.token).unwrap();
        assert_eq!(result.status, RollbackStatus::PartialFailure);
        assert_eq!(result.outcomes[0].status, OutcomeStatus::WriteFailed);
        assert!(writes.lock().unwrap().is_empty());
        assert_eq!(result.current.unwrap().output_volume, Some(0.73));
    }

    #[test]
    fn preview_is_one_use_and_rejects_external_drift_before_any_write() {
        let directory = directory("preview-drift");
        let (controller, live, writes) = scripted(&directory, state("speakers"), None, None);
        let baseline = controller.capture().unwrap();
        live.lock().unwrap().values.output_volume = Some(0.8);
        let preview = controller.preview(&baseline.id).unwrap();
        assert_eq!(preview.changes.len(), 1);
        assert_eq!(preview.changes[0].field, "Output volume");
        live.lock().unwrap().values.output_volume = Some(0.7);

        let error = controller.confirm(&preview.token).unwrap_err();

        assert!(error.contains("stale"));
        assert!(writes.lock().unwrap().is_empty());
        assert!(controller
            .confirm(&preview.token)
            .unwrap_err()
            .contains("already used"));
    }

    #[test]
    fn expired_preview_is_consumed_without_writes() {
        let directory = directory("expired");
        let (mut controller, live, writes) = scripted(&directory, state("speakers"), None, None);
        controller.preview_ttl = Duration::ZERO;
        let baseline = controller.capture().unwrap();
        live.lock().unwrap().values.output_muted = Some(true);
        let preview = controller.preview(&baseline.id).unwrap();

        assert!(controller
            .confirm(&preview.token)
            .unwrap_err()
            .contains("expired"));
        assert!(writes.lock().unwrap().is_empty());
    }

    #[test]
    fn matching_baseline_is_a_verified_no_op_without_a_journal() {
        let directory = directory("noop");
        let (controller, _live, writes) = scripted(&directory, state("speakers"), None, None);
        let baseline = controller.capture().unwrap();
        let preview = controller.preview(&baseline.id).unwrap();
        assert!(preview.changes.is_empty());

        let result = controller.confirm(&preview.token).unwrap();

        assert_eq!(result.status, RollbackStatus::NoChanges);
        assert!(result.outcomes.is_empty());
        assert!(writes.lock().unwrap().is_empty());
        assert!(!directory
            .join("system-settings-defaults-v1/rollback-journals")
            .exists());
    }

    #[test]
    fn intent_is_durable_before_writes_and_each_setting_is_read_back() {
        let directory = directory("verified");
        let (controller, live, writes) = scripted(&directory, state("speakers"), None, None);
        let baseline = controller.capture().unwrap();
        {
            let mut live = live.lock().unwrap();
            live.values.output_volume = Some(0.9);
            live.values.output_muted = Some(true);
        }
        let preview = controller.preview(&baseline.id).unwrap();

        let result = controller.confirm(&preview.token).unwrap();

        assert_eq!(result.status, RollbackStatus::Verified);
        assert_eq!(result.outcomes.len(), 2);
        assert!(result
            .outcomes
            .iter()
            .all(|outcome| outcome.status == OutcomeStatus::Verified));
        assert_eq!(
            *writes.lock().unwrap(),
            vec![Setting::OutputVolume, Setting::OutputMuted]
        );
        assert_eq!(live.lock().unwrap().values, state("speakers").values);
    }

    #[test]
    fn write_failure_stops_and_returns_a_durable_partial_result() {
        let directory = directory("partial-write");
        let (controller, live, writes) = scripted(
            &directory,
            state("speakers"),
            Some(Setting::OutputMuted),
            None,
        );
        let baseline = controller.capture().unwrap();
        {
            let mut live = live.lock().unwrap();
            live.values.output_volume = Some(0.9);
            live.values.output_muted = Some(true);
        }
        let preview = controller.preview(&baseline.id).unwrap();

        let result = controller.confirm(&preview.token).unwrap();

        assert_eq!(result.status, RollbackStatus::PartialFailure);
        assert_eq!(result.outcomes[0].status, OutcomeStatus::Verified);
        assert_eq!(result.outcomes[1].status, OutcomeStatus::WriteFailed);
        assert_eq!(writes.lock().unwrap().len(), 2);
        let page = controller.history(None, 20).unwrap();
        assert_eq!(page.interrupted_operations, 0);
    }

    #[test]
    fn readback_mismatch_stops_after_the_first_write() {
        let directory = directory("readback-mismatch");
        let (controller, live, writes) = scripted(
            &directory,
            state("speakers"),
            None,
            Some(Setting::OutputVolume),
        );
        let baseline = controller.capture().unwrap();
        {
            let mut live = live.lock().unwrap();
            live.values.output_volume = Some(0.9);
            live.values.output_muted = Some(true);
        }
        let preview = controller.preview(&baseline.id).unwrap();

        let result = controller.confirm(&preview.token).unwrap();

        assert_eq!(result.status, RollbackStatus::PartialFailure);
        assert_eq!(result.outcomes[0].status, OutcomeStatus::ReadbackMismatch);
        assert_eq!(*writes.lock().unwrap(), vec![Setting::OutputVolume]);
    }

    #[derive(Clone)]
    struct DriftBeforeSecondWrite {
        state: Arc<Mutex<NativeState>>,
        reads: Arc<AtomicU64>,
        writes: Arc<Mutex<Vec<Setting>>>,
    }

    impl NativeSettings for DriftBeforeSecondWrite {
        fn read(&self) -> Result<NativeState, String> {
            let read = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
            if read == 6 {
                self.state.lock().unwrap().values.output_muted = Some(false);
            }
            Ok(self.state.lock().unwrap().clone())
        }

        fn write(
            &self,
            _expected: &NativeState,
            setting: Setting,
            value: SettingValue,
        ) -> Result<(), String> {
            self.writes.lock().unwrap().push(setting);
            set_value(&mut self.state.lock().unwrap().values, setting, value);
            Ok(())
        }
    }

    #[test]
    fn external_drift_is_rechecked_before_each_individual_write() {
        let directory = directory("between-write-drift");
        let state = Arc::new(Mutex::new(state("speakers")));
        let writes = Arc::new(Mutex::new(Vec::new()));
        let controller = Controller::new(
            directory,
            Arc::new(DriftBeforeSecondWrite {
                state: state.clone(),
                reads: Arc::new(AtomicU64::new(0)),
                writes: writes.clone(),
            }),
        );
        let baseline = controller.capture().unwrap();
        {
            let mut state = state.lock().unwrap();
            state.values.output_volume = Some(0.9);
            state.values.output_muted = Some(true);
        }
        let preview = controller.preview(&baseline.id).unwrap();

        let result = controller.confirm(&preview.token).unwrap();

        assert_eq!(result.status, RollbackStatus::PartialFailure);
        assert_eq!(result.outcomes[0].status, OutcomeStatus::Verified);
        assert_eq!(result.outcomes[1].status, OutcomeStatus::ExternalDrift);
        assert_eq!(*writes.lock().unwrap(), vec![Setting::OutputVolume]);
    }

    #[derive(Clone)]
    struct ReadFailureAfterWrite {
        state: Arc<Mutex<NativeState>>,
        reads: Arc<AtomicU64>,
        writes: Arc<AtomicU64>,
    }

    impl NativeSettings for ReadFailureAfterWrite {
        fn read(&self) -> Result<NativeState, String> {
            let read = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
            if read == 5 {
                return Err("injected readback failure".into());
            }
            Ok(self.state.lock().unwrap().clone())
        }

        fn write(
            &self,
            _expected: &NativeState,
            setting: Setting,
            value: SettingValue,
        ) -> Result<(), String> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            set_value(&mut self.state.lock().unwrap().values, setting, value);
            Ok(())
        }
    }

    #[test]
    fn readback_error_after_a_write_is_durably_a_partial_failure() {
        let directory = directory("readback-error");
        let state = Arc::new(Mutex::new(state("speakers")));
        let writes = Arc::new(AtomicU64::new(0));
        let controller = Controller::new(
            directory,
            Arc::new(ReadFailureAfterWrite {
                state: state.clone(),
                reads: Arc::new(AtomicU64::new(0)),
                writes: writes.clone(),
            }),
        );
        let baseline = controller.capture().unwrap();
        state.lock().unwrap().values.output_volume = Some(0.9);
        let preview = controller.preview(&baseline.id).unwrap();

        let result = controller.confirm(&preview.token).unwrap();

        assert_eq!(result.status, RollbackStatus::PartialFailure);
        assert_eq!(result.outcomes[0].status, OutcomeStatus::ReadbackMismatch);
        assert!(result.outcomes[0]
            .error
            .as_deref()
            .unwrap()
            .contains("injected readback failure"));
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        assert!(
            result.current.is_none(),
            "failed readback must not fabricate actual values"
        );
        assert_eq!(
            controller.history(None, 20).unwrap().interrupted_operations,
            0
        );
    }

    #[test]
    fn incomplete_intent_journal_is_reported_but_never_replayed() {
        let directory = directory("interrupted-journal");
        let native = FakeNative {
            state: Arc::new(Mutex::new(state("speakers"))),
        };
        let controller = Controller::new(directory.clone(), Arc::new(native));
        let snapshot = controller.capture().unwrap();
        let target = controller.read_snapshot(&snapshot.id).unwrap();
        let journal = JournalFile {
            schema_version: SCHEMA_VERSION,
            id: "0000000000003-0000000000000003".into(),
            target_id: snapshot.id,
            started_at_ms: 3,
            status: JournalStatus::Intent,
            expected_context: target.context,
            expected_values: target.values.clone(),
            target_values: target.values,
            outcomes: Vec::new(),
        };
        let path = directory
            .join("system-settings-defaults-v1/rollback-journals")
            .join(format!("{}.json", journal.id));
        atomic_create(&path, &encode(&journal).unwrap()).unwrap();

        let page = controller.history(None, 20).unwrap();

        assert_eq!(page.interrupted_operations, 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "manual read-only CoreAudio/identity discovery; never writes native settings"]
    fn real_macos_discovery_is_read_only() {
        match native_adapter().read() {
            Ok(discovered) => {
                println!("Read-only native discovery: supported controls found (identities and values omitted).");
                assert_eq!(discovered.identity.platform, "macos");
                assert_eq!(
                    discovered.values.preferences.len(),
                    Preference::ALL.len(),
                    "native discovery must capture every allowlisted preference"
                );
                assert_eq!(
                    usize::from(discovered.values.output_volume.is_some())
                        + usize::from(discovered.values.output_muted.is_some()),
                    2,
                    "native discovery must capture both audio controls"
                );
                discovered.values.validate().unwrap();
                let inventory = native_adapter().coverage(&discovered.values);
                assert!(
                    discovered.values.hardware.contains_key(&0),
                    "settable audio rate/formats must be captured on probe hardware"
                );
                assert!(
                    discovered.values.hardware.contains_key(&1),
                    "enumerated display modes must be captured on probe hardware"
                );
                assert!(
                    discovered
                        .values
                        .hardware
                        .values()
                        .any(|v| matches!(v, HardwareValue::Power { .. })),
                    "reported power timers must be captured"
                );
                for (slot, value) in &discovered.values.hardware {
                    let item = inventory
                        .iter()
                        .find(|item| item.id == format!("hardware_{slot}"))
                        .unwrap();
                    assert!(item.included);
                    assert_eq!(
                        item.status,
                        if matches!(
                            value,
                            HardwareValue::Power { .. }
                                | HardwareValue::EnergyPolicy { .. }
                                | HardwareValue::TimeMachine { .. }
                        ) {
                            CapabilityStatus::NeedsPermission
                        } else {
                            CapabilityStatus::Supported
                        }
                    );
                }
                for item in &inventory {
                    println!(
                        "{}: {:?}; included={}",
                        item.label, item.status, item.included
                    );
                }
                let directory = directory("native-read-only-probe");
                let controller = Controller::new(directory.clone(), native_adapter());
                let saved = controller.capture().unwrap();
                assert_eq!(saved.coverage.len(), inventory.len());
                assert!(controller.preview(&saved.id).unwrap().changes.is_empty());
                fs::remove_dir_all(directory).unwrap();
            }
            Err(error) => {
                println!("Read-only native discovery failed closed: {error}");
                assert!(
                    error.contains("default output device")
                        || error.contains("supported settable controls")
                );
            }
        }
    }
}
