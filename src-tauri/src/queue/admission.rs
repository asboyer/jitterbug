//! Bounded, history-based admission. These are estimates, never runtime resource caps.
//! Every launch is rechecked under the queue lock by both coordinator and worker.
use super::*;

pub const MAX_CONCURRENT: usize = 2;
const MIN_RUNS: usize = 2;
const HISTORY_SECONDS: u64 = 7 * 24 * 60 * 60;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Estimate {
    pub runs: usize,
    pub cpu_percent: Option<f32>,
    pub memory_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sample {
    pub captured_at: u64,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scheduling {
    pub running: usize,
    pub waiting: usize,
    pub learned_profiles: usize,
    pub cpu_ceiling: f32,
    pub memory_reserve_bytes: u64,
    pub state: String,
    pub title: String,
    pub detail: String,
}

impl Store {
    pub fn estimate(&self, profile: &Profile) -> Estimate {
        // Match the entire approval, including command, folder, memory floor and PATH.
        // Failed/stale or poorly sampled runs break the learning streak. Expiry and the
        // existing bounded job history prevent a permanent, stale resource model.
        let runs: Vec<_> = self
            .jobs
            .iter()
            .rev()
            .filter(|j| &j.profile == profile && !j.stage.active())
            .take(5)
            .take_while(|j| {
                j.stage == Stage::Passed
                    && j.observed_samples >= 3
                    && j.peak_cpu_percent.is_finite()
                    && (0.0..=100.0).contains(&j.peak_cpu_percent)
                    && j.peak_memory_bytes > 0
                    && j.ended_at.is_some_and(|at| {
                        at <= now() && at.saturating_add(HISTORY_SECONDS) >= now()
                    })
            })
            .collect();
        let cpu = runs.iter().map(|j| j.peak_cpu_percent).fold(0.0, f32::max);
        let memory = runs.iter().map(|j| j.peak_memory_bytes).max().unwrap_or(0);
        Estimate {
            runs: runs.len(),
            cpu_percent: (runs.len() >= MIN_RUNS).then(|| (cpu * 1.25 + 5.0).max(10.0)),
            memory_bytes: profile.memory_bytes.max(
                memory
                    .saturating_add(memory / 4)
                    .saturating_add(if memory > 0 { GIB / 4 } else { 0 }),
            ),
        }
    }

    pub fn cpu_ceiling(&self) -> f32 {
        if self.control.goal == control::Goal::Responsive {
            55.0
        } else {
            70.0
        }
    }

    pub fn global_block(&self) -> Option<String> {
        if self.paused {
            return Some("New checks are paused.".into());
        }
        if self.control.protection().is_some() {
            return Some("Making room for your apps. New checks are held.".into());
        }
        if self.jobs.iter().any(|j| j.stage == Stage::Interrupted) {
            return Some("Review the interrupted check before starting more work.".into());
        }
        self.resources
            .block_reason(0)
            .or_else(|| self.goal_block())
            .or_else(|| {
                (self.resources.consecutive_good < 2)
                    .then(|| "Waiting for two healthy resource samples.".into())
            })
    }

    pub fn admission_reason(&self, candidate: &Job) -> Option<String> {
        if let Some(reason) = self.global_block().or_else(|| self.job_hold(candidate)) {
            return Some(reason);
        }
        let active: Vec<_> = self
            .jobs
            .iter()
            .filter(|j| j.id != candidate.id && j.stage.occupies_slot())
            .collect();
        if active.len() >= MAX_CONCURRENT {
            return Some("Two checks are running. The next will follow.".into());
        }
        let estimate = self.estimate(&candidate.profile);
        let mut additional_memory = estimate.memory_bytes;
        if !active.is_empty() {
            let Some(mut additional_cpu) = estimate.cpu_percent else {
                return Some(
                    "This check runs alone while Jitterbug learns its resource use.".into(),
                );
            };
            let folder = candidate.profile.worktree.canonicalize().ok();
            for job in active {
                let other_folder = job.profile.worktree.canonicalize().ok();
                if folder
                    .as_ref()
                    .zip(other_folder.as_ref())
                    .is_none_or(|(a, b)| a.starts_with(b) || b.starts_with(a))
                {
                    return Some("Checks in the same working folder take turns.".into());
                }
                let running = self.estimate(&job.profile);
                let Some(cpu) = running.cpu_percent else {
                    return Some("Learning the running check before sharing capacity.".into());
                };
                let Some(sample) = job.last_sample.as_ref().filter(|sample| {
                    job.stage == Stage::Running
                        && sample.captured_at == self.resources.captured_at
                        && job
                            .started_at
                            .is_some_and(|at| sample.captured_at >= at.saturating_add(5))
                        && job.observed_samples >= 2
                        && sample.cpu_percent.is_finite()
                        && sample.cpu_percent >= 0.0
                }) else {
                    return Some("Measuring the running check before adding another.".into());
                };
                // Current readings already include running jobs. Reserve only their
                // unconsumed estimate, plus the full cost of the candidate.
                additional_cpu += (cpu - sample.cpu_percent).max(0.0);
                additional_memory = additional_memory
                    .saturating_add(running.memory_bytes.saturating_sub(sample.memory_bytes));
            }
            if self.resources.cpu_percent + additional_cpu >= self.cpu_ceiling() {
                return Some("Leaving CPU room for your apps. This check will follow.".into());
            }
        }
        self.resources.block_reason(additional_memory)
    }

    pub fn scheduling(&self) -> Scheduling {
        let active: Vec<_> = self
            .jobs
            .iter()
            .filter(|j| j.stage.occupies_slot())
            .collect();
        let waiting = self
            .jobs
            .iter()
            .filter(|j| j.stage == Stage::Queued)
            .count();
        let next = self.next_index().map(|i| &self.jobs[i]);
        let global = self.global_block();
        let block = global
            .clone()
            .or_else(|| next.and_then(|j| self.admission_reason(j)));
        let learning = active
            .iter()
            .any(|j| self.estimate(&j.profile).cpu_percent.is_none());
        let (state, title) = if self.jobs.iter().any(|j| j.stage == Stage::Interrupted) {
            ("review", "A check needs your review")
        } else if self.paused || self.control.protection().is_some() {
            ("paused", "Keeping room for your apps")
        } else if global.is_some() {
            ("waiting", "Checking available capacity")
        } else if active.len() > 1 {
            ("parallel", "Putting spare capacity to work")
        } else if waiting > 0 && block.is_some() && !learning {
            ("waiting", "Waiting for room")
        } else if waiting > 0 && next.is_none() {
            ("paused", "Checks are on hold")
        } else if learning {
            ("learning", "Learning this check")
        } else if !active.is_empty() {
            ("running", "Work is moving")
        } else {
            ("ready", "Ready for your next task")
        };
        Scheduling {
            running: self.jobs.iter().filter(|j| matches!(j.stage, Stage::Starting | Stage::Running)).count(), waiting,
            learned_profiles: self.profiles.iter()
                .filter(|p| self.estimate(p).cpu_percent.is_some()).count(),
            cpu_ceiling: self.cpu_ceiling(),
            memory_reserve_bytes: self.resources.memory_reserve(),
            state: state.into(), title: title.into(),
            detail: block.unwrap_or_else(|| if waiting > 0 && next.is_none() {
                "Waiting checks are held by you or their agent session.".into()
            } else if learning {
                "New checks run alone. Two sampled, successful runs unlock capacity sharing.".into()
            } else {
                "Up to two familiar checks can share capacity. Current apps and local models count toward the same budget.".into()
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str) -> Profile {
        Profile {
            name: name.into(),
            worktree: std::env::temp_dir().join(name),
            command: vec!["synthetic-check".into()],
            memory_bytes: GIB,
            search_path: None,
        }
    }

    fn job(id: &str, profile: Profile, stage: Stage) -> Job {
        Job {
            id: id.into(),
            request_id: id.into(),
            held: false,
            task: "Synthetic check".into(),
            harness: "test".into(),
            session: id.into(),
            profile,
            inputs: Default::default(),
            stage,
            submitted_at: now() - 30,
            started_at: Some(now() - 20),
            ended_at: Some(now() - 1),
            worker: None,
            child: None,
            exit_code: Some(0),
            message: String::new(),
            peak_cpu_percent: 8.0,
            peak_memory_bytes: GIB / 2,
            observed_samples: 4,
            last_sample: Some(Sample {
                captured_at: now(),
                cpu_percent: 8.0,
                memory_bytes: GIB / 2,
            }),
        }
    }

    fn store() -> Store {
        let mut s = Store {
            resources: Resources {
                captured_at: now(),
                ready: true,
                cpu_percent: 15.0,
                available_bytes: 12 * GIB,
                total_bytes: 16 * GIB,
                consecutive_good: 2,
                ..Default::default()
            },
            ..Default::default()
        };
        // Existing distinct folders keep the scheduling tests independent of personal files.
        let mut a = profile("a");
        a.worktree = std::env::temp_dir().canonicalize().unwrap();
        let mut b = profile("b");
        b.worktree = std::env::current_dir().unwrap().canonicalize().unwrap();
        s.profiles = vec![a.clone(), b.clone()];
        for p in [a.clone(), b.clone()] {
            for i in 0..2 {
                s.jobs
                    .push(job(&format!("{}-{i}", p.name), p.clone(), Stage::Passed));
            }
        }
        s.jobs.push(job("active", a, Stage::Running));
        s.jobs.push(job("next", b, Stage::Queued));
        s
    }

    #[test]
    fn interrupted_and_held_work_is_not_reported_as_running_or_ready() {
        let mut s = store();
        s.jobs[4].stage = Stage::Interrupted;
        assert_eq!(s.scheduling().state, "review");
        assert_eq!(s.scheduling().running, 0);
        s.jobs[4].stage = Stage::Failed;
        s.jobs[5].held = true;
        assert_eq!(s.scheduling().state, "paused");
        assert_eq!(s.scheduling().waiting, 1);
        s.resources.ready = false;
        assert_eq!(s.scheduling().state, "waiting");
    }

    #[test]
    fn unknown_checks_run_alone_and_old_state_has_no_sample() {
        let mut s = store();
        s.jobs.retain(|j| j.stage != Stage::Passed);
        assert!(s
            .admission_reason(&s.jobs[1])
            .unwrap()
            .contains("runs alone"));
        s.jobs.remove(0);
        assert!(s.admission_reason(&s.jobs[0]).is_none());
        let mut json = serde_json::to_value(&s.jobs[0]).unwrap();
        json.as_object_mut().unwrap().remove("last_sample");
        let old: Job = serde_json::from_value(json).unwrap();
        assert!(old.last_sample.is_none());
    }

    #[test]
    fn two_sampled_runs_learn_peaks_with_padding_and_declared_memory_floor() {
        let mut s = store();
        let p = s.profiles[0].clone();
        let estimate = s.estimate(&p);
        assert_eq!(estimate.cpu_percent, Some(15.0));
        assert_eq!(estimate.memory_bytes, GIB);
        s.jobs[0].peak_memory_bytes = 2 * GIB;
        s.jobs[0].peak_cpu_percent = 20.0;
        let estimate = s.estimate(&p);
        assert_eq!(estimate.cpu_percent, Some(30.0));
        assert_eq!(estimate.memory_bytes, 2 * GIB + GIB / 2 + GIB / 4);
        s.jobs[1].stage = Stage::Failed;
        assert_eq!(s.estimate(&p).cpu_percent, None);
    }

    #[test]
    fn changed_approval_expired_or_incomplete_history_requires_relearning() {
        let s = store();
        let mut p = s.profiles[0].clone();
        p.command.push("--changed".into());
        assert_eq!(s.estimate(&p).runs, 0);
        for mutate in [
            |j: &mut Job| j.ended_at = Some(now() - HISTORY_SECONDS - 1),
            |j: &mut Job| j.observed_samples = 2,
            |j: &mut Job| j.peak_cpu_percent = f32::NAN,
            |j: &mut Job| j.stage = Stage::Stale,
        ] {
            let mut s = store();
            mutate(&mut s.jobs[1]);
            assert!(s.estimate(&s.profiles[0]).cpu_percent.is_none());
        }
    }

    #[test]
    fn familiar_checks_share_spare_capacity_and_reserve_only_unconsumed_memory() {
        let mut s = store();
        assert!(s.admission_reason(s.jobs.last().unwrap()).is_none());
        // 4 GiB free covers a 3.2 GiB system reserve plus 0.5 GiB still owed
        // to the active check, but not the next 1 GiB check.
        s.resources.available_bytes = 4 * GIB;
        assert!(s
            .admission_reason(s.jobs.last().unwrap())
            .unwrap()
            .contains("memory"));
        s.resources.available_bytes = 5 * GIB;
        assert!(s.admission_reason(s.jobs.last().unwrap()).is_none());
    }

    #[test]
    fn competing_apps_and_local_models_count_against_cpu_budget() {
        let mut s = store();
        s.resources.cpu_percent = 40.0;
        assert!(s
            .admission_reason(s.jobs.last().unwrap())
            .unwrap()
            .contains("CPU"));
        s.control.goal = control::Goal::Throughput;
        assert!(s.admission_reason(s.jobs.last().unwrap()).is_none());
        s.resources.cpu_percent = 60.0;
        assert!(s
            .admission_reason(s.jobs.last().unwrap())
            .unwrap()
            .contains("CPU"));
    }

    #[test]
    fn launch_warmup_missing_attribution_and_stale_samples_prevent_overlap() {
        for mutate in [
            |j: &mut Job| j.stage = Stage::Starting,
            |j: &mut Job| j.started_at = Some(now()),
            |j: &mut Job| j.last_sample = None,
            |j: &mut Job| j.last_sample.as_mut().unwrap().captured_at -= 5,
            |j: &mut Job| j.last_sample.as_mut().unwrap().cpu_percent = f32::NAN,
            |j: &mut Job| j.observed_samples = 1,
        ] {
            let mut s = store();
            mutate(&mut s.jobs[4]);
            assert!(s
                .admission_reason(&s.jobs[5])
                .unwrap()
                .contains("Measuring"));
        }
    }

    #[test]
    fn same_folder_never_overlaps_and_worker_excludes_only_its_own_reservation() {
        let mut s = store();
        let mut candidate = s.jobs[5].clone();
        candidate.profile = s.jobs[4].profile.clone();
        assert!(s
            .admission_reason(&candidate)
            .unwrap()
            .contains("same working folder"));
        s.jobs[5].stage = Stage::Starting;
        assert!(s.admission_reason(&s.jobs[5]).is_none());
        let mut third = s.jobs[5].clone();
        third.id = "third".into();
        assert!(s.admission_reason(&third).unwrap().contains("Two checks"));
    }

    #[test]
    fn worker_rechecks_policy_and_fresh_whole_machine_guards() {
        for mutate in [
            |s: &mut Store| s.paused = true,
            |s: &mut Store| s.control.protected_until = Some(now() + 60),
            |s: &mut Store| s.jobs[4].stage = Stage::Interrupted,
            |s: &mut Store| s.jobs[5].held = true,
            |s: &mut Store| s.resources.thermal = Some("serious".into()),
            |s: &mut Store| s.resources.memory_pressure = Some("warning".into()),
            |s: &mut Store| s.resources.swap_growth_bytes = GIB,
            |s: &mut Store| s.resources.captured_at -= 16,
            |s: &mut Store| s.resources.consecutive_good = 1,
        ] {
            let mut s = store();
            s.jobs[5].stage = Stage::Starting;
            mutate(&mut s);
            assert!(s.admission_reason(&s.jobs[5]).is_some());
        }
    }
}
