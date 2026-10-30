//! One-second pulse sampler for short hitches the five-second collector cannot see.
//! It keeps the last 120 ticks in memory only and never refreshes the full process table.
use crate::{
    automation::Origin,
    jitter::{self, Capture, Victim},
    platform,
    types::Snapshot,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use sysinfo::System;
#[cfg(not(target_os = "macos"))]
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate};

pub const CAPACITY: usize = 120;
pub const WATCHED: usize = 16;
const INTERVAL: Duration = Duration::from_secs(1);
/// Short sleep whose late wake-up approximates scheduler delay for ordinary threads.
const PROBE: Duration = Duration::from_millis(10);
/// A tick that arrives this late is itself evidence of a stall, not timer leeway.
const STALLED_TICK: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Always sample; snapshots look back 60 seconds and short spikes can open a case.
    #[default]
    Auto,
    /// No background sampling; a press records the next 15 seconds only.
    SnapshotOnly,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tick {
    pub at_ms: u64,
    /// Whole-machine CPU, 0–100.
    pub cpu_percent: f32,
    /// Each logical core, 0–100.
    pub cores: Vec<u8>,
    /// How late a 10 ms sleep woke up (or the tick itself, when grossly late).
    pub lag_ms: f32,
    /// System-wide paging deltas since the previous tick; unavailable outside macOS.
    pub swap_in_bytes: Option<u64>,
    pub swap_out_bytes: Option<u64>,
    pub page_in_bytes: Option<u64>,
    pub compressed_bytes: Option<u64>,
    pub decompressed_bytes: Option<u64>,
    /// 0 normal, 1 warning, 2 critical.
    pub memory_pressure: Option<u8>,
    /// 0 nominal, 1 fair, 2 serious, 3 critical.
    pub thermal: Option<u8>,
    /// Watched PIDs and their CPU over the tick, where 100 means one logical core.
    pub processes: Vec<(u32, f32)>,
}

impl Tick {
    pub fn core_max(&self) -> f32 {
        self.cores.iter().copied().max().unwrap_or_default() as f32
    }
    /// Swap traffic plus pages pulled back out of the compressor, in bytes.
    pub fn paging_bytes(&self) -> Option<u64> {
        match (
            self.swap_in_bytes,
            self.swap_out_bytes,
            self.decompressed_bytes,
        ) {
            (Some(a), Some(b), Some(c)) => Some(a.saturating_add(b).saturating_add(c)),
            _ => None,
        }
    }
}

/// Names for watched PIDs. Kept in memory and dropped once no buffered tick refers to them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Identity {
    pub pid: u32,
    pub start_time: u64,
    pub name: String,
    pub workload_id: String,
    pub label: String,
    pub category: String,
}

/// Bounded tick history. A gap or clock jump starts a new run, so deltas never span a sleep.
#[derive(Debug, Default)]
pub struct Ring(VecDeque<Tick>);

impl Ring {
    pub fn push(&mut self, tick: Tick) {
        if self
            .0
            .back()
            .is_some_and(|last| tick.at_ms <= last.at_ms || tick.at_ms - last.at_ms > 5000)
        {
            self.0.clear();
        }
        self.0.push_back(tick);
        while self.0.len() > CAPACITY {
            self.0.pop_front();
        }
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn window(&self, from_ms: u64, to_ms: u64) -> Vec<Tick> {
        self.0
            .iter()
            .filter(|t| (from_ms..=to_ms).contains(&t.at_ms))
            .cloned()
            .collect()
    }
    pub fn latest(&self, count: usize) -> Vec<Tick> {
        self.0
            .iter()
            .skip(self.0.len().saturating_sub(count))
            .cloned()
            .collect()
    }
    fn references(&self, pid: u32) -> bool {
        self.0
            .iter()
            .any(|t| t.processes.iter().any(|(p, _)| *p == pid))
    }
}

/// Cost of the sampler itself, for the budget probe. Contains no process data.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Stats {
    pub ticks: u64,
    pub work_ns_total: u64,
    pub work_ns_max: u64,
}

type SpikeHandler = Arc<dyn Fn(Capture) + Send + Sync>;

