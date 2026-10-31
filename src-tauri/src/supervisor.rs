//! Native incident lifecycle. It continues while the window is hidden; models cannot execute tools.
use crate::{
    actions,
    automation::{self, Case, Monitor, Origin, Policy, Rule, Stage},
    budget, details, jitter, priorities, profiles, providers, settings, settings_history, storage,
    system_settings,
    types::*,
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant},
};

type Publish = Arc<dyn Fn(Case) + Send + Sync>;
type SettingsWriter = Arc<
    dyn Fn(&std::path::Path, &settings::Settings) -> Result<(), storage::WriteError> + Send + Sync,
>;

#[derive(Default)]
struct Book {
    monitor: Monitor,
    cases: VecDeque<Case>,
    sequence: u64,
}

const RESTORE_PREVIEW_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_RESTORE_PREVIEWS: usize = 32;

struct PendingRestore {
    token: String,
    target_id: String,
    target_digest: String,
    current_default: Option<settings_history::CurrentState>,
    settings_revision: u64,
    settings_digest: String,
    expires_at: Instant,
}

#[derive(Default)]
struct RestorePreviews {
    entries: VecDeque<PendingRestore>,
}

impl RestorePreviews {
    fn issue(&mut self, pending: PendingRestore) {
        self.entries
            .retain(|entry| entry.expires_at > Instant::now());
        self.entries.push_back(pending);
        while self.entries.len() > MAX_RESTORE_PREVIEWS {
            self.entries.pop_front();
        }
    }

    fn take(&mut self, token: &str) -> Option<PendingRestore> {
        let position = self.entries.iter().position(|entry| entry.token == token)?;
        self.entries.remove(position)
    }

    fn cancel(&mut self, token: &str) -> bool {
        self.take(token).is_some()
    }
}

/// Agent requests kept within the bounded incident history per hour.
const AGENT_REQUESTS_PER_HOUR: usize = 6;

/// Triggers about the same workload within this window form one incident.
const MERGE_SECONDS: u64 = 300;

/// An open or recent case about the same workload.
fn related(cases: &VecDeque<Case>, workload: Option<&str>, at: u64) -> Option<u64> {
    let workload = workload?;
    cases
        .iter()
        .find(|c| {
            // An agent's request is not an incident: monitoring and snapshots never merge into it.
            c.stage != Stage::Dismissed
                && c.origin != Origin::Agent
                && at.saturating_sub(c.detected_at) <= MERGE_SECONDS
                && c.trigger
                    .target
                    .as_ref()
                    .is_some_and(|t| t.workload_id == workload)
        })
        .map(|c| c.id)
}

/// The most specific alternative for a workload: close its heaviest tab or pause the command
/// session using it, else stop a supported process. Never applied without approval.
async fn alternative(
    snapshot: &Snapshot,
    workload: &str,
    memory: bool,
) -> Option<automation::Proposal> {
    let (sample, id) = (snapshot.clone(), workload.to_string());
    tokio::task::spawn_blocking(move || crate::inside::options::option(&sample, &id, memory))
        .await
        .ok()
        .flatten()
        .or_else(|| automation::stop_option(snapshot, workload))
}

pub struct Runtime {
    pub snapshot: Arc<RwLock<Snapshot>>,
    pub settings: Mutex<settings::Settings>,
    pub engine: providers::Engine,
    pub inference: tokio::sync::Mutex<()>,
    pub icons: crate::applications::Icons,
    pub profiles: Mutex<crate::profiles::Cache>,
    pub task_summaries: Mutex<crate::profiles::Cache>,
    action: tokio::sync::Mutex<()>,
    settings_revision: AtomicU64,
    settings_defaults: Mutex<()>,
    system_settings: system_settings::Controller,
    restore_previews: Mutex<RestorePreviews>,
    settings_writer: SettingsWriter,
    pub directory: PathBuf,
    book: Mutex<Book>,
    priorities: Mutex<priorities::Ledger>,
    publish: Publish,
}

/// What is lowered now, and the workloads with a saved "always lower" permission.
#[derive(serde::Serialize)]
pub struct PriorityChanges {
    pub changes: Vec<priorities::View>,
    pub rules: Vec<Rule>,
}

impl Runtime {
    pub fn new(directory: PathBuf, settings: settings::Settings, publish: Publish) -> Arc<Self> {
        Self::new_with_settings_writer(
            directory,
            settings,
            publish,
            Arc::new(storage::write_detailed),
        )
    }

    fn new_with_settings_writer(
        directory: PathBuf,
        settings: settings::Settings,
        publish: Publish,
        settings_writer: SettingsWriter,
    ) -> Arc<Self> {
        Self::new_with_components(
            directory,
            settings,
            publish,
            settings_writer,
            system_settings::native_adapter(),
        )
    }

    #[cfg(test)]
    fn new_with_system_native(
        directory: PathBuf,
        settings: settings::Settings,
        publish: Publish,
        native: Arc<dyn system_settings::NativeSettings>,
    ) -> Arc<Self> {
        Self::new_with_components(
            directory,
            settings,
            publish,
            Arc::new(storage::write_detailed),
            native,
        )
    }

    fn new_with_components(
        directory: PathBuf,
        settings: settings::Settings,
        publish: Publish,
        settings_writer: SettingsWriter,
        native: Arc<dyn system_settings::NativeSettings>,
    ) -> Arc<Self> {
        Arc::new(Self {
            snapshot: Arc::new(RwLock::new(Snapshot::default())),
            settings: Mutex::new(settings),
            engine: providers::Engine::new(&directory),
            inference: tokio::sync::Mutex::new(()),
            icons: crate::applications::Icons::default(),
            profiles: Mutex::new(crate::profiles::Cache::read(&directory)),
            task_summaries: Mutex::new(crate::profiles::Cache::default()),
            action: tokio::sync::Mutex::new(()),
            settings_revision: AtomicU64::new(0),
            settings_defaults: Mutex::new(()),
            system_settings: system_settings::Controller::new(directory.clone(), native),
            restore_previews: Mutex::new(RestorePreviews::default()),
            settings_writer,
            directory,
            book: Mutex::new(Book::default()),
            priorities: Mutex::new(priorities::Ledger::default()),
            publish,
        })
    }

