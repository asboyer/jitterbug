pub mod actions;
pub mod agent_sessions;
pub mod applications;
pub mod attribution;
pub mod automation;
pub mod budget;
mod capture;
mod claude_sessions;
pub mod collector;
pub mod control;
mod desktop;
pub mod details;
pub mod diagnostics;
mod floating;
pub mod governor;
pub mod harness;
pub mod inside;
pub mod jitter;
mod lab;
pub mod origins;
pub mod pause;
pub mod platform;
mod popup;
pub mod pressure;
pub mod priorities;
pub mod probe;
pub mod profiles;
pub mod providers;
pub mod pulse;
pub mod queue;
pub mod research;
pub mod settings;
pub mod settings_history;
pub mod storage;
pub mod supervisor;
pub mod system_settings;
pub mod types;
pub mod workflow_summary;
pub mod workflows;

use serde::Serialize;
use std::{sync::Arc, time::Duration};
use tauri::{Emitter, Manager, State};

pub type AppState = Arc<supervisor::Runtime>;

#[tauri::command]
async fn discover_managed_workload(
    state: State<'_, AppState>,
    selection: queue::manage::discovery::Selection,
) -> Result<queue::manage::discovery::Discovery, String> {
    let snapshot = state
        .snapshot
        .read()
        .map_err(|_| "Telemetry unavailable.")?
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        queue::manage::discovery::discover(&snapshot, &selection)
    })
    .await
    .map_err(|_| "Discovery unavailable.")?
}

#[tauri::command]
async fn preview_managed_workload(
    request: queue::manage::Request,
) -> Result<queue::manage::Preview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        queue::manage::prepare(
            &queue::directory()?,
            &queue::manage::connections::Locations::current()?,
            &std::env::current_exe().map_err(|_| "Cannot locate the app.")?,
            request,
        )
    })
    .await
    .map_err(|_| "Setup unavailable.")?
}

#[tauri::command]
fn discard_managed_preview(token: String) {
    queue::manage::discard(&token);
}

#[tauri::command]
async fn approve_managed_workload(token: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || queue::manage::apply(&token))
        .await
        .map_err(|_| "Setup unavailable.")?
}

#[tauri::command]
async fn get_managed_workloads() -> Result<Vec<queue::manage::ManagedProfile>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        queue::manage::overview(
            &queue::directory()?,
            &queue::manage::connections::Locations::current()?,
        )
    })
    .await
    .map_err(|_| "Setup unavailable.")?
}

#[tauri::command]
async fn run_managed_workload(name: String) -> Result<queue::JobView, String> {
    tauri::async_runtime::spawn_blocking(move || queue::manage::submit_profile(&name))
        .await
        .map_err(|_| "Queue unavailable.")?
}

#[tauri::command]
async fn revoke_managed_workload(name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        queue::manage::revoke_profile(&queue::directory()?, &name)
    })
    .await
    .map_err(|_| "Queue unavailable.")?
}

#[tauri::command]
async fn get_validation_queue() -> Result<queue::View, String> {
    tauri::async_runtime::spawn_blocking(|| queue::inspect(&queue::directory()?).map(|s| s.view()))
        .await
        .map_err(|_| "Queue service unavailable.")?
}

#[tauri::command]
async fn control_validation_queue(action: String, job_id: Option<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let directory = queue::directory()?;
        match action.as_str() {
            "pause" | "resume" => queue::transaction(&directory, |s| {
                s.paused = action == "pause";
                Ok(())
            }),
            "cancel" => queue::cancel(&directory, job_id.as_deref().ok_or("Missing job ID.")?),
            "release" => {
                queue::acknowledge(&directory, job_id.as_deref().ok_or("Missing job ID.")?)
            }
            _ => queue::control::change(&directory, &action, job_id.as_deref()),
        }
    })
    .await
    .map_err(|_| "Queue service unavailable.")?
}

