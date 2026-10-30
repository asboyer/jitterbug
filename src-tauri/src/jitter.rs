//! Deterministic Jitter Snapshot analysis over a frozen window of one-second pulse ticks.
//! Rules choose every finding, culprit, and action; a model may only explain them.
use crate::{
    automation::Origin,
    pulse::{Identity, Tick},
    types::{ProcessRecord, Snapshot},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
};

pub const LOOK_BACK_MS: u64 = 60_000;
pub const POST_ROLL_MS: u64 = 10_000;
pub const FORWARD_MS: u64 = 15_000;
/// Baseline is −60 s to −15 s; the suspect window is −15 s to +10 s around the press.
const SUSPECT_BEFORE_MS: u64 = 15_000;
/// A 10 ms sleep that wakes this late is a noticeable scheduling delay.
pub const LAG_MS: f32 = 20.0;
/// Sustained lag this high for three ticks counts as a short spike in Auto mode.
pub const SPIKE_LAG_MS: f32 = 30.0;
const PEGGED: f32 = 95.0;
const PAGING_FLOOR: f64 = 32.0 * 1048576.0;
const MAX_FINDINGS: usize = 6;
const TALLY_LIMIT: usize = 50;
const RECENT: usize = 6;
pub const RECURRING: usize = 3;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Victim {
    pub pid: u32,
    pub name: Option<String>,
}

/// One frozen window. Memory only; it never leaves the process except as a bounded summary.
#[derive(Clone, Debug, Default)]
pub struct Capture {
    pub origin: Origin,
    pub press_at_ms: u64,
    pub forward_only: bool,
    pub ticks: Vec<Tick>,
    pub identities: Vec<Identity>,
    pub victim: Option<Victim>,
    pub logical_cpus: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    Unexplained,
    Possible,
    Strong,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    ProcessSpike,
    CorePegged,
    MemoryThrash,
    ThermalEscalation,
    SchedulerLag,
    VictimBusy,
    NothingFound,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
    pub kind: FindingKind,
    pub strength: Strength,
    pub title: String,
    /// Measured facts from the capture.
    pub observed: Vec<String>,
    /// Interpretation of those facts; never a measurement.
    pub inferred: String,
    pub workload_id: Option<String>,
    pub label: Option<String>,
    pub category: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Point {
    pub offset_s: i32,
    pub cpu_percent: f32,
    pub core_max: f32,
    pub lag_ms: f32,
    pub paging_mib_s: Option<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Party {
    pub workload_id: Option<String>,
    pub label: String,
    pub category: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Suggestion {
    pub action: String,
    pub process: ProcessRecord,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub origin: Origin,
    pub press_at_ms: u64,
    pub forward_only: bool,
    pub window: [i32; 2],
    pub suspect: [i32; 2],
    pub timeline: Vec<Point>,
    pub findings: Vec<Finding>,
    pub victim: Option<Party>,
    pub culprit: Option<Party>,
    pub happened: String,
    pub likely_cause: String,
    pub in_app: Option<Suggestion>,
    pub manual: Option<String>,
    pub if_again: String,
    pub recurrence: usize,
    pub offer_permission: bool,
    pub limits: String,
}

struct Windows<'a> {
    baseline: Vec<&'a Tick>,
    suspect: Vec<&'a Tick>,
    rel: Box<dyn Fn(u64) -> i32 + 'a>,
}

fn split(capture: &Capture) -> Windows<'_> {
    let press = capture.press_at_ms as i64;
    let rel = move |at: u64| ((at as i64 - press) as f64 / 1000.0).round() as i32;
    let start = press - SUSPECT_BEFORE_MS as i64;
    let (baseline, suspect) = if capture.forward_only {
        (vec![], capture.ticks.iter().collect())
    } else {
        capture.ticks.iter().partition(|t| (t.at_ms as i64) < start)
    };
    Windows {
        baseline,
        suspect,
        rel: Box::new(rel),
    }
}

fn mean(values: impl Iterator<Item = f32>) -> f32 {
    let (sum, n) = values.fold((0.0, 0), |(s, n), v| (s + v, n + 1));
    if n == 0 {
        0.0
    } else {
        sum / n as f32
    }
}

fn shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘⇧J"
    } else {
        "Ctrl+Shift+J"
    }
}

fn pressure_name(level: u8) -> &'static str {
    ["normal", "warning", "critical"][level.min(2) as usize]
}

fn thermal_name(level: u8) -> &'static str {
    ["nominal", "fair", "serious", "critical"][level.min(3) as usize]
}

/// Per-workload CPU from the pulse PIDs and the collector's five-second workload history.
struct Load {
    id: String,
    label: String,
    category: String,
    base: f32,
    peak: f32,
    peak_at: i32,
    sustained: usize,
    pid: Option<(u32, u64)>,
    pulse: bool,
}

fn loads(capture: &Capture, snapshot: &Snapshot, w: &Windows) -> Vec<Load> {
    let cores = capture.logical_cpus.max(1) as f32;
    let identities: HashMap<u32, &Identity> =
        capture.identities.iter().map(|i| (i.pid, i)).collect();
    let mut found: HashMap<String, Load> = HashMap::new();
    let series = |tick: &Tick, id: &str| -> f32 {
        tick.processes
            .iter()
            .filter(|(pid, _)| identities.get(pid).is_some_and(|i| i.workload_id == id))
            .map(|(_, cpu)| cpu)
            .sum()
    };
    for identity in &capture.identities {
        if found.contains_key(&identity.workload_id) {
            continue;
        }
        let id = identity.workload_id.as_str();
        let base = mean(w.baseline.iter().map(|t| series(t, id)));
        let (peak, peak_at) = w
            .suspect
            .iter()
            .map(|t| (series(t, id), (w.rel)(t.at_ms)))
            .fold((0.0_f32, 0), |a, b| if b.0 > a.0 { b } else { a });
        found.insert(
            id.into(),
            Load {
                id: id.into(),
                label: identity.label.clone(),
                category: identity.category.clone(),
                base,
                peak,
                peak_at,
                sustained: 0,
                pid: None,
                pulse: true,
            },
        );
    }
    // Five-second history covers workloads outside the 16 watched PIDs and pre-press baselines.
    let press_s = capture.press_at_ms / 1000;
    let (lo, mid, hi) = if capture.forward_only {
        (press_s.saturating_sub(60), press_s, press_s + 15)
    } else {
        (
            press_s.saturating_sub(60),
            press_s.saturating_sub(15),
            press_s + 10,
        )
    };
    for workload in &snapshot.workloads {
        let before: Vec<_> = workload
            .history
            .iter()
            .filter(|p| (lo..mid).contains(&p.time))
            .map(|p| p.cpu_percent * cores)
            .collect();
        let during = workload
            .history
            .iter()
            .filter(|p| (mid..=hi).contains(&p.time))
            .map(|p| (p.cpu_percent * cores, p.time as i64 - press_s as i64))
            .fold((0.0_f32, 0_i64), |a, b| if b.0 > a.0 { b } else { a });
        let base = mean(before.iter().copied());
        let entry = found.entry(workload.id.clone()).or_insert_with(|| Load {
            id: workload.id.clone(),
            label: workload.label.clone(),
            category: workload.category.clone(),
            base,
            peak: during.0,
            peak_at: during.1 as i32,
            sustained: 0,
            pid: None,
            pulse: false,
        });
        // The longer, complete history is the safer baseline when the PID was not yet watched.
        entry.base = entry.base.max(base);
        if !entry.pulse && during.0 > entry.peak {
            entry.peak = during.0;
            entry.peak_at = during.1 as i32;
        }
    }
    for load in found.values_mut() {
        let threshold = (load.base.max(1.0) * 3.0).max(25.0);
        if load.pulse {
            load.sustained = w
                .suspect
                .iter()
                .filter(|t| series(t, &load.id) >= threshold)
                .count();
        }
        load.pid = capture
            .identities
            .iter()
            .filter(|i| i.workload_id == load.id)
            .max_by(|a, b| {
                let cpu = |pid: u32| {
                    w.suspect
                        .iter()
                        .flat_map(|t| t.processes.iter())
                        .filter(|(p, _)| *p == pid)
                        .map(|(_, c)| *c)
                        .fold(0.0_f32, f32::max)
                };
                cpu(a.pid).total_cmp(&cpu(b.pid))
            })
            .map(|i| (i.pid, i.start_time));
    }
    let mut loads: Vec<_> = found.into_values().collect();
    loads.sort_by(|a, b| (b.peak - b.base).total_cmp(&(a.peak - a.base)));
    loads
}

