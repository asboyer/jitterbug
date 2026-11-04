//! Saved, opt-in optimization sessions. Persisted data is bounded and never leaves app data.
use crate::{
    applications, storage,
    types::{now, Snapshot},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const FILE_NAME: &str = "optimized-sessions.json";
pub const MAX_SESSIONS: usize = 20;
const MAX_EVENTS: usize = 40;
const MAX_FINDINGS: usize = 24;
const MAX_RUNS: usize = 12;
const MAX_FINGERPRINT_ITEMS: usize = 32;
const MAX_FILE_BYTES: u64 = 1_048_576;
const REEVALUATION_COOLDOWN_SECONDS: u64 = 15 * 60;
const SUSTAINED_DRIFT_WINDOWS: u8 = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Saved,
    Monitoring,
    Paused,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Baseline,
    MeasuringChange,
    Watching,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Setting {
    StageManager,
    KeepAwake,
    DockAutohide,
    DockMagnification,
    DockAnimate,
    DockMinimizeToApplication,
    DockShowRecents,
    PressAndHold,
    NaturalScroll,
    AutomaticSpelling,
    AutomaticCapitalization,
    AutomaticPeriod,
    AutomaticDash,
    AutomaticQuote,
    DsStoreNetwork,
    DsStoreUsb,
    MouseLinear,
    IcloudOptimizeStorage,
}

impl Setting {
    pub const ALL: [Self; 18] = [
        Self::KeepAwake,
        Self::StageManager,
        Self::DockAutohide,
        Self::DockMagnification,
        Self::DockAnimate,
        Self::DockMinimizeToApplication,
        Self::DockShowRecents,
        Self::PressAndHold,
        Self::NaturalScroll,
        Self::AutomaticSpelling,
        Self::AutomaticCapitalization,
        Self::AutomaticPeriod,
        Self::AutomaticDash,
        Self::AutomaticQuote,
        Self::DsStoreNetwork,
        Self::DsStoreUsb,
        Self::MouseLinear,
        Self::IcloudOptimizeStorage,
    ];

    pub(crate) fn preference(self) -> Option<crate::system_settings::Preference> {
        use crate::system_settings::Preference::*;
        Some(match self {
            Self::KeepAwake => return None,
            Self::StageManager => StageManager,
            Self::DockAutohide => DockAutohide,
            Self::DockMagnification => DockMagnification,
            Self::DockAnimate => DockAnimate,
            Self::DockMinimizeToApplication => DockMinimizeToApplication,
            Self::DockShowRecents => DockShowRecents,
            Self::PressAndHold => PressAndHold,
            Self::NaturalScroll => NaturalScroll,
            Self::AutomaticSpelling => AutomaticSpelling,
            Self::AutomaticCapitalization => AutomaticCapitalization,
            Self::AutomaticPeriod => AutomaticPeriod,
            Self::AutomaticDash => AutomaticDash,
            Self::AutomaticQuote => AutomaticQuote,
            Self::DsStoreNetwork => DsStoreNetwork,
            Self::DsStoreUsb => DsStoreUsb,
            Self::MouseLinear => MouseLinear,
            Self::IcloudOptimizeStorage => IcloudOptimizeStorage,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::KeepAwake => "Keep this PC awake",
            other => other
                .preference()
                .map(|key| key.descriptor().2)
                .unwrap_or("Setting"),
        }
    }

    pub fn group(self) -> crate::session_permissions::GroupId {
        use crate::session_permissions::GroupId::*;
        match self {
            Self::KeepAwake => PowerAndSleep,
            Self::StageManager
            | Self::DockAutohide
            | Self::DockMagnification
            | Self::DockAnimate
            | Self::DockMinimizeToApplication
            | Self::DockShowRecents => Appearance,
            Self::PressAndHold
            | Self::AutomaticSpelling
            | Self::AutomaticCapitalization
            | Self::AutomaticPeriod
            | Self::AutomaticDash
            | Self::AutomaticQuote => Focus,
            Self::NaturalScroll | Self::MouseLinear => Displays,
            Self::DsStoreNetwork | Self::DsStoreUsb | Self::IcloudOptimizeStorage => BackgroundWork,
        }
    }

    pub fn experiment_target(self, original: bool) -> Option<bool> {
        match self {
            Self::KeepAwake | Self::DockAutohide => (!original).then_some(true),
            Self::StageManager => Some(!original),
            Self::DockMagnification
            | Self::DockAnimate
            | Self::DockShowRecents
            | Self::PressAndHold
            | Self::AutomaticSpelling
            | Self::AutomaticCapitalization
            | Self::AutomaticPeriod
            | Self::AutomaticDash
            | Self::AutomaticQuote => original.then_some(false),
            Self::NaturalScroll
            | Self::MouseLinear
            | Self::IcloudOptimizeStorage
            | Self::DsStoreNetwork
            | Self::DsStoreUsb
            | Self::DockMinimizeToApplication => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    #[default]
    MonitorOnly,
    KeepAwake,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedProfile {
    pub capability: Capability,
    pub rationale: String,
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingState {
    pub setting: Setting,
    pub value: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub os: String,
    pub logical_cpus: usize,
    pub workloads: Vec<String>,
    #[serde(default)]
    pub applications: Vec<ApplicationFingerprint>,
    #[serde(default)]
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationFingerprint {
    pub workload_id: String,
    pub identifier: Option<String>,
    pub version: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Window {
    pub started_at: u64,
    pub ended_at: u64,
    pub samples: usize,
    pub average_cpu_percent: f32,
    pub average_available_memory_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    GoalMet,
    Improved,
    Regressed,
    Inconclusive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub fingerprint_id: String,
    pub setting: Setting,
    pub before_value: bool,
    pub after_value: bool,
    pub before: Window,
    pub after: Window,
    pub outcome: Outcome,
    pub readback_verified: bool,
    pub rolled_back: bool,
    pub explanation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub fingerprint_id: String,
    pub started_at: u64,
    pub completed_at: u64,
    pub before: Window,
    pub after: Window,
    pub outcome: Outcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionKind {
    ApplyExperiment,
    Restore,
    Reapply,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingTransaction {
    pub kind: TransactionKind,
    pub setting: Setting,
    pub expected_current: bool,
    pub target: bool,
    pub original: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Sample {
    at: u64,
    cpu_percent: f32,
    available_memory_bytes: u64,
}

pub trait SettingAdapter: Send + Sync {
    fn supported(&self, setting: Setting) -> Result<bool, String>;
    fn read(&self, setting: Setting) -> Result<bool, String>;
    /// Applies a fixed allowlisted setting and returns its immediate readback.
    fn apply(&self, setting: Setting, value: bool) -> Result<bool, String>;
}

#[derive(Default)]
struct NativeAdapter {
    #[cfg(target_os = "macos")]
    idle_sleep: crate::idle_sleep::IdleSleep,
}

impl SettingAdapter for NativeAdapter {
    fn supported(&self, setting: Setting) -> Result<bool, String> {
        match setting {
            Setting::KeepAwake => Ok(cfg!(target_os = "macos")),
            other => match other.preference() {
                Some(key) => Ok(crate::system_settings::session_boolean(key)?.is_some()),
                None => Ok(false),
            },
        }
    }

    fn read(&self, setting: Setting) -> Result<bool, String> {
        match setting {
            Setting::KeepAwake => {
                #[cfg(target_os = "macos")]
                {
                    self.idle_sleep.read()
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Err("Keep awake is supported only on macOS.".into())
                }
            }
            other => {
                let key = other
                    .preference()
                    .ok_or("Setting has no reversible preference adapter.")?;
                crate::system_settings::session_boolean(key)?
                    .ok_or_else(|| "No saved boolean override exists for this setting.".into())
            }
        }
    }

    fn apply(&self, setting: Setting, value: bool) -> Result<bool, String> {
        match setting {
            Setting::KeepAwake => {
                #[cfg(target_os = "macos")]
                {
                    self.idle_sleep.apply(value)
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = value;
                    Err("Keep awake is supported only on macOS.".into())
                }
            }
            other => {
                let key = other
                    .preference()
                    .ok_or("Setting has no reversible preference adapter.")?;
                let expected = self.read(other)?;
                crate::system_settings::session_apply_boolean(key, expected, value)
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub at: u64,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub goal: String,
    #[serde(default)]
    pub proposal: ProposedProfile,
    pub created_at: u64,
    pub updated_at: u64,
    pub opted_in: bool,
    pub status: Status,
    pub events: Vec<Event>,
    #[serde(default)]
    pub phase: Phase,
    #[serde(default)]
    pub baseline: Option<Window>,
    #[serde(default)]
    pub latest: Option<Window>,
    #[serde(default)]
    pub original_configuration: Vec<SettingState>,
    #[serde(default)]
    pub fingerprint: Option<Fingerprint>,
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub stale_reason: Option<String>,
    #[serde(default)]
    pub setting_support: String,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub best_configuration: Vec<SettingState>,
    #[serde(default)]
    pub applied_configuration: Vec<SettingState>,
    #[serde(default)]
    pub pending_transaction: Option<PendingTransaction>,
    #[serde(default)]
    pub runs: Vec<RunRecord>,
    #[serde(default)]
    pub last_run_at: u64,
    #[serde(default)]
    drift_windows: u8,
    #[serde(default)]
    samples: Vec<Sample>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    sessions: Vec<Session>,
    sequence: u64,
}

pub struct Store {
    directory: PathBuf,
    saved: Saved,
    adapter: Arc<dyn SettingAdapter>,
    applications: ApplicationCache,
    os_version: String,
}

#[derive(Clone)]
struct CachedApplication {
    path: PathBuf,
    modified: Option<std::time::SystemTime>,
    expires_at: u64,
    identifier: Option<String>,
    version: Option<String>,
}

#[derive(Default)]
struct ApplicationCache(VecDeque<CachedApplication>);

impl ApplicationCache {
    fn identity(&mut self, path: &Path) -> (Option<String>, Option<String>) {
        let bundle = crate::attribution::application_bundle(path)
            .unwrap_or(path)
            .to_path_buf();
        let modified = std::fs::metadata(&bundle)
            .and_then(|metadata| metadata.modified())
            .ok();
        let at = now();
        self.0.retain(|entry| entry.expires_at > at);
        if let Some(index) = self
            .0
            .iter()
            .position(|entry| entry.path == bundle && entry.modified == modified)
        {
            if let Some(entry) = self.0.remove(index) {
                let result = (entry.identifier.clone(), entry.version.clone());
                self.0.push_back(entry);
                return result;
            }
        }
        let application = applications::identify(path);
        let result = (application.identifier, application.version);
        if self.0.len() >= 512 {
            self.0.pop_front();
        }
        self.0.push_back(CachedApplication {
            path: bundle,
            modified,
            expires_at: at.saturating_add(300),
            identifier: result.0.clone(),
            version: result.1.clone(),
        });
        result
    }
}

fn validate(label: &str, value: &str, maximum: usize, multiline: bool) -> Result<String, String> {
    let normalized = if multiline {
        value.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        value.into()
    };
    let trimmed = normalized.trim();
    let invalid_control = trimmed
        .chars()
        .any(|character| character.is_control() && !(multiline && character == '\n'));
    if trimmed.is_empty() || trimmed.len() > maximum || invalid_control {
        return Err(format!(
            "Enter a valid session {label} (1–{maximum} characters)."
        ));
    }
    Ok(trimmed.into())
}

impl Store {
    pub fn load(directory: PathBuf) -> Result<Self, String> {
        Self::load_inner(directory, Arc::new(NativeAdapter::default()))
    }

    #[cfg(test)]
    fn load_with_adapter(
        directory: PathBuf,
        adapter: Arc<dyn SettingAdapter>,
    ) -> Result<Self, String> {
        Self::load_inner(directory, adapter)
    }

    fn load_inner(directory: PathBuf, adapter: Arc<dyn SettingAdapter>) -> Result<Self, String> {
        let path = directory.join(FILE_NAME);
        if path
            .metadata()
            .is_ok_and(|metadata| metadata.len() > MAX_FILE_BYTES)
        {
            return Err("Optimized session data exceeds its private storage limit.".into());
        }
        let mut saved: Saved = storage::read(&path)?;
        let mut changed = false;
        while saved.sessions.len() > MAX_SESSIONS {
            let index = saved
                .sessions
                .iter()
                .rposition(Session::safe_to_evict)
                .ok_or("Optimized session storage is over capacity, but every excess session owns safety state; refusing to discard a restore journal.")?;
            saved.sessions.remove(index);
            changed = true;
        }
        let sequence = saved.sequence.max(saved.sessions.len() as u64);
        changed |= sequence != saved.sequence;
        saved.sequence = sequence;
        for session in &mut saved.sessions {
            session.validate_journal()?;
            let name = validate("name", &session.name, 80, false)?;
            let goal = validate("goal", &session.goal, 500, true)?;
            changed |= name != session.name || goal != session.goal;
            session.name = name;
            session.goal = goal;
            if session.events.len() > MAX_EVENTS {
                session.events.drain(0..session.events.len() - MAX_EVENTS);
                changed = true;
            }
            if session.findings.len() > MAX_FINDINGS
                || session.runs.len() > MAX_RUNS
                || session.samples.len() > 3
                || session.original_configuration.len() > 1
                || session.best_configuration.len() > 1
                || session.applied_configuration.len() > 1
                || session.fingerprint.as_ref().is_some_and(|fingerprint| {
                    fingerprint.workloads.len() > MAX_FINGERPRINT_ITEMS
                        || fingerprint.applications.len() > MAX_FINGERPRINT_ITEMS
                })
            {
                changed = true;
            }
            session.findings.truncate(MAX_FINDINGS);
            session.runs.truncate(MAX_RUNS);
            session.samples.truncate(3);
            session.original_configuration.truncate(1);
            session.best_configuration.truncate(1);
            session.applied_configuration.truncate(1);
            if let Some(fingerprint) = &mut session.fingerprint {
                fingerprint.workloads.truncate(MAX_FINGERPRINT_ITEMS);
                fingerprint.applications.truncate(MAX_FINGERPRINT_ITEMS);
            }
        }
        let mut store = Self {
            directory,
            saved,
            adapter,
            applications: ApplicationCache::default(),
            os_version: sysinfo::System::long_os_version()
                .unwrap_or_else(|| "unknown-version".into()),
        };
        let setting_support = store.setting_support();
        for session in &mut store.saved.sessions {
            if session.setting_support != setting_support {
                session.setting_support = setting_support.clone();
                changed = true;
            }
        }
        let ids: Vec<_> = store
            .saved
            .sessions
            .iter()
            .map(|session| session.id.clone())
            .collect();
        for id in ids {
            changed |= store.recover(&id)?;
        }
        if changed {
            store.persist()?;
        }
        Ok(store)
    }

    pub fn sessions(&self) -> &[Session] {
        &self.saved.sessions
    }

    #[cfg(test)]
    pub fn create(&mut self, name: &str, goal: &str) -> Result<Session, String> {
        self.create_proposed(
            name,
            goal,
            ProposedProfile {
                capability: Capability::KeepAwake,
                rationale: "Test-only allowlisted capability.".into(),
                provider: "test".into(),
                model: "test".into(),
            },
        )
    }

    pub fn create_proposed(
        &mut self,
        name: &str,
        goal: &str,
        proposal: ProposedProfile,
    ) -> Result<Session, String> {
        let name = validate("name", name, 80, false)?;
        let goal = validate("goal", goal, 500, true)?;
        let previous = self.saved.clone();
        if self.saved.sessions.len() >= MAX_SESSIONS {
            let index = self
                .saved
                .sessions
                .iter()
                .rposition(Session::safe_to_evict)
                .ok_or("The session limit is full and every session is monitoring or owns pending setting state. Pause and restore one before creating another.")?;
            self.saved.sessions.remove(index);
        }
        let at = now();
        self.saved.sequence = self.saved.sequence.saturating_add(1);
        let session = Session {
            id: format!("session-{at}-{}", self.saved.sequence),
            name,
            goal,
            proposal,
            created_at: at,
            updated_at: at,
            opted_in: false,
            status: Status::Saved,
            events: vec![Event {
                at,
                message: "Session saved. No monitoring or settings changes have started.".into(),
            }],
            phase: Phase::Baseline,
            baseline: None,
            latest: None,
            original_configuration: vec![],
            fingerprint: None,
            stale: false,
            stale_reason: None,
            setting_support: self.setting_support(),
            findings: vec![],
            best_configuration: vec![],
            applied_configuration: vec![],
            pending_transaction: None,
            runs: vec![],
            last_run_at: 0,
            drift_windows: 0,
            samples: vec![],
        };
        self.saved.sessions.insert(0, session.clone());
        if let Err(error) = self.persist() {
            self.saved = previous;
            return Err(error);
        }
        Ok(session)
    }

    pub fn update_proposal(
        &mut self,
        id: &str,
        proposal: ProposedProfile,
    ) -> Result<Session, String> {
        if self.session(id)?.status == Status::Monitoring || self.session(id)?.owns_state() {
            return Err(
                "Stop and restore this session before asking AI to propose a new profile.".into(),
            );
        }
        if proposal.rationale.trim().is_empty() || proposal.provider.trim().is_empty() {
            return Err("The AI session proposal was incomplete; nothing was saved.".into());
        }
        let previous = self.saved.clone();
        let session = self.session_mut(id)?;
        session.proposal = proposal;
        session.updated_at = now();
        push_event(
            session,
            "AI proposed an updated optimization profile. No setting has changed yet.",
        );
        let result = session.clone();
        if let Err(error) = self.persist() {
            self.saved = previous;
            return Err(error);
        }
        Ok(result)
    }

    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        let index = self
            .saved
            .sessions
            .iter()
            .position(|session| session.id == id)
            .ok_or("Optimization session not found.")?;
        if !self.saved.sessions[index].safe_to_evict() {
            return Err(
                "Stop this session and restore any owned setting before deleting it.".into(),
            );
        }
        let previous = self.saved.clone();
        self.saved.sessions.remove(index);
        if let Err(error) = self.persist() {
            self.saved = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn monitoring(&self) -> bool {
        self.saved
            .sessions
            .iter()
            .any(|session| session.status == Status::Monitoring)
    }

    pub fn owns_native_state(&self) -> bool {
        self.saved.sessions.iter().any(Session::owns_state)
    }

    pub fn monitoring_or_owned(&self) -> bool {
        self.monitoring() || self.owns_native_state()
    }

    pub fn start(
        &mut self,
        id: &str,
        explicit_opt_in: bool,
        snapshot: &Snapshot,
        automation_busy: bool,
    ) -> Result<Session, String> {
        self.start_restricted(id, explicit_opt_in, snapshot, automation_busy, None)
    }

    pub fn start_restricted(
        &mut self,
        id: &str,
        explicit_opt_in: bool,
        snapshot: &Snapshot,
        automation_busy: bool,
        allowed: Option<&[Setting]>,
    ) -> Result<Session, String> {
        if !explicit_opt_in {
            return Err("Explicit per-session optimization permission is required.".into());
        }
        if automation_busy {
            return Err("Wait for the existing automatic incident action to finish.".into());
        }
        if !valid_snapshot(snapshot) {
            return Err(
                "A fresh live telemetry sample is required; no setting was changed.".into(),
            );
        }
        if self
            .saved
            .sessions
            .iter()
            .any(|session| session.status == Status::Monitoring)
        {
            return Err("Only one optimized session can monitor at a time.".into());
        }
        if self.saved.sessions.iter().any(|session| {
            session.id != id && (session.status == Status::Monitoring || session.owns_state())
        }) {
            return Err("Another optimized session is monitoring or owns a native setting. Pause and restore it first.".into());
        }
        if self.session(id)?.owns_state() {
            self.restore_owned(id, "before starting a fresh baseline")?;
        }
        let mut originals = Vec::new();
        #[cfg(test)]
        let fallback = if self.session(id)?.proposal.provider == "test" {
            if self
                .session(id)?
                .goal
                .to_ascii_lowercase()
                .starts_with("keep awake:")
            {
                Some(Setting::KeepAwake)
            } else {
                Some(Setting::StageManager)
            }
        } else {
            match self.session(id)?.proposal.capability {
                Capability::MonitorOnly => None,
                Capability::KeepAwake => Some(Setting::KeepAwake),
            }
        };
        #[cfg(not(test))]
        let fallback = match self.session(id)?.proposal.capability {
            Capability::MonitorOnly => None,
            Capability::KeepAwake => Some(Setting::KeepAwake),
        };
        let permission_denied = allowed.is_some_and(|list| list.is_empty()) && fallback.is_some();
        let requested: Vec<Setting> = match allowed {
            Some(list) => list.to_vec(),
            None => fallback.into_iter().collect(),
        };
        for setting in requested {
            if self.adapter.supported(setting)? {
                originals.push(SettingState {
                    setting,
                    value: self.adapter.read(setting)?,
                });
            }
        }
        let setting_available = !originals.is_empty();
        let fingerprint = self.fingerprint(snapshot);
        let at = now();
        // Snapshot after any completed restoration; never undo its ownership bookkeeping.
        let previous = self.saved.clone();
        let session = self.session_mut(id)?;
        if session.stale
            || session
                .fingerprint
                .as_ref()
                .is_some_and(|previous| previous != &fingerprint)
        {
            session.stale = true;
            session.best_configuration.clear();
            session.stale_reason =
                Some("The OS, workload set, or installed application version changed.".into());
            push_event(session, "The OS, workload set, or installed application version changed. Earlier evidence is stale; starting a fresh baseline.");
        }
        session.opted_in = true;
        session.status = Status::Monitoring;
        session.phase = Phase::Baseline;
        session.baseline = None;
        session.latest = None;
        session.samples.clear();
        session.original_configuration = originals;
        session.applied_configuration.clear();
        session.fingerprint = Some(fingerprint);
        session.stale = false;
        session.stale_reason = None;
        session.setting_support = if permission_denied {
            "Monitor-only: this change is not allowed in Settings → System. Enable the matching session right, then relearn.".into()
        } else if !setting_available {
            "Monitor-only: no safe supported runtime-setting API and readback are available.".into()
        } else {
            "A reversible runtime setting is available for this goal. Keep awake prevents idle system sleep only; it may increase energy use and does not improve FPS.".into()
        };
        session.drift_windows = 0;
        session.updated_at = at;
        push_event(session, "Optimization monitoring started with explicit permission. Collecting a native telemetry baseline; no setting has changed yet.");
        let result = session.clone();
        if let Err(error) = self.persist() {
            self.saved = previous;
            return Err(error);
        }
        Ok(result)
    }

    pub fn observe(&mut self, snapshot: &Snapshot) -> Result<(), String> {
        enum Next {
            SkipPersist,
            None,
            Apply(SettingState, TransactionKind),
            Complete(Window, Window, SettingState, SettingState, Outcome),
            Reevaluate,
        }
        if snapshot.source != "live" || !snapshot.sample_ready {
            return Ok(());
        }
        if !valid_snapshot(snapshot) {
            if let Some(session) = self
                .saved
                .sessions
                .iter_mut()
                .find(|session| session.status == Status::Monitoring)
            {
                session.samples.clear();
                push_event(session, "Ignored stale, non-finite, incomplete, or non-five-second telemetry. The evidence window was reset and no setting changed.");
                self.persist()?;
            }
            return Ok(());
        }
        self.refresh_paused_compatibility(snapshot)?;
        let Some(index) = self
            .saved
            .sessions
            .iter()
            .position(|session| session.status == Status::Monitoring)
        else {
            return Ok(());
        };
        let id = self.saved.sessions[index].id.clone();
        let current_fingerprint = self.fingerprint(snapshot);
        if self.saved.sessions[index].fingerprint.as_ref() != Some(&current_fingerprint) {
            if self.saved.sessions[index].owns_state() {
                let restoration = self.restore_owned(&id, "after environment drift");
                let session = self.session_mut(&id)?;
                session.status = Status::Paused;
                session.stale = true;
                session.stale_reason = Some(
                    "Monitoring stopped because the OS, workload set, or application version changed."
                        .into(),
                );
                session.samples.clear();
                push_event(session, "Monitoring paused because the OS, workload set, or installed application version changed. The current window was discarded; no performance claim was made.");
                self.persist()?;
                restoration?;
                return Ok(());
            }
            let session = self.session_mut(&id)?;
            session.fingerprint = Some(current_fingerprint);
            session.samples.clear();
            session.baseline = None;
            session.phase = Phase::Baseline;
            session.drift_windows = 0;
            push_event(session, "The running app set changed before any setting was owned. The baseline window was restarted; learning continued.");
            self.persist()?;
            return Ok(());
        }
        let next = {
            let session = &mut self.saved.sessions[index];
            if session
                .samples
                .last()
                .is_some_and(|sample| sample.at >= snapshot.captured_at)
            {
                return Ok(());
            }
            if let Some(previous) = session.samples.last() {
                let gap = snapshot.captured_at.saturating_sub(previous.at);
                if !(4..=7).contains(&gap) {
                    session.samples.clear();
                    push_event(session, "A telemetry gap reset the evidence window. Three fresh five-second-separated samples are required before any conclusion or setting change.");
                }
            }
            session.samples.push(Sample {
                at: snapshot.captured_at,
                cpu_percent: snapshot.cpu_percent,
                available_memory_bytes: snapshot.memory_available_bytes,
            });
            session.samples.truncate(3);
            session.updated_at = now();
            if session.phase == Phase::Watching {
                if session.samples.len() == 3 {
                    let latest = window(&session.samples);
                    let drifted = !session
                        .original_configuration
                        .iter()
                        .any(|state| state.setting == Setting::KeepAwake)
                        && session
                            .baseline
                            .as_ref()
                            .is_some_and(|baseline| drift(baseline, &latest));
                    session.latest = Some(latest);
                    session.samples.clear();
                    session.drift_windows = if drifted {
                        session.drift_windows.saturating_add(1)
                    } else {
                        0
                    };
                    if session.drift_windows >= SUSTAINED_DRIFT_WINDOWS
                        && snapshot.captured_at
                            >= session
                                .last_run_at
                                .saturating_add(REEVALUATION_COOLDOWN_SECONDS)
                    {
                        Next::Reevaluate
                    } else {
                        Next::None
                    }
                } else {
                    Next::SkipPersist
                }
            } else if session.phase == Phase::Baseline && session.samples.len() == 3 {
                session.baseline = Some(window(&session.samples));
                session.samples.clear();
                if let Some(original) = session.original_configuration.iter().find_map(|original| {
                    original
                        .setting
                        .experiment_target(original.value)
                        .filter(|target| *target != original.value)
                        .map(|_| original.clone())
                }) {
                    if !self
                        .adapter
                        .read(original.setting)
                        .is_ok_and(|value| value == original.value)
                    {
                        session.status = Status::Paused;
                        session.stale = true;
                        session.best_configuration.clear();
                        session.stale_reason = Some("The runtime setting changed or could not be read during baseline; no write was attempted.".into());
                        push_event(session, "Baseline runtime check failed. External changes were preserved without claiming ownership.");
                        return self.persist();
                    }
                    if original.setting == Setting::KeepAwake && original.value {
                        session.phase = Phase::Watching;
                        push_event(session, "The keep-awake request is already active. No change or new ownership was claimed; CPU reduction is not this goal.");
                        return self.persist();
                    }
                    let applied = SettingState {
                        setting: original.setting,
                        value: original
                            .setting
                            .experiment_target(original.value)
                            .unwrap_or(!original.value),
                    };
                    // This durable intent makes a crash between write and readback recoverable.
                    session.pending_transaction = Some(PendingTransaction {
                        kind: TransactionKind::ApplyExperiment,
                        setting: applied.setting,
                        expected_current: original.value,
                        target: applied.value,
                        original: original.value,
                    });
                    session.phase = Phase::MeasuringChange;
                    Next::Apply(applied, TransactionKind::ApplyExperiment)
                } else {
                    session.phase = Phase::Watching;
                    push_event(session, "Baseline complete. No safely supported native setting is available, so the session will monitor without making changes.");
                    Next::None
                }
            } else if session.phase == Phase::MeasuringChange && session.samples.len() == 3 {
                let after = window(&session.samples);
                let before = session
                    .baseline
                    .clone()
                    .ok_or("Baseline evidence is unavailable; no result was recorded.")?;
                let original = session
                    .original_configuration
                    .first()
                    .cloned()
                    .ok_or("Original setting state is unavailable.")?;
                let applied = session
                    .applied_configuration
                    .first()
                    .cloned()
                    .ok_or("Applied setting state is unavailable.")?;
                let outcome = if applied.setting == Setting::KeepAwake && applied.value {
                    Outcome::GoalMet
                } else {
                    compare(&before, &after)
                };
                Next::Complete(before, after, original, applied, outcome)
            } else {
                Next::None
            }
        };
        if matches!(&next, Next::SkipPersist) {
            return Ok(());
        }
        // Persist before any native write; only fixed adapter actions can follow this point.
        self.persist()?;
        match next {
            Next::SkipPersist => unreachable!(),
            Next::None => Ok(()),
            Next::Apply(applied, kind) => {
                let readback = self.adapter.apply(applied.setting, applied.value);
                if !matches!(readback, Ok(value) if value == applied.value) {
                    let session = &mut self.saved.sessions[index];
                    session.status = Status::Paused;
                    push_event(session, "The allowlisted setting write or immediate readback failed. Monitoring paused with the pending transaction journal intact; recovery will compare current state before any rollback.");
                    self.persist()?;
                    return Err("Setting write or readback failed; monitoring was paused.".into());
                }
                let session = &mut self.saved.sessions[index];
                session.pending_transaction = None;
                session.applied_configuration = vec![applied];
                push_event(session, match kind {
                    TransactionKind::ApplyExperiment => "Applied one reversible allowlisted runtime change. Immediate readback matched; runtime evidence is still observational.",
                    _ => "Applied a journaled setting change with immediate readback.",
                });
                self.persist()
            }
            Next::Complete(before, after, original, applied, outcome) => {
                match self.adapter.read(applied.setting) {
                    Ok(value) if value == applied.value => {}
                    Ok(_) => {
                        let session = &mut self.saved.sessions[index];
                        session.status = Status::Paused;
                        session.stale = true;
                        session.stale_reason = Some(
                            "The setting changed outside Jitterbug during the after window.".into(),
                        );
                        session.applied_configuration.clear();
                        session.samples.clear();
                        push_event(session, "The after-window-end readback no longer matched Jitterbug's owned value. Ownership was relinquished without overwriting the external change, and no result was recorded.");
                        self.persist()?;
                        return Ok(());
                    }
                    Err(_) => {
                        let session = &mut self.saved.sessions[index];
                        session.status = Status::Paused;
                        session.samples.clear();
                        push_event(session, "The after-window-end readback failed. Ownership and the restore journal were retained, monitoring paused, and no result was recorded.");
                        self.persist()?;
                        return Err(
                            "After-window setting readback failed; ownership was retained.".into(),
                        );
                    }
                }
                let rolled_back = !matches!(outcome, Outcome::Improved | Outcome::GoalMet);
                if rolled_back {
                    self.restore_owned(&id, "after an inconclusive or regressed run")?;
                }
                let session = &mut self.saved.sessions[index];
                if !rolled_back {
                    session.best_configuration = vec![applied.clone()];
                }
                let completed_at = after.ended_at;
                let run_id = format!("{}-run-{}", session.id, completed_at);
                let fingerprint_id = session
                    .fingerprint
                    .as_ref()
                    .map_or_else(String::new, |fingerprint| fingerprint.id.clone());
                session.findings.insert(0, Finding {
                    run_id: run_id.clone(),
                    fingerprint_id: fingerprint_id.clone(),
                    setting: applied.setting,
                    before_value: original.value,
                    after_value: applied.value,
                    before: before.clone(),
                    after: after.clone(),
                    outcome: outcome.clone(),
                    readback_verified: true,
                    rolled_back,
                    explanation: if applied.setting == Setting::KeepAwake { "Keep-awake goal: the process-owned idle-sleep assertion was verified active immediately and after the observation window. CPU and memory are context, not a success score. This does not prevent lid-close, explicit sleep, thermal or low-battery sleep; energy use can increase.".into() } else { "Matched native CPU and memory windows are observational evidence only. Immediate setting readback verifies configuration, not causality; five-second CPU samples cannot prove game FPS or jitter.".into() },
                });
                session.findings.truncate(MAX_FINDINGS);
                session.runs.insert(
                    0,
                    RunRecord {
                        id: run_id,
                        fingerprint_id,
                        started_at: before.started_at,
                        completed_at,
                        before: before.clone(),
                        after: after.clone(),
                        outcome: outcome.clone(),
                    },
                );
                session.runs.truncate(MAX_RUNS);
                session.samples.clear();
                session.phase = Phase::Watching;
                session.last_run_at = completed_at;
                session.drift_windows = 0;
                push_event(session, match outcome {
                    Outcome::GoalMet => "The owned idle-sleep assertion was active at both runtime checks. The keep-awake configuration goal is met, not a performance improvement or proof of uninterrupted execution.",
                    Outcome::Improved => "The after window was better by the session threshold. Kept as best observed, without claiming causality.",
                    Outcome::Regressed => "The after window regressed. Restored the original setting and verified readback.",
                    Outcome::Inconclusive => "The after window was inconclusive. Restored the original setting and verified readback.",
                });
                self.persist()
            }
            Next::Reevaluate => {
                if self.saved.sessions[index].owns_state() {
                    self.restore_owned(&id, "before conservative drift reevaluation")?;
                }
                let session = self.session_mut(&id)?;
                session.phase = Phase::Baseline;
                session.baseline = None;
                session.samples.clear();
                session.drift_windows = 0;
                push_event(session, "Sustained telemetry drift remained after the cooldown. A fresh baseline reevaluation is scheduled; no rapid setting flip is allowed.");
                self.persist()
            }
        }
    }

    pub fn pause(&mut self, id: &str) -> Result<Session, String> {
        let restoration = if self.session(id)?.owns_state() {
            self.restore_owned(id, "while pausing").map(|_| ())
        } else {
            Ok(())
        };
        let session = self.session_mut(id)?;
        session.status = Status::Paused;
        session.samples.clear();
        session.updated_at = now();
        push_event(
            session,
            if restoration.is_ok() {
                "Paused. Findings were preserved; any Jitterbug-owned setting was compare-checked and restored, while a session with no owned change performed no write."
            } else {
                "Paused and preserved findings, but original-setting restoration could not be verified. Restart recovery will retry any pending restoration."
            },
        );
        let result = session.clone();
        self.persist()?;
        restoration?;
        Ok(result)
    }

    pub fn reapply_best(
        &mut self,
        id: &str,
        explicit_opt_in: bool,
        snapshot: &Snapshot,
        automation_busy: bool,
    ) -> Result<Session, String> {
        if !explicit_opt_in {
            return Err("Explicit permission is required for this reapply.".into());
        }
        if automation_busy {
            return Err("Wait for the existing automatic incident action to finish.".into());
        }
        if !valid_snapshot(snapshot) {
            return Err(
                "Fresh live five-second telemetry is required; no setting was changed.".into(),
            );
        }
        if self.monitoring() || self.owns_native_state() {
            return Err("Another optimized session is monitoring or owns a native setting. Pause and restore it first.".into());
        }
        if self.session(id)?.status != Status::Paused {
            return Err("Only a paused, previously measured session can be reapplied.".into());
        }
        if self.session(id)?.stale {
            return Err(
                "This saved result is stale. Resume with a fresh baseline before reapply.".into(),
            );
        }
        let current = self.fingerprint(snapshot);
        let (expected, best) = {
            let session = self
                .saved
                .sessions
                .iter()
                .find(|session| session.id == id)
                .ok_or("Session unavailable.")?;
            (
                session.fingerprint.clone(),
                session.best_configuration.clone(),
            )
        };
        if expected.as_ref() != Some(&current) {
            let session = self.session_mut(id)?;
            session.stale = true;
            session.stale_reason =
                Some("The OS, workload set, or installed application version changed.".into());
            push_event(session, "Best configuration was not reapplied because the OS, workload set, or installed application version changed.");
            self.persist()?;
            return Err(
                "The saved best configuration is stale. Resume with a fresh baseline.".into(),
            );
        }
        if best.is_empty() {
            return Err("This session has no compatible best-observed configuration.".into());
        }
        for state in &best {
            if !self.adapter.supported(state.setting)? {
                return Err("Best-setting compatibility failed; no setting was changed.".into());
            }
        }
        let original = self
            .session(id)?
            .original_configuration
            .first()
            .cloned()
            .ok_or("The original setting journal is unavailable; no setting was changed.")?;
        let state = best[0].clone();
        if self.adapter.read(state.setting)? != original.value {
            let session = self.session_mut(id)?;
            session.stale = true;
            session.stale_reason = Some(
                "The current setting no longer matches the saved original; external changes were preserved."
                    .into(),
            );
            self.persist()?;
            return Err("The current setting changed outside Jitterbug; no write was made.".into());
        }
        self.session_mut(id)?.pending_transaction = Some(PendingTransaction {
            kind: TransactionKind::Reapply,
            setting: state.setting,
            expected_current: original.value,
            target: state.value,
            original: original.value,
        });
        self.persist()?;
        if !self
            .adapter
            .apply(state.setting, state.value)
            .is_ok_and(|readback| readback == state.value)
        {
            self.session_mut(id)?.status = Status::Paused;
            self.persist()?;
            return Err("Best-setting write or readback failed; the pending recovery journal was preserved.".into());
        }
        let session = self.session_mut(id)?;
        session.pending_transaction = None;
        session.applied_configuration = best;
        push_event(session, "Reapplied the compatible best-observed configuration through the runtime adapter with immediate readback. Monitoring remains paused and no causal or runtime-Stage-Manager claim is made.");
        let result = session.clone();
        self.persist()?;
        Ok(result)
    }

    pub fn restore_original(&mut self, id: &str) -> Result<Session, String> {
        if self.monitoring() {
            return Err(
                "Pause the monitoring session before using a separate restore action.".into(),
            );
        }
        if self
            .saved
            .sessions
            .iter()
            .any(|session| session.id != id && session.owns_state())
        {
            return Err("Another optimized session owns the native setting.".into());
        }
        if !self.session(id)?.owns_state() {
            return Err("This session does not own a setting change; no write was made.".into());
        }
        if self.session(id)?.pending_transaction.is_some() {
            self.recover(id)?;
            self.persist()?;
            if self.session(id)?.pending_transaction.is_some() {
                return Err(
                    "Recovery could not verify the runtime setting; journal retained.".into(),
                );
            }
        }
        self.restore_owned(id, "on explicit restore")?;
        let session = self.session_mut(id)?;
        session.status = Status::Paused;
        push_event(
            session,
            "Compare-checked Jitterbug's owned value, restored the original configuration, and verified immediate readback.",
        );
        let result = session.clone();
        self.persist()?;
        Ok(result)
    }

    fn restore_owned(&mut self, id: &str, reason: &str) -> Result<bool, String> {
        let (original, applied) = {
            let session = self.session(id)?;
            (
                session.original_configuration.first().cloned(),
                session.applied_configuration.first().cloned(),
            )
        };
        let (Some(original), Some(applied)) = (original, applied) else {
            if self.session(id)?.pending_transaction.is_some() {
                return Err(
                    "A pending setting transaction must be recovered before restoration.".into(),
                );
            }
            return Ok(false);
        };
        let current = self.adapter.read(applied.setting)?;
        if current == original.value {
            let session = self.session_mut(id)?;
            session.applied_configuration.clear();
            session.pending_transaction = None;
            push_event(session, "The setting already matched the original value. Ownership was cleared without writing.");
            self.persist()?;
            return Ok(false);
        }
        if current != applied.value {
            let session = self.session_mut(id)?;
            session.applied_configuration.clear();
            session.pending_transaction = None;
            session.stale = true;
            session.stale_reason = Some(
                "The setting changed externally; Jitterbug relinquished ownership without writing."
                    .into(),
            );
            push_event(session, "The current setting no longer matched either Jitterbug's owned value or the original. Ownership was relinquished without overwriting the external change.");
            self.persist()?;
            return Err(
                "The setting changed outside Jitterbug; the external value was not overwritten."
                    .into(),
            );
        }
        self.session_mut(id)?.pending_transaction = Some(PendingTransaction {
            kind: TransactionKind::Restore,
            setting: original.setting,
            expected_current: applied.value,
            target: original.value,
            original: original.value,
        });
        self.persist()?;
        if !self
            .adapter
            .apply(original.setting, original.value)
            .is_ok_and(|readback| readback == original.value)
        {
            let session = self.session_mut(id)?;
            session.status = Status::Paused;
            push_event(session, "Restore write or readback failed. The pending restore journal and ownership were retained for safe restart recovery.");
            self.persist()?;
            return Err(
                "Original-setting readback failed; the pending restore journal remains durable."
                    .into(),
            );
        }
        let session = self.session_mut(id)?;
        session.pending_transaction = None;
        session.applied_configuration.clear();
        push_event(session, &format!("Restored the original setting {reason} after compare-current ownership verification."));
        self.persist()?;
        Ok(true)
    }

    fn recover(&mut self, id: &str) -> Result<bool, String> {
        self.session(id)?.validate_journal()?;
        let (status, pending, applied, original) = {
            let session = self.session(id)?;
            (
                session.status.clone(),
                session.pending_transaction.clone(),
                session.applied_configuration.first().cloned(),
                session.original_configuration.first().cloned(),
            )
        };
        if status != Status::Monitoring && pending.is_none() && applied.is_none() {
            return Ok(false);
        }
        let mut changed = status == Status::Monitoring;
        self.session_mut(id)?.status = Status::Paused;
        if let Some(setting) = pending
            .as_ref()
            .map(|transaction| transaction.setting)
            .or_else(|| applied.as_ref().map(|state| state.setting))
        {
            if !self.adapter.supported(setting).unwrap_or(false) {
                push_event(self.session_mut(id)?, "Recovery retained an unsupported setting journal without attempting a native write.");
                return Ok(true);
            }
        }
        if let Some(transaction) = pending {
            let current = match self.adapter.read(transaction.setting) {
                Ok(current) => current,
                Err(_) => {
                    push_event(self.session_mut(id)?, "Restart paused this session, but the pending transaction could not be read safely. The journal was retained and no write was attempted.");
                    return Ok(true);
                }
            };
            let resolved = match transaction.kind {
                TransactionKind::Restore => {
                    if current == transaction.target {
                        true
                    } else if current == transaction.expected_current {
                        self.adapter
                            .apply(transaction.setting, transaction.target)
                            .is_ok_and(|readback| readback == transaction.target)
                    } else {
                        let session = self.session_mut(id)?;
                        session.stale = true;
                        session.stale_reason = Some(
                            "External setting change detected during restart recovery.".into(),
                        );
                        push_event(session, "Restart recovery found an external setting value and did not overwrite it.");
                        true
                    }
                }
                TransactionKind::ApplyExperiment | TransactionKind::Reapply => {
                    if current == transaction.original {
                        true
                    } else if current == transaction.target {
                        self.adapter
                            .apply(transaction.setting, transaction.original)
                            .is_ok_and(|readback| readback == transaction.original)
                    } else {
                        let session = self.session_mut(id)?;
                        session.stale = true;
                        session.stale_reason = Some(
                            "External setting change detected during restart recovery.".into(),
                        );
                        push_event(session, "Restart recovery found an external setting value and did not overwrite it.");
                        true
                    }
                }
            };
            if resolved {
                let session = self.session_mut(id)?;
                session.pending_transaction = None;
                session.applied_configuration.clear();
                push_event(session, "Restart recovery resolved the pending transaction using compare-current checks. Optimization did not resume.");
            } else {
                push_event(self.session_mut(id)?, "Restart recovery could not verify rollback. The transaction journal remains pending and optimization did not resume.");
            }
            return Ok(true);
        }
        if let (Some(applied), Some(original)) = (applied, original) {
            let current = match self.adapter.read(applied.setting) {
                Ok(current) => current,
                Err(_) => {
                    push_event(self.session_mut(id)?, "Restart paused the session but could not read the intentionally applied setting. Ownership was retained and no write was attempted.");
                    return Ok(true);
                }
            };
            if current == applied.value {
                if self
                    .adapter
                    .apply(original.setting, original.value)
                    .is_ok_and(|readback| readback == original.value)
                {
                    self.session_mut(id)?.applied_configuration.clear();
                    push_event(self.session_mut(id)?, "Restart compare-checked and restored the intentionally applied configuration. Optimization did not resume.");
                } else {
                    push_event(self.session_mut(id)?, "Restart could not verify restoration of the intentionally applied configuration. Ownership was retained.");
                }
            } else if current == original.value {
                self.session_mut(id)?.applied_configuration.clear();
                push_event(self.session_mut(id)?, "Restart found the original value already present and cleared ownership without writing.");
            } else {
                let session = self.session_mut(id)?;
                session.applied_configuration.clear();
                session.stale = true;
                session.stale_reason =
                    Some("External setting change detected during restart recovery.".into());
                push_event(session, "Restart found an external setting value, relinquished ownership, and did not overwrite it.");
            }
            changed = true;
        }
        if changed {
            push_event(
                self.session_mut(id)?,
                "Monitoring was paused after application restart and never resumes automatically.",
            );
        }
        Ok(changed)
    }

    fn setting_support(&self) -> String {
        if self.adapter.supported(Setting::KeepAwake).unwrap_or(false) {
            return "macOS: goals beginning Keep awake: can request a temporary, runtime-verified idle-sleep assertion. Other goals monitor only; Stage Manager is unsupported. No FPS or CPU improvement is promised.".into();
        }
        if self
            .adapter
            .supported(Setting::StageManager)
            .unwrap_or(false)
        {
            "One reversible runtime setting is reported by the active adapter.".into()
        } else {
            "Monitor-only: Stage Manager preference readback is not runtime-state verification, and no safe supported runtime mutation API is available.".into()
        }
    }

    fn refresh_paused_compatibility(&mut self, snapshot: &Snapshot) -> Result<(), String> {
        let current = self.fingerprint(snapshot);
        let ids: Vec<_> = self
            .saved
            .sessions
            .iter()
            .filter(|session| {
                session.status != Status::Monitoring
                    && (!session.best_configuration.is_empty() || session.owns_state())
                    && session.fingerprint.as_ref() != Some(&current)
                    && !session.stale
            })
            .map(|session| session.id.clone())
            .collect();
        for id in ids {
            let restoration = if self.session(&id)?.owns_state() {
                self.restore_owned(&id, "after paused-session environment drift")
            } else {
                Ok(false)
            };
            let session = self.session_mut(&id)?;
            session.stale = true;
            session.stale_reason = Some(
                "The current OS, workload set, or installed application versions no longer match this result."
                    .into(),
            );
            push_event(session, "Live compatibility refresh marked this paused result stale before reapply. Any Jitterbug-owned state was compare-checked for conservative restoration.");
            self.persist()?;
            restoration?;
        }
        Ok(())
    }

    fn fingerprint(&mut self, snapshot: &Snapshot) -> Fingerprint {
        let mut workloads: Vec<_> = snapshot
            .workloads
            .iter()
            .map(|workload| workload.id.clone())
            .collect();
        workloads.sort();
        workloads.dedup();

        let mut applications = Vec::with_capacity(workloads.len());
        for workload_id in &workloads {
            let identity = snapshot
                .processes
                .iter()
                .filter(|process| process.workload_id == *workload_id)
                .filter_map(|process| process.executable.as_deref())
                .min()
                .map(|path| self.applications.identity(path))
                .unwrap_or_default();
            applications.push(ApplicationFingerprint {
                workload_id: workload_id.clone(),
                identifier: identity.0,
                version: identity.1,
            });
        }
        let mut fingerprint = Fingerprint {
            os: format!("{}:{}", snapshot.platform.os, self.os_version),
            logical_cpus: snapshot.logical_cpus,
            workloads,
            applications,
            id: String::new(),
        };
        fingerprint.id = fingerprint_id(&fingerprint);
        // Hash the entire collector inventory before bounding the persisted/display detail.
        fingerprint.workloads.truncate(MAX_FINGERPRINT_ITEMS);
        fingerprint.applications.truncate(MAX_FINGERPRINT_ITEMS);
        fingerprint
    }

    fn session(&self, id: &str) -> Result<&Session, String> {
        self.saved
            .sessions
            .iter()
            .find(|session| session.id == id)
            .ok_or("Session unavailable.".into())
    }

    fn session_mut(&mut self, id: &str) -> Result<&mut Session, String> {
        self.saved
            .sessions
            .iter_mut()
            .find(|session| session.id == id)
            .ok_or("Session unavailable.".into())
    }

    fn persist(&self) -> Result<(), String> {
        storage::write(&self.directory.join(FILE_NAME), &self.saved)
    }

    #[cfg(test)]
    pub(crate) fn test_set_status(&mut self, id: &str, status: Status) -> Result<(), String> {
        self.saved
            .sessions
            .iter_mut()
            .find(|session| session.id == id)
            .ok_or("Session unavailable.")?
            .status = status;
        self.persist()
    }
}

impl Session {
    fn validate_journal(&self) -> Result<(), String> {
        let invalid = || {
            "Invalid optimized-session recovery journal; no native write was attempted.".to_string()
        };
        if self.applied_configuration.len() > 1 || self.best_configuration.len() > 1 {
            return Err(invalid());
        }
        if !self.owns_state() {
            return Ok(());
        }
        if !self.opted_in {
            return Err(invalid());
        }
        let active = self
            .applied_configuration
            .first()
            .map(|state| state.setting)
            .or_else(|| self.pending_transaction.as_ref().map(|tx| tx.setting))
            .ok_or_else(invalid)?;
        let original = self
            .original_configuration
            .iter()
            .find(|state| state.setting == active)
            .ok_or_else(invalid)?;
        // This process-owned assertion starts absent; recovery must never create one.
        if original.setting == Setting::KeepAwake && original.value {
            return Err(invalid());
        }
        if let Some(applied) = self.applied_configuration.first() {
            if applied.setting != original.setting || applied.value == original.value {
                return Err(invalid());
            }
        }
        if let Some(transaction) = &self.pending_transaction {
            if transaction.setting != original.setting || transaction.original != original.value {
                return Err(invalid());
            }
            let valid = match transaction.kind {
                TransactionKind::Restore => {
                    transaction.target == original.value
                        && transaction.expected_current != original.value
                }
                TransactionKind::ApplyExperiment | TransactionKind::Reapply => {
                    transaction.expected_current == original.value
                        && transaction.target != original.value
                }
            };
            if !valid {
                return Err(invalid());
            }
        }
        Ok(())
    }

    fn owns_state(&self) -> bool {
        self.pending_transaction.is_some() || !self.applied_configuration.is_empty()
    }

    fn safe_to_evict(&self) -> bool {
        self.status != Status::Monitoring && !self.owns_state()
    }
}

fn valid_snapshot(snapshot: &Snapshot) -> bool {
    snapshot.source == "live"
        && snapshot.sample_ready
        && snapshot.captured_at.saturating_add(20) >= now()
        && (4_500..=5_500).contains(&snapshot.interval_ms)
        && snapshot.cpu_percent.is_finite()
        && (0.0..=100.0).contains(&snapshot.cpu_percent)
        && snapshot.memory_total_bytes > 0
        && snapshot.memory_available_bytes <= snapshot.memory_total_bytes
        && snapshot.logical_cpus > 0
        && !snapshot.workloads.is_empty()
}

fn fingerprint_id(fingerprint: &Fingerprint) -> String {
    // Explicit FNV-1a keeps persisted evidence IDs stable across processes and Rust releases.
    let mut hash = 0xcbf29ce484222325_u64;
    let mut update = |value: &[u8]| {
        for byte in value.iter().copied().chain(std::iter::once(0xff)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    update(fingerprint.os.as_bytes());
    update(&fingerprint.logical_cpus.to_le_bytes());
    for workload in &fingerprint.workloads {
        update(workload.as_bytes());
    }
    for application in &fingerprint.applications {
        update(application.workload_id.as_bytes());
        update(application.identifier.as_deref().unwrap_or("").as_bytes());
        update(application.version.as_deref().unwrap_or("").as_bytes());
    }
    format!("environment-{hash:016x}")
}

fn window(samples: &[Sample]) -> Window {
    Window {
        started_at: samples.first().map_or(0, |sample| sample.at),
        ended_at: samples.last().map_or(0, |sample| sample.at),
        samples: samples.len(),
        average_cpu_percent: samples.iter().map(|sample| sample.cpu_percent).sum::<f32>()
            / samples.len().max(1) as f32,
        average_available_memory_bytes: samples
            .iter()
            .map(|sample| sample.available_memory_bytes as u128)
            .sum::<u128>()
            .checked_div(samples.len().max(1) as u128)
            .unwrap_or_default() as u64,
    }
}

fn compare(before: &Window, after: &Window) -> Outcome {
    let cpu_change = after.average_cpu_percent - before.average_cpu_percent;
    let memory_floor = before.average_available_memory_bytes / 20;
    if cpu_change <= -5.0
        && after
            .average_available_memory_bytes
            .saturating_add(memory_floor)
            >= before.average_available_memory_bytes
    {
        Outcome::Improved
    } else if cpu_change >= 5.0
        || after
            .average_available_memory_bytes
            .saturating_add(memory_floor)
            < before.average_available_memory_bytes
    {
        Outcome::Regressed
    } else {
        Outcome::Inconclusive
    }
}

fn drift(before: &Window, after: &Window) -> bool {
    (after.average_cpu_percent - before.average_cpu_percent).abs() >= 10.0
        || after.average_available_memory_bytes
            < before
                .average_available_memory_bytes
                .saturating_sub(before.average_available_memory_bytes / 10)
}

fn push_event(session: &mut Session, message: &str) {
    session.events.push(Event {
        at: now(),
        message: message.into(),
    });
    if session.events.len() > MAX_EVENTS {
        session.events.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FakeAdapter {
        value: Mutex<bool>,
        writes: Mutex<Vec<bool>>,
        fail_reads: Mutex<usize>,
        fail_writes: Mutex<usize>,
    }

    impl SettingAdapter for FakeAdapter {
        fn supported(&self, setting: Setting) -> Result<bool, String> {
            assert!(matches!(
                setting,
                Setting::StageManager | Setting::KeepAwake
            ));
            Ok(true)
        }

        fn read(&self, setting: Setting) -> Result<bool, String> {
            assert!(matches!(
                setting,
                Setting::StageManager | Setting::KeepAwake
            ));
            let mut failures = self.fail_reads.lock().unwrap();
            if *failures > 0 {
                *failures -= 1;
                return Err("Synthetic adapter read failure.".into());
            }
            Ok(*self.value.lock().unwrap())
        }

        fn apply(&self, setting: Setting, value: bool) -> Result<bool, String> {
            assert!(matches!(
                setting,
                Setting::StageManager | Setting::KeepAwake
            ));
            let mut failures = self.fail_writes.lock().unwrap();
            if *failures > 0 {
                *failures -= 1;
                return Err("Synthetic adapter write failure.".into());
            }
            self.writes.lock().unwrap().push(value);
            *self.value.lock().unwrap() = value;
            Ok(value)
        }
    }

    struct UnsupportedAdapter;

    impl SettingAdapter for UnsupportedAdapter {
        fn supported(&self, _: Setting) -> Result<bool, String> {
            Ok(false)
        }

        fn read(&self, _: Setting) -> Result<bool, String> {
            Err("Unsupported setting read.".into())
        }

        fn apply(&self, _: Setting, _: bool) -> Result<bool, String> {
            panic!("unsupported adapters must never receive writes")
        }
    }

    fn snapshot(at: u64, cpu: f32, apps: &[&str]) -> crate::types::Snapshot {
        crate::types::Snapshot {
            source: "live".into(),
            captured_at: at,
            sample_ready: true,
            interval_ms: 5_000,
            cpu_percent: cpu,
            logical_cpus: 8,
            memory_total_bytes: 16 * 1024 * 1024 * 1024,
            memory_available_bytes: 8 * 1024 * 1024 * 1024,
            workloads: apps
                .iter()
                .map(|name| crate::types::Workload {
                    id: (*name).into(),
                    label: (*name).into(),
                    ..Default::default()
                })
                .collect(),
            platform: crate::types::PlatformReadings {
                os: "macOS test".into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn temp_directory(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "jitterbug-optimized-session-{label}-{}-{}",
            std::process::id(),
            crate::types::now()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn creates_validated_saved_sessions_in_private_bounded_app_data() {
        let directory = temp_directory("create");
        let mut store = Store::load(directory.clone()).unwrap();
        assert!(store.create("", "keep the editor responsive").is_err());
        assert!(store.create("Build", "\u{0000}").is_err());

        for number in 0..25 {
            store
                .create(
                    &format!("Build {number}"),
                    "Keep the editor responsive while compiling",
                )
                .unwrap();
        }

        let reloaded = Store::load(directory.clone()).unwrap();
        assert_eq!(reloaded.sessions().len(), MAX_SESSIONS);
        assert_eq!(
            reloaded
                .sessions()
                .iter()
                .map(|session| &session.id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            MAX_SESSIONS
        );
        assert_eq!(reloaded.sessions()[0].name, "Build 24");
        assert_eq!(
            reloaded.sessions()[0].goal,
            "Keep the editor responsive while compiling"
        );
        assert!(!reloaded.sessions()[0].opted_in);
        assert_eq!(reloaded.sessions()[0].status, Status::Saved);
        assert!(directory.join(FILE_NAME).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(directory.join(FILE_NAME))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o077,
                0
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_session_permission_forces_monitor_only() {
        let directory = temp_directory("permission-gate");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store
            .create_proposed(
                "Export",
                "Keep this PC awake overnight",
                ProposedProfile {
                    capability: Capability::KeepAwake,
                    rationale: "Long unattended export.".into(),
                    provider: "test".into(),
                    model: "test".into(),
                },
            )
            .unwrap();
        store
            .start_restricted(
                &session.id,
                true,
                &snapshot(now(), 40.0, &["export"]),
                false,
                Some(&[]),
            )
            .unwrap();
        assert!(store.sessions()[0].original_configuration.is_empty());
        assert!(store.sessions()[0].setting_support.contains("not allowed"));
        assert!(adapter.writes.lock().unwrap().is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn update_proposal_replaces_legacy_monitor_only_profile() {
        let directory = temp_directory("repropose");
        let mut store = Store::load(directory.clone()).unwrap();
        let session = store.create("Legacy", "Keep work reliable").unwrap();
        store
            .update_proposal(
                &session.id,
                ProposedProfile {
                    capability: Capability::KeepAwake,
                    rationale: "Stay awake for the long job.".into(),
                    provider: "meta".into(),
                    model: "muse-spark-1.3".into(),
                },
            )
            .unwrap();
        let saved = Store::load(directory.clone()).unwrap();
        assert_eq!(
            saved.sessions()[0].proposal.capability,
            Capability::KeepAwake
        );
        assert_eq!(saved.sessions()[0].proposal.provider, "meta");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn deletes_only_sessions_that_do_not_own_active_or_recovery_state() {
        let directory = temp_directory("delete");
        let mut store = Store::load(directory.clone()).unwrap();
        let removable = store.create("Removable", "Observe a task").unwrap();
        let active = store.create("Active", "Observe another task").unwrap();
        store
            .test_set_status(&active.id, Status::Monitoring)
            .unwrap();

        assert!(store.delete(&active.id).is_err());
        store.delete(&removable.id).unwrap();
        assert!(store.sessions().iter().all(|item| item.id != removable.id));
        assert!(Store::load(directory)
            .unwrap()
            .sessions()
            .iter()
            .all(|item| item.id != removable.id));
    }

    #[test]
    fn capacity_never_evicts_a_monitoring_session_or_its_restore_journal() {
        let directory = temp_directory("safe-capacity");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter).unwrap();
        let protected = store
            .create("Protected", "Keep my rollback journal")
            .unwrap();
        store
            .test_set_status(&protected.id, Status::Monitoring)
            .unwrap();
        for number in 1..MAX_SESSIONS {
            store
                .create(
                    &format!("Safe {number}"),
                    "A safely evictable saved session",
                )
                .unwrap();
        }

        store
            .create("Newest", "Do not evict the active owner")
            .unwrap();

        assert_eq!(store.sessions().len(), MAX_SESSIONS);
        assert!(store
            .sessions()
            .iter()
            .any(|session| session.id == protected.id));
        assert!(store.monitoring());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn pause_and_restore_do_not_write_without_an_owned_change() {
        let directory = temp_directory("no-owned-write");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store.create("Game", "Observe only for now").unwrap();
        let sample = snapshot(crate::types::now(), 60.0, &["game"]);
        store.start(&session.id, true, &sample, false).unwrap();

        store.pause(&session.id).unwrap();
        assert!(adapter.writes.lock().unwrap().is_empty());
        assert!(store.restore_original(&session.id).is_err());
        assert!(adapter.writes.lock().unwrap().is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn freeform_goal_accepts_newlines_but_not_other_control_characters() {
        let directory = temp_directory("multiline-goal");
        let mut store = Store::load(directory.clone()).unwrap();
        let session = store
            .create("Build", "Keep the editor responsive\nwhile tests run")
            .unwrap();
        assert!(session.goal.contains('\n'));
        assert!(store.create("Bad", "hidden\u{0000}control").is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn restart_pauses_monitoring_session_without_auto_resume() {
        let directory = temp_directory("restart");
        let mut store = Store::load(directory.clone()).unwrap();
        let session = store
            .create("Game", "Reduce background contention")
            .unwrap();
        store
            .test_set_status(&session.id, Status::Monitoring)
            .unwrap();

        let reloaded = Store::load(directory.clone()).unwrap();
        assert_eq!(reloaded.sessions()[0].status, Status::Paused);
        assert!(reloaded.sessions()[0]
            .events
            .last()
            .unwrap()
            .message
            .contains("restart"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn requires_explicit_opt_in_and_builds_baseline_before_one_allowlisted_change() {
        let directory = temp_directory("baseline");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let first = store
            .create("Game", "Reduce background contention")
            .unwrap();
        let second = store.create("Build", "Keep calls smooth").unwrap();
        let sample = snapshot(crate::types::now(), 60.0, &["game"]);

        assert!(store.start(&first.id, false, &sample, false).is_err());
        assert!(store.start(&first.id, true, &sample, true).is_err());
        store.start(&first.id, true, &sample, false).unwrap();
        assert!(store.start(&second.id, true, &sample, false).is_err());
        assert!(adapter.writes.lock().unwrap().is_empty());

        store
            .observe(&snapshot(sample.captured_at + 5, 62.0, &["game"]))
            .unwrap();
        store
            .observe(&snapshot(sample.captured_at + 10, 61.0, &["game"]))
            .unwrap();
        assert!(adapter.writes.lock().unwrap().is_empty());
        store
            .observe(&snapshot(sample.captured_at + 15, 60.0, &["game"]))
            .unwrap();

        assert_eq!(*adapter.writes.lock().unwrap(), vec![true]);
        let active = store.sessions().iter().find(|s| s.id == first.id).unwrap();
        assert!(active.opted_in);
        assert_eq!(active.status, Status::Monitoring);
        assert_eq!(active.baseline.as_ref().unwrap().samples, 3);
        assert_eq!(active.phase, Phase::MeasuringChange);
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn run_baseline(store: &mut Store, at: u64, cpu: f32, apps: &[&str]) {
        for offset in [5, 10, 15] {
            store.observe(&snapshot(at + offset, cpu, apps)).unwrap();
        }
    }

    #[test]
    fn regressed_change_is_rolled_back_with_observational_evidence() {
        let directory = temp_directory("rollback");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store.create("Game", "Reduce jitter").unwrap();
        let at = crate::types::now();
        store
            .start(&session.id, true, &snapshot(at, 50.0, &["game"]), false)
            .unwrap();
        run_baseline(&mut store, at, 50.0, &["game"]);
        for offset in [20, 25, 30] {
            store
                .observe(&snapshot(at + offset, 62.0, &["game"]))
                .unwrap();
        }

        assert_eq!(*adapter.writes.lock().unwrap(), vec![true, false]);
        let session = &store.sessions()[0];
        assert_eq!(session.findings[0].outcome, Outcome::Regressed);
        assert!(session.findings[0].readback_verified);
        assert!(session.findings[0]
            .explanation
            .contains("cannot prove game FPS or jitter"));
        assert_eq!(session.phase, Phase::Watching);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn pause_reapply_restore_and_stale_resume_are_safe_and_preserve_findings() {
        let directory = temp_directory("reapply");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let created = store.create("Game", "Reduce jitter").unwrap();
        let at = crate::types::now();
        let same = snapshot(at, 60.0, &["game"]);
        store.start(&created.id, true, &same, false).unwrap();
        run_baseline(&mut store, at, 60.0, &["game"]);
        for offset in [20, 25, 30] {
            store
                .observe(&snapshot(at + offset, 45.0, &["game"]))
                .unwrap();
        }
        assert_eq!(store.sessions()[0].findings[0].outcome, Outcome::Improved);
        assert_eq!(store.sessions()[0].best_configuration.len(), 1);

        store.pause(&created.id).unwrap();
        assert_eq!(store.sessions()[0].status, Status::Paused);
        assert_eq!(store.sessions()[0].findings.len(), 1);
        assert!(!*adapter.value.lock().unwrap());

        store.reapply_best(&created.id, true, &same, false).unwrap();
        assert!(*adapter.value.lock().unwrap());
        assert_eq!(store.sessions()[0].status, Status::Paused);
        store.restore_original(&created.id).unwrap();
        assert!(!*adapter.value.lock().unwrap());

        let changed = snapshot(at + 35, 55.0, &["game", "streaming-app"]);
        store.observe(&changed).unwrap();
        assert!(store.sessions()[0].stale);
        assert!(store
            .reapply_best(&created.id, true, &changed, false)
            .is_err());
        assert!(store.sessions()[0].stale);
        store.start(&created.id, true, &changed, false).unwrap();
        assert!(!store.sessions()[0].stale);
        assert!(store.sessions()[0].best_configuration.is_empty());
        assert_eq!(store.sessions()[0].phase, Phase::Baseline);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn reapply_is_rejected_while_another_session_monitors() {
        let directory = temp_directory("reapply-owner");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let first = store.create("First", "Find a best setting").unwrap();
        let second = store.create("Second", "Monitor exclusively").unwrap();
        let at = crate::types::now();
        let sample = snapshot(at, 60.0, &["game"]);
        store.start(&first.id, true, &sample, false).unwrap();
        run_baseline(&mut store, at, 60.0, &["game"]);
        for offset in [20, 25, 30] {
            store
                .observe(&snapshot(at + offset, 45.0, &["game"]))
                .unwrap();
        }
        store.pause(&first.id).unwrap();
        store.start(&second.id, true, &sample, false).unwrap();

        assert!(store.reapply_best(&first.id, true, &sample, false).is_err());
        assert!(adapter.writes.lock().unwrap().ends_with(&[false]));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn restart_recovers_a_paused_reapply_without_resuming_optimization() {
        let directory = temp_directory("paused-reapply-recovery");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let created = store.create("Game", "Reduce jitter").unwrap();
        let at = crate::types::now();
        let same = snapshot(at, 60.0, &["game"]);
        store.start(&created.id, true, &same, false).unwrap();
        run_baseline(&mut store, at, 60.0, &["game"]);
        for offset in [20, 25, 30] {
            store
                .observe(&snapshot(at + offset, 45.0, &["game"]))
                .unwrap();
        }
        store.pause(&created.id).unwrap();
        store.reapply_best(&created.id, true, &same, false).unwrap();
        assert!(*adapter.value.lock().unwrap());
        drop(store);

        let reloaded = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        assert!(!*adapter.value.lock().unwrap());
        assert_eq!(reloaded.sessions()[0].status, Status::Paused);
        assert!(reloaded.sessions()[0].applied_configuration.is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn invalid_or_gapped_telemetry_never_triggers_a_setting_write() {
        let directory = temp_directory("invalid-telemetry");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let created = store.create("Game", "Reject weak evidence").unwrap();
        let at = crate::types::now();
        store
            .start(&created.id, true, &snapshot(at, 60.0, &["game"]), false)
            .unwrap();
        store
            .observe(&snapshot(at + 5, f32::NAN, &["game"]))
            .unwrap();
        store.observe(&snapshot(at + 30, 60.0, &["game"])).unwrap();
        store.observe(&snapshot(at + 35, 60.0, &["game"])).unwrap();

        assert!(adapter.writes.lock().unwrap().is_empty());
        assert!(store.sessions()[0].baseline.is_none());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn after_window_external_change_is_inconclusive_and_never_overwritten() {
        let directory = temp_directory("external-change");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let created = store.create("Game", "Preserve user changes").unwrap();
        let at = crate::types::now();
        store
            .start(&created.id, true, &snapshot(at, 60.0, &["game"]), false)
            .unwrap();
        run_baseline(&mut store, at, 60.0, &["game"]);
        *adapter.value.lock().unwrap() = false;
        for offset in [20, 25, 30] {
            store
                .observe(&snapshot(at + offset, 40.0, &["game"]))
                .unwrap();
        }

        assert_eq!(*adapter.writes.lock().unwrap(), vec![true]);
        assert!(store.sessions()[0].findings.is_empty());
        assert_eq!(store.sessions()[0].status, Status::Paused);
        assert!(store.sessions()[0].stale);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unsupported_adapter_is_monitor_only_and_inconclusive_runs_roll_back() {
        let unsupported_directory = temp_directory("unsupported");
        let mut unsupported =
            Store::load_with_adapter(unsupported_directory.clone(), Arc::new(UnsupportedAdapter))
                .unwrap();
        let session = unsupported.create("Observe", "Monitor only").unwrap();
        let at = crate::types::now();
        unsupported
            .start(&session.id, true, &snapshot(at, 50.0, &["game"]), false)
            .unwrap();
        run_baseline(&mut unsupported, at, 50.0, &["game"]);
        assert_eq!(unsupported.sessions()[0].phase, Phase::Watching);
        assert!(unsupported.sessions()[0].original_configuration.is_empty());

        let inconclusive_directory = temp_directory("inconclusive");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store =
            Store::load_with_adapter(inconclusive_directory.clone(), adapter.clone()).unwrap();
        let session = store
            .create("Compare", "Require adequate evidence")
            .unwrap();
        store
            .start(&session.id, true, &snapshot(at, 50.0, &["game"]), false)
            .unwrap();
        run_baseline(&mut store, at, 50.0, &["game"]);
        for offset in [20, 25, 30] {
            store
                .observe(&snapshot(at + offset, 49.0, &["game"]))
                .unwrap();
        }
        assert_eq!(
            store.sessions()[0].findings[0].outcome,
            Outcome::Inconclusive
        );
        assert_eq!(*adapter.writes.lock().unwrap(), vec![true, false]);
        std::fs::remove_dir_all(unsupported_directory).unwrap();
        std::fs::remove_dir_all(inconclusive_directory).unwrap();
    }

    #[test]
    fn interrupted_apply_keeps_a_recoverable_transaction_and_never_resumes() {
        let directory = temp_directory("interrupted-apply");
        let adapter = Arc::new(FakeAdapter::default());
        *adapter.fail_writes.lock().unwrap() = 1;
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store.create("Game", "Recover safely").unwrap();
        let at = crate::types::now();
        store
            .start(&session.id, true, &snapshot(at, 60.0, &["game"]), false)
            .unwrap();
        assert!(store.observe(&snapshot(at + 5, 60.0, &["game"])).is_ok());
        assert!(store.observe(&snapshot(at + 10, 60.0, &["game"])).is_ok());
        assert!(store.observe(&snapshot(at + 15, 60.0, &["game"])).is_err());
        assert!(store.sessions()[0].pending_transaction.is_some());
        drop(store);

        let reloaded = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        assert_eq!(reloaded.sessions()[0].status, Status::Paused);
        assert!(reloaded.sessions()[0].pending_transaction.is_none());
        assert!(adapter.writes.lock().unwrap().is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_rollback_pauses_and_restart_recovers_the_pending_restore() {
        let directory = temp_directory("failed-rollback");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store.create("Game", "Recover rollback").unwrap();
        let at = crate::types::now();
        store
            .start(&session.id, true, &snapshot(at, 60.0, &["game"]), false)
            .unwrap();
        run_baseline(&mut store, at, 60.0, &["game"]);
        *adapter.fail_writes.lock().unwrap() = 1;
        for offset in [20, 25] {
            store
                .observe(&snapshot(at + offset, 75.0, &["game"]))
                .unwrap();
        }
        assert!(store.observe(&snapshot(at + 30, 75.0, &["game"])).is_err());
        assert_eq!(store.sessions()[0].status, Status::Paused);
        assert_eq!(
            store.sessions()[0]
                .pending_transaction
                .as_ref()
                .unwrap()
                .kind,
            TransactionKind::Restore
        );
        assert!(*adapter.value.lock().unwrap());
        drop(store);

        let reloaded = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        assert!(!*adapter.value.lock().unwrap());
        assert!(reloaded.sessions()[0].pending_transaction.is_none());
        assert_eq!(reloaded.sessions()[0].status, Status::Paused);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_after_window_readback_retains_ownership_without_a_claim() {
        let directory = temp_directory("failed-final-read");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store.create("Game", "Keep recovery state").unwrap();
        let at = crate::types::now();
        store
            .start(&session.id, true, &snapshot(at, 60.0, &["game"]), false)
            .unwrap();
        run_baseline(&mut store, at, 60.0, &["game"]);
        *adapter.fail_reads.lock().unwrap() = 1;
        for offset in [20, 25] {
            store
                .observe(&snapshot(at + offset, 45.0, &["game"]))
                .unwrap();
        }
        assert!(store.observe(&snapshot(at + 30, 45.0, &["game"])).is_err());

        assert!(store.sessions()[0].findings.is_empty());
        assert_eq!(store.sessions()[0].status, Status::Paused);
        assert_eq!(store.sessions()[0].applied_configuration.len(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn reapply_requires_permission_paused_status_freshness_and_exclusive_ownership() {
        let directory = temp_directory("reapply-guards");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store.create("Game", "Guard reapply").unwrap();
        let at = crate::types::now();
        let current = snapshot(at, 60.0, &["game"]);
        store.start(&session.id, true, &current, false).unwrap();
        run_baseline(&mut store, at, 60.0, &["game"]);
        for offset in [20, 25, 30] {
            store
                .observe(&snapshot(at + offset, 45.0, &["game"]))
                .unwrap();
        }
        store.pause(&session.id).unwrap();
        let writes = adapter.writes.lock().unwrap().len();

        assert!(store
            .reapply_best(&session.id, false, &current, false)
            .is_err());
        let stale = snapshot(at.saturating_sub(60), 60.0, &["game"]);
        assert!(store
            .reapply_best(&session.id, true, &stale, false)
            .is_err());
        assert!(store
            .reapply_best(&session.id, true, &current, true)
            .is_err());
        assert_eq!(adapter.writes.lock().unwrap().len(), writes);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn interrupted_reapply_is_recovered_from_paused_state() {
        let directory = temp_directory("interrupted-reapply");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store.create("Game", "Recover reapply").unwrap();
        let at = crate::types::now();
        let current = snapshot(at, 60.0, &["game"]);
        store.start(&session.id, true, &current, false).unwrap();
        run_baseline(&mut store, at, 60.0, &["game"]);
        for offset in [20, 25, 30] {
            store
                .observe(&snapshot(at + offset, 45.0, &["game"]))
                .unwrap();
        }
        store.pause(&session.id).unwrap();
        *adapter.fail_writes.lock().unwrap() = 1;
        assert!(store
            .reapply_best(&session.id, true, &current, false)
            .is_err());
        assert!(store.sessions()[0].pending_transaction.is_some());
        drop(store);

        let reloaded = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        assert_eq!(reloaded.sessions()[0].status, Status::Paused);
        assert!(reloaded.sessions()[0].pending_transaction.is_none());
        assert!(!*adapter.value.lock().unwrap());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn sustained_drift_waits_for_cooldown_and_records_distinct_bounded_runs() {
        let directory = temp_directory("continuous-runs");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store.create("Game", "Reevaluate conservatively").unwrap();
        let at = crate::types::now();
        store
            .start(&session.id, true, &snapshot(at, 60.0, &["game"]), false)
            .unwrap();
        run_baseline(&mut store, at, 60.0, &["game"]);
        for offset in [20, 25, 30] {
            store
                .observe(&snapshot(at + offset, 45.0, &["game"]))
                .unwrap();
        }
        assert_eq!(store.sessions()[0].runs.len(), 1);

        for offset in [930, 935, 940, 945, 950, 955, 960, 965, 970] {
            store
                .observe(&snapshot(at + offset, 80.0, &["game"]))
                .unwrap();
        }
        assert_eq!(store.sessions()[0].phase, Phase::Baseline);
        assert!(!*adapter.value.lock().unwrap());
        for offset in [975, 980, 985] {
            store
                .observe(&snapshot(at + offset, 80.0, &["game"]))
                .unwrap();
        }
        for offset in [990, 995, 1000] {
            store
                .observe(&snapshot(at + offset, 60.0, &["game"]))
                .unwrap();
        }

        let runs = &store.sessions()[0].runs;
        assert_eq!(runs.len(), 2);
        assert_ne!(runs[0].id, runs[1].id);
        assert_eq!(runs[0].fingerprint_id, runs[1].fingerprint_id);
        assert!(runs.len() <= MAX_RUNS);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn baseline_workload_churn_does_not_abort_learning() {
        let directory = temp_directory("baseline-churn");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store.create("Export", "Finish a long export").unwrap();
        let at = crate::types::now();
        store
            .start(&session.id, true, &snapshot(at, 40.0, &["export"]), false)
            .unwrap();
        store
            .observe(&snapshot(at + 5, 40.0, &["export", "helper"]))
            .unwrap();
        assert_eq!(store.sessions()[0].status, Status::Monitoring);
        assert!(!store.sessions()[0].stale);
        store
            .observe(&snapshot(at + 10, 40.0, &["export", "helper"]))
            .unwrap();
        store
            .observe(&snapshot(at + 15, 40.0, &["export", "helper"]))
            .unwrap();
        store
            .observe(&snapshot(at + 20, 40.0, &["export", "helper"]))
            .unwrap();
        assert_eq!(store.sessions()[0].status, Status::Monitoring);
        assert!(store.sessions()[0].baseline.is_some());
        assert!(!store.sessions()[0].stale);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn monitoring_environment_drift_restores_only_owned_state_and_discards_evidence() {
        let directory = temp_directory("monitor-drift");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store.create("Game", "Stop on drift").unwrap();
        let at = crate::types::now();
        store
            .start(&session.id, true, &snapshot(at, 60.0, &["game"]), false)
            .unwrap();
        run_baseline(&mut store, at, 60.0, &["game"]);
        store
            .observe(&snapshot(at + 20, 45.0, &["game", "new-app"]))
            .unwrap();

        assert_eq!(*adapter.writes.lock().unwrap(), vec![true, false]);
        assert_eq!(store.sessions()[0].status, Status::Paused);
        assert!(store.sessions()[0].stale);
        assert!(store.sessions()[0].findings.is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn paused_reapply_is_restored_and_marked_stale_on_live_environment_drift() {
        let directory = temp_directory("paused-owner-drift");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store.create("Game", "Refresh compatibility").unwrap();
        let at = crate::types::now();
        let current = snapshot(at, 60.0, &["game"]);
        store.start(&session.id, true, &current, false).unwrap();
        run_baseline(&mut store, at, 60.0, &["game"]);
        for offset in [20, 25, 30] {
            store
                .observe(&snapshot(at + offset, 45.0, &["game"]))
                .unwrap();
        }
        store.pause(&session.id).unwrap();
        store
            .reapply_best(&session.id, true, &current, false)
            .unwrap();
        assert!(*adapter.value.lock().unwrap());

        store
            .observe(&snapshot(at + 35, 45.0, &["game", "updated-app"]))
            .unwrap();
        assert!(!*adapter.value.lock().unwrap());
        assert!(store.sessions()[0].stale);
        assert!(store.sessions()[0].applied_configuration.is_empty());
        assert_eq!(store.sessions()[0].status, Status::Paused);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn persistence_failure_happens_before_any_native_write() {
        let directory = temp_directory("persist-before-write");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let created = store.create("Game", "Fail closed").unwrap();
        let at = crate::types::now();
        store
            .start(&created.id, true, &snapshot(at, 60.0, &["game"]), false)
            .unwrap();
        store.observe(&snapshot(at + 5, 60.0, &["game"])).unwrap();
        store.observe(&snapshot(at + 10, 60.0, &["game"])).unwrap();
        let path = directory.join(FILE_NAME);
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();

        assert!(store.observe(&snapshot(at + 15, 60.0, &["game"])).is_err());
        assert!(adapter.writes.lock().unwrap().is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn restart_rolls_back_an_in_progress_change_and_does_not_resume() {
        let directory = temp_directory("crash-rollback");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let created = store.create("Game", "Reduce jitter").unwrap();
        let at = crate::types::now();
        store
            .start(&created.id, true, &snapshot(at, 60.0, &["game"]), false)
            .unwrap();
        run_baseline(&mut store, at, 60.0, &["game"]);
        assert!(*adapter.value.lock().unwrap());
        drop(store);

        let reloaded = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        assert!(!*adapter.value.lock().unwrap());
        assert_eq!(reloaded.sessions()[0].status, Status::Paused);
        assert!(!reloaded.monitoring());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn native_keep_awake_goal_selects_a_runtime_setting_without_writing() {
        let directory = temp_directory("native-intent");
        let mut store = Store::load(directory.clone()).unwrap();
        let session = store
            .create("Long render", "Keep awake: finish my render")
            .unwrap();
        store
            .start(
                &session.id,
                true,
                &snapshot(now(), 30.0, &["render"]),
                false,
            )
            .unwrap();
        assert_eq!(store.sessions()[0].original_configuration.len(), 1);
        assert!(!store.sessions()[0].original_configuration[0].value);
        store.pause(&session.id).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn keep_awake_intent_is_not_scored_as_cpu_minimization() {
        let directory = temp_directory("intent-evidence");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store
            .create("Render", "Keep awake: finish rendering")
            .unwrap();
        let at = now();
        store
            .start(&session.id, true, &snapshot(at, 40.0, &["render"]), false)
            .unwrap();
        run_baseline(&mut store, at, 40.0, &["render"]);
        for offset in [20, 25, 30] {
            store
                .observe(&snapshot(at + offset, 70.0, &["render"]))
                .unwrap();
        }
        let finding = &store.sessions()[0].findings[0];
        assert_eq!(serde_json::to_value(&finding.outcome).unwrap(), "goal_met");
        assert_eq!(store.sessions()[0].best_configuration.len(), 1);
        for offset in [930, 935, 940, 945, 950, 955, 960, 965, 970] {
            store
                .observe(&snapshot(at + offset, 90.0, &["render"]))
                .unwrap();
        }
        assert_eq!(store.sessions()[0].phase, Phase::Watching);
        assert_eq!(*adapter.writes.lock().unwrap(), vec![true]);
        store.pause(&session.id).unwrap();
        assert_eq!(*adapter.writes.lock().unwrap(), vec![true, false]);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn baseline_external_change_is_not_overwritten() {
        let directory = temp_directory("baseline-external");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store
            .create("Observe", "Do not overwrite external changes")
            .unwrap();
        let at = now();
        store
            .start(&session.id, true, &snapshot(at, 40.0, &["app"]), false)
            .unwrap();
        *adapter.value.lock().unwrap() = true;
        let _ = store.observe(&snapshot(at + 5, 40.0, &["app"]));
        let _ = store.observe(&snapshot(at + 10, 40.0, &["app"]));
        let _ = store.observe(&snapshot(at + 15, 40.0, &["app"]));
        assert!(adapter.writes.lock().unwrap().is_empty());
        assert_eq!(store.sessions()[0].status, Status::Paused);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn fingerprint_is_independent_of_workload_ranking() {
        let directory = temp_directory("fingerprint-ranking");
        let mut store = Store::load(directory.clone()).unwrap();
        let names: Vec<_> = (0..40).map(|index| format!("app-{index:02}")).collect();
        let refs: Vec<_> = names.iter().map(String::as_str).collect();
        let first = snapshot(now(), 30.0, &refs);
        let mut second = first.clone();
        second.workloads.reverse();
        assert_eq!(store.fingerprint(&first), store.fingerprint(&second));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_best_cannot_be_revived_by_resuming_same_fingerprint() {
        let directory = temp_directory("stale-best");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter).unwrap();
        let session = store.create("Observe", "Keep evidence honest").unwrap();
        let sample = snapshot(now(), 40.0, &["app"]);
        store.start(&session.id, true, &sample, false).unwrap();
        store.pause(&session.id).unwrap();
        let saved = store.session_mut(&session.id).unwrap();
        saved.stale = true;
        saved.best_configuration = vec![SettingState {
            setting: Setting::StageManager,
            value: true,
        }];
        store.start(&session.id, true, &sample, false).unwrap();
        assert!(store.sessions()[0].best_configuration.is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn explicit_restore_retries_pending_apply_without_restart() {
        let directory = temp_directory("retry-pending");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store.create("Observe", "Retry recovery").unwrap();
        let at = now();
        store
            .start(&session.id, true, &snapshot(at, 40.0, &["app"]), false)
            .unwrap();
        *adapter.fail_writes.lock().unwrap() = 1;
        store.observe(&snapshot(at + 5, 40.0, &["app"])).unwrap();
        store.observe(&snapshot(at + 10, 40.0, &["app"])).unwrap();
        assert!(store.observe(&snapshot(at + 15, 40.0, &["app"])).is_err());
        assert!(store.restore_original(&session.id).is_ok());
        assert!(!store.owns_native_state());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_start_persistence_does_not_leave_monitoring_enabled() {
        let directory = temp_directory("start-persist-failure");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter).unwrap();
        let session = store.create("Observe", "Start must be durable").unwrap();
        let path = directory.join(FILE_NAME);
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(store
            .start(&session.id, true, &snapshot(now(), 40.0, &["app"]), false)
            .is_err());
        assert!(!store.monitoring());
        assert!(!store.sessions()[0].opted_in);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn keep_awake_never_experiments_with_disabling_an_existing_request() {
        let directory = temp_directory("keep-awake-already-on");
        let adapter = Arc::new(FakeAdapter::default());
        *adapter.value.lock().unwrap() = true;
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store
            .create("Render", "Keep awake: finish rendering")
            .unwrap();
        let at = now();
        store
            .start(&session.id, true, &snapshot(at, 40.0, &["render"]), false)
            .unwrap();
        run_baseline(&mut store, at, 40.0, &["render"]);
        assert!(adapter.writes.lock().unwrap().is_empty());
        assert_eq!(store.sessions()[0].phase, Phase::Watching);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn startup_rejects_a_journal_that_would_enable_keep_awake() {
        let directory = temp_directory("invalid-recovery");
        let adapter = Arc::new(FakeAdapter::default());
        let mut store = Store::load_with_adapter(directory.clone(), adapter.clone()).unwrap();
        let session = store
            .create("Unsafe journal", "Keep awake: malformed persisted state")
            .unwrap();
        let saved = store.session_mut(&session.id).unwrap();
        saved.opted_in = true;
        saved.original_configuration = vec![SettingState {
            setting: Setting::KeepAwake,
            value: true,
        }];
        saved.pending_transaction = Some(PendingTransaction {
            kind: TransactionKind::Restore,
            setting: Setting::KeepAwake,
            expected_current: false,
            target: true,
            original: true,
        });
        store.persist().unwrap();
        drop(store);
        assert!(Store::load_with_adapter(directory.clone(), adapter.clone()).is_err());
        assert!(adapter.writes.lock().unwrap().is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn fingerprint_detects_changes_beyond_display_limit() {
        let directory = temp_directory("fingerprint-overflow");
        let mut store = Store::load(directory.clone()).unwrap();
        let names: Vec<_> = (0..40).map(|index| format!("app-{index:02}")).collect();
        let refs: Vec<_> = names.iter().map(String::as_str).collect();
        let first = snapshot(now(), 30.0, &refs);
        let mut second = first.clone();
        second.workloads.pop();
        assert_ne!(store.fingerprint(&first).id, store.fingerprint(&second).id);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn native_capability_smoke_is_read_only() {
        let adapter = NativeAdapter::default();
        let _ = adapter.supported(Setting::KeepAwake).unwrap();
        let stage = adapter.supported(Setting::StageManager).unwrap();
        if stage {
            let _ = adapter.read(Setting::StageManager);
        }
        let _ = adapter.read(Setting::KeepAwake);
    }
}
