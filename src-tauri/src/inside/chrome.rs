//! Chromium tabs through the browser's own DevTools endpoint (local loopback only).
//!
//! The protocol lists tabs but never says which renderer process hosts a tab. Each tab does
//! report its renderer's cumulative CPU time (`Performance.getMetrics` → `ProcessTime`), and
//! `SystemInfo.getProcessInfo` reports the same counter per PID. Reading both twice, bracketed in
//! time, identifies the tab's PID; its memory then comes from the normal collector inventory.
//!
//! A background tab can also be paused: `Page.setWebLifecycleState` freezes its scripts and
//! timers, but only while the DevTools session that froze it stays attached. Jitterbug keeps
//! that session open on the browser connection; when the connection closes (Jitterbug quits or
//! the browser restarts), the tab wakes up by itself. Pauses also expire after
//! `pause::LIMIT_SECONDS`. Freezing frees no memory; only closing a tab does.
use super::{host, Inside, ProcessLabel, Remainder, Unit};
use crate::types::*;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    net::TcpStream,
    path::PathBuf,
    sync::{Mutex, MutexGuard, TryLockError},
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

pub const SOURCE: &str = "Browser DevTools";
const WINDOW: Duration = Duration::from_secs(1);
/// Counter granularity: one tab's reading must fall inside its process's bracket.
const EPSILON: f64 = 0.000_2;
const TAB_LIMIT: usize = 100;
/// A tab's address as shown and rechecked: long addresses are cut to this many characters.
const DETAIL_LIMIT: usize = 400;

/// Kept open: a browser that asks before allowing a debugging client asks once per connection.
/// Paused tabs stay frozen only while their connection is open. Lock order: this, then `PAUSED`.
static CONNECTIONS: Mutex<Vec<Cdp>> = Mutex::new(Vec::new());
/// Tabs paused through an open connection. Held only briefly, so readers never wait on a
/// browser measurement or permission prompt.
static PAUSED: Mutex<Vec<PausedTab>> = Mutex::new(Vec::new());

struct Cdp {
    endpoint: String,
    socket: WebSocket<TcpStream>,
    next: u64,
    /// The socket failed; dropping the connection wakes every tab it paused.
    broken: bool,
    /// Sessions the browser ended, such as a paused tab the person closed.
    detached: Vec<String>,
}

/// One tab Jitterbug froze. Title and host are for display; the session keeps it frozen.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PausedTab {
    pub target_id: String,
    pub title: String,
    pub host: Option<String>,
    pub paused_at: u64,
    /// When it resumes by itself.
    pub until: u64,
    #[serde(skip)]
    endpoint: String,
    #[serde(skip)]
    session: String,
}

fn connections() -> MutexGuard<'static, Vec<Cdp>> {
    CONNECTIONS.lock().unwrap_or_else(|e| e.into_inner())
}

