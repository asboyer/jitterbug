//! Chromium tabs through the browser's own DevTools endpoint (local loopback only).
//!
//! The protocol lists tabs but never says which renderer process hosts a tab. Each tab does
//! report its renderer's cumulative CPU time (`Performance.getMetrics` → `ProcessTime`), and
//! `SystemInfo.getProcessInfo` reports the same counter per PID. Reading both twice, bracketed in
//! time, identifies the tab's PID; its memory then comes from the normal collector inventory.
use super::{host, Inside, Remainder, Unit};
use crate::types::*;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    net::TcpStream,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
use tungstenite::{Message, WebSocket};

pub const BROWSERS: &[&str] = &[
    "google chrome",
    "google chrome beta",
    "google chrome dev",
    "google chrome canary",
    "chromium",
    "brave browser",
    "microsoft edge",
    "vivaldi",
];

const SOURCE: &str = "Browser DevTools";
const WINDOW: Duration = Duration::from_secs(1);
/// Counter granularity: one tab's reading must fall inside its process's bracket.
const EPSILON: f64 = 0.000_2;
const TAB_LIMIT: usize = 100;

/// Kept open: a browser that asks before allowing a debugging client asks once per connection.
static CONNECTIONS: Mutex<Vec<Cdp>> = Mutex::new(Vec::new());

struct Cdp {
    endpoint: String,
    socket: WebSocket<TcpStream>,
    next: u64,
}

