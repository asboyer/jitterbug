//! Out-of-app popup that follows an incident while another app is in front: capturing,
//! analyzing, asking for approval, and the result. It only surfaces existing cases; approval
//! still runs through the incident commands and their fresh identity, ownership, trigger, and
//! permission rechecks.
use crate::{
    automation::{self, Case, Origin, Policy, Proposal, Stage},
    types::now,
    AppState,
};
use serde_json::{json, Value};
use std::{collections::VecDeque, sync::Mutex, time::Duration};
use tauri::{Emitter, Manager};

pub const LABEL: &str = "popup";
const WIDTH: f64 = 360.0;
const HEIGHT: f64 = 300.0;
const GAP: f64 = 12.0;
/// A finished result stays eligible this long; the popup itself closes it sooner unless the
/// pointer is over it. A later event never resurfaces a stale outcome.
const RESULT_SECONDS: u64 = 60;

#[derive(Default)]
pub struct Popups {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// A snapshot is recording: shown before its case exists.
    capturing: Option<String>,
    /// "Later", Details, Escape, or a closed result. Kept in memory like incident history.
    deferred: VecDeque<u64>,
    /// Synthetic case from the demo menu. It is never sent to an action command.
    demo: Option<Case>,
    current: Option<Value>,
}

pub struct Gate {
    pub snoozed: bool,
    /// The main window is visible and focused; the in-app notice and Inbox handle the case.
    pub main_active: bool,
    pub now: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: u64,
    /// Other cases still waiting for approval.
    pub more: usize,
}

/// Pure display decision: one popup at a time. An approval request wins, then an incident in
/// progress, then a recent result; the newest wins within each.
pub fn choose(cases: &[Case], policy: &Policy, gate: &Gate, deferred: &[u64]) -> Option<Choice> {
    if !policy.action_popups || gate.main_active {
        return None;
    }
    let rank = |c: &Case| match c.stage {
        Stage::AwaitingApproval => 2,
        Stage::Detected | Stage::Investigating | Stage::Applying | Stage::Verifying => 1,
        _ => 0,
    };
    let visible: Vec<&Case> = cases
        .iter()
        .filter(|c| {
            !deferred.contains(&c.id)
                // A manual snapshot was asked for; automatic cases respect pause and disable.
                && (c.origin == Origin::Snapshot || (policy.enabled && !gate.snoozed))
                && match c.stage {
                    // Workloads with a saved permission are applied automatically.
                    Stage::AwaitingApproval => match &c.proposal {
                        Some(p) => !policy.allows(p),
                        None => c.alternative.is_some(),
                    },
                    Stage::Detected | Stage::Investigating | Stage::Applying | Stage::Verifying => {
                        true
                    }
                    Stage::Completed | Stage::Resolved | Stage::Blocked => c
                        .steps
                        .last()
                        .is_some_and(|s| gate.now.saturating_sub(s.at) <= RESULT_SECONDS),
                    Stage::Dismissed => false,
                }
        })
        .collect();
    let chosen = visible
        .iter()
        .max_by_key(|c| (rank(c), c.detected_at, c.id))?;
    let more = visible
        .iter()
        .filter(|c| c.id != chosen.id && c.stage == Stage::AwaitingApproval)
        .count();
    Some(Choice {
        id: chosen.id,
        more,
    })
}

/// Only what the in-app notice and review already show: no paths, PIDs, evidence, or diagnosis.
pub fn view(case: &Case, more: usize) -> Value {
    json!({
        "kind": "case",
        "id": case.id,
        "source": case.source,
        "stage": case.stage,
        "trigger": {
            "kind": case.trigger.kind,
            "title": case.trigger.title,
            "target": case.trigger.target.as_ref().map(|t| json!({ "label": t.label })),
        },
        "proposal": case.proposal.as_ref().map(option),
        "alternative": case.alternative.as_ref().map(option),
        // Shares of total CPU capacity, as everywhere in the app.
        "report": case.report.as_ref().map(|r| json!({
            "fresh": r.fresh,
            "after": r.after.as_ref().map(|_| true),
            "action": r.action,
            "process": r.before.name,
            "cpu_before": r.before.cpu_percent,
            "cpu_after": r.after.as_ref().map(|a| a.cpu_percent),
            "memory_before": r.before.memory_bytes,
            "memory_after": r.after.as_ref().map(|a| a.memory_bytes),
        })),
        "verification": case.verification.as_ref().map(|v| json!({
            "priority_verified": v.priority_verified,
            "system_cpu_before": v.system_cpu_before,
            "system_cpu_after": v.system_cpu_after,
            "available_memory_before": v.available_memory_before,
            "available_memory_after": v.available_memory_after,
        })),
        "origin": case.origin,
        "jitter": case.jitter.as_ref().map(|j| json!({
            "culprit": j.culprit.as_ref().map(|c| json!({ "label": c.label })),
            "victim": j.victim.as_ref().map(|v| json!({ "label": v.label })),
            "happened": j.happened,
            "likely_cause": j.likely_cause,
            "manual": j.manual,
        })),
        "more": more,
    })
}

