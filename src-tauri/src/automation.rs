//! Bounded detection, policy, and evidence for the automatic incident loop.
use crate::{
    diagnostics::{self, Investigation, Target},
    providers::Reply,
    types::*,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
    pub workload_id: String,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    pub enabled: bool,
    pub model_analysis: bool,
    pub system_cpu_percent: f32,
    pub available_memory_percent: f32,
    /// 100 means one logical core. The dashboard uses total machine capacity instead.
    pub app_cpu_core_percent: f32,
    /// Minimum share of total machine capacity for any app-specific CPU alert.
    pub app_cpu_percent: f32,
    pub sustained_samples: u32,
    pub cooldown_seconds: u64,
    pub max_investigations_per_hour: usize,
    pub rules: Vec<Rule>,
    /// Jitter Snapshot sampling: always-on pulse, or recording only after a press.
    pub jitter_mode: crate::pulse::Mode,
    /// Out-of-app prompts for cases that need the user. Older settings files default to on.
    pub action_popups: bool,
    /// Attach a snapshot to automatic incidents and open short-hitch cases. Off by default:
    /// snapshots are taken on demand unless the user turns this on.
    pub auto_snapshots: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            model_analysis: true,
            system_cpu_percent: 85.0,
            available_memory_percent: 12.0,
            app_cpu_core_percent: 100.0,
            app_cpu_percent: 10.0,
            sustained_samples: 3,
            cooldown_seconds: 300,
            max_investigations_per_hour: 4,
            rules: vec![],
            jitter_mode: crate::pulse::Mode::Auto,
            action_popups: true,
            auto_snapshots: false,
        }
    }
}

