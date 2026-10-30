//! Cooperative workflow controls. No signals, arbitrary commands, or inferred ownership.
use super::*;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Goal {
    #[default]
    Responsive,
    Throughput,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub profile: String,
    pub harness: String,
    pub session: String,
    pub task: String,
    pub phase: String,
    pub pause_requested: bool,
    pub waiting: bool,
    pub last_seen_at: u64,
}

impl Session {
    pub fn paused(&self) -> bool {
        self.pause_requested && self.waiting && self.last_seen_at.saturating_add(3) >= now()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub at: u64,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Controls {
    pub goal: Goal,
    pub protected_until: Option<u64>,
    pub next_job: Option<String>,
    pub sessions: Vec<Session>,
    pub events: Vec<Event>,
    pub last_block: Option<String>,
    pub busy_samples: u32,
    pub last_busy_sample_at: u64,
    pub notice_at: u64,
}

impl Controls {
    pub fn observe_pressure(&mut self, resources: &Resources, eligible: bool) {
        let at = resources.captured_at;
        if at == self.last_busy_sample_at {
            return;
        }
        let continuous =
            at > self.last_busy_sample_at && at <= self.last_busy_sample_at.saturating_add(15);
        let busy = eligible
            && resources.ready
            && at <= now().saturating_add(2)
            && at.saturating_add(15) >= now()
            && (resources.cpu_percent >= 85.0
                || matches!(
                    resources.memory_pressure.as_deref(),
                    Some("warning" | "critical")
                ));
        self.busy_samples = if !busy {
            0
        } else if continuous {
            self.busy_samples.saturating_add(1)
        } else {
            1
        };
        self.last_busy_sample_at = at;
        let required = if resources.cpu_percent >= 85.0
            || resources.memory_pressure.as_deref() == Some("critical")
        {
            3
        } else {
            6
        };
        if self.busy_samples >= required && self.notice_at.saturating_add(1800) < now() {
            self.notice_at = now();
        }
    }

    pub fn record(&mut self, message: impl Into<String>) {
        self.events.push(Event {
            at: now(),
            message: message.into(),
        });
        if self.events.len() > 24 {
            self.events.remove(0);
        }
    }

    pub fn protection(&self) -> Option<u64> {
        self.protected_until.filter(|until| *until > now())
    }

    pub fn session_for(&self, job: &Job) -> Option<&Session> {
        self.sessions.iter().find(|s| {
            s.profile == job.profile.name && s.harness == job.harness && s.session == job.session
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub action: String,
    pub job_id: Option<String>,
    pub reason: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
}

impl Plan {
    pub fn parse(text: &str, state: &Store) -> Result<Self, String> {
        let plan: Self = serde_json::from_str(text.trim()).map_err(|_| {
            "The model did not return a valid scheduling plan. No action was taken."
        })?;
        if plan.reason.trim().is_empty()
            || plan.reason.chars().count() > 180
            || plan.reason.chars().any(char::is_control)
        {
            return Err("Plan explanation exceeded its limit.".into());
        }
        match plan.action.as_str() {
            "protect" | "none" if plan.job_id.is_none() => (),
            "run_next"
                if state.jobs.iter().any(|j| {
                    Some(&j.id) == plan.job_id.as_ref() && j.stage == Stage::Queued && !j.held
                }) => {}
            _ => return Err("Plan proposed an unavailable action. No action was taken.".into()),
        }
        Ok(plan)
    }
}

/// Only user-facing commands can change permissions or priority. MCP clients cannot resume
/// themselves, bypass protection, or expand their profile allowlist.
pub fn change(directory: &Path, action: &str, id: Option<&str>) -> Result<(), String> {
    transaction(directory, |s| {
        match action {
            "protect" => {
                s.control.protected_until = Some(now() + 20 * 60);
                s.control.record("Upcoming checks held for 20 minutes.");
            }
            "unprotect" => {
                s.control.protected_until = None;
                s.control
                    .record("Protection ended. Checks resume when there is room.");
            }
            "responsive" | "throughput" => {
                s.control.goal = if action == "responsive" {
                    Goal::Responsive
                } else {
                    Goal::Throughput
                };
                s.control.record(if action == "responsive" {
                    "Keeping more CPU headroom for your apps."
                } else {
                    "Checks may start sooner, within resource limits."
                });
            }
            "hold_job" | "resume_job" | "run_next" => {
                let id = id.ok_or("Missing job ID.")?;
                let job = s
                    .jobs
                    .iter_mut()
                    .find(|j| j.id == id)
                    .ok_or("Job is no longer available.")?;
                if job.stage != Stage::Queued
                    && !(job.stage == Stage::Starting && action == "hold_job")
                {
                    return Err("This check has already started or finished. Running checks finish normally.".into());
                }
                if action == "run_next" {
                    s.control.next_job = Some(id.into());
                } else {
                    job.held = action == "hold_job";
                }
                s.control.record(match action {
                    "run_next" => "Selected check gets the next eligible slot.",
                    "hold_job" => "Check held before launch.",
                    _ => "Check released; resource limits still apply.",
                });
            }
            "pause_session" | "resume_session" => {
                let session = s
                    .control
                    .sessions
                    .iter_mut()
                    .find(|v| Some(v.id.as_str()) == id)
                    .ok_or("Checkpoint session is no longer available.")?;
                if session.phase == "complete" {
                    return Err("This session has finished.".into());
                }
                session.pause_requested = action == "pause_session";
                session.waiting = false;
                s.control.record(if action == "pause_session" {
                    "Agent pause requested at its next checkpoint."
                } else {
                    "Agent released from its checkpoint."
                });
            }
            _ => return Err("Unsupported workflow control.".into()),
        }
        Ok(())
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub session_id: String,
    pub may_proceed: bool,
    pub state: String,
    pub instruction: String,
}

/// A bounded, interruptible long poll. An acknowledged pause exists only while a checkpoint
/// caller is actually waiting. Timed-out clients must check again, never treat timeout as consent.
pub fn checkpoint(
    directory: &Path,
    profile: &str,
    harness: &str,
    session: &str,
    task: &str,
    phase: &str,
    wait_seconds: u64,
) -> Result<Checkpoint, String> {
    identifier(profile)?;
    label(harness, 40)?;
    label(session, 96)?;
    label(task, 160)?;
    if !["planning", "editing", "tests", "review", "complete"].contains(&phase) || wait_seconds > 20
    {
        return Err("Invalid phase or checkpoint wait (0–20 seconds).".into());
    }
    let id = format!(
        "session-{:x}",
        Sha256::digest(
            serde_json::to_vec(&(profile, harness, session))
                .map_err(|_| "Cannot identify session.")?
        )
    );
    transaction(directory, |s| {
        if !s.profiles.iter().any(|p| p.name == profile) {
            return Err("Profile was revoked.".into());
        }
        if !s.control.sessions.iter().any(|v| v.id == id) {
            s.control
                .sessions
                .retain(|v| v.last_seen_at + 86400 > now() || v.pause_requested);
            if s.control.sessions.len() >= 32 {
                if let Some(index) = s
                    .control
                    .sessions
                    .iter()
                    .position(|v| v.phase == "complete")
                {
                    s.control.sessions.remove(index);
                }
            }
            if s.control.sessions.len() >= 32 {
                return Err("Checkpoint registry is full; finish existing sessions first.".into());
            }
            s.control.sessions.push(Session {
                id: id.clone(),
                profile: profile.into(),
                harness: harness.into(),
                session: session.into(),
                task: task.into(),
                phase: phase.into(),
                pause_requested: false,
                waiting: false,
                last_seen_at: now(),
            });
        }
        let record = s.control.sessions.iter_mut().find(|v| v.id == id).unwrap();
        record.task = task.into();
        record.phase = phase.into();
        record.last_seen_at = now();
        if phase == "complete" {
            record.pause_requested = false;
            record.waiting = false;
        }
        Ok(())
    })?;
    let deadline = std::time::Instant::now() + Duration::from_secs(wait_seconds);
    loop {
        let paused = transaction(directory, |s| {
            if !s.profiles.iter().any(|p| p.name == profile) {
                return Err("Profile was revoked; stop using this connection.".into());
            }
            let record = s
                .control
                .sessions
                .iter_mut()
                .find(|v| v.id == id)
                .ok_or("Session expired.")?;
            let first = record.pause_requested && !record.waiting;
            record.last_seen_at = now();
            record.waiting = record.pause_requested && std::time::Instant::now() < deadline;
            let paused = record.pause_requested;
            if first && record.waiting {
                s.control.record("Agent reached its pause checkpoint.");
            }
            Ok(paused)
        })?;
        if !paused || std::time::Instant::now() >= deadline {
            transaction(directory, |s| {
                if let Some(record) = s.control.sessions.iter_mut().find(|v| v.id == id) {
                    record.waiting = false;
                }
                Ok(())
            })?;
            return Ok(Checkpoint { session_id: id, may_proceed: !paused, state: if paused { "pause_requested" } else { "ready" }.into(), instruction: if paused { "Remain paused. Call workflow_checkpoint again with the same session before doing any more work. A timeout is not permission to continue." } else { "Proceed to the reported step. Check in again before the next step; submit approved heavy checks through validation_submit." }.into() });
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

impl Store {
    pub fn job_hold(&self, job: &Job) -> Option<String> {
        if job.held {
            return Some("Held by you.".into());
        }
        if self
            .control
            .session_for(job)
            .is_some_and(|s| s.pause_requested)
        {
            return Some("Waiting for its agent session to resume.".into());
        }
        if self.control.protection().is_some() {
            return Some("Protected session: upcoming checks are held.".into());
        }
        None
    }

    pub fn next_index(&self) -> Option<usize> {
        let eligible = |j: &&Job| j.stage == Stage::Queued && self.job_hold(j).is_none();
        self.jobs
            .iter()
            .enumerate()
            .find(|(_, j)| eligible(j) && Some(&j.id) == self.control.next_job.as_ref())
            .or_else(|| self.jobs.iter().enumerate().find(|(_, j)| eligible(j)))
            .map(|(i, _)| i)
    }

    pub fn goal_block(&self) -> Option<String> {
        if self.control.goal == Goal::Responsive && self.resources.cpu_percent >= 55.0 {
            Some("Keeping CPU headroom: waiting below 55%.".into())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_require_sustained_fresh_pressure_and_pending_work() {
        let mut c = Controls::default();
        let mut r = Resources {
            ready: true,
            cpu_percent: 90.0,
            ..Default::default()
        };
        for at in [now() - 10, now() - 5, now()] {
            r.captured_at = at;
            c.observe_pressure(&r, true);
        }
        assert!(c.notice_at > 0);
        let notice = c.notice_at;
        c.observe_pressure(&r, true);
        assert_eq!(c.busy_samples, 3); // Same observation cannot count twice.
        r.captured_at += 1;
        c.observe_pressure(&r, true);
        assert_eq!(c.notice_at, notice); // Cooldown applies.
        for (ready, cpu, eligible, at) in [
            (true, 2.0, true, now()),
            (false, 99.0, true, now()),
            (true, 99.0, false, now()),
            (true, 99.0, true, now() - 30),
            (true, 99.0, true, now() + 30),
        ] {
            let mut c = Controls::default();
            r.ready = ready;
            r.cpu_percent = cpu;
            for offset in [2, 1, 0] {
                r.captured_at = at - offset;
                c.observe_pressure(&r, eligible);
            }
            assert_eq!(c.notice_at, 0);
        }
        let mut c = Controls::default();
        r.ready = true;
        r.cpu_percent = 99.0;
        for at in [now() - 11, now() - 6, now() + 10, now()] {
            r.captured_at = at;
            c.observe_pressure(&r, true);
        }
        assert_eq!(c.notice_at, 0); // Gaps, future samples, clock reversals reset history.
    }
}
