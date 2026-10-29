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
    pub sustained_samples: u32,
    pub cooldown_seconds: u64,
    pub max_investigations_per_hour: usize,
    pub rules: Vec<Rule>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            model_analysis: true,
            system_cpu_percent: 85.0,
            available_memory_percent: 12.0,
            app_cpu_core_percent: 100.0,
            sustained_samples: 3,
            cooldown_seconds: 300,
            max_investigations_per_hour: 4,
            rules: vec![],
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
    pub report: Option<ActionReport>,
    pub verification: Option<Verification>,
    pub automatic: bool,
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
            report: None,
            verification: None,
            automatic: false,
        };
        case.transition(
            Stage::Detected,
            "A sustained threshold or unusual workload change was detected.",
        );
        case
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
    healthy: u32,
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
    let memory_target = s
        .workloads
        .iter()
        .max_by_key(|w| w.memory_bytes)
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
    let available = s.memory_available_bytes as f64 / s.memory_total_bytes.max(1) as f64 * 100.0;
    if (s.memory_total_bytes > 0 && available < policy.available_memory_percent as f64)
        || matches!(
            s.platform.memory_pressure.as_deref(),
            Some("warning" | "critical")
        )
        || s.swap_growth_bytes > 256 * 1024 * 1024
    {
        found.push(Trigger { cpu_floor_percent: None, memory_floor_bytes: None, key: "memory-pressure".into(), kind: Kind::MemoryPressure, title: "Memory needs attention".into(), reason: "Available memory, OS memory pressure, or active swap growth crossed a limit. Summed app memory is attribution evidence, not proof of a leak.".into(), target: memory_target });
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
        let core_cpu = w.cpu_percent * s.logical_cpus.max(1) as f32;
        if core_cpu >= policy.app_cpu_core_percent {
            found.push(Trigger { cpu_floor_percent: None, memory_floor_bytes: None, key: format!("app-cpu:{}", w.id), kind: Kind::AppCpu, title: format!("{} is using sustained CPU", w.label), reason: format!("Workload CPU exceeded {:.0}% on a one-core scale (100% = one logical core). High usage may be legitimate work.", policy.app_cpu_core_percent), target: Some(target(w)) });
        }
        // A fixed earlier window makes a step change detectable across consecutive samples.
        // Require at least six baseline samples, separated from the most recent three.
        if w.history.len() >= 9 {
            let baseline: Vec<_> = w.history.iter().take(w.history.len() - 3).collect();
            let cpu = baseline.iter().map(|p| p.cpu_percent).sum::<f32>() / baseline.len() as f32;
            let memory =
                baseline.iter().map(|p| p.memory_bytes as f64).sum::<f64>() / baseline.len() as f64;
            if core_cpu >= 25.0
                && w.cpu_percent >= cpu.max(0.1) * 3.0
                && (w.cpu_percent - cpu) * s.logical_cpus.max(1) as f32 >= 25.0
            {
                found.push(Trigger { cpu_floor_percent: Some((cpu.max(0.1) * 3.0).max(cpu + 25.0 / s.logical_cpus.max(1) as f32)), memory_floor_bytes: None, key: format!("cpu-anomaly:{}",w.id), kind: Kind::CpuAnomaly, title: format!("{} has unusual CPU activity",w.label), reason: "CPU is at least three times its recent baseline, with an increase of at least 25% of one core. This is an anomaly to investigate, not a diagnosis.".into(), target: Some(target(w)) });
            }
            if w.memory_bytes as f64 >= memory * 1.5
                && w.memory_bytes as f64 - memory >= 512.0 * 1048576.0
            {
                found.push(Trigger { cpu_floor_percent: None, memory_floor_bytes: Some((memory * 1.5).max(memory + 512.0 * 1048576.0) as u64), key: format!("memory-growth:{}",w.id), kind: Kind::MemoryGrowth, title: format!("{} memory grew unusually",w.label), reason: "Summed resident memory grew at least 50% and 512 MiB above its recent baseline. Shared pages can be counted more than once.".into(), target: Some(target(w)) });
            }
        }
    }
    found
}

