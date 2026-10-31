//! Keep the computer responsive while many coding agents run. When memory or CPU stays tight,
//! the busiest working agent session is paused; once there is room again, held sessions resume
//! one at a time, in the order they were paused. Each step waits to see its effect first.
//!
//! Opt-in (`Policy::hold_agents`). Only agent sessions that are working are held: never one
//! waiting for the person, never the last one still working, and never one the person resumed
//! recently. It resumes only pauses it made, and pauses still expire on their own.
use crate::agent_sessions::Session;
use crate::pause::Paused;
use crate::types::Snapshot;
use serde::{Deserialize, Serialize};

/// Samples (five seconds apart) of strain before a session is paused.
const TIGHT_SAMPLES: u32 = 2;
/// Samples of room before a held session resumes.
const CALM_SAMPLES: u32 = 4;
/// After a pause, time to see its effect before pausing another.
const SETTLE_SECONDS: u64 = 15;
/// After any change, time before the next resume, so resumes do not pile up.
const RESUME_GAP_SECONDS: u64 = 30;
/// A session the person resumed, or one that could not be paused, is left alone this long.
const RESPECT_SECONDS: u64 = 10 * 60;
/// A held session resumes only if its CPU at pause time, added to the current load, stays
/// under this share of total CPU: resuming into a saturated processor just pauses it again.
const RESUME_CPU_PERCENT: f32 = 85.0;
/// Share of total CPU that counts as working for an agent without its own status.
const BUSY_CPU_PERCENT: f32 = 3.0;
const MIB: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strain {
    Memory,
    Cpu,
}

impl Strain {
    fn reason(self) -> &'static str {
        match self {
            Strain::Memory => "memory was running out",
            Strain::Cpu => "the processor was saturated",
        }
    }
}

fn available(s: &Snapshot) -> f64 {
    s.memory_available_bytes as f64 / s.memory_total_bytes.max(1) as f64
}

fn os_memory_pressure(s: &Snapshot) -> bool {
    matches!(
        s.platform.memory_pressure.as_deref(),
        Some("warning" | "critical")
    )
}

/// What is tight right now, memory first: it is what freezes the whole computer.
pub fn strain(s: &Snapshot) -> Option<Strain> {
    if os_memory_pressure(s) || available(s) < 0.10 || s.swap_growth_bytes > 256 * MIB {
        Some(Strain::Memory)
    } else if s.sample_ready && s.cpu_percent >= 90.0 {
        Some(Strain::Cpu)
    } else {
        None
    }
}

/// Clear room, well below the strain thresholds, so a resume does not tip it straight back.
fn calm(s: &Snapshot) -> bool {
    s.sample_ready
        && !os_memory_pressure(s)
        && available(s) >= 0.18
        && s.swap_growth_bytes < 64 * MIB
        && s.cpu_percent < 70.0
}

/// A session held for room.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Held {
    pub pid: u32,
    pub start_time: u64,
    pub title: String,
    pub reason: String,
    pub paused_at: u64,
    /// When the pause expires on its own if there is still no room.
    pub until: u64,
    /// Its share of total CPU when it was paused: what resuming it would add back.
    pub cpu_percent: f32,
}

/// What the person sees: whether it is on and what it is holding.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub enabled: bool,
    pub held: Vec<Held>,
    /// The last thing it did, in a sentence.
    pub note: String,
}

#[derive(Debug, PartialEq)]
pub enum Step {
    Idle,
    /// Pause this session; its times are filled in by `paused` once the pause is in place.
    Pause(Held),
    Resume(Vec<Held>),
}

#[derive(Default)]
pub struct Governor {
    tight: u32,
    calm: u32,
    last_change: u64,
    held: Vec<Held>,
    /// Sessions left alone until the given time.
    respected: Vec<((u32, u64), u64)>,
    note: String,
}

fn busy(s: &Session) -> bool {
    match s.status.as_deref() {
        Some("working") => true,
        Some(_) => false,
        None => s.cpu_percent >= BUSY_CPU_PERCENT,
    }
}