#[tauri::command]
async fn plan_workflow_queue(state: State<'_, AppState>) -> Result<queue::control::Plan, String> {
    let _guard = state
        .inference
        .try_lock()
        .map_err(|_| "One model request is already active.")?;
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Settings unavailable.")?
        .clone();
    let snapshot = state
        .snapshot
        .read()
        .map_err(|_| "Telemetry unavailable.")?
        .clone();
    if snapshot.source != "live"
        || !snapshot.sample_ready
        || snapshot.captured_at > types::now().saturating_add(2)
        || snapshot.captured_at.saturating_add(15) < types::now()
    {
        return Err("Fresh telemetry is required to plan work.".into());
    }
    let queue = tauri::async_runtime::spawn_blocking(|| queue::inspect(&queue::directory()?))
        .await
        .map_err(|_| "Queue unavailable.")??;
    if !queue
        .jobs
        .iter()
        .any(|job| job.stage == queue::Stage::Queued)
    {
        return Err("No upcoming checks need a plan.".into());
    }
    state
        .engine
        .workflow_plan(&settings, &snapshot, &queue)
        .await
}

#[tauri::command]
fn search_workloads(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<types::Workload>, String> {
    let snapshot = state
        .snapshot
        .read()
        .map_err(|_| "Telemetry unavailable.")?;
    details::search(&snapshot, &query)
}

#[tauri::command]
fn get_workload_details(
    state: State<'_, AppState>,
    workload_id: String,
    pid: Option<u32>,
    start_time: Option<u64>,
) -> Result<details::Details, String> {
    let selected = match (pid, start_time) {
        (None, None) => None,
        (Some(pid), Some(start)) => Some((pid, start)),
        _ => return Err("A process ID and start time are required together.".into()),
    };
    let mut detail = details::inspect(
        &*state
            .snapshot
            .read()
            .map_err(|_| "Telemetry unavailable.")?,
        &workload_id,
        selected,
    )?;
    if detail.profile.source != "reference" {
        if let Some(app) = &detail.application {
            if let Some(profile) = state
                .profiles
                .lock()
                .map_err(|_| "Profile cache unavailable.")?
                .get(&profiles::key(app))
            {
                detail.profile = profile;
            }
        }
    }
    Ok(detail)
}

#[tauri::command]
async fn get_inside_view(
    state: State<'_, AppState>,
    workload_id: String,
    connect: Option<bool>,
) -> Result<inside::Inside, String> {
    let snapshot = state
        .snapshot
        .read()
        .map_err(|_| "Telemetry unavailable.")?
        .clone();
    // Automatic loads pass `connect: false`: only an explicit request may make a browser ask.
    let connect = connect.unwrap_or(true);
    tauri::async_runtime::spawn_blocking(move || {
        inside::inspect_with(&snapshot, &workload_id, connect)
    })
    .await
    .map_err(|_| "Inside view unavailable.")?
}

/// Every command session inside terminals, editors, and agent apps, agents first.
#[tauri::command]
async fn get_agent_sessions(
    state: State<'_, AppState>,
) -> Result<agent_sessions::Overview, String> {
    let snapshot = state
        .snapshot
        .read()
        .map_err(|_| "Telemetry unavailable.")?
        .clone();
    tauri::async_runtime::spawn_blocking(move || agent_sessions::overview(&snapshot))
        .await
        .map_err(|_| "Agent sessions unavailable.".into())
}

/// Open a new terminal window running an allowlisted agent in a folder.
#[tauri::command]
async fn launch_agent_session(request: agent_sessions::LaunchRequest) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || agent_sessions::launch(&request))
        .await
        .map_err(|_| "Launching is unavailable.")?
}

#[tauri::command]
async fn open_folder_in_vscode(folder: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || agent_sessions::open_in_vscode(&folder))
        .await
        .map_err(|_| "VS Code is unavailable.")?
}

#[tauri::command]
async fn tab_action(target_id: String, url: String, action: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || inside::chrome::act(&target_id, &url, &action))
        .await
        .map_err(|_| "Tab action unavailable.")?
}