#[derive(Default)]
struct State {
    mode: Mode,
    ring: Ring,
    watched: Vec<(u32, u64)>,
    identities: HashMap<u32, Identity>,
    logical_cpus: usize,
    running: bool,
    generation: u64,
    record_until: Option<Instant>,
    capturing: bool,
    /// Whether the active press records forward only (no look-back was being sampled).
    forward_only: bool,
    pending_spike: Option<(u64, Instant)>,
    stats: Stats,
    spikes: Option<SpikeHandler>,
}

impl State {
    fn wanted(&self) -> bool {
        self.mode == Mode::Auto || self.record_until.is_some_and(|t| t > Instant::now())
    }
}

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    wake: Condvar,
}

#[derive(Default)]
pub struct Pulse {
    inner: Arc<Inner>,
}

impl Pulse {
    /// Creates a stopped sampler. `set_mode(Mode::Auto)` starts background sampling.
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_mode(&self, mode: Mode) {
        let mut state = self.lock();
        state.mode = mode;
        if state.wanted() {
            self.spawn(&mut state);
        }
        drop(state);
        self.inner.wake.notify_all();
    }
    pub fn mode(&self) -> Mode {
        self.lock().mode
    }
    pub fn is_running(&self) -> bool {
        self.lock().running
    }
    pub fn stats(&self) -> Stats {
        self.lock().stats
    }
    pub fn tick_count(&self) -> usize {
        self.lock().ring.len()
    }
    pub fn latest(&self, count: usize) -> Vec<Tick> {
        self.lock().ring.latest(count)
    }
    /// Called with a frozen capture when Auto mode sees a sustained short hitch.
    pub fn set_spike_handler(&self, handler: SpikeHandler) {
        self.lock().spikes = Some(handler);
    }
    /// Refresh the watched PIDs from the latest full five-second sample.
    pub fn watch(&self, snapshot: &Snapshot) {
        let mut top: Vec<_> = snapshot
            .processes
            .iter()
            .filter(|p| p.pid != std::process::id())
            .collect();
        top.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
        top.truncate(WATCHED);
        let mut state = self.lock();
        state.logical_cpus = snapshot.logical_cpus;
        state.watched = top.iter().map(|p| (p.pid, p.start_time)).collect();
        for p in top {
            let workload = snapshot.workloads.iter().find(|w| w.id == p.workload_id);
            state.identities.insert(
                p.pid,
                Identity {
                    pid: p.pid,
                    start_time: p.start_time,
                    name: p.name.clone(),
                    workload_id: p.workload_id.clone(),
                    label: workload.map_or_else(|| p.name.clone(), |w| w.label.clone()),
                    category: workload.map(|w| w.category.clone()).unwrap_or_default(),
                },
            );
        }
        let State {
            identities,
            watched,
            ring,
            ..
        } = &mut *state;
        identities.retain(|pid, _| watched.iter().any(|(p, _)| p == pid) || ring.references(*pid));
    }
    /// Begin a snapshot. Returns how long to wait before `freeze`, or an error if one is active.
    pub fn press(&self) -> Result<Duration, String> {
        let mut state = self.lock();
        if state.capturing {
            return Err("A Jitter Snapshot is already being recorded.".into());
        }
        state.capturing = true;
        state.forward_only = !(state.mode == Mode::Auto && state.running);
        if !state.forward_only {
            return Ok(Duration::from_millis(jitter::POST_ROLL_MS));
        }
        // Snapshot-only: sample now for the forward window, then stop again.
        let forward = Duration::from_millis(jitter::FORWARD_MS);
        state.record_until = Some(Instant::now() + forward + INTERVAL);
        self.spawn(&mut state);
        Ok(forward)
    }
    /// Cancel a press that could not be completed.
    pub fn release(&self) {
        self.lock().capturing = false;
    }
    /// Freeze the window around a press into a capture and allow the next snapshot.
    pub fn freeze(&self, press_at_ms: u64, victim: Option<Victim>) -> Capture {
        let mut state = self.lock();
        state.capturing = false;
        capture(
            &state,
            Origin::Snapshot,
            press_at_ms,
            victim,
            state.forward_only,
        )
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    fn spawn(&self, state: &mut State) {
        if state.running {
            return;
        }
        state.running = true;
        state.generation += 1;
        let generation = state.generation;
        let inner = self.inner.clone();
        let started = std::thread::Builder::new()
            .name("jitterbug-pulse".into())
            .spawn(move || run(inner, generation));
        if started.is_err() {
            state.running = false;
        }
    }
}

fn capture(
    state: &State,
    origin: Origin,
    press_at_ms: u64,
    victim: Option<Victim>,
    forward_only: bool,
) -> Capture {
    let (from, to) = if forward_only {
        (press_at_ms, press_at_ms + jitter::FORWARD_MS)
    } else {
        (
            press_at_ms.saturating_sub(jitter::LOOK_BACK_MS),
            press_at_ms + jitter::POST_ROLL_MS,
        )
    };
    let ticks = state.ring.window(from, to);
    let mut identities: Vec<_> = state
        .identities
        .values()
        .filter(|i| {
            ticks
                .iter()
                .any(|t| t.processes.iter().any(|(p, _)| *p == i.pid))
        })
        .cloned()
        .collect();
    identities.sort_by_key(|i| i.pid);
    Capture {
        origin,
        press_at_ms,
        forward_only,
        ticks,
        identities,
        victim,
        logical_cpus: state.logical_cpus.max(1),
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn run(inner: Arc<Inner>, generation: u64) {
    // Hold an activity assertion only while sampling: App Nap would otherwise coalesce timers
    // and the probe would report that coalescing as scheduler lag.
    let _activity = platform::Activity::begin();
    let mut sampler = Sampler::new();
    let mut next = Instant::now() + INTERVAL;
    loop {
        let watched = {
            let mut state = inner.state.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                if state.generation != generation || !state.wanted() {
                    if state.generation == generation {
                        state.running = false;
                        state.record_until = None;
                    }
                    return;
                }
                let now = Instant::now();
                if now >= next {
                    break;
                }
                state = inner
                    .wake
                    .wait_timeout(state, next - now)
                    .unwrap_or_else(|p| p.into_inner())
                    .0;
            }
            state.watched.clone()
        };
        let late = next.elapsed();
        let started = Instant::now();
        let mut tick = sampler.tick(&watched);
        let work = started.elapsed();
        tick.lag_ms = lateness(PROBE, late);
        next += INTERVAL;
        if next <= Instant::now() {
            next = Instant::now() + INTERVAL;
        }
        let due = {
            let mut state = inner.state.lock().unwrap_or_else(|p| p.into_inner());
            state.ring.push(tick);
            state.stats.ticks += 1;
            let ns = work.as_nanos() as u64;
            state.stats.work_ns_total = state.stats.work_ns_total.saturating_add(ns);
            state.stats.work_ns_max = state.stats.work_ns_max.max(ns);
            spike_due(&mut state)
        };
        if let Some((handler, capture)) = due {
            handler(capture);
        }
    }
}

/// Sleep briefly and report how late the wake-up was, in milliseconds.
fn lateness(probe: Duration, tick_late: Duration) -> f32 {
    let start = Instant::now();
    std::thread::sleep(probe);
    late_ms(start.elapsed(), probe, tick_late)
}

pub fn late_ms(elapsed: Duration, requested: Duration, tick_late: Duration) -> f32 {
    let probe = elapsed.saturating_sub(requested);
    let lag = if tick_late >= STALLED_TICK {
        probe.max(tick_late)
    } else {
        probe
    };
    lag.as_secs_f32() * 1000.0
}

fn spike_due(state: &mut State) -> Option<(SpikeHandler, Capture)> {
    if state.mode != Mode::Auto || state.spikes.is_none() {
        state.pending_spike = None;
        return None;
    }
    if let Some((start, due)) = state.pending_spike {
        if Instant::now() < due {
            return None;
        }
        state.pending_spike = None;
        let handler = state.spikes.clone()?;
        return Some((handler, capture(state, Origin::Spike, start, None, false)));
    }
    // A user's snapshot already covers this window.
    if state.capturing {
        return None;
    }
    let start = jitter::spike(&state.ring.latest(4))?;
    state.pending_spike = Some((
        start,
        Instant::now() + Duration::from_millis(jitter::POST_ROLL_MS),
    ));
    None
}

/// Sampler-thread state. Owns a small sysinfo instance limited to CPUs and watched PIDs.
struct Sampler {
    system: System,
    #[cfg(not(target_os = "macos"))]
    processes: System,
    vm: Option<platform::VmCounters>,
    cpu_times: HashMap<(u32, u64), (u64, Instant)>,
}

impl Sampler {
    fn new() -> Self {
        let mut system = System::new();
        system.refresh_cpu_usage();
        Self {
            system,
            #[cfg(not(target_os = "macos"))]
            processes: System::new(),
            vm: platform::vm_counters(),
            cpu_times: HashMap::new(),
        }
    }
    /// Cumulative CPU time in nanoseconds per watched PID whose start time still matches.
    #[cfg(target_os = "macos")]
    fn cpu_times(&mut self, watched: &[(u32, u64)]) -> Vec<(u32, u64, u64)> {
        // Direct proc_pidinfo reads: sysinfo would list every PID on each tick.
        watched
            .iter()
            .filter_map(|&(pid, start)| Some((pid, start, platform::process_cpu_ns(pid, start)?)))
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    fn cpu_times(&mut self, watched: &[(u32, u64)]) -> Vec<(u32, u64, u64)> {
        // sysinfo cannot forget live PIDs, so rebuild the small table once it accumulates
        // processes that are no longer watched. Rates use absolute CPU time, so none are lost.
        if self.processes.processes().len() > WATCHED * 4 {
            self.processes = System::new();
        }
        let pids: Vec<_> = watched.iter().map(|(p, _)| Pid::from_u32(*p)).collect();
        if !pids.is_empty() {
            self.processes.refresh_processes_specifics(
                ProcessesToUpdate::Some(&pids),
                true,
                ProcessRefreshKind::nothing().with_cpu(),
            );
        }
        watched
            .iter()
            .filter_map(|&(pid, start)| {
                let p = self.processes.process(Pid::from_u32(pid))?;
                (p.start_time() == start)
                    .then(|| (pid, start, p.accumulated_cpu_time() * 1_000_000))
            })
            .collect()
    }
    fn tick(&mut self, watched: &[(u32, u64)]) -> Tick {
        self.system.refresh_cpu_usage();
        let now = Instant::now();
        let mut processes = Vec::with_capacity(watched.len());
        let mut times = HashMap::with_capacity(watched.len());
        // Accumulated CPU time avoids sysinfo keeping a stale rate for an idle process.
        for (pid, start, total) in self.cpu_times(watched) {
            if let Some((old, at)) = self.cpu_times.get(&(pid, start)) {
                let elapsed = now.duration_since(*at).as_nanos() as f64;
                if elapsed >= 1e8 {
                    let cpu = total.saturating_sub(*old) as f64 / elapsed * 100.0;
                    processes.push((pid, cpu as f32));
                }
            }
            times.insert((pid, start), (total, now));
        }
        self.cpu_times = times;
        let vm = platform::vm_counters();
        let previous = std::mem::replace(&mut self.vm, vm);
        let delta = |field: fn(&platform::VmCounters) -> u64| match (previous, vm) {
            (Some(a), Some(b)) if field(&b) >= field(&a) => Some(field(&b) - field(&a)),
            _ => None,
        };
        let readings = platform::readings();
        Tick {
            at_ms: now_ms(),
            cpu_percent: self.system.global_cpu_usage().clamp(0.0, 100.0),
            cores: self
                .system
                .cpus()
                .iter()
                .map(|c| c.cpu_usage().clamp(0.0, 100.0).round() as u8)
                .collect(),
            lag_ms: 0.0,
            swap_in_bytes: delta(|v| v.swap_in),
            swap_out_bytes: delta(|v| v.swap_out),
            page_in_bytes: delta(|v| v.page_in),
            compressed_bytes: delta(|v| v.compressed),
            decompressed_bytes: delta(|v| v.decompressed),
            memory_pressure: match readings.memory_pressure.as_deref() {
                Some("normal") => Some(0),
                Some("warning") => Some(1),
                Some("critical") => Some(2),
                _ => None,
            },
            thermal: match readings.thermal.as_deref() {
                Some("nominal") => Some(0),
                Some("fair") => Some(1),
                Some("serious") => Some(2),
                Some("critical") => Some(3),
                _ => None,
            },
            processes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(at_ms: u64) -> Tick {
        Tick {
            at_ms,
            ..Default::default()
        }
    }

    #[test]
    fn ring_is_bounded_and_restarts_after_gaps_or_clock_jumps() {
        let mut ring = Ring::default();
        for n in 0..300 {
            ring.push(tick(1_000_000 + n * 1000));
        }
        assert_eq!(ring.len(), CAPACITY);
        assert_eq!(ring.latest(1)[0].at_ms, 1_000_000 + 299 * 1000);
        assert_eq!(
            ring.window(1_000_000 + 290_000, 1_000_000 + 295_000).len(),
            6
        );
        ring.push(tick(1_000_000 + 310_000));
        assert_eq!(ring.len(), 1, "a sleep gap must not join two runs");
        ring.push(tick(1_000));
        assert_eq!(ring.len(), 1, "a backwards clock must not join two runs");
    }

    #[test]
    fn lateness_reports_probe_delay_and_only_gross_tick_delays() {
        let ms = Duration::from_millis;
        assert_eq!(late_ms(ms(10), ms(10), ms(0)), 0.0);
        assert_eq!(late_ms(ms(9), ms(10), ms(0)), 0.0);
        assert!((late_ms(ms(35), ms(10), ms(40)) - 25.0).abs() < 0.01);
        // Ordinary timer leeway on the one-second wait is ignored...
        assert!((late_ms(ms(12), ms(10), ms(200)) - 2.0).abs() < 0.01);
        // ...but a tick that itself ran far behind is a stall.
        assert!((late_ms(ms(12), ms(10), ms(900)) - 900.0).abs() < 0.01);
    }

    #[test]
    fn snapshot_only_mode_never_starts_the_sampler_until_pressed() {
        let pulse = Pulse::new();
        pulse.set_mode(Mode::SnapshotOnly);
        pulse.watch(&crate::automation::tests::sample(100, 5.0));
        pulse.set_mode(Mode::SnapshotOnly);
        assert!(!pulse.is_running());
        assert_eq!(pulse.tick_count(), 0);
        // A press records forward only, then the thread stops by itself.
        assert_eq!(
            pulse.press().unwrap(),
            Duration::from_millis(jitter::FORWARD_MS)
        );
        assert!(pulse.is_running());
        assert!(pulse.press().is_err(), "only one capture at a time");
        let capture = pulse.freeze(now_ms(), None);
        assert!(capture.forward_only);
        pulse.lock().record_until = Some(Instant::now());
        pulse.inner.wake.notify_all();
        let deadline = Instant::now() + Duration::from_secs(3);
        while pulse.is_running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!pulse.is_running());
    }

    #[test]
    fn auto_mode_samples_and_switching_off_stops_the_thread() {
        let pulse = Pulse::new();
        pulse.set_mode(Mode::Auto);
        assert!(pulse.is_running());
        assert_eq!(
            pulse.press().unwrap(),
            Duration::from_millis(jitter::POST_ROLL_MS)
        );
        pulse.release();
        pulse.set_mode(Mode::SnapshotOnly);
        let deadline = Instant::now() + Duration::from_secs(3);
        while pulse.is_running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!pulse.is_running());
    }

    #[test]
    fn watched_identities_are_bounded_to_the_busiest_processes() {
        let pulse = Pulse::new();
        let mut s = crate::automation::tests::sample(100, 5.0);
        s.processes = (0..40)
            .map(|n| crate::types::ProcessRecord {
                pid: 10_000 + n,
                start_time: 1,
                name: format!("p{n}"),
                workload_id: "fixture".into(),
                cpu_percent: n as f32,
                ..Default::default()
            })
            .collect();
        pulse.watch(&s);
        let state = pulse.lock();
        assert_eq!(state.watched.len(), WATCHED);
        assert_eq!(state.watched[0].0, 10_039);
        assert_eq!(state.identities.len(), WATCHED);
        assert_eq!(state.identities[&10_039].label, "Example");
    }
}
