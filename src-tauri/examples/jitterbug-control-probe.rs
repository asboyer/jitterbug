//! Opt-in headless agent-control server for end-to-end checks without the desktop UI.
//!
//!   JITTERBUG_CONTROL_SOCKET=/abs/path.sock \
//!     cargo run --example jitterbug-control-probe -- --seconds 300 [--pause-without-asking]
//!
//! Runs the real collector, supervisor, and control socket against a temporary settings
//! directory, so `jitterbug-run control-mcp` clients can be exercised. Approval requests are
//! printed by title; `--approve` approves each request as it arrives (standing in for the
//! person's click). Everything paused is resumed on exit. Prints titles and counts only.
#[cfg(unix)]
use jitterbug_lib::{automation::Stage, collector, control, pause, settings, supervisor};
#[cfg(unix)]
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[cfg(not(unix))]
fn main() {
    eprintln!("Agent control is macOS and Linux only for now.");
}

#[cfg(unix)]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seconds: u64 = args
        .iter()
        .position(|a| a == "--seconds")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let approve = args.iter().any(|a| a == "--approve");
    let directory = std::env::temp_dir().join(format!("jitterbug-control-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("temporary directory");
    let mut settings = settings::Settings::default();
    settings.automation.agent_control = true;
    settings.automation.agent_pause_without_asking =
        args.iter().any(|a| a == "--pause-without-asking");
    // Only agent requests in this probe: no automatic incidents or model calls.
    settings.automation.enabled = false;
    pause::init(&directory);
    let requests: Arc<Mutex<Vec<u64>>> = Arc::default();
    let seen = requests.clone();
    let runtime = supervisor::Runtime::new(
        directory.clone(),
        settings,
        Arc::new(move |case| {
            if case.stage == Stage::AwaitingApproval {
                println!("request: {}", case.trigger.title);
                seen.lock().unwrap().push(case.id);
            } else if let Some(step) = case.steps.last() {
                println!("  {:?}: {}", case.stage, step.message);
            }
        }),
    );
    let socket = control::socket_path().expect("socket path");
    let handler_runtime = runtime.clone();
    control::server::serve(
        &socket,
        Arc::new(move |caller, method, params| {
            control::handlers::handle(&handler_runtime, caller, method, params)
        }),
    )
    .expect("control socket");
    println!("Serving agent control for {seconds} s.");
    let sampler = runtime.clone();
    std::thread::spawn(move || {
        let mut collector = collector::Collector::default();
        loop {
            let sample = collector.sample();
            pause::expire();
            let _ = sampler.observe(&sample);
            std::thread::sleep(Duration::from_secs(5));
        }
    });
    let started = Instant::now();
    let mut approved = HashSet::new();
    while started.elapsed() < Duration::from_secs(seconds) {
        std::thread::sleep(Duration::from_millis(500));
        if approve {
            let pending: Vec<u64> = requests.lock().unwrap().clone();
            for id in pending.into_iter().filter(|id| approved.insert(*id)) {
                let r = runtime.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = r.stop(id).await {
                        println!("  approval failed: {e}");
                    }
                });
            }
        }
    }
    println!("Paused at exit: {}", pause::list().len());
    pause::resume_all();
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_dir_all(&directory);
}
