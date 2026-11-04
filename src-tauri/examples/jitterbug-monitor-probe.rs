//! Opt-in provider smoke check with synthetic data only. No collection or process actions.
//! Uses the saved provider, privacy mode, credentials and shared spending ledger.
use jitterbug_lib::{
    applications::Application,
    providers::{ChatMessage, Engine},
    settings::{Provider, Settings},
    storage,
    types::*,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    if std::env::args().nth(1).as_deref() != Some("--live-provider") {
        return Err("Use --live-provider to make three paid Meta requests with synthetic data. No real process inventory is collected or sent.".into());
    }
    jitterbug_lib::load_development_env();
    let directory = dirs::data_dir()
        .ok_or("App data unavailable.")?
        .join("com.jitterbug.desktop");
    let mut settings: Settings = storage::read(&directory.join("settings.json"))?;
    if settings.intelligent.provider != Provider::Meta || !settings.mode.permits(&Provider::Meta) {
        return Err(
            "This check requires Meta as the saved, permitted intelligent provider.".into(),
        );
    }
    let engine = Engine::new(&directory);
    let before = engine.spending()?;
    settings.budget_usd = settings.budget_usd.min(before.reserved_usd + 0.05);
    let snapshot = Snapshot {
        source: "live".into(),
        sample_ready: true,
        captured_at: now(),
        interval_ms: 5000,
        cpu_percent: 22.0,
        logical_cpus: 8,
        memory_total_bytes: 16 * 1024 * 1024 * 1024,
        memory_available_bytes: 8 * 1024 * 1024 * 1024,
        pressure: Pressure {
            level: "normal".into(),
            ..Default::default()
        },
        workloads: vec![Workload {
            id: "synthetic-workload".into(),
            label: "Python".into(),
            category: "python".into(),
            cpu_percent: 18.0,
            memory_bytes: 256 * 1024 * 1024,
            process_count: 1,
            ..Default::default()
        }],
        processes: vec![ProcessRecord {
            pid: 4_000_000_000,
            start_time: now() - 60,
            name: "python3".into(),
            workload_id: "synthetic-workload".into(),
            resource_role: "test".into(),
            cpu_percent: 18.0,
            memory_bytes: 256 * 1024 * 1024,
            ..Default::default()
        }],
        ..Default::default()
    };
    let overview = engine.monitor_overview(&settings, &snapshot).await?;
    assert_eq!(overview.source, "model");
    assert!(overview.insight.is_some());
    let apps = [
        Application {
            name: "Python".into(),
            executable_name: "python3".into(),
            installation: "Standalone executable".into(),
            ..Default::default()
        },
        Application {
            name: "Node.js".into(),
            executable_name: "node".into(),
            installation: "Standalone executable".into(),
            ..Default::default()
        },
    ];
    let profiles = engine.app_profiles(&settings, &snapshot, &apps).await?;
    assert_eq!(profiles.len(), 2);
    let process = &snapshot.processes[0];
    let reply=engine.chat(&settings,&snapshot,vec![ChatMessage{role:"user".into(),content:"Investigate this selected process. Explain this instance's observed role and resource use; distinguish inference from measurements.".into()}],Some(&process.workload_id),"auto",Some((process.pid,process.start_time))).await?;
    assert_eq!(reply.source, "model");
    assert_eq!(
        reply.investigation.as_ref().and_then(|i| i.target.pid),
        Some(process.pid)
    );
    let after = engine.spending()?;
    println!("Monitor overview, batched descriptions and process investigation passed with Meta. Requests: {}. Estimated additional usage: ${:.6}.",after.requests-before.requests,after.reported_estimate_usd-before.reported_estimate_usd);
    Ok(())
}