fn party(snapshot: &Snapshot, capture: &Capture) -> Option<Party> {
    let victim = capture.victim.as_ref()?;
    let process = snapshot.processes.iter().find(|p| p.pid == victim.pid);
    let workload = process.and_then(|p| snapshot.workloads.iter().find(|w| w.id == p.workload_id));
    match (workload, &victim.name) {
        (Some(w), _) => Some(Party {
            workload_id: Some(w.id.clone()),
            label: victim.name.clone().unwrap_or_else(|| w.label.clone()),
            category: w.category.clone(),
        }),
        (None, Some(name)) => Some(Party {
            workload_id: None,
            label: name.clone(),
            category: "app".into(),
        }),
        (None, None) => process.map(|p| Party {
            workload_id: Some(p.workload_id.clone()),
            label: p.name.clone(),
            category: String::new(),
        }),
    }
}

/// Shells and launch plumbing between an app and the command running in it.
const WRAPPERS: &[&str] = &[
    "login",
    "zsh",
    "bash",
    "fish",
    "sh",
    "dash",
    "nu",
    "tmux",
    "screen",
    "script",
    "sudo",
    "pwsh",
    "powershell",
    "cmd",
];

fn wrapper(name: &str) -> bool {
    let name = name.trim_start_matches('-').to_lowercase();
    WRAPPERS.contains(&name.strip_suffix(".exe").unwrap_or(&name))
}

