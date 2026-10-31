//! What agents can see and ask for. Results go to the agent's model provider, so they carry
//! names an agent needs to decide (app labels, session titles and folder names, tab titles and
//! hosts) but no full paths, URLs, PIDs of unrelated processes, or command lines.
use crate::{
    automation::{Case, Stage},
    inside::{self, sessions, Unit},
    supervisor::Runtime,
    types::*,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Tabs an agent has seen, so a pause or close request is checked against the reviewed address.
static TABS: Mutex<Vec<(Unit, String)>> = Mutex::new(Vec::new());
const TAB_MEMORY: usize = 200;
const SESSIONS_PER_APP: usize = 5;

fn gib(bytes: u64) -> f64 {
    (bytes as f64 / 1_073_741_824.0 * 10.0).round() / 10.0
}

fn mib(bytes: u64) -> u64 {
    bytes / 1_048_576
}

fn round(value: f32) -> f64 {
    (value as f64 * 10.0).round() / 10.0
}

fn folder(path: &str) -> Option<String> {
    let path = std::path::Path::new(path);
    // The home folder's name is the account name, not a project.
    if dirs::home_dir().as_deref() == Some(path) {
        return None;
    }
    path.file_name()
        .map(|f| f.to_string_lossy().chars().take(60).collect())
}

pub fn session_id(unit: &Unit) -> String {
    format!(
        "session:{}-{}",
        unit.pid.unwrap_or_default(),
        unit.start_time.unwrap_or_default()
    )
}

fn parse_tab(id: &str) -> Result<&str, String> {
    id.strip_prefix("tab:")
        .filter(|rest| !rest.is_empty())
        .ok_or_else(|| "Unknown tab ID. Call jitterbug_inspect_app first.".into())
}

fn parse_session(id: &str) -> Result<(u32, u64), String> {
    id.strip_prefix("session:")
        .and_then(|rest| rest.split_once('-'))
        .and_then(|(pid, start)| Some((pid.parse().ok()?, start.parse().ok()?)))
        .ok_or_else(|| "Unknown session ID. Call jitterbug_inspect_app first.".into())
}

/// The caller's live ancestors, newest first. The caller may be newer than the last sample.
fn lineage(caller: u32) -> Vec<(u32, u64)> {
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let mut out = vec![];
    let mut current = Some(Pid::from_u32(caller));
    while let Some(pid) = current {
        let Some(p) = system.process(pid) else { break };
        if out.len() >= 64 || out.iter().any(|(id, _)| *id == pid.as_u32()) {
            break;
        }
        out.push((pid.as_u32(), p.start_time()));
        current = p.parent();
    }
    out
}

/// The session the caller runs inside, when an app launched it.
fn own_session(snapshot: &Snapshot, lineage: &[(u32, u64)]) -> Option<Unit> {
    lineage
        .iter()
        .find_map(|(pid, _)| sessions::session_of(snapshot, *pid))
}

fn inside_caller(unit: &Unit, lineage: &[(u32, u64)]) -> bool {
    lineage
        .iter()
        .any(|(pid, start)| Some(*pid) == unit.pid && Some(*start) == unit.start_time)
}

fn session_json(unit: &Unit, lineage: &[(u32, u64)]) -> Value {
    json!({
        "unit_id": session_id(unit),
        "title": unit.title,
        "command": unit.host,
        "folder": folder(&unit.detail),
        "cpu_percent_of_machine": round(unit.cpu_percent),
        "memory_mib": mib(unit.memory_bytes),
        "process_count": unit.process_count,
        "paused_until": unit.paused_until,
        "agent_status": unit.agent_status,
        "is_your_session": inside_caller(unit, lineage),
        "actions": unit.actions.iter().filter(|a| matches!(a.as_str(), "pause" | "resume")).collect::<Vec<_>>(),
        "evidence": unit.evidence,
        "confidence": unit.confidence,
    })
}

/// A tab's title, or its host when the browser shows the address in place of a title.
fn tab_title(unit: &Unit) -> String {
    if unit.title.is_empty() || unit.detail.starts_with(&unit.title) {
        unit.host.clone().unwrap_or_else(|| "Untitled tab".into())
    } else {
        unit.title.clone()
    }
}

fn tab_json(unit: &Unit) -> Value {
    // Pausing is offered only for a tab in the background; a paused tab can only be resumed.
    let actions: Vec<&str> = if unit.paused_until.is_some() {
        vec!["resume_tab"]
    } else {
        ["pause_tab", "close_tab"]
            .into_iter()
            .filter(|a| unit.actions.iter().any(|u| u == a))
            .collect()
    };
    json!({
        "unit_id": format!("tab:{}", unit.id),
        "title": tab_title(unit),
        "host": unit.host,
        "cpu_percent_of_machine": round(unit.cpu_percent),
        "memory_mib": mib(unit.memory_bytes),
        "shares_process_with_tabs": unit.shared_with,
        "resumes_at": unit.paused_until,
        "actions": actions,
        "confidence": unit.confidence,
    })
}

fn case_json(case: &Case) -> Value {
    // While what the request paused is still paused, when it resumes by itself.
    let resumes_at = case
        .proposal
        .as_ref()
        .and_then(|p| match p.action.as_str() {
            "pause_tab" => p
                .unit
                .as_ref()
                .and_then(|u| inside::chrome::paused_until(&u.id)),
            "pause" => crate::pause::list()
                .into_iter()
                .find(|q| q.pid == p.process.pid && q.start_time == p.process.start_time)
                .map(|q| q.until),
            _ => None,
        });
    json!({
        "request_id": case.id,
        "status": case.stage,
        "title": case.trigger.title,
        "latest": case.steps.last().map(|s| s.message.clone()),
        "result": case.verification.as_ref().map(|v| v.summary.clone()),
        "waiting_for_person": case.stage == Stage::AwaitingApproval,
        "resumes_at": resumes_at,
    })
}

fn snapshot(runtime: &Runtime) -> Result<Snapshot, String> {
    let s = runtime
        .snapshot
        .read()
        .map_err(|_| "Telemetry unavailable.")?
        .clone();
    if s.source != "live" || !s.sample_ready || s.captured_at.saturating_add(20) < now() {
        return Err("Jitterbug has no fresh measurements yet. Try again in a few seconds.".into());
    }
    Ok(s)
}

fn requester(snapshot: &Snapshot, lineage: &[(u32, u64)]) -> String {
    own_session(snapshot, lineage)
        .map(|u| format!("An agent in {}", u.title))
        .unwrap_or_else(|| "An agent".into())
}

pub fn handle(
    runtime: &Arc<Runtime>,
    caller: u32,
    method: &str,
    params: &Value,
) -> Result<Value, String> {
    let policy = runtime
        .settings
        .lock()
        .map_err(|_| "Settings unavailable.")?
        .automation
        .clone();
    if !policy.agent_control {
        return Err("Agent control is turned off in Jitterbug's settings.".into());
    }
    let text = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Missing {key}."))
    };
    match method {
        "overview" => overview(&snapshot(runtime)?, &lineage(caller)),
        "inspect_app" => inspect_app(&snapshot(runtime)?, text("workload_id")?, &lineage(caller)),
        "pause_session" => {
            let snapshot = snapshot(runtime)?;
            let lineage = lineage(caller);
            let (pid, start) = parse_session(text("unit_id")?)?;
            let unit = sessions::session_of(&snapshot, pid)
                .filter(|u| u.pid == Some(pid) && u.start_time == Some(start))
                .ok_or("That session is no longer running inside an app.")?;
            if inside_caller(&unit, &lineage) {
                return Err("This is your own session. An agent cannot pause itself.".into());
            }
            if !unit.actions.iter().any(|a| a == "pause") {
                return Err("This session is already paused.".into());
            }
            let workload = snapshot
                .processes
                .iter()
                .find(|p| p.pid == pid && p.start_time == start)
                .map(|p| p.workload_id.clone());
            let option =
                inside::options::pause(&snapshot, unit).ok_or("This session cannot be paused.")?;
            let who = requester(&snapshot, &lineage);
            let case = tauri::async_runtime::block_on(runtime.agent_request(
                option,
                workload,
                &who,
                text("reason").unwrap_or("No reason given."),
            ))?;
            Ok(case_json(&case))
        }
        "resume_session" => {
            let (pid, start) = parse_session(text("unit_id")?)?;
            Ok(json!({ "message": runtime.agent_resume(pid, start)? }))
        }
        "resume_tab" => {
            let id = parse_tab(text("unit_id")?)?;
            Ok(json!({ "message": runtime.agent_resume_tab(id)? }))
        }
        "close_tab" | "pause_tab" => {
            let snapshot = snapshot(runtime)?;
            let lineage = lineage(caller);
            let id = parse_tab(text("unit_id")?)?;
            let (mut unit, workload_id) = TABS
                .lock()
                .map_err(|_| "Tab list unavailable.")?
                .iter()
                .find(|(u, _)| u.id == id)
                .cloned()
                .ok_or("Unknown tab ID. Call jitterbug_inspect_app on the browser first.")?;
            // The request's title is returned to the agent, so it never carries an address.
            unit.title = tab_title(&unit);
            let workload = snapshot
                .workloads
                .iter()
                .find(|w| w.id == workload_id)
                .ok_or("That browser is no longer running.")?;
            let members = crate::details::members(&snapshot, &workload_id)?;
            let option = if method == "pause_tab" {
                if inside::chrome::paused_until(&unit.id).is_some() {
                    return Err("This tab is already paused.".into());
                }
                if !unit.actions.iter().any(|a| a == "pause_tab") {
                    return Err("This tab is on screen. Only background tabs can be paused; call jitterbug_inspect_app again later.".into());
                }
                inside::options::pause_tab(workload, &members, unit)
            } else {
                inside::options::close_tab(workload, &members, unit)
            }
            .ok_or("That tab can no longer be identified.")?;
            let who = requester(&snapshot, &lineage);
            let case = tauri::async_runtime::block_on(runtime.agent_request(
                option,
                Some(workload_id),
                &who,
                text("reason").unwrap_or("No reason given."),
            ))?;
            Ok(case_json(&case))
        }
        "request_background" => {
            let snapshot = snapshot(runtime)?;
            let option = background_option(&snapshot, &policy, text("workload_id")?)?;
            let who = requester(&snapshot, &lineage(caller));
            let workload = option.process.workload_id.clone();
            let case = tauri::async_runtime::block_on(runtime.agent_request(
                option,
                Some(workload),
                &who,
                text("reason").unwrap_or("No reason given."),
            ))?;
            Ok(case_json(&case))
        }
        "request_status" => {
            let id = params
                .get("request_id")
                .and_then(Value::as_u64)
                .ok_or("Missing request_id.")?;
            let case = runtime.request(id)?;
            if case.origin != crate::automation::Origin::Agent {
                return Err("Unknown request.".into());
            }
            Ok(case_json(&case))
        }
        _ => Err("Unknown method.".into()),
    }
}