/// An option's process name and current use, plus what it belongs to and, for a stop, what
/// ends. Names only: no paths or process identity.
fn option(p: &Proposal) -> Value {
    json!({
        "action": p.action,
        "process": {
            "name": p.process.name,
            "cpu_percent": p.process.cpu_percent,
            "memory_bytes": p.process.memory_bytes,
        },
        "impact": p.impact,
        "closes": (p.action == "terminate").then_some(&p.rationale),
    })
}

fn main_active(app: &tauri::AppHandle) -> bool {
    // A window shown on detection can be key while another app stays frontmost.
    #[cfg(target_os = "macos")]
    if !objc2::MainThreadMarker::new()
        .is_some_and(|marker| objc2_app_kit::NSApplication::sharedApplication(marker).isActive())
    {
        return false;
    }
    app.get_webview_window("main").is_some_and(|w| {
        w.is_visible().unwrap_or(false)
            && w.is_focused().unwrap_or(false)
            && !w.is_minimized().unwrap_or(false)
    })
}

/// Re-evaluates after any case, policy, or focus change. Runs on the UI thread so window
/// changes stay ordered and native window calls are safe.
pub fn refresh(app: &tauri::AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || update(&handle));
}

fn update(app: &tauri::AppHandle) {
    let (Some(state), Some(popups)) = (app.try_state::<AppState>(), app.try_state::<Popups>())
    else {
        return;
    };
    let (Ok(mut cases), Ok(snoozed_until), Ok(policy)) = (
        state.cases(),
        state.snoozed_until(),
        state.settings.lock().map(|s| s.automation.clone()),
    ) else {
        return;
    };
    let gate = Gate {
        snoozed: snoozed_until > now(),
        main_active: main_active(app),
        now: now(),
    };
    let Ok(mut inner) = popups.inner.lock() else {
        return;
    };
    cases.extend(inner.demo.clone());
    let deferred: Vec<u64> = inner.deferred.iter().copied().collect();
    let choice = choose(&cases, &policy, &gate, &deferred);
    let asking = choice.as_ref().is_some_and(|choice| {
        cases
            .iter()
            .any(|c| c.id == choice.id && c.stage == Stage::AwaitingApproval)
    });
    // An approval request stays on top; otherwise a recording snapshot is the news.
    let capture = (policy.action_popups && !gate.main_active && !asking)
        .then(|| inner.capturing.clone())
        .flatten()
        .map(|message| json!({ "kind": "capturing", "id": 0, "message": message }));
    let shown = capture.or_else(|| {
        let choice = choice?;
        let case = cases.iter().find(|c| c.id == choice.id)?;
        Some(view(case, choice.more))
    });
    inner.current = shown.clone();
    drop(inner);
    match shown {
        Some(payload) => {
            if let Err(error) = present(app, &payload) {
                // No case details are included in this error.
                eprintln!("Could not show the action popup: {error}");
            }
        }
        None => hide(app, true),
    }
}

fn present(app: &tauri::AppHandle, payload: &Value) -> tauri::Result<()> {
    let window = match app.get_webview_window(LABEL) {
        Some(window) => window,
        None => create(app)?,
    };
    let _ = window.emit("action-popup", payload);
    if !window.is_visible()? {
        place(app, &window)?;
        show_inactive(&window)?;
    }
    Ok(())
}