#[tauri::command]
async fn get_workload_icon(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workload_id: String,
) -> Result<Option<String>, String> {
    let path = {
        let snapshot = state
            .snapshot
            .read()
            .map_err(|_| "Telemetry unavailable.")?;
        let members = details::members(&snapshot, &workload_id)?;
        if snapshot
            .workloads
            .iter()
            .any(|w| w.id == workload_id && w.category == "project")
        {
            return Ok(None);
        }
        details::representative(&members)
            .executable
            .as_ref()
            .map(|path| {
                crate::attribution::application_bundle(path)
                    .unwrap_or(path)
                    .to_path_buf()
            })
    };
    let Some(path) = path else { return Ok(None) };
    let runtime = state.inner().clone();
    #[cfg(target_os = "macos")]
    {
        let (send, receive) = tokio::sync::oneshot::channel();
        // AppKit drawing stays on the UI thread; frontend limits icon requests to two at a time.
        app.run_on_main_thread(move || {
            let _ = send.send(runtime.icons.get(&path));
        })
        .map_err(|_| "Icon service unavailable.")?;
        receive.await.map_err(|_| "Icon service closed.".into())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        // Shell icon handlers may block. Keep them off the Windows UI thread.
        tauri::async_runtime::spawn_blocking(move || runtime.icons.get(&path))
            .await
            .map_err(|_| "Icon service unavailable.".into())
    }
}

#[tauri::command]
async fn explain_application(
    state: State<'_, AppState>,
    workload_id: String,
    pid: u32,
    start_time: u64,
) -> Result<profiles::Profile, String> {
    state
        .explain_application(&workload_id, pid, start_time)
        .await
}

#[tauri::command]
fn get_agent_task_summary(
    state: State<'_, AppState>,
    task_id: String,
    task_key: String,
) -> Result<Option<profiles::Profile>, String> {
    let snapshot = state
        .snapshot
        .read()
        .map_err(|_| "Telemetry unavailable.")?;
    if !snapshot
        .agent_workflows
        .iter()
        .any(|w| w.task_id == task_id && w.narrative.task_key == task_key)
    {
        return Ok(None);
    }
    Ok(state
        .task_summaries
        .lock()
        .map_err(|_| "Task cache unavailable.")?
        .get(&task_key))
}

#[tauri::command]
async fn summarize_agent_task(
    state: State<'_, AppState>,
    task_id: String,
    task_key: String,
) -> Result<profiles::Profile, String> {
    let _guard = state
        .inference
        .try_lock()
        .map_err(|_| "One model request is already active. Please wait.")?;
    let snapshot = state
        .snapshot
        .read()
        .map_err(|_| "Telemetry unavailable.")?
        .clone();
    if snapshot.source != "live" {
        return Err("Task summaries require a real registered task.".into());
    }
    let workflow = snapshot
        .agent_workflows
        .iter()
        .find(|w| w.task_id == task_id)
        .ok_or("This task is no longer available.")?;
    if workflow.narrative.task_key != task_key {
        return Err("The task request changed. Review the latest excerpts and try again.".into());
    }
    if let Some(profile) = state
        .task_summaries
        .lock()
        .map_err(|_| "Task cache unavailable.")?
        .get(&task_key)
    {
        return Ok(profile);
    }
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Settings unavailable.")?
        .clone();
    let profile = state
        .engine
        .task_summary(&settings, &snapshot, workflow)
        .await?;
    state
        .task_summaries
        .lock()
        .map_err(|_| "Task cache unavailable.")?
        .insert_memory(task_key, profile.clone());
    Ok(profile)
}

#[derive(Serialize)]
pub struct Bootstrap {
    settings: settings::Settings,
    credentials: Vec<String>,
    spending: budget::Ledger,
    automation_cases: Vec<automation::Case>,
    automation_snoozed_until: u64,
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> Result<types::Snapshot, String> {
    state
        .snapshot
        .read()
        .map(|s| s.dashboard_view())
        .map_err(|_| "Telemetry state unavailable.".into())
}

#[tauri::command]
fn get_quick_snapshot(state: State<'_, AppState>) -> Result<types::Snapshot, String> {
    state
        .snapshot
        .read()
        .map(|s| s.quick_view())
        .map_err(|_| "Telemetry unavailable.".into())
}

#[tauri::command]
fn get_bootstrap(state: State<'_, AppState>) -> Result<Bootstrap, String> {
    Ok(Bootstrap {
        settings: state
            .settings
            .lock()
            .map_err(|_| "Settings unavailable.")?
            .clone(),
        credentials: [
            settings::Provider::Meta,
            settings::Provider::Anthropic,
            settings::Provider::Jev,
        ]
        .iter()
        .filter(|p| settings::key(p).is_ok())
        .map(|p| p.name().into())
        .collect(),
        spending: state.engine.spending()?,
        automation_cases: state.cases()?,
        automation_snoozed_until: state.snoozed_until()?,
    })
}

#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: settings::Settings) -> Result<(), String> {
    state.save_settings(settings)
}