impl Cdp {
    fn open(endpoint: &str) -> Result<Self, String> {
        let address = endpoint
            .strip_prefix("ws://")
            .and_then(|rest| rest.split('/').next())
            .ok_or("Unsupported DevTools endpoint.")?;
        if !address.starts_with("127.0.0.1:") && !address.starts_with("localhost:") {
            return Err("DevTools endpoints must be on this computer.".into());
        }
        let addr = address
            .replace("localhost", "127.0.0.1")
            .parse()
            .map_err(|_| "Invalid DevTools address.")?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))
            .map_err(|_| "The browser is not accepting debugging connections.")?;
        // The browser may wait for the person to allow this connection.
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .map_err(|e| e.to_string())?;
        let (socket, _) = tungstenite::client(endpoint, stream)
            .map_err(|_| "The browser declined the debugging connection.")?;
        socket
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            endpoint: endpoint.into(),
            socket,
            next: 0,
        })
    }

    fn call(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, String> {
        self.next += 1;
        let mut message = json!({"id": self.next, "method": method, "params": params});
        if let Some(session) = session {
            message["sessionId"] = session.into();
        }
        self.socket
            .send(Message::text(message.to_string()))
            .map_err(|_| "The browser closed the debugging connection.")?;
        loop {
            let text = match self.socket.read() {
                Ok(Message::Text(text)) => text,
                Ok(Message::Close(_)) | Err(_) => {
                    return Err("The browser closed the debugging connection.".into())
                }
                Ok(_) => continue,
            };
            // Events and replies to other sessions are skipped.
            let Ok(reply) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if reply.get("id").and_then(Value::as_u64) != Some(self.next) {
                continue;
            }
            if let Some(error) = reply.get("error") {
                return Err(error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("DevTools request failed.")
                    .into());
            }
            return Ok(reply.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

fn profile_directories() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("JITTERBUG_BROWSER_PROFILES")
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default();
    #[cfg(target_os = "macos")]
    let (base, names) = (
        dirs::data_dir(),
        [
            "Google/Chrome",
            "Google/Chrome Beta",
            "Google/Chrome Dev",
            "Google/Chrome Canary",
            "Chromium",
            "BraveSoftware/Brave-Browser",
            "Microsoft Edge",
            "Vivaldi",
        ],
    );
    #[cfg(target_os = "windows")]
    let (base, names) = (
        dirs::data_local_dir(),
        [
            "Google/Chrome/User Data",
            "Google/Chrome Beta/User Data",
            "Google/Chrome Dev/User Data",
            "Google/Chrome SxS/User Data",
            "Chromium/User Data",
            "BraveSoftware/Brave-Browser/User Data",
            "Microsoft/Edge/User Data",
            "Vivaldi/User Data",
        ],
    );
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let (base, names) = (
        dirs::config_dir(),
        [
            "google-chrome",
            "google-chrome-beta",
            "google-chrome-unstable",
            "chromium",
            "BraveSoftware/Brave-Browser",
            "microsoft-edge",
            "vivaldi",
            "chromium-canary",
        ],
    );
    if let Some(base) = base {
        dirs.extend(names.iter().map(|n| base.join(n)));
    }
    dirs
}

/// `DevToolsActivePort` holds the port and browser path while remote debugging is enabled.
fn endpoints() -> Vec<String> {
    profile_directories()
        .into_iter()
        .filter_map(|dir| std::fs::read_to_string(dir.join("DevToolsActivePort")).ok())
        .filter_map(|text| {
            let mut lines = text.lines();
            let port: u16 = lines.next()?.trim().parse().ok()?;
            let path = lines.next()?.trim();
            path.starts_with("/devtools/browser/")
                .then(|| format!("ws://127.0.0.1:{port}{path}"))
        })
        .collect()
}

/// Run `f` on every reachable browser, reusing open connections.
fn with_browsers<T>(mut f: impl FnMut(&mut Cdp) -> Result<T, String>) -> (Vec<T>, Vec<String>) {
    let mut open = CONNECTIONS.lock().unwrap_or_else(|e| e.into_inner());
    let wanted = endpoints();
    open.retain(|c| wanted.contains(&c.endpoint));
    let (mut results, mut errors) = (Vec::new(), Vec::new());
    for endpoint in wanted {
        let index = match open.iter().position(|c| c.endpoint == endpoint) {
            Some(i) => i,
            None => match Cdp::open(&endpoint) {
                Ok(c) => {
                    open.push(c);
                    open.len() - 1
                }
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            },
        };
        match f(&mut open[index]) {
            Ok(r) => results.push(r),
            Err(e) => {
                // A stale socket is dropped; the next request reconnects.
                open.remove(index);
                errors.push(e);
            }
        }
    }
    (results, errors)
}

#[derive(Clone, Default)]
struct Processes {
    at: Option<Instant>,
    cpu: HashMap<u32, f64>,
    kind: HashMap<u32, String>,
}

fn processes(cdp: &mut Cdp) -> Option<Processes> {
    let reply = cdp
        .call("SystemInfo.getProcessInfo", json!({}), None)
        .ok()?;
    let mut out = Processes {
        at: Some(Instant::now()),
        ..Default::default()
    };
    for p in reply.get("processInfo")?.as_array()? {
        let pid = p.get("id")?.as_u64()? as u32;
        out.cpu.insert(pid, p.get("cpuTime")?.as_f64()?);
        out.kind
            .insert(pid, p.get("type")?.as_str().unwrap_or("").to_string());
    }
    Some(out)
}

#[derive(Clone, Copy, Default)]
struct Metrics {
    process_time: Option<f64>,
    task_seconds: f64,
    heap_bytes: f64,
}

fn metrics(cdp: &mut Cdp, session: &str) -> Metrics {
    let Ok(reply) = cdp.call("Performance.getMetrics", json!({}), Some(session)) else {
        return Metrics::default();
    };
    let mut out = Metrics::default();
    for m in reply
        .get("metrics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let value = m.get("value").and_then(Value::as_f64).unwrap_or(0.0);
        match m.get("name").and_then(Value::as_str) {
            Some("ProcessTime") => out.process_time = Some(value),
            Some("TaskDuration") => out.task_seconds = value,
            Some("JSHeapUsedSize") => out.heap_bytes = value,
            _ => {}
        }
    }
    out
}

/// PIDs whose cumulative CPU time brackets both of a tab's readings. Best match first.
fn candidates(readings: [f64; 2], brackets: [(&Processes, &Processes); 2]) -> Vec<u32> {
    let mut found: Vec<(f64, u32)> = brackets[0]
        .0
        .cpu
        .keys()
        .filter_map(|pid| {
            let mut distance = 0.0;
            for (reading, (before, after)) in readings.iter().zip(brackets) {
                let (lo, hi) = (*before.cpu.get(pid)?, *after.cpu.get(pid)?);
                if *reading < lo - EPSILON || *reading > hi + EPSILON {
                    return None;
                }
                distance += reading - lo;
            }
            Some((distance, *pid))
        })
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().map(|(_, pid)| pid).collect()
}

struct Tab {
    id: String,
    title: String,
    url: String,
    session: Option<String>,
    first: Metrics,
    second: Metrics,
    pid: Option<u32>,
    ambiguous: bool,
}

struct Measurement {
    tabs: Vec<Tab>,
    before: Option<Processes>,
    after: Option<Processes>,
}

fn measure(cdp: &mut Cdp) -> Result<Measurement, String> {
    let a = processes(cdp);
    let targets = cdp.call("Target.getTargets", json!({}), None)?;
    let mut tabs: Vec<Tab> = targets
        .get("targetInfos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|t| t.get("type").and_then(Value::as_str) == Some("page"))
        .take(TAB_LIMIT)
        .map(|t| Tab {
            id: t["targetId"].as_str().unwrap_or_default().into(),
            title: t["title"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(120)
                .collect(),
            url: t["url"].as_str().unwrap_or_default().into(),
            session: None,
            first: Metrics::default(),
            second: Metrics::default(),
            pid: None,
            ambiguous: false,
        })
        .collect();
    let started = Instant::now();
    for tab in &mut tabs {
        let Ok(reply) = cdp.call(
            "Target.attachToTarget",
            json!({"targetId": tab.id, "flatten": true}),
            None,
        ) else {
            continue;
        };
        let Some(session) = reply.get("sessionId").and_then(Value::as_str) else {
            continue;
        };
        let _ = cdp.call("Performance.enable", json!({}), Some(session));
        tab.first = metrics(cdp, session);
        tab.session = Some(session.into());
    }
    let b = processes(cdp);
    std::thread::sleep(WINDOW.saturating_sub(started.elapsed()));
    let c = processes(cdp);
    for tab in &mut tabs {
        if let Some(session) = &tab.session {
            tab.second = metrics(cdp, session);
        }
    }
    let d = processes(cdp);
    for tab in &mut tabs {
        if let Some(session) = tab.session.take() {
            let _ = cdp.call(
                "Target.detachFromTarget",
                json!({"sessionId": session}),
                None,
            );
        }
    }
    if let (Some(a), Some(b), Some(c), Some(d)) = (&a, &b, &c, &d) {
        for tab in &mut tabs {
            if let (Some(x), Some(y)) = (tab.first.process_time, tab.second.process_time) {
                let found = candidates([x, y], [(a, b), (c, d)]);
                tab.pid = found.first().copied();
                tab.ambiguous = found.len() > 1;
            }
        }
    }
    Ok(Measurement {
        tabs,
        before: a,
        after: d,
    })
}

/// Fraction of one core between two process readings.
fn core_share(pid: u32, before: &Processes, after: &Processes) -> Option<f64> {
    let seconds = after.at?.duration_since(before.at?).as_secs_f64();
    let used = after.cpu.get(&pid)? - before.cpu.get(&pid)?;
    (seconds > 0.0).then(|| (used / seconds).max(0.0))
}

fn split(weights: &[f64]) -> Vec<f64> {
    let total: f64 = weights.iter().sum();
    if total > 0.0 {
        weights.iter().map(|w| w / total).collect()
    } else {
        vec![1.0 / weights.len().max(1) as f64; weights.len()]
    }
}

pub fn inspect(snapshot: &Snapshot, workload_id: &str, members: &[&ProcessRecord]) -> Inside {
    if endpoints().is_empty() {
        let mut inside = Inside::status(
            workload_id,
            SOURCE,
            "setup",
            "Allow Jitterbug to read this browser's tabs. This is a one-time browser setting.",
        );
        inside.setup = vec![
            "Open chrome://inspect/#remote-debugging in the browser (Edge: edge://inspect, Brave: brave://inspect).".into(),
            "Turn on “Allow remote debugging for this browser instance”.".into(),
            "Come back and choose Show tabs. The browser asks once to allow the connection.".into(),
            "Only programs on this computer can connect. Turn the setting off to revoke access.".into(),
        ];
        return inside;
    }
    let (measured, errors) = with_browsers(measure);
    if measured.is_empty() {
        let mut inside = Inside::status(
            workload_id,
            SOURCE,
            "setup",
            errors
                .first()
                .cloned()
                .unwrap_or_else(|| "The browser could not be reached.".into()),
        );
        inside.setup = vec![
            "Make sure the browser is running and remote debugging is still allowed at chrome://inspect/#remote-debugging.".into(),
            "If the browser asked to allow the connection, choose Allow and try again.".into(),
        ];
        return inside;
    }
    let by_pid: HashMap<u32, &ProcessRecord> =
        snapshot.processes.iter().map(|p| (p.pid, p)).collect();
    let member_pids: std::collections::HashSet<u32> = members.iter().map(|p| p.pid).collect();
    let cores = snapshot.logical_cpus.max(1) as f64;
    let mut units = Vec::new();
    let mut attributed: std::collections::HashSet<u32> = Default::default();
    let mut remainder: HashMap<&'static str, Remainder> = HashMap::new();
    let mut verified = 0;
    for m in &measured {
        // The endpoint must belong to this workload, not another browser or profile.
        let browser = m.before.as_ref().and_then(|p| {
            p.kind
                .iter()
                .find(|(_, k)| k.as_str() == "browser")
                .map(|(pid, _)| *pid)
        });
        if browser.is_some_and(|pid| !member_pids.contains(&pid)) {
            continue;
        }
        verified += 1;
        let mut groups: HashMap<Option<u32>, Vec<&Tab>> = HashMap::new();
        for tab in &m.tabs {
            groups.entry(tab.pid).or_default().push(tab);
        }
        for (pid, tabs) in &groups {
            let process = pid.and_then(|pid| by_pid.get(&pid));
            let cpu = match (pid, &m.before, &m.after) {
                (Some(pid), Some(a), Some(d)) => core_share(*pid, a, d).map(|s| s * 100.0 / cores),
                _ => None,
            }
            .or(process.map(|p| p.cpu_percent as f64))
            .unwrap_or(0.0);
            let cpu_split = split(
                &tabs
                    .iter()
                    .map(|t| (t.second.task_seconds - t.first.task_seconds).max(0.0))
                    .collect::<Vec<_>>(),
            );
            let memory_split = split(&tabs.iter().map(|t| t.second.heap_bytes).collect::<Vec<_>>());
            if let Some(pid) = pid {
                attributed.insert(*pid);
            }
            for (i, tab) in tabs.iter().enumerate() {
                let shared = pid.is_some() && tabs.len() > 1;
                let mut evidence = Vec::new();
                match (pid, process) {
                    (Some(pid), Some(p)) => evidence.push(format!(
                        "Runs in renderer process {pid} ({}).",
                        super::size(p.memory_bytes)
                    )),
                    (Some(pid), None) => evidence.push(format!(
                        "Runs in renderer process {pid}, which started after the last full sample."
                    )),
                    (None, _) => evidence.push(
                        "Its renderer process could not be identified; only the tab's JavaScript heap is shown.".into(),
                    ),
                }
                if shared {
                    evidence.push(format!(
                        "Shares that process with {} other tab{}; its share is estimated from JavaScript heap and main-thread time.",
                        tabs.len() - 1,
                        if tabs.len() == 2 { "" } else { "s" }
                    ));
                }
                if tab.second.heap_bytes > 0.0 {
                    evidence.push(format!(
                        "JavaScript heap: {}.",
                        super::size(tab.second.heap_bytes as u64)
                    ));
                }
                units.push(Unit {
                    id: tab.id.clone(),
                    kind: "tab".into(),
                    title: if tab.title.is_empty() {
                        tab.url.chars().take(120).collect()
                    } else {
                        tab.title.clone()
                    },
                    detail: tab.url.chars().take(400).collect(),
                    host: host(&tab.url),
                    cpu_percent: (cpu * cpu_split[i]) as f32,
                    memory_bytes: match process {
                        Some(p) if shared => (p.memory_bytes as f64 * memory_split[i]) as u64,
                        Some(p) => p.memory_bytes,
                        None => tab.second.heap_bytes as u64,
                    },
                    shared_with: if shared { tabs.len() - 1 } else { 0 },
                    pid: *pid,
                    start_time: process.map(|p| p.start_time),
                    actions: vec!["focus_tab".into(), "close_tab".into()],
                    evidence,
                    confidence: match (pid, tab.ambiguous) {
                        (None, _) => "Process not identified".into(),
                        (Some(_), true) => "Probable process match".into(),
                        (Some(_), false) => "Matched by CPU time".into(),
                    },
                });
            }
        }
        if let (Some(a), Some(d)) = (&m.before, &m.after) {
            for (pid, kind) in &a.kind {
                if attributed.contains(pid) {
                    continue;
                }
                let label = match kind.as_str() {
                    "browser" => "Browser window, tab strip, and downloads",
                    "GPU" => "Graphics (GPU process, shared by all tabs)",
                    "renderer" => "Extensions, background pages, and workers",
                    _ => "Network, storage, audio, and utility services",
                };
                let entry = remainder.entry(label).or_default();
                entry.cpu_percent += core_share(*pid, a, d)
                    .map(|s| s * 100.0 / cores)
                    .unwrap_or(0.0) as f32;
                entry.memory_bytes += by_pid.get(pid).map(|p| p.memory_bytes).unwrap_or(0);
                entry.process_count += 1;
                attributed.insert(*pid);
            }
        }
    }
    if verified == 0 {
        return Inside::status(
            workload_id,
            SOURCE,
            "unavailable",
            "The browser that allowed debugging is a different app or profile than this workload.",
        );
    }
    let rest: Vec<_> = members
        .iter()
        .filter(|p| !attributed.contains(&p.pid))
        .collect();
    if !rest.is_empty() {
        remainder.insert(
            "Other processes of this app (other profiles or windows not connected)",
            Remainder {
                label: String::new(),
                cpu_percent: rest.iter().map(|p| p.cpu_percent).sum(),
                memory_bytes: rest.iter().map(|p| p.memory_bytes).sum(),
                process_count: rest.len(),
            },
        );
    }
    units.sort_by(|a, b| {
        (b.cpu_percent + b.memory_bytes as f32 / 1e9)
            .total_cmp(&(a.cpu_percent + a.memory_bytes as f32 / 1e9))
    });
    let mut remainder: Vec<Remainder> = remainder
        .into_iter()
        .map(|(label, r)| Remainder {
            label: label.into(),
            ..r
        })
        .collect();
    remainder.sort_by_key(|r| std::cmp::Reverse(r.memory_bytes));
    let mut inside = Inside::status(
        workload_id,
        SOURCE,
        "ready",
        format!(
            "{} tab{} measured over one second.",
            units.len(),
            if units.len() == 1 { "" } else { "s" }
        ),
    );
    inside.units = units;
    inside.remainder = remainder;
    inside.measured_seconds = WINDOW.as_secs_f32();
    inside.limits = "Tab CPU is measured over the last second; memory is the tab's renderer process from the last five-second sample. Tabs that share a renderer are split by an estimate. The GPU, network, and browser processes serve every tab and are shown separately. Discarded/sleeping tabs use no renderer. Titles and addresses stay on this computer.".into();
    inside
}

/// Focus or close a tab after confirming it still shows the address the person reviewed.
pub fn act(target_id: &str, expected_url: &str, action: &str) -> Result<String, String> {
    if !matches!(action, "close_tab" | "focus_tab") {
        return Err("Unsupported tab action.".into());
    }
    let (results, errors) = with_browsers(|cdp| {
        let targets = cdp.call("Target.getTargets", json!({}), None)?;
        let Some(target) = targets
            .get("targetInfos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|t| t.get("targetId").and_then(Value::as_str) == Some(target_id))
            .cloned()
        else {
            return Ok(None);
        };
        if target.get("url").and_then(Value::as_str) != Some(expected_url) {
            return Err("This tab navigated since you looked. Refresh the tab list first.".into());
        }
        let method = if action == "close_tab" {
            "Target.closeTarget"
        } else {
            "Target.activateTarget"
        };
        cdp.call(method, json!({"targetId": target_id}), None)?;
        Ok(Some(()))
    });
    if results.iter().any(Option::is_some) {
        return Ok(if action == "close_tab" {
            "Tab closed. Its memory is released once the browser tears down the renderer; the next sample shows the change.".into()
        } else {
            "Tab brought to the front in its browser window.".into()
        });
    }
    Err(errors
        .into_iter()
        .find(|e| e.contains("navigated"))
        .unwrap_or_else(|| "That tab is no longer open.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readings(values: &[(u32, f64)]) -> Processes {
        Processes {
            at: Some(Instant::now()),
            cpu: values.iter().copied().collect(),
            kind: values
                .iter()
                .map(|(p, _)| (*p, "renderer".into()))
                .collect(),
        }
    }

    #[test]
    fn brackets_pick_the_process_that_ran_the_tab() {
        let a = readings(&[(1, 0.0694), (2, 0.0684), (3, 0.4607)]);
        let b = readings(&[(1, 0.0702), (2, 0.0684), (3, 0.4616)]);
        let c = readings(&[(1, 0.0703), (2, 0.0684), (3, 0.9600)]);
        let d = readings(&[(1, 0.0710), (2, 0.0684), (3, 0.9611)]);
        assert_eq!(candidates([0.0699, 0.0706], [(&a, &b), (&c, &d)]), vec![1]);
        assert_eq!(candidates([0.4614, 0.9605], [(&a, &b), (&c, &d)]), vec![3]);
        // A busy reading cannot match an idle process with an older counter.
        assert!(candidates([0.5, 1.0], [(&a, &b), (&c, &d)]).is_empty());
    }

    #[test]
    fn shared_processes_split_by_weight_or_evenly() {
        assert_eq!(split(&[3.0, 1.0]), vec![0.75, 0.25]);
        assert_eq!(split(&[0.0, 0.0]), vec![0.5, 0.5]);
    }

    #[test]
    fn only_loopback_endpoints_are_opened() {
        assert!(Cdp::open("ws://example.com:9222/devtools/browser/x").is_err());
    }
}
