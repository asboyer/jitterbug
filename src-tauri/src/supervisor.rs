//! Native incident lifecycle. It continues while the window is hidden; models cannot execute tools.
use crate::{
    actions,
    automation::{self, Case, Monitor, Policy, Rule, Stage},
    budget, details, profiles, providers, settings, storage,
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
        Ok(self.policy()?.enabled
            && self.snoozed_until()? <= now()
            && self.case(id)?.stage != Stage::Dismissed)
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
        let case = {
            let mut book = self
                .book
                .lock()
                .map_err(|_| "Incident state unavailable.")?;
            let busy = book.cases.iter().any(|c| c.stage.running());
            let Some(trigger) = book.monitor.observe(sample, &policy, busy) else {
                return Ok(None);
            };
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
        if let Err(error) = self.investigate(id).await {
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
                .chat(
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
        let Some(proposal) = automation::proposal(&snapshot, &case.trigger) else {
            self.update(id, |c| c.transition(Stage::Completed, "Investigation complete. No supported priority adjustment fits this incident. Review the diagnosis; memory problems and protected processes need a different intervention."))?;
            return Ok(());
        };
        if actions::priority(proposal.process.pid, proposal.process.start_time)
            .is_ok_and(|priority| priority >= 10)
        {
            self.update(id, |c| c.transition(Stage::Completed, "The busy process already has reduced scheduling priority. No repeated change was applied."))?;
            return Ok(());
        }
        let automatic = policy.allows(&proposal);
        self.update(id, |c| { c.proposal = Some(proposal); c.transition(Stage::AwaitingApproval,
            if automatic { "This workload has permission for automatic priority changes. Rechecking live evidence before acting." }
            else { "A priority adjustment is available. Approve it once, or allow future automatic priority changes for this workload." }); })?;
        if automatic {
            self.apply(id, false, false).await?;
        }
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
        if !automation::still_present(&before, &policy, &case.trigger) {
            self.update(id, |c| {
                c.transition(
                    Stage::Resolved,
                    "The trigger cleared before approval. No action was applied.",
                )
            })?;
            return Ok(());
        }
        if !approved && !policy.allows(&proposal) {
            return Err("This workload is not allowed to change priority automatically.".into());
        }
        if remember {
            if !approved {
                return Err("Only explicit user approval can add a permission.".into());
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
                let verification = automation::verify(&before, &after, &report, verified);
                self.update(id, |c| { c.report = Some(report); c.verification = Some(verification); c.transition(Stage::Completed, "The action finished and follow-up evidence is available. No further changes will be made for this incident."); })?;
                Ok(())
            }
            Err(error) => {
                self.update(id, |c| c.transition(Stage::Blocked, &error))?;
                Err(error)
            }
        }
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
                if !settings.automation.enabled || book.monitor.snoozed_until > now() || case.stage != Stage::Applying
                    || (automatic && !settings.automation.allows(proposal)) { return Err("The permission was withdrawn or automation was paused; no action was applied.".into()); }
                let snapshot = runtime.snapshot()?;
                if !automation::still_present(&snapshot, &settings.automation, &case.trigger)
                    || !snapshot.processes.iter().any(|p| p.pid == pid && p.start_time == start_time && p.workload_id == proposal.process.workload_id && p.cpu_percent * snapshot.logical_cpus.max(1) as f32 >= 10.0) {
                    return Err("The trigger or process activity changed. No action was applied.".into());
                }
            }
            actions::execute(pid, start_time, &operation)
        }).await.map_err(|_| "The process action could not finish.")??;
        let applied_at = now();
        if let Some((id, _)) = incident {
            self.update(id, |c| c.transition(Stage::Verifying, "Priority change sent. Waiting for a complete new sampling interval to measure the result."))?;
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
    use crate::automation::tests::sample;
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
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
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