#[tauri::command]
fn set_settings_default(
    state: State<'_, AppState>,
) -> Result<settings_history::SnapshotView, String> {
    state.set_settings_default()
}

#[tauri::command]
fn get_settings_default_history(
    state: State<'_, AppState>,
    cursor: Option<String>,
    page_size: usize,
) -> Result<settings_history::HistoryPage, String> {
    state.settings_history(cursor.as_deref(), page_size)
}

#[tauri::command]
fn inspect_settings_default(
    state: State<'_, AppState>,
    id: String,
) -> Result<settings_history::SnapshotView, String> {
    state.inspect_settings_default(&id)
}

#[tauri::command]
fn preview_settings_restore(
    state: State<'_, AppState>,
    id: String,
) -> Result<settings_history::RestorePreview, String> {
    state.preview_settings_restore(&id)
}

#[tauri::command]
fn cancel_settings_restore(state: State<'_, AppState>, token: String) -> Result<(), String> {
    state.cancel_settings_restore(&token)
}

#[tauri::command]
async fn confirm_settings_restore(
    state: State<'_, AppState>,
    token: String,
    confirmed: bool,
) -> Result<settings::Settings, String> {
    if !confirmed {
        let _ = state.cancel_settings_restore(&token);
        return Err("Explicit restore confirmation is required.".into());
    }
    state.confirm_settings_restore(&token).await
}

#[tauri::command]
fn get_system_capabilities(state: State<'_, AppState>) -> Vec<system_settings::Capability> {
    state.system_capabilities()
}

#[tauri::command]
fn open_system_settings_pane(pane: String) -> Result<(), String> {
    let url = system_settings::settings_url(&pane)?;
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSString, NSURL};
        let url = NSURL::URLWithString(&NSString::from_str(url)).ok_or("Invalid settings URL.")?;
        if objc2_app_kit::NSWorkspace::sharedWorkspace().openURL(&url) {
            Ok(())
        } else {
            Err(
                "macOS could not open this System Settings pane. Open System Settings manually."
                    .into(),
            )
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = url;
        Err("System Settings destinations are supported on macOS only.".into())
    }
}

#[tauri::command]
fn set_system_default(
    state: State<'_, AppState>,
    groups: Option<Vec<String>>,
) -> Result<system_settings::SnapshotView, String> {
    state.set_system_default_selected(groups.as_deref())
}

#[tauri::command]
fn get_system_default_history(
    state: State<'_, AppState>,
    cursor: Option<String>,
    page_size: usize,
) -> Result<system_settings::HistoryPage, String> {
    state.system_history(cursor.as_deref(), page_size)
}

#[tauri::command]
fn inspect_system_default(
    state: State<'_, AppState>,
    id: String,
) -> Result<system_settings::SnapshotView, String> {
    state.inspect_system_default(&id)
}

#[tauri::command]
fn preview_system_restore(
    state: State<'_, AppState>,
    id: String,
) -> Result<system_settings::RestorePreview, String> {
    state.preview_system_restore(&id)
}

#[tauri::command]
fn cancel_system_restore(state: State<'_, AppState>, token: String) -> Result<(), String> {
    state.cancel_system_restore(&token)
}

#[tauri::command]
async fn confirm_system_restore(
    state: State<'_, AppState>,
    token: String,
    confirmed: bool,
) -> Result<system_settings::RollbackResult, String> {
    if !confirmed {
        let _ = state.cancel_system_restore(&token);
        return Err("Explicit rollback confirmation is required.".into());
    }
    state.confirm_system_restore(&token).await
}