impl Governor {
    /// Whether this sample needs the agent sessions listed: only under strain or while holding.
    pub fn wants_sessions(&self, sample: &Snapshot) -> bool {
        !self.held.is_empty() || strain(sample).is_some()
    }

    pub fn view(&self, enabled: bool) -> View {
        View {
            enabled,
            held: self.held.clone(),
            note: self.note.clone(),
        }
    }

    fn respected(&self, id: (u32, u64), now: u64) -> bool {
        self.respected
            .iter()
            .any(|(r, until)| *r == id && *until > now)
    }

    fn respect(&mut self, id: (u32, u64), now: u64) {
        self.respected.retain(|(r, until)| *r != id && *until > now);
        self.respected.push((id, now + RESPECT_SECONDS));
    }

    /// Drop held sessions whose pause is gone. One resumed before it expired, while the
    /// session still runs, was resumed by the person, so it is left alone for a while.
    fn reconcile(&mut self, sessions: &[Session], paused: &[Paused], now: u64) {
        let mut resumed_by_person = vec![];
        self.held.retain(|h| {
            let still = paused.iter().any(|p| {
                p.pid == h.pid && p.start_time == h.start_time && p.paused_at == h.paused_at
            });
            let running = sessions
                .iter()
                .any(|s| s.pid == h.pid && s.start_time == h.start_time);
            if !still && running && now < h.until {
                resumed_by_person.push((h.pid, h.start_time));
            }
            still
        });
        for id in resumed_by_person {
            self.respect(id, now);
        }
    }

    /// Decide one step for this sample. `sessions` may be empty when `wants_sessions` was false.
    pub fn decide(
        &mut self,
        sample: &Snapshot,
        sessions: &[Session],
        paused: &[Paused],
        enabled: bool,
        snoozed: bool,
        now: u64,
    ) -> Step {
        if sample.source != "live" || !sample.sample_ready {
            return Step::Idle;
        }
        self.reconcile(sessions, paused, now);
        if !enabled {
            self.tight = 0;
            self.calm = 0;
            if self.held.is_empty() {
                return Step::Idle;
            }
            self.note = "Turned off: every held session resumed.".into();
            return Step::Resume(std::mem::take(&mut self.held));
        }
        let strain = strain(sample);
        match strain {
            Some(_) => {
                self.tight += 1;
                self.calm = 0;
            }
            None if calm(sample) => {
                self.calm += 1;
                self.tight = 0;
            }
            None => {
                self.tight = 0;
                self.calm = 0;
            }
        }
        if let Some(strain) = strain.filter(|_| self.tight >= TIGHT_SAMPLES) {
            if snoozed || now < self.last_change + SETTLE_SECONDS {
                return Step::Idle;
            }
            let working: Vec<&Session> = sessions
                .iter()
                .filter(|s| s.agent.is_some() && s.paused_until.is_none() && busy(s))
                .collect();
            // The last working agent keeps going: pausing everything is no better than a freeze.
            if working.len() < 2 {
                return Step::Idle;
            }
            let pick = working
                .into_iter()
                .filter(|s| s.actions.iter().any(|a| a == "pause"))
                .filter(|s| !self.respected(s.key(), now))
                .max_by(|a, b| match strain {
                    Strain::Memory => a.memory_bytes.cmp(&b.memory_bytes),
                    Strain::Cpu => a.cpu_percent.total_cmp(&b.cpu_percent),
                });
            let Some(session) = pick else {
                return Step::Idle;
            };
            self.tight = 0;
            self.last_change = now;
            return Step::Pause(Held {
                pid: session.pid,
                start_time: session.start_time,
                title: session.title.clone(),
                reason: strain.reason().into(),
                cpu_percent: session.cpu_percent,
                ..Default::default()
            });
        }
        if self.calm >= CALM_SAMPLES && now >= self.last_change + RESUME_GAP_SECONDS {
            // Oldest first, skipping any whose load would saturate the processor again.
            let fits = self
                .held
                .iter()
                .position(|h| sample.cpu_percent + h.cpu_percent < RESUME_CPU_PERCENT);
            if let Some(index) = fits {
                self.calm = 0;
                self.last_change = now;
                let next = self.held.remove(index);
                self.note = format!("Resumed {}: there is room again.", next.title);
                return Step::Resume(vec![next]);
            }
        }
        Step::Idle
    }

