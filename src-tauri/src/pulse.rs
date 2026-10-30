//! One-second pulse sampler for short hitches the five-second collector cannot see.
//! It keeps the last 120 ticks in memory only and never refreshes the full sysinfo table.
//! On macOS each tick reads every process's CPU time natively, so a process that starts or
//! peaks between five-second samples is still seen; elsewhere it follows the 16 busiest.
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
/// Busiest processes kept per tick from the all-process scan.
pub const TOP: usize = 16;
/// Ignore idle noise when ranking the all-process scan (percent of one core).
const SCAN_FLOOR: f32 = 0.5;
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
    /// Busiest PIDs and their CPU over the tick, where 100 means one logical core.
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
    pub parent_pid: Option<u32>,
    /// Outermost app bundle holding the executable; `None` for commands an app launched.
    pub bundle: Option<String>,
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

/// Busiest PID rates for one tick, plus native details for naming them.
type Scanned = (Vec<(u32, f32)>, Vec<platform::NativeProcess>);

type SpikeHandler = Arc<dyn Fn(Capture) + Send + Sync>;

#[derive(Default)]
struct State {
    mode: Mode,
    ring: Ring,
    watched: Vec<(u32, u64)>,
    identities: HashMap<u32, Identity>,
    /// Every process from the last full sample, for naming PIDs the scan finds busy.
    known: HashMap<u32, Identity>,
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
        let identity = |p: &crate::types::ProcessRecord| {
            let workload = snapshot.workloads.iter().find(|w| w.id == p.workload_id);
            Identity {
                pid: p.pid,
                start_time: p.start_time,
                name: p.name.clone(),
                workload_id: p.workload_id.clone(),
                label: workload.map_or_else(|| p.name.clone(), |w| w.label.clone()),
                category: workload.map(|w| w.category.clone()).unwrap_or_default(),
                parent_pid: p.parent_pid,
                bundle: p
                    .executable
                    .as_deref()
                    .and_then(crate::attribution::application_bundle)
                    .and_then(|b| b.file_stem())
                    .map(|s| s.to_string_lossy().into_owned()),
            }
        };
        let mut top: Vec<_> = snapshot
            .processes
            .iter()
            .filter(|p| p.pid != std::process::id())
            .collect();
        top.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
        let known: HashMap<u32, Identity> = if cfg!(target_os = "macos") {
            top.iter().map(|p| (p.pid, identity(p))).collect()
        } else {
            HashMap::new()
        };
        top.truncate(WATCHED);
        let mut state = self.lock();
        state.logical_cpus = snapshot.logical_cpus;
        state.known = known;
        state.watched = top.iter().map(|p| (p.pid, p.start_time)).collect();
        for p in top {
            state.identities.insert(p.pid, identity(p));
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
        let (mut tick, found) = sampler.tick(&watched);
        let work = started.elapsed();
        tick.lag_ms = lateness(PROBE, late);
        next += INTERVAL;
        if next <= Instant::now() {
            next = Instant::now() + INTERVAL;
        }
        let due = {
            let mut state = inner.state.lock().unwrap_or_else(|p| p.into_inner());
            name(&mut state, &tick, found);
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

/// Give every PID in a tick an identity: the last full sample's when the start time matches,
/// otherwise the native scan's own name and parent (a process newer than that sample).
fn name(state: &mut State, tick: &Tick, found: Vec<platform::NativeProcess>) {
    for (pid, _) in &tick.processes {
        let start = found.iter().find(|n| n.pid == *pid).map(|n| n.start_time);
        let current = state.identities.get(pid).map(|i| i.start_time);
        if current.is_some() && (start.is_none() || current == start) {
            continue;
        }
        let identity = match state.known.get(pid) {
            Some(known) if start.is_none_or(|s| s == known.start_time) => known.clone(),
            _ => {
                let Some(native) = found.iter().find(|n| n.pid == *pid) else {
                    continue;
                };
                Identity {
                    pid: native.pid,
                    start_time: native.start_time,
                    name: native.name.to_string(),
                    workload_id: format!("p-{}-{}", native.pid, native.start_time),
                    label: native.name.to_string(),
                    category: crate::attribution::category(&native.name).0.into(),
                    parent_pid: native.parent_pid,
                    bundle: platform::bundle_name(native.pid),
                }
            }
        };
        state.identities.insert(*pid, identity);
    }
}

/// Rank one all-process scan against the previous one. A process that started after the
/// previous scan is rated over its own lifetime, so a new hog shows up on its first tick.
pub fn rank(
    scan: &[platform::NativeProcess],
    previous: &HashMap<(u32, u64), u64>,
    previous_us: Option<u64>,
    now_us: u64,
    elapsed_ns: f64,
    exclude: u32,
) -> Vec<(u32, f32)> {
    let mut rates: Vec<(u32, f32)> = scan
        .iter()
        .filter(|p| p.pid != exclude)
        .filter_map(|p| {
            let rate = match previous.get(&(p.pid, p.start_time)) {
                Some(old) if elapsed_ns >= 1e8 => {
                    p.cpu_ns.saturating_sub(*old) as f64 / elapsed_ns * 100.0
                }
                Some(_) => return None,
                None => {
                    let born = previous_us.is_some_and(|at| p.start_us > at);
                    let age_ns = now_us.saturating_sub(p.start_us) as f64 * 1000.0;
                    if !born || age_ns < 5e7 {
                        return None;
                    }
                    p.cpu_ns as f64 / age_ns * 100.0
                }
            };
            Some((p.pid, rate as f32))
        })
        .filter(|(_, rate)| *rate >= SCAN_FLOOR)
        .collect();
    rates.sort_by(|a, b| b.1.total_cmp(&a.1));
    rates.truncate(TOP);
    rates
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
    /// Previous all-process scan: CPU time per identity and when it was read.
    scan: HashMap<(u32, u64), u64>,
    scanned: Option<(Instant, u64)>,
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
            scan: HashMap::new(),
            scanned: None,
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
    /// All-process scan (macOS). `None` when unsupported, so callers use the watched PIDs.
    fn scan(&mut self, now: Instant) -> Option<Scanned> {
        let scan = platform::all_processes();
        if scan.is_empty() {
            return None;
        }
        let now_us = now_ms() * 1000;
        let elapsed = self
            .scanned
            .map(|(at, _)| now.duration_since(at).as_nanos() as f64)
            .unwrap_or(0.0);
        let top = rank(
            &scan,
            &self.scan,
            self.scanned.map(|(_, us)| us),
            now_us,
            elapsed,
            std::process::id(),
        );
        self.scan = scan
            .iter()
            .map(|p| ((p.pid, p.start_time), p.cpu_ns))
            .collect();
        self.scanned = Some((now, now_us));
        let found = scan
            .into_iter()
            .filter(|p| top.iter().any(|(pid, _)| *pid == p.pid))
            .collect();
        Some((top, found))
    }
    fn tick(&mut self, watched: &[(u32, u64)]) -> (Tick, Vec<platform::NativeProcess>) {
        self.system.refresh_cpu_usage();
        let now = Instant::now();
        let (processes, found) = match self.scan(now) {
            Some(scanned) => scanned,
            None => (self.watched_rates(watched, now), Vec::new()),
        };
        (self.reading(processes), found)
    }
    fn watched_rates(&mut self, watched: &[(u32, u64)], now: Instant) -> Vec<(u32, f32)> {
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
        processes
    }
    fn reading(&mut self, processes: Vec<(u32, f32)>) -> Tick {
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
    fn scan_ranks_deltas_and_rates_new_processes_over_their_lifetime() {
        let p = |pid: u32, start: u64, start_us: u64, cpu_ns: u64| platform::NativeProcess {
            pid,
            start_time: start,
            start_us,
            parent_pid: Some(1),
            cpu_ns,
            name: format!("p{pid}").into(),
        };
        let previous: HashMap<(u32, u64), u64> =
            [((10, 5), 1_000_000_000), ((11, 5), 0), ((12, 5), 0)].into();
        let now_us = 100_000_000;
        let scan = vec![
            // 0.5 s of CPU in a 1 s tick: half a core.
            p(10, 5, 5_000_000, 1_500_000_000),
            // Idle noise stays out.
            p(11, 5, 5_000_000, 1_000_000),
            // Started 400 ms ago, after the previous scan, and used 380 ms: a new hog.
            p(20, 99, now_us - 400_000, 380_000_000),
            // Unknown but older than the previous scan: needs a second reading.
            p(21, 1, 1_000_000, 9_000_000_000),
            // The sampler itself is never a suspect.
            p(12, 5, 5_000_000, 900_000_000),
        ];
        let ranked = rank(&scan, &previous, Some(now_us - 1_000_000), now_us, 1e9, 12);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].0, 20);
        assert!((ranked[0].1 - 95.0).abs() < 0.5);
        assert_eq!(ranked[1].0, 10);
        assert!((ranked[1].1 - 50.0).abs() < 0.5);
        // The first scan has nothing to compare against.
        assert!(rank(&scan, &HashMap::new(), None, now_us, 0.0, 0).is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_scan_sees_this_process_and_its_cpu_time() {
        let scan = platform::all_processes();
        let me = scan
            .iter()
            .find(|p| p.pid == std::process::id())
            .expect("own process readable");
        assert!(me.cpu_ns > 0 && !me.name.is_empty() && me.parent_pid.is_some());
        assert!(scan.len() > 10);
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