#[tauri::command]
fn save_credential(provider: settings::Provider, credential: String) -> Result<(), String> {
    settings::save_key(provider, &credential)
}

#[tauri::command]
async fn list_local_models(
    state: State<'_, AppState>,
) -> Result<Vec<providers::LocalModel>, String> {
    state.engine.local_models().await
}

#[tauri::command]
async fn unload_model(state: State<'_, AppState>, model: String) -> Result<String, String> {
    let _guard = state
        .inference
        .try_lock()
        .map_err(|_| "A model request is still active.")?;
    state.engine.unload(&model).await
}

#[tauri::command]
async fn ask(
    state: State<'_, AppState>,
    messages: Vec<providers::ChatMessage>,
    workload_id: Option<String>,
    tier: String,
    pid: Option<u32>,
    start_time: Option<u64>,
) -> Result<providers::Reply, String> {
    let _guard = state
        .inference
        .try_lock()
        .map_err(|_| "One model request is already active. Please wait.")?;
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Settings unavailable.")?
        .clone();
    let snapshot = state
        .snapshot
        .read()
        .map_err(|_| "Telemetry unavailable.")?
        .clone();
    if !snapshot.sample_ready || snapshot.captured_at + 20 < types::now() {
        return Err("Wait for a fresh telemetry sample.".into());
    }
    if let Some(pid) = pid {
        if !snapshot.processes.iter().any(|p| {
            p.pid == pid
                && Some(p.start_time) == start_time
                && workload_id.as_ref().is_some_and(|id| p.workload_id == *id)
        }) {
            return Err(
                "The selected process has exited or changed identity. Choose a current process."
                    .into(),
            );
        }
    }
    state
        .engine
        .chat(
            &settings,
            &snapshot,
            messages,
            workload_id.as_deref(),
            &tier,
            pid.zip(start_time),
        )
        .await
}

#[tauri::command]
async fn process_action(
    state: State<'_, AppState>,
    pid: u32,
    start_time: u64,
    action: String,
) -> Result<types::ActionReport, String> {
    state.inner().process_action(pid, start_time, &action).await
}

#[tauri::command]
fn save_automation_policy(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    jitter: State<'_, capture::Jitter>,
    policy: automation::Policy,
) -> Result<(), String> {
    let mode = policy.jitter_mode;
    state.save_policy(policy)?;
    jitter.pulse.set_mode(mode);
    popup::refresh(&app);
    Ok(())
}

#[tauri::command]
fn snooze_automation(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    minutes: u64,
) -> Result<(), String> {
    state.snooze(minutes)?;
    popup::refresh(&app);
    Ok(())
}

#[tauri::command]
fn dismiss_automation_case(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    state.dismiss(id)
}

/// Stops only the process the incident offered, after the same identity and policy rechecks.
/// With `fallback`, applies the firmer option offered after it (closing a tab instead of
/// pausing it).
#[tauri::command]
async fn approve_stop(
    state: State<'_, AppState>,
    id: u64,
    fallback: Option<bool>,
) -> Result<(), String> {
    state
        .inner()
        .stop_option(id, fallback.unwrap_or(false))
        .await
}

#[tauri::command]
async fn resume_case(state: State<'_, AppState>, id: u64) -> Result<String, String> {
    // Resuming scans the process table; keep it off the main thread.
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || runtime.resume(id))
        .await
        .map_err(|_| "Resume unavailable.")?
}

/// Pause or resume one command session from the inside view. Pausing rechecks ownership.
#[tauri::command]
async fn session_control(
    pid: u32,
    start_time: u64,
    action: String,
    label: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || match action.as_str() {
        "pause" => pause::pause(pid, start_time, &label).map(|p| {
            format!(
                "Paused {} ({} processes). It resumes automatically after {} minutes.",
                p.label,
                p.members.len(),
                pause::LIMIT_SECONDS / 60
            )
        }),
        "resume" => pause::resume(pid, start_time),
        _ => Err("Unknown session action.".into()),
    })
    .await
    .map_err(|_| "Session control unavailable.")?
}

