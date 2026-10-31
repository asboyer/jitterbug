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
/// A session paused again this soon after the governor resumed it is held longer each time.
const REPAUSE_WINDOW_SECONDS: u64 = 2 * 60;
const FIRST_BACKOFF_SECONDS: u64 = 60;
const MAX_BACKOFF_SECONDS: u64 = 10 * 60;
/// Under memory strain, only an agent whose memory grew at least this much since the previous
/// sample is paused: pausing stops growth but frees nothing, so a steady session is no help.
const GROWING_BYTES: u64 = 32 * MIB;
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

/// What is tight right now, memory first: it is what freezes the whole computer. A pause stops
/// growth but frees nothing, so memory counts only while it is still getting worse: swap
/// growing, or available memory falling since the previous sample. macOS can report a warning
/// level for minutes after growth stops; pausing more sessions then would not help.
pub fn strain(s: &Snapshot, previous_available: Option<u64>) -> Option<Strain> {
    let falling = previous_available.is_some_and(|before| {
        before.saturating_sub(s.memory_available_bytes) >= s.memory_total_bytes / 100
    }) || s.swap_growth_bytes > 64 * MIB;
    let tight = os_memory_pressure(s) || available(s) < 0.10;
    if (tight && falling) || s.swap_growth_bytes > 256 * MIB {
        Some(Strain::Memory)
    } else if s.sample_ready && s.cpu_percent >= 90.0 {
        Some(Strain::Cpu)
    } else {
        None
    }
}