    fn persist_settings(
        &self,
        current: &mut settings::Settings,
        next: &settings::Settings,
    ) -> Result<(), String> {
        let path = self.directory.join("settings.json");
        match (self.settings_writer)(&path, next) {
            Ok(()) => {
                // Reconcile the actual persisted file, not just the requested values.
                let effective: settings::Settings = storage::read(&path)?;
                effective.validate()?;
                *current = effective;
                self.settings_revision.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
            Err(error) if error.committed() => {
                let original = error.into_message();
                let effective: settings::Settings = storage::read(&path).map_err(|reconcile| {
                    format!("{original} Runtime reconciliation also failed: {reconcile}")
                })?;
                effective.validate().map_err(|reconcile| {
                    format!("{original} Runtime reconciliation also failed: {reconcile}")
                })?;
                *current = effective;
                self.settings_revision.fetch_add(1, Ordering::SeqCst);
                Err(original)
            }
            Err(error) => Err(error.into_message()),
        }
    }
    /// One explicit detail-view request; cached across restarts, never run by the sampler.
    pub async fn explain_application(
        &self,
        workload_id: &str,
        pid: u32,
        start_time: u64,
    ) -> Result<profiles::Profile, String> {
        let snapshot = self
            .snapshot
            .read()
            .map_err(|_| "Telemetry unavailable.")?
            .clone();
        let detail = details::inspect(&snapshot, workload_id, Some((pid, start_time)))?;
        if detail.profile.source == "reference" {
            return Ok(detail.profile);
        }
        if !detail.can_generate_profile {
            return Err("The OS did not expose an executable identity for this process. Its observed parent chain is still available below.".into());
        }
        let app = detail
            .application
            .ok_or("Application identity unavailable.")?;
        let key = profiles::key(&app);
        if let Some(profile) = self
            .profiles
            .lock()
            .map_err(|_| "Profile cache unavailable.")?
            .get(&key)
        {
            return Ok(profile);
        }
        let _guard = self
            .inference
            .try_lock()
            .map_err(|_| "One model request is already active. Please wait.")?;
        let settings = self
            .settings
            .lock()
            .map_err(|_| "Settings unavailable.")?
            .clone();
        let profile = self.engine.app_profile(&settings, &snapshot, &app).await?;
        self.profiles
            .lock()
            .map_err(|_| "Profile cache unavailable.")?
            .insert(key, profile.clone(), &self.directory)?;
        Ok(profile)
    }
    pub fn cases(&self) -> Result<Vec<Case>, String> {
        Ok(self
            .book
            .lock()
            .map_err(|_| "Incident state unavailable.")?
            .cases
            .iter()
            .cloned()
            .collect())
    }
    pub fn notifications_allowed(&self) -> bool {
        self.policy().is_ok_and(|policy| policy.enabled)
            && self.snoozed_until().is_ok_and(|until| until <= now())
    }
    pub fn snoozed_until(&self) -> Result<u64, String> {
        Ok(self
            .book
            .lock()
            .map_err(|_| "Incident state unavailable.")?
            .monitor
            .snoozed_until)
    }
    fn case(&self, id: u64) -> Result<Case, String> {
        self.cases()?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or("This incident is no longer available.".into())
    }
    fn update(&self, id: u64, change: impl FnOnce(&mut Case)) -> Result<Case, String> {
        let changed = {
            let mut book = self
                .book
                .lock()
                .map_err(|_| "Incident state unavailable.")?;
            let case = book
                .cases
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or("Incident unavailable.")?;
            // A dismissed investigation may finish its HTTP request, but cannot revive itself.
            if case.stage == Stage::Dismissed {
                return Ok(case.clone());
            }
            change(case);
            case.clone()
        };
        (self.publish)(changed.clone());
        Ok(changed)
    }
    fn snapshot(&self) -> Result<Snapshot, String> {
        let s = self
            .snapshot
            .read()
            .map_err(|_| "Telemetry unavailable.")?
            .clone();
        if s.source != "live" || !s.sample_ready || s.captured_at.saturating_add(20) < now() {
            return Err("Fresh telemetry is required; no action was taken.".into());
        }
        Ok(s)
    }
    fn policy(&self) -> Result<Policy, String> {
        Ok(self
            .settings
            .lock()
            .map_err(|_| "Settings unavailable.")?
            .automation
            .clone())
    }
    fn enabled(&self, id: u64) -> Result<bool, String> {
        let case = self.case(id)?;
        // A manual snapshot is an explicit request: it runs even when automation is off or paused.
        if case.origin == Origin::Snapshot {
            return Ok(case.stage != Stage::Dismissed);
        }
        Ok(self.policy()?.enabled
            && self.snoozed_until()? <= now()
            && case.stage != Stage::Dismissed)
    }
    pub fn observe(&self, sample: &Snapshot) -> Result<Option<u64>, String> {
        *self
            .snapshot
            .write()
            .map_err(|_| "Telemetry unavailable.")? = sample.clone();
        if sample.source != "live" {
            return Ok(None);
        }
        let policy = self.policy()?;
        // Keep a finished memory warning from lingering after the machine recovers.
        // Running investigations already recheck pressure before and after inference.
        for case in self.cases()? {
            let recovered = case.trigger.kind == automation::Kind::MemoryPressure
                || (case.trigger.kind == automation::Kind::AppUnresponsive
                    && case
                        .trigger
                        .target
                        .as_ref()
                        .and_then(|t| sample.workloads.iter().find(|w| w.id == t.workload_id))
                        .is_some_and(|w| w.unresponsive == Some(false)));
            if recovered
                // An unanswered stop offer is withdrawn too; nothing has been applied yet.
                && (matches!(case.stage, Stage::Completed | Stage::Blocked)
                    || (case.stage == Stage::AwaitingApproval && case.proposal.is_none()))
                && sample.sample_ready
                && sample.captured_at.saturating_add(20) >= now()
                && !automation::still_present(sample, &policy, &case.trigger)
            {
                self.update(case.id, |c| {
                    c.transition(
                        Stage::Resolved,
                        "The observed problem has cleared. No process changes were needed.",
                    );
                    c.alternative = None;
                })?;
            }
        }
        let case = {
            let mut book = self
                .book
                .lock()
                .map_err(|_| "Incident state unavailable.")?;
            let busy = book.cases.iter().any(|c| c.stage.running());
            let Some(trigger) = book.monitor.observe(sample, &policy, busy) else {
                return Ok(None);
            };
            let workload = trigger.target.as_ref().map(|t| t.workload_id.as_str());
            if let Some(id) = related(&book.cases, workload, sample.captured_at) {
                // Same workload, same incident: add to the existing card instead of a second one.
                let Some(case) = book.cases.iter_mut().find(|c| c.id == id) else {
                    return Ok(None);
                };
                case.note(&format!(
                    "Automatic monitoring also flagged this: {}",
                    trigger.reason
                ));
                let changed = case.clone();
                drop(book);
                (self.publish)(changed);
                return Ok(None);
            }
            book.sequence += 1;
            let id = now().saturating_mul(1000).saturating_add(book.sequence);
            let case = Case::new(id, sample, trigger);
            book.cases.push_front(case.clone());
            book.cases.truncate(12);
            case
        };
        (self.publish)(case.clone());
        Ok(Some(case.id))
    }
    /// Open a case for a user's Jitter Snapshot. It bypasses the monitor's cooldown and hourly
    /// cap because the user asked, but inference, mode, and spending limits still apply.
    /// When a recent case already covers the culprit, the snapshot joins that case instead.
    /// Returns the case and whether it needs a (new) investigation run.
    pub fn report(
        &self,
        capture: jitter::Capture,
        tally: &mut jitter::Tally,
    ) -> Result<(u64, bool), String> {
        let report = {
            let snapshot = self.snapshot.read().map_err(|_| "Telemetry unavailable.")?;
            if snapshot.source != "live" || !snapshot.sample_ready {
                return Err("Wait for live telemetry before taking a Jitter Snapshot.".into());
            }
            let mut report = jitter::analyze(&capture, &snapshot);
            tally.record(report.implicated().as_deref(), now());
            // The tally is a convenience; failing to persist it never loses the snapshot.
            let _ = tally.save(&self.directory);
            report.apply_recurrence(tally);
            report
        };
        let culprit = report.culprit.as_ref().and_then(|c| c.workload_id.clone());
        let merged = {
            let mut book = self
                .book
                .lock()
                .map_err(|_| "Incident state unavailable.")?;
            related(&book.cases, culprit.as_deref(), now()).and_then(|id| {
                let case = book.cases.iter_mut().find(|c| c.id == id)?;
                case.jitter = Some(report.clone());
                case.origin = Origin::Snapshot;
                // A finished case is analyzed again with your snapshot; a live one keeps going.
                let rerun = matches!(
                    case.stage,
                    Stage::Completed | Stage::Resolved | Stage::Blocked
                );
                if rerun {
                    case.proposal = None;
                    case.transition(
                        Stage::Detected,
                        "You took a Jitter Snapshot of this incident; analyzing it next.",
                    );
                } else {
                    case.note("You took a Jitter Snapshot; it was added to this incident.");
                }
                Some((case.clone(), rerun))
            })
        };
        if let Some((case, rerun)) = merged {
            (self.publish)(case.clone());
            return Ok((case.id, rerun));
        }
        Ok((self.open_capture_case(report, Origin::Snapshot)?.id, true))
    }
    /// Attach the automatic snapshot taken when the monitor opened this case.
    pub fn attach_snapshot(&self, id: u64, capture: jitter::Capture) -> Result<(), String> {
        let report = {
            let snapshot = self.snapshot.read().map_err(|_| "Telemetry unavailable.")?;
            jitter::analyze(&capture, &snapshot)
        };
        self.update(id, |c| {
            c.jitter = Some(report);
            c.note("Captured a snapshot of the surrounding one-second samples.");
        })?;
        Ok(())
    }
    /// Open a case for an Auto-mode short spike, under the monitor's automatic limits. Spikes
    /// without a measured finding are dropped silently.
    pub fn spike(&self, capture: jitter::Capture) -> Result<Option<u64>, String> {
        if !self.policy()?.auto_snapshots {
            return Ok(None);
        }
        let report = {
            let snapshot = self.snapshot.read().map_err(|_| "Telemetry unavailable.")?;
            if snapshot.source != "live" || !snapshot.sample_ready {
                return Ok(None);
            }
            jitter::analyze(&capture, &snapshot)
        };
        if report.findings.iter().all(|f| {
            matches!(
                f.kind,
                jitter::FindingKind::NothingFound | jitter::FindingKind::SchedulerLag
            )
        }) {
            return Ok(None);
        }
        let key = format!("spike:{:?}", report.findings[0].kind);
        let policy = self.policy()?;
        let culprit = report.culprit.as_ref().and_then(|c| c.workload_id.clone());
        {
            let mut book = self
                .book
                .lock()
                .map_err(|_| "Incident state unavailable.")?;
            if related(&book.cases, culprit.as_deref(), now()).is_some() {
                return Ok(None);
            }
            let busy = book.cases.iter().any(|c| c.stage.running());
            if !book.monitor.admit(&key, now(), &policy, busy) {
                return Ok(None);
            }
        }
        Ok(Some(self.open_capture_case(report, Origin::Spike)?.id))
    }
    fn open_capture_case(&self, report: jitter::Report, origin: Origin) -> Result<Case, String> {
        let snapshot = self
            .snapshot
            .read()
            .map_err(|_| "Telemetry unavailable.")?
            .clone();
        let target = report.culprit.as_ref().and_then(|c| {
            Some(crate::diagnostics::Target {
                workload_id: c.workload_id.clone()?,
                label: c.label.clone(),
                pid: None,
            })
        });
        let snapshot_case = origin == Origin::Snapshot;
        let trigger = automation::Trigger {
            key: format!(
                "{}:{}",
                if snapshot_case { "snapshot" } else { "spike" },
                report.press_at_ms
            ),
            kind: if snapshot_case {
                automation::Kind::UserReported
            } else {
                automation::Kind::ShortSpike
            },
            title: if snapshot_case {
                "Jitter Snapshot".into()
            } else {
                "Short hitch detected".into()
            },
            reason: report.happened.clone(),
            target,
            cpu_floor_percent: None,
            memory_floor_bytes: None,
        };
        let case = {
            let mut book = self
                .book
                .lock()
                .map_err(|_| "Incident state unavailable.")?;
            book.sequence += 1;
            let id = now().saturating_mul(1000).saturating_add(book.sequence);
            let mut case = Case::new(id, &snapshot, trigger);
            case.origin = origin;
            case.detected_at = report.press_at_ms / 1000;
            if let Some(step) = case.steps.first_mut() {
                step.message = if snapshot_case {
                    "Captured. The surrounding one-second samples were analyzed with local rules; analyzing the findings next.".into()
                } else {
                    "A sustained short hitch was detected in the one-second samples and analyzed with local rules.".into()
                };
            }
            case.jitter = Some(report);
            book.cases.push_front(case.clone());
            book.cases.truncate(12);
            case
        };
        (self.publish)(case.clone());
        Ok(case)
    }
    pub fn save_policy(&self, policy: Policy) -> Result<(), String> {
        policy.validate()?;
        let enabled = policy.enabled;
        {
            let mut settings = self.settings.lock().map_err(|_| "Settings unavailable.")?;
            let mut next = settings.clone();
            next.automation = policy;
            storage::write(&self.directory.join("settings.json"), &next)?;
            *settings = next;
        }
        if !enabled {
            self.cancel_pending(
                "Automatic assistance was disabled. No further action will be started.",
            )?;
        }
        Ok(())
    }
    pub fn snooze(&self, minutes: u64) -> Result<(), String> {
        if ![0, 5, 15, 60].contains(&minutes) {
            return Err("Choose a supported pause duration.".into());
        }
        self.book
            .lock()
            .map_err(|_| "Incident state unavailable.")?
            .monitor
            .snoozed_until = now() + minutes * 60;
        if minutes > 0 {
            self.cancel_pending("Automatic assistance was paused. Any already-applied priority change remains in effect.")?;
        }
        Ok(())
    }
    fn cancel_pending(&self, message: &str) -> Result<(), String> {
        for case in self.cases()? {
            if matches!(
                case.stage,
                Stage::Detected | Stage::Investigating | Stage::AwaitingApproval
            ) {
                self.update(case.id, |c| c.transition(Stage::Dismissed, message))?;
            }
        }
        Ok(())
    }
    pub fn dismiss(&self, id: u64) -> Result<(), String> {
        let mut book = self
            .book
            .lock()
            .map_err(|_| "Incident state unavailable.")?;
        let case = book
            .cases
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or("Incident unavailable.")?;
        if matches!(case.stage, Stage::Applying | Stage::Verifying) {
            return Err("The action is already in progress. Wait for its measurement.".into());
        }
        case.transition(
            Stage::Dismissed,
            "Dismissed. No further action will be taken for this incident.",
        );
        let changed = case.clone();
        drop(book);
        (self.publish)(changed);
        Ok(())
    }
    pub async fn run(self: &Arc<Self>, id: u64) {
        let capture = self.case(id).is_ok_and(|c| c.origin != Origin::Automatic);
        let result = if capture {
            self.investigate_capture(id).await
        } else {
            self.investigate(id).await
        };
        if let Err(error) = result {
            let _ = self.update(id, |case| case.transition(Stage::Blocked, &error));
        }
    }
    async fn investigate(self: &Arc<Self>, id: u64) -> Result<(), String> {
        if !self.enabled(id)? {
            return Ok(());
        }
        let claimed = {
            let mut book = self
                .book
                .lock()
                .map_err(|_| "Incident state unavailable.")?;
            let case = book
                .cases
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or("Incident unavailable.")?;
            if case.stage != Stage::Detected {
                return Ok(());
            }
            case.transition(Stage::Investigating, "Collecting evidence and waiting for the intelligent tier. One inference request runs at a time.");
            case.clone()
        };
        (self.publish)(claimed);
        {
            let _guard = tokio::time::timeout(Duration::from_secs(50), self.inference.lock())
                .await
                .map_err(|_| "The model was busy. No retry or action was started.")?;
            if !self.enabled(id)? {
                return Ok(());
            }
            let snapshot = self.snapshot()?;
            let case = self.case(id)?;
            let settings = self
                .settings
                .lock()
                .map_err(|_| "Settings unavailable.")?
                .clone();
            if !automation::still_present(&snapshot, &settings.automation, &case.trigger) {
                self.update(id, |c| c.transition(Stage::Resolved, "The trigger cleared before investigation. No model call or action was needed."))?;
                return Ok(());
            }
            let target = case.trigger.target.as_ref().map(|t| t.workload_id.as_str());
            let question = if target.is_some() {
                format!("Investigate this selected workload after an automatic resource alert. Evidence: {} Explain likely causes and whether this could be expected work. Suggest one useful next step. No action has been applied yet.", case.trigger.reason)
            } else {
                "Why is my computer struggling? Investigate this automatic resource alert and explain one useful next step. No action has been applied yet.".into()
            };
            let reply = self
                .engine
                .chat_with_capture(
                    &settings,
                    &snapshot,
                    vec![providers::ChatMessage {
                        role: "user".into(),
                        content: question,
                    }],
                    target,
                    if settings.automation.model_analysis {
                        "intelligent"
                    } else {
                        "rules"
                    },
                    None,
                    case.jitter.as_ref().map(|j| j.model_context()),
                )
                .await?;
            self.update(id, |c| {
                c.evidence = reply.investigation.clone().or(c.evidence.clone());
                c.diagnosis = Some(reply);
            })?;
        }
        if !self.enabled(id)? {
            return Ok(());
        }
        let snapshot = self.snapshot()?;
        let policy = self.policy()?;
        let case = self.case(id)?;
        if !automation::still_present(&snapshot, &policy, &case.trigger) {
            self.update(id, |c| c.transition(Stage::Resolved, "The resource trigger cleared during investigation. No optimization was applied."))?;
            return Ok(());
        }
        // An already-lowered process gets no repeated change.
        let proposal = automation::proposal(&snapshot, &case.trigger).filter(|p| {
            !actions::priority(p.process.pid, p.process.start_time).is_ok_and(|n| n >= 10)
        });
        let memory = matches!(
            case.trigger.kind,
            automation::Kind::MemoryPressure | automation::Kind::MemoryGrowth
        );
        let alternative = match (&case.trigger.target, &case.trigger.kind) {
            (Some(t), automation::Kind::AppUnresponsive) => {
                automation::stop_option(&snapshot, &t.workload_id)
            }
            (Some(t), _) => alternative(&snapshot, &t.workload_id, memory).await,
            (None, _) => None,
        };
        if proposal.is_none() && alternative.is_none() {
            self.update(id, |c| c.transition(Stage::Completed, "Investigation complete. No supported change fits this incident. Review the diagnosis; protected processes and app helpers need a different intervention."))?;
            return Ok(());
        }
        let automatic = proposal.as_ref().is_some_and(|p| policy.allows(p));
        self.update(id, |c| { c.proposal = proposal; c.alternative = alternative; c.transition(Stage::AwaitingApproval,
            if automatic { "This workload has permission for automatic priority changes. Rechecking live evidence before acting." }
            else { "A change is available. Approve it once, or allow future automatic priority changes for this workload." }); })?;
        if automatic {
            self.apply(id, false, false).await?;
        }
        Ok(())
    }
    /// Snapshot and spike cases describe a past moment: no trigger recheck, and they always stop
    /// at approval. The model explains the deterministic findings and cannot add actions.
    async fn investigate_capture(self: &Arc<Self>, id: u64) -> Result<(), String> {
        if !self.enabled(id)? {
            return Ok(());
        }
        let claimed = {
            let mut book = self
                .book
                .lock()
                .map_err(|_| "Incident state unavailable.")?;
            let case = book
                .cases
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or("Incident unavailable.")?;
            if case.stage != Stage::Detected {
                return Ok(());
            }
            case.transition(Stage::Investigating, "Asking the intelligent tier to explain the findings. One inference request runs at a time.");
            case.clone()
        };
        (self.publish)(claimed);
        let report = self
            .case(id)?
            .jitter
            .ok_or("Snapshot evidence unavailable.")?;
        {
            let _guard = tokio::time::timeout(Duration::from_secs(50), self.inference.lock())
                .await
                .map_err(|_| {
                    "The model was busy. The local findings are still shown; no action was started."
                })?;
            if !self.enabled(id)? {
                return Ok(());
            }
            let snapshot = self.snapshot()?;
            let settings = self
                .settings
                .lock()
                .map_err(|_| "Settings unavailable.")?
                .clone();
            let case = self.case(id)?;
            let target = case.trigger.target.as_ref().map(|t| t.workload_id.as_str());
            // A command inside an app is not a workload of its own: investigate its busiest
            // live process within the app's workload, or the whole machine once it exited.
            let (target, process) = match target {
                Some(t) if !snapshot.workloads.iter().any(|w| w.id == t) => {
                    jitter::members(&snapshot, t)
                        .into_iter()
                        .max_by(|a, b| a.cpu_percent.total_cmp(&b.cpu_percent))
                        .map_or((None, None), |p| {
                            (Some(p.workload_id.as_str()), Some((p.pid, p.start_time)))
                        })
                }
                t => (t, None),
            };
            let question = if target.is_some() {
                "Investigate this selected workload around a Jitter Snapshot. Explain the supplied jitter_capture findings in plain language: what happened, the likely cause, and what the user could try. Keep the frontmost app (victim) separate from culprits. No action has been applied."
            } else {
                "Explain this Jitter Snapshot of my computer using the supplied jitter_capture findings: what happened, the likely cause, and what the user could try. No action has been applied."
            };
            // Manual snapshots always ask; automatic spikes respect the model-analysis setting.
            let tier = if case.origin == Origin::Snapshot || settings.automation.model_analysis {
                "intelligent"
            } else {
                "rules"
            };
            let mut reply = self
                .engine
                .chat_with_capture(
                    &settings,
                    &snapshot,
                    vec![providers::ChatMessage {
                        role: "user".into(),
                        content: question.into(),
                    }],
                    target,
                    tier,
                    process,
                    Some(report.model_context()),
                )
                .await?;
            if reply.source != "model" {
                reply.text = report.text();
            }
            self.update(id, |c| {
                c.evidence = reply.investigation.clone().or(c.evidence.clone());
                c.diagnosis = Some(reply);
            })?;
        }
        if !self.enabled(id)? {
            return Ok(());
        }
        let snapshot = self.snapshot()?;
        let proposal = report.in_app.as_ref().filter(|s| s.action == "lower_priority").and_then(|s| {
            // Re-resolve against fresh identity; the suggestion itself never authorizes anything.
            let process = snapshot.processes.iter().find(|p| {
                p.pid == s.process.pid
                    && p.start_time == s.process.start_time
                    && p.workload_id == s.process.workload_id
                    && p.actions.iter().any(|a| a == "lower_priority")
            })?;
            Some(automation::Proposal {
                impact: automation::impact(&snapshot, process),
                process: process.clone(),
                action: "lower_priority".into(),
                rationale: "Give the app you were using scheduling preference over this workload the next time they compete. This does not free memory, and a lower priority stays until the process exits.".into(),
                unit: None,
            })
        });
        // An already-lowered culprit gets no repeated change.
        let proposal = proposal.filter(|p| {
            !actions::priority(p.process.pid, p.process.start_time).is_ok_and(|n| n >= 10)
        });
        let culprit = report.culprit.as_ref().and_then(|c| c.workload_id.clone());
        let memory = report
            .findings
            .iter()
            .any(|f| f.kind == jitter::FindingKind::MemoryThrash && f.workload_id == culprit);
        let alternative = match &culprit {
            Some(workload) => alternative(&snapshot, workload, memory).await,
            None => None,
        };
        if proposal.is_none() && alternative.is_none() {
            self.update(id, |c| c.transition(Stage::Completed, "Analysis complete. No in-app change fits these findings; review the suggestions."))?;
            return Ok(());
        }
        // Never auto-applied, even when this workload has a saved permission.
        self.update(id, |c| {
            c.proposal = proposal;
            c.alternative = alternative;
            c.transition(
                Stage::AwaitingApproval,
                "A change is available for the culprit. It needs your approval.",
            );
        })?;
        Ok(())
    }
    /// Approval is for the stored proposal, never an arbitrary command supplied by a model or UI.
    pub async fn apply(
        self: &Arc<Self>,
        id: u64,
        approved: bool,
        remember: bool,
    ) -> Result<(), String> {
        let _guard = self
            .action
            .try_lock()
            .map_err(|_| "Another action is being measured; wait before applying this proposal.")?;
        let case = self.case(id)?;
        if case.source != "live" || case.stage != Stage::AwaitingApproval {
            return Err("This incident has no pending live action.".into());
        }
        if !self.enabled(id)? {
            return Err("Automatic assistance is paused or disabled.".into());
        }
        let proposal = case
            .proposal
            .ok_or("No supported optimization was proposed.")?;
        let before = self.snapshot()?;
        if !before.processes.iter().any(|p| {
            p.pid == proposal.process.pid
                && p.start_time == proposal.process.start_time
                && p.workload_id == proposal.process.workload_id
        }) {
            self.update(id, |c| c.transition(Stage::Resolved, "The proposed process exited or changed identity. No permission or action was applied."))?;
            return Ok(());
        }
        let policy = self.policy()?;
        let past = case.origin != Origin::Automatic;
        if !past && !automation::still_present(&before, &policy, &case.trigger) {
            self.update(id, |c| {
                c.transition(
                    Stage::Resolved,
                    "The trigger cleared before approval. No action was applied.",
                )
            })?;
            return Ok(());
        }
        if !approved && (past || !policy.allows(&proposal)) {
            return Err("This workload is not allowed to change priority automatically.".into());
        }
        if remember {
            if !approved {
                return Err("Only explicit user approval can add a permission.".into());
            }
            if past && !case.jitter.as_ref().is_some_and(|j| j.offer_permission) {
                return Err(
                    "A permission is offered only after this workload recurs in snapshots.".into(),
                );
            }
            let mut settings = self.settings.lock().map_err(|_| "Settings unavailable.")?;
            if !settings
                .automation
                .rules
                .iter()
                .any(|r| r.workload_id == proposal.process.workload_id)
            {
                let mut next = settings.clone();
                next.automation.rules.push(Rule {
                    workload_id: proposal.process.workload_id.clone(),
                    label: case
                        .trigger
                        .target
                        .as_ref()
                        .map(|t| t.label.clone())
                        .unwrap_or_else(|| proposal.process.name.clone()),
                });
                next.validate()?;
                storage::write(&self.directory.join("settings.json"), &next)?;
                *settings = next;
            }
        }
        self.update(id, |c| { c.automatic = !approved; c.transition(Stage::Applying, "Rechecking identity, ownership, current telemetry, and action policy before lowering priority."); })?;
        let result = self
            .apply_and_measure(
                proposal.process.pid,
                proposal.process.start_time,
                &proposal.action,
                Some((id, !approved)),
            )
            .await;
        match result {
            Ok(report) => {
                let after = self
                    .snapshot
                    .read()
                    .map_err(|_| "Telemetry unavailable.")?
                    .clone();
                let verified = actions::priority(proposal.process.pid, proposal.process.start_time)
                    .is_ok_and(|p| p >= 10);
                let mut verification = automation::verify(&before, &after, &report, verified);
                if past {
                    // A past spike cannot be re-measured; readback and recurrence are the check.
                    verification.summary = if verified {
                        "The OS confirms the lower priority. This was a past moment, so current CPU is not a measure of success. If it recurs, take another snapshot: Jitterbug counts how often this workload is implicated."
                    } else {
                        "The priority change could not be confirmed by the OS. Review the process before trying again."
                    }
                    .into();
                }
                self.update(id, |c| { c.report = Some(report); c.verification = Some(verification); c.transition(Stage::Completed, "The action finished and follow-up evidence is available. No further changes will be made for this incident."); })?;
                Ok(())
            }
            Err(error) => {
                self.update(id, |c| c.transition(Stage::Blocked, &error))?;
                Err(error)
            }
        }
    }
    /// Approve the offered stop. Always an explicit choice: never automatic, never remembered.
    pub async fn stop(self: &Arc<Self>, id: u64) -> Result<(), String> {
        let _guard = self
            .action
            .try_lock()
            .map_err(|_| "Another action is being measured; wait before applying this proposal.")?;
        let case = self.case(id)?;
        if case.source != "live" || case.stage != Stage::AwaitingApproval {
            return Err("This incident has no pending live action.".into());
        }
        let stop = case
            .alternative
            .ok_or("No stop was offered for this incident.")?;
        if matches!(stop.action.as_str(), "pause" | "close_tab") {
            return self.granular(id, stop).await;
        }
        let before = self.snapshot()?;
        if !before.processes.iter().any(|p| {
            p.pid == stop.process.pid
                && p.start_time == stop.process.start_time
                && p.workload_id == stop.process.workload_id
        }) {
            self.update(id, |c| {
                c.transition(
                    Stage::Resolved,
                    "The process already exited or changed identity. Nothing was stopped.",
                )
            })?;
            return Ok(());
        }
        self.update(id, |c| {
            c.proposal = Some(stop.clone());
            c.transition(
                Stage::Applying,
                "Rechecking identity, ownership, and action policy before stopping.",
            );
        })?;
        match self
            .apply_and_measure(
                stop.process.pid,
                stop.process.start_time,
                "terminate",
                Some((id, false)),
            )
            .await
        {
            Ok(report) => {
                let after = self
                    .snapshot
                    .read()
                    .map_err(|_| "Telemetry unavailable.")?
                    .clone();
                let mut verification = automation::verify(&before, &after, &report, false);
                verification.summary = if report.after.is_none() {
                    "The process is gone in a fresh sample. Compare whole-machine CPU and memory before and after."
                } else {
                    "The process is still running. It may be saving work or ignoring the request; check it before trying again."
                }
                .into();
                self.update(id, |c| {
                    c.report = Some(report);
                    c.verification = Some(verification);
                    c.transition(
                        Stage::Completed,
                        "The stop request finished and follow-up evidence is available.",
                    );
                })?;
                Ok(())
            }
            Err(error) => {
                self.update(id, |c| c.transition(Stage::Blocked, &error))?;
                Err(error)
            }
        }
    }
    /// Pause a command session or close a tab, then measure with a fresh sample.
    async fn granular(
        self: &Arc<Self>,
        id: u64,
        option: automation::Proposal,
    ) -> Result<(), String> {
        let unit = option
            .unit
            .clone()
            .ok_or("The offered target is unavailable.")?;
        let before = self.snapshot()?;
        let pausing = option.action == "pause";
        self.update(id, |c| {
            c.proposal = Some(option.clone());
            c.transition(
                Stage::Applying,
                if pausing {
                    "Rechecking ownership and pausing the whole command."
                } else {
                    "Checking the tab still shows the same page, then closing it."
                },
            );
        })?;
        let (pid, start) = (option.process.pid, option.process.start_time);
        let target = unit.clone();
        let result = tokio::task::spawn_blocking(move || {
            if pausing {
                crate::pause::pause(pid, start, &target.title).map(|p| {
                    format!(
                        "Paused {} ({} processes). It resumes automatically after {} minutes.",
                        p.label,
                        p.members.len(),
                        crate::pause::LIMIT_SECONDS / 60
                    )
                })
            } else {
                crate::inside::chrome::act(&target.id, &target.detail, "close_tab")
            }
        })
        .await
        .map_err(|_| "The action could not finish.".to_string())
        .and_then(|r| r);
        let message = match result {
            Ok(message) => message,
            Err(error) => {
                self.update(id, |c| c.transition(Stage::Blocked, &error))?;
                return Err(error);
            }
        };
        let applied_at = now();
        self.update(id, |c| {
            c.transition(
                Stage::Verifying,
                "Waiting for a complete new sampling interval to measure the result.",
            )
        })?;
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if self
                .snapshot
                .read()
                .map_err(|_| "Telemetry unavailable.")?
                .captured_at
                >= applied_at + 6
            {
                break;
            }
        }
        let after = self
            .snapshot
            .read()
            .map_err(|_| "Telemetry unavailable.")?
            .clone();
        let fresh = after.captured_at >= applied_at + 6;
        let as_record = |cpu: f32, memory: u64| ProcessRecord {
            name: unit.title.clone(),
            cpu_percent: cpu,
            memory_bytes: memory,
            ..option.process.clone()
        };
        let now_unit = (pausing && fresh)
            .then(|| crate::inside::sessions::session_of(&after, pid))
            .flatten()
            .filter(|u| u.start_time == Some(start));
        let report = ActionReport {
            action: option.action.clone(),
            message,
            before: as_record(unit.cpu_percent, unit.memory_bytes),
            after: now_unit
                .as_ref()
                .map(|u| as_record(u.cpu_percent, u.memory_bytes)),
            measured_after_seconds: after.captured_at.saturating_sub(applied_at),
            fresh,
        };
        let mut verification = automation::verify(&before, &after, &report, false);
        verification.summary = match (pausing, fresh) {
            (_, false) => "The action was sent, but no fresh follow-up sample arrived yet.".into(),
            (true, true) => format!(
                "Paused. Its memory stays allocated until it resumes; macOS can compress or swap it meanwhile. Choose Resume when you want it back, or it resumes by itself in {} minutes.",
                crate::pause::LIMIT_SECONDS / 60
            ),
            (false, true) => "The tab is closed. The browser releases that renderer's memory shortly; compare available memory before and after.".into(),
        };
        self.update(id, |c| {
            c.report = Some(report);
            c.verification = Some(verification);
            c.transition(
                Stage::Completed,
                if pausing {
                    "The command is paused and follow-up evidence is available."
                } else {
                    "The tab was closed and follow-up evidence is available."
                },
            );
        })?;
        Ok(())
    }