fn hide(app: &tauri::AppHandle, restore_focus: bool) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let _ = window.emit("action-popup", Value::Null);
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    let restore = restore_focus && window.is_focused().unwrap_or(false) && !main_active(app);
    #[cfg(target_os = "windows")]
    windows::hide(&window);
    #[cfg(not(target_os = "windows"))]
    let _ = window.hide();
    // After a click in the popup, return focus to the app the user was working in, unless they
    // chose to open Jitterbug.
    #[cfg(target_os = "macos")]
    if restore {
        if let Some(marker) = objc2::MainThreadMarker::new() {
            objc2_app_kit::NSApplication::sharedApplication(marker).deactivate();
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = restore;
}

/// Created on first use only, like the quick view.
fn create(app: &tauri::AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    let builder = tauri::WebviewWindowBuilder::new(
        app,
        LABEL,
        tauri::WebviewUrl::App("index.html?view=popup".into()),
    )
    .title("Jitterbug")
    .inner_size(WIDTH, HEIGHT)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .always_on_top(true)
    .visible_on_all_workspaces(true)
    .skip_taskbar(true)
    .focused(false)
    .accept_first_mouse(true)
    .visible(false)
    .shadow(true);
    #[cfg(target_os = "macos")]
    let builder = builder.transparent(true).effects(
        tauri::window::EffectsBuilder::new()
            .effect(tauri::utils::WindowEffect::Popover)
            .state(tauri::utils::WindowEffectState::Active)
            .radius(12.0)
            .build(),
    );
    let window = builder.build()?;
    crate::floating::prepare(&window);
    Ok(window)
}

/// Top-right of the display under the pointer, inside the work area (below the menu bar).
fn place(app: &tauri::AppHandle, window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let monitor = match app.cursor_position() {
        Ok(point) => app.monitor_from_point(point.x, point.y)?,
        Err(_) => None,
    };
    let Some(monitor) = monitor.or(app.primary_monitor()?) else {
        return Ok(());
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let (x, y) = corner(
        [
            area.position.x as f64,
            area.position.y as f64,
            area.size.width as f64,
            area.size.height as f64,
        ],
        [WIDTH * scale, HEIGHT * scale],
        GAP * scale,
    );
    window.set_position(tauri::PhysicalPosition::new(x as i32, y as i32))
}

fn corner(area: [f64; 4], panel: [f64; 2], gap: f64) -> (f64, f64) {
    let x = (area[0] + area[2] - panel[0] - gap).max(area[0]);
    (x, area[1] + gap)
}

/// Orders the popup in front without activating Jitterbug or taking keyboard focus.
fn show_inactive(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    #[cfg(target_os = "macos")]
    return crate::floating::show(window, false);
    #[cfg(target_os = "windows")]
    return windows::show(window);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    window.show()
}

#[cfg(target_os = "windows")]
mod windows {
    use ::windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
            SW_HIDE, SW_SHOWNOACTIVATE,
        },
    };

    fn hwnd(window: &tauri::WebviewWindow) -> tauri::Result<HWND> {
        Ok(HWND(window.hwnd()?.0))
    }

    // The window manager's own show activates after creation, so both directions use Win32.
    pub fn show(window: &tauri::WebviewWindow) -> tauri::Result<()> {
        let hwnd = hwnd(window)?;
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        Ok(())
    }

    pub fn hide(window: &tauri::WebviewWindow) {
        if let Ok(hwnd) = hwnd(window) {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }
}

fn defer(app: &tauri::AppHandle, id: u64) {
    if let Some(popups) = app.try_state::<Popups>() {
        if let Ok(mut inner) = popups.inner.lock() {
            if !inner.deferred.contains(&id) {
                inner.deferred.push_back(id);
            }
            // Incident history holds 12 cases; keep a little more than that.
            while inner.deferred.len() > 32 {
                inner.deferred.pop_front();
            }
            if inner.demo.as_ref().is_some_and(|c| c.id == id) {
                inner.demo = None;
            }
        }
    }
}

/// Show or clear the "Capturing snapshot…" state.
pub fn capturing(app: &tauri::AppHandle, message: Option<String>) {
    if let Some(popups) = app.try_state::<Popups>() {
        if let Ok(mut inner) = popups.inner.lock() {
            inner.capturing = message;
        }
    }
    refresh(app);
}

/// A manual snapshot joined this case: show it again even if it was put off earlier.
pub fn reopen(app: &tauri::AppHandle, id: u64) {
    if let Some(popups) = app.try_state::<Popups>() {
        if let Ok(mut inner) = popups.inner.lock() {
            inner.deferred.retain(|deferred| *deferred != id);
        }
    }
    refresh(app);
}

#[tauri::command]
pub fn get_action_popup(popups: tauri::State<'_, Popups>) -> Option<Value> {
    popups
        .inner
        .lock()
        .ok()
        .and_then(|inner| inner.current.clone())
}

/// "Later", Escape, a finished result, or the in-app notice's "Later": keep the case in Inbox.
#[tauri::command]
pub fn close_action_popup(app: tauri::AppHandle, id: u64) {
    defer(&app, id);
    refresh(&app);
}

/// Details: opens the main window on this case through the app's own incident view.
#[tauri::command]
pub fn open_action_case(app: tauri::AppHandle, id: u64, source: String) -> Result<(), String> {
    defer(&app, id);
    if let Some(popups) = app.try_state::<Popups>() {
        if let Ok(mut inner) = popups.inner.lock() {
            inner.current = None;
        }
    }
    hide(&app, false);
    let window = app
        .get_webview_window("main")
        .ok_or("Main window unavailable.")?;
    window
        .emit("open-incident", json!({ "id": id, "source": source }))
        .map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.unminimize().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

/// Demo menu: after a short delay (time to switch apps), offer a synthetic approval request.
#[tauri::command]
pub fn preview_action_popup(app: tauri::AppHandle) -> Result<(), String> {
    let case = demo_case(now())?;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if let Some(popups) = app.try_state::<Popups>() {
            if let Ok(mut inner) = popups.inner.lock() {
                inner.deferred.retain(|id| *id != case.id);
                inner.demo = Some(case);
            }
        }
        refresh(&app);
    });
    Ok(())
}

