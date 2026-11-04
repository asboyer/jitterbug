//! One agent conversation up close: what its process tree uses now and over the last hour, its
//! heaviest processes, and its recent tool calls with the CPU and memory measured while each
//! ran. History is kept in memory only, from the moment Jitterbug starts watching; tool calls
//! come from the agent's own local log. Nothing here is sent to a model.
use crate::{
    activity::{self, ToolCall},
    agents::{self, Tree},
    claude_sessions,
    turns::{Finished, State},
    types::{now, ProcessRecord, Snapshot},
};
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
};

/// One hour at the collector's cadence, and a bound on how many conversations are tracked.
const POINTS: usize = 1800;
const SESSIONS: usize = 48;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Point {
    pub time: u64,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}

static HISTORY: Mutex<Option<HashMap<String, VecDeque<Point>>>> = Mutex::new(None);

fn session_id(p: &ProcessRecord) -> String {
    format!("{}-{}", p.pid, p.start_time)
}

/// Called with every sample: add each agent tree's usage and forget conversations that ended.
pub fn record(snapshot: &Snapshot) {
    let tree = Tree::new(snapshot);
    let roots = agents::roots(snapshot, &tree);
    let Ok(mut guard) = HISTORY.lock() else {
        return;
    };
    let history = guard.get_or_insert_with(HashMap::new);
    let live: Vec<String> = roots.iter().map(|p| session_id(p)).collect();
    history.retain(|id, _| live.contains(id));
    for root in roots {
        let (cpu, memory) = tree.usage(root);
        let id = session_id(root);
        if !history.contains_key(&id) && history.len() >= SESSIONS {
            continue;
        }
        let points = history.entry(id).or_default();
        points.push_back(Point {
            time: snapshot.captured_at,
            cpu_percent: cpu,
            memory_bytes: memory,
        });
        while points.len() > POINTS {
            points.pop_front();
        }
    }
}

fn history(id: &str) -> Vec<Point> {
    HISTORY
        .lock()
        .ok()
        .and_then(|h| h.as_ref().and_then(|h| h.get(id).cloned()))
        .map(Vec::from)
        .unwrap_or_default()
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}

/// A tool call with what the conversation's processes used while it ran. The figures cover the
/// whole conversation, so overlapping calls (parallel subagents) share them.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Call {
    #[serde(flatten)]
    pub call: ToolCall,
    pub seconds: u64,
    /// Samples Jitterbug took while the call ran; zero when it was not watching then.
    pub samples: usize,
    pub average_cpu_percent: f32,
    pub peak_cpu_percent: f32,
    pub peak_memory_bytes: u64,
    /// CPU time in seconds of one core: the measure the "heaviest" list is ranked by.
    pub cpu_seconds: f32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Detail {
    pub session: agents::SessionView,
    pub started_at: Option<u64>,
    pub process_count: usize,
    pub tracked_since: Option<u64>,
    pub peak_cpu_percent: f32,
    pub peak_memory_bytes: u64,
    pub series: Vec<Point>,
    pub processes: Vec<Process>,
    /// Most recent first.
    pub calls: Vec<Call>,
    /// Ranked by CPU time; only calls Jitterbug measured.
    pub heaviest: Vec<Call>,
    /// Heavy commands this conversation ran through the turns line, newest first.
    pub heavy: Vec<Finished>,
    /// Logs this harness keeps that Jitterbug can read tool calls from.
    pub calls_available: bool,
}

/// CPU samples are percentages of the whole computer; convert to one-core seconds.
fn measure(call: ToolCall, series: &[Point], cores: f32, at: u64) -> Call {
    let end = call.ended_at.unwrap_or(at);
    let inside: Vec<&Point> = series
        .iter()
        .filter(|p| p.time >= call.started_at && p.time <= end)
        .collect();
    let mut cpu_seconds = 0.0;
    for pair in series.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let from = a.time.max(call.started_at);
        let to = b.time.min(end);
        if to > from {
            cpu_seconds += b.cpu_percent / 100.0 * cores * (to - from) as f32;
        }
    }
    Call {
        seconds: end.saturating_sub(call.started_at),
        samples: inside.len(),
        average_cpu_percent: if inside.is_empty() {
            0.0
        } else {
            inside.iter().map(|p| p.cpu_percent).sum::<f32>() / inside.len() as f32
        },
        peak_cpu_percent: inside.iter().map(|p| p.cpu_percent).fold(0.0, f32::max),
        peak_memory_bytes: inside.iter().map(|p| p.memory_bytes).max().unwrap_or(0),
        cpu_seconds,
        call,
    }
}

