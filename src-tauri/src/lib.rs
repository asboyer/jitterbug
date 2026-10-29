pub mod actions;
pub mod attribution;
pub mod budget;
pub mod collector;
pub mod platform;
pub mod pressure;
pub mod providers;
pub mod settings;
pub mod storage;
pub mod types;

use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
use tauri::{Emitter, Manager, State};

pub struct AppState {
    snapshot: Arc<RwLock<types::Snapshot>>,
    settings: Mutex<settings::Settings>,
    engine: providers::Engine,
    inference: tokio::sync::Mutex<()>,
    directory: PathBuf,
}

#[derive(Serialize)]
pub struct Bootstrap {
    settings: settings::Settings,
    credentials: Vec<String>,
    spending: budget::Ledger,
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> Result<types::Snapshot, String> {
    state
        .snapshot
        .read()
        .map(|s| s.clone())
        .map_err(|_| "Telemetry state unavailable.".into())
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
    })
}

#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: settings::Settings) -> Result<(), String> {
    let _guard = state
        .inference
        .try_lock()
        .map_err(|_| "Wait for the active model request before changing policy.")?;
    settings.validate()?;
    let mut current = state.settings.lock().map_err(|_| "Settings unavailable.")?;
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
    if workload_id
        .as_ref()
        .is_some_and(|id| !snapshot.workloads.iter().any(|w| &w.id == id))
    {
        return Err(
            "This workload is no longer in the current sample. Select a current workload.".into(),
        );
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
async fn process_action(pid: u32, start_time: u64, action: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || actions::execute(pid, start_time, &action))
        .await
        .map_err(|_| "Process action failed.")?
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
            let snapshot = Arc::new(RwLock::new(types::Snapshot::default()));
            app.manage(AppState {
                snapshot: snapshot.clone(),
                settings: Mutex::new(settings),
                engine: providers::Engine::new(&directory),
                inference: tokio::sync::Mutex::new(()),
                directory,
            });
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("jitterbug-collector".into())
                .spawn(move || {
                    let mut collector = collector::Collector::default();
                    let mut last_incident = None;
                    loop {
                        let sample = collector.sample();
                        if let Some(incident) = &sample.incident {
                            if last_incident != Some(incident.id) {
                                let _ = notify_pressure(&handle, "pressure-incident", incident);
                                last_incident = Some(incident.id);
                            }
                        }
                        if let Ok(mut state) = snapshot.write() {
                            *state = sample.clone();
                        }
                        let _ = handle.emit("telemetry", &sample);
                        std::thread::sleep(Duration::from_secs(5));
                    }
                })?;
            let show =
                tauri::menu::MenuItem::with_id(app, "show", "Open Jitterbug", true, None::<&str>)?;
            let quit =
                tauri::menu::MenuItem::with_id(app, "quit", "Quit Jitterbug", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&show, &quit])?;
            tauri::tray::TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Jitterbug · resource assistant")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => (),
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_bootstrap,
            save_settings,
            save_credential,
            list_local_models,
            unload_model,
            ask,
            process_action,
            simulate_pressure
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