/// The request behind `jitterbug_request_background`: the app's busiest process it may lower,
/// never a Protected app or one already in Background.
fn background_option(
    snapshot: &Snapshot,
    policy: &crate::automation::Policy,
    workload_id: &str,
) -> Result<crate::automation::Proposal, String> {
    let workload = snapshot
        .workloads
        .iter()
        .find(|w| w.id == workload_id)
        .ok_or("That app is not running. Call jitterbug_overview for current workload IDs.")?;
    match policy.bucket(workload_id) {
        crate::automation::Bucket::Protected => {
            return Err("The person marked this app Protected. Leave it as it is.".into())
        }
        crate::automation::Bucket::Background => {
            return Err("This app is already in Background.".into())
        }
        crate::automation::Bucket::Normal => {}
    }
    let process = snapshot
        .processes
        .iter()
        .filter(|p| p.workload_id == workload.id && p.actions.iter().any(|a| a == "lower_priority"))
        .max_by(|a, b| a.cpu_percent.total_cmp(&b.cpu_percent))
        .ok_or("Jitterbug cannot change this app's priority.")?;
    Ok(crate::automation::Proposal {
        impact: crate::automation::impact(snapshot, process),
        process: process.clone(),
        action: "lower_priority".into(),
        rationale: "An agent asked to move this app to Background: macOS runs it in background mode, so other apps get CPU, disk, and network first. It keeps running, and you can move it back to Normal at any time.".into(),
        unit: None,
    })
}