fn assemble(
    session: agents::SessionView,
    series: Vec<Point>,
    calls: Vec<ToolCall>,
    cores: f32,
    at: u64,
) -> Detail {
    let measured: Vec<Call> = calls
        .into_iter()
        .rev()
        .take(200)
        .map(|c| measure(c, &series, cores, at))
        .collect();
    let mut heaviest: Vec<Call> = measured
        .iter()
        .filter(|c| c.samples > 0 && c.cpu_seconds >= 1.0)
        .cloned()
        .collect();
    heaviest.sort_by(|a, b| b.cpu_seconds.total_cmp(&a.cpu_seconds));
    heaviest.truncate(5);
    let hour = at.saturating_sub(3600);
    Detail {
        tracked_since: series.first().map(|p| p.time),
        peak_cpu_percent: series.iter().map(|p| p.cpu_percent).fold(0.0, f32::max),
        peak_memory_bytes: series.iter().map(|p| p.memory_bytes).max().unwrap_or(0),
        series: series.into_iter().filter(|p| p.time >= hour).collect(),
        calls: measured.into_iter().take(40).collect(),
        heaviest,
        session,
        ..Default::default()
    }
}

pub fn detail(
    snapshot: &Snapshot,
    view: &agents::View,
    state: &State,
    id: &str,
) -> Result<Detail, String> {
    let session = view
        .sessions
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .ok_or("This conversation is no longer running.")?;
    let tree = Tree::new(snapshot);
    let root = agents::roots(snapshot, &tree)
        .into_iter()
        .find(|p| session_id(p) == id);
    let claude = root
        .filter(|p| agents::harness_of(p) == Some("claude"))
        .and_then(|p| claude_sessions::lookup(p.pid, p.start_time));
    let calls = claude
        .as_ref()
        .and_then(|c| c.session_id.as_deref())
        .and_then(activity::claude_transcript)
        .map(|t| activity::claude_calls(&t))
        .unwrap_or_default();
    let cores = std::thread::available_parallelism()
        .map(|n| n.get() as f32)
        .unwrap_or(1.0);
    let mut detail = assemble(session, history(id), calls, cores, now());
    detail.calls_available = claude.is_some();
    if let Some(root) = root {
        detail.started_at = Some(root.start_time);
        let mut members = vec![root];
        let mut i = 0;
        while i < members.len() && members.len() < 4096 {
            if let Some(children) = tree.children.get(&members[i].pid) {
                members.extend(children.iter().copied());
            }
            i += 1;
        }
        detail.process_count = members.len();
        members.sort_by(|a, b| {
            (b.cpu_percent, b.memory_bytes)
                .partial_cmp(&(a.cpu_percent, a.memory_bytes))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        detail.processes = members
            .into_iter()
            .take(8)
            .map(|p| Process {
                pid: p.pid,
                name: p.name.clone(),
                cpu_percent: p.cpu_percent,
                memory_bytes: p.memory_bytes,
            })
            .collect();
    }
    if let Some(key) = claude.and_then(|c| c.session_id) {
        detail.heavy = state
            .history
            .iter()
            .rev()
            .filter(|f| f.session == key)
            .take(10)
            .cloned()
            .collect();
    }
    Ok(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(time: u64, cpu: f32) -> Point {
        Point {
            time,
            cpu_percent: cpu,
            memory_bytes: (cpu as u64) << 20,
        }
    }

    #[test]
    fn calls_are_charged_only_for_the_time_they_ran_and_ranked_by_cpu_time() {
        // 10 cores; a build at 50% of the machine for 60 s = 300 core-seconds.
        let series: Vec<Point> = (0..=12)
            .map(|i| point(1000 + i * 10, if (2..=8).contains(&i) { 50.0 } else { 1.0 }))
            .collect();
        let build = ToolCall {
            label: "Running cargo build".into(),
            tool: "Bash".into(),
            started_at: 1020,
            ended_at: Some(1080),
        };
        let read = ToolCall {
            label: "Reading lib.rs".into(),
            tool: "Read".into(),
            started_at: 1100,
            ended_at: Some(1101),
        };
        let old = ToolCall {
            label: "Before Jitterbug".into(),
            tool: "Bash".into(),
            started_at: 10,
            ended_at: Some(20),
        };
        let detail = assemble(
            Default::default(),
            series,
            vec![old, build, read],
            10.0,
            1200,
        );
        assert_eq!(detail.calls[0].call.label, "Reading lib.rs");
        let heavy = &detail.heaviest[0];
        assert_eq!(heavy.call.label, "Running cargo build");
        assert!(
            (heavy.cpu_seconds - 300.0).abs() < 1.0,
            "{}",
            heavy.cpu_seconds
        );
        assert_eq!(heavy.peak_cpu_percent, 50.0);
        assert_eq!(heavy.seconds, 60);
        assert!(detail
            .heaviest
            .iter()
            .all(|c| c.call.label != "Before Jitterbug"));
        assert_eq!(detail.peak_cpu_percent, 50.0);
    }
}
