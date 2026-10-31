//! End-to-end native automation probe: acts only on the disposable child it owns.
//! Run after building jitterbug-worker. No model calls, no machine data printed.
#[cfg(target_os = "macos")]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    use jitterbug_lib::{
        automation::{AssignedBy, Assignment, Bucket, Stage},
        collector::Collector,
        settings::Settings,
        supervisor::Runtime,
    };
    use std::{
        process::{Child, Command},
        sync::Arc,
        time::{Duration, Instant},
    };
    struct Fixture(Child);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.local/verification/automation");
    let application = directory.join("Example.app/Contents/MacOS/Example");
    std::fs::create_dir_all(application.parent().unwrap()).unwrap();
    std::fs::copy(
        std::env::current_exe()
            .unwrap()
            .with_file_name("jitterbug-worker"),
        &application,
    )
    .unwrap();
    let mut fixture = Fixture(
        Command::new(application)
            .arg("--run-automation-fixture")
            .spawn()
            .unwrap(),
    );
    let mut settings = Settings {
        budget_usd: 0.0,
        ..Default::default()
    };
    settings.automation.model_analysis = false;
    let runtime = Runtime::new(
        directory.clone(),
        settings,
        Arc::new(|c| {
            println!("Incident stage: {:?}", c.stage);
        }),
    );
    let mut collector = Collector::default();
    let deadline = Instant::now() + Duration::from_secs(110);
    let mut permission = false;
    let mut task = None;
    loop {
        assert!(
            Instant::now() < deadline,
            "Automation did not finish within the bounded test window"
        );
        assert!(
            fixture.0.try_wait().unwrap().is_none(),
            "Fixture exited or declined its resource guard"
        );
        let mut snapshot = collector.sample();
        if let Some(process) = snapshot
            .processes
            .iter()
            .find(|p| p.pid == fixture.0.id())
            .cloned()
        {
            if !permission {
                assert!(process.actions.iter().any(|a| a == "lower_priority"));
                // Background bucket in memory only: this probe runs no bucket enforcement, so
                // only the incident's own automatic change touches the fixture.
                runtime
                    .settings
                    .lock()
                    .unwrap()
                    .automation
                    .assign(Assignment {
                        workload_id: process.workload_id.clone(),
                        label: "Example".into(),
                        bucket: Bucket::Background,
                        since: 0,
                        source: AssignedBy::You,
                    });
                permission = true;
            }
            // Scope the real inventory to our child. Do not mutate unrelated applications or
            // synthesize measurements. OS global pressure remains real and may also trigger.
            snapshot.processes.retain(|p| p.pid == fixture.0.id());
            snapshot.workloads.retain(|w| w.id == process.workload_id);
            if let Some(id) = runtime.observe(&snapshot).unwrap() {
                assert!(
                    task.is_none(),
                    "Only one incident should dispatch for this continuous load"
                );
                let next = runtime.clone();
                task = Some(tokio::spawn(async move { next.run(id).await }));
            }
            if let Some(case) = runtime.cases().unwrap().first() {
                if matches!(case.stage, Stage::Blocked | Stage::Resolved) {
                    panic!(
                        "Incident did not complete: {:?}",
                        case.steps.last().unwrap().message
                    );
                }
                if case.stage == Stage::Completed {
                    assert!(case.automatic);
                    assert_eq!(case.proposal.as_ref().unwrap().process.pid, fixture.0.id());
                    assert!(case.verification.as_ref().unwrap().priority_verified);
                    assert!(case.report.as_ref().unwrap().fresh);
                    assert_eq!(
                        jitterbug_lib::platform::in_background(process.pid, process.start_time),
                        Some(true)
                    );
                    assert_eq!(runtime.spending().unwrap().requests, 0);
                    let stages: Vec<_> = case.steps.iter().map(|s| s.stage.clone()).collect();
                    for stage in [
                        Stage::Detected,
                        Stage::Investigating,
                        Stage::AwaitingApproval,
                        Stage::Applying,
                        Stage::Verifying,
                        Stage::Completed,
                    ] {
                        assert!(stages.contains(&stage));
                    }
                    assert!(
                        runtime.apply(case.id, true).await.is_err(),
                        "Completed actions cannot repeat"
                    );
                    jitterbug_lib::storage::write(&directory.join("result.json"), case).unwrap();
                    println!("PASS: sustained real workload detected, investigated with rules, allowed priority change applied automatically, background mode verified, fresh follow-up measured, duplicate action rejected. No cloud calls. Private evidence stays ignored.");
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    if let Some(task) = task {
        task.await.unwrap();
    }
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Native automatic action probe is macOS-only in this MVP.");
}