impl Monitor {
    pub fn observe(&mut self, s: &Snapshot, policy: &Policy, busy: bool) -> Option<Trigger> {
        if s.captured_at <= self.last_sample {
            return None;
        }
        let gap = s.captured_at.saturating_sub(self.last_sample);
        self.last_sample = s.captured_at;
        if !policy.enabled || !s.sample_ready || gap > 20 {
            self.tracks.clear();
            if !policy.enabled || !s.sample_ready {
                return None;
            }
        }
        let found = conditions(s, policy);
        self.tracks.retain(|key, track| {
            if !found.iter().any(|t| &t.key == key) {
                track.consecutive = 0;
                track.healthy += 1;
                if track.healthy >= 3 {
                    track.fired = false;
                }
            }
            track.healthy < 120
                || s.captured_at.saturating_sub(track.last_fired) < policy.cooldown_seconds
        });
        for trigger in &found {
            let track = self.tracks.entry(trigger.key.clone()).or_default();
            track.consecutive = track.consecutive.saturating_add(1);
            track.healthy = 0;
        }
        while self
            .recent
            .front()
            .is_some_and(|at| s.captured_at.saturating_sub(*at) >= 3600)
        {
            self.recent.pop_front();
        }
        if busy
            || self.snoozed_until > s.captured_at
            || self.recent.len() >= policy.max_investigations_per_hour
            || self
                .last_dispatch
                .is_some_and(|at| s.captured_at.saturating_sub(at) < 60)
        {
            return None;
        }
        for trigger in found {
            let track = self.tracks.get_mut(&trigger.key).unwrap();
            if track.consecutive >= policy.sustained_samples
                && !track.fired
                && (track.last_fired == 0
                    || s.captured_at.saturating_sub(track.last_fired) >= policy.cooldown_seconds)
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

pub fn still_present(s: &Snapshot, policy: &Policy, trigger: &Trigger) -> bool {
    if let Some(w) = trigger
        .target
        .as_ref()
        .and_then(|t| s.workloads.iter().find(|w| w.id == t.workload_id))
    {
        if let Some(floor) = trigger.cpu_floor_percent {
            return w.cpu_percent >= floor;
        }
        if let Some(floor) = trigger.memory_floor_bytes {
            return w.memory_bytes >= floor;
        }
    }
    conditions(s, policy).iter().any(|t| t.key == trigger.key)
}

pub fn proposal(s: &Snapshot, trigger: &Trigger) -> Option<Proposal> {
    if matches!(trigger.kind, Kind::MemoryGrowth | Kind::MemoryPressure) {
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
    Some(Proposal { process, action: "lower_priority".into(), rationale: "Give competing work more CPU scheduling preference by lowering this busy process’s priority. This does not free memory or guarantee lower CPU usage. Priority stays lowered until the process exits; restoring it may require OS privileges.".into() })
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
        let mut s = sample(100, 6.0);
        let p = Policy::default();
        assert!(conditions(&s, &p).is_empty());
        s.workloads[0].history = (0..9)
            .map(|i| ResourcePoint {
                cpu_percent: if i < 6 { 0.1 } else { 6.0 },
                memory_bytes: 32 << 20,
                ..Default::default()
            })
            .collect();
        let trigger = conditions(&s, &p)
            .into_iter()
            .find(|t| t.kind == Kind::CpuAnomaly)
            .unwrap();
        for point in &mut s.workloads[0].history {
            point.cpu_percent = 6.0;
        }
        assert!(still_present(&s, &p, &trigger)); // recent baseline can adapt during inference
        s.workloads[0].cpu_percent = 0.2;
        assert!(!still_present(&s, &p, &trigger));
        s.workloads[0].memory_bytes = 1 << 30;
        let trigger = conditions(&s, &p)
            .into_iter()
            .find(|t| t.kind == Kind::MemoryGrowth)
            .unwrap();
        assert!(proposal(&s, &trigger).is_none()); // renice does not fix memory
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
