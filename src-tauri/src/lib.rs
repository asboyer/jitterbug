pub mod actions;
pub mod applications;
pub mod attribution;
pub mod automation;
pub mod budget;
pub mod collector;
mod desktop;
pub mod details;
pub mod diagnostics;
pub mod harness;
pub mod platform;
pub mod pressure;
pub mod profiles;
pub mod providers;
pub mod settings;
pub mod storage;
pub mod supervisor;
pub mod types;
pub mod workflow_summary;
pub mod workflows;

use serde::Serialize;
use std::{sync::Arc, time::Duration};
use tauri::{Emitter, Manager, State};

pub type AppState = Arc<supervisor::Runtime>;

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
    let _guard = state
        .inference
        .try_lock()
        .map_err(|_| "One model request is already active. Please wait.")?;
    let snapshot = state
        .snapshot
        .read()
        .map_err(|_| "Telemetry unavailable.")?
        .clone();
    let detail = details::inspect(&snapshot, &workload_id, Some((pid, start_time)))?;
    if detail.profile.source == "reference" {
        return Ok(detail.profile);
    }
    if !detail.can_generate_profile {
        return Err("A reusable model explanation requires an application bundle or Windows executable. Use the contextual assistant to investigate this process instead.".into());
    }
    let app = detail
        .application
        .ok_or("Application identity unavailable.")?;
    let key = profiles::key(&app);
    if let Some(profile) = state
        .profiles
        .lock()
        .map_err(|_| "Profile cache unavailable.")?
        .get(&key)
    {
        return Ok(profile);
    }
    let settings = state
        .settings
        .lock()
        .map_err(|_| "Settings unavailable.")?
        .clone();
    let profile = state.engine.app_profile(&settings, &snapshot, &app).await?;
    state
        .profiles
        .lock()
        .map_err(|_| "Profile cache unavailable.")?
        .insert(key, profile.clone(), &state.directory)?;
    Ok(profile)
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
fn save_settings(
    state: State<'_, AppState>,
    mut settings: settings::Settings,
) -> Result<(), String> {
    let _guard = state
        .inference
        .try_lock()
        .map_err(|_| "Wait for the active model request before changing policy.")?;
    let mut current = state.settings.lock().map_err(|_| "Settings unavailable.")?;
    // Automation permissions have a separate command so an older provider form cannot overwrite them.
    settings.automation = current.automation.clone();
    settings.validate()?;
    storage::write(&state.directory.join("settings.json"), &settings)?;
    *current = settings;
    Ok(())
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
    state: State<'_, AppState>,
    policy: automation::Policy,
) -> Result<(), String> {
    state.save_policy(policy)
}

#[tauri::command]
fn snooze_automation(state: State<'_, AppState>, minutes: u64) -> Result<(), String> {
    state.snooze(minutes)
}

#[tauri::command]
fn dismiss_automation_case(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    state.dismiss(id)
}

#[tauri::command]
async fn approve_optimization(
    state: State<'_, AppState>,
    id: u64,
    remember: bool,
) -> Result<(), String> {
    state.inner().apply(id, true, remember).await
}

#[tauri::command]
fn simulate_pressure(app: tauri::AppHandle) -> Result<(), String> {
    notify_pressure(
        &app,
        "demo-pressure-incident",
        &pressure::simulated_incident(),
    )
}

fn notify_pressure(
    app: &tauri::AppHandle,
    event: &str,
    incident: &types::Incident,
) -> Result<(), String> {
    app.emit(event, incident)
        .map_err(|_| "Could not deliver the pressure incident.")?;
    if let Some(window) = app.get_webview_window("main") {
        if !window.is_focused().unwrap_or(false) {
            let _ = window.show();
            let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
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
        .setup(|app| {
            let directory = app.path().app_data_dir()?;
            let settings = storage::read::<settings::Settings>(&directory.join("settings.json"))?;
            settings.validate()?;
            let events = app.handle().clone();
            let runtime = supervisor::Runtime::new(
                directory,
                settings,
                Arc::new(move |case| {
                    let _ = events.emit("automation-case", &case);
                    if case.stage == automation::Stage::Detected {
                        if let Some(window) = events.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.request_user_attention(Some(
                                tauri::UserAttentionType::Informational,
                            ));
                        }
                    }
                }),
            );
            app.manage(runtime.clone());
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("jitterbug-collector".into())
                .spawn(move || {
                    let mut collector = collector::Collector::default();
                    loop {
                        let sample = collector.sample();
                        if let Ok(Some(id)) = runtime.observe(&sample) {
                            let task = runtime.clone();
                            tauri::async_runtime::spawn(async move {
                                task.run(id).await;
                            });
                        }
                        desktop::publish(&handle, &sample);
                        std::thread::sleep(Duration::from_secs(5));
                    }
                })?;
            desktop::install(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(focused) = event {
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
            get_snapshot,
            get_workload_details,
            get_workload_icon,
            explain_application,
            summarize_agent_task,
            get_agent_task_summary,
            get_quick_snapshot,
            desktop::open_main,
            desktop::hide_quick_view,
            get_bootstrap,
            save_settings,
            save_credential,
            list_local_models,
            unload_model,
            ask,
            process_action,
            simulate_pressure,
            save_automation_policy,
            snooze_automation,
            dismiss_automation_case,
            approve_optimization
        ])
        .build(tauri::generate_context!())
        .expect("Unable to start Jitterbug")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if matches!(event, tauri::RunEvent::Reopen { .. }) {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}