    /// An agent asked, through the local MCP control server, for one granular action. It
    /// becomes an approval request in the popup and in the app; nothing happens until the
    /// person approves, except a pause when they allowed agents to pause without asking.
    pub async fn agent_request(
        self: &Arc<Self>,
        option: automation::Proposal,
        workload: Option<String>,
        requester: &str,
        reason: &str,
    ) -> Result<Case, String> {
        let snapshot = self.snapshot()?;
        let policy = self.policy()?;
        if !policy.agent_control {
            return Err("Agent control is turned off in Jitterbug's settings.".into());
        }
        let unit = option
            .unit
            .as_ref()
            .ok_or("The request names no tab or session.")?;
        let verb = if option.action == "pause" {
            "pause"
        } else {
            "close the tab"
        };
        // Agent text is untrusted: bounded, single-line, shown as a quotation.
        let clean = |text: &str, limit: usize| -> String {
            text.chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .take(limit)
                .collect::<String>()
                .trim()
                .to_string()
        };
        let trigger = automation::Trigger {
            key: format!("agent:{}:{}", option.action, unit.id),
            kind: automation::Kind::AgentRequest,
            title: format!(
                "{} asks to {verb} “{}”",
                clean(requester, 60),
                clean(&unit.title, 80)
            ),
            reason: format!("“{}”", clean(reason, 200)),
            target: workload.and_then(|id| {
                let w = snapshot.workloads.iter().find(|w| w.id == id)?;
                Some(crate::diagnostics::Target {
                    workload_id: w.id.clone(),
                    label: w.label.clone(),
                    pid: None,
                })
            }),
            cpu_floor_percent: None,
            memory_floor_bytes: None,
        };
        let case = {
            let mut book = self
                .book
                .lock()
                .map_err(|_| "Incident state unavailable.")?;
            // While the person has paused actions, even a reversible pause waits for them.
            let automatic = option.action == "pause"
                && policy.agent_pause_without_asking
                && book.monitor.snoozed_until <= now();
            // One open request per target, and a bounded rate, so a looping agent cannot
            // crowd real incidents out of the bounded history.
            if let Some(open) = book.cases.iter().find(|c| {
                c.trigger.key == trigger.key
                    && matches!(
                        c.stage,
                        Stage::AwaitingApproval | Stage::Applying | Stage::Verifying
                    )
            }) {
                return Ok(open.clone());
            }
            let recent = book
                .cases
                .iter()
                .filter(|c| {
                    c.origin == automation::Origin::Agent
                        && now().saturating_sub(c.detected_at) < 3600
                })
                .count();
            if recent >= AGENT_REQUESTS_PER_HOUR {
                return Err(
                    "Too many agent requests in the last hour. Ask the person directly.".into(),
                );
            }
            book.sequence += 1;
            let id = now().saturating_mul(1000).saturating_add(book.sequence);
            let mut case = Case::new(id, &snapshot, trigger);
            case.origin = automation::Origin::Agent;
            case.alternative = Some(option);
            case.transition(
                Stage::AwaitingApproval,
                if automatic {
                    "An agent asked for this pause, and agents may pause without asking."
                } else {
                    "An agent asked for this. Nothing happens until you approve it."
                },
            );
            book.cases.push_front(case.clone());
            book.cases.truncate(12);
            (case, automatic)
        };
        let (case, automatic) = case;
        (self.publish)(case.clone());
        if automatic {
            // A failure is recorded on the request, or it stays waiting for approval.
            let _ = self.stop(case.id).await;
        }
        self.case(case.id)
    }