/// Clear room, above the strain thresholds, so a resume does not tip it straight back. Held
/// sessions keep their memory, so the memory margin is modest: growth after a resume is what
/// matters, and a session that tips it back is held longer next time.
fn calm(s: &Snapshot) -> bool {
    s.sample_ready
        && s.platform.memory_pressure.as_deref() != Some("critical")
        && available(s) >= 0.15
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
    /// Minimum time held before it may resume: longer each time a resume tipped it back.
    pub backoff_seconds: u64,
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
    /// Available memory in the previous sample, to tell growth from a steady tight state.
    previous_available: Option<u64>,
    /// Each listed session's memory in the previous sample, to find the one growing.
    previous_memory: std::collections::HashMap<(u32, u64), u64>,
    /// Sessions the governor resumed: when, and the backoff they were held under.
    resumed: Vec<((u32, u64), u64, u64)>,
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
        !self.held.is_empty() || strain(sample, self.previous_available).is_some()
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

    /// Decide one step for this sample. `sessions` may be empty when `wants_sessions` was false;
    /// their `memory_bytes` should be each session's physical footprint where available.
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
        let previous_memory = std::mem::replace(
            &mut self.previous_memory,
            sessions.iter().map(|s| (s.key(), s.memory_bytes)).collect(),
        );
        let growth = |s: &Session| {
            previous_memory
                .get(&s.key())
                .map_or(0, |before| s.memory_bytes.saturating_sub(*before))
        };
        if !enabled {
            self.previous_available = Some(sample.memory_available_bytes);
            self.tight = 0;
            self.calm = 0;
            if self.held.is_empty() {
                return Step::Idle;
            }
            self.note = "Turned off: every held session resumed.".into();
            return Step::Resume(std::mem::take(&mut self.held));
        }
        let strain = strain(sample, self.previous_available);
        self.previous_available = Some(sample.memory_available_bytes);
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
                .filter(|s| strain == Strain::Cpu || growth(s) >= GROWING_BYTES)
                .max_by(|a, b| match strain {
                    Strain::Memory => growth(a).cmp(&growth(b)),
                    Strain::Cpu => a.cpu_percent.total_cmp(&b.cpu_percent),
                });
            let Some(session) = pick else {
                return Step::Idle;
            };
            self.tight = 0;
            self.last_change = now;
            self.resumed
                .retain(|(_, at, _)| now < at + REPAUSE_WINDOW_SECONDS);
            let backoff = self
                .resumed
                .iter()
                .find(|(id, _, _)| *id == session.key())
                .map_or(0, |(_, _, last)| {
                    (last * 2).clamp(FIRST_BACKOFF_SECONDS, MAX_BACKOFF_SECONDS)
                });
            return Step::Pause(Held {
                pid: session.pid,
                start_time: session.start_time,
                title: session.title.clone(),
                reason: strain.reason().into(),
                cpu_percent: session.cpu_percent,
                backoff_seconds: backoff,
                ..Default::default()
            });
        }
        if self.calm >= CALM_SAMPLES && now >= self.last_change + RESUME_GAP_SECONDS {
            // Oldest first, skipping any still backing off or whose load would saturate the
            // processor again.
            let fits = self.held.iter().position(|h| {
                now >= h.paused_at + h.backoff_seconds
                    && sample.cpu_percent + h.cpu_percent < RESUME_CPU_PERCENT
            });
            if let Some(index) = fits {
                self.calm = 0;
                self.last_change = now;
                let next = self.held.remove(index);
                self.resumed
                    .retain(|(id, _, _)| *id != (next.pid, next.start_time));
                self.resumed
                    .push(((next.pid, next.start_time), now, next.backoff_seconds));
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

/// Replace each session's resident memory with its tree's physical footprint where the platform
/// reports one: macOS compresses memory under pressure, which hides it from resident size.
pub fn measure_footprints(sample: &Snapshot, sessions: &mut [Session]) {
    if sessions.is_empty() || !cfg!(target_os = "macos") {
        return;
    }
    let resident: std::collections::HashMap<(u32, u64), u64> = sample
        .processes
        .iter()
        .map(|p| ((p.pid, p.start_time), p.memory_bytes))
        .collect();
    let mut children: std::collections::HashMap<u32, Vec<&crate::types::ProcessRecord>> =
        std::collections::HashMap::new();
    for p in &sample.processes {
        if let Some(parent) = p.parent_pid.filter(|parent| *parent != p.pid) {
            children.entry(parent).or_default().push(p);
        }
    }
    for session in sessions {
        let mut stack = vec![(session.pid, session.start_time)];
        let mut total = 0u64;
        let mut seen = 0usize;
        while let Some((pid, start_time)) = stack.pop() {
            seen += 1;
            if seen > 512 {
                break;
            }
            // A process that just exited or cannot be read counts at its resident size.
            let bytes = crate::platform::footprint(pid, start_time)
                .or_else(|| resident.get(&(pid, start_time)).copied())
                .unwrap_or(0);
            total = total.saturating_add(bytes);
            for child in children.get(&pid).into_iter().flatten() {
                if child.start_time >= start_time {
                    stack.push((child.pid, child.start_time));
                }
            }
        }
        session.memory_bytes = total;
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
    fn memory_strain_pauses_the_working_session_that_is_growing_fastest() {
        let mut g = Governor::default();
        let mut now = 1_000;
        let mut tight = sample(1, 50.0);
        tight.swap_growth_bytes = 128 * MIB;
        let mut sessions = vec![
            // Largest, but steady: pausing it frees nothing.
            session(10, Some("working"), 20.0, 6),
            session(11, Some("working"), 5.0, 1),
            session(12, Some("working"), 5.0, 1),
            // Growing fastest, but waiting for the person: never paused.
            session(13, Some("idle"), 0.0, 1),
        ];
        let mut steps = vec![];
        for _ in 0..3 {
            now += 5;
            steps.push(g.decide(&tight, &sessions, &[], true, false, now));
            sessions[1].memory_bytes += 512 * MIB;
            sessions[2].memory_bytes += 128 * MIB;
            sessions[3].memory_bytes += GIB;
        }
        let paused: Vec<_> = steps
            .into_iter()
            .filter_map(|s| match s {
                Step::Pause(held) => Some(held),
                _ => None,
            })
            .collect();
        assert_eq!(paused.len(), 1);
        assert_eq!(paused[0].pid, 11);
        assert_eq!(paused[0].reason, "memory was running out");
    }

    #[test]
    fn memory_strain_with_no_agent_growing_pauses_nothing() {
        let mut g = Governor::default();
        let mut now = 1_000;
        let mut tight = sample(1, 50.0);
        tight.swap_growth_bytes = 300 * MIB;
        let sessions = [
            session(10, Some("working"), 20.0, 6),
            session(11, Some("working"), 5.0, 1),
        ];
        assert_eq!(
            run(&mut g, &tight, &sessions, &[], &mut now, 10),
            Step::Idle
        );
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

    #[test]
    fn a_session_that_tips_it_back_after_a_resume_is_held_longer_each_time() {
        let mut g = Governor::default();
        let mut now = 1_000;
        let sessions = [
            session(10, Some("working"), 10.0, 8),
            session(11, Some("working"), 10.0, 1),
        ];
        let mut tight = sample(1, 50.0);
        tight.swap_growth_bytes = 128 * MIB;
        let roomy = sample(4, 20.0);
        let mut backoffs = vec![];
        for _ in 0..4 {
            let mut growing = sessions.clone();
            let mut step = Step::Idle;
            for _ in 0..10 {
                now += 5;
                growing[0].memory_bytes += 256 * MIB;
                step = g.decide(&tight, &growing, &[], true, false, now);
                if step != Step::Idle {
                    break;
                }
            }
            let Step::Pause(held) = step else {
                panic!("expected a pause");
            };
            assert_eq!(held.pid, 10);
            backoffs.push(held.backoff_seconds);
            let record = record(10, now);
            g.paused(&record, held);
            let paused_at = now;
            let mut held_sessions = sessions.clone();
            held_sessions[0].paused_until = Some(record.until);
            let Step::Resume(_) = run(&mut g, &roomy, &held_sessions, &[record], &mut now, 200)
            else {
                panic!("expected a resume");
            };
            assert!(now >= paused_at + backoffs.last().unwrap());
        }
        assert_eq!(backoffs, [0, 60, 120, 240]);
    }

    #[test]
    fn steady_tight_memory_pauses_nothing_more_and_still_lets_held_sessions_resume() {
        let mut g = Governor::default();
        let mut now = 1_000;
        let sessions = [
            session(10, Some("working"), 10.0, 4),
            session(11, Some("working"), 10.0, 2),
        ];
        // macOS still reports a warning, but nothing is growing: no pause helps.
        let mut steady = sample(3, 20.0);
        steady.platform.memory_pressure = Some("warning".into());
        assert_eq!(strain(&steady, Some(steady.memory_available_bytes)), None);
        assert_eq!(
            run(&mut g, &steady, &sessions, &[], &mut now, 10),
            Step::Idle
        );
        // Still falling: that is strain.
        assert_eq!(
            strain(&steady, Some(steady.memory_available_bytes + GIB)),
            Some(Strain::Memory)
        );
        // A held session resumes under a lingering warning once there is a margin.
        let record = record(10, now);
        g.paused(&record, held("held", 5.0));
        let mut roomy = sample(4, 20.0);
        roomy.platform.memory_pressure = Some("warning".into());
        let Step::Resume(back) = run(&mut g, &roomy, &sessions, &[record], &mut now, 10) else {
            panic!("expected a resume");
        };
        assert_eq!(back[0].title, "held");
    }
}
