//! Native incident lifecycle. It continues while the window is hidden; models cannot execute tools.
use crate::{
    actions,
    automation::{self, Case, Monitor, Origin, Policy, Rule, Stage},
    budget, details, jitter, profiles, providers, settings, storage,
    types::*,
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

type Publish = Arc<dyn Fn(Case) + Send + Sync>;

#[derive(Default)]
struct Book {
    monitor: Monitor,
    cases: VecDeque<Case>,
    sequence: u64,
}

/// Triggers about the same workload within this window form one incident.
const MERGE_SECONDS: u64 = 300;

/// An open or recent case about the same workload.
fn related(cases: &VecDeque<Case>, workload: Option<&str>, at: u64) -> Option<u64> {
    let workload = workload?;
    cases
        .iter()
        .find(|c| {
            c.stage != Stage::Dismissed
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
    pub directory: PathBuf,
    book: Mutex<Book>,
    publish: Publish,
}

impl Runtime {
    pub fn new(directory: PathBuf, settings: settings::Settings, publish: Publish) -> Arc<Self> {
        Arc::new(Self {
            snapshot: Arc::new(RwLock::new(Snapshot::default())),
            settings: Mutex::new(settings),
            engine: providers::Engine::new(&directory),
            inference: tokio::sync::Mutex::new(()),
            icons: crate::applications::Icons::default(),
            profiles: Mutex::new(crate::profiles::Cache::read(&directory)),
            task_summaries: Mutex::new(crate::profiles::Cache::default()),
            action: tokio::sync::Mutex::new(()),
            directory,
            book: Mutex::new(Book::default()),
            publish,
        })
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
        let message = tokio::task::spawn_blocking(move || {
            // Serialize policy changes with the last authorization check and the OS action.
            let settings = runtime.settings.lock().map_err(|_| "Settings unavailable.")?;
            let book = runtime.book.lock().map_err(|_| "Incident state unavailable.")?;
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
            }
            actions::execute(pid, start_time, &operation)
        }).await.map_err(|_| "The process action could not finish.")??;
        let applied_at = now();
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
    pub fn spending(&self) -> Result<budget::Ledger, String> {
        self.engine.spending()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::tests::{memory_sample, sample};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
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
}
