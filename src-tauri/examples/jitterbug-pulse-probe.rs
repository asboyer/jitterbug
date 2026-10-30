//! Headless Jitter Snapshot checks. Prints sampler cost and finding kinds only: never process
//! names, paths, PIDs, or other machine details.
//!
//!   cargo run --release --example jitterbug-pulse-probe -- --seconds 120
//!   cargo run --release --example jitterbug-pulse-probe -- --lab pegged|contention|memory|disk
//!
//! `--lab` starts a disposable load near the press time, freezes a real capture, runs the
//! deterministic analysis, and reports whether it blamed the load. The load is always stopped.
use jitterbug_lib::{
    automation::Origin,
    collector::Collector,
    jitter,
    pulse::{self, Mode, Pulse},
};
use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Default)]
struct Usage {
    cpu_ns: u64,
    wakeups: u64,
    resident: u64,
}

#[cfg(target_os = "macos")]
#[allow(deprecated)] // libc exposes the stable Mach timebase API; no extra crate is needed.
fn usage() -> Usage {
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as i32,
            libc::RUSAGE_INFO_V4,
            (&mut info as *mut libc::rusage_info_v4).cast(),
        )
    };
    assert_eq!(result, 0, "proc_pid_rusage failed");
    let mut timebase = libc::mach_timebase_info { numer: 0, denom: 0 };
    unsafe { libc::mach_timebase_info(&mut timebase) };
    let ticks = info.ri_user_time + info.ri_system_time;
    Usage {
        cpu_ns: ticks * timebase.numer as u64 / timebase.denom.max(1) as u64,
        wakeups: info.ri_pkg_idle_wkups + info.ri_interrupt_wkups,
        resident: info.ri_resident_size,
    }
}

#[cfg(not(target_os = "macos"))]
fn usage() -> Usage {
    let mut system = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(std::process::id());
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    let p = system.process(pid).expect("own process");
    Usage {
        cpu_ns: p.accumulated_cpu_time() * 1_000_000,
        wakeups: 0,
        resident: p.memory(),
    }
}

fn percentile(values: &mut [f32], p: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f32::total_cmp);
    values[((values.len() - 1) as f32 * p).round() as usize]
}

fn budget(seconds: u64) {
    // One full sample gives the pulse its 16 busiest PIDs, exactly as the app does.
    let mut collector = Collector::default();
    collector.sample();
    std::thread::sleep(Duration::from_secs(2));
    let snapshot = collector.sample();
    drop(collector);
    std::thread::sleep(Duration::from_secs(1));
    let idle_start = usage();
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(10));
    let idle = usage();
    let idle_wall = started.elapsed().as_secs_f64();

    let pulse = Pulse::new();
    pulse.watch(&snapshot);
    pulse.set_mode(Mode::Auto);
    let before = usage();
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(seconds));
    let after = usage();
    let wall = started.elapsed().as_secs_f64();
    let stats = pulse.stats();
    let mut lags: Vec<f32> = pulse
        .latest(pulse::CAPACITY)
        .iter()
        .map(|t| t.lag_ms)
        .collect();
    pulse.set_mode(Mode::SnapshotOnly);

    let cpu = (after.cpu_ns - before.cpu_ns) as f64 / (wall * 1e9) * 100.0;
    let idle_cpu = (idle.cpu_ns - idle_start.cpu_ns) as f64 / (idle_wall * 1e9) * 100.0;
    let wakeups = (after.wakeups - before.wakeups) as f64 / wall;
    let idle_wakeups = (idle.wakeups - idle_start.wakeups) as f64 / idle_wall;
    let rss = after.resident as f64 - idle.resident as f64;
    let avg_us = stats.work_ns_total as f64 / stats.ticks.max(1) as f64 / 1000.0;
    println!("Pulse budget over {wall:.0} s ({} ticks):", stats.ticks);
    println!(
        "  CPU: {:.3}% of one core (idle process {:.3}%); target < 0.5%",
        cpu - idle_cpu,
        idle_cpu
    );
    println!(
        "  Resident memory: {:+.2} MiB vs idle process; target < 5 MiB",
        rss / 1048576.0
    );
    println!(
        "  Wakeups: {:.2}/s extra (idle process {:.2}/s)",
        wakeups - idle_wakeups,
        idle_wakeups
    );
    println!(
        "  Tick work: {avg_us:.0} µs average, {:.0} µs max (excludes the 10 ms probe sleep)",
        stats.work_ns_max as f64 / 1000.0
    );
    println!(
        "  Scheduler probe: p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms late",
        percentile(&mut lags, 0.5),
        percentile(&mut lags, 0.95),
        percentile(&mut lags, 1.0)
    );
}

