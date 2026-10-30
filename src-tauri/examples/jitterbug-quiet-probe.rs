//! Opt-in check that ordinary use stays quiet: feeds live five-second samples, with the real
//! frontmost app, through the automatic monitor and prints what would have opened a case.
//! `--seconds N` sets the run length; `--front NAME` pretends the workload whose label contains
//! NAME is in front (for a run started from a terminal). Prints labels and shares, no paths.
use jitterbug_lib::{
    automation::{conditions, Monitor, Policy},
    collector::Collector,
    platform,
};
use std::time::Duration;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let seconds: u64 = arg("--seconds").and_then(|s| s.parse().ok()).unwrap_or(60);
    let pretend = arg("--front").map(|s| s.to_lowercase());
    let policy = Policy::default();
    let mut monitor = Monitor::default();
    let mut collector = Collector::default();
    let (mut opened, mut old_rule) = (0, 0);
    let rounds = seconds / 5 + 1;
    for round in 0..rounds {
        let mut s = collector.sample();
        s.frontmost_workload = match &pretend {
            Some(name) => s
                .workloads
                .iter()
                .find(|w| w.label.to_lowercase().contains(name))
                .map(|w| w.id.clone()),
            None => platform::frontmost().and_then(|(pid, _)| {
                s.processes
                    .iter()
                    .find(|p| p.pid == pid)
                    .map(|p| p.workload_id.clone())
            }),
        };
        if s.sample_ready {
            let front = s
                .workloads
                .iter()
                .find(|w| Some(&w.id) == s.frontmost_workload.as_ref());
            let top = s
                .workloads
                .iter()
                .max_by(|a, b| a.cpu_percent.total_cmp(&b.cpu_percent));
            // The rule before this change: one core and 10% of total, for any app.
            let old = s.workloads.iter().any(|w| {
                w.cpu_percent >= policy.app_cpu_percent
                    && w.cpu_percent * s.logical_cpus as f32 >= policy.app_cpu_core_percent
            });
            old_rule += old as usize;
            let now: Vec<String> = conditions(&s, &policy)
                .iter()
                .map(|t| t.title.clone())
                .collect();
            let fired = monitor.observe(&s, &policy, false);
            println!(
                "{:>3}s total {:>5.1}% | front {} | top {} | old rule {} | now {:?}{}",
                round * 5,
                s.cpu_percent,
                front.map_or("-".into(), |w| format!("{} {:.1}%", w.label, w.cpu_percent)),
                top.map_or("-".into(), |w| format!("{} {:.1}%", w.label, w.cpu_percent)),
                if old { "BUSY" } else { "-" },
                now,
                fired.map_or(String::new(), |t| {
                    opened += 1;
                    format!("  => WOULD OPEN: {}", t.title)
                }),
            );
        }
        if round + 1 < rounds {
            std::thread::sleep(Duration::from_secs(5));
        }
    }
    println!("cases opened: {opened}; samples where the old per-app rule matched: {old_rule}");
}
