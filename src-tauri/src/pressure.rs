use crate::types::{Incident, Pressure, Snapshot};

#[derive(Default)]
pub struct Detector {
    samples: u32,
    last_alert: Option<u64>,
    incident: Option<Incident>,
}

impl Detector {
    pub fn update(&mut self, snapshot: &Snapshot) -> (Pressure, Option<Incident>) {
        let mut reasons = Vec::new();
        if snapshot.sample_ready && snapshot.cpu_percent >= 85.0 {
            reasons.push("Total CPU use has reached 85% of machine capacity.".into());
        }
        if snapshot.memory_total_bytes > 0
            && snapshot.memory_available_bytes as f64 / (snapshot.memory_total_bytes as f64) < 0.12
        {
            reasons.push("Less than 12% of physical memory is available.".into());
        }
        if matches!(
            snapshot.platform.memory_pressure.as_deref(),
            Some("warning" | "critical")
        ) {
            reasons.push("The operating system reports memory pressure.".into());
        }
        if matches!(
            snapshot.platform.thermal.as_deref(),
            Some("serious" | "critical")
        ) {
            reasons.push("The operating system reports thermal pressure.".into());
        }
        if snapshot.swap_growth_bytes > 256 * 1024 * 1024 {
            reasons.push("Swap use grew by over 256 MiB since the last sample.".into());
        }
        self.samples = if reasons.is_empty() {
            0
        } else {
            self.samples.saturating_add(1)
        };
        let level = if self.samples >= 3 {
            "high"
        } else if self.samples > 0 {
            "watch"
        } else {
            "normal"
        };
        if self.samples >= 3
            && self
                .last_alert
                .is_none_or(|last| snapshot.captured_at.saturating_sub(last) >= 300)
        {
            self.last_alert = Some(snapshot.captured_at);
            self.incident = Some(Incident {
                id: snapshot.captured_at,
                created_at: snapshot.captured_at,
                title: "Your computer is under sustained pressure".into(),
                reasons: reasons.clone(),
                workload_id: snapshot.workloads.first().map(|w| w.id.clone()),
            });
        }
        (
            Pressure {
                level: level.into(),
                reasons,
                sustained_samples: self.samples,
            },
            self.incident.clone(),
        )
    }
}

pub fn local_eligible(snapshot: &Snapshot) -> Result<(), String> {
    if !snapshot.sample_ready {
        return Err("Waiting for a complete resource sample.".into());
    }
    if snapshot.captured_at + 20 < crate::types::now() {
        return Err("Telemetry is stale; local inference is deferred.".into());
    }
    if snapshot.memory_available_bytes < 4 * 1024 * 1024 * 1024
        || snapshot.memory_available_bytes as f64 / (snapshot.memory_total_bytes.max(1) as f64)
            < 0.20
    {
        return Err(
            "Local inference is deferred: keep at least 4 GiB and 20% of memory available.".into(),
        );
    }
    if snapshot.cpu_percent >= 75.0
        || snapshot.pressure.level != "normal"
        || matches!(
            snapshot.platform.thermal.as_deref(),
            Some("fair" | "serious" | "critical")
        )
    {
        return Err("Local inference is deferred while the computer is busy or warm.".into());
    }
    Ok(())
}

/// Exercises the actual detector without placing load on the host computer.
pub fn simulated_incident() -> Incident {
    let mut detector = Detector::default();
    let mut snapshot = Snapshot {
        source: "demo".into(),
        captured_at: crate::types::now(),
        sample_ready: true,
        cpu_percent: 92.0,
        memory_total_bytes: 16 << 30,
        memory_available_bytes: 1 << 30,
        ..Default::default()
    };
    for _ in 0..2 {
        detector.update(&snapshot);
        snapshot.captured_at += 5;
    }
    let mut incident = detector
        .update(&snapshot)
        .1
        .expect("Three pressure samples produce an incident");
    incident.title = "Demo: your computer is under sustained pressure".into();
    incident.reasons.insert(
        0,
        "Synthetic samples passed through the real detector. No host load was generated.".into(),
    );
    incident
}

#[cfg(test)]
mod tests {
    use super::*;
    fn healthy() -> Snapshot {
        Snapshot {
            source: "test".into(),
            captured_at: crate::types::now(),
            sample_ready: true,
            cpu_percent: 10.0,
            memory_total_bytes: 16 << 30,
            memory_available_bytes: 8 << 30,
            pressure: Pressure {
                level: "normal".into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
    #[test]
    fn sustained_and_cooldown() {
        let mut d = Detector::default();
        let mut s = healthy();
        s.cpu_percent = 90.0;
        assert!(d.update(&s).1.is_none());
        s.captured_at += 5;
        assert!(d.update(&s).1.is_none());
        s.captured_at += 5;
        let first = d.update(&s).1.unwrap();
        s.captured_at += 5;
        assert_eq!(d.update(&s).1.unwrap().id, first.id);
        s.cpu_percent = 5.0;
        assert_eq!(d.update(&s).0.level, "normal");
    }
    #[test]
    fn local_guard_uses_available_memory_and_thermal_state() {
        let mut s = healthy();
        assert!(local_eligible(&s).is_ok());
        s.memory_available_bytes = 3 << 30;
        assert!(local_eligible(&s).is_err());
        s.memory_available_bytes = 8 << 30;
        s.platform.thermal = Some("fair".into());
        assert!(local_eligible(&s).is_err());
        s.platform.thermal = None;
        s.captured_at -= 30;
        assert!(local_eligible(&s).is_err());
    }
}
