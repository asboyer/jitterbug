//! Desktop glue for Jitter Snapshot: the global shortcut, tray and quick-view triggers,
//! immediate feedback, and the hand-off from a frozen capture to the incident runtime.
use crate::{jitter, platform, pulse, AppState};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

pub type Jitter = Arc<Service>;

pub struct Service {
    pub pulse: pulse::Pulse,
    tally: Mutex<jitter::Tally>,
    directory: PathBuf,
    /// The app in front just before Jitterbug's own quick view took focus.
    remembered: Mutex<Option<(jitter::Victim, Instant)>>,
}

impl Service {
    pub fn new(directory: PathBuf, mode: pulse::Mode) -> Jitter {
        let service = Arc::new(Self {
            pulse: pulse::Pulse::new(),
            tally: Mutex::new(jitter::Tally::read(&directory)),
            directory,
            remembered: Mutex::new(None),
        });
        service.pulse.set_mode(mode);
        service
    }
    fn victim(&self) -> Option<jitter::Victim> {
        match platform::frontmost() {
            Some((pid, name)) if pid != std::process::id() => Some(jitter::Victim { pid, name }),
            _ => self
                .remembered
                .lock()
                .ok()?
                .clone()
                .filter(|(_, at)| at.elapsed() < Duration::from_secs(120))
                .map(|(victim, _)| victim),
        }
    }
}

#[derive(Clone, Serialize)]
struct Feedback {
    phase: &'static str,
    message: String,
}

fn shortcut() -> Shortcut {
    let primary = if cfg!(target_os = "macos") {
        Modifiers::SUPER
    } else {
        Modifiers::CONTROL
    };
    Shortcut::new(Some(primary | Modifiers::SHIFT), Code::KeyJ)
}

/// Registered from Rust only; the webview gets no global-shortcut permission.
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, pressed, event| {
            if event.state() == ShortcutState::Pressed && *pressed == shortcut() {
                if let Err(error) = trigger(app) {
                    feedback(app, "error", &error);
                }
            }
        })
        .build()
}

pub fn install(app: &tauri::App, runtime: AppState, service: Jitter) {
    if let Err(error) = app.global_shortcut().register(shortcut()) {
        // Another app may own the shortcut; the tray and quick-view triggers still work.
        eprintln!("Jitter Snapshot shortcut unavailable: {error}");
    }
    service.pulse.set_spike_handler(Arc::new(move |capture| {
        if let Ok(Some(id)) = runtime.spike(capture) {
            let task = runtime.clone();
            tauri::async_runtime::spawn(async move { task.run(id).await });
        }
    }));
}

/// Remember the frontmost app before the quick view activates Jitterbug.
pub fn remember_frontmost(app: &AppHandle) {
    let Some(service) = app.try_state::<Jitter>() else {
        return;
    };
    if let Some((pid, name)) = platform::frontmost().filter(|(pid, _)| *pid != std::process::id()) {
        if let Ok(mut remembered) = service.remembered.lock() {
            *remembered = Some((jitter::Victim { pid, name }, Instant::now()));
        }
    }
}

fn feedback(app: &AppHandle, phase: &'static str, message: &str) {
    let _ = app.emit(
        "jitter-capture",
        Feedback {
            phase,
            message: message.into(),
        },
    );
    // The popup shows the capture over whatever app is in front.
    crate::popup::capturing(app, (phase == "recording").then(|| message.to_string()));
    // The menu-bar title is the immediate cue when no window is visible (macOS only).
    if let Some(tray) = app.tray_by_id("jitterbug") {
        let recording = phase == "recording";
        let _ = tray.set_title(recording.then_some("Capturing…"));
        let _ = tray.set_tooltip(Some(if recording {
            "Jitterbug — capturing a Jitter Snapshot"
        } else {
            "Jitterbug — resource assistant"
        }));
    }
}