/// Approving a priority change moves the app to the Background bucket.
#[tauri::command]
async fn approve_optimization(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    state.inner().apply(id, true).await
}

/// Apps per priority bucket, with when and by whom they were moved. No process identity.
#[tauri::command]
fn get_priority_buckets(state: State<'_, AppState>) -> Result<supervisor::Priorities, String> {
    state.priorities()
}

/// Move an app to Protected, Normal, or Background, applied at once.
#[tauri::command]
async fn move_to_bucket(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workload_id: String,
    bucket: automation::Bucket,
) -> Result<String, String> {
    let result = state
        .inner()
        .move_blocking(&workload_id, bucket, automation::AssignedBy::You)
        .await;
    popup::refresh(&app);
    result
}

/// Popup Undo: the app this incident moved to Background goes back to Normal.
#[tauri::command]
async fn undo_background(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: u64,
) -> Result<String, String> {
    let runtime = state.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || runtime.undo_background(id))
        .await
        .map_err(|_| "Undo unavailable.")?;
    popup::refresh(&app);
    result
}

#[tauri::command]
fn simulate_pressure(app: tauri::AppHandle) -> Result<(), String> {
    publish_automation_case(
        &app,
        "demo-pressure-case",
        &automation::simulated_case(types::now())?,
    )
}

fn publish_automation_case(
    app: &tauri::AppHandle,
    event: &str,
    case: &automation::Case,
) -> Result<(), String> {
    app.emit(event, case)
        .map_err(|_| "Could not deliver the automatic-assistance case.")?;
    popup::refresh(app);
    // With popups on, the popup is the cue; raising the main window would pull the user away.
    let popups = app
        .try_state::<AppState>()
        .and_then(|state| {
            state
                .settings
                .lock()
                .ok()
                .map(|s| s.automation.action_popups)
        })
        .unwrap_or(false);
    if case.stage == automation::Stage::Detected && !popups {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            if !window.is_focused().unwrap_or(false) {
                let _ =
                    window.request_user_attention(Some(tauri::UserAttentionType::Informational));
            }
        }
    }
    Ok(())
}

pub fn load_development_env() {
    #[cfg(debug_assertions)]
    {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.env");
        let _ = dotenvy::from_path(path);
    }
}

