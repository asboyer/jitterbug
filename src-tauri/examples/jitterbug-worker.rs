//! Bounded opt-in feedback-loop fixture. Never started by the app itself.
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn main() {
    let automation = std::env::args().any(|arg| arg == "--run-automation-fixture");
    if !automation && !std::env::args().any(|arg| arg == "--run-bounded-fixture") {
        println!("Test fixture only. Pass --run-bounded-fixture to allocate 32 MiB for up to 90 seconds with a short, single-thread CPU pulse.");
        return;
    }
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    if system.available_memory() < 4 * 1024 * 1024 * 1024 {
        eprintln!("Fixture skipped: insufficient spare memory.");
        return;
    }
    let memory = vec![7u8; 32 * 1024 * 1024];
    let started = Instant::now();
    if automation {
        // Quiet baseline followed by a bounded 70% duty cycle on one thread.
        // Never stress all cores or allocate beyond the original 32 MiB.
        std::thread::sleep(Duration::from_secs(35));
        while started.elapsed() < Duration::from_secs(115) {
            system.refresh_memory();
            if system.available_memory() < 4 * 1024 * 1024 * 1024
                || matches!(
                    jitterbug_lib::platform::readings().thermal.as_deref(),
                    Some("serious" | "critical")
                )
            {
                break;
            }
            let slice = Instant::now();
            while slice.elapsed() < Duration::from_millis(70) {
                black_box(memory.iter().step_by(4096).map(|v| *v as u64).sum::<u64>());
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        return;
    }
    let pulse = Instant::now();
    while pulse.elapsed() < Duration::from_secs(3) {
        black_box(memory.iter().step_by(4096).map(|v| *v as u64).sum::<u64>());
    }
    while started.elapsed() < Duration::from_secs(90) {
        black_box(&memory);
        std::thread::sleep(Duration::from_millis(250));
    }
}