pub fn overview(snapshot: &Snapshot, lineage: &[(u32, u64)]) -> Result<Value, String> {
    let mut apps: Vec<&Workload> = snapshot.workloads.iter().collect();
    apps.sort_by(|a, b| {
        let score = |w: &Workload| w.cpu_percent + w.memory_bytes as f32 / 1e9;
        score(b).total_cmp(&score(a))
    });
    let apps: Vec<Value> = apps
        .into_iter()
        .take(12)
        .map(|w| {
            let members = crate::details::members(snapshot, &w.id).unwrap_or_default();
            let lens = inside::lens(&members);
            let sessions: Vec<Value> = match lens {
                inside::Lens::Sessions => sessions::inspect(snapshot, &w.id, &members)
                    .units
                    .iter()
                    .take(SESSIONS_PER_APP)
                    .map(|u| session_json(u, lineage))
                    .collect(),
                _ => vec![],
            };
            json!({
                "workload_id": w.id,
                "label": w.label,
                "category": w.category,
                "cpu_percent_of_machine": round(w.cpu_percent),
                "memory_mib": mib(w.memory_bytes),
                "process_count": w.process_count,
                "inside": match lens {
                    inside::Lens::Browser => Some("tabs: call jitterbug_inspect_app to measure them"),
                    inside::Lens::Sessions => Some("command sessions"),
                    inside::Lens::None => None,
                },
                "sessions": sessions,
            })
        })
        .collect();
    let paused: Vec<Value> = crate::pause::list()
        .into_iter()
        .map(|p| {
            json!({
                "unit_id": format!("session:{}-{}", p.pid, p.start_time),
                "title": p.label,
                "resumes_at": p.until,
            })
        })
        .chain(inside::chrome::paused().into_iter().map(|p| {
            json!({
                "unit_id": format!("tab:{}", p.target_id),
                "title": p.title,
                "host": p.host,
                "resumes_at": p.until,
            })
        }))
        .collect();
    Ok(json!({
        "measured_at": snapshot.captured_at,
        "machine": {
            "pressure": snapshot.pressure.level,
            "reasons": snapshot.pressure.reasons,
            "cpu_percent": round(snapshot.cpu_percent),
            "memory_available_gib": gib(snapshot.memory_available_bytes),
            "memory_total_gib": gib(snapshot.memory_total_bytes),
            "swap_used_gib": gib(snapshot.swap_used_bytes),
            "os_memory_pressure": snapshot.platform.memory_pressure,
        },
        "apps": apps,
        "paused": paused,
        "your_session": own_session(snapshot, lineage).map(|u| u.title),
        "notes": "CPU is a share of the whole machine. Memory is resident memory and may double-count shared pages. Pausing freezes a session and everything it started, or a background browser tab, without closing it; it resumes on request or after 15 minutes. Prefer pausing an idle or low-priority session or tab over anything that loses work.",
    }))
}