fn ledger() -> MutexGuard<'static, Vec<PausedTab>> {
    PAUSED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Forget pauses that no longer hold: their connection closed (the browser woke the tab when
/// the session ended) or the browser ended their session (the tab closed). Returns them.
fn settle(paused: &mut Vec<PausedTab>, connected: &[&str], detached: &[String]) -> Vec<PausedTab> {
    let (gone, kept) = std::mem::take(paused)
        .into_iter()
        .partition(|p| !connected.contains(&p.endpoint.as_str()) || detached.contains(&p.session));
    *paused = kept;
    gone
}

/// Take the pauses that reached their limit.
fn due(paused: &mut Vec<PausedTab>, at: u64) -> Vec<PausedTab> {
    let (due, kept) = std::mem::take(paused)
        .into_iter()
        .partition(|p| p.until <= at);
    *paused = kept;
    due
}

/// Drop failed connections and reconcile the paused list with what is still open.
fn tidy(open: &mut Vec<Cdp>) {
    open.retain(|c| !c.broken);
    let detached: Vec<String> = open
        .iter_mut()
        .flat_map(|c| std::mem::take(&mut c.detached))
        .collect();
    let connected: Vec<&str> = open.iter().map(|c| c.endpoint.as_str()).collect();
    settle(&mut ledger(), &connected, &detached);
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
            broken: false,
            detached: Vec::new(),
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
        if self.broken
            || self
                .socket
                .send(Message::text(message.to_string()))
                .is_err()
        {
            self.broken = true;
            return Err("The browser closed the debugging connection.".into());
        }
        loop {
            let text = match self.socket.read() {
                Ok(Message::Text(text)) => text,
                Ok(Message::Close(_)) | Err(_) => {
                    self.broken = true;
                    return Err("The browser closed the debugging connection.".into());
                }
                Ok(_) => continue,
            };
            // Events and replies to other sessions are skipped.
            let Ok(reply) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if reply.get("method").and_then(Value::as_str) == Some("Target.detachedFromTarget") {
                if let Some(session) = reply["params"]["sessionId"].as_str() {
                    self.detached.push(session.into());
                }
                continue;
            }
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

/// Whether a browser connection the person allowed is already open.
pub fn connected() -> bool {
    !connections().is_empty()
}

/// Tabs paused now, oldest first.
pub fn paused() -> Vec<PausedTab> {
    ledger().clone()
}

/// When a paused tab resumes by itself, if it is paused.
pub fn paused_until(target_id: &str) -> Option<u64> {
    ledger()
        .iter()
        .find(|p| p.target_id == target_id)
        .map(|p| p.until)
}

fn detail(url: &str) -> String {
    url.chars().take(DETAIL_LIMIT).collect()
}

/// Run `f` on every reachable browser, reusing open connections. Without `connect`, only
/// connections that are already open are used, so the browser never asks to allow one.
fn with_browsers<T>(
    connect: bool,
    mut f: impl FnMut(&mut Cdp) -> Result<T, String>,
) -> (Vec<T>, Vec<String>) {
    let mut open = connections();
    let wanted = endpoints();
    open.retain(|c| wanted.contains(&c.endpoint));
    tidy(&mut open);
    let (mut results, mut errors) = (Vec::new(), Vec::new());
    for endpoint in wanted {
        let index = match open.iter().position(|c| c.endpoint == endpoint) {
            Some(i) => i,
            None if !connect => continue,
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
            Err(e) => errors.push(e),
        }
        // A stale socket is dropped below; the next request reconnects. A refused request on
        // a working connection keeps it, so tabs paused through it stay paused.
        if open[index].broken {
            open.remove(index);
        }
    }
    tidy(&mut open);
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
    /// On screen (or its visibility could not be read): never offered for pausing.
    visible: bool,
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
            visible: true,
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
            tab.visible = visible(cdp, &session);
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

/// Whether the page is on screen. A page whose visibility cannot be read counts as visible, so
/// it is never paused. Works on a frozen page too.
fn visible(cdp: &mut Cdp, session: &str) -> bool {
    cdp.call(
        "Runtime.evaluate",
        json!({"expression": "document.visibilityState", "returnByValue": true}),
        Some(session),
    )
    .ok()
    .and_then(|r| r["result"]["value"].as_str().map(|v| v != "hidden"))
    .unwrap_or(true)
}

/// What can be done with a tab: a paused tab can only be resumed or closed; a tab on screen is
/// never paused (a frozen page would stop drawing in front of the person).
fn tab_actions(paused: bool, visible: bool) -> Vec<String> {
    let actions: &[&str] = match (paused, visible) {
        (true, _) => &["resume_tab", "close_tab"],
        (false, true) => &["focus_tab", "close_tab"],
        (false, false) => &["focus_tab", "pause_tab", "close_tab"],
    };
    actions.iter().map(|a| a.to_string()).collect()
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
    measure_tabs(snapshot, workload_id, members, true)
}

/// Like `inspect`, but only through a connection the person already allowed: for background
/// incidents, which must never make the browser ask.
pub fn inspect_connected(
    snapshot: &Snapshot,
    workload_id: &str,
    members: &[&ProcessRecord],
) -> Inside {
    measure_tabs(snapshot, workload_id, members, false)
}

fn measure_tabs(
    snapshot: &Snapshot,
    workload_id: &str,
    members: &[&ProcessRecord],
    connect: bool,
) -> Inside {
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
    let (measured, errors) = with_browsers(connect, measure);
    let paused = self::paused();
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
    let mut roles: Vec<(u32, &str)> = Vec::new();
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
                let paused_until = paused
                    .iter()
                    .find(|p| p.target_id == tab.id)
                    .map(|p| p.until);
                if paused_until.is_some() {
                    evidence.push(
                        "Paused by Jitterbug: its scripts and timers are frozen until it resumes. Its memory stays allocated.".into(),
                    );
                } else if tab.visible {
                    evidence.push("On screen now, so it is not offered for pausing.".into());
                }
                units.push(Unit {
                    id: tab.id.clone(),
                    kind: "tab".into(),
                    title: if tab.title.is_empty() {
                        tab.url.chars().take(120).collect()
                    } else {
                        tab.title.clone()
                    },
                    detail: detail(&tab.url),
                    host: host(&tab.url),
                    cpu_percent: (cpu * cpu_split[i]) as f32,
                    memory_bytes: match process {
                        Some(p) if shared => (p.memory_bytes as f64 * memory_split[i]) as u64,
                        Some(p) => p.memory_bytes,
                        None => tab.second.heap_bytes as u64,
                    },
                    process_count: usize::from(pid.is_some()),
                    shared_with: if shared { tabs.len() - 1 } else { 0 },
                    pid: *pid,
                    start_time: process.map(|p| p.start_time),
                    actions: tab_actions(paused_until.is_some(), tab.visible),
                    evidence,
                    confidence: match (pid, tab.ambiguous) {
                        (None, _) => "Process not identified".into(),
                        (Some(_), true) => "Probable process match".into(),
                        (Some(_), false) => "Matched by CPU time".into(),
                    },
                    paused_until,
                });
            }
        }
        if let (Some(a), Some(d)) = (&m.before, &m.after) {
            for (pid, kind) in &a.kind {
                if attributed.contains(pid) {
                    continue;
                }
                let label = role(kind).0;
                roles.push((*pid, kind));
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
    inside.labels = labels(snapshot, &units, &roles);
    inside.units = units;
    inside.remainder = remainder;
    inside.measured_seconds = WINDOW.as_secs_f32();
    inside.limits = "Tab CPU is measured over the last second; memory is the tab's renderer process from the last five-second sample. Tabs that share a renderer are split by an estimate. The GPU, network, and browser processes serve every tab and are shown separately. Discarded/sleeping tabs use no renderer. Titles and addresses stay on this computer.".into();
    inside
}

/// A browser process kind as (summary row label, (process title, subtitle)).
pub fn role(kind: &str) -> (&'static str, (&'static str, &'static str)) {
    match kind {
        "browser" => (
            "Browser window, tab strip, and downloads",
            ("Browser window", "Windows, tab strip, and downloads"),
        ),
        "GPU" => (
            "Graphics (GPU process, shared by all tabs)",
            ("Graphics (shared by all tabs)", "Draws every tab on screen"),
        ),
        "renderer" => (
            "Extensions, background pages, and workers",
            ("Extensions", "Extensions, background pages, and workers"),
        ),
        _ => (
            "Network, storage, audio, and utility services",
            (
                "Network & storage",
                "Network, storage, audio, and utility services",
            ),
        ),
    }
}

/// Name each measured process: a renderer by the tab(s) it runs, the rest by their role.
fn labels(snapshot: &Snapshot, units: &[Unit], roles: &[(u32, &str)]) -> Vec<ProcessLabel> {
    let by_pid: HashMap<u32, &ProcessRecord> =
        snapshot.processes.iter().map(|p| (p.pid, p)).collect();
    let mut tabs: Vec<(u32, Vec<&Unit>)> = Vec::new();
    for unit in units.iter().filter(|u| u.kind == "tab") {
        let Some(pid) = unit.pid else { continue };
        match tabs.iter_mut().find(|(p, _)| *p == pid) {
            Some((_, group)) => group.push(unit),
            None => tabs.push((pid, vec![unit])),
        }
    }
    let mut out = Vec::new();
    for (pid, mut group) in tabs {
        let Some(process) = by_pid.get(&pid) else {
            continue;
        };
        // The tab holding most of the process's memory names it first.
        group.sort_by_key(|u| std::cmp::Reverse(u.memory_bytes));
        let mut hosts: Vec<&str> = Vec::new();
        // An extension page's "host" is an opaque extension ID, not a site.
        for host in group
            .iter()
            .filter(|u| !u.detail.starts_with("chrome-extension://"))
            .filter_map(|u| u.host.as_deref())
        {
            if !hosts.contains(&host) {
                hosts.push(host);
            }
        }
        let title = match group.as_slice() {
            [one] => one.title.clone(),
            many => {
                let names: Vec<String> = many.iter().take(2).map(|u| short(&u.title)).collect();
                format!(
                    "{} tabs: {}{}",
                    many.len(),
                    names.join(", "),
                    if many.len() > 2 { ", …" } else { "" }
                )
            }
        };
        let subtitle = match hosts.len() {
            0..=3 => hosts.join(", "),
            n => format!("{} and {} more", hosts[..2].join(", "), n - 2),
        };
        out.push(ProcessLabel::new(process, title, subtitle));
    }
    for (pid, kind) in roles {
        if let Some(process) = by_pid.get(pid) {
            let (title, subtitle) = role(kind).1;
            out.push(ProcessLabel::new(process, title, subtitle));
        }
    }
    out
}

fn short(title: &str) -> String {
    if title.chars().count() <= 32 {
        return title.into();
    }
    let cut: String = title.chars().take(31).collect();
    format!("{}…", cut.trim_end())
}

/// Focus, pause, or close a tab after confirming it still shows the address the person
/// reviewed; resume a tab Jitterbug paused.
pub fn act(target_id: &str, expected_url: &str, action: &str) -> Result<String, String> {
    if action == "resume_tab" {
        return resume(target_id);
    }
    if !matches!(action, "close_tab" | "focus_tab" | "pause_tab") {
        return Err("Unsupported tab action.".into());
    }
    if action == "pause_tab" && paused_until(target_id).is_some() {
        return Err("This tab is already paused.".into());
    }
    // A refusal is an answer on a working connection, not a failed one.
    let (results, errors) = with_browsers(true, |cdp| {
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
        // The address was shortened for display; compare it the same way.
        let url = target.get("url").and_then(Value::as_str).unwrap_or("");
        if detail(url) != expected_url {
            return Ok(Some(Err(
                "This tab navigated since you looked. Refresh the tab list first.".to_string(),
            )));
        }
        if action == "pause_tab" {
            let title = target.get("title").and_then(Value::as_str).unwrap_or("");
            return freeze(cdp, target_id, title, url).map(Some);
        }
        let method = if action == "close_tab" {
            "Target.closeTarget"
        } else {
            "Target.activateTarget"
        };
        cdp.call(method, json!({"targetId": target_id}), None)?;
        if action == "close_tab" {
            ledger().retain(|p| p.target_id != target_id);
        }
        Ok(Some(Ok(match action {
            "close_tab" => "Tab closed. Its memory is released once the browser tears down the renderer; the next sample shows the change.".into(),
            _ => "Tab brought to the front in its browser window.".into(),
        })))
    });
    if let Some(outcome) = results.into_iter().flatten().next() {
        return outcome;
    }
    Err(errors
        .into_iter()
        .next()
        .unwrap_or_else(|| "That tab is no longer open.".into()))
}

/// Attach to a background tab and freeze it, keeping the session attached. The tab must still
/// be off screen: a frozen page in front of the person would stop drawing.
fn freeze(
    cdp: &mut Cdp,
    target_id: &str,
    title: &str,
    url: &str,
) -> Result<Result<String, String>, String> {
    let reply = cdp.call(
        "Target.attachToTarget",
        json!({"targetId": target_id, "flatten": true}),
        None,
    )?;
    let session = reply
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or("The browser did not attach to the tab.")?
        .to_string();
    let refused = if visible(cdp, &session) {
        Some("This tab is on screen. Jitterbug only pauses tabs in the background; switch to another tab first.")
    } else if cdp
        .call(
            "Page.setWebLifecycleState",
            json!({"state": "frozen"}),
            Some(&session),
        )
        .is_err()
    {
        Some("The browser did not pause this tab.")
    } else {
        None
    };
    if let Some(refused) = refused {
        let _ = cdp.call(
            "Target.detachFromTarget",
            json!({"sessionId": session}),
            None,
        );
        return Ok(Err(refused.into()));
    }
    let at = now();
    let title: String = title.chars().take(120).collect();
    ledger().push(PausedTab {
        target_id: target_id.into(),
        // An untitled tab shows its address as the title; keep only the host.
        title: if title.is_empty() || url.starts_with(&title) {
            host(url).unwrap_or_else(|| "Untitled tab".into())
        } else {
            title
        },
        host: host(url),
        paused_at: at,
        until: at + crate::pause::LIMIT_SECONDS,
        endpoint: cdp.endpoint.clone(),
        session,
    });
    Ok(Ok(format!(
        "Tab paused. Its scripts and timers stop until you resume it, or by themselves in {} minutes. Its memory stays allocated; close the tab to free it.",
        crate::pause::LIMIT_SECONDS / 60
    )))
}

/// Unfreeze and let go of the tab. Errors are ignored: a failed connection is dropped, and
/// closing it wakes the tab anyway.
fn wake(cdp: &mut Cdp, session: &str) {
    let _ = cdp.call(
        "Page.setWebLifecycleState",
        json!({"state": "active"}),
        Some(session),
    );
    let _ = cdp.call(
        "Target.detachFromTarget",
        json!({"sessionId": session}),
        None,
    );
}

fn wake_all(open: &mut Vec<Cdp>, tabs: &[PausedTab]) {
    for tab in tabs {
        if let Some(cdp) = open.iter_mut().find(|c| c.endpoint == tab.endpoint) {
            wake(cdp, &tab.session);
        }
    }
    tidy(open);
}

/// Resume one paused tab.
pub fn resume(target_id: &str) -> Result<String, String> {
    let mut open = connections();
    let tab = {
        let mut paused = ledger();
        let index = paused.iter().position(|p| p.target_id == target_id);
        index.map(|i| paused.remove(i))
    };
    let Some(tab) = tab else {
        return Err("This tab is not paused. It may have resumed by itself or been closed.".into());
    };
    wake_all(&mut open, std::slice::from_ref(&tab));
    Ok(format!("Resumed “{}”.", tab.title))
}

/// Resume tabs whose pause reached its limit. Called every sample tick; skipped while a browser
/// is being measured or asked for permission, so monitoring never waits. Returns their titles.
pub fn expire() -> Vec<String> {
    let mut open = match CONNECTIONS.try_lock() {
        Ok(open) => open,
        Err(TryLockError::Poisoned(e)) => e.into_inner(),
        Err(TryLockError::WouldBlock) => return Vec::new(),
    };
    let due = due(&mut ledger(), now());
    if due.is_empty() {
        return Vec::new();
    }
    wake_all(&mut open, &due);
    due.into_iter().map(|p| p.title).collect()
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

    #[test]
    fn long_addresses_are_rechecked_as_shown() {
        let url = format!("https://meet.google.com/abc?{}", "x".repeat(500));
        assert_eq!(detail(&url).chars().count(), DETAIL_LIMIT);
        assert_eq!(detail(&detail(&url)), detail(&url));
    }

    #[test]
    fn renderers_are_named_by_their_tabs_and_the_rest_by_role() {
        let process = |pid: u32| ProcessRecord {
            pid,
            start_time: 100 + pid as u64,
            name: "Google Chrome Helper (Renderer)".into(),
            ..Default::default()
        };
        let snapshot = Snapshot {
            processes: (1..=6).map(process).collect(),
            ..Default::default()
        };
        let tab = |title: &str, url: &str, pid: Option<u32>, memory: u64| Unit {
            kind: "tab".into(),
            title: title.into(),
            detail: url.into(),
            host: host(url),
            pid,
            memory_bytes: memory,
            ..Default::default()
        };
        let units = vec![
            tab(
                "lofi hip hop radio",
                "https://www.youtube.com/watch?v=x",
                Some(1),
                400,
            ),
            tab("Docs", "https://docs.example.com/d/1", Some(2), 300),
            tab("Mail", "https://mail.example.com/", Some(2), 200),
            tab("Calendar", "https://calendar.example.com/", Some(2), 100),
            tab("Sleeping", "https://example.org/", None, 0),
            // A tab whose renderer exited since the inventory sample is not labeled.
            tab("Gone", "https://gone.example.com/", Some(9), 50),
            tab(
                "Viewer",
                "chrome-extension://abcdef/viewer.html",
                Some(6),
                10,
            ),
        ];
        let labels = labels(
            &snapshot,
            &units,
            &[(3, "GPU"), (4, "renderer"), (5, "utility")],
        );
        let titled = |pid: u32| labels.iter().find(|l| l.pid == pid).unwrap();
        assert_eq!(labels.len(), 6);
        assert_eq!(titled(1).title, "lofi hip hop radio");
        assert_eq!(titled(1).subtitle, "www.youtube.com");
        assert_eq!(titled(1).start_time, 101);
        assert_eq!(titled(2).title, "3 tabs: Docs, Mail, …");
        assert_eq!(
            titled(2).subtitle,
            "docs.example.com, mail.example.com, calendar.example.com"
        );
        assert_eq!(titled(3).title, "Graphics (shared by all tabs)");
        assert_eq!(titled(4).title, "Extensions");
        assert_eq!(titled(5).title, "Network & storage");
        assert_eq!(titled(6).subtitle, "");
        assert!(short(&"x".repeat(80)).ends_with('…'));
    }

    fn paused(id: &str, endpoint: &str, session: &str, until: u64) -> PausedTab {
        PausedTab {
            target_id: id.into(),
            title: id.into(),
            host: None,
            paused_at: until - 900,
            until,
            endpoint: endpoint.into(),
            session: session.into(),
        }
    }

    #[test]
    fn pauses_expire_at_their_limit() {
        let mut list = vec![
            paused("a", "ws://x", "s1", 1000),
            paused("b", "ws://x", "s2", 2000),
        ];
        assert!(due(&mut list, 999).is_empty());
        let gone = due(&mut list, 1000);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].target_id, "a");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].target_id, "b");
    }

    #[test]
    fn a_closed_connection_or_ended_session_counts_as_resumed() {
        let mut list = vec![
            paused("a", "ws://one", "s1", 5000),
            paused("b", "ws://two", "s2", 5000),
            paused("c", "ws://one", "s3", 5000),
        ];
        // The browser woke every tab of the dropped connection when its sessions ended.
        let gone = settle(&mut list, &["ws://one"], &[]);
        assert_eq!(
            gone.iter()
                .map(|p| p.target_id.as_str())
                .collect::<Vec<_>>(),
            ["b"]
        );
        // A tab the person closed ends its session.
        let gone = settle(&mut list, &["ws://one"], &["s3".to_string()]);
        assert_eq!(gone[0].target_id, "c");
        assert_eq!(list.len(), 1);
        assert!(settle(&mut list, &[], &[]).len() == 1 && list.is_empty());
    }

    #[test]
    fn only_background_tabs_offer_a_pause_and_paused_tabs_only_resume_or_close() {
        assert_eq!(
            tab_actions(false, false),
            ["focus_tab", "pause_tab", "close_tab"]
        );
        assert_eq!(tab_actions(false, true), ["focus_tab", "close_tab"]);
        assert_eq!(tab_actions(true, false), ["resume_tab", "close_tab"]);
        assert_eq!(tab_actions(true, true), ["resume_tab", "close_tab"]);
    }

    #[test]
    fn nothing_paused_means_nothing_to_resume_or_expire() {
        assert!(resume("no-such-tab").is_err());
        assert!(expire().is_empty());
        assert_eq!(paused_until("no-such-tab"), None);
        assert!(act("x", "", "reload").is_err());
    }

    #[test]
    fn background_reads_never_open_a_connection() {
        let mut calls = 0;
        let _ = with_browsers(false, |_| {
            calls += 1;
            Ok(())
        });
        assert_eq!(calls, 0);
        assert!(!connected());
    }
}
