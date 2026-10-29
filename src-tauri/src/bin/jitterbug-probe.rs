#[tokio::main(flavor = "current_thread")]
async fn main() {
    jitterbug_lib::load_development_env();
    let mut collector = jitterbug_lib::collector::Collector::default();
    collector.sample();
    std::thread::sleep(std::time::Duration::from_secs(5));
    let snapshot = collector.sample();
    assert!(snapshot.sample_ready && snapshot.memory_total_bytes > 0 && snapshot.process_count > 0);
    assert!(!snapshot.workloads.is_empty());
    // Never print process names, paths, machine identifiers, or telemetry values.
    println!("Live collector smoke passed: sample, process inventory, workload attribution, and bounded history.");

    let option = std::env::args().nth(1);
    if option.as_deref() == Some("--actions") {
        action_smoke();
        return;
    }
    if let Some(option) = option
        .filter(|arg| ["--local-model", "--meta-model", "--decision-model"].contains(&arg.as_str()))
    {
        use jitterbug_lib::{
            providers::{ChatMessage, Engine},
            settings::{Mode, Settings},
        };
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.local/verification");
        let engine = Engine::new(&directory);
        let mut settings = Settings {
            mode: if option == "--meta-model" {
                Mode::CloudOnly
            } else {
                Mode::LocalOnly
            },
            budget_usd: 0.25,
            ..Default::default()
        };
        settings.decision_enabled = option == "--decision-model";
        let tier = if option == "--meta-model" {
            "intelligent"
        } else if option == "--decision-model" {
            "auto"
        } else {
            "cheap"
        };
        let reply = engine.chat(&settings, &snapshot, vec![ChatMessage { role: "user".into(), content: "What does CPU usage mean? Give one short explanation without repeating any telemetry measurements.".into() }], None, tier).await.expect("Model smoke request failed");
        if option == "--decision-model" {
            assert!(
                reply
                    .route
                    .iter()
                    .any(|step| step.starts_with("Decision tier selected")),
                "Decision model did not answer: {:?}",
                reply.route
            );
        } else {
            assert_eq!(
                reply.source, "model",
                "Model did not answer: {:?}",
                reply.route
            );
            assert!(!reply.text.trim().is_empty());
        }
        // Response text remains local because it can contain telemetry-derived observations.
        jitterbug_lib::storage::write(
            &directory.join(format!("{}.json", option.trim_start_matches('-'))),
            &reply,
        )
        .unwrap();
        println!("Model smoke passed; response saved only in ignored local evidence. Estimated call cost: ${:.6}", reply.estimated_cost_usd);
        println!("Verification ledger: {} reserved requests, ${:.6} estimated usage, ${:.6} conservatively reserved.", engine.spending().unwrap().requests, engine.spending().unwrap().reported_estimate_usd, engine.spending().unwrap().reserved_usd);
    }
}

/// Only acts on a fixture this probe creates; cleanup also runs on assertion failure.
#[cfg(target_os = "macos")]
fn action_smoke() {
    use std::{
        process::{Child, Command},
        time::{Duration, Instant},
    };
    struct Fixture(Child);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let executable = std::env::current_exe()
        .unwrap()
        .with_file_name("jitterbug-worker");
    let mut fixture = Fixture(
        Command::new(executable)
            .arg("--run-bounded-fixture")
            .spawn()
            .expect("Build jitterbug-worker before running the action probe"),
    );
    let mut collector = jitterbug_lib::collector::Collector::default();
    collector.sample();
    std::thread::sleep(Duration::from_millis(750));
    let snapshot = collector.sample();
    let process = snapshot
        .processes
        .iter()
        .find(|p| p.pid == fixture.0.id())
        .expect("Fixture must appear in real telemetry");
    assert!(process.memory_bytes >= 24 * 1024 * 1024 && process.cpu_percent > 0.0);
    assert!(process.actionable);
    assert!(
        jitterbug_lib::actions::execute(process.pid, process.start_time + 1, "terminate").is_err()
    );
    jitterbug_lib::actions::execute(process.pid, process.start_time, "lower_priority").unwrap();
    #[cfg(target_os = "macos")]
    assert!(unsafe { libc::getpriority(libc::PRIO_PROCESS, process.pid) } >= 10);
    jitterbug_lib::actions::execute(process.pid, process.start_time, "terminate").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while fixture.0.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "Fixture should exit after termination request"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("Action smoke passed: real bounded CPU/RAM fixture observed, stale identity rejected, priority lowered, and only the fixture terminated.");
}

#[cfg(not(target_os = "macos"))]
fn action_smoke() {
    panic!("Native actions are macOS-only in this MVP.");
}