struct Load(Vec<Child>, Option<std::path::PathBuf>);

impl Drop for Load {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(path) = &self.1 {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn start(scenario: &str, cores: usize) -> Load {
    let quiet = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Could not start the lab load")
    };
    match scenario {
        "pegged" => Load(vec![quiet("yes", &[])], None),
        "contention" => Load((0..cores).map(|_| quiet("yes", &[])).collect(), None),
        "memory" => Load(vec![quiet("memory_pressure", &["-l", "warn"])], None),
        "disk" => {
            let path = std::env::temp_dir().join(format!("jitterbug-lab-{}", std::process::id()));
            let target = format!("of={}", path.display());
            Load(
                vec![quiet(
                    "dd",
                    &["if=/dev/zero", &target, "bs=1m", "count=4096"],
                )],
                Some(path),
            )
        }
        _ => panic!("Unknown lab scenario. Use pegged, contention, memory, or disk."),
    }
}

fn lab(scenario: &str) {
    let pulse = Pulse::new();
    pulse.set_mode(Mode::Auto);
    let mut collector = Collector::default();
    collector.sample();
    let mut snapshot;
    let started = Instant::now();
    let mut load: Option<Load> = None;
    let mut press = None;
    let mut load_pids = vec![];
    println!("Lab '{scenario}': 55 s quiet baseline, load at −8 s, press at 0, 10 s post-roll.");
    loop {
        std::thread::sleep(Duration::from_secs(5) - Duration::from_millis(10));
        snapshot = collector.sample();
        pulse.watch(&snapshot);
        let elapsed = started.elapsed().as_secs();
        if load.is_none() && elapsed >= 55 {
            let l = start(scenario, snapshot.logical_cpus);
            load_pids = l.0.iter().map(Child::id).collect();
            load = Some(l);
        }
        if press.is_none() && elapsed >= 63 {
            press = Some(pulse::now_ms());
            assert!(pulse.press().is_ok());
        }
        if press.is_some_and(|p| pulse::now_ms() >= p + jitter::POST_ROLL_MS) {
            break;
        }
    }
    let victim =
        jitterbug_lib::platform::frontmost().map(|(pid, name)| jitter::Victim { pid, name });
    let had_victim = victim.is_some();
    let capture = pulse.freeze(press.unwrap(), victim);
    // Workload IDs of the load, from the same real inventory the analysis sees.
    let load_workloads: Vec<_> = snapshot
        .processes
        .iter()
        .filter(|p| load_pids.contains(&p.pid))
        .filter_map(|p| jitter::suspect_id(&snapshot, p.pid))
        .collect();
    drop(load);
    pulse.set_mode(Mode::SnapshotOnly);
    assert_eq!(capture.origin, Origin::Snapshot);
    let report = jitter::analyze(&capture, &snapshot);
    println!(
        "  Capture: {} ticks, frontmost app identified: {had_victim}",
        capture.ticks.len()
    );
    for f in &report.findings {
        println!("  Finding: {:?} ({:?})", f.kind, f.strength);
    }
    let blamed = report
        .culprit
        .as_ref()
        .and_then(|c| c.workload_id.as_ref())
        .is_some_and(|id| load_workloads.contains(id));
    let mentioned = report.findings.iter().any(|f| {
        f.workload_id
            .as_ref()
            .is_some_and(|id| load_workloads.contains(id))
    });
    let mut distinct = load_workloads.clone();
    distinct.sort();
    distinct.dedup();
    let naming = report
        .findings
        .iter()
        .filter(|f| {
            f.workload_id
                .as_ref()
                .is_some_and(|id| load_workloads.contains(id))
        })
        .count();
    println!(
        "  Load in inventory: {} ({} workload group(s)); findings naming it: {naming} of {}",
        !load_workloads.is_empty(),
        distinct.len(),
        report.findings.len()
    );
    println!("  Culprit is the lab load: {blamed}; any finding names the load: {mentioned}");
    println!(
        "  In-app action: {}",
        report.in_app.as_ref().map_or("none", |s| s.action.as_str())
    );
    let lags: Vec<_> = capture.ticks.iter().map(|t| t.lag_ms).collect();
    println!(
        "  Probe lag max {:.1} ms; busiest core max {:.0}%",
        lags.iter().copied().fold(0.0, f32::max),
        capture
            .ticks
            .iter()
            .map(|t| t.core_max())
            .fold(0.0, f32::max)
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    if let Some(scenario) = value("--lab") {
        lab(&scenario);
        return;
    }
    let seconds = value("--seconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(60)
        .clamp(10, 3600);
    budget(seconds);
}
