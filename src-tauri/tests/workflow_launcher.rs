use jitterbug_lib::workflows::RunManifest;
use std::{fs, path::PathBuf, process::Command};

#[test]
#[ignore = "Invoked as an owned descendant by the event integration test"]
fn harness_event_child_fixture() {
    use std::{io::Write, process::Stdio};
    assert!(std::env::var_os("JITTERBUG_RUN_ID").is_some());
    for event in [
        serde_json::json!({"hook_event_name":"UserPromptSubmit", "session_id":"example-session", "prompt":"Build a search screen", "transcript_path":"private-transcript"}),
        serde_json::json!({"hook_event_name":"PostToolUse", "session_id":"example-session", "tool_name":"Edit", "tool_input":{"new_string":"private-code"}, "tool_response":"private-output"}),
        serde_json::json!({"hook_event_name":"UserPromptSubmit", "session_id":"different-session", "prompt":"Wrong assignment"}),
        serde_json::json!({"hook_event_name":"Stop", "session_id":"example-session"}),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_jitterbug-run"))
            .arg("event")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(event.to_string().as_bytes())
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(result.status.success());
        assert!(result.stdout.is_empty());
        assert!(
            result.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn real_descendant_events_survive_completion_and_outside_events_cannot_change_them() {
    use std::{io::Write, process::Stdio};
    let directory = directory().with_extension("events");
    fs::create_dir_all(&directory).unwrap();
    let registry = directory.join("registry");
    let binary = env!("CARGO_BIN_EXE_jitterbug-run");
    let result = Command::new(binary)
        .env("JITTERBUG_AGENT_RUNS_DIR", &registry)
        .args([
            "--task-id",
            "EXAMPLE-EVENT",
            "--task-title",
            "Search",
            "--agent",
            "fixture",
            "--worktree",
        ])
        .arg(&directory)
        .arg("--")
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "harness_event_child_fixture",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let path = fs::read_dir(&registry)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "json"))
        .unwrap();
    let before = fs::read(&path).unwrap();
    let mut run: RunManifest = serde_json::from_slice(&before).unwrap();
    assert_eq!(run.status, "completed");
    assert_eq!(run.context.request, "Build a search screen");
    assert_eq!(run.context.completed_tools["edit"], 1);
    assert_eq!(run.context.last_activity, "idle");
    for secret in [
        "private-transcript",
        "private-code",
        "private-output",
        "Wrong assignment",
    ] {
        assert!(!String::from_utf8_lossy(&before).contains(secret));
    }
    // Simulate abandoned metadata claiming the exited wrapper is still active.
    run.status = "active".into();
    run.ended_at = None;
    run.exit_code = None;
    jitterbug_lib::storage::write(&path, &run).unwrap();
    let stale = fs::read(&path).unwrap();
    let mut outsider = Command::new(binary)
        .arg("event")
        .env("JITTERBUG_AGENT_RUNS_DIR", &registry)
        .env("JITTERBUG_RUN_ID", &run.run_id)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    outsider.stdin.take().unwrap().write_all(br#"{"hook_event_name":"UserPromptSubmit","session_id":"example-session","prompt":"Spoofed"}"#).unwrap();
    let result = outsider.wait_with_output().unwrap();
    assert!(result.status.success(), "Hooks must never block a harness");
    assert!(String::from_utf8_lossy(&result.stderr).contains("not a descendant"));
    assert_eq!(fs::read(&path).unwrap(), stale);
    fs::remove_dir_all(directory).unwrap();
}

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
