//! Bounded opt-in feedback-loop fixture. Never started by the app itself.
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn main() {
    if !std::env::args().any(|arg| arg == "--run-bounded-fixture") {
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
    let pulse = Instant::now();
    while pulse.elapsed() < Duration::from_secs(3) {
        black_box(memory.iter().step_by(4096).map(|v| *v as u64).sum::<u64>());
    }
    while started.elapsed() < Duration::from_secs(90) {
        black_box(&memory);
        std::thread::sleep(Duration::from_millis(250));
    }
}
