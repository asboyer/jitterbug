//! Print what the Agents view sees right now: each coding agent conversation, its state, what
//! it is doing, and its usage. Local check only; nothing is stored or sent.
use jitterbug_lib::{agents, collector::Collector, turns, types::now};

fn main() {
    let mut collector = Collector::default();
    collector.sample();
    std::thread::sleep(std::time::Duration::from_secs(2));
    let snapshot = collector.sample();
    let state = turns::directory()
        .and_then(|d| turns::inspect(&d))
        .unwrap_or_default();
    let view = agents::view(&snapshot, &state, vec![]);
    println!(
        "cpu {:.0}%  shown {}  hidden (quiet over a day) {}",
        view.cpu_percent,
        view.sessions.len(),
        view.stale
    );
    for s in &view.sessions {
        println!(
            "{:<6} {:<26} {:<8} {:>5}m ago  cpu {:>5.1}%  {:>5} MiB  doing: {}",
            s.harness,
            format!("{} {}", s.id, s.title)
                .chars()
                .take(26)
                .collect::<String>(),
            s.state,
            s.last_active.map_or(0, |t| now().saturating_sub(t) / 60),
            s.cpu_percent,
            s.memory_bytes / 1_048_576,
            s.doing.as_deref().unwrap_or("-"),
        );
        for a in &s.agents {
            println!(
                "         - {} [{}] {}",
                a.label,
                a.state,
                a.doing.as_deref().or(a.command.as_deref()).unwrap_or("")
            );
        }
    }
}