    /// Resume a session for an agent: only one an agent's request paused, never one the
    /// person paused or an incident offered.
    pub fn agent_resume(&self, pid: u32, start_time: u64) -> Result<String, String> {
        // The current pause must be the one the request made: the person may have resumed it
        // and paused the same session again later.
        let paused_at = crate::pause::list()
            .into_iter()
            .find(|p| p.pid == pid && p.start_time == start_time)
            .map(|p| p.paused_at);
        let requested = self.cases()?.iter().any(|c| {
            c.origin == Origin::Agent
                && c.proposal.as_ref().is_some_and(|p| {
                    p.action == "pause"
                        && p.process.pid == pid
                        && p.process.start_time == start_time
                })
                && paused_at.is_none_or(|at| {
                    (c.detected_at..=c.steps.last().map_or(c.detected_at, |s| s.at)).contains(&at)
                })
        });
        if !requested {
            return Err(
                "Only sessions paused at an agent's request can be resumed by an agent. Ask the person to resume this one in Jitterbug.".into(),
            );
        }
        crate::pause::resume(pid, start_time)
    }

    /// Current state of one incident, for an agent following its request.
    pub fn request(&self, id: u64) -> Result<Case, String> {
        self.case(id)
    }

    /// Resume the command an incident paused.
    pub fn resume(&self, id: u64) -> Result<String, String> {
        let case = self.case(id)?;
        let paused = case
            .proposal
            .filter(|p| p.action == "pause")
            .ok_or("This incident paused nothing.")?;
        let message = crate::pause::resume(paused.process.pid, paused.process.start_time)?;
        self.update(id, |c| {
            c.steps.push(automation::Step {
                at: now(),
                stage: c.stage.clone(),
                message: message.clone(),
            })
        })?;
        Ok(message)
    }

    pub async fn process_action(
        self: &Arc<Self>,
        pid: u32,
        start_time: u64,
        action: &str,
    ) -> Result<ActionReport, String> {
        let _guard = self
            .action
            .try_lock()
            .map_err(|_| "Another action is being measured. Please wait.")?;
        self.apply_and_measure(pid, start_time, action, None).await
    }
    async fn apply_and_measure(
        self: &Arc<Self>,
        pid: u32,
        start_time: u64,
        action: &str,
        incident: Option<(u64, bool)>,
    ) -> Result<ActionReport, String> {
        let before = self
            .snapshot()?
            .processes
            .iter()
            .find(|p| p.pid == pid && p.start_time == start_time)
            .cloned()
            .ok_or("The process is no longer in the inventory.")?;
        let runtime = self.clone();
        let operation = action.to_string();
        let (message, lowered, source) = tokio::task::spawn_blocking(move || {
            // Serialize policy changes with the last authorization check and the OS action.
            let settings = runtime.settings.lock().map_err(|_| "Settings unavailable.")?;
            let book = runtime.book.lock().map_err(|_| "Incident state unavailable.")?;
            let mut source = priorities::Source::Approval;
            if let Some((id, automatic)) = incident {
                let case = book.cases.iter().find(|c| c.id == id).ok_or("Incident unavailable.")?;
                let proposal = case.proposal.as_ref().ok_or("Proposal unavailable.")?;
                let past = case.origin != Origin::Automatic;
                let paused = !settings.automation.enabled || book.monitor.snoozed_until > now();
                if (paused && case.origin != Origin::Snapshot) || case.stage != Stage::Applying
                    || (automatic && (past || !settings.automation.allows(proposal))) { return Err("The permission was withdrawn or automation was paused; no action was applied.".into()); }
                let snapshot = runtime.snapshot()?;
                // A past moment has no live trigger to recheck; identity and workload still must match.
                // The CPU floor is for priority: a stop may target an idle process holding memory.
                let busy_enough = |p: &ProcessRecord| past || operation == "terminate" || p.cpu_percent * snapshot.logical_cpus.max(1) as f32 >= 10.0;
                if (!past && !automation::still_present(&snapshot, &settings.automation, &case.trigger))
                    || !snapshot.processes.iter().any(|p| p.pid == pid && p.start_time == start_time && p.workload_id == proposal.process.workload_id && busy_enough(p)) {
                    return Err("The trigger or process activity changed. No action was applied.".into());
                }
                source = if automatic {
                    priorities::Source::Rule
                } else if case.origin == Origin::Agent {
                    priorities::Source::Agent
                } else {
                    priorities::Source::Approval
                };
            }
            actions::execute(pid, start_time, &operation)
                .map(|(message, lowered)| (message, lowered, source))
        }).await.map_err(|_| "The process action could not finish.")??;
        let applied_at = now();
        if let Some(lowered) = lowered {
            self.record_priority(&before, lowered, source, incident.map(|(id, _)| id))?;
        }
        if let Some((id, _)) = incident {
            let sent = if action == "terminate" {
                "Stop request sent."
            } else {
                "Priority change sent."
            };
            self.update(id, |c| c.transition(Stage::Verifying, &format!("{sent} Waiting for a complete new sampling interval to measure the result.")))?;
        }
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if self
                .snapshot
                .read()
                .map_err(|_| "Telemetry unavailable.")?
                .captured_at
                >= applied_at + 6
            {
                break;
            }
        }
        let after = self.snapshot.read().map_err(|_| "Telemetry unavailable.")?;
        let fresh = after.captured_at >= applied_at + 6;
        Ok(ActionReport {
            action: action.into(),
            message,
            before,
            after: if fresh {
                after
                    .processes
                    .iter()
                    .find(|p| p.pid == pid && p.start_time == start_time)
                    .cloned()
            } else {
                None
            },
            measured_after_seconds: after.captured_at.saturating_sub(applied_at),
            fresh,
        })
    }
    fn record_priority(
        &self,
        process: &ProcessRecord,
        lowered: actions::Lowered,
        source: priorities::Source,
        case: Option<u64>,
    ) -> Result<(), String> {
        let label = self
            .snapshot
            .read()
            .map_err(|_| "Telemetry unavailable.")?
            .workloads
            .iter()
            .find(|w| w.id == process.workload_id)
            .map_or_else(|| process.name.clone(), |w| w.label.clone());
        self.priorities
            .lock()
            .map_err(|_| "Priority history unavailable.")?
            .record(priorities::Change {
                id: 0,
                pid: process.pid,
                start_time: process.start_time,
                workload_id: process.workload_id.clone(),
                label,
                process: process.name.clone(),
                previous: lowered.previous,
                lowered_to: lowered.now,
                at: now(),
                source,
                case,
            });
        Ok(())
    }

    /// The change an incident made, while it is still listed, so its result can offer Undo.
    pub fn priority_change_for(&self, case: u64) -> Option<u64> {
        self.priorities.lock().ok()?.for_case(case)
    }

    /// Lowered processes, newest first. Ones that exited or are back to normal are dropped.
    pub fn priority_changes(&self) -> Result<PriorityChanges, String> {
        let entries = self
            .priorities
            .lock()
            .map_err(|_| "Priority history unavailable.")?
            .entries();
        // Reading OS priority scans processes; do it without holding the lock.
        let current: Vec<u64> = entries
            .iter()
            .filter(|c| priorities::still_lowered(c))
            .map(|c| c.id)
            .collect();
        let mut ledger = self
            .priorities
            .lock()
            .map_err(|_| "Priority history unavailable.")?;
        // Entries recorded meanwhile were not checked yet; keep them.
        let mut keep = current;
        keep.extend(
            ledger
                .entries()
                .iter()
                .filter(|c| !entries.iter().any(|e| e.id == c.id))
                .map(|c| c.id),
        );
        ledger.keep(&keep);
        Ok(PriorityChanges {
            changes: ledger.views(),
            rules: self.policy()?.rules,
        })
    }

    /// Undo one recorded change, only for the same process (PID and start time) it was made to.
    pub fn restore_priority(&self, id: u64) -> Result<String, String> {
        let change = self
            .priorities
            .lock()
            .map_err(|_| "Priority history unavailable.")?
            .get(id)
            .ok_or("This priority change is no longer listed. Its process may have exited.")?;
        let result = actions::restore(change.pid, change.start_time, change.previous);
        let mut ledger = self
            .priorities
            .lock()
            .map_err(|_| "Priority history unavailable.")?;
        match result {
            Ok(()) => {
                ledger.remove(id);
                Ok(format!("{} is back to its usual priority.", change.label))
            }
            Err(actions::RestoreError::Gone) => {
                ledger.remove(id);
                Err(format!(
                    "{} already quit or restarted, so it runs at its usual priority. Nothing was changed.",
                    change.process
                ))
            }
            Err(actions::RestoreError::Denied) => Err(format!(
                "macOS lets only an administrator raise a priority again. {} returns to normal when it restarts.",
                change.process
            )),
            Err(actions::RestoreError::Failed(message)) => Err(message),
        }
    }

    /// Forget a saved "always lower" permission. Changes it already made stay listed.
    pub fn remove_rule(&self, workload_id: &str) -> Result<(), String> {
        let mut settings = self.settings.lock().map_err(|_| "Settings unavailable.")?;
        if !settings
            .automation
            .rules
            .iter()
            .any(|r| r.workload_id == workload_id)
        {
            return Err("This app has no saved permission.".into());
        }
        let mut next = settings.clone();
        next.automation
            .rules
            .retain(|r| r.workload_id != workload_id);
        next.validate()?;
        storage::write(&self.directory.join("settings.json"), &next)?;
        *settings = next;
        Ok(())
    }

    pub fn spending(&self) -> Result<budget::Ledger, String> {
        self.engine.spending()
    }

    pub fn save_settings(&self, mut settings: settings::Settings) -> Result<(), String> {
        let _guard = self
            .inference
            .try_lock()
            .map_err(|_| "Wait for the active model request before changing policy.")?;
        let mut current = self.settings.lock().map_err(|_| "Settings unavailable.")?;
        // Automation permissions have a separate command so an older provider form cannot overwrite them.
        settings.automation = current.automation.clone();
        settings.validate()?;
        self.persist_settings(&mut current, &settings)
    }

    pub fn set_settings_default(&self) -> Result<settings_history::SnapshotView, String> {
        let _defaults = self
            .settings_defaults
            .lock()
            .map_err(|_| "Saved default state unavailable.")?;
        let settings = self
            .settings
            .lock()
            .map_err(|_| "Settings unavailable.")?
            .clone();
        settings_history::capture(&self.directory, &settings)
    }

    pub fn system_capabilities(&self) -> Vec<system_settings::Capability> {
        self.system_settings.inventory()
    }

    pub fn set_system_default(&self) -> Result<system_settings::SnapshotView, String> {
        self.set_system_default_selected(None)
    }

    pub fn set_system_default_selected(
        &self,
        groups: Option<&[String]>,
    ) -> Result<system_settings::SnapshotView, String> {
        let _action = self.action.try_lock().map_err(|_| {
            "A Jitterbug action is active. Wait for verification before capturing a baseline."
        })?;
        if self
            .book
            .lock()
            .map_err(|_| "Incident state unavailable.")?
            .cases
            .iter()
            .any(|case| case.stage.running())
        {
            return Err("Automatic assistance is active. Wait for it to finish before capturing a baseline.".into());
        }
        match groups {
            None => self.system_settings.capture(),
            Some(_) => self.system_settings.capture_selected(groups),
        }
    }

    pub fn system_history(
        &self,
        cursor: Option<&str>,
        page_size: usize,
    ) -> Result<system_settings::HistoryPage, String> {
        self.system_settings.history(cursor, page_size)
    }

    pub fn inspect_system_default(
        &self,
        id: &str,
    ) -> Result<system_settings::SnapshotView, String> {
        self.system_settings.inspect(id)
    }

    pub fn preview_system_restore(
        &self,
        id: &str,
    ) -> Result<system_settings::RestorePreview, String> {
        self.system_settings.preview(id)
    }

    pub fn cancel_system_restore(&self, token: &str) -> Result<(), String> {
        self.system_settings.cancel(token)
    }

    pub async fn confirm_system_restore(
        &self,
        token: &str,
    ) -> Result<system_settings::RollbackResult, String> {
        let _action = match self.action.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                let _ = self.system_settings.cancel(token);
                return Err(
                    "A Jitterbug action is active. Wait for its verification before rollback."
                        .into(),
                );
            }
        };
        if self
            .book
            .lock()
            .map_err(|_| "Incident state unavailable.")?
            .cases
            .iter()
            .any(|case| case.stage.running())
        {
            let _ = self.system_settings.cancel(token);
            return Err(
                "Automatic assistance is active. Wait for it to finish before rollback.".into(),
            );
        }
        self.system_settings.confirm(token)
    }

    pub fn settings_history(
        &self,
        cursor: Option<&str>,
        page_size: usize,
    ) -> Result<settings_history::HistoryPage, String> {
        let _defaults = self
            .settings_defaults
            .lock()
            .map_err(|_| "Saved default state unavailable.")?;
        settings_history::history(&self.directory, cursor, page_size)
    }

    pub fn inspect_settings_default(
        &self,
        id: &str,
    ) -> Result<settings_history::SnapshotView, String> {
        let _defaults = self
            .settings_defaults
            .lock()
            .map_err(|_| "Saved default state unavailable.")?;
        settings_history::inspect(&self.directory, id)
    }

    pub fn preview_settings_restore(
        &self,
        id: &str,
    ) -> Result<settings_history::RestorePreview, String> {
        let _defaults = self
            .settings_defaults
            .lock()
            .map_err(|_| "Saved default state unavailable.")?;
        let target = settings_history::inspect(&self.directory, id)?;
        let target_settings = target.settings.as_ref().ok_or_else(|| {
            target
                .error
                .clone()
                .unwrap_or_else(|| "Saved default unavailable.".into())
        })?;
        let current = {
            let settings = self.settings.lock().map_err(|_| "Settings unavailable.")?;
            settings_history::SupportedSettings::from_saved(&settings)
        };
        let target_digest = settings_history::content_digest(target_settings)?;
        let changes = settings_history::changes(&current, target_settings);
        let token = secure_token()?;
        let expires_at_ms = current_time_ms()?
            .saturating_add(u64::try_from(RESTORE_PREVIEW_TTL.as_millis()).unwrap_or(u64::MAX));
        let pending = PendingRestore {
            token: token.clone(),
            target_id: id.to_owned(),
            target_digest,
            current_default: settings_history::current_state(&self.directory)?,
            settings_revision: self.settings_revision.load(Ordering::SeqCst),
            settings_digest: settings_history::content_digest(&current)?,
            expires_at: Instant::now() + RESTORE_PREVIEW_TTL,
        };
        self.restore_previews
            .lock()
            .map_err(|_| "Restore preview state unavailable.")?
            .issue(pending);
        Ok(settings_history::RestorePreview {
            token,
            expires_at_ms,
            target,
            changes,
        })
    }

    pub fn cancel_settings_restore(&self, token: &str) -> Result<(), String> {
        if self
            .restore_previews
            .lock()
            .map_err(|_| "Restore preview state unavailable.")?
            .cancel(token)
        {
            Ok(())
        } else {
            Err("Restore preview is unavailable or already used.".into())
        }
    }

    pub async fn confirm_settings_restore(
        &self,
        token: &str,
    ) -> Result<settings::Settings, String> {
        // Consume first: every confirmation attempt is one-shot, including rejected stale attempts.
        let pending = self
            .restore_previews
            .lock()
            .map_err(|_| "Restore preview state unavailable.")?
            .take(token)
            .ok_or("Restore preview is unavailable or already used.")?;
        if pending.expires_at <= Instant::now() {
            return Err("Restore preview expired. Review the current values again.".into());
        }
        let _action = self.action.try_lock().map_err(|_| {
            "A process action is active. Wait for its verification before restoring settings."
        })?;
        let _inference = self
            .inference
            .try_lock()
            .map_err(|_| "A model request is active. Wait before restoring settings.")?;
        if self
            .book
            .lock()
            .map_err(|_| "Incident state unavailable.")?
            .cases
            .iter()
            .any(|case| case.stage.running())
        {
            return Err(
                "Automatic assistance is active. Wait for it to finish before restoring settings."
                    .into(),
            );
        }
        let _defaults = self
            .settings_defaults
            .lock()
            .map_err(|_| "Saved default state unavailable.")?;
        let mut current = self.settings.lock().map_err(|_| "Settings unavailable.")?;
        let current_supported = settings_history::SupportedSettings::from_saved(&current);
        let target = settings_history::load_supported(&self.directory, &pending.target_id)?;
        let stale = self.settings_revision.load(Ordering::SeqCst) != pending.settings_revision
            || settings_history::current_state(&self.directory)? != pending.current_default
            || settings_history::content_digest(&current_supported)? != pending.settings_digest
            || settings_history::content_digest(&target)? != pending.target_digest;
        if stale {
            return Err(
                "This restore preview is stale. Review the latest saved and current settings."
                    .into(),
            );
        }
        let restored = target.apply_to(&current);
        restored.validate()?;
        self.persist_settings(&mut current, &restored)?;
        let effective = current.clone();
        if settings_history::SupportedSettings::from_saved(&effective) != target {
            return Err("Persisted settings did not match the selected saved default.".into());
        }
        Ok(effective)
    }
}

