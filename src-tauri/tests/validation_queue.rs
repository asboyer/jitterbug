use jitterbug_lib::{
    queue::{self, resources::GIB, JobView, Profile, Stage},
    types::{now, Snapshot},
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

struct Fixture {
    directory: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "jitterbug-queue-{}-{unique}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        Self {
            directory: directory.canonicalize().unwrap(),
        }
    }
    fn queue(&self) -> PathBuf {
        self.directory.join("queue")
    }
    fn profile(&self, name: &str, mode: &str, git: bool) -> PathBuf {
        let cwd = self.directory.join(name);
        fs::create_dir_all(&cwd).unwrap();
        fs::write(cwd.join("input.txt"), "first version").unwrap();
        fs::write(cwd.join("mode.txt"), mode).unwrap();
        if git {
            for args in [
                vec!["init", "-q"],
                vec!["add", "."],
                vec![
                    "-c",
                    "user.name=Queue Test",
                    "-c",
                    "user.email=queue@example.invalid",
                    "commit",
                    "-qm",
                    "fixture",
                ],
            ] {
                assert!(Command::new("git")
                    .args(args)
                    .current_dir(&cwd)
                    .output()
                    .unwrap()
                    .status
                    .success());
            }
        }
        queue::add_profile(
            &self.queue(),
            Profile {
                name: name.into(),
                worktree: cwd.clone(),
                command: vec![
                    std::env::current_exe().unwrap().to_string_lossy().into(),
                    "--exact".into(),
                    "validation_child_fixture".into(),
                    "--ignored".into(),
                    "--nocapture".into(),
                ],
                memory_bytes: GIB,
                search_path: None,
            },
        )
        .unwrap();
        cwd
    }
    fn submit(&self, name: &str, request: &str) -> JobView {
        queue::submit(
            &self.queue(),
            name,
            request,
            "Check a synthetic change",
            "test",
            request,
        )
        .unwrap()
    }
    fn tick_command(&self, pressure: bool) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "validation_tick_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("JITTERBUG_QUEUE_DIR", self.queue())
            .env("META_API_KEY", "synthetic-secret-must-not-reach-validation")
            .env("JITTERBUG_AGENT_RUNS_DIR", self.directory.join("runs"))
            .env(
                "JITTERBUG_TEST_PRESSURE",
                if pressure { "yes" } else { "no" },
            );
        command
    }
    fn tick(&self, pressure: bool) {
        let output = self.tick_command(pressure).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn settle(&self) {
        let start = Instant::now();
        loop {
            self.tick(false);
            if !queue::inspect(&self.queue())
                .unwrap()
                .jobs
                .iter()
                .any(|j| j.stage.active())
            {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "queue did not settle"
            );
            std::thread::sleep(Duration::from_millis(80));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
#[ignore = "Owned command fixture, invoked only by a queue worker"]
fn validation_child_fixture() {
    assert!(std::env::var_os("META_API_KEY").is_none());
    let directory = queue::directory().unwrap();
    let id = std::env::var("JITTERBUG_QUEUE_JOB_ID").unwrap();
    let event = |kind: &str| {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(directory.join("events.txt"))
            .unwrap();
        fs2::FileExt::lock_exclusive(&file).unwrap();
        writeln!(file, "{kind} {id}").unwrap();
    };
    event("start");
    let mode = fs::read_to_string("mode.txt").unwrap();
    std::thread::sleep(Duration::from_millis(if mode == "slow" {
        1200
    } else {
        120
    }));
    if mode == "change" {
        fs::write("input.txt", "changed during validation").unwrap();
    }
    if mode == "path" {
        let output = Command::new("node")
            .args(["-e", "console.log('helper found through approved PATH')"])
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    println!("validation stdout");
    eprintln!("validation stderr");
    if mode == "large" {
        print!("{}", "x".repeat(100_000));
    }
    event("end");
    if mode == "fail" {
        std::process::exit(7);
    }
}

#[test]
#[ignore = "Synthetic telemetry coordinator isolated in its own environment"]
fn validation_tick_fixture() {
    let directory = queue::directory().unwrap();
    let mut sample = Snapshot {
        source: "live".into(),
        captured_at: now() - 1,
        sample_ready: true,
        cpu_percent: if std::env::var("JITTERBUG_TEST_PRESSURE").unwrap() == "yes" {
            90.0
        } else {
            10.0
        },
        memory_total_bytes: 16 * GIB,
        memory_available_bytes: 10 * GIB,
        ..Default::default()
    };
    queue::tick(
        &directory,
        &sample,
        Path::new(env!("CARGO_BIN_EXE_jitterbug-run")),
    )
    .unwrap();
    sample.captured_at = now();
    queue::tick(
        &directory,
        &sample,
        Path::new(env!("CARGO_BIN_EXE_jitterbug-run")),
    )
    .unwrap();
    // A coordinator may exit while its worker remains alive; this is intentional.
}

#[test]
fn independent_sessions_share_one_fifo_slot_and_return_exit_and_output() {
    let f = Fixture::new();
    for (name, mode) in [
        ("claude-check", "ok"),
        ("codex-check", "fail"),
        ("meta-check", "large"),
    ] {
        f.profile(name, mode, true);
    }
    let one = f.submit("claude-check", "session-1");
    let two = f.submit("codex-check", "session-2");
    let three = f.submit("meta-check", "session-3");
    let mut a = f.tick_command(false).spawn().unwrap();
    let mut b = f.tick_command(false).spawn().unwrap();
    assert!(a.wait().unwrap().success());
    assert!(b.wait().unwrap().success());
    f.settle();
    let state = queue::inspect(&f.queue()).unwrap();
    assert_eq!(
        state
            .jobs
            .iter()
            .map(|j| j.stage.clone())
            .collect::<Vec<_>>(),
        vec![Stage::Passed, Stage::Failed, Stage::Passed]
    );
    assert_eq!(state.jobs[1].exit_code, Some(7));
    let mut task_ids = std::collections::HashSet::new();
    for entry in fs::read_dir(f.directory.join("runs"))
        .unwrap()
        .filter_map(Result::ok)
    {
        if entry.path().extension().is_some_and(|e| e == "json") {
            let manifest: jitterbug_lib::workflows::RunManifest =
                serde_json::from_slice(&fs::read(entry.path()).unwrap()).unwrap();
            assert!(task_ids.insert(manifest.task_id));
        }
    }
    assert_eq!(task_ids.len(), 3);
    let events = fs::read_to_string(f.queue().join("events.txt")).unwrap();
    assert_eq!(
        events.lines().collect::<Vec<_>>(),
        vec![
            format!("start {}", one.id),
            format!("end {}", one.id),
            format!("start {}", two.id),
            format!("end {}", two.id),
            format!("start {}", three.id),
            format!("end {}", three.id)
        ]
    );
    let result = queue::mcp::call(
        &f.queue(),
        &["meta-check".into()],
        "test",
        "validation_result",
        &serde_json::json!({"job_id":three.id}),
    )
    .unwrap();
    assert_eq!(result["output"]["truncated"], true);
    assert!(result["output"]["stdout"].as_str().unwrap().len() <= 32_768);
    assert!(result["output"]["stderr"]
        .as_str()
        .unwrap()
        .contains("validation stderr"));
}

#[test]
fn pressure_pause_cancellation_and_idempotency_do_not_launch_work() {
    let f = Fixture::new();
    f.profile("check", "ok", false);
    let job = f.submit("check", "same-request");
    assert_eq!(f.submit("check", "same-request").id, job.id);
    f.tick(true);
    assert_eq!(
        queue::inspect(&f.queue()).unwrap().jobs[0].stage,
        Stage::Queued
    );
    assert!(!f.queue().join("events.txt").exists());
    queue::transaction(&f.queue(), |s| {
        s.paused = true;
        Ok(())
    })
    .unwrap();
    f.tick(false);
    assert_eq!(
        queue::inspect(&f.queue()).unwrap().jobs[0].stage,
        Stage::Queued
    );
    queue::cancel(&f.queue(), &job.id).unwrap();
    queue::transaction(&f.queue(), |s| {
        s.paused = false;
        Ok(())
    })
    .unwrap();
    f.tick(false);
    assert!(!f.queue().join("events.txt").exists());
}

#[test]
fn source_changes_while_waiting_and_running_cannot_report_a_pass() {
    let f = Fixture::new();
    let cwd = f.profile("waiting", "ok", true);
    f.profile("running", "change", true);
    f.submit("waiting", "before");
    fs::write(cwd.join("input.txt"), "new input").unwrap();
    f.submit("running", "during");
    f.settle();
    let state = queue::inspect(&f.queue()).unwrap();
    assert_eq!(state.jobs[0].stage, Stage::Stale);
    assert!(state.jobs[0].exit_code.is_none());
    assert_eq!(
        state.jobs[1].stage,
        Stage::Stale,
        "{}",
        queue::mcp::call(
            &f.queue(),
            &["running".into()],
            "test",
            "validation_result",
            &serde_json::json!({"job_id":state.jobs[1].id})
        )
        .unwrap()
    );
    assert_eq!(state.jobs[1].exit_code, Some(0));
    assert_eq!(
        fs::read_to_string(f.queue().join("events.txt"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[test]
fn removed_profiles_and_mcp_scope_are_enforced() {
    let f = Fixture::new();
    f.profile("approved", "ok", false);
    f.profile("another-project", "ok", false);
    let job = f.submit("approved", "removed");
    let private = f.submit("another-project", "private");
    assert!(queue::mcp::call(
        &f.queue(),
        &["approved".into()],
        "test",
        "validation_result",
        &serde_json::json!({"job_id":private.id})
    )
    .is_err());
    assert!(queue::mcp::call(&f.queue(), &["approved".into()], "test", "validation_submit", &serde_json::json!({"profile":"approved","request_id":"injection","task":"check","session":"test","command":"arbitrary command"})).is_err());
    queue::transaction(&f.queue(), |s| {
        s.profiles.retain(|p| p.name != "approved");
        Ok(())
    })
    .unwrap();
    queue::cancel(&f.queue(), &private.id).unwrap();
    f.settle();
    assert_eq!(
        queue::inspect(&f.queue())
            .unwrap()
            .jobs
            .iter()
            .find(|j| j.id == job.id)
            .unwrap()
            .stage,
        Stage::Stale
    );
}

#[test]
fn missing_worker_blocks_all_new_work_until_explicit_review() {
    let f = Fixture::new();
    f.profile("check", "ok", false);
    let lost = f.submit("check", "lost");
    f.submit("check", "next");
    queue::transaction(&f.queue(), |s| {
        s.jobs[0].stage = Stage::Running;
        s.jobs[0].worker = Some(queue::Identity {
            pid: std::process::id(),
            start: 1,
        });
        Ok(())
    })
    .unwrap();
    f.tick(false);
    let state = queue::inspect(&f.queue()).unwrap();
    assert_eq!(state.jobs[0].stage, Stage::Interrupted);
    assert_eq!(state.jobs[1].stage, Stage::Queued);
    assert!(!f.queue().join("events.txt").exists());
    queue::transaction(&f.queue(), |s| {
        s.jobs[0].child = queue::identity(std::process::id());
        Ok(())
    })
    .unwrap();
    assert!(queue::acknowledge(&f.queue(), &lost.id).is_err());
    queue::transaction(&f.queue(), |s| {
        s.jobs[0].child = None;
        Ok(())
    })
    .unwrap();
    queue::acknowledge(&f.queue(), &lost.id).unwrap();
    f.settle();
    assert_eq!(
        queue::inspect(&f.queue()).unwrap().jobs[1].stage,
        Stage::Passed
    );
}

#[test]
fn a_contending_reader_waits_for_the_platform_lock() {
    let f = Fixture::new();
    let directory = f.queue();
    let (ready, receive) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        queue::transaction(&directory, |_s| {
            ready.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(80));
            Ok(())
        })
        .unwrap();
    });
    receive.recv().unwrap();
    assert!(queue::inspect(&f.queue()).is_ok());
    holder.join().unwrap();
}

#[test]
fn desktop_workers_use_registered_path_and_preserve_executable_entrypoints() {
    let f = Fixture::new();
    let cwd = f.directory.join("tools");
    fs::create_dir(&cwd).unwrap();
    fs::write(cwd.join("mode.txt"), "path").unwrap();
    let executable = std::env::current_exe().unwrap();
    #[cfg(unix)]
    let executable = {
        let entrypoint = f.directory.join("entrypoint");
        std::os::unix::fs::symlink(&executable, &entrypoint).unwrap();
        entrypoint
    };
    let output = Command::new(env!("CARGO_BIN_EXE_jitterbug-run"))
        .env("JITTERBUG_QUEUE_DIR", f.queue())
        .args(["queue", "add", "path-check", "--worktree"])
        .arg(&cwd)
        .arg("--")
        .arg(&executable)
        .args([
            "--exact",
            "validation_child_fixture",
            "--ignored",
            "--nocapture",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stored = queue::inspect(&f.queue()).unwrap();
    assert_eq!(stored.profiles[0].command[0], executable.to_string_lossy());
    assert_eq!(stored.profiles[0].search_path, std::env::var("PATH").ok());
    f.submit("path-check", "desktop-path");
    let started = Instant::now();
    loop {
        let output = f
            .tick_command(false)
            .env("PATH", &f.directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let state = queue::inspect(&f.queue()).unwrap();
        if !state.jobs[0].stage.active() {
            assert_eq!(
                state.jobs[0].stage,
                Stage::Passed,
                "{}",
                state.jobs[0].message
            );
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(15));
        std::thread::sleep(Duration::from_millis(80));
    }
}

#[test]
fn stdio_mcp_handshake_and_tools_work_without_a_model() {
    use std::process::Stdio;
    let f = Fixture::new();
    f.profile("check", "ok", false);
    let mut process = Command::new(env!("CARGO_BIN_EXE_jitterbug-run"))
        .args(["mcp", "--profiles", "check", "--harness", "test"])
        .env("JITTERBUG_QUEUE_DIR", f.queue())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = process.stdin.take().unwrap();
    for request in [
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"validation_profiles","arguments":{}}}),
    ] {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let output = process.wait_with_output().unwrap();
    assert!(output.status.success());
    let messages: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1]["result"]["tools"].as_array().unwrap().len(), 5);
    assert!(messages[2]["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("check"));
}

#[test]
fn workflow_controls_hold_prioritize_protect_and_resume_real_commands() {
    let f = Fixture::new();
    f.profile("check", "ok", false);
    let one = f.submit("check", "one");
    let two = f.submit("check", "two");
    let three = f.submit("check", "three");
    queue::control::change(&f.queue(), "hold_job", Some(&one.id)).unwrap();
    queue::control::change(&f.queue(), "run_next", Some(&three.id)).unwrap();
    queue::control::change(&f.queue(), "protect", None).unwrap();
    f.tick(false);
    assert!(!f.queue().join("events.txt").exists());
    // Protection expiry is evaluated by the native scheduler, without an open UI.
    queue::transaction(&f.queue(), |s| {
        s.control.protected_until = Some(now() - 1);
        Ok(())
    })
    .unwrap();
    f.tick(false);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = queue::inspect(&f.queue()).unwrap();
        if state
            .jobs
            .iter()
            .filter(|j| j.id != one.id)
            .all(|j| !j.stage.active())
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(80));
        f.tick(false);
    }
    let state = queue::inspect(&f.queue()).unwrap();
    assert_eq!(state.jobs[0].stage, Stage::Queued);
    assert!(state
        .view()
        .jobs
        .iter()
        .find(|j| j.id == one.id)
        .unwrap()
        .hold_reason
        .as_ref()
        .unwrap()
        .contains("Held"));
    let events = fs::read_to_string(f.queue().join("events.txt")).unwrap();
    assert_eq!(
        events.lines().next().unwrap(),
        format!("start {}", three.id)
    );
    assert!(events.contains(&format!("end {}", two.id)));
    queue::control::change(&f.queue(), "resume_job", Some(&one.id)).unwrap();
    f.settle();
    assert!(queue::control::change(&f.queue(), "run_next", Some(&one.id)).is_err());
    assert!(queue::inspect(&f.queue())
        .unwrap()
        .control
        .protection()
        .is_none());
}

#[test]
fn claude_codex_and_meta_checkpoints_acknowledge_pause_and_respect_scopes() {
    let f = Fixture::new();
    f.profile("check", "ok", false);
    f.profile("other", "ok", false);
    for harness in ["claude", "codex", "muse"] {
        let arguments = serde_json::json!({"profile":"check","session":harness,"task":"Synthetic task","phase":"editing","wait_seconds":0});
        let first = queue::mcp::call(
            &f.queue(),
            &["check".into()],
            harness,
            "workflow_checkpoint",
            &arguments,
        )
        .unwrap();
        assert_eq!(first["may_proceed"], true);
        let id = first["session_id"].as_str().unwrap().to_string();
        queue::control::change(&f.queue(), "pause_session", Some(&id)).unwrap();
        let job = queue::submit(
            &f.queue(),
            "check",
            harness,
            "Synthetic check",
            harness,
            harness,
        )
        .unwrap();
        f.tick(false);
        assert_eq!(
            queue::inspect(&f.queue())
                .unwrap()
                .jobs
                .iter()
                .find(|j| j.id == job.id)
                .unwrap()
                .stage,
            Stage::Queued
        );
        let directory = f.queue();
        let thread = std::thread::spawn(move || {
            queue::control::checkpoint(
                &directory,
                "check",
                harness,
                harness,
                "Synthetic task",
                "tests",
                5,
            )
            .unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if queue::inspect(&f.queue())
                .unwrap()
                .control
                .sessions
                .iter()
                .find(|s| s.id == id)
                .unwrap()
                .paused()
            {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(25));
        }
        queue::control::change(&f.queue(), "resume_session", Some(&id)).unwrap();
        assert!(thread.join().unwrap().may_proceed);
        f.settle();
        let mut forbidden = arguments.clone();
        forbidden["profile"] = serde_json::json!("other");
        assert!(queue::mcp::call(
            &f.queue(),
            &["check".into()],
            harness,
            "workflow_checkpoint",
            &forbidden
        )
        .is_err());
        let mut injection = arguments.clone();
        injection["command"] = serde_json::json!("arbitrary command");
        assert!(queue::mcp::call(
            &f.queue(),
            &["check".into()],
            harness,
            "workflow_checkpoint",
            &injection
        )
        .is_err());
    }
}

#[test]
fn checkpoint_timeout_is_not_permission_and_model_plans_cannot_expand_actions() {
    let f = Fixture::new();
    f.profile("check", "ok", false);
    let checkpoint = queue::control::checkpoint(
        &f.queue(),
        "check",
        "test",
        "session",
        "Synthetic task",
        "editing",
        0,
    )
    .unwrap();
    queue::control::change(&f.queue(), "pause_session", Some(&checkpoint.session_id)).unwrap();
    let denied = queue::control::checkpoint(
        &f.queue(),
        "check",
        "test",
        "session",
        "Synthetic task",
        "editing",
        0,
    )
    .unwrap();
    assert!(!denied.may_proceed);
    assert!(!queue::inspect(&f.queue()).unwrap().control.sessions[0].paused());
    let state = queue::inspect(&f.queue()).unwrap();
    for invalid in [
        r#"{"action":"terminate","job_id":null,"reason":"Stop it."}"#,
        r#"{"action":"run_next","job_id":"missing","reason":"Go."}"#,
        r#"{"action":"protect","job_id":null,"reason":"Hold","command":"rm"}"#,
    ] {
        assert!(queue::control::Plan::parse(invalid, &state).is_err());
    }
    assert!(queue::control::Plan::parse(r#"{"action":"protect","job_id":null,"reason":"Hold upcoming checks while your apps are busy."}"#, &state).is_ok());
}