/// Start a snapshot from any trigger. Returns at once; analysis follows the post-roll.
pub fn trigger(app: &AppHandle) -> Result<String, String> {
    let service = app.state::<Jitter>().inner().clone();
    let runtime = app.state::<AppState>().inner().clone();
    let press = pulse::now_ms();
    let victim = service.victim();
    let wait = service.pulse.press()?;
    let message = format!("Recording {} more seconds, then analyzing.", wait.as_secs());
    feedback(app, "recording", &message);
    let handle = app.clone();
    let worker = service.clone();
    let started = std::thread::Builder::new()
        .name("jitterbug-snapshot".into())
        .spawn(move || {
            std::thread::sleep(wait);
            let capture = worker.pulse.freeze(press, victim);
            let result = match worker.tally.lock() {
                Ok(mut tally) => runtime.report(capture, &mut tally),
                Err(_) => Err("Snapshot history unavailable.".into()),
            };
            match result {
                Ok((id, rerun)) => {
                    // You asked: show this case again even if its popup was put off earlier.
                    crate::popup::reopen(&handle, id);
                    feedback(&handle, "analyzing", "Captured, analyzing…");
                    if rerun {
                        tauri::async_runtime::spawn(async move { runtime.run(id).await });
                    }
                }
                Err(error) => feedback(&handle, "error", &error),
            }
        });
    if started.is_err() {
        service.pulse.release();
        return Err("Could not start the snapshot.".into());
    }
    Ok(message)
}

/// The monitor opened a case: capture the surrounding seconds before investigating, so
/// automatic incidents carry the same snapshot evidence as a manual press.
pub fn automatic(app: &AppHandle, id: u64) {
    let runtime = app.state::<AppState>().inner().clone();
    let investigate = move |runtime: AppState| {
        tauri::async_runtime::spawn(async move { runtime.run(id).await });
    };
    let Some(service) = app.try_state::<Jitter>().map(|s| s.inner().clone()) else {
        return investigate(runtime);
    };
    // Automatic snapshots are opt-in. Snapshot-only mode keeps no look-back, and a manual
    // capture may already be recording.
    let wanted = runtime
        .settings
        .lock()
        .is_ok_and(|s| s.automation.auto_snapshots);
    let wait = (wanted && service.pulse.mode() == pulse::Mode::Auto && service.pulse.is_running())
        .then(|| service.pulse.press().ok())
        .flatten();
    let Some(wait) = wait else {
        return investigate(runtime);
    };
    let press = pulse::now_ms();
    let victim = service.victim();
    let message = format!(
        "Something looks off. Recording {} more seconds, then analyzing.",
        wait.as_secs()
    );
    feedback(app, "recording", &message);
    let handle = app.clone();
    let worker = service.clone();
    let task = runtime.clone();
    let started = std::thread::Builder::new()
        .name("jitterbug-auto-snapshot".into())
        .spawn(move || {
            std::thread::sleep(wait);
            let capture = worker.pulse.freeze(press, victim);
            // A failed attach still investigates with the regular evidence.
            let _ = task.attach_snapshot(id, capture);
            feedback(&handle, "analyzing", "Captured, analyzing…");
            investigate(task);
        });
    if started.is_err() {
        service.pulse.release();
        feedback(app, "error", "Could not start the snapshot.");
        investigate(runtime);
    }
}

#[tauri::command]
pub fn report_jitter(app: AppHandle) -> Result<String, String> {
    trigger(&app)
}

#[derive(Serialize)]
pub struct TallyView {
    entries: Vec<jitter::TallyEntry>,
    mode: pulse::Mode,
    sampling: bool,
}

#[tauri::command]
pub fn get_jitter_tally(service: State<'_, Jitter>) -> Result<TallyView, String> {
    Ok(TallyView {
        entries: service
            .tally
            .lock()
            .map_err(|_| "Snapshot history unavailable.")?
            .entries
            .clone(),
        mode: service.pulse.mode(),
        sampling: service.pulse.is_running(),
    })
}

#[tauri::command]
pub fn clear_jitter_tally(service: State<'_, Jitter>) -> Result<(), String> {
    let mut tally = service
        .tally
        .lock()
        .map_err(|_| "Snapshot history unavailable.")?;
    tally.clear();
    tally.save(&service.directory)
}