pub fn run() {
    load_development_env();
    tauri::Builder::default()
        .plugin(capture::plugin())
        .setup(|app| {
            let directory = app.path().app_data_dir()?;
            let settings = storage::read::<settings::Settings>(&directory.join("settings.json"))?;
            settings.validate()?;
            // Anything a previous run left paused (a crash or forced quit) resumes now.
            pause::init(&directory);
            let jitter = capture::Service::new(directory.clone(), settings.automation.jitter_mode);
            app.manage(jitter.clone());
            app.manage(popup::Popups::default());
            let events = app.handle().clone();
            let runtime = supervisor::Runtime::new(
                directory,
                settings,
                Arc::new(move |case| {
                    let _ = publish_automation_case(&events, "automation-case", &case);
                }),
            );
            app.manage(runtime.clone());
            // Agent control is optional: a failure leaves monitoring and the app unaffected.
            #[cfg(unix)]
            if let Ok(path) = control::socket_path() {
                let control = runtime.clone();
                if let Err(error) = control::server::serve(
                    &path,
                    Arc::new(move |caller, method, params| {
                        control::handlers::handle(&control, caller, method, params)
                    }),
                ) {
                    eprintln!("Agent control unavailable: {error}");
                }
            }
            capture::install(app, runtime.clone(), jitter.clone());
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("jitterbug-collector".into())
                .spawn(move || {
                    let mut collector = collector::Collector::default();
                    let mut last_workflow_notice = types::now();
                    loop {
                        let mut sample = collector.sample();
                        sample.frontmost_workload = crate::platform::frontmost()
                            .filter(|(pid, _)| *pid != std::process::id())
                            .and_then(|(pid, _)| sample.processes.iter().find(|p| p.pid == pid))
                            .map(|p| p.workload_id.clone());
                        pause::expire();
                        inside::chrome::expire();
                        if let (Ok(directory), Ok(executable)) =
                            (queue::directory(), std::env::current_exe())
                        {
                            // Queue failure never stops resource monitoring. Clients fail closed
                            // when this heartbeat is absent; the UI reports unreadable state.
                            if queue::tick(&directory, &sample, &executable).is_ok() {
                                if let Ok(queue) = queue::inspect(&directory) {
                                    if queue.control.notice_at > last_workflow_notice {
                                        last_workflow_notice = queue.control.notice_at;
                                        if runtime.notifications_allowed() {
                                            let _ = handle.emit(
                                                "workflow-attention",
                                                queue.control.notice_at,
                                            );
                                            if let Some(window) = handle.get_webview_window("main")
                                            {
                                                let _ = window.show();
                                                let _ = window.request_user_attention(Some(
                                                    tauri::UserAttentionType::Informational,
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // The one-second pulse watches the busiest PIDs of each full sample.
                        jitter.pulse.watch(&sample);
                        if let Ok(view) = runtime.govern(&sample) {
                            sample.governor = view;
                        }
                        if let Ok(Some(id)) = runtime.observe(&sample) {
                            capture::automatic(&handle, id);
                        }
                        // Background apps' new processes, and the app now in front.
                        let _ = runtime.enforce_buckets(&sample);
                        desktop::publish(&handle, &sample);
                        std::thread::sleep(Duration::from_secs(5));
                    }
                })?;
            desktop::install(app)?;
            #[cfg(debug_assertions)]
            lab::open(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(focused) = event {
                // In-app prompts take over while the main window is in front.
                if window.label() == "main" && *focused {
                    popup::refresh(window.app_handle());
                }
                if window.label() == "menubar" && !focused {
                    let _ = window.hide();
                } else if *focused {
                    let state = window.state::<AppState>();
                    if let Ok(snapshot) = state.snapshot.read() {
                        desktop::publish(window.app_handle(), &snapshot);
                    };
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            discover_managed_workload,
            discard_managed_preview,
            preview_managed_workload,
            approve_managed_workload,
            get_managed_workloads,
            run_managed_workload,
            revoke_managed_workload,
            get_validation_queue,
            plan_workflow_queue,
            control_validation_queue,
            get_snapshot,
            search_workloads,
            get_workload_details,
            get_workload_icon,
            get_inside_view,
            get_agent_sessions,
            launch_agent_session,
            open_folder_in_vscode,
            tab_action,
            explain_application,
            summarize_agent_task,
            get_agent_task_summary,
            get_quick_snapshot,
            desktop::open_main,
            desktop::hide_quick_view,
            get_bootstrap,
            save_settings,
            set_settings_default,
            get_settings_default_history,
            inspect_settings_default,
            preview_settings_restore,
            cancel_settings_restore,
            confirm_settings_restore,
            get_system_capabilities,
            open_system_settings_pane,
            set_system_default,
            get_system_default_history,
            inspect_system_default,
            preview_system_restore,
            cancel_system_restore,
            confirm_system_restore,
            save_credential,
            list_local_models,
            unload_model,
            ask,
            process_action,
            simulate_pressure,
            save_automation_policy,
            snooze_automation,
            dismiss_automation_case,
            approve_optimization,
            approve_stop,
            resume_case,
            get_priority_buckets,
            move_to_bucket,
            undo_background,
            session_control,
            capture::report_jitter,
            capture::get_jitter_tally,
            capture::clear_jitter_tally,
            popup::get_action_popup,
            popup::close_action_popup,
            popup::open_action_case,
            popup::preview_action_popup,
            lab::lab_mcp
        ])
        .build(tauri::generate_context!())
        .expect("Unable to start Jitterbug")
        .run(|app, event| {
            // Never leave a paused command frozen after Jitterbug exits.
            if matches!(event, tauri::RunEvent::Exit) {
                pause::resume_all();
            }
            #[cfg(target_os = "macos")]
            if matches!(event, tauri::RunEvent::Reopen { .. }) {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = app;
        });
}
