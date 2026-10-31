//! Opt-in live check of the agent governor: the real sampling loop with pausing turned on.
//!
//!   cargo run --example jitterbug-governor-probe -- --seconds 180
//!
//! Samples every five seconds like the app, lets the governor pause and resume real agent
//! sessions, and prints each sample's CPU, strain, and what is held. State goes to a temporary
//! folder, never the app's own. Anything still held is resumed on exit.
use jitterbug_lib::{collector::Collector, pause, settings::Settings, supervisor};
use std::{sync::Arc, time::Duration};

fn arg(name: &str) -> Option<u64> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
}

fn main() {
    let seconds = arg("--seconds").unwrap_or(180);
    let directory = std::env::temp_dir().join(format!("jitterbug-governor-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("temporary folder");
    pause::init(&directory);
    let mut settings = Settings::default();
    settings.automation.hold_agents = true;
    let runtime = supervisor::Runtime::new(directory.clone(), settings, Arc::new(|_| {}));
    let mut collector = Collector::default();
    let started = std::time::Instant::now();
    let mut note = String::new();
    while started.elapsed().as_secs() < seconds {
        let sample = collector.sample();
        pause::expire();
        let view = runtime.govern(&sample).expect("governor");
        let held: Vec<&str> = view.held.iter().map(|h| h.title.as_str()).collect();
        println!(
            "{:>4}s cpu {:>5.1}% mem free {:>4.1}% os {:<8} held {:?}",
            started.elapsed().as_secs(),
            sample.cpu_percent,
            100.0 * sample.memory_available_bytes as f64 / sample.memory_total_bytes.max(1) as f64,
            sample.platform.memory_pressure.as_deref().unwrap_or("-"),
            held
        );
        if view.note != note {
            note = view.note.clone();
            println!("      -> {note}");
        }
        std::thread::sleep(Duration::from_secs(5));
    }
    pause::resume_all();
    let _ = std::fs::remove_dir_all(&directory);
}
