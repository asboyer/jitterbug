use jitterbug_lib::workflows::RunManifest;
use std::{fs, path::PathBuf, process::Command};

fn directory() -> PathBuf {
    std::env::temp_dir().join(format!(
        "jitterbug-launcher-integration-{}",
        std::process::id()
    ))
}

#[test]
fn wrapper_preserves_output_exit_status_and_private_bounded_registration() {
    let directory = directory();
    fs::create_dir_all(&directory).unwrap();
    let registry = directory.join("registrations");
    let binary = env!("CARGO_BIN_EXE_jitterbug-run");
    let launch = |argument: &str| {
        Command::new(binary)
            .env("JITTERBUG_AGENT_RUNS_DIR", &registry)
            .args([
                "--task-id",
                "EXAMPLE-1",
                "--task-title",
                "Example task",
                "--agent",
                "fixture",
                "--harness",
                "other",
                "--worktree",
            ])
            .arg(&directory)
            .args(["--", binary, argument])
            .output()
            .unwrap()
    };
    let success = launch("--help");
    assert!(success.status.success());
    assert!(String::from_utf8_lossy(&success.stdout).contains("Usage: jitterbug-run"));
    let failure = launch("--invalid-fixture-option");
    assert_eq!(failure.status.code(), Some(2));
    let mut runs: Vec<_> = fs::read_dir(&registry)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .map(|entry| {
            let bytes = fs::read(entry.path()).unwrap();
            let text = String::from_utf8(bytes.clone()).unwrap();
            assert!(!text.contains("--help"));
            assert!(!text.contains("--invalid-fixture-option"));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(entry.path()).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
            serde_json::from_slice::<RunManifest>(&bytes).unwrap()
        })
        .collect();
    runs.sort_by_key(|run| run.exit_code);
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, "completed");
    assert_eq!(runs[0].exit_code, Some(0));
    assert_eq!(runs[1].status, "failed");
    assert_eq!(runs[1].exit_code, Some(2));
    assert!(runs
        .iter()
        .all(|run| run.ended_at.is_some() && run.root_start_time > 0));
    assert_ne!(runs[0].run_id, runs[1].run_id);
    fs::remove_dir_all(directory).unwrap();
}