fn secure_token() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "Cannot create a secure restore preview token.")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn current_time_ms() -> Result<u64, String> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "System clock is unavailable.")?
        .as_millis();
    u64::try_from(millis).map_err(|_| "System clock is out of range.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::tests::{memory_sample, sample};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    #[derive(Clone)]
    struct FakeSystemNative {
        state: Arc<Mutex<crate::system_settings::NativeState>>,
        writes: Arc<AtomicU64>,
    }
    impl crate::system_settings::NativeSettings for FakeSystemNative {
        fn read(&self) -> Result<crate::system_settings::NativeState, String> {
            Ok(self.state.lock().unwrap().clone())
        }
        fn write(
            &self,
            _expected: &crate::system_settings::NativeState,
            _setting: crate::system_settings::Setting,
            _value: crate::system_settings::SettingValue,
        ) -> Result<(), String> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    fn fake_system_native() -> (
        Arc<dyn crate::system_settings::NativeSettings>,
        Arc<AtomicU64>,
    ) {
        let writes = Arc::new(AtomicU64::new(0));
        (
            Arc::new(FakeSystemNative {
                state: Arc::new(Mutex::new(crate::system_settings::NativeState {
                    identity: crate::system_settings::NativeIdentity {
                        platform: "macos".into(),
                        os_version: "15.7.1".into(),
                        user: "user".into(),
                        machine: "machine".into(),
                        device: "device".into(),
                    },
                    values: crate::system_settings::SystemValues {
                        preferences: Default::default(),
                        hardware: Default::default(),
                        output_volume: Some(0.5),
                        output_muted: Some(false),
                    },
                })),
                writes: writes.clone(),
            }),
            writes,
        )
    }
    struct Fixture {
        runtime: Arc<Runtime>,
        directory: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "jitterbug-supervisor-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut settings = settings::Settings {
                budget_usd: 0.0,
                ..Default::default()
            };
            settings.automation.model_analysis = false;
            Self {
                runtime: Runtime::new(directory.clone(), settings, Arc::new(|_| {})),
                directory,
            }
        }
        fn with_settings_writer(writer: SettingsWriter) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "jitterbug-supervisor-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut settings = settings::Settings {
                budget_usd: 0.0,
                ..Default::default()
            };
            settings.automation.model_analysis = false;
            Self {
                runtime: Runtime::new_with_settings_writer(
                    directory.clone(),
                    settings,
                    Arc::new(|_| {}),
                    writer,
                ),
                directory,
            }
        }
        fn with_system_native(native: Arc<dyn crate::system_settings::NativeSettings>) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "jitterbug-supervisor-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut settings = settings::Settings {
                budget_usd: 0.0,
                ..Default::default()
            };
            settings.automation.model_analysis = false;
            Self {
                runtime: Runtime::new_with_system_native(
                    directory.clone(),
                    settings,
                    Arc::new(|_| {}),
                    native,
                ),
                directory,
            }
        }
        fn incident(&self) -> u64 {
            let at = now();
            assert!(self
                .runtime
                .observe(&sample(at - 10, 15.0))
                .unwrap()
                .is_none());
            assert!(self
                .runtime
                .observe(&sample(at - 5, 15.0))
                .unwrap()
                .is_none());
            self.runtime.observe(&sample(at, 15.0)).unwrap().unwrap()
        }
    }
    #[test]
    fn workflow_notifications_obey_disable_and_snooze() {
        let f = Fixture::new();
        assert!(f.runtime.notifications_allowed());
        f.runtime.snooze(15).unwrap();
        assert!(!f.runtime.notifications_allowed());
        let f = Fixture::new();
        f.runtime.settings.lock().unwrap().automation.enabled = false;
        assert!(!f.runtime.notifications_allowed());
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    fn agent_option(id: &str) -> automation::Proposal {
        automation::Proposal {
            process: sample(now(), 1.0).processes[0].clone(),
            action: "pause".into(),
            rationale: "Pause it.".into(),
            impact: None,
            unit: Some(crate::inside::Unit {
                id: id.into(),
                kind: "session".into(),
                title: "claude · other".into(),
                ..Default::default()
            }),
        }
    }

    /// Agent control is opt-in; these tests turn it on as the person would in Settings.
    fn allow_agents(f: &Fixture) -> Policy {
        let mut policy = f.runtime.policy().unwrap();
        policy.agent_control = true;
        f.runtime.save_policy(policy.clone()).unwrap();
        policy
    }

    #[tokio::test]
    async fn agent_requests_wait_for_approval_dedupe_and_are_bounded() {
        let f = Fixture::new();
        f.runtime.observe(&sample(now(), 1.0)).unwrap();
        // Control is off until the person turns it on: nothing is recorded.
        assert!(!Policy::default().agent_control);
        let mut policy = f.runtime.policy().unwrap();
        f.runtime.save_policy(policy.clone()).unwrap();
        assert!(f
            .runtime
            .agent_request(agent_option("a"), None, "An agent", "busy")
            .await
            .is_err());
        policy.agent_control = true;
        f.runtime.save_policy(policy).unwrap();
        let case = f
            .runtime
            .agent_request(
                agent_option("a"),
                Some("fixture".into()),
                "An agent\nin x",
                "it is \u{7}slow",
            )
            .await
            .unwrap();
        // Nothing applied: the person approves, the reason is quoted and cleaned.
        assert_eq!(case.stage, Stage::AwaitingApproval);
        assert_eq!(case.origin, automation::Origin::Agent);
        assert!(case.report.is_none());
        assert_eq!(case.trigger.reason, "“it is  slow”");
        assert!(!case.trigger.title.contains('\n'));
        assert_eq!(case.trigger.target.as_ref().unwrap().label, "Example");
        // The same request again returns the open one.
        let again = f
            .runtime
            .agent_request(agent_option("a"), None, "An agent", "busy")
            .await
            .unwrap();
        assert_eq!(again.id, case.id);
        for n in 1..AGENT_REQUESTS_PER_HOUR {
            f.runtime
                .agent_request(agent_option(&n.to_string()), None, "An agent", "busy")
                .await
                .unwrap();
        }
        let limited = f
            .runtime
            .agent_request(agent_option("over"), None, "An agent", "busy")
            .await
            .unwrap_err();
        assert!(limited.contains("Too many"));
    }
    #[tokio::test]
    async fn agent_pauses_wait_while_snoozed_and_agents_resume_only_their_own() {
        let f = Fixture::new();
        f.runtime.observe(&sample(now(), 1.0)).unwrap();
        let mut policy = allow_agents(&f);
        policy.agent_pause_without_asking = true;
        f.runtime.save_policy(policy).unwrap();
        f.runtime.snooze(15).unwrap();
        let case = f
            .runtime
            .agent_request(agent_option("a"), None, "An agent", "busy")
            .await
            .unwrap();
        assert_eq!(case.stage, Stage::AwaitingApproval);
        assert!(case.proposal.is_none());
        let process = agent_option("a").process;
        let refused = f
            .runtime
            .agent_resume(process.pid, process.start_time)
            .unwrap_err();
        assert!(refused.contains("Ask the person"));
        f.runtime
            .update(case.id, |c| c.proposal = c.alternative.clone())
            .unwrap();
        // Allowed now; nothing is actually paused in this test.
        let answer = f.runtime.agent_resume(process.pid, process.start_time);
        assert!(answer.unwrap_err().contains("not paused by Jitterbug"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn agents_cannot_resume_a_pause_the_person_made_after_their_request() {
        use std::process::{Command, Stdio};
        let mut child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let pid = child.id();
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let start = system
            .process(sysinfo::Pid::from_u32(pid))
            .unwrap()
            .start_time();
        let f = Fixture::new();
        f.runtime.observe(&sample(now(), 1.0)).unwrap();
        allow_agents(&f);
        let mut option = agent_option("old");
        option.process.pid = pid;
        option.process.start_time = start;
        let case = f
            .runtime
            .agent_request(option, None, "An agent", "busy")
            .await
            .unwrap();
        // The agent's pause finished long ago; the person paused the session again since.
        f.runtime
            .update(case.id, |c| {
                c.proposal = c.alternative.clone();
                c.detected_at = now() - 600;
                for step in &mut c.steps {
                    step.at = now() - 500;
                }
            })
            .unwrap();
        crate::pause::pause(pid, start, "person").unwrap();
        let refused = f.runtime.agent_resume(pid, start).unwrap_err();
        assert!(refused.contains("Ask the person"), "{refused}");
        assert!(crate::pause::is_paused(pid, start));
        crate::pause::resume(pid, start).unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }

    #[tokio::test]
    async fn real_incidents_are_never_merged_into_an_agent_request() {
        let f = Fixture::new();
        allow_agents(&f);
        let at = now();
        for offset in [10, 5] {
            assert!(f
                .runtime
                .observe(&sample(at - offset, 15.0))
                .unwrap()
                .is_none());
        }
        let request = f
            .runtime
            .agent_request(
                agent_option("a"),
                Some("fixture".into()),
                "An agent",
                "busy",
            )
            .await
            .unwrap();
        // Same workload within the merge window: a separate incident, investigated as usual.
        let incident = f.runtime.observe(&sample(at, 15.0)).unwrap().unwrap();
        assert_ne!(incident, request.id);
        let request = f.runtime.case(request.id).unwrap();
        assert_eq!(request.origin, automation::Origin::Agent);
        assert_eq!(request.steps.len(), 2);
    }

    #[tokio::test]
    async fn memory_growth_stays_quiet_until_sustained_pressure_needs_investigation() {
        let f = Fixture::new();
        let at = now();
        for offset in (35..=100).rev().step_by(5) {
            assert!(f
                .runtime
                .observe(&memory_sample(at - offset, "normal"))
                .unwrap()
                .is_none());
        }
        assert!(f.runtime.cases().unwrap().is_empty());
        assert_eq!(f.runtime.spending().unwrap().requests, 0);
        for offset in (5..=30).rev().step_by(5) {
            assert!(f
                .runtime
                .observe(&memory_sample(at - offset, "warning"))
                .unwrap()
                .is_none());
        }
        let id = f
            .runtime
            .observe(&memory_sample(at, "warning"))
            .unwrap()
            .unwrap();
        f.runtime.run(id).await;
        let case = f.runtime.case(id).unwrap();
        assert_eq!(case.trigger.kind, automation::Kind::MemoryPressure);
        // Priority cannot free memory: only an explicit, never-automatic stop is offered.
        assert_eq!(case.stage, Stage::AwaitingApproval);
        assert!(case.alternative.is_some());
        assert_eq!(case.diagnosis.unwrap().source, "rules");
        assert!(case.proposal.is_none());
        assert!(case.report.is_none());
        assert_eq!(f.runtime.cases().unwrap().len(), 1);
        assert_eq!(f.runtime.spending().unwrap().requests, 0);
        assert!(f
            .runtime
            .observe(&memory_sample(at + 5, "normal"))
            .unwrap()
            .is_none());
        assert_eq!(f.runtime.case(id).unwrap().stage, Stage::Resolved);
        assert!(f.runtime.case(id).unwrap().diagnosis.is_some()); // retained in Inbox
    }

    #[tokio::test]
    async fn a_memory_stop_is_not_held_to_the_priority_cpu_floor() {
        let f = Fixture::new();
        let at = now();
        for offset in (5..=30).rev().step_by(5) {
            let _ = f.runtime.observe(&memory_sample(at - offset, "warning"));
        }
        let id = f
            .runtime
            .observe(&memory_sample(at, "warning"))
            .unwrap()
            .unwrap();
        f.runtime.run(id).await;
        assert!(f.runtime.case(id).unwrap().alternative.is_some());
        // An idle process holding memory passes the approval gate. The fixture PID does not
        // exist, so the OS action itself is what refuses.
        let refused = f.runtime.stop(id).await.unwrap_err();
        assert!(!refused.contains("activity changed"), "{refused}");
    }

    #[tokio::test]
    async fn recovered_memory_pressure_skips_inference_even_if_app_memory_stays_high() {
        let f = Fixture::new();
        let at = now();
        for offset in (5..=30).rev().step_by(5) {
            assert!(f
                .runtime
                .observe(&memory_sample(at - offset, "warning"))
                .unwrap()
                .is_none());
        }
        let id = f
            .runtime
            .observe(&memory_sample(at, "warning"))
            .unwrap()
            .unwrap();
        *f.runtime.snapshot.write().unwrap() = memory_sample(at, "normal");
        f.runtime.run(id).await;
        let case = f.runtime.case(id).unwrap();
        assert_eq!(case.stage, Stage::Resolved);
        assert!(case.diagnosis.is_none());
        assert!(case.proposal.is_none());
        assert!(case.report.is_none());
        assert_eq!(f.runtime.spending().unwrap().requests, 0);
    }

    #[tokio::test]
    async fn process_profiles_recheck_identity_and_cached_reads_need_no_model_slot() {
        let f = Fixture::new();
        let mut snapshot = sample(now(), 0.0);
        snapshot.processes[0].executable = Some("/example/bin/service".into());
        let process = snapshot.processes[0].clone();
        let app = crate::applications::identify(process.executable.as_ref().unwrap());
        *f.runtime.snapshot.write().unwrap() = snapshot;
        f.runtime.profiles.lock().unwrap().insert_memory(
            profiles::key(&app),
            profiles::Profile {
                text: "Synthetic service explanation.".into(),
                source: "model".into(),
                provider: Some("meta".into()),
                model: Some("fixture".into()),
                generated_at: Some(now()),
                cached: false,
            },
        );
        let guard = f.runtime.inference.lock().await;
        let cached = f
            .runtime
            .explain_application(&process.workload_id, process.pid, process.start_time)
            .await
            .unwrap();
        assert!(cached.cached);
        assert!(f
            .runtime
            .explain_application(&process.workload_id, process.pid, process.start_time + 1)
            .await
            .is_err());
        drop(guard);
        f.runtime.snapshot.write().unwrap().processes[0].executable =
            Some("/example/bin/other-service".into());
        f.runtime.settings.lock().unwrap().mode = settings::Mode::LocalOnly;
        assert!(f
            .runtime
            .explain_application(&process.workload_id, process.pid, process.start_time)
            .await
            .unwrap_err()
            .contains("excluded"));
        assert_eq!(f.runtime.spending().unwrap().requests, 0);
    }

    #[tokio::test]
    async fn confirmed_settings_restore_preserves_automation_and_spending() {
        let f = Fixture::new();
        {
            let mut settings = f.runtime.settings.lock().unwrap();
            settings.mode = settings::Mode::CloudOnly;
            settings.budget_usd = 8.0;
        }
        let saved = f.runtime.set_settings_default().unwrap();
        let mut changed = f.runtime.settings.lock().unwrap().clone();
        changed.mode = settings::Mode::LocalOnly;
        changed.budget_usd = 1.0;
        f.runtime.save_settings(changed).unwrap();
        let mut policy = f.runtime.policy().unwrap();
        policy.enabled = false;
        f.runtime.save_policy(policy).unwrap();
        std::fs::write(
            f.directory.join("spending.json"),
            br#"{"reserved_usd":0.75,"reported_estimate_usd":0.25,"requests":4}"#,
        )
        .unwrap();
        let spending_before = std::fs::read(f.directory.join("spending.json")).unwrap();

        let preview = f.runtime.preview_settings_restore(&saved.id).unwrap();
        assert_eq!(preview.target.id, saved.id);
        let restored = f
            .runtime
            .confirm_settings_restore(&preview.token)
            .await
            .unwrap();
        assert_eq!(restored.mode, settings::Mode::CloudOnly);
        assert_eq!(restored.budget_usd, 8.0);
        assert!(!restored.automation.enabled);
        assert_eq!(
            std::fs::read(f.directory.join("spending.json")).unwrap(),
            spending_before
        );
        assert!(f
            .runtime
            .confirm_settings_restore(&preview.token)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn post_commit_sync_failure_reconciles_restore_without_hiding_durability_error() {
        let fail_sync = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer_fail_sync = fail_sync.clone();
        let writer: SettingsWriter = Arc::new(move |path, settings| {
            storage::write_with_parent_sync(path, settings, |_| {
                if writer_fail_sync.load(Ordering::SeqCst) {
                    Err("Cannot durably persist local application state.".into())
                } else {
                    Ok(())
                }
            })
        });
        let f = Fixture::with_settings_writer(writer);
        let mut target = f.runtime.settings.lock().unwrap().clone();
        target.mode = settings::Mode::CloudOnly;
        target.budget_usd = 8.0;
        f.runtime.save_settings(target).unwrap();
        let saved = f.runtime.set_settings_default().unwrap();
        let mut changed = f.runtime.settings.lock().unwrap().clone();
        changed.mode = settings::Mode::LocalOnly;
        changed.budget_usd = 1.0;
        f.runtime.save_settings(changed).unwrap();
        let mut policy = f.runtime.policy().unwrap();
        policy.enabled = false;
        f.runtime.save_policy(policy).unwrap();
        std::fs::write(
            f.directory.join("spending.json"),
            br#"{"reserved_usd":0.75,"reported_estimate_usd":0.25,"requests":4}"#,
        )
        .unwrap();
        let spending_before = std::fs::read(f.directory.join("spending.json")).unwrap();
        let preview = f.runtime.preview_settings_restore(&saved.id).unwrap();

        fail_sync.store(true, Ordering::SeqCst);
        let error = f
            .runtime
            .confirm_settings_restore(&preview.token)
            .await
            .unwrap_err();

        assert_eq!(error, "Cannot durably persist local application state.");
        let disk: settings::Settings = storage::read(&f.directory.join("settings.json")).unwrap();
        let runtime = f.runtime.settings.lock().unwrap().clone();
        assert_eq!(runtime.mode, settings::Mode::CloudOnly);
        assert_eq!(runtime.budget_usd, 8.0);
        assert!(!runtime.automation.enabled);
        assert_eq!(runtime.mode, disk.mode);
        assert_eq!(runtime.budget_usd, disk.budget_usd);
        assert_eq!(runtime.automation.enabled, disk.automation.enabled);
        assert_eq!(
            std::fs::read(f.directory.join("spending.json")).unwrap(),
            spending_before
        );
    }

    #[tokio::test]
    async fn restore_checks_actual_file_instead_of_assuming_writer_persisted_target() {
        let writer: SettingsWriter = Arc::new(|path, settings| {
            let mut effective = settings.clone();
            effective.budget_usd = 9.0;
            storage::write_detailed(path, &effective)
        });
        let f = Fixture::with_settings_writer(writer);
        let saved = f.runtime.set_settings_default().unwrap();
        let preview = f.runtime.preview_settings_restore(&saved.id).unwrap();
        assert!(f
            .runtime
            .confirm_settings_restore(&preview.token)
            .await
            .unwrap_err()
            .contains("did not match"));
        assert_eq!(f.runtime.settings.lock().unwrap().budget_usd, 9.0);
    }

    #[test]
    fn post_commit_sync_failure_reconciles_provider_settings_save() {
        let fail_sync = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer_fail_sync = fail_sync.clone();
        let writer: SettingsWriter = Arc::new(move |path, settings| {
            storage::write_with_parent_sync(path, settings, |_| {
                if writer_fail_sync.load(Ordering::SeqCst) {
                    Err("Cannot durably persist local application state.".into())
                } else {
                    Ok(())
                }
            })
        });
        let f = Fixture::with_settings_writer(writer);
        let mut changed = f.runtime.settings.lock().unwrap().clone();
        changed.mode = settings::Mode::CloudOnly;
        changed.budget_usd = 6.0;

        fail_sync.store(true, Ordering::SeqCst);
        let error = f.runtime.save_settings(changed).unwrap_err();

        assert_eq!(error, "Cannot durably persist local application state.");
        let disk: settings::Settings = storage::read(&f.directory.join("settings.json")).unwrap();
        let runtime = f.runtime.settings.lock().unwrap().clone();
        assert_eq!(runtime.mode, settings::Mode::CloudOnly);
        assert_eq!(runtime.budget_usd, 6.0);
        assert_eq!(runtime.mode, disk.mode);
        assert_eq!(runtime.budget_usd, disk.budget_usd);
    }

    #[tokio::test]
    async fn restore_cancel_and_optimistic_validation_prevent_stale_changes() {
        let f = Fixture::new();
        let saved = f.runtime.set_settings_default().unwrap();
        let preview = f.runtime.preview_settings_restore(&saved.id).unwrap();
        f.runtime.cancel_settings_restore(&preview.token).unwrap();
        assert!(f
            .runtime
            .confirm_settings_restore(&preview.token)
            .await
            .is_err());

        let preview = f.runtime.preview_settings_restore(&saved.id).unwrap();
        let mut changed = f.runtime.settings.lock().unwrap().clone();
        changed.budget_usd = 3.0;
        f.runtime.save_settings(changed).unwrap();
        assert!(f
            .runtime
            .confirm_settings_restore(&preview.token)
            .await
            .unwrap_err()
            .contains("stale"));

        let preview = f.runtime.preview_settings_restore(&saved.id).unwrap();
        f.runtime.set_settings_default().unwrap();
        assert!(f
            .runtime
            .confirm_settings_restore(&preview.token)
            .await
            .unwrap_err()
            .contains("stale"));
    }

    #[tokio::test]
    async fn restore_rechecks_inference_and_native_action_slots() {
        let f = Fixture::new();
        let saved = f.runtime.set_settings_default().unwrap();
        let preview = f.runtime.preview_settings_restore(&saved.id).unwrap();
        let inference = f.runtime.inference.lock().await;
        assert!(f
            .runtime
            .confirm_settings_restore(&preview.token)
            .await
            .unwrap_err()
            .contains("model request"));
        drop(inference);

        let preview = f.runtime.preview_settings_restore(&saved.id).unwrap();
        let action = f.runtime.action.lock().await;
        assert!(f
            .runtime
            .confirm_settings_restore(&preview.token)
            .await
            .unwrap_err()
            .contains("action"));
        drop(action);
    }

    #[tokio::test]
    async fn system_capture_excludes_concurrent_native_actions() {
        let (native, writes) = fake_system_native();
        let f = Fixture::with_system_native(native);
        let action = f.runtime.action.lock().await;
        assert!(f.runtime.set_system_default().is_err());
        assert!(f
            .runtime
            .system_history(None, 20)
            .unwrap()
            .entries
            .is_empty());
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        drop(action);
        assert!(f.runtime.set_system_default().is_ok());
    }

    #[tokio::test]
    async fn system_rollback_uses_the_supervisor_action_lock() {
        let (native, writes) = fake_system_native();
        let f = Fixture::with_system_native(native);
        let saved = f.runtime.set_system_default().unwrap();
        let preview = f.runtime.preview_system_restore(&saved.id).unwrap();
        let action = f.runtime.action.lock().await;

        let error = f
            .runtime
            .confirm_system_restore(&preview.token)
            .await
            .unwrap_err();

        assert!(error.contains("action is active"));
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        drop(action);
    }

    #[tokio::test]
    async fn snapshot_lifecycle_blocks_only_active_work_without_losing_baselines() {
        let (native, writes) = fake_system_native();
        let f = Fixture::with_system_native(native);
        let legacy = f.runtime.set_settings_default().unwrap();
        let first = f.runtime.set_system_default().unwrap();
        let second = f.runtime.set_system_default().unwrap();
        let (id, _) = f
            .runtime
            .report(spike_capture(&f), &mut jitter::Tally::default())
            .unwrap();
        assert_eq!(f.runtime.case(id).unwrap().origin, Origin::Snapshot);
        for stage in [
            Stage::Detected,
            Stage::Investigating,
            Stage::Applying,
            Stage::Verifying,
        ] {
            f.runtime.update(id, |case| case.stage = stage).unwrap();
            assert!(f
                .runtime
                .set_system_default()
                .unwrap_err()
                .contains("active"));
            let preview = f.runtime.preview_system_restore(&first.id).unwrap();
            assert!(f
                .runtime
                .confirm_system_restore(&preview.token)
                .await
                .unwrap_err()
                .contains("active"));
            assert!(f
                .runtime
                .confirm_system_restore(&preview.token)
                .await
                .is_err());
        }
        for stage in [
            Stage::AwaitingApproval,
            Stage::Completed,
            Stage::Resolved,
            Stage::Blocked,
            Stage::Dismissed,
        ] {
            f.runtime.update(id, |case| case.stage = stage).unwrap();
            let preview = f.runtime.preview_system_restore(&first.id).unwrap();
            f.runtime
                .confirm_system_restore(&preview.token)
                .await
                .unwrap();
        }
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        let history = f.runtime.system_history(None, 20).unwrap();
        assert_eq!(history.entries.len(), 2);
        assert!(history.entries.iter().any(|entry| entry.id == first.id));
        assert!(history
            .entries
            .iter()
            .any(|entry| entry.id == second.id && entry.current));
        assert_eq!(
            f.runtime.inspect_settings_default(&legacy.id).unwrap().id,
            legacy.id
        );
        assert!(f.runtime.case(id).unwrap().jitter.is_some());
    }

    #[tokio::test]
    async fn restore_tokens_expire_and_target_content_is_revalidated() {
        let f = Fixture::new();
        let saved = f.runtime.set_settings_default().unwrap();
        let preview = f.runtime.preview_settings_restore(&saved.id).unwrap();
        f.runtime
            .restore_previews
            .lock()
            .unwrap()
            .entries
            .front_mut()
            .unwrap()
            .expires_at = Instant::now() - Duration::from_secs(1);
        assert!(f
            .runtime
            .confirm_settings_restore(&preview.token)
            .await
            .unwrap_err()
            .contains("expired"));

        let preview = f.runtime.preview_settings_restore(&saved.id).unwrap();
        let path = f
            .directory
            .join("settings-default-history/snapshots")
            .join(format!("{}.json", saved.id));
        let changed =
            std::fs::read_to_string(&path)
                .unwrap()
                .replacen("muse-spark-1.3", "muse-spark-1.2", 1);
        std::fs::write(path, changed).unwrap();
        assert!(f
            .runtime
            .confirm_settings_restore(&preview.token)
            .await
            .unwrap_err()
            .contains("stale"));
    }

    #[test]
    fn restore_preview_memory_is_bounded() {
        let f = Fixture::new();
        let saved = f.runtime.set_settings_default().unwrap();
        let mut tokens = Vec::new();
        for _ in 0..(MAX_RESTORE_PREVIEWS + 8) {
            tokens.push(f.runtime.preview_settings_restore(&saved.id).unwrap().token);
        }
        assert_eq!(
            f.runtime.restore_previews.lock().unwrap().entries.len(),
            MAX_RESTORE_PREVIEWS
        );
        assert!(f.runtime.cancel_settings_restore(&tokens[0]).is_err());
        assert!(f
            .runtime
            .cancel_settings_restore(tokens.last().unwrap())
            .is_ok());
    }

    #[tokio::test]
    async fn active_automatic_investigation_blocks_restore() {
        let f = Fixture::new();
        let saved = f.runtime.set_settings_default().unwrap();
        let preview = f.runtime.preview_settings_restore(&saved.id).unwrap();
        f.incident();
        assert!(f
            .runtime
            .confirm_settings_restore(&preview.token)
            .await
            .unwrap_err()
            .contains("Automatic assistance"));
    }

    #[tokio::test]
    async fn unapproved_incident_investigates_but_never_changes_a_process() {
        let f = Fixture::new();
        let id = f.incident();
        f.runtime.run(id).await;
        let case = f.runtime.case(id).unwrap();
        assert_eq!(case.stage, Stage::AwaitingApproval);
        assert_eq!(case.diagnosis.unwrap().source, "rules");
        assert!(case.report.is_none());
        assert!(f
            .runtime
            .apply(id, false, false)
            .await
            .unwrap_err()
            .contains("not allowed"));
        assert_eq!(f.runtime.spending().unwrap().requests, 0);
        f.runtime.run(id).await; // repeated dispatch must not repeat investigation
        assert_eq!(f.runtime.case(id).unwrap().steps.len(), 3);
    }
    #[tokio::test]
    async fn dismissal_cancels_a_queued_investigation_and_cannot_be_revived() {
        let f = Fixture::new();
        let id = f.incident();
        let guard = f.runtime.inference.lock().await;
        let runtime = f.runtime.clone();
        let task = tokio::spawn(async move { runtime.run(id).await });
        tokio::task::yield_now().await;
        f.runtime.dismiss(id).unwrap();
        drop(guard);
        task.await.unwrap();
        let case = f.runtime.case(id).unwrap();
        assert_eq!(case.stage, Stage::Dismissed);
        assert!(case.diagnosis.is_none());
        assert!(f.runtime.apply(id, true, false).await.is_err());
    }
    #[tokio::test]
    async fn cleared_trigger_or_changed_identity_never_saves_permission() {
        for changed_identity in [false, true] {
            let f = Fixture::new();
            let id = f.incident();
            f.runtime.run(id).await;
            let mut next = sample(now(), if changed_identity { 15.0 } else { 0.0 });
            if changed_identity {
                next.processes[0].start_time += 1;
            }
            *f.runtime.snapshot.write().unwrap() = next;
            f.runtime.apply(id, true, true).await.unwrap();
            assert_eq!(f.runtime.case(id).unwrap().stage, Stage::Resolved);
            assert!(f.runtime.policy().unwrap().rules.is_empty());
            assert!(f.runtime.case(id).unwrap().report.is_none());
        }
    }
    #[tokio::test]
    async fn pause_disable_and_revoked_permissions_block_pending_actions() {
        for mode in ["pause", "disable", "revoke"] {
            let f = Fixture::new();
            let id = f.incident();
            f.runtime.run(id).await;
            let mut p = f.runtime.policy().unwrap();
            p.rules.push(Rule {
                workload_id: "fixture".into(),
                label: "Example".into(),
            });
            f.runtime.save_policy(p.clone()).unwrap();
            match mode {
                "pause" => f.runtime.snooze(15).unwrap(),
                "disable" => {
                    p.enabled = false;
                    f.runtime.save_policy(p).unwrap();
                }
                _ => {
                    p.rules.clear();
                    f.runtime.save_policy(p).unwrap();
                }
            }
            assert!(f.runtime.apply(id, false, false).await.is_err());
            assert!(f.runtime.case(id).unwrap().report.is_none());
        }
    }
    #[tokio::test]
    async fn privacy_mode_excludes_cloud_and_falls_back_without_spending() {
        let f = Fixture::new();
        {
            let mut s = f.runtime.settings.lock().unwrap();
            s.mode = settings::Mode::LocalOnly;
            s.automation.model_analysis = true;
        }
        let id = f.incident();
        f.runtime.run(id).await;
        let reply = f.runtime.case(id).unwrap().diagnosis.unwrap();
        assert_eq!(reply.source, "rules");
        assert!(reply.route.iter().any(|r| r.contains("privacy mode")));
        assert_eq!(f.runtime.spending().unwrap().requests, 0);
    }
    /// A capture where the fixture workload spikes; no frontmost app, local-only inference.
    fn spike_capture(f: &Fixture) -> jitter::Capture {
        f.runtime.settings.lock().unwrap().mode = settings::Mode::LocalOnly;
        *f.runtime.snapshot.write().unwrap() = sample(now(), 15.0);
        let mut c = jitter::tests::capture(|o, t| {
            if (-3..=0).contains(&o) {
                t.processes[0].1 = 150.0;
                t.lag_ms = 40.0;
            }
            t.processes[0].0 = u32::MAX;
        });
        c.identities[0].pid = u32::MAX;
        c.identities[0].workload_id = "fixture".into();
        c.identities[0].label = "Example".into();
        c.victim = None;
        c
    }
    #[tokio::test]
    async fn a_command_culprit_is_investigated_through_its_busiest_process() {
        let f = Fixture::new();
        let mut c = spike_capture(&f);
        // A shell inside the Example app runs a busy `perl` the collector folded into the app.
        let (app, shell, hog) = (u32::MAX, u32::MAX - 1, u32::MAX - 2);
        {
            let mut s = f.runtime.snapshot.write().unwrap();
            s.processes[0].executable = Some("/Applications/Example.app/Contents/MacOS/Ex".into());
            for (pid, parent, name) in [(shell, app, "zsh"), (hog, shell, "perl")] {
                s.processes.push(ProcessRecord {
                    pid,
                    start_time: 1,
                    parent_pid: Some(parent),
                    workload_id: "fixture".into(),
                    name: name.into(),
                    cpu_percent: if pid == hog { 12.0 } else { 0.0 },
                    actions: vec!["lower_priority".into()],
                    ..Default::default()
                });
            }
        }
        for t in &mut c.ticks {
            t.processes[0].0 = hog;
        }
        c.identities[0].pid = hog;
        c.identities[0].name = "perl".into();
        c.identities[0].parent_pid = Some(shell);
        let (id, _) = f.runtime.report(c, &mut jitter::Tally::default()).unwrap();
        let culprit = f.runtime.case(id).unwrap().jitter.unwrap().culprit.unwrap();
        assert_eq!(
            culprit.workload_id.as_deref(),
            Some(format!("cmd-{hog}-1").as_str())
        );
        f.runtime.run(id).await;
        let case = f.runtime.case(id).unwrap();
        // Not "the selected workload is no longer in the inventory".
        assert_eq!(case.diagnosis.unwrap().source, "rules");
        let evidence = case.evidence.unwrap();
        assert_eq!(evidence.target.pid, Some(hog));
        assert!(evidence.processes.iter().all(|p| p.pid == hog));
    }
    #[tokio::test]
    async fn snapshots_bypass_cooldown_and_never_auto_apply_with_a_saved_permission() {
        let f = Fixture::new();
        let mut p = f.runtime.policy().unwrap();
        p.max_investigations_per_hour = 1;
        p.rules.push(Rule {
            workload_id: "fixture".into(),
            label: "Example".into(),
        });
        f.runtime.save_policy(p).unwrap();
        let first = f.incident();
        f.runtime.run(first).await;
        // Dismissed, so the snapshot cannot join it. The automatic hourly cap is now spent;
        // a user's snapshot still opens a case.
        f.runtime.dismiss(first).unwrap();
        let mut tally = jitter::Tally::default();
        let capture = spike_capture(&f);
        let (id, run) = f.runtime.report(capture.clone(), &mut tally).unwrap();
        assert!(run);
        // A second snapshot of the same workload joins the open case instead of a new card.
        let (second, rerun) = f.runtime.report(capture, &mut tally).unwrap();
        assert_eq!((id, rerun), (second, false));
        f.runtime.run(id).await;
        let case = f.runtime.case(id).unwrap();
        assert_eq!(case.origin, Origin::Snapshot);
        assert_eq!(case.trigger.kind, automation::Kind::UserReported);
        assert_eq!(case.stage, Stage::AwaitingApproval);
        assert!(!case.automatic && case.report.is_none());
        let diagnosis = case.diagnosis.unwrap();
        assert_eq!(diagnosis.source, "rules");
        assert!(diagnosis.text.starts_with("What happened"));
        assert!(f
            .runtime
            .apply(id, false, false)
            .await
            .unwrap_err()
            .contains("not allowed"));
        assert!(f
            .runtime
            .apply(id, true, true)
            .await
            .unwrap_err()
            .contains("recurs"));
        assert_eq!(f.runtime.spending().unwrap().requests, 0);
        assert_eq!(tally.recurrence("Example"), 2);
        assert!(jitter::Tally::path(&f.directory).exists());
    }
    #[tokio::test]
    async fn a_stop_runs_only_when_offered_and_never_through_priority_approval() {
        let f = Fixture::new();
        let id = f.incident();
        f.runtime.run(id).await;
        f.runtime
            .update(id, |c| {
                c.alternative = None;
                c.stage = Stage::AwaitingApproval;
            })
            .unwrap();
        assert!(f
            .runtime
            .stop(id)
            .await
            .unwrap_err()
            .contains("No stop was offered"));
        // Nothing was changed by the refused request.
        let case = f.runtime.case(id).unwrap();
        assert_eq!(case.stage, Stage::AwaitingApproval);
        assert!(case.report.is_none());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn an_approved_pause_stops_the_whole_command_and_resume_continues_it() {
        use std::process::{Command, Stdio};
        fn stopped(pid: u32) -> bool {
            let out = Command::new("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().starts_with('T')
        }
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30 & wait"])
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let pid = child.id();
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let start = system
            .process(sysinfo::Pid::from_u32(pid))
            .unwrap()
            .start_time();
        let f = Fixture::new();
        let id = f.incident();
        f.runtime.run(id).await;
        let unit = crate::inside::Unit {
            id: format!("{pid}-{start}"),
            kind: "session".into(),
            title: "sh · test".into(),
            cpu_percent: 10.0,
            memory_bytes: 8 << 20,
            process_count: 2,
            pid: Some(pid),
            start_time: Some(start),
            actions: vec!["pause".into()],
            ..Default::default()
        };
        f.runtime
            .update(id, |c| {
                c.alternative = Some(automation::Proposal {
                    process: ProcessRecord {
                        pid,
                        start_time: start,
                        name: "sh".into(),
                        workload_id: "fixture".into(),
                        ..Default::default()
                    },
                    action: "pause".into(),
                    rationale: "Pause sh · test.".into(),
                    impact: None,
                    unit: Some(unit),
                });
                c.stage = Stage::AwaitingApproval;
            })
            .unwrap();
        // A sample newer than the pause stands in for the collector's next one.
        *f.runtime.snapshot.write().unwrap() = sample(now() + 30, 0.0);
        f.runtime.stop(id).await.unwrap();
        let case = f.runtime.case(id).unwrap();
        assert_eq!(case.stage, Stage::Completed);
        let report = case.report.unwrap();
        assert_eq!(report.action, "pause");
        assert!(report.fresh);
        let members = crate::pause::list()
            .into_iter()
            .find(|p| p.pid == pid && p.start_time == start)
            .unwrap()
            .members;
        assert!(members.len() >= 2, "{members:?}");
        assert!(members.iter().all(|(p, _)| stopped(*p)));
        // Approving again changes nothing; resuming continues every member.
        assert!(f.runtime.stop(id).await.is_err());
        assert!(f.runtime.resume(id).unwrap().ends_with("resumed."));
        assert!(members.iter().all(|(p, _)| !stopped(*p)));
        assert!(!crate::pause::is_paused(pid, start));
        assert!(f.runtime.resume(id).is_err());
        let _ = child.kill();
        let _ = child.wait();
    }
    #[tokio::test]
    async fn one_workload_one_incident_in_either_order() {
        // Automatic first: the manual snapshot joins it and marks it as a snapshot.
        let f = Fixture::new();
        let first = f.incident();
        let capture = spike_capture(&f);
        let (id, _) = f
            .runtime
            .report(capture.clone(), &mut jitter::Tally::default())
            .unwrap();
        assert_eq!(id, first);
        let case = f.runtime.case(id).unwrap();
        assert_eq!(case.origin, Origin::Snapshot);
        assert!(case.jitter.is_some());
        assert_eq!(f.runtime.cases().unwrap().len(), 1);
        // A finished case is analyzed again with the new snapshot.
        f.runtime
            .update(id, |c| c.transition(Stage::Completed, "Done."))
            .unwrap();
        let (again, rerun) = f
            .runtime
            .report(capture.clone(), &mut jitter::Tally::default())
            .unwrap();
        assert_eq!((again, rerun), (id, true));
        assert_eq!(f.runtime.case(id).unwrap().stage, Stage::Detected);

        // Snapshot first: a later automatic trigger adds a note instead of a second card.
        let f = Fixture::new();
        let (id, _) = f
            .runtime
            .report(spike_capture(&f), &mut jitter::Tally::default())
            .unwrap();
        // The monitor waits while a case is still running, as in the live loop.
        f.runtime.run(id).await;
        let at = now();
        for offset in [10, 5, 0] {
            assert!(f
                .runtime
                .observe(&sample(at - offset, 15.0))
                .unwrap()
                .is_none());
        }
        let cases = f.runtime.cases().unwrap();
        assert_eq!(cases.len(), 1);
        assert!(cases[0]
            .steps
            .iter()
            .any(|s| s.message.starts_with("Automatic monitoring also flagged")));
        assert_eq!(cases[0].id, id);
    }
    #[tokio::test]
    async fn approved_snapshot_actions_skip_the_live_trigger_but_recheck_identity() {
        let f = Fixture::new();
        let (id, _) = f
            .runtime
            .report(spike_capture(&f), &mut jitter::Tally::default())
            .unwrap();
        f.runtime.run(id).await;
        // The workload is quiet now; a sustained-incident case would resolve instead of acting.
        *f.runtime.snapshot.write().unwrap() = sample(now(), 0.0);
        assert!(f.runtime.apply(id, true, false).await.is_err());
        let case = f.runtime.case(id).unwrap();
        assert_eq!(case.stage, Stage::Blocked);
        assert!(case.steps.last().unwrap().message.contains("exited"));
    }
    #[tokio::test]
    async fn manual_snapshots_run_when_automation_is_off_but_spikes_do_not() {
        let f = Fixture::new();
        let capture = spike_capture(&f);
        let mut p = f.runtime.policy().unwrap();
        p.enabled = false;
        f.runtime.save_policy(p.clone()).unwrap();
        assert_eq!(f.runtime.spike(capture.clone()).unwrap(), None);
        let (id, _) = f
            .runtime
            .report(capture.clone(), &mut jitter::Tally::default())
            .unwrap();
        f.runtime.run(id).await;
        assert!(f.runtime.case(id).unwrap().diagnosis.is_some());
        // While that case is open, a spike about the same workload joins it silently.
        p.enabled = true;
        f.runtime.save_policy(p.clone()).unwrap();
        assert_eq!(f.runtime.spike(capture.clone()).unwrap(), None);
        f.runtime.dismiss(id).unwrap();
        // Short-hitch cases are opt-in, like automatic snapshots.
        assert_eq!(f.runtime.spike(capture.clone()).unwrap(), None);
        p.auto_snapshots = true;
        f.runtime.save_policy(p).unwrap();
        let spike = f.runtime.spike(capture.clone()).unwrap().unwrap();
        assert_eq!(f.runtime.case(spike).unwrap().origin, Origin::Spike);
        // Spikes share the automatic global gap and per-reason cooldown.
        assert_eq!(f.runtime.spike(capture).unwrap(), None);
        f.runtime.run(spike).await;
        let case = f.runtime.case(spike).unwrap();
        assert_eq!(case.stage, Stage::AwaitingApproval);
        assert!(f.runtime.apply(spike, false, false).await.is_err());
        // Spikes with nothing measured are dropped rather than interrupting the user.
        let quiet = jitter::tests::capture(|_, _| {});
        assert_eq!(f.runtime.spike(quiet).unwrap(), None);
    }
    #[tokio::test]
    async fn demos_and_stale_telemetry_cannot_start_actions() {
        let f = Fixture::new();
        for at in [now() - 10, now() - 5, now()] {
            let mut s = sample(at, 15.0);
            s.source = "demo".into();
            assert!(f.runtime.observe(&s).unwrap().is_none());
        }
        let id = f.incident();
        f.runtime.run(id).await;
        f.runtime.snapshot.write().unwrap().captured_at = now() - 30;
        assert!(f
            .runtime
            .apply(id, true, true)
            .await
            .unwrap_err()
            .contains("Fresh telemetry"));
        assert!(f.runtime.policy().unwrap().rules.is_empty());
    }

    #[test]
    fn priority_changes_are_listed_restored_by_identity_and_dropped_once_gone() {
        use std::process::{Command, Stdio};
        let mut child = Command::new("/bin/cat")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut system = sysinfo::System::new();
        system.refresh_processes(
            sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
            true,
        );
        let start = system
            .process(sysinfo::Pid::from_u32(pid))
            .unwrap()
            .start_time();
        let f = Fixture::new();
        let process = ProcessRecord {
            pid,
            start_time: start,
            name: "cat".into(),
            workload_id: "fixture".into(),
            ..Default::default()
        };
        let record = |process: &ProcessRecord, previous: i32, case: Option<u64>| {
            f.runtime
                .record_priority(
                    process,
                    actions::Lowered { previous, now: 10 },
                    priorities::Source::Rule,
                    case,
                )
                .unwrap();
        };
        // Stands in for Jitterbug's own lowering, which only supported apps receive.
        #[cfg(target_os = "macos")]
        unsafe {
            assert_eq!(libc::setpriority(libc::PRIO_PROCESS, pid, 10), 0);
        }
        record(&process, 0, Some(42));
        let id = f.runtime.priority_change_for(42).unwrap();
        #[cfg(target_os = "macos")]
        {
            let listed = f.runtime.priority_changes().unwrap();
            assert_eq!(listed.changes.len(), 1);
            assert_eq!(listed.changes[0].source, priorities::Source::Rule);
            assert_eq!(listed.changes[0].label, "cat");
            // Raising a priority back needs an administrator; the change stays listed.
            if unsafe { libc::geteuid() } != 0 {
                let error = f.runtime.restore_priority(id).unwrap_err();
                assert!(error.contains("administrator"), "{error}");
                assert_eq!(f.runtime.priority_changes().unwrap().changes.len(), 1);
            }
        }
        // A reused PID is another process: it is never touched and the entry is dropped.
        let stranger = ProcessRecord {
            start_time: start + 1,
            ..process.clone()
        };
        record(&stranger, 0, Some(43));
        let other = f.runtime.priority_change_for(43).unwrap();
        assert_ne!(other, id);
        let error = f.runtime.restore_priority(other).unwrap_err();
        assert!(error.contains("Nothing was changed"), "{error}");
        assert!(f.runtime.priority_change_for(43).is_none());
        #[cfg(target_os = "macos")]
        assert_eq!(actions::priority(pid, start), Ok(10));
        let _ = child.kill();
        let _ = child.wait();
        // An exited process leaves the list.
        assert!(f.runtime.priority_changes().unwrap().changes.is_empty());
        assert!(f.runtime.restore_priority(id).is_err());
    }

    #[test]
    fn a_saved_permission_can_be_removed() {
        let f = Fixture::new();
        let mut policy = f.runtime.policy().unwrap();
        policy.rules.push(Rule {
            workload_id: "w-1".into(),
            label: "Example".into(),
        });
        f.runtime.save_policy(policy).unwrap();
        assert_eq!(f.runtime.priority_changes().unwrap().rules.len(), 1);
        f.runtime.remove_rule("w-1").unwrap();
        assert!(f.runtime.policy().unwrap().rules.is_empty());
        let saved: settings::Settings = storage::read(&f.directory.join("settings.json")).unwrap();
        assert!(saved.automation.rules.is_empty());
        assert!(f.runtime.remove_rule("w-1").is_err());
    }
}
