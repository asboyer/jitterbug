//! Opt-in check of pausing one command's whole tree and resuming it.
//!
//!   cargo run --example jitterbug-pause-probe -- --pid <command pid> --seconds 20
//!
//! Pauses the command (stopping its launching shell first), confirms every member is stopped
//! and stays at zero CPU, then resumes and confirms they run again. Prints counts only.
use std::time::Duration;
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

fn arg(name: &str) -> Option<u64> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
}

fn states(members: &[(u32, u64)]) -> (usize, usize, u64) {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu(),
    );
    let (mut stopped, mut alive, mut cpu_ms) = (0, 0, 0);
    for (pid, start) in members {
        if let Some(p) = system
            .process(Pid::from_u32(*pid))
            .filter(|p| p.start_time() == *start)
        {
            alive += 1;
            cpu_ms += p.accumulated_cpu_time();
            if p.status() == ProcessStatus::Stop {
                stopped += 1;
            }
        }
    }
    (stopped, alive, cpu_ms)
}

fn main() {
    let pid = arg("--pid").expect("--pid is required") as u32;
    let seconds = arg("--seconds").unwrap_or(20);
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let start = system
        .process(Pid::from_u32(pid))
        .expect("no such process")
        .start_time();
    let record = match jitterbug_lib::pause::pause(pid, start, "probe command") {
        Ok(record) => record,
        Err(e) => {
            eprintln!("Pause refused: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "paused: {} members, launching shell stopped first: {}",
        record.members.len(),
        record.shell.is_some()
    );
    std::thread::sleep(Duration::from_secs(1));
    let (stopped, alive, cpu_a) = states(&record.members);
    std::thread::sleep(Duration::from_secs(seconds.saturating_sub(1)));
    let (stopped_later, _, cpu_b) = states(&record.members);
    println!(
        "while paused: {stopped}/{alive} stopped; CPU used during the pause: {} ms",
        cpu_b.saturating_sub(cpu_a)
    );
    println!("{}", jitterbug_lib::pause::resume(pid, start).unwrap());
    std::thread::sleep(Duration::from_secs(1));
    let (after, alive_after, _) = states(&record.members);
    println!("after resume: {after}/{alive_after} stopped");
    if stopped != alive || stopped_later != stopped || after != 0 || cpu_b > cpu_a + 20 {
        eprintln!("Pause probe failed.");
        std::process::exit(1);
    }
    println!("Pause probe passed.");
}