    /// Record a pause the step asked for once it is in place.
    pub fn paused(&mut self, record: &Paused, held: Held) {
        self.note = format!(
            "Paused {} because {}. It resumes when there is room.",
            held.title, held.reason
        );
        self.held.push(Held {
            pid: record.pid,
            start_time: record.start_time,
            paused_at: record.paused_at,
            until: record.until,
            ..held
        });
    }

    /// A pause the operating system or the pause rules refused is not retried for a while.
    pub fn refused(&mut self, pid: u32, start_time: u64, now: u64) {
        self.respect((pid, start_time), now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Pressure;

    const GIB: u64 = 1 << 30;

    fn sample(available_gib: u64, cpu: f32) -> Snapshot {
        Snapshot {
            source: "live".into(),
            sample_ready: true,
            cpu_percent: cpu,
            memory_total_bytes: 16 * GIB,
            memory_available_bytes: available_gib * GIB,
            pressure: Pressure {
                level: "normal".into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn session(pid: u32, status: Option<&str>, cpu: f32, memory_gib: u64) -> Session {
        Session {
            id: format!("{pid}-1"),
            app: "Visual Studio Code".into(),
            agent: Some("Claude Code".into()),
            command: "claude".into(),
            title: format!("S{pid} · Claude Code"),
            status: status.map(Into::into),
            task: None,
            folder: "~/CODE".into(),
            path: "/Users/example/CODE".into(),
            started_at: 1,
            cpu_percent: cpu,
            memory_bytes: memory_gib * GIB,
            process_count: 3,
            paused_until: None,
            actions: vec!["pause".into()],
            connected: false,
            pid,
            start_time: 1,
        }
    }

    fn record(pid: u32, at: u64) -> Paused {
        Paused {
            pid,
            start_time: 1,
            label: String::new(),
            members: vec![(pid, 1)],
            shell: None,
            paused_at: at,
            until: at + crate::pause::LIMIT_SECONDS,
            watchdog: None,
        }
    }

    fn held(title: &str, cpu: f32) -> Held {
        Held {
            title: title.into(),
            cpu_percent: cpu,
            ..Default::default()
        }
    }

    /// Run samples until a non-idle step, at five-second intervals from `now`.
    fn run(
        g: &mut Governor,
        s: &Snapshot,
        sessions: &[Session],
        paused: &[Paused],
        now: &mut u64,
        limit: usize,
    ) -> Step {
        for _ in 0..limit {
            *now += 5;
            let step = g.decide(s, sessions, paused, true, false, *now);
            if step != Step::Idle {
                return step;
            }
        }
        Step::Idle
    }

    #[test]
    fn sustained_memory_strain_pauses_the_largest_working_session() {
        let mut g = Governor::default();
        let mut now = 1_000;
        let sessions = [
            session(10, Some("working"), 20.0, 2),
            session(11, Some("working"), 5.0, 6),
            // Biggest of all, but waiting for the person: never paused.
            session(12, Some("idle"), 0.0, 9),
        ];
        let tight = sample(1, 50.0);
        now += 5;
        assert_eq!(
            g.decide(&tight, &sessions, &[], true, false, now),
            Step::Idle
        );
        match run(&mut g, &tight, &sessions, &[], &mut now, 1) {
            Step::Pause(held) => {
                assert_eq!(held.pid, 11);
                assert_eq!(held.reason, "memory was running out");
            }
            other => panic!("expected a pause, got {other:?}"),
        }
    }

    #[test]
    fn cpu_strain_pauses_the_busiest_and_keeps_the_last_one_going() {
        let mut g = Governor::default();
        let mut now = 1_000;
        let mut sessions = vec![
            session(10, Some("working"), 40.0, 1),
            session(11, None, 30.0, 1),
        ];
        let hot = sample(8, 95.0);
        let Step::Pause(Held { pid, .. }) = run(&mut g, &hot, &sessions, &[], &mut now, 3) else {
            panic!("expected a pause");
        };
        assert_eq!(pid, 10);
        g.paused(&record(10, now), held("S10", 0.0));
        sessions[0].paused_until = Some(now + 900);
        // Only one working session is left: it keeps running however long the strain lasts.
        assert_eq!(
            run(&mut g, &hot, &sessions, &[record(10, now)], &mut now, 20),
            Step::Idle
        );
    }

    #[test]
    fn room_resumes_held_sessions_one_at_a_time_in_pause_order() {
        let mut g = Governor::default();
        let mut now = 1_000;
        let paused = [record(10, 990), record(11, 995)];
        g.paused(&paused[0], held("first", 0.0));
        g.paused(&paused[1], held("second", 0.0));
        let sessions = [session(10, None, 0.0, 1), session(11, None, 0.0, 1)];
        let roomy = sample(8, 20.0);
        let Step::Resume(first) = run(&mut g, &roomy, &sessions, &paused, &mut now, 10) else {
            panic!("expected a resume");
        };
        assert_eq!(first[0].title, "first");
        let resumed_at = now;
        let Step::Resume(second) = run(&mut g, &roomy, &sessions, &paused[1..], &mut now, 20)
        else {
            panic!("expected a second resume");
        };
        assert_eq!(second[0].title, "second");
        assert!(now >= resumed_at + RESUME_GAP_SECONDS);
    }

    #[test]
    fn a_session_the_person_resumed_is_left_alone() {
        let mut g = Governor::default();
        let mut now = 1_000;
        g.paused(&record(10, now), held("S10", 0.0));
        let sessions = [
            session(10, Some("working"), 50.0, 8),
            session(11, Some("working"), 10.0, 1),
            session(12, Some("working"), 10.0, 1),
        ];
        // The pause is gone before it expired while the session still runs.
        let tight = sample(1, 95.0);
        let Step::Pause(Held { pid, .. }) = run(&mut g, &tight, &sessions, &[], &mut now, 5) else {
            panic!("expected a pause");
        };
        assert_ne!(pid, 10);
        assert!(g.held.is_empty());
    }

    #[test]
    fn turning_it_off_resumes_everything_it_holds() {
        let mut g = Governor::default();
        let paused = [record(10, 990), record(11, 995)];
        g.paused(&paused[0], held("a", 0.0));
        g.paused(&paused[1], held("b", 0.0));
        let Step::Resume(all) = g.decide(&sample(1, 99.0), &[], &paused, false, false, 1_000)
        else {
            panic!("expected every held session to resume");
        };
        assert_eq!(all.len(), 2);
        assert!(g.view(false).held.is_empty());
    }

    #[test]
    fn snoozed_or_settling_means_no_new_pauses() {
        let mut g = Governor::default();
        let sessions = [
            session(10, Some("working"), 40.0, 1),
            session(11, Some("working"), 30.0, 1),
        ];
        let hot = sample(8, 95.0);
        let mut now = 1_000;
        for _ in 0..10 {
            now += 5;
            assert_eq!(g.decide(&hot, &sessions, &[], true, true, now), Step::Idle);
        }
        assert!(!g.wants_sessions(&sample(8, 20.0)));
        assert!(g.wants_sessions(&hot));
    }

    #[test]
    fn a_session_whose_load_would_saturate_the_processor_stays_held() {
        let mut g = Governor::default();
        let mut now = 1_000;
        let paused = [record(10, 990), record(11, 995)];
        g.paused(&paused[0], held("heavy", 45.0));
        g.paused(&paused[1], held("light", 10.0));
        let sessions = [session(10, None, 0.0, 1), session(11, None, 0.0, 1)];
        // 55% now: the heavy one would bring it to 100%, the light one to 65%.
        let busy = sample(8, 55.0);
        let Step::Resume(first) = run(&mut g, &busy, &sessions, &paused, &mut now, 10) else {
            panic!("expected the light session to resume");
        };
        assert_eq!(first[0].title, "light");
        assert_eq!(
            run(&mut g, &busy, &sessions, &paused[..1], &mut now, 40),
            Step::Idle
        );
        assert_eq!(g.view(true).held.len(), 1);
    }
}