fn bundle(p: &ProcessRecord) -> Option<String> {
    p.executable
        .as_deref()
        .and_then(crate::attribution::application_bundle)
        .and_then(|b| b.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
}

/// A command launched inside an app (a terminal command, an agent session, a script).
#[derive(Clone, Debug, PartialEq)]
struct Command {
    id: String,
    label: String,
    /// The app workload the collector folded this command into.
    app_workload: String,
}

/// Walk up through shells to the app that launched this process. The command's root is the
/// outermost non-shell ancestor below the app, so a session's children group together.
fn command(
    by_pid: &HashMap<u32, &ProcessRecord>,
    (pid, start, name): (u32, u64, &str),
    parent: Option<u32>,
    own_bundle: Option<&str>,
) -> Option<Command> {
    if own_bundle.is_some() {
        return None;
    }
    let (mut root_pid, mut root_start, mut root_name) = (pid, start, name.to_string());
    let mut child_start = start;
    let mut current = parent;
    for _ in 0..16 {
        let p = by_pid.get(&current?)?;
        // A reused parent PID must not adopt an unrelated process.
        if p.start_time > child_start {
            return None;
        }
        if let Some(app) = bundle(p) {
            let folder = by_pid
                .get(&root_pid)
                .filter(|r| r.start_time == root_start)
                .and_then(|r| r.working_directory.as_deref())
                .and_then(|d| d.file_name())
                .map(|f| f.to_string_lossy().chars().take(40).collect::<String>());
            return Some(Command {
                id: format!("cmd-{root_pid}-{root_start}"),
                label: match folder {
                    Some(folder) => format!("{root_name} ({folder}) in {app}"),
                    None => format!("{root_name} in {app}"),
                },
                app_workload: p.workload_id.clone(),
            });
        }
        if !wrapper(&p.name) {
            (root_pid, root_start, root_name) = (p.pid, p.start_time, p.name.clone());
        }
        child_start = p.start_time;
        current = p.parent_pid;
    }
    None
}

/// Commands that the collector folded into an app workload (or that started after the last
/// full sample) become their own suspects, keyed by command. Returns the regrouped capture and
/// the command of every current process that belongs to one.
fn regroup(
    capture: &Capture,
    snapshot: &Snapshot,
) -> (Capture, HashMap<u32, Command>, HashMap<String, String>) {
    let by_pid: HashMap<u32, &ProcessRecord> =
        snapshot.processes.iter().map(|p| (p.pid, p)).collect();
    let folded =
        |workload: &str, cmd: &Command| workload == cmd.app_workload || workload.starts_with("p-");
    let mut capture = capture.clone();
    let mut apps = HashMap::new();
    for identity in &mut capture.identities {
        if let Some(cmd) = command(
            &by_pid,
            (identity.pid, identity.start_time, &identity.name),
            identity.parent_pid,
            identity.bundle.as_deref(),
        )
        .filter(|cmd| folded(&identity.workload_id, cmd))
        {
            apps.insert(cmd.id.clone(), cmd.app_workload);
            identity.workload_id = cmd.id;
            identity.label = cmd.label;
            identity.category = "command".into();
        }
    }
    let members: HashMap<u32, Command> = snapshot
        .processes
        .iter()
        .filter_map(|p| {
            let cmd = command(
                &by_pid,
                (p.pid, p.start_time, &p.name),
                p.parent_pid,
                bundle(p).as_deref(),
            )?;
            folded(&p.workload_id, &cmd).then_some((p.pid, cmd))
        })
        .collect();
    for cmd in members.values() {
        apps.insert(cmd.id.clone(), cmd.app_workload.clone());
    }
    (capture, members, apps)
}

/// The suspect key the analysis uses for a live process: its command inside an app when the
/// collector folded it into that app, otherwise its workload.
pub fn suspect_id(snapshot: &Snapshot, pid: u32) -> Option<String> {
    let by_pid: HashMap<u32, &ProcessRecord> =
        snapshot.processes.iter().map(|p| (p.pid, p)).collect();
    let p = by_pid.get(&pid)?;
    Some(
        command(
            &by_pid,
            (p.pid, p.start_time, &p.name),
            p.parent_pid,
            bundle(p).as_deref(),
        )
        .filter(|c| p.workload_id == c.app_workload || p.workload_id.starts_with("p-"))
        .map_or_else(|| p.workload_id.clone(), |c| c.id),
    )
}

pub fn analyze(capture: &Capture, snapshot: &Snapshot) -> Report {
    let (regrouped, commands, apps) = regroup(capture, snapshot);
    let capture = &regrouped;
    let w = split(capture);
    let victim = party(snapshot, capture);
    let victim_id = victim.as_ref().and_then(|v| v.workload_id.clone());
    let mut findings: Vec<Finding> = vec![];

    // Process spikes: ≥3× baseline and over a quarter of one core.
    let loads = loads(capture, snapshot, &w);
    for load in loads
        .iter()
        .filter(|l| l.peak >= 25.0 && l.peak >= l.base.max(1.0) * 3.0 && l.peak - l.base >= 25.0)
    {
        if findings
            .iter()
            .filter(|f| f.kind == FindingKind::ProcessSpike)
            .count()
            >= 3
        {
            break;
        }
        let strong = load.pulse && load.sustained >= 2;
        // Inside a command (an agent session, a shell script), say which process did the work.
        let busiest = load
            .pid
            .and_then(|(pid, _)| capture.identities.iter().find(|i| i.pid == pid))
            .filter(|i| {
                load.category == "command" && !load.label.starts_with(&format!("{} ", i.name))
            })
            .map(|i| format!("Busiest process in it: {}.", i.name));
        findings.push(Finding {
            kind: FindingKind::ProcessSpike,
            strength: if strong {
                Strength::Strong
            } else {
                Strength::Possible
            },
            title: format!("{} spiked", load.label),
            observed: vec![format!(
                "{} rose from {:.0}% to {:.0}% of one core (peak at {:+}s{}).",
                load.label,
                load.base,
                load.peak,
                load.peak_at,
                if load.pulse {
                    format!(", {} s above its spike threshold", load.sustained)
                } else {
                    ", from five-second samples".into()
                }
            )]
            .into_iter()
            .chain(busiest)
            .collect(),
            inferred: format!(
                "{} competed for CPU at the moment you noticed the slowdown.",
                load.label
            ),
            workload_id: Some(load.id.clone()),
            label: Some(load.label.clone()),
            category: Some(load.category.clone()),
        });
    }

    // A saturated core, even when whole-machine CPU looks low.
    let mut run = (0usize, 0usize, 0usize); // (best length, best start, current length)
    for (i, t) in w.suspect.iter().enumerate() {
        run.2 = if t.core_max() >= PEGGED { run.2 + 1 } else { 0 };
        if run.2 > run.0 {
            run = (run.2, i + 1 - run.2, run.2);
        }
    }
    let identities: HashMap<u32, &Identity> =
        capture.identities.iter().map(|i| (i.pid, i)).collect();
    // A single busy thread can migrate between cores, so a process near exactly one core counts.
    let thread = |ticks: &[&Tick], floor: f32, ceiling: f32| {
        let mut best: Option<(u32, f32)> = None;
        for identity in &capture.identities {
            let avg = mean(ticks.iter().map(|t| {
                t.processes
                    .iter()
                    .find(|(p, _)| *p == identity.pid)
                    .map_or(0.0, |(_, c)| *c)
            }));
            if (floor..=ceiling).contains(&avg) && best.is_none_or(|(_, b)| avg > b) {
                best = Some((identity.pid, avg));
            }
        }
        best
    };
    // (run length, start index, busiest single-thread process and its average)
    type Pegged = (usize, usize, Option<(u32, f32)>);
    let mut pegged: Option<Pegged> = None;
    if run.0 >= 2 {
        let ticks = &w.suspect[run.1..run.1 + run.0];
        pegged = Some((run.0, run.1, thread(ticks, 80.0, f32::MAX)));
    } else {
        for start in 0..w.suspect.len().saturating_sub(1) {
            let pair = &w.suspect[start..start + 2];
            if mean(pair.iter().map(|t| t.cpu_percent)) < 50.0 {
                if let Some(hit) = thread(pair, 90.0, 110.0) {
                    pegged = Some((2, start, Some(hit)));
                    break;
                }
            }
        }
    }
    if let Some((length, start, hit)) = pegged {
        let ticks = &w.suspect[start..start + length];
        let total = mean(ticks.iter().map(|t| t.cpu_percent));
        let who = hit.and_then(|(pid, avg)| identities.get(&pid).map(|i| (*i, avg)));
        let mut observed = vec![format!(
            "A CPU core stayed at or near 100% for {length} s starting {:+}s, while whole-machine CPU averaged {total:.0}%.",
            (w.rel)(ticks[0].at_ms)
        )];
        if let Some((i, avg)) = who {
            observed.push(format!(
                "{} ({}) averaged {avg:.0}% of one core in that run.",
                i.label, i.name
            ));
        }
        findings.push(Finding {
            kind: FindingKind::CorePegged,
            strength: if who.is_some() {
                Strength::Strong
            } else {
                Strength::Possible
            },
            title: match who {
                Some((i, _)) => format!("{} saturated one core", i.label),
                None => "One CPU core was saturated".into(),
            },
            observed,
            inferred: "Single-threaded work cannot use the other cores, so anything waiting on that thread stalls even when total CPU looks low.".into(),
            workload_id: who.map(|(i, _)| i.workload_id.clone()),
            label: who.map(|(i, _)| i.label.clone()),
            category: who.map(|(i, _)| i.category.clone()),
        });
    }

    // Memory thrash: a paging burst, ideally with rising OS memory pressure.
    let rate = |t: &&Tick| t.paging_bytes().map(|b| b as f64);
    if w.suspect.iter().any(|t| t.paging_bytes().is_some()) {
        let base = {
            let v: Vec<f64> = w.baseline.iter().filter_map(rate).collect();
            v.iter().sum::<f64>() / v.len().max(1) as f64
        };
        let peak = w.suspect.iter().filter_map(rate).fold(0.0, f64::max);
        let swap_in: u64 = w.suspect.iter().filter_map(|t| t.swap_in_bytes).sum();
        let before = w.baseline.iter().filter_map(|t| t.memory_pressure).max();
        let during = w.suspect.iter().filter_map(|t| t.memory_pressure).max();
        let rise = match (before, during) {
            (Some(b), Some(d)) => d > b,
            (None, Some(d)) => d >= 1,
            _ => false,
        };
        let burst = peak >= PAGING_FLOOR && peak >= base * 4.0;
        if burst || (rise && swap_in > 0) {
            let growth = snapshot
                .workloads
                .iter()
                .filter(|wl| Some(&wl.id) != victim_id.as_ref())
                .filter_map(|wl| {
                    let first = wl.history.first()?.memory_bytes;
                    let last = wl.history.last()?.memory_bytes;
                    (last >= first + (256 << 20)).then_some((wl, last - first))
                })
                .max_by_key(|(_, g)| *g);
            let mut observed = vec![format!(
                "Swap and compressor traffic peaked at {:.0} MiB/s (baseline {:.1} MiB/s).",
                peak / 1048576.0,
                base / 1048576.0
            )];
            if let (Some(b), Some(d)) = (before.or(Some(0)), during) {
                if rise {
                    observed.push(format!(
                        "OS memory pressure rose from {} to {}.",
                        pressure_name(b),
                        pressure_name(d)
                    ));
                }
            }
            if let Some((wl, g)) = growth {
                observed.push(format!(
                    "{}'s memory grew by {:.1} GiB over the last two minutes.",
                    wl.label,
                    g as f64 / 1073741824.0
                ));
            }
            findings.push(Finding {
                kind: FindingKind::MemoryThrash,
                strength: if burst && (rise || during.is_some_and(|d| d >= 1)) {
                    Strength::Strong
                } else {
                    Strength::Possible
                },
                title: "Memory was paging".into(),
                observed,
                inferred: match growth {
                    Some((wl, _)) => format!("The Mac was moving memory to and from swap or the compressor; every app slows while that happens. {} grew the most, which makes it a likely contributor, not a proven cause.", wl.label),
                    None => "The Mac was moving memory to and from swap or the compressor; every app slows while that happens.".into(),
                },
                workload_id: growth.map(|(wl, _)| wl.id.clone()),
                label: growth.map(|(wl, _)| wl.label.clone()),
                category: growth.map(|(wl, _)| wl.category.clone()),
            });
        }
    }

    // Thermal escalation.
    let before = w.baseline.iter().filter_map(|t| t.thermal).max();
    let during = w.suspect.iter().filter_map(|t| t.thermal).max();
    if let Some(d) = during {
        let b = before.unwrap_or(if capture.forward_only { 1 } else { d });
        if d > b || (capture.forward_only && d >= 2) {
            findings.push(Finding {
                kind: FindingKind::ThermalEscalation,
                strength: if d >= 2 {
                    Strength::Strong
                } else {
                    Strength::Possible
                },
                title: "Your Mac got warmer".into(),
                observed: vec![format!(
                    "The OS thermal state rose from {} to {}.",
                    thermal_name(b.min(d)),
                    thermal_name(d)
                )],
                inferred: "When warm, the system slows the CPU to cool down; everything gets slower without any one process spiking.".into(),
                workload_id: None,
                label: None,
                category: None,
            });
        }
    }

    // Scheduler lag: supporting evidence, or the only symptom.
    let lag_base = w.baseline.iter().map(|t| t.lag_ms).fold(0.0_f32, f32::max);
    let (lag_peak, lag_at) = w
        .suspect
        .iter()
        .map(|t| (t.lag_ms, (w.rel)(t.at_ms)))
        .fold((0.0_f32, 0), |a, b| if b.0 > a.0 { b } else { a });
    let lagged = lag_peak >= LAG_MS && lag_peak >= lag_base.max(1.0) * 3.0;
    let lag_line = format!(
        "Jitterbug's own 10 ms timer woke {lag_peak:.0} ms late at {lag_at:+}s (baseline worst {lag_base:.0} ms)."
    );

    // A command running inside the frontmost app (a terminal command, an agent session) is a
    // separate suspect. When one spiked, the app's own folded-in total is that command's load.
    let inside_victim = |f: &Finding| {
        f.kind == FindingKind::ProcessSpike
            && f.workload_id
                .as_ref()
                .and_then(|id| apps.get(id))
                .is_some_and(|app| Some(app) == victim_id.as_ref())
    };
    if findings.iter().any(inside_victim) {
        if let Some(vid) = &victim_id {
            findings.retain(|f| f.workload_id.as_ref() != Some(vid));
        }
    }
    // The frontmost app is the victim: its own work is not a culprit to deprioritize.
    if let Some(vid) = &victim_id {
        for f in findings.iter_mut().filter(|f| {
            matches!(f.kind, FindingKind::ProcessSpike | FindingKind::CorePegged)
                && f.workload_id.as_ref() == Some(vid)
        }) {
            let name = victim
                .as_ref()
                .map_or("The app you were using", |v| &v.label);
            f.kind = FindingKind::VictimBusy;
            f.title = format!("{name} was busy with its own work");
            f.inferred = format!("The slowdown most likely came from {name}'s own work rather than competition from other apps. Lowering its priority would not help.");
        }
    }
    findings.sort_by_key(|f| std::cmp::Reverse(f.strength));
    if lagged {
        if let Some(first) = findings.first_mut() {
            first.observed.push(lag_line.clone());
        } else {
            findings.push(Finding {
                kind: FindingKind::SchedulerLag,
                strength: Strength::Unexplained,
                title: "Scheduling delay with no visible cause".into(),
                observed: vec![lag_line],
                inferred: "Something delayed ordinary threads, but no busy watched process, saturated core, memory paging, or heat was measured. GPU, display, disk, and network activity are not measured and may explain it.".into(),
                workload_id: None,
                label: None,
                category: None,
            });
        }
    }
    if findings.is_empty() {
        findings.push(Finding {
            kind: FindingKind::NothingFound,
            strength: Strength::Unexplained,
            title: "No measured cause".into(),
            observed: vec![if capture.ticks.is_empty() {
                "No pulse samples covered this moment.".into()
            } else {
                "CPU, individual cores, memory paging, thermal state, and scheduling stayed near their baseline.".into()
            }],
            inferred: "GPU, display, disk, and network activity are not measured, so the hitch may have come from there.".into(),
            workload_id: None,
            label: None,
            category: None,
        });
    }
    findings.truncate(MAX_FINDINGS);

    // Culprits never include the victim. Actions come only from existing permitted actions.
    let culprit_finding = findings.iter().find(|f| {
        f.workload_id.is_some()
            && f.workload_id != victim_id
            && matches!(
                f.kind,
                FindingKind::ProcessSpike | FindingKind::CorePegged | FindingKind::MemoryThrash
            )
    });
    let culprit = culprit_finding.map(|f| Party {
        workload_id: f.workload_id.clone(),
        label: f.label.clone().unwrap_or_default(),
        category: f.category.clone().unwrap_or_default(),
    });
    let in_app = culprit_finding.and_then(|f| suggestion(snapshot, f, &loads, &commands));
    let top = &findings[0];
    let victim_name = victim.as_ref().map(|v| v.label.clone());
    let manual = Some(match top.kind {
        FindingKind::ProcessSpike | FindingKind::CorePegged => format!(
            "If {} is not urgent, pause or quit it while you work{}.",
            top.label.as_deref().unwrap_or("that work"),
            victim_name
                .as_ref()
                .map(|v| format!(" in {v}"))
                .unwrap_or_default()
        ),
        FindingKind::MemoryThrash => {
            "Quit apps or close windows and tabs you do not need; paging slows every app.".into()
        }
        FindingKind::ThermalEscalation => {
            "Give the Mac airflow and let sustained work finish; heat slows the whole machine."
                .into()
        }
        FindingKind::VictimBusy => format!(
            "Check what {} was doing at that moment, such as a large file, an extension, or a heavy page.",
            victim_name.as_deref().unwrap_or("the app")
        ),
        FindingKind::SchedulerLag | FindingKind::NothingFound => format!(
            "Press {} again the next time it happens; repeated snapshots separate coincidence from cause.",
            shortcut()
        ),
    });
    let happened = match capture.origin {
        Origin::Spike => format!("Jitterbug noticed a short hitch. {}", top.observed[0]),
        _ => top.observed[0].clone(),
    };
    let timeline = capture
        .ticks
        .iter()
        .take(90)
        .map(|t| Point {
            offset_s: (w.rel)(t.at_ms),
            cpu_percent: t.cpu_percent,
            core_max: t.core_max(),
            lag_ms: t.lag_ms,
            paging_mib_s: t.paging_bytes().map(|b| b as f32 / 1048576.0),
        })
        .collect();
    Report {
        origin: capture.origin.clone(),
        press_at_ms: capture.press_at_ms,
        forward_only: capture.forward_only,
        window: if capture.forward_only {
            [0, (FORWARD_MS / 1000) as i32]
        } else {
            [
                -((LOOK_BACK_MS / 1000) as i32),
                (POST_ROLL_MS / 1000) as i32,
            ]
        },
        suspect: if capture.forward_only {
            [0, (FORWARD_MS / 1000) as i32]
        } else {
            [
                -((SUSPECT_BEFORE_MS / 1000) as i32),
                (POST_ROLL_MS / 1000) as i32,
            ]
        },
        timeline,
        likely_cause: top.inferred.clone(),
        findings,
        victim,
        culprit,
        happened,
        in_app,
        manual,
        if_again: if capture.forward_only {
            format!("Press {} the moment it happens. Snapshot-only mode records the 15 seconds after your press; Auto mode also keeps the previous 60 seconds in memory.", shortcut())
        } else {
            format!("Jitterbug takes a snapshot automatically when it detects pressure. If you feel a hitch it missed, press {} the moment it happens; the last 60 seconds of one-second samples stay in memory only.", shortcut())
        },
        recurrence: 0,
        offer_permission: false,
        limits: "One-second samples of CPU per core, the 16 busiest processes from the last full sample, system paging, OS memory pressure, thermal state, and a scheduler-delay probe. GPU, display, disk, and network activity are not measured. Process names come from the latest five-second inventory.".into(),
    }
}

fn suggestion(
    snapshot: &Snapshot,
    finding: &Finding,
    loads: &[Load],
    commands: &HashMap<u32, Command>,
) -> Option<Suggestion> {
    let id = finding.workload_id.as_ref()?;
    let members = snapshot.processes.iter().filter(|p| {
        commands
            .get(&p.pid)
            .map_or(&p.workload_id == id, |c| &c.id == id)
    });
    if finding.kind == FindingKind::MemoryThrash {
        // Priority does not free memory. Stopping is offered only where actions.rs allows it.
        if finding.strength != Strength::Strong {
            return None;
        }
        let process = members
            .filter(|p| p.actions.iter().any(|a| a == "terminate"))
            .max_by_key(|p| p.memory_bytes)?;
        return Some(Suggestion {
            action: "terminate".into(),
            process: process.clone(),
            label: finding.label.clone().unwrap_or_default(),
        });
    }
    let pid = loads.iter().find(|l| &l.id == id).and_then(|l| l.pid);
    let process = members
        .filter(|p| p.actions.iter().any(|a| a == "lower_priority"))
        .max_by(|a, b| {
            let preferred = |p: &ProcessRecord| pid == Some((p.pid, p.start_time));
            preferred(a)
                .cmp(&preferred(b))
                .then(a.cpu_percent.total_cmp(&b.cpu_percent))
        })?;
    Some(Suggestion {
        action: "lower_priority".into(),
        process: process.clone(),
        label: finding.label.clone().unwrap_or_default(),
    })
}

impl Report {
    /// The workload this snapshot implicates for the recurrence tally.
    pub fn implicated(&self) -> Option<String> {
        self.culprit.as_ref().map(|c| c.label.clone()).or_else(|| {
            self.findings
                .iter()
                .find(|f| f.kind == FindingKind::VictimBusy)
                .and_then(|f| f.label.clone())
        })
    }
    pub fn apply_recurrence(&mut self, tally: &Tally) {
        let Some(label) = self.implicated() else {
            return;
        };
        self.recurrence = tally.recurrence(&label);
        if self.recurrence >= 2 {
            self.if_again = format!(
                "{} {label} was implicated in {} of your last {} snapshots.",
                self.if_again,
                self.recurrence,
                tally.recent.len()
            );
        }
        self.offer_permission = self.recurrence >= RECURRING
            && self
                .in_app
                .as_ref()
                .is_some_and(|s| s.action == "lower_priority" && s.label == label);
    }
    /// Plain deterministic explanation used when no model answers.
    pub fn text(&self) -> String {
        let mut lines = vec![
            format!("What happened: {}", self.happened),
            format!("Likely cause: {}", self.likely_cause),
        ];
        let mut steps = vec![];
        if let Some(s) = &self.in_app {
            steps.push(format!(
                "{} {} in Jitterbug (needs your approval).",
                if s.action == "terminate" {
                    "Stop"
                } else {
                    "Lower the priority of"
                },
                s.process.name
            ));
        }
        if let Some(m) = &self.manual {
            steps.push(m.clone());
        }
        if !steps.is_empty() {
            lines.push(format!("What you can do: {}", steps.join(" ")));
        }
        lines.push(format!("If it happens again: {}", self.if_again));
        lines.push(
            "These are deterministic observations from one-second samples, not a model diagnosis."
                .into(),
        );
        lines.join("\n\n")
    }
    /// Bounded model context: findings and a five-second timeline summary. Project labels,
    /// paths, PIDs, and identifiers stay local.
    pub fn model_context(&self) -> Value {
        let mut aliases: Vec<(String, String)> = vec![];
        for (label, category) in self
            .findings
            .iter()
            .filter_map(|f| Some((f.label.clone()?, f.category.as_deref()?)))
            .chain(
                self.victim
                    .iter()
                    .chain(self.culprit.iter())
                    .map(|p| (p.label.clone(), p.category.as_str())),
            )
        {
            if category == "project" && !label.is_empty() {
                aliases.push((label, "a development workload".into()));
            } else if category == "command" {
                // "claude (my-project) in Code" → "claude in Code": folder names stay local.
                if let (Some(open), Some(close)) = (label.find(" ("), label.rfind(") in ")) {
                    let alias = format!("{}{}", &label[..open], &label[close + 1..]);
                    aliases.push((label, alias));
                }
            }
        }
        let clean = |text: &str| {
            aliases.iter().fold(text.to_string(), |t, (label, alias)| {
                t.replace(label, alias)
            })
        };
        let mut buckets: Vec<Value> = vec![];
        for chunk in self.timeline.chunks(5).take(18) {
            buckets.push(json!({
                "offset_s": chunk[0].offset_s,
                "cpu_percent_max": chunk.iter().map(|p| p.cpu_percent).fold(0.0, f32::max).round(),
                "busiest_core_max": chunk.iter().map(|p| p.core_max).fold(0.0, f32::max).round(),
                "scheduler_lag_ms_max": chunk.iter().map(|p| p.lag_ms).fold(0.0, f32::max).round(),
                "paging_mib_s_max": chunk.iter().filter_map(|p| p.paging_mib_s).reduce(f32::max).map(f32::round),
            }));
        }
        let findings: Vec<_> = self
            .findings
            .iter()
            .map(|f| {
                json!({
                    "kind": f.kind, "strength": f.strength, "title": clean(&f.title),
                    "observed": f.observed.iter().map(|o| clean(o)).collect::<Vec<_>>(),
                    "inferred": clean(&f.inferred),
                })
            })
            .collect();
        json!({
            "origin": self.origin,
            "window_seconds_relative_to_press": self.window,
            "suspect_window_seconds": self.suspect,
            "victim_frontmost_app": self.victim.as_ref().map(|v| clean(&v.label)),
            "culprit": self.culprit.as_ref().map(|c| clean(&c.label)),
            "findings": findings,
            "timeline_5s": buckets,
            "in_app_action": self.in_app.as_ref().map(|s| if s.action == "terminate" { "Stop process" } else { "Lower priority" }),
            "recurrence_in_recent_snapshots": self.recurrence,
            "limits": self.limits,
        })
    }
}

/// Rising edge of a sustained hitch over the newest ticks (oldest first): scheduler lag for
/// three seconds, or a saturated core for three seconds with lag on at least two of them.
/// Returns the start time of the run.
pub fn spike(ticks: &[Tick]) -> Option<u64> {
    let hot = |run: &[Tick]| {
        let lag = run.iter().filter(|t| t.lag_ms >= SPIKE_LAG_MS).count();
        let lagging = run.iter().filter(|t| t.lag_ms >= LAG_MS).count();
        lag == run.len() || (run.iter().all(|t| t.core_max() >= PEGGED) && lagging >= 2)
    };
    if ticks.len() < 3 {
        return None;
    }
    let run = &ticks[ticks.len() - 3..];
    let contiguous = run
        .windows(2)
        .all(|p| p[1].at_ms > p[0].at_ms && p[1].at_ms - p[0].at_ms <= 2500);
    if !contiguous || !hot(run) {
        return None;
    }
    // Fire once per episode: the tick before the run must not already have been part of it.
    if ticks.len() >= 4 && hot(&ticks[ticks.len() - 4..ticks.len() - 1]) {
        return None;
    }
    Some(run[0].at_ms)
}

/// Local recurrence tally: workload labels and counts only, never paths or process data.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Tally {
    pub entries: Vec<TallyEntry>,
    /// Implicated label (if any) for each of the most recent snapshots, newest last.
    pub recent: VecDeque<Option<String>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TallyEntry {
    pub label: String,
    pub count: u32,
    pub last_at: u64,
}

impl Tally {
    pub fn path(directory: &Path) -> PathBuf {
        directory.join("jitter-tally.json")
    }
    /// A missing or unreadable tally starts empty; it is never written to the repository.
    pub fn read(directory: &Path) -> Self {
        crate::storage::read(&Self::path(directory)).unwrap_or_default()
    }
    pub fn save(&self, directory: &Path) -> Result<(), String> {
        crate::storage::write(&Self::path(directory), self)
    }
    pub fn record(&mut self, label: Option<&str>, at: u64) {
        let label = label
            .map(|l| {
                l.chars()
                    .filter(|c| !c.is_control())
                    .take(80)
                    .collect::<String>()
            })
            .filter(|l| !l.trim().is_empty());
        self.recent.push_back(label.clone());
        while self.recent.len() > RECENT {
            self.recent.pop_front();
        }
        let Some(label) = label else { return };
        if let Some(entry) = self.entries.iter_mut().find(|e| e.label == label) {
            entry.count = entry.count.saturating_add(1);
            entry.last_at = at;
        } else {
            if self.entries.len() >= TALLY_LIMIT {
                // Evict the least implicated, least recent entry.
                if let Some(index) = self
                    .entries
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, e)| (e.count, e.last_at))
                    .map(|(i, _)| i)
                {
                    self.entries.remove(index);
                }
            }
            self.entries.push(TallyEntry {
                label,
                count: 1,
                last_at: at,
            });
        }
        self.entries
            .sort_by(|a, b| b.count.cmp(&a.count).then(b.last_at.cmp(&a.last_at)));
    }
    pub fn recurrence(&self, label: &str) -> usize {
        self.recent
            .iter()
            .filter(|l| l.as_deref() == Some(label))
            .count()
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::types::{ResourcePoint, Workload};

    pub const PRESS: u64 = 1_800_000_000_000;

    /// Seventy one-second ticks from −60 s to +10 s. `f` customizes each by offset.
    pub fn capture(f: impl Fn(i32, &mut Tick)) -> Capture {
        let ticks = (-60..=10)
            .map(|offset| {
                let mut t = Tick {
                    at_ms: (PRESS as i64 + offset as i64 * 1000) as u64,
                    cpu_percent: 10.0,
                    cores: vec![20, 10, 5, 5, 5, 5, 5, 5],
                    lag_ms: 0.4,
                    swap_in_bytes: Some(0),
                    swap_out_bytes: Some(0),
                    page_in_bytes: Some(0),
                    compressed_bytes: Some(0),
                    decompressed_bytes: Some(0),
                    memory_pressure: Some(0),
                    thermal: Some(0),
                    processes: vec![(100, 2.0), (200, 3.0)],
                };
                f(offset, &mut t);
                t
            })
            .collect();
        Capture {
            origin: Origin::Snapshot,
            press_at_ms: PRESS,
            forward_only: false,
            ticks,
            identities: vec![
                Identity {
                    parent_pid: None,
                    bundle: None,
                    pid: 100,
                    start_time: 1,
                    name: "builder".into(),
                    workload_id: "build".into(),
                    label: "Build Tool".into(),
                    category: "app".into(),
                },
                Identity {
                    parent_pid: None,
                    bundle: None,
                    pid: 200,
                    start_time: 1,
                    name: "Editor".into(),
                    workload_id: "editor".into(),
                    label: "Editor".into(),
                    category: "app".into(),
                },
            ],
            victim: Some(Victim {
                pid: 200,
                name: Some("Editor".into()),
            }),
            logical_cpus: 8,
        }
    }

    pub fn snapshot() -> Snapshot {
        let workload = |id: &str, label: &str| Workload {
            id: id.into(),
            label: label.into(),
            category: "app".into(),
            cpu_percent: 1.0,
            memory_bytes: 1 << 30,
            history: (0..24)
                .map(|i| ResourcePoint {
                    time: PRESS / 1000 - 115 + i * 5,
                    cpu_percent: 0.3,
                    memory_bytes: 1 << 30,
                    unresponsive: None,
                })
                .collect(),
            ..Default::default()
        };
        let process = |pid: u32, id: &str, name: &str| ProcessRecord {
            pid,
            start_time: 1,
            name: name.into(),
            workload_id: id.into(),
            cpu_percent: 1.0,
            memory_bytes: 1 << 30,
            actions: vec!["lower_priority".into(), "terminate".into()],
            ..Default::default()
        };
        Snapshot {
            source: "live".into(),
            captured_at: PRESS / 1000 + 10,
            sample_ready: true,
            logical_cpus: 8,
            memory_total_bytes: 16 << 30,
            memory_available_bytes: 8 << 30,
            workloads: vec![
                workload("build", "Build Tool"),
                workload("editor", "Editor"),
            ],
            processes: vec![
                process(100, "build", "builder"),
                process(200, "editor", "Editor"),
            ],
            ..Default::default()
        }
    }

    fn kinds(report: &Report) -> Vec<(FindingKind, Strength)> {
        report
            .findings
            .iter()
            .map(|f| (f.kind, f.strength))
            .collect()
    }

    #[test]
    fn process_spike_blames_the_culprit_and_offers_lower_priority_only() {
        let c = capture(|o, t| {
            if (-5..=2).contains(&o) {
                t.processes[0].1 = 160.0;
                t.cpu_percent = 30.0;
            }
        });
        let r = analyze(&c, &snapshot());
        assert_eq!(r.findings[0].kind, FindingKind::ProcessSpike);
        assert_eq!(r.findings[0].strength, Strength::Strong);
        assert_eq!(r.culprit.as_ref().unwrap().label, "Build Tool");
        assert_eq!(r.victim.as_ref().unwrap().label, "Editor");
        let action = r.in_app.unwrap();
        assert_eq!(
            (action.action.as_str(), action.process.pid),
            ("lower_priority", 100)
        );
        assert!(r.happened.contains("160%"));
    }

    #[test]
    fn single_tick_or_small_rises_are_not_strong() {
        let c = capture(|o, t| {
            if o == 0 {
                t.processes[0].1 = 60.0;
            }
            if o == 3 {
                t.processes[0].1 = 20.0; // below a quarter core
            }
        });
        let r = analyze(&c, &snapshot());
        assert_eq!(
            kinds(&r),
            vec![(FindingKind::ProcessSpike, Strength::Possible)]
        );
    }

    #[test]
    fn pegged_core_is_found_even_when_total_cpu_is_low() {
        let c = capture(|o, t| {
            if (-4..=0).contains(&o) {
                t.cores[3] = 100;
                t.cpu_percent = 14.0;
                t.processes[0].1 = 99.0;
            }
        });
        let r = analyze(&c, &snapshot());
        let pegged = r
            .findings
            .iter()
            .find(|f| f.kind == FindingKind::CorePegged)
            .unwrap();
        assert_eq!(pegged.strength, Strength::Strong);
        assert!(pegged.observed[0].contains("5 s"));
        assert_eq!(pegged.label.as_deref(), Some("Build Tool"));
        // Without a watched process, the saturated core is still reported, more cautiously.
        let c = capture(|o, t| {
            if (-4..=0).contains(&o) {
                t.cores[3] = 100;
            }
        });
        assert_eq!(
            kinds(&analyze(&c, &snapshot())),
            vec![(FindingKind::CorePegged, Strength::Possible)]
        );
    }

    #[test]
    fn memory_thrash_never_suggests_priority() {
        let mut s = snapshot();
        s.workloads[0].history.last_mut().unwrap().memory_bytes = 3 << 30;
        let c = capture(|o, t| {
            if (-3..=3).contains(&o) {
                t.swap_in_bytes = Some(80 << 20);
                t.decompressed_bytes = Some(40 << 20);
                t.memory_pressure = Some(1);
            }
        });
        let r = analyze(&c, &s);
        assert_eq!(r.findings[0].kind, FindingKind::MemoryThrash);
        assert_eq!(r.findings[0].strength, Strength::Strong);
        assert!(r.findings[0]
            .observed
            .iter()
            .any(|o| o.contains("normal to warning")));
        let action = r.in_app.unwrap();
        assert_eq!(action.action, "terminate");
        assert_eq!(action.process.pid, 100);
        // A burst alone is possible, and a possible memory finding offers no in-app action.
        let c = capture(|o, t| {
            if o == 0 {
                t.swap_out_bytes = Some(64 << 20);
            }
        });
        let r = analyze(&c, &snapshot());
        assert_eq!(
            kinds(&r),
            vec![(FindingKind::MemoryThrash, Strength::Possible)]
        );
        assert!(r.in_app.is_none());
    }

    #[test]
    fn thermal_escalation_is_reported() {
        let c = capture(|o, t| {
            if o >= -5 {
                t.thermal = Some(2);
            }
        });
        let r = analyze(&c, &snapshot());
        assert_eq!(
            kinds(&r),
            vec![(FindingKind::ThermalEscalation, Strength::Strong)]
        );
        assert!(r.findings[0].observed[0].contains("nominal to serious"));
        assert!(r.in_app.is_none());
    }

    #[test]
    fn lag_without_cause_is_unexplained_and_nothing_found_mentions_gpu() {
        let c = capture(|o, t| {
            if o == -1 {
                t.lag_ms = 180.0;
            }
        });
        let r = analyze(&c, &snapshot());
        assert_eq!(
            kinds(&r),
            vec![(FindingKind::SchedulerLag, Strength::Unexplained)]
        );
        assert!(r.findings[0].inferred.contains("GPU"));
        let r = analyze(&capture(|_, _| {}), &snapshot());
        assert_eq!(
            kinds(&r),
            vec![(FindingKind::NothingFound, Strength::Unexplained)]
        );
        assert!(r.findings[0].inferred.contains("GPU, display"));
        assert!(r.in_app.is_none() && r.culprit.is_none());
    }

    #[test]
    fn lag_becomes_supporting_evidence_for_a_visible_cause() {
        let c = capture(|o, t| {
            if (-3..=0).contains(&o) {
                t.processes[0].1 = 150.0;
                t.lag_ms = 60.0;
            }
        });
        let r = analyze(&c, &snapshot());
        assert_eq!(r.findings.len(), 1);
        assert!(r.findings[0]
            .observed
            .iter()
            .any(|o| o.contains("60 ms late")));
    }

    #[test]
    fn a_busy_victim_is_limited_by_its_own_work_and_never_targeted() {
        let c = capture(|o, t| {
            if (-3..=0).contains(&o) {
                t.processes[1].1 = 180.0;
            }
        });
        let r = analyze(&c, &snapshot());
        assert_eq!(r.findings[0].kind, FindingKind::VictimBusy);
        assert!(r.findings[0].inferred.contains("would not help"));
        assert!(r.culprit.is_none());
        assert!(r.in_app.is_none());
        assert_eq!(r.implicated().as_deref(), Some("Editor"));
        // With both busy, the other app is the culprit and the victim is left alone.
        let c = capture(|o, t| {
            if (-3..=0).contains(&o) {
                t.processes[1].1 = 180.0;
                t.processes[0].1 = 150.0;
            }
        });
        let r = analyze(&c, &snapshot());
        assert_eq!(r.culprit.unwrap().label, "Build Tool");
        assert_eq!(r.in_app.unwrap().process.pid, 100);
    }

    #[test]
    fn five_second_history_catches_workloads_outside_the_watched_pids() {
        let mut s = snapshot();
        s.workloads.push(Workload {
            id: "other".into(),
            label: "Other".into(),
            history: (0..24)
                .map(|i| ResourcePoint {
                    time: PRESS / 1000 - 115 + i * 5,
                    cpu_percent: if i >= 21 { 20.0 } else { 0.2 },
                    memory_bytes: 1,
                    unresponsive: None,
                })
                .collect(),
            ..Default::default()
        });
        let r = analyze(&capture(|_, _| {}), &s);
        assert_eq!(
            kinds(&r),
            vec![(FindingKind::ProcessSpike, Strength::Possible)]
        );
        assert_eq!(r.culprit.unwrap().label, "Other");
    }

    #[test]
    fn spike_detector_fires_once_on_the_rising_edge_of_sustained_lag() {
        let ticks = |lags: &[f32]| -> Vec<Tick> {
            lags.iter()
                .enumerate()
                .map(|(i, l)| Tick {
                    at_ms: PRESS + i as u64 * 1000,
                    lag_ms: *l,
                    cores: vec![10],
                    ..Default::default()
                })
                .collect()
        };
        assert_eq!(spike(&ticks(&[50.0, 50.0, 50.0])), Some(PRESS));
        assert_eq!(spike(&ticks(&[1.0, 50.0, 50.0, 50.0])), Some(PRESS + 1000));
        assert_eq!(spike(&ticks(&[1.0, 50.0, 50.0, 50.0, 50.0])), None);
        assert_eq!(spike(&ticks(&[1.0, 50.0, 1.0, 50.0])), None);
        assert_eq!(spike(&ticks(&[1.0, 1.0, 1.0])), None);
        assert_eq!(spike(&ticks(&[50.0, 50.0])), None);
        // A pegged core without lag is ordinary work, not a hitch.
        let mut busy = ticks(&[1.0, 1.0, 1.0, 1.0]);
        busy.iter_mut().for_each(|t| t.cores = vec![100]);
        assert_eq!(spike(&busy), None);
        busy[2].lag_ms = 25.0;
        busy[3].lag_ms = 25.0;
        assert_eq!(spike(&busy[1..]), Some(PRESS + 1000));
    }

    #[test]
    fn tally_is_capped_clearable_and_tracks_recent_recurrence() {
        let mut tally = Tally::default();
        for n in 0..80 {
            tally.record(Some(&format!("App {n}")), n);
        }
        assert_eq!(tally.entries.len(), TALLY_LIMIT);
        assert_eq!(tally.recent.len(), RECENT);
        for at in 100..103 {
            tally.record(Some("Build Tool"), at);
        }
        tally.record(None, 104);
        assert_eq!(tally.recurrence("Build Tool"), 3);
        assert_eq!(tally.entries[0].label, "Build Tool");
        assert_eq!(tally.entries[0].count, 3);
        let json = serde_json::to_string(&tally).unwrap();
        assert!(!json.contains('/'), "names and counts only");
        tally.clear();
        assert!(tally.entries.is_empty() && tally.recent.is_empty());
    }

    #[test]
    fn recurrence_offers_permission_only_for_a_repeated_priority_culprit() {
        let c = capture(|o, t| {
            if (-3..=0).contains(&o) {
                t.processes[0].1 = 150.0;
            }
        });
        let mut tally = Tally::default();
        let mut r = analyze(&c, &snapshot());
        for _ in 0..2 {
            tally.record(r.implicated().as_deref(), 1);
        }
        r.apply_recurrence(&tally);
        assert!(!r.offer_permission);
        tally.record(r.implicated().as_deref(), 2);
        r.apply_recurrence(&tally);
        assert!(r.offer_permission);
        assert!(r.if_again.contains("3 of your last 3"));
    }

    /// The editor (pid 200) is frontmost; a shell inside it runs `pid` from `parent`.
    fn editor_with_terminal(snapshot: &mut Snapshot) {
        let editor = &mut snapshot.processes[1];
        editor.executable = Some("/Applications/Editor.app/Contents/MacOS/Editor".into());
        snapshot.processes.push(ProcessRecord {
            pid: 210,
            start_time: 2,
            parent_pid: Some(200),
            name: "zsh".into(),
            workload_id: "editor".into(),
            executable: Some("/bin/zsh".into()),
            ..Default::default()
        });
    }

    #[test]
    fn a_new_hog_in_the_frontmost_terminal_is_named_not_folded_into_the_victim() {
        // perl started after the last full sample: only the per-second scan saw it.
        let mut c = capture(|o, t| {
            if (-6..=-1).contains(&o) {
                t.processes.push((300, 100.0));
            }
        });
        c.identities.push(Identity {
            pid: 300,
            start_time: 9,
            name: "perl".into(),
            workload_id: "p-300-9".into(),
            label: "perl".into(),
            category: "other".into(),
            parent_pid: Some(210),
            bundle: None,
        });
        let mut s = snapshot();
        editor_with_terminal(&mut s);
        let r = analyze(&c, &s);
        assert_eq!(r.culprit.as_ref().unwrap().label, "perl in Editor");
        assert!(r.findings.iter().all(|f| f.kind != FindingKind::VictimBusy));
        assert_eq!(r.victim.as_ref().unwrap().label, "Editor");
    }

    #[test]
    fn an_agent_session_inside_the_editor_groups_its_children_and_hides_the_folder() {
        // The collector folded the session into the editor; its node child is the busy one.
        let mut c = capture(|o, t| {
            if (-8..=2).contains(&o) {
                t.processes.push((310, 20.0));
                t.processes.push((311, 90.0));
            }
        });
        for (pid, parent, name) in [(310, 210, "claude"), (311, 310, "node")] {
            c.identities.push(Identity {
                pid,
                start_time: 3,
                name: name.into(),
                workload_id: "editor".into(),
                label: "Editor".into(),
                category: "app".into(),
                parent_pid: Some(parent),
                bundle: None,
            });
        }
        let mut s = snapshot();
        editor_with_terminal(&mut s);
        for (pid, parent, name) in [(310, 210, "claude"), (311, 310, "node")] {
            s.processes.push(ProcessRecord {
                pid,
                start_time: 3,
                parent_pid: Some(parent),
                name: name.into(),
                workload_id: "editor".into(),
                working_directory: Some("/w/secret-repo".into()),
                actions: if name == "node" {
                    vec!["lower_priority".into(), "terminate".into()]
                } else {
                    vec![]
                },
                ..Default::default()
            });
        }
        let r = analyze(&c, &s);
        let culprit = r.culprit.as_ref().unwrap();
        assert_eq!(culprit.label, "claude (secret-repo) in Editor");
        assert!(r.findings[0]
            .observed
            .iter()
            .any(|o| o == "Busiest process in it: node."));
        // The suggestion targets the session's own busy child, never the editor app.
        let action = r.in_app.as_ref().unwrap();
        assert_eq!(action.process.pid, 311);
        let context = r.model_context().to_string();
        assert!(context.contains("claude in Editor"));
        assert!(!context.contains("secret-repo"));
    }

    #[test]
    fn model_context_is_bounded_and_aliases_project_labels() {
        let mut c = capture(|o, t| {
            if (-3..=0).contains(&o) {
                t.processes[0].1 = 150.0;
            }
        });
        c.identities[0].category = "project".into();
        c.identities[0].label = "secret-project".into();
        let mut s = snapshot();
        s.workloads[0].category = "project".into();
        s.workloads[0].label = "secret-project".into();
        let r = analyze(&c, &s);
        let context = r.model_context().to_string();
        assert!(!context.contains("secret-project"));
        assert!(!context.contains("\"pid\""));
        assert!(context.len() < 8000);
        assert!(r.model_context()["timeline_5s"].as_array().unwrap().len() <= 18);
    }

    #[test]
    fn forward_only_captures_use_history_for_baseline() {
        let mut c = capture(|o, t| {
            if o >= 0 {
                t.processes[0].1 = 150.0;
            }
        });
        c.forward_only = true;
        c.ticks.retain(|t| t.at_ms >= PRESS);
        let r = analyze(&c, &snapshot());
        assert_eq!(r.findings[0].kind, FindingKind::ProcessSpike);
        assert_eq!(r.window, [0, 15]);
        assert!(r.if_again.contains("Snapshot-only"));
    }
}