impl Policy {
    pub fn validate(&self) -> Result<(), String> {
        if !self.system_cpu_percent.is_finite()
            || !(50.0..=100.0).contains(&self.system_cpu_percent)
            || !self.available_memory_percent.is_finite()
            || !(5.0..=30.0).contains(&self.available_memory_percent)
            || !self.app_cpu_core_percent.is_finite()
            || !(50.0..=1600.0).contains(&self.app_cpu_core_percent)
            || !self.app_cpu_percent.is_finite()
            || !(5.0..=100.0).contains(&self.app_cpu_percent)
            || !(3..=12).contains(&self.sustained_samples)
            || !(60..=3600).contains(&self.cooldown_seconds)
            || !(1..=12).contains(&self.max_investigations_per_hour)
            || self.rules.len() > 64
        {
            return Err("Invalid automation limits. Use 3–12 sustained samples, a 60–3600 second cooldown, and 1–12 investigations per hour.".into());
        }
        let mut ids = std::collections::HashSet::new();
        for rule in &self.rules {
            if rule.workload_id.is_empty()
                || rule.workload_id.len() > 100
                || rule.label.len() > 80
                || rule.label.chars().any(char::is_control)
                || !ids.insert(&rule.workload_id)
            {
                return Err(
                    "Automatic priority permissions must have unique, valid workload identifiers."
                        .into(),
                );
            }
        }
        Ok(())
    }
    pub fn allows(&self, proposal: &Proposal) -> bool {
        self.enabled
            && proposal.action == "lower_priority"
            && self
                .rules
                .iter()
                .any(|r| r.workload_id == proposal.process.workload_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    SystemCpu,
    MemoryPressure,
    ThermalPressure,
    AppCpu,
    CpuAnomaly,
    MemoryGrowth,
    AppUnresponsive,
    /// The user pressed Jitter Snapshot.
    UserReported,
    /// Auto-mode pulse detector saw a sustained short hitch.
    ShortSpike,
}

/// Where a case came from. Snapshot and spike cases always wait for explicit approval.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    #[default]
    Automatic,
    Snapshot,
    Spike,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trigger {
    pub key: String,
    pub kind: Kind,
    pub title: String,
    pub reason: String,
    pub target: Option<Target>,
    pub cpu_floor_percent: Option<f32>,
    pub memory_floor_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Detected,
    Investigating,
    AwaitingApproval,
    Applying,
    Verifying,
    Completed,
    Resolved,
    Blocked,
    Dismissed,
}

impl Stage {
    pub fn running(&self) -> bool {
        matches!(
            self,
            Self::Detected | Self::Investigating | Self::Applying | Self::Verifying
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Step {
    pub at: u64,
    pub stage: Stage,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    pub process: ProcessRecord,
    pub action: String,
    pub rationale: String,
    #[serde(default)]
    pub impact: Option<Impact>,
    /// The tab or command session a granular action targets (pause, close_tab).
    #[serde(default)]
    pub unit: Option<crate::inside::Unit>,
}

/// The workload around a proposal, measured when it was made, so "now" and "after" can be
/// compared. CPU is a share of total machine capacity; summed memory may double-count.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Impact {
    pub label: String,
    pub workload_cpu_percent: f32,
    pub workload_memory_bytes: u64,
    pub process_count: usize,
    /// Stopping this process quits the whole app (its main executable), not one helper.
    pub whole_app: bool,
}

fn whole_app(process: &ProcessRecord) -> bool {
    process
        .executable
        .as_ref()
        .and_then(|exe| exe.parent())
        .is_some_and(|dir| dir.ends_with("Contents/MacOS"))
}

pub fn impact(s: &Snapshot, process: &ProcessRecord) -> Option<Impact> {
    let workload = s.workloads.iter().find(|w| w.id == process.workload_id)?;
    Some(Impact {
        label: workload.label.clone(),
        workload_cpu_percent: workload.cpu_percent,
        workload_memory_bytes: workload.memory_bytes,
        process_count: workload.process_count,
        whole_app: whole_app(process),
    })
}

/// A stop the user may approve: an app's main executable (quits the app) or a supported
/// development process. App helpers such as browser tabs are never offered, and a stop is
/// never applied automatically or remembered as a permission.
pub fn stop_option(s: &Snapshot, workload_id: &str) -> Option<Proposal> {
    let candidates = s
        .processes
        .iter()
        .filter(|p| p.workload_id == workload_id && p.actions.iter().any(|a| a == "terminate"));
    let process = candidates
        .max_by(|a, b| {
            (whole_app(a), a.cpu_percent)
                .partial_cmp(&(whole_app(b), b.cpu_percent))
                .unwrap_or(std::cmp::Ordering::Equal)
        })?
        .clone();
    let impact = impact(s, &process)?;
    let rationale = if impact.whole_app {
        format!("Quit {}. Every {} window and tab closes, including any call or unsaved work in progress.", impact.label, impact.label)
    } else {
        format!(
            "Stop {} in {}. Whatever it is running, such as a dev server, build, or script, ends.",
            process.name, impact.label
        )
    };
    Some(Proposal {
        impact: Some(impact),
        process,
        action: "terminate".into(),
        rationale,
        unit: None,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Verification {
    pub summary: String,
    pub system_cpu_before: f32,
    pub system_cpu_after: Option<f32>,
    pub available_memory_before: u64,
    pub available_memory_after: Option<u64>,
    pub pressure_before: String,
    pub pressure_after: Option<String>,
    pub priority_verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Case {
    pub id: u64,
    pub source: String,
    pub detected_at: u64,
    pub trigger: Trigger,
    pub stage: Stage,
    pub steps: Vec<Step>,
    pub evidence: Option<Investigation>,
    pub diagnosis: Option<Reply>,
    pub proposal: Option<Proposal>,
    /// A stop the user can choose instead, shown with what it would free and what it closes.
    #[serde(default)]
    pub alternative: Option<Proposal>,
    pub report: Option<ActionReport>,
    pub verification: Option<Verification>,
    pub automatic: bool,
    #[serde(default)]
    pub origin: Origin,
    #[serde(default)]
    pub jitter: Option<crate::jitter::Report>,
}

impl Case {
    pub fn new(id: u64, snapshot: &Snapshot, trigger: Trigger) -> Self {
        let evidence = trigger
            .target
            .clone()
            .map(|t| diagnostics::investigate(snapshot, t));
        let mut case = Self {
            id,
            source: snapshot.source.clone(),
            detected_at: snapshot.captured_at,
            trigger,
            stage: Stage::Detected,
            steps: vec![],
            evidence,
            diagnosis: None,
            proposal: None,
            alternative: None,
            report: None,
            verification: None,
            automatic: false,
            origin: Origin::Automatic,
            jitter: None,
        };
        case.transition(
            Stage::Detected,
            "A sustained threshold or unusual workload change was detected.",
        );
        case
    }
    /// Record an event without changing the stage, such as a merged trigger.
    pub fn note(&mut self, message: &str) {
        let stage = self.stage.clone();
        self.transition(stage, message);
    }
    pub fn transition(&mut self, stage: Stage, message: &str) {
        self.stage = stage.clone();
        self.steps.push(Step {
            at: now(),
            stage,
            message: message.into(),
        });
        if self.steps.len() > 12 {
            self.steps.remove(0);
        }
    }
}

#[derive(Default)]
struct Track {
    consecutive: u32,
    active_since: Option<u64>,
    healthy: u32,
    healthy_since: Option<u64>,
    fired: bool,
    last_fired: u64,
}

#[derive(Default)]
pub struct Monitor {
    tracks: HashMap<String, Track>,
    recent: VecDeque<u64>,
    last_sample: u64,
    last_dispatch: Option<u64>,
    pub snoozed_until: u64,
}

fn target(w: &Workload) -> Target {
    Target {
        workload_id: w.id.clone(),
        label: w.label.clone(),
        pid: None,
    }
}

const MEMORY_KEY: &str = "memory-pressure";
const MEMORY_SUSTAIN_SECONDS: u64 = 30;
const MEMORY_RECOVERY_SECONDS: u64 = 60;
const MEMORY_COOLDOWN_SECONDS: u64 = 900;

fn cooldown(key: &str, policy: &Policy) -> u64 {
    if key == MEMORY_KEY {
        policy.cooldown_seconds.max(MEMORY_COOLDOWN_SECONDS)
    } else {
        policy.cooldown_seconds
    }
}

/// App allocations and cached pages are not, on their own, a reason to interrupt.
/// Prefer native pressure when available; use headroom as a fallback on other hosts.
fn memory_pressure_reason(s: &Snapshot, policy: &Policy) -> Option<String> {
    if matches!(
        s.platform.memory_pressure.as_deref(),
        Some("warning" | "critical")
    ) {
        return Some("The operating system reports memory pressure.".into());
    }
    if s.swap_growth_bytes > 256 * 1024 * 1024 {
        return Some("Swap use is growing by over 256 MiB per sample.".into());
    }
    let available = s.memory_available_bytes as f64 / s.memory_total_bytes.max(1) as f64 * 100.0;
    if s.platform.memory_pressure.as_deref() != Some("normal")
        && s.memory_total_bytes > 0
        && available < policy.available_memory_percent as f64
        && s.memory_available_bytes < 2 * 1024 * 1024 * 1024
    {
        return Some(format!(
            "Available memory is below {:.0}% and 2 GiB; native memory pressure is unavailable.",
            policy.available_memory_percent
        ));
    }
    None
}

/// Growth supplies context for a machine-wide incident, never a separate alert.
fn memory_growth(w: &Workload) -> Option<u64> {
    if w.history.len() < 9 {
        return None;
    }
    let baseline = &w.history[..w.history.len() - 3];
    let memory =
        baseline.iter().map(|p| p.memory_bytes as f64).sum::<f64>() / baseline.len() as f64;
    let growth = w.memory_bytes as f64 - memory;
    (w.memory_bytes as f64 >= memory * 1.5 && growth >= 512.0 * 1048576.0)
        .then_some(growth.max(0.0) as u64)
}

/// Pure rule evaluation: no inference, file access, or process changes in the sampler.
pub fn conditions(s: &Snapshot, policy: &Policy) -> Vec<Trigger> {
    if !s.sample_ready {
        return vec![];
    }
    let mut found = vec![];
    let cpu_target = s
        .workloads
        .iter()
        .max_by(|a, b| a.cpu_percent.total_cmp(&b.cpu_percent))
        .map(target);
    if s.cpu_percent >= policy.system_cpu_percent {
        found.push(Trigger {
            cpu_floor_percent: None,
            memory_floor_bytes: None,
            key: "system-cpu".into(),
            kind: Kind::SystemCpu,
            title: "Your computer is busy".into(),
            reason: format!(
                "Total CPU has reached the configured {:.0}% threshold.",
                policy.system_cpu_percent
            ),
            target: cpu_target.clone(),
        });
    }
    if let Some(mut reason) = memory_pressure_reason(s, policy) {
        let growing = s
            .workloads
            .iter()
            .filter_map(|w| memory_growth(w).map(|g| (w, g)))
            .max_by_key(|(_, growth)| *growth)
            .map(|(w, _)| w);
        let workload = growing.or_else(|| s.workloads.iter().max_by_key(|w| w.memory_bytes));
        if workload.is_some() {
            reason.push_str(if growing.is_some() {
                " The selected workload's reported memory also rose at least 50% and 512 MiB. This is context, not proof that it caused the pressure or has a leak."
            } else {
                " The selected workload has the largest reported memory footprint; it is not necessarily the cause."
            });
        }
        found.push(Trigger {
            cpu_floor_percent: None,
            memory_floor_bytes: None,
            key: MEMORY_KEY.into(),
            kind: Kind::MemoryPressure,
            title: "Memory needs attention".into(),
            reason,
            target: workload.map(target),
        });
    }
    if matches!(s.platform.thermal.as_deref(), Some("serious" | "critical")) {
        found.push(Trigger {
            cpu_floor_percent: None,
            memory_floor_bytes: None,
            key: "thermal-pressure".into(),
            kind: Kind::ThermalPressure,
            title: "Your computer is under thermal pressure".into(),
            reason: "The operating system reports serious or critical thermal pressure.".into(),
            target: cpu_target,
        });
    }
    for w in &s.workloads {
        if w.unresponsive == Some(true) {
            found.push(Trigger {
                key: format!("unresponsive:{}", w.id), kind: Kind::AppUnresponsive,
                title: format!("{} is not responding", w.label),
                reason: "The macOS window server repeatedly reports an unresponsive app. Low CPU does not rule out a blocked main thread or another stalled operation. This signal does not monitor individual web pages.".into(),
                target: Some(target(w)), cpu_floor_percent: None, memory_floor_bytes: None,
            });
        }
        let core_cpu = w.cpu_percent * s.logical_cpus.max(1) as f32;
        if w.cpu_percent >= policy.app_cpu_percent && core_cpu >= policy.app_cpu_core_percent {
            found.push(Trigger { cpu_floor_percent: None, memory_floor_bytes: None, key: format!("app-cpu:{}", w.id), kind: Kind::AppCpu, title: format!("{} is using sustained CPU", w.label), reason: format!("Workload CPU exceeded {:.0}% of total capacity and {:.0}% on a one-core scale (100% = one logical core). High usage may be legitimate work.", policy.app_cpu_percent, policy.app_cpu_core_percent), target: Some(target(w)) });
        }
        // A fixed earlier window makes a step change detectable across consecutive samples.
        // Require at least six baseline samples, separated from the most recent three.
        if w.history.len() >= 9 {
            let baseline: Vec<_> = w.history.iter().take(w.history.len() - 3).collect();
            let cpu = baseline.iter().map(|p| p.cpu_percent).sum::<f32>() / baseline.len() as f32;
            // A large ratio against an idle baseline is normal when an app wakes up.
            // Require an absolute machine-wide impact as well as a significant rise.
            let cpu_floor = policy
                .app_cpu_percent
                .max(cpu.max(0.1) * 3.0)
                .max(cpu + 5.0);
            if w.cpu_percent >= cpu_floor {
                found.push(Trigger {
                    cpu_floor_percent: Some(cpu_floor),
                    memory_floor_bytes: None,
                    key: format!("cpu-anomaly:{}", w.id),
                    kind: Kind::CpuAnomaly,
                    title: format!("{} has unusual CPU activity", w.label),
                    reason: format!("CPU reached at least {:.0}% of total capacity, three times its baseline, and a rise of five percentage points. This is an anomaly to investigate, not a diagnosis.", policy.app_cpu_percent),
                    target: Some(target(w)),
                });
            }
        }
    }
    // A stalled window is more specific than simultaneous resource thresholds.
    found.sort_by_key(|trigger| trigger.kind != Kind::AppUnresponsive);
    found
}

impl Monitor {
    /// Admit an automatic short-spike case under the same pause, global gap, hourly cap, and
    /// per-reason cooldown as sustained incidents. Manual snapshots never pass through here.
    pub fn admit(&mut self, key: &str, at: u64, policy: &Policy, busy: bool) -> bool {
        while self
            .recent
            .front()
            .is_some_and(|t| at.saturating_sub(*t) >= 3600)
        {
            self.recent.pop_front();
        }
        let track = self.tracks.entry(key.into()).or_default();
        if !policy.enabled
            || busy
            || self.snoozed_until > at
            || self.recent.len() >= policy.max_investigations_per_hour
            || self
                .last_dispatch
                .is_some_and(|t| at.saturating_sub(t) < 60)
            || (track.last_fired > 0
                && at.saturating_sub(track.last_fired) < policy.cooldown_seconds)
        {
            return false;
        }
        track.last_fired = at;
        self.recent.push_back(at);
        self.last_dispatch = Some(at);
        true
    }
    pub fn observe(&mut self, s: &Snapshot, policy: &Policy, busy: bool) -> Option<Trigger> {
        if s.captured_at <= self.last_sample {
            return None;
        }
        let gap = s.captured_at.saturating_sub(self.last_sample);
        self.last_sample = s.captured_at;
        if !policy.enabled || !s.sample_ready || gap > 20 {
            // A sampling gap breaks evidence continuity, not the alert cooldown.
            for track in self.tracks.values_mut() {
                track.consecutive = 0;
                track.active_since = None;
                track.healthy = 0;
                track.healthy_since = None;
            }
            if !policy.enabled || !s.sample_ready {
                return None;
            }
        }
        let found = conditions(s, policy);
        self.tracks.retain(|key, track| {
            if !found.iter().any(|t| &t.key == key) {
                track.consecutive = 0;
                track.active_since = None;
                track.healthy += 1;
                let since = *track.healthy_since.get_or_insert(s.captured_at);
                let recovered = if key == MEMORY_KEY {
                    s.captured_at.saturating_sub(since) >= MEMORY_RECOVERY_SECONDS
                } else {
                    track.healthy >= 3
                };
                if recovered {
                    track.fired = false;
                }
            }
            track.healthy < 120
                || s.captured_at.saturating_sub(track.last_fired) < cooldown(key, policy)
        });
        for trigger in &found {
            let track = self.tracks.entry(trigger.key.clone()).or_default();
            track.consecutive = track.consecutive.saturating_add(1);
            track.active_since.get_or_insert(s.captured_at);
            track.healthy = 0;
            track.healthy_since = None;
        }
        while self
            .recent
            .front()
            .is_some_and(|at| s.captured_at.saturating_sub(*at) >= 3600)
        {
            self.recent.pop_front();
        }
        if self.snoozed_until > s.captured_at
            || self.recent.len() >= policy.max_investigations_per_hour
        {
            return None;
        }
        let resource_alert_waiting = busy
            || self
                .last_dispatch
                .is_some_and(|at| s.captured_at.saturating_sub(at) < 60);
        for trigger in found {
            // A newly hung app must not disappear behind an unrelated resource
            // investigation. The supervisor still serializes all model requests.
            if resource_alert_waiting && trigger.kind != Kind::AppUnresponsive {
                continue;
            }
            let track = self.tracks.get_mut(&trigger.key).unwrap();
            let sustained = if trigger.key == MEMORY_KEY {
                let seconds = if s.platform.memory_pressure.as_deref() == Some("critical") {
                    10
                } else {
                    MEMORY_SUSTAIN_SECONDS
                };
                track
                    .active_since
                    .is_some_and(|at| s.captured_at.saturating_sub(at) >= seconds)
            } else {
                true
            };
            if track.consecutive >= policy.sustained_samples
                && sustained
                && !track.fired
                && (track.last_fired == 0
                    || s.captured_at.saturating_sub(track.last_fired)
                        >= cooldown(&trigger.key, policy))
            {
                track.fired = true;
                track.last_fired = s.captured_at;
                self.recent.push_back(s.captured_at);
                self.last_dispatch = Some(s.captured_at);
                // Suppress other reasons for this same workload until it recovers, too.
                let target_id = trigger.target.as_ref().map(|t| t.workload_id.as_str());
                for other in conditions(s, policy) {
                    if other.target.as_ref().map(|t| t.workload_id.as_str()) == target_id {
                        if let Some(track) = self.tracks.get_mut(&other.key) {
                            track.fired = true;
                            track.last_fired = s.captured_at;
                        }
                    }
                }
                return Some(trigger);
            }
        }
        None
    }
}

/// Exercise the real sustained-trigger monitor with three small in-memory fixtures. This
/// deliberately has no Runtime, collector, provider, storage, or process-action capability.
/// The caller only publishes the returned demo case; it must never dispatch Runtime::run.
pub fn simulated_case(detected_at: u64) -> Result<Case, String> {
    let detected_at = detected_at.max(11);
    let mut monitor = Monitor::default();
    let policy = Policy::default();
    for at in [detected_at - 10, detected_at - 5, detected_at] {
        let snapshot = simulation_sample(at, true);
        if let Some(trigger) = monitor.observe(&snapshot, &policy, false) {
            let mut case = Case::new(detected_at, &snapshot, trigger);
            case.trigger.title = format!("Demo: {}", case.trigger.title);
            if let Some(step) = case.steps.first_mut() {
                step.at = detected_at;
                step.message = "Demo: three synthetic samples, spaced five seconds apart, passed through the real automatic-assistance monitor. No host load, model call, or process action was generated.".into();
            }
            return Ok(case);
        }
    }
    Err("The synthetic samples did not produce the expected sustained alert.".into())
}

fn simulation_sample(at: u64, busy: bool) -> Snapshot {
    let app_cpu = if busy { 72.0 } else { 0.5 };
    Snapshot {
        source: "demo".into(),
        captured_at: at,
        sample_ready: true,
        interval_ms: 5000,
        logical_cpus: 8,
        cpu_percent: if busy { 91.0 } else { 5.0 },
        memory_total_bytes: 16 << 30,
        memory_available_bytes: if busy {
            (1.4 * (1_u64 << 30) as f64) as u64
        } else {
            8 << 30
        },
        process_count: 1,
        workloads: vec![Workload {
            id: "demo-project".into(),
            label: "sample-web-app".into(),
            category: "project".into(),
            description: "Synthetic development workload for the automatic-assistance demo.".into(),
            attribution: "Synthetic fixture; no real process was inspected.".into(),
            cpu_percent: app_cpu,
            memory_bytes: (2.4 * (1_u64 << 30) as f64) as u64,
            process_count: 1,
            ..Default::default()
        }],
        processes: vec![ProcessRecord {
            pid: 9000,
            start_time: 1,
            name: "node".into(),
            workload_id: "demo-project".into(),
            description: "Synthetic process; this PID is never resolved on the host.".into(),
            evidence: vec!["Generated fixture, not live telemetry.".into()],
            cpu_percent: app_cpu,
            memory_bytes: (2.4 * (1_u64 << 30) as f64) as u64,
            // No allowed actions: even a future consumer must not treat this as executable.
            actionable: false,
            actions: vec![],
            ..Default::default()
        }],
        ..Default::default()
    }
}

pub fn still_present(s: &Snapshot, policy: &Policy, trigger: &Trigger) -> bool {
    if let Some(w) = trigger
        .target
        .as_ref()
        .and_then(|t| s.workloads.iter().find(|w| w.id == t.workload_id))
    {
        if let Some(floor) = trigger.cpu_floor_percent {
            return s.sample_ready && w.cpu_percent >= floor.max(policy.app_cpu_percent);
        }
        if let Some(floor) = trigger.memory_floor_bytes {
            return s.sample_ready
                && w.memory_bytes >= floor
                && memory_pressure_reason(s, policy).is_some();
        }
    }
    conditions(s, policy).iter().any(|t| t.key == trigger.key)
}

pub fn proposal(s: &Snapshot, trigger: &Trigger) -> Option<Proposal> {
    if matches!(
        trigger.kind,
        Kind::MemoryGrowth | Kind::MemoryPressure | Kind::AppUnresponsive
    ) {
        return None;
    }
    let target = trigger.target.as_ref()?;
    let process = s
        .processes
        .iter()
        .filter(|p| {
            p.workload_id == target.workload_id
                && p.actions.iter().any(|a| a == "lower_priority")
                && p.cpu_percent * s.logical_cpus.max(1) as f32 >= 10.0
        })
        .max_by(|a, b| a.cpu_percent.total_cmp(&b.cpu_percent))?
        .clone();
    Some(Proposal { impact: impact(s, &process), process, action: "lower_priority".into(), rationale: "Give competing work more CPU scheduling preference by lowering this busy process’s priority. This does not free memory or guarantee lower CPU usage. Priority stays lowered until the process exits; restoring it may require OS privileges.".into(), unit: None })
}

pub fn verify(
    before: &Snapshot,
    after: &Snapshot,
    report: &ActionReport,
    priority_verified: bool,
) -> Verification {
    let summary = if !report.fresh {
        "The action was sent, but no fresh follow-up sample arrived. Its effect is unverified."
    } else if report.after.is_none() {
        "The process is no longer in the collected inventory. No continuing resource improvement can be attributed to the priority change."
    } else if !priority_verified {
        "Fresh resource measurements arrived, but the OS priority change could not be verified. Review the process before trying another action."
    } else if after.cpu_percent + 5.0 < before.cpu_percent {
        "The OS priority change is verified. Total CPU was lower in the follow-up sample; this is an observation, not proof the action caused it."
    } else {
        "The OS priority change is verified. No clear reduction in total CPU was observed. Lower priority helps scheduling contention and need not reduce CPU or memory usage."
    };
    Verification {
        summary: summary.into(),
        system_cpu_before: before.cpu_percent,
        system_cpu_after: report.fresh.then_some(after.cpu_percent),
        available_memory_before: before.memory_available_bytes,
        available_memory_after: report.fresh.then_some(after.memory_available_bytes),
        pressure_before: before.pressure.level.clone(),
        pressure_after: report.fresh.then(|| after.pressure.level.clone()),
        priority_verified,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn stoppable(s: &mut Snapshot, name: &str, exe: &str, cpu: f32, actions: &[&str]) {
        s.processes.push(ProcessRecord {
            pid: 1000 + s.processes.len() as u32,
            name: name.into(),
            executable: Some(exe.into()),
            workload_id: "app".into(),
            cpu_percent: cpu,
            memory_bytes: 1 << 30,
            actions: actions.iter().map(|a| a.to_string()).collect(),
            ..Default::default()
        });
    }

    #[test]
    fn stop_quits_the_whole_app_never_a_helper() {
        let mut s = Snapshot {
            workloads: vec![Workload {
                id: "app".into(),
                label: "Browser".into(),
                cpu_percent: 45.0,
                memory_bytes: 3 << 30,
                process_count: 3,
                ..Default::default()
            }],
            ..Default::default()
        };
        let main = "/Applications/Browser.app/Contents/MacOS/Browser";
        let helper = "/Applications/Browser.app/Contents/Frameworks/H.app/Contents/MacOS/Helper";
        // The busy helper may only be reprioritized; it is never offered as a stop.
        stoppable(
            &mut s,
            "Browser Helper (Renderer)",
            helper,
            40.0,
            &["lower_priority"],
        );
        assert!(stop_option(&s, "app").is_none());
        stoppable(
            &mut s,
            "Browser",
            main,
            2.0,
            &["lower_priority", "terminate"],
        );
        let stop = stop_option(&s, "app").unwrap();
        assert_eq!(stop.process.name, "Browser");
        assert_eq!(stop.action, "terminate");
        let impact = stop.impact.unwrap();
        assert!(impact.whole_app);
        assert_eq!(
            (impact.workload_cpu_percent, impact.process_count),
            (45.0, 3)
        );
        assert!(stop
            .rationale
            .contains("Every Browser window and tab closes"));
    }

    #[test]
    fn stop_names_a_development_process_and_only_it() {
        let mut s = Snapshot {
            workloads: vec![Workload {
                id: "app".into(),
                label: "my-project".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        stoppable(
            &mut s,
            "node",
            "/opt/node/bin/node",
            12.0,
            &["lower_priority", "terminate"],
        );
        let stop = stop_option(&s, "app").unwrap();
        assert!(!stop.impact.unwrap().whole_app);
        assert!(stop.rationale.starts_with("Stop node in my-project."));
        assert!(stop_option(&s, "other").is_none());
    }
    pub fn sample(at: u64, cpu: f32) -> Snapshot {
        Snapshot {
            source: "live".into(),
            captured_at: at,
            sample_ready: true,
            logical_cpus: 8,
            cpu_percent: 20.0,
            memory_total_bytes: 16 << 30,
            memory_available_bytes: 8 << 30,
            workloads: vec![Workload {
                id: "fixture".into(),
                label: "Example".into(),
                cpu_percent: cpu,
                memory_bytes: 32 << 20,
                ..Default::default()
            }],
            processes: vec![ProcessRecord {
                pid: u32::MAX,
                start_time: 1,
                workload_id: "fixture".into(),
                name: "Example".into(),
                cpu_percent: cpu,
                actions: vec!["lower_priority".into(), "terminate".into()],
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    fn tick(m: &mut Monitor, at: u64, cpu: f32, p: &Policy) -> Option<Trigger> {
        m.observe(&sample(at, cpu), p, false)
    }

    #[test]
    fn unresponsive_apps_alert_at_low_cpu_without_a_priority_proposal() {
        let mut m = Monitor::default();
        let policy = Policy::default();
        let mut s = sample(100, 0.2);
        s.workloads[0].unresponsive = Some(true);
        assert!(m.observe(&s, &policy, false).is_none());
        s.captured_at = 105;
        s.workloads[0].unresponsive = None; // unavailable is not a hang
        assert!(m.observe(&s, &policy, false).is_none());
        for at in [110, 115, 120] {
            s.captured_at = at;
            s.workloads[0].unresponsive = Some(true);
            let result = m.observe(&s, &policy, false);
            if at == 120 {
                let trigger = result.unwrap();
                assert_eq!(trigger.kind, Kind::AppUnresponsive);
                assert!(proposal(&s, &trigger).is_none());
                assert!(still_present(&s, &policy, &trigger));
                s.workloads[0].unresponsive = Some(false);
                assert!(!still_present(&s, &policy, &trigger));
            } else {
                assert!(result.is_none());
            }
        }
    }

    #[test]
    fn hung_apps_are_not_hidden_by_resource_cooldowns_but_obey_pause_and_hourly_cap() {
        let mut m = Monitor {
            last_dispatch: Some(100),
            ..Default::default()
        };
        let policy = Policy {
            max_investigations_per_hour: 1,
            ..Default::default()
        };
        let mut s = sample(105, 0.1);
        s.workloads[0].unresponsive = Some(true);
        for at in [105, 110, 115] {
            s.captured_at = at;
            let result = m.observe(&s, &policy, true);
            assert_eq!(result.is_some(), at == 115);
        }
        s.workloads[0].id = "different-app".into();
        for at in [120, 125, 130] {
            s.captured_at = at;
            assert!(m.observe(&s, &policy, false).is_none());
        }
        let mut paused = Monitor {
            snoozed_until: 1000,
            ..Default::default()
        };
        for at in [135, 140, 145] {
            s.captured_at = at;
            assert!(paused.observe(&s, &policy, false).is_none());
        }
    }

    #[test]
    fn simulation_healthy_samples_and_transient_spikes_do_not_alert() {
        let mut monitor = Monitor::default();
        let policy = Policy::default();
        for at in (100..=150).step_by(5) {
            assert!(monitor
                .observe(&simulation_sample(at, false), &policy, false)
                .is_none());
        }
        for (at, busy) in [
            (155, true),
            (160, true),
            (165, false),
            (170, true),
            (175, false),
        ] {
            assert!(monitor
                .observe(&simulation_sample(at, busy), &policy, false)
                .is_none());
        }
    }

    #[test]
    fn simulation_requires_three_sustained_samples_and_returns_detected_demo_case() {
        let mut monitor = Monitor::default();
        let policy = Policy::default();
        assert!(monitor
            .observe(&simulation_sample(100, true), &policy, false)
            .is_none());
        assert!(monitor
            .observe(&simulation_sample(105, true), &policy, false)
            .is_none());
        assert!(monitor
            .observe(&simulation_sample(110, true), &policy, false)
            .is_some());
        let case = simulated_case(110).unwrap();
        assert_eq!(case.source, "demo");
        assert_eq!(case.stage, Stage::Detected);
        assert_eq!(case.detected_at, 110);
        assert_eq!(case.trigger.kind, Kind::SystemCpu);
        let target = case.trigger.target.unwrap();
        assert_eq!(target.workload_id, "demo-project");
        assert_eq!(target.label, "sample-web-app");
        assert_eq!(case.steps.len(), 1);
        assert_eq!(case.steps[0].stage, Stage::Detected);
        assert!(case.proposal.is_none());
        assert!(case.diagnosis.is_none());
        assert!(case.report.is_none());
        assert!(case.verification.is_none());
        assert!(!case.automatic);
    }

    #[test]
    fn simulation_monitor_cooldown_suppresses_duplicate_alerts() {
        let mut monitor = Monitor::default();
        let policy = Policy::default();
        for at in [100, 105] {
            assert!(monitor
                .observe(&simulation_sample(at, true), &policy, false)
                .is_none());
        }
        assert!(monitor
            .observe(&simulation_sample(110, true), &policy, false)
            .is_some());
        // Recovery clears the fired latch, but the same trigger still has its cooldown.
        for at in [115, 120, 125] {
            assert!(monitor
                .observe(&simulation_sample(at, false), &policy, false)
                .is_none());
        }
        for at in (130..410).step_by(5) {
            assert!(monitor
                .observe(&simulation_sample(at, true), &policy, false)
                .is_none());
        }
        assert!(monitor
            .observe(&simulation_sample(410, true), &policy, false)
            .is_some());
    }

    #[test]
    fn simulation_only_contains_synthetic_non_actionable_evidence() {
        let case = simulated_case(110).unwrap();
        let evidence = case.evidence.unwrap();
        assert_eq!(evidence.processes.len(), 1);
        let process = &evidence.processes[0];
        assert_eq!((process.pid, process.start_time), (9000, 1));
        assert_eq!(process.name, "node");
        assert!(!process.actionable);
        assert!(process.actions.is_empty());
        assert!(process.executable.is_none());
        assert!(process.working_directory.is_none());
        let snapshot = simulation_sample(110, true);
        assert_eq!(snapshot.source, "demo");
        assert!(proposal(&snapshot, &case.trigger).is_none());
        // Independent calls are deterministic fixtures, not mutations of a live monitor.
        assert_eq!(
            serde_json::to_value(simulated_case(110).unwrap()).unwrap(),
            serde_json::to_value(simulated_case(110).unwrap()).unwrap()
        );
    }

    #[test]
    fn sustained_unique_samples_and_recovery_are_required() {
        let p = Policy {
            cooldown_seconds: 60,
            ..Default::default()
        };
        let mut m = Monitor::default();
        assert!(tick(&mut m, 100, 15.0, &p).is_none());
        assert!(tick(&mut m, 100, 15.0, &p).is_none());
        assert!(tick(&mut m, 105, 0.0, &p).is_none()); // spike cleared
        assert!(tick(&mut m, 110, 15.0, &p).is_none());
        assert!(tick(&mut m, 115, 15.0, &p).is_none());
        assert_eq!(tick(&mut m, 120, 15.0, &p).unwrap().kind, Kind::AppCpu);
        for at in (125..=190).step_by(5) {
            assert!(tick(&mut m, at, 15.0, &p).is_none());
        }
        for at in [195, 200, 205] {
            assert!(tick(&mut m, at, 0.0, &p).is_none());
        }
        assert!(tick(&mut m, 210, 15.0, &p).is_none());
        assert!(tick(&mut m, 215, 15.0, &p).is_none());
        assert!(tick(&mut m, 220, 15.0, &p).is_some());
    }
    #[test]
    fn gaps_busy_pause_and_hourly_cap_prevent_dispatch() {
        let p = Policy {
            max_investigations_per_hour: 1,
            ..Default::default()
        };
        let mut m = Monitor::default();
        tick(&mut m, 100, 15.0, &p);
        tick(&mut m, 105, 15.0, &p);
        assert!(tick(&mut m, 150, 15.0, &p).is_none()); // sleep resets streak
        assert!(m.observe(&sample(155, 15.0), &p, true).is_none());
        m.snoozed_until = 170;
        assert!(tick(&mut m, 160, 15.0, &p).is_none());
        assert!(tick(&mut m, 170, 15.0, &p).is_some());
        for at in [175, 180, 185] {
            tick(&mut m, at, 0.0, &p);
        }
        for at in (190..3800).step_by(5) {
            let event = tick(&mut m, at, 15.0, &p);
            if at < 3770 {
                assert!(event.is_none());
            }
            if at == 3770 {
                assert!(event.is_some());
            }
        }
    }
    #[test]
    fn different_reasons_do_not_combine_into_a_streak() {
        let p = Policy::default();
        let mut m = Monitor::default();
        for at in [100, 105, 110, 115] {
            let mut s = sample(at, 0.0);
            if at % 10 == 0 {
                s.cpu_percent = 90.0;
            } else {
                s.memory_available_bytes = 1;
            }
            assert!(m.observe(&s, &p, false).is_none());
        }
    }
    #[test]
    fn anomaly_uses_history_and_preserves_its_original_baseline() {
        let mut s = sample(100, 12.0);
        let p = Policy::default();
        assert!(!conditions(&s, &p)
            .iter()
            .any(|t| t.kind == Kind::CpuAnomaly));
        s.workloads[0].history = (0..9)
            .map(|i| ResourcePoint {
                cpu_percent: if i < 6 { 0.1 } else { 12.0 },
                memory_bytes: 32 << 20,
                ..Default::default()
            })
            .collect();
        let trigger = conditions(&s, &p)
            .into_iter()
            .find(|t| t.kind == Kind::CpuAnomaly)
            .unwrap();
        for point in &mut s.workloads[0].history {
            point.cpu_percent = 12.0;
        }
        assert!(still_present(&s, &p, &trigger)); // recent baseline can adapt during inference
        s.workloads[0].cpu_percent = 0.2;
        assert!(!still_present(&s, &p, &trigger));
    }

    pub(crate) fn memory_sample(at: u64, pressure: &str) -> Snapshot {
        let mut s = sample(at, 0.5);
        s.platform.memory_pressure = Some(pressure.into());
        s.workloads[0].memory_bytes = 1 << 30;
        s.workloads[0].history = vec![
            ResourcePoint {
                cpu_percent: 0.5,
                memory_bytes: 32 << 20,
                ..Default::default()
            };
            12
        ];
        s
    }

    #[test]
    fn normal_app_memory_growth_never_dispatches_an_investigation() {
        let policy = Policy::default();
        for total_gib in [8, 16, 64] {
            for native_pressure in [None, Some("normal".into())] {
                let mut monitor = Monitor::default();
                for at in (100..=700).step_by(5) {
                    let mut s = memory_sample(at, "normal");
                    s.platform.memory_pressure = native_pressure.clone();
                    s.memory_total_bytes = total_gib << 30;
                    s.memory_available_bytes = s.memory_total_bytes / 2;
                    // Startup/cache/test-worker allocations grow well beyond the old rule.
                    s.workloads[0].memory_bytes = (1 << 30) + (at - 100) * (1 << 20);
                    assert!(memory_growth(&s.workloads[0]).is_some());
                    assert!(conditions(&s, &policy).is_empty());
                    assert!(monitor.observe(&s, &policy, false).is_none());
                }
            }
        }
    }

    #[test]
    fn memory_alerts_prefer_os_pressure_and_bound_the_headroom_fallback() {
        let policy = Policy::default();
        let mut s = memory_sample(100, "normal");
        s.memory_available_bytes = 1 << 30;
        assert!(conditions(&s, &policy).is_empty()); // native normal, cached pages are OK
        s.platform.memory_pressure = None;
        assert_eq!(conditions(&s, &policy)[0].kind, Kind::MemoryPressure);
        s.memory_total_bytes = 64 << 30;
        s.memory_available_bytes = 4 << 30;
        assert!(conditions(&s, &policy).is_empty()); // low percentage, ample absolute room
        s.memory_total_bytes = 8 << 30;
        s.memory_available_bytes = 1 << 30;
        assert!(conditions(&s, &policy).is_empty()); // 12.5%, above configured threshold
        s.memory_available_bytes = 512 << 20;
        assert_eq!(conditions(&s, &policy)[0].kind, Kind::MemoryPressure);
        s.platform.memory_pressure = Some("normal".into());
        s.swap_used_bytes = 8 << 30;
        assert!(conditions(&s, &policy).is_empty()); // old swap is not current pressure
        s.swap_growth_bytes = 257 << 20;
        assert_eq!(conditions(&s, &policy)[0].kind, Kind::MemoryPressure);
    }

    #[test]
    fn memory_pressure_needs_thirty_seconds_and_growth_is_only_context() {
        let policy = Policy::default();
        let mut monitor = Monitor::default();
        for at in (100..130).step_by(5) {
            assert!(monitor
                .observe(&memory_sample(at, "warning"), &policy, false)
                .is_none());
        }
        let mut s = memory_sample(130, "warning");
        // A second growing app is part of the same machine-wide memory episode.
        let mut second = s.workloads[0].clone();
        second.id = "another-workload".into();
        second.memory_bytes = 2 << 30;
        s.workloads.push(second);
        let trigger = monitor.observe(&s, &policy, false).unwrap();
        assert_eq!(trigger.kind, Kind::MemoryPressure);
        assert_eq!(
            trigger.target.as_ref().unwrap().workload_id,
            "another-workload"
        );
        assert!(trigger.reason.contains("reported memory also rose"));
        assert_eq!(conditions(&s, &policy).len(), 1);
        assert!(proposal(&s, &trigger).is_none()); // priority changes cannot free memory
        assert!(still_present(&s, &policy, &trigger));
        s.platform.memory_pressure = Some("normal".into());
        assert!(!still_present(&s, &policy, &trigger)); // allocation can remain high
    }

    #[test]
    fn brief_pressure_or_swap_bursts_clear_without_a_popup() {
        let policy = Policy::default();
        for pressure in ["warning", "normal"] {
            let mut monitor = Monitor::default();
            for at in (100..=400).step_by(5) {
                let mut s = memory_sample(at, pressure);
                if pressure == "normal" {
                    s.swap_growth_bytes = 300 << 20;
                }
                if at % 30 == 0 {
                    s.platform.memory_pressure = Some("normal".into());
                    s.swap_growth_bytes = 0;
                }
                assert!(monitor.observe(&s, &policy, false).is_none());
            }
        }
    }

    #[test]
    fn critical_memory_pressure_keeps_the_faster_sustained_alert() {
        let policy = Policy::default();
        let mut monitor = Monitor::default();
        for at in [100, 105] {
            assert!(monitor
                .observe(&memory_sample(at, "critical"), &policy, false)
                .is_none());
        }
        assert_eq!(
            monitor
                .observe(&memory_sample(110, "critical"), &policy, false)
                .unwrap()
                .kind,
            Kind::MemoryPressure
        );
    }

    #[test]
    fn repeated_memory_alerts_require_real_recovery_and_fifteen_minute_cooldown() {
        let policy = Policy::default();
        let mut monitor = Monitor::default();
        for at in (100..=130).step_by(5) {
            let event = monitor.observe(&memory_sample(at, "warning"), &policy, false);
            assert_eq!(event.is_some(), at == 130);
        }
        // Brief normal readings must not rearm the alert, even beyond the cooldown.
        for at in (135..=1100).step_by(5) {
            let state = if at % 60 < 20 { "normal" } else { "warning" };
            assert!(monitor
                .observe(&memory_sample(at, state), &policy, false)
                .is_none());
        }
        // A full minute of observed recovery permits a new episode.
        for at in (1105..=1165).step_by(5) {
            assert!(monitor
                .observe(&memory_sample(at, "normal"), &policy, false)
                .is_none());
        }
        for at in (1170..=1200).step_by(5) {
            let event = monitor.observe(&memory_sample(at, "warning"), &policy, false);
            assert_eq!(event.is_some(), at == 1200);
        }
        // Recovery alone is insufficient: the second episode retains its cooldown.
        for at in (1205..=1270).step_by(5) {
            assert!(monitor
                .observe(&memory_sample(at, "normal"), &policy, false)
                .is_none());
        }
        for at in (1275..=2100).step_by(5) {
            let event = monitor.observe(&memory_sample(at, "warning"), &policy, false);
            assert_eq!(event.is_some(), at == 2100);
        }
    }

    #[test]
    fn sampling_gaps_and_pause_do_not_erase_memory_cooldowns_or_count_as_recovery() {
        let mut policy = Policy::default();
        let mut monitor = Monitor::default();
        for at in [100, 105, 110] {
            monitor.observe(&memory_sample(at, "warning"), &policy, false);
        }
        // Sleep during a pending condition restarts the thirty-second observation.
        for at in (200..=230).step_by(5) {
            let event = monitor.observe(&memory_sample(at, "warning"), &policy, false);
            assert_eq!(event.is_some(), at == 230);
        }
        policy.enabled = false;
        assert!(monitor
            .observe(&memory_sample(235, "normal"), &policy, false)
            .is_none());
        policy.enabled = true;
        // A long sleep is not evidence of recovery, even once the cooldown has elapsed.
        for at in (2000..=2100).step_by(5) {
            assert!(monitor
                .observe(&memory_sample(at, "warning"), &policy, false)
                .is_none());
        }
    }
    #[test]
    fn ordinary_app_wakeups_do_not_interrupt_even_on_many_core_machines() {
        for cores in [4, 12, 16, 64] {
            let mut monitor = Monitor::default();
            let mut s = sample(100, 2.0);
            s.logical_cpus = cores;
            s.workloads[0].history = vec![
                ResourcePoint {
                    cpu_percent: 0.01,
                    memory_bytes: 32 << 20,
                    ..Default::default()
                };
                12
            ];
            for at in (100..200).step_by(5) {
                s.captured_at = at;
                assert!(conditions(&s, &Policy::default()).is_empty());
                assert!(monitor.observe(&s, &Policy::default(), false).is_none());
            }
            // Whole-system pressure still alerts even when no one app is large.
            s.cpu_percent = 90.0;
            assert!(conditions(&s, &Policy::default())
                .iter()
                .any(|t| t.kind == Kind::SystemCpu));
        }
    }

    #[test]
    fn substantial_anomalies_are_sustained_and_rechecked_against_current_limits() {
        let mut policy = Policy {
            app_cpu_core_percent: 1600.0,
            ..Default::default()
        };
        let mut s = sample(100, 12.0);
        s.workloads[0].history = vec![
            ResourcePoint {
                cpu_percent: 0.1,
                memory_bytes: 32 << 20,
                ..Default::default()
            };
            12
        ];
        let mut monitor = Monitor::default();
        assert!(monitor.observe(&s, &policy, false).is_none());
        s.captured_at += 5;
        assert!(monitor.observe(&s, &policy, false).is_none());
        s.captured_at += 5;
        let trigger = monitor.observe(&s, &policy, false).unwrap();
        assert_eq!(trigger.kind, Kind::CpuAnomaly);
        assert!(still_present(&s, &policy, &trigger));
        policy.app_cpu_percent = 20.0;
        assert!(!still_present(&s, &policy, &trigger));
        assert!(conditions(&s, &policy).is_empty());
        policy.app_cpu_percent = 2.0;
        assert!(policy.validate().is_err());
        let migrated: Policy = serde_json::from_str(r#"{"app_cpu_core_percent": 150.0}"#).unwrap();
        assert_eq!(migrated.app_cpu_percent, 10.0);
        assert_eq!(migrated.app_cpu_core_percent, 150.0);
        assert!(migrated.rules.is_empty());
    }

    #[test]
    fn permissions_are_workload_scoped_and_never_allow_termination() {
        let s = sample(100, 15.0);
        let mut p = Policy::default();
        let trigger = conditions(&s, &p).remove(0);
        let mut action = proposal(&s, &trigger).unwrap();
        assert!(!p.allows(&action));
        p.rules.push(Rule {
            workload_id: "fixture".into(),
            label: "Example".into(),
        });
        assert!(p.allows(&action));
        action.action = "terminate".into();
        assert!(!p.allows(&action));
        action.action = "lower_priority".into();
        action.process.workload_id = "different".into();
        assert!(!p.allows(&action));
        let mut protected = s.clone();
        protected.processes[0].actions.clear();
        assert!(proposal(&protected, &trigger).is_none());
    }
    #[test]
    fn verification_does_not_invent_improvements_or_fresh_evidence() {
        let s = sample(100, 15.0);
        let mut r = ActionReport {
            action: "lower_priority".into(),
            message: "sent".into(),
            before: s.processes[0].clone(),
            after: Some(s.processes[0].clone()),
            measured_after_seconds: 10,
            fresh: true,
        };
        let v = verify(&s, &s, &r, true);
        assert!(v.summary.contains("No clear reduction"));
        assert!(verify(&s, &s, &r, false)
            .summary
            .contains("could not be verified"));
        r.fresh = false;
        let v = verify(&s, &s, &r, true);
        assert!(v.system_cpu_after.is_none());
        assert!(v.summary.contains("unverified"));
    }
    #[test]
    fn legacy_settings_migrate_without_granting_action_permissions() {
        let s: crate::settings::Settings = serde_json::from_str("{}").unwrap();
        assert!(s.automation.enabled);
        assert!(s.automation.rules.is_empty());
        let mut p = Policy {
            system_cpu_percent: f32::NAN,
            ..Default::default()
        };
        assert!(p.validate().is_err());
        p.system_cpu_percent = 85.0;
        p.rules = vec![
            Rule {
                workload_id: "same".into(),
                label: "Example".into()
            };
            2
        ];
        assert!(p.validate().is_err());
    }
}
