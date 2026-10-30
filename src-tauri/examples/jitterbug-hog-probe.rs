//! Opt-in check that a short-lived CPU hog started inside the frontmost app is named.
//!
//! Starts a single-core `perl` busy loop a few seconds before a simulated Jitter Snapshot press,
//! with the app hosting this probe (a terminal or editor) as the frontmost "victim". The hog
//! must be found by the one-second scan and reported as a separate culprit, not as the app's
//! own work. `--exit-before N` stops the hog N seconds before the press, `--procs N` runs N
//! loops, and `--shell` starts them from a shell as a terminal command would. Prints no paths.
use jitterbug_lib::{
    collector::Collector,
    jitter::{self, FindingKind, Victim},
    pulse::{self, Mode, Pulse},
};
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const BASELINE_S: f64 = 30.0;

fn arg(name: &str) -> Option<f64> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
}

fn main() {
    let lead = arg("--lead").unwrap_or(4.0);
    let exit_before = arg("--exit-before");
    let procs = arg("--procs").unwrap_or(1.0).max(1.0) as usize;
    let shell = std::env::args().any(|a| a == "--shell");
    let pulse = Pulse::new();
    pulse.set_mode(Mode::Auto);
    let mut collector = Collector::default();
    let mut snapshot = collector.sample();
    pulse.watch(&snapshot);
    // The app this probe runs inside plays the frontmost app the person was using.
    let victim = host_app(&snapshot);
    // The hog is this probe's child, so it belongs to the same command inside that app.
    let expected = jitter::suspect_id(&snapshot, std::process::id());
    let started = Instant::now();
    let mut hog: Option<Hog> = None;
    let mut press: Option<u64> = None;
    let mut next_sample = Instant::now() + Duration::from_secs(5);
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let elapsed = started.elapsed().as_secs_f64();
        if hog.is_none() && press.is_none() && elapsed >= BASELINE_S - lead {
            hog = Some(Hog::start(procs, shell));
            println!("[{elapsed:4.1}s] hog started");
        }
        if let (Some(before), Some(h)) = (exit_before, hog.as_mut()) {
            if elapsed >= BASELINE_S - before && h.stop() {
                println!("[{elapsed:4.1}s] hog exited");
            }
        }
        if press.is_none() && elapsed >= BASELINE_S {
            press = Some(pulse::now_ms());
            pulse.press().expect("press");
            println!("[{elapsed:4.1}s] press");
        }
        if Instant::now() >= next_sample {
            next_sample += Duration::from_secs(5);
            snapshot = collector.sample();
            pulse.watch(&snapshot);
        }
        if press.is_some_and(|p| pulse::now_ms() >= p + jitter::POST_ROLL_MS) {
            break;
        }
    }
    let capture = pulse.freeze(press.unwrap(), victim.clone());
    if let Some(mut h) = hog {
        h.stop();
    }
    pulse.set_mode(Mode::SnapshotOnly);
    let stats = pulse.stats();
    let report = jitter::analyze(&capture, &snapshot);
    for f in &report.findings {
        println!("finding: {:?} {:?}", f.kind, f.strength);
    }
    println!(
        "victim: {}",
        report.victim.as_ref().map_or("none", |v| v.label.as_str())
    );
    let culprit = report.culprit.as_ref().map(|c| c.label.as_str());
    // Folder names stay out of the printed result.
    let shown = culprit.map(|c| match (c.find(" ("), c.rfind(") in ")) {
        (Some(open), Some(close)) => format!("{}{}", &c[..open], &c[close + 1..]),
        _ => c.to_string(),
    });
    println!("culprit: {}", shown.as_deref().unwrap_or("none"));
    println!(
        "sampler: {} ticks, {:.2} ms average, {:.2} ms max per tick",
        stats.ticks,
        stats.work_ns_total as f64 / stats.ticks.max(1) as f64 / 1e6,
        stats.work_ns_max as f64 / 1e6
    );
    let spiked = report.findings.iter().any(|f| {
        matches!(f.kind, FindingKind::ProcessSpike | FindingKind::CorePegged)
            && f.workload_id == expected
    });
    println!(
        "hog finding: {}, top culprit: {}",
        if spiked { "yes" } else { "no" },
        match &report.culprit {
            Some(c) if c.workload_id == expected => "the hog",
            Some(_) => "other load",
            None => "none",
        }
    );
    let victim_busy = report
        .findings
        .iter()
        .any(|f| f.kind == FindingKind::VictimBusy);
    let separate = match (&report.culprit, &report.victim) {
        (Some(c), Some(v)) => c.workload_id != v.workload_id,
        (Some(_), None) => true,
        _ => false,
    };
    if !spiked || !separate || victim_busy {
        eprintln!("Hog probe failed: the hog was not reported as a separate culprit.");
        std::process::exit(1);
    }
    println!("Hog probe passed: the short-lived hog was named as a separate culprit.");
}

/// Single-core `perl` loops. Bounded: each exits on its own after 60 seconds even if this probe
/// dies. With `--shell` they are a shell's children, in their own process group.
struct Hog(Vec<std::process::Child>);

impl Hog {
    fn start(procs: usize, shell: bool) -> Self {
        const LOOP: &str = "alarm 60; 1 while 1";
        let spawn = |command: &mut Command| {
            #[cfg(unix)]
            std::os::unix::process::CommandExt::process_group(command, 0);
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .spawn()
                .expect("perl and sh are required for this probe")
        };
        if shell {
            let script = vec![format!("perl -e '{LOOP}' &"); procs].join(" ") + " wait";
            Self(vec![spawn(Command::new("sh").args(["-c", &script]))])
        } else {
            Self(
                (0..procs)
                    .map(|_| spawn(Command::new("perl").args(["-e", LOOP])))
                    .collect(),
            )
        }
    }
    /// Stop every loop. Returns whether any was still running.
    fn stop(&mut self) -> bool {
        let mut running = false;
        for child in &mut self.0 {
            if child.try_wait().ok().flatten().is_some() {
                continue;
            }
            running = true;
            // The whole group, so a shell's children stop too.
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        running
    }
}

/// Outermost app bundle among this process's ancestors, as the frontmost-app stand-in.
fn host_app(snapshot: &jitterbug_lib::types::Snapshot) -> Option<Victim> {
    let mut pid = Some(std::process::id());
    let mut found = None;
    for _ in 0..32 {
        let Some(p) = pid.and_then(|id| snapshot.processes.iter().find(|p| p.pid == id)) else {
            break;
        };
        if p.executable
            .as_deref()
            .and_then(jitterbug_lib::attribution::application_bundle)
            .is_some()
        {
            found = Some(Victim {
                pid: p.pid,
                name: None,
            });
        }
        pid = p.parent_pid;
    }
    found
}
