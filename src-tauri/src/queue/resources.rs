use crate::types::{now, Snapshot};
use serde::{Deserialize, Serialize};

pub const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Resources {
    pub captured_at: u64,
    pub ready: bool,
    pub cpu_percent: f32,
    pub available_bytes: u64,
    pub total_bytes: u64,
    pub thermal: Option<String>,
    pub memory_pressure: Option<String>,
    pub swap_growth_bytes: u64,
    pub consecutive_good: u32,
    pub suggestions: Vec<Suggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Suggestion {
    pub workload_id: String,
    pub label: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub recommendation: String,
}

impl Resources {
    pub fn from_snapshot(snapshot: &Snapshot, previous: &Self) -> Self {
        let mut value = Self {
            captured_at: snapshot.captured_at,
            ready: snapshot.source == "live" && snapshot.sample_ready,
            cpu_percent: snapshot.cpu_percent,
            available_bytes: snapshot.memory_available_bytes,
            total_bytes: snapshot.memory_total_bytes,
            thermal: snapshot.platform.thermal.clone(),
            memory_pressure: snapshot.platform.memory_pressure.clone(),
            swap_growth_bytes: snapshot.swap_growth_bytes,
            ..Default::default()
        };
        value.consecutive_good = if value.block_reason(0).is_some() {
            0
        } else if snapshot.captured_at == previous.captured_at {
            previous.consecutive_good
        } else if snapshot.captured_at > previous.captured_at
            && previous.captured_at.saturating_add(15) >= snapshot.captured_at
        {
            previous.consecutive_good.saturating_add(1)
        } else {
            1
        };
        let memory_busy = value.available_bytes < 4 * GIB
            || value.available_bytes as f64 / (value.total_bytes.max(1) as f64) < 0.25;
        let mut workloads: Vec<_> = snapshot
            .workloads
            .iter()
            .filter(|w| {
                w.category != "jitterbug"
                    && (w.cpu_percent >= 10.0 || (memory_busy && w.memory_bytes >= GIB / 2))
            })
            .collect();
        workloads.sort_by(|a, b| {
            let score = |w: &&crate::types::Workload| {
                w.cpu_percent as f64
                    + if memory_busy {
                        w.memory_bytes as f64 / GIB as f64 * 10.0
                    } else {
                        0.0
                    }
            };
            score(b).total_cmp(&score(a))
        });
        value.suggestions = workloads.into_iter().take(3).map(|w| Suggestion {
            workload_id: w.id.clone(), label: w.label.clone(), cpu_percent: w.cpu_percent,
            memory_bytes: w.memory_bytes,
            recommendation: match w.category.as_str() {
                "model" => "If unused, unload its local model. For your next inference, consider a cloud provider within your budget.".into(),
                "browser" => "Close unused tabs or pause background media after checking what you still need.".into(),
                "project" | "build" | "test" | "python" | "javascript" => "Defer optional work or reduce its workers. A configured remote runner could move this work off this computer; remote execution is not connected yet.".into(),
                "system" => "Inspect this service before changing it. System services may be essential; do not stop it based on usage alone.".into(),
                _ => "If you are not using this app, save your work and quit it, or pause its background activity in the app's settings.".into(),
            },
        }).collect();
        value
    }

    pub fn memory_reserve(&self) -> u64 {
        (2 * GIB).max(self.total_bytes / 5)
    }

    pub fn block_reason(&self, requested_bytes: u64) -> Option<String> {
        if !self.ready || self.captured_at > now() + 2 || self.captured_at + 15 < now() {
            return Some("Waiting for fresh telemetry. Keep Jitterbug open.".into());
        }
        if self.total_bytes == 0
            || self.available_bytes > self.total_bytes
            || self.available_bytes < requested_bytes.saturating_add(self.memory_reserve())
        {
            return Some(
                "Leaving memory for your apps: at least 2 GiB or 20% after reservations.".into(),
            );
        }
        if !self.cpu_percent.is_finite() || !(0.0..70.0).contains(&self.cpu_percent) {
            return Some("Waiting for total CPU use to fall below 70%.".into());
        }
        if matches!(
            self.memory_pressure.as_deref(),
            Some("warning" | "critical")
        ) {
            return Some("Waiting for OS memory pressure to recover.".into());
        }
        if matches!(
            self.thermal.as_deref(),
            Some("fair" | "serious" | "critical")
        ) {
            return Some("Waiting for the computer to cool down.".into());
        }
        if self.swap_growth_bytes >= 128 * 1024 * 1024 {
            return Some("Waiting for swap growth to settle.".into());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gates_include_whole_machine_and_missing_telemetry() {
        let mut r = Resources {
            ready: true,
            captured_at: now(),
            available_bytes: 8 * GIB,
            total_bytes: 16 * GIB,
            ..Default::default()
        };
        assert!(r.block_reason(2 * GIB).is_none());
        r.cpu_percent = 80.0;
        assert!(r.block_reason(0).unwrap().contains("CPU"));
        r.cpu_percent = 0.0;
        r.thermal = Some("fair".into());
        assert!(r.block_reason(0).unwrap().contains("cool"));
        r.thermal = None;
        r.available_bytes = 3 * GIB;
        assert!(r.block_reason(2 * GIB).unwrap().contains("memory"));
        r.captured_at = now() - 16;
        assert!(r.block_reason(0).unwrap().contains("fresh"));
    }
    #[test]
    fn recommendations_include_non_agent_workloads() {
        let snapshot = Snapshot {
            source: "live".into(),
            sample_ready: true,
            captured_at: now(),
            memory_total_bytes: 16 * GIB,
            memory_available_bytes: 3 * GIB,
            workloads: vec![crate::types::Workload {
                id: "music".into(),
                label: "Music app".into(),
                category: "application".into(),
                cpu_percent: 25.0,
                ..Default::default()
            }],
            ..Default::default()
        };
        let r = Resources::from_snapshot(&snapshot, &Resources::default());
        assert_eq!(r.suggestions[0].workload_id, "music");
        assert!(r.suggestions[0].recommendation.contains("save your work"));
    }
}
