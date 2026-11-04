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
        return Err("Use --live-provider to make two paid Meta requests with synthetic data. No real process inventory is collected or sent.".into());
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
    settings.budget_usd = settings.budget_usd.min(before.reserved_usd + 0.20);
    let mut snapshot = Snapshot {
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
    let apps = [
        Application {
            name: "Grok Bot".into(),
            executable_name: "Grok Bot".into(),
            installation: "Application bundle".into(),
            ..Default::default()
        },
        Application {
            name: "Google Chrome".into(),
            executable_name: "Google Chrome Helper (GPU)".into(),
            identifier: Some("com.google.Chrome".into()),
            component: Some("Google Chrome Helper (GPU)".into()),
            component_kind: "app helper".into(),
            installation: "Application bundle".into(),
            ..Default::default()
        },
    ];
    if !std::env::args().any(|argument| argument == "--chat-only") {
        let profiles = engine.app_profiles(&settings, &snapshot, &apps).await?;
        assert_eq!(profiles.len(), 2);
        for profile in &profiles {
            println!(
                "Synthetic identity: {} Sources: {}",
                profile.text,
                profile.sources.len()
            );
        }
        assert!(
            !profiles[0].sources.is_empty(),
            "Unfamiliar app research must retain retrieved sources"
        );
    }
    snapshot.captured_at = now();
    let process = &snapshot.processes[0];
    let started = std::time::Instant::now();
    let reply = engine.monitor_answer(
        &settings, &snapshot,
        vec![ChatMessage { role: "user".into(), content: "Why is this Python process using memory, and what can you actually tell about this instance?".into() }],
        Some(&process.workload_id), Some((process.pid, process.start_time)), None,
    ).await?;
    assert!(!reply.text.is_empty());
    assert!(reply.action.is_none());
    println!(
        "Synthetic conversation ({} ms): {}",
        started.elapsed().as_millis(),
        reply.text
    );
    let after = engine.spending()?;
    println!("Requested Monitor provider checks passed with Meta. Requests: {}. Estimated additional usage: ${:.6}.",after.requests-before.requests,after.reported_estimate_usd-before.reported_estimate_usd);
    Ok(())
}