fn inspect_app(
    snapshot: &Snapshot,
    workload_id: &str,
    lineage: &[(u32, u64)],
) -> Result<Value, String> {
    let view = inside::inspect(snapshot, workload_id)?;
    let units: Vec<Value> = view
        .units
        .iter()
        .map(|u| {
            if u.kind == "tab" {
                tab_json(u)
            } else {
                session_json(u, lineage)
            }
        })
        .collect();
    let mut tabs = TABS.lock().map_err(|_| "Tab list unavailable.")?;
    for unit in view.units.iter().filter(|u| u.kind == "tab") {
        tabs.retain(|(seen, _)| seen.id != unit.id);
        tabs.push((unit.clone(), workload_id.to_string()));
    }
    let excess = tabs.len().saturating_sub(TAB_MEMORY);
    tabs.drain(..excess);
    Ok(json!({
        "workload_id": workload_id,
        "source": view.source,
        "status": view.status,
        "message": view.message,
        "setup": view.setup,
        "units": units,
        "rest_of_app": view.remainder.iter().map(|r| json!({
            "label": r.label,
            "cpu_percent_of_machine": round(r.cpu_percent),
            "memory_mib": mib(r.memory_bytes),
            "process_count": r.process_count,
        })).collect::<Vec<_>>(),
        "limits": view.limits,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: Option<u32>, name: &str, exe: &str, mem: u64) -> ProcessRecord {
        ProcessRecord {
            pid,
            start_time: pid as u64,
            parent_pid: parent,
            name: name.into(),
            executable: Some(exe.into()),
            memory_bytes: mem << 20,
            cpu_percent: 2.0,
            workload_id: "code".into(),
            ..Default::default()
        }
    }

    fn editor() -> Snapshot {
        let app = "/A/Visual Studio Code.app/Contents";
        let mut a = p(5, Some(4), "claude", "/u/bin/claude", 900);
        a.working_directory = Some("/private/w/feat-search".into());
        let mut b = p(7, Some(6), "claude", "/u/bin/claude", 300);
        b.working_directory = Some("/private/w/fix-login".into());
        Snapshot {
            source: "live".into(),
            captured_at: now(),
            sample_ready: true,
            workloads: vec![Workload {
                id: "code".into(),
                label: "Visual Studio Code".into(),
                category: "editor".into(),
                memory_bytes: 1600 << 20,
                cpu_percent: 6.0,
                process_count: 6,
                ..Default::default()
            }],
            processes: vec![
                p(1, None, "Code", &format!("{app}/MacOS/Code"), 300),
                p(
                    2,
                    Some(1),
                    "Code Helper",
                    &format!("{app}/Frameworks/H.app/Contents/MacOS/h"),
                    100,
                ),
                p(4, Some(2), "zsh", "/bin/zsh", 1),
                a,
                p(6, Some(2), "zsh", "/bin/zsh", 1),
                b,
            ],
            ..Default::default()
        }
    }

    #[test]
    fn overview_lists_sessions_inside_editors_without_paths() {
        let s = editor();
        let value = overview(&s, &[(99, 99), (7, 7), (6, 6)]).unwrap();
        let app = &value["apps"][0];
        assert_eq!(app["label"], "Visual Studio Code");
        let sessions = app["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0]["unit_id"], "session:5-5");
        assert_eq!(sessions[0]["folder"], "feat-search");
        assert_eq!(sessions[0]["is_your_session"], false);
        assert_eq!(sessions[1]["is_your_session"], true);
        assert_eq!(
            value["your_session"],
            "claude · fix-login in Visual Studio Code"
        );
        assert!(!value.to_string().contains("/private/w"));
    }

    #[test]
    fn untitled_tabs_are_named_by_host_not_address() {
        let tab = Unit {
            id: "T1".into(),
            kind: "tab".into(),
            title: "https://example.com/reset?token=abc".into(),
            detail: "https://example.com/reset?token=abc&more=1".into(),
            host: Some("example.com".into()),
            ..Default::default()
        };
        let value = tab_json(&tab);
        assert_eq!(value["title"], "example.com");
        assert!(!value.to_string().contains("token"));
        let titled = Unit {
            title: "Inbox".into(),
            ..tab
        };
        assert_eq!(tab_json(&titled)["title"], "Inbox");
    }

    #[test]
    fn tab_actions_offer_pause_before_close_and_resume_when_paused() {
        let tab = Unit {
            id: "T1".into(),
            kind: "tab".into(),
            title: "Live blog".into(),
            detail: "https://news.example.com/live".into(),
            host: Some("news.example.com".into()),
            actions: vec!["focus_tab".into(), "pause_tab".into(), "close_tab".into()],
            ..Default::default()
        };
        let value = tab_json(&tab);
        assert_eq!(value["actions"], json!(["pause_tab", "close_tab"]));
        assert!(value["resumes_at"].is_null());
        let on_screen = Unit {
            actions: vec!["focus_tab".into(), "close_tab".into()],
            ..tab.clone()
        };
        assert_eq!(tab_json(&on_screen)["actions"], json!(["close_tab"]));
        let paused = Unit {
            actions: vec!["resume_tab".into(), "close_tab".into()],
            paused_until: Some(1234),
            ..tab
        };
        let value = tab_json(&paused);
        assert_eq!(value["actions"], json!(["resume_tab"]));
        assert_eq!(value["resumes_at"], 1234);
        assert!(!value.to_string().contains("/live"));
    }

    #[test]
    fn tab_ids_need_the_tab_prefix() {
        assert_eq!(parse_tab("tab:ABC").unwrap(), "ABC");
        assert!(parse_tab("tab:").is_err());
        assert!(parse_tab("session:5-7").is_err());
        assert!(parse_tab("ABC").is_err());
    }

    #[test]
    fn session_ids_round_trip_and_reject_garbage() {
        assert_eq!(parse_session("session:5-77").unwrap(), (5, 77));
        assert!(parse_session("tab:5-77").is_err());
        assert!(parse_session("session:x-1").is_err());
    }
}
