//! Opt-in end-to-end profile check using one live executable and the saved provider.
//! No process actions. Never commit its output or replace fixtures with live data.
use jitterbug_lib::{
    collector::Collector, details, settings::Settings, storage, supervisor::Runtime,
};
use std::{sync::Arc, time::Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args().collect();
    if arguments.len() != 3 || arguments[1] != "--process" {
        return Err("Usage: jitterbug-profile-probe --process NAME. May make one paid request through the saved intelligent provider; uses the app's existing spending ledger.".into());
    }
    jitterbug_lib::load_development_env();
    let directory = dirs::data_dir()
        .ok_or("App data directory unavailable")?
        .join("com.jitterbug.desktop");
    let mut settings: Settings = storage::read(&directory.join("settings.json"))?;
    let before = jitterbug_lib::providers::Engine::new(&directory).spending()?;
    // Respect the saved ceiling and reserve at most five additional cents for this check.
    settings.budget_usd = settings.budget_usd.min(before.reserved_usd + 0.05);
    let runtime = Runtime::new(directory, settings, Arc::new(|_| {}));
    let mut collector = Collector::default();
    collector.sample();
    tokio::time::sleep(Duration::from_secs(5)).await;
    let snapshot = collector.sample();
    let process = snapshot
        .processes
        .iter()
        .find(|p| p.name == arguments[2])
        .ok_or("Requested process is not currently observed")?
        .clone();
    let detail = details::inspect(
        &snapshot,
        &process.workload_id,
        Some((process.pid, process.start_time)),
    )?;
    assert!(detail.application.is_some());
    *runtime.snapshot.write().unwrap() = snapshot;
    let profile = runtime
        .explain_application(&process.workload_id, process.pid, process.start_time)
        .await?;
    assert!(!profile.text.trim().is_empty());
    let after = runtime.engine.spending()?;
    let cached = runtime
        .explain_application(&process.workload_id, process.pid, process.start_time)
        .await?;
    assert!(cached.cached);
    assert_eq!(cached.text, profile.text);
    assert_eq!(runtime.engine.spending()?.requests, after.requests);
    assert!(after.requests <= before.requests + 1);
    println!("Profile generation and cache reuse passed. Provider requests: {}. Estimated additional usage: ${:.6}.", after.requests - before.requests, after.reported_estimate_usd - before.reported_estimate_usd);
    println!("{}", profile.text);
    Ok(())
}
