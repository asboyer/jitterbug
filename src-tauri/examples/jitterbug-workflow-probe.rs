//! Opt-in bounded collector probe. Output contains private local task metadata: keep it local.
use jitterbug_lib::collector::Collector;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut samples = 6_u32;
    let mut interval = 5000_u64;
    let mut task_id = None;
    while let Some(flag) = args.next() {
        if flag == "--help" {
            println!("Usage: jitterbug-workflow-probe [--samples 1..120] [--interval-ms 1000..5000] [--task-id ID]\nOutput is private local workflow JSON; do not commit it. No model calls or process changes.");
            return Ok(());
        }
        let value = args.next().ok_or("Missing option value")?;
        match flag.as_str() {
            "--samples" => samples = value.parse()?,
            "--interval-ms" => interval = value.parse()?,
            "--task-id" => task_id = Some(value),
            _ => return Err("Unknown option; use --help".into()),
        }
    }
    if !(1..=120).contains(&samples) || !(1000..=5000).contains(&interval) {
        return Err("Sample count or interval is outside the bounded range".into());
    }
    eprintln!("Private workflow telemetry; no inference or process actions. Keep output local.");
    let mut collector = Collector::default();
    for index in 0..samples {
        let snapshot = collector.sample();
        let workflows: Vec<_> = snapshot
            .agent_workflows
            .iter()
            .filter(|workflow| task_id.as_ref().is_none_or(|id| &workflow.task_id == id))
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "captured_at": snapshot.captured_at,
                "sample_ready": snapshot.sample_ready,
                "cpu_percent": snapshot.cpu_percent,
                "available_memory_bytes": snapshot.memory_available_bytes,
                "collector_ms": snapshot.collector_ms,
                "agent_workflows": workflows,
            })
        );
        if index + 1 < samples {
            std::thread::sleep(Duration::from_millis(interval));
        }
    }
    Ok(())
}