fn demo_case(at: u64) -> Result<Case, String> {
    let mut case = automation::simulated_case(at)?;
    let process = case
        .evidence
        .as_ref()
        .and_then(|e| e.processes.first())
        .cloned()
        .ok_or("The synthetic example has no process.")?;
    let label = case
        .trigger
        .target
        .as_ref()
        .map_or_else(|| process.name.clone(), |t| t.label.clone());
    let impact = case.evidence.as_ref().map(|e| automation::Impact {
        label: label.clone(),
        workload_cpu_percent: e.cpu_percent,
        workload_memory_bytes: e.memory_bytes,
        process_count: e.process_count,
        whole_app: false,
    });
    case.alternative = Some(Proposal {
        process: process.clone(),
        action: "terminate".into(),
        rationale: format!("Demo: quit {label}. Nothing will be stopped."),
        impact: impact.clone().map(|i| automation::Impact {
            whole_app: true,
            ..i
        }),
    });
    case.proposal = Some(Proposal {
        process,
        action: "lower_priority".into(),
        rationale: "Demo: a synthetic busy process. Nothing will be applied.".into(),
        impact,
    });
    case.transition(
        Stage::AwaitingApproval,
        "Demo: a priority adjustment is available for review. Nothing will be applied.",
    );
    Ok(case)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::Rule;

    fn waiting(id: u64, detected_at: u64) -> Case {
        let mut case = demo_case(detected_at).unwrap();
        case.id = id;
        case.source = "live".into();
        case
    }
    fn open(now: u64) -> Gate {
        Gate {
            snoozed: false,
            main_active: false,
            now,
        }
    }

    #[test]
    fn follows_the_incident_through_every_stage() {
        let policy = Policy::default();
        let mut case = waiting(1, 100);
        assert_eq!(
            choose(&[case.clone()], &policy, &open(200), &[]),
            Some(Choice { id: 1, more: 0 })
        );
        for stage in [
            Stage::Detected,
            Stage::Investigating,
            Stage::Applying,
            Stage::Verifying,
        ] {
            case.stage = stage;
            assert!(choose(&[case.clone()], &policy, &open(200), &[]).is_some());
        }
        case.transition(Stage::Dismissed, "Dismissed.");
        assert_eq!(choose(&[case], &policy, &open(200), &[]), None);
        // Nothing to apply is still news: a protected process gets a result, not silence.
        let mut finished = waiting(2, 100);
        finished.proposal = None;
        finished.transition(Stage::Completed, "No in-app change fits.");
        let at = finished.steps.last().unwrap().at;
        let cases = [finished];
        assert_eq!(
            choose(&cases, &policy, &open(at + 5), &[]),
            Some(Choice { id: 2, more: 0 })
        );
        assert_eq!(
            choose(&cases, &policy, &open(at + RESULT_SECONDS + 1), &[]),
            None
        );
        assert_eq!(choose(&cases, &policy, &open(at), &[2]), None);
        // A stop alone is still something to ask; with neither option there is nothing.
        let mut stop_only = waiting(3, 100);
        stop_only.proposal = None;
        assert!(choose(&[stop_only.clone()], &policy, &open(200), &[]).is_some());
        stop_only.alternative = None;
        assert_eq!(choose(&[stop_only], &policy, &open(200), &[]), None);
    }

    #[test]
    fn focus_later_snooze_and_settings_suppress_the_popup() {
        let case = waiting(1, 100);
        let cases = [case.clone()];
        let policy = Policy::default();
        let focused = Gate {
            main_active: true,
            ..open(200)
        };
        assert_eq!(choose(&cases, &policy, &focused, &[]), None);
        assert_eq!(choose(&cases, &policy, &open(200), &[1]), None);
        let snoozed = Gate {
            snoozed: true,
            ..open(200)
        };
        assert_eq!(choose(&cases, &policy, &snoozed, &[]), None);
        let off = Policy {
            action_popups: false,
            ..Default::default()
        };
        assert_eq!(choose(&cases, &off, &open(200), &[]), None);
        let disabled = Policy {
            enabled: false,
            ..Default::default()
        };
        assert_eq!(choose(&cases, &disabled, &open(200), &[]), None);
        // A manual snapshot was explicitly requested: pause and disable don't hide it.
        let mut manual = case.clone();
        manual.origin = Origin::Snapshot;
        assert!(choose(&[manual.clone()], &disabled, &open(200), &[]).is_some());
        assert!(choose(&[manual], &policy, &snoozed, &[]).is_some());
        // A saved permission applies automatically without asking.
        let permitted = Policy {
            rules: vec![Rule {
                workload_id: case.proposal.unwrap().process.workload_id,
                label: "Example".into(),
            }],
            ..Default::default()
        };
        assert_eq!(choose(&cases, &permitted, &open(200), &[]), None);
    }

    #[test]
    fn asks_beat_progress_and_newest_wins() {
        let policy = Policy::default();
        let cases = [waiting(1, 100), waiting(3, 300), waiting(2, 200)];
        assert_eq!(
            choose(&cases, &policy, &open(400), &[]),
            Some(Choice { id: 3, more: 2 })
        );
        assert_eq!(
            choose(&cases, &policy, &open(400), &[3]),
            Some(Choice { id: 2, more: 1 })
        );
        let mut running = waiting(4, 500);
        running.transition(Stage::Investigating, "Investigating.");
        let mixed = [waiting(1, 100), running.clone()];
        assert_eq!(
            choose(&mixed, &policy, &open(600), &[]),
            Some(Choice { id: 1, more: 0 })
        );
        assert_eq!(
            choose(&[running], &policy, &open(600), &[]),
            Some(Choice { id: 4, more: 0 })
        );
    }

    #[test]
    fn payload_carries_no_paths_or_process_identity() {
        let mut case = waiting(1, 100);
        if let Some(proposal) = case.proposal.as_mut() {
            proposal.process.executable = Some("/private/example/tool".into());
            proposal.process.working_directory = Some("/private/example".into());
        }
        let text = view(&case, 2).to_string();
        for private in [
            "/private",
            "pid",
            "start_time",
            "rationale",
            "evidence",
            "reason",
        ] {
            assert!(!text.contains(private), "{private} leaked");
        }
        assert!(text.contains("\"more\":2"));
        assert!(text.contains("awaiting_approval"));
    }

    #[test]
    fn popup_sits_top_right_inside_the_work_area() {
        assert_eq!(
            corner([0., 25., 1440., 875.], [360., 216.], 12.),
            (1068., 37.)
        );
        assert_eq!(
            corner([-1920., 0., 1920., 1040.], [720., 432.], 24.),
            (-744., 24.)
        );
        assert_eq!(corner([0., 0., 300., 600.], [360., 216.], 12.), (0., 12.));
    }
}
