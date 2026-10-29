use jitterbug_lib::{
    queue::{
        self,
        manage::{self, connections, discovery, Request},
    },
    types::Snapshot,
};
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

struct Fixture {
    root: PathBuf,
    queue: PathBuf,
    project: PathBuf,
    locations: connections::Locations,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "jitterbug-enrollment-{}-{unique}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let project = root.join("project");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("package.json"), r#"{"scripts":{"check":"node --test","precheck":"node prepare.cjs","dev":"vite","test:watch":"vitest --watch","build":"vite build"}}"#).unwrap();
        let locations = connections::Locations {
            claude: root.join("claude.json"),
            codex: root.join("codex.toml"),
            meta: root.join("muse.json"),
        };
        fs::write(&locations.claude, r#"{"theme":"light","apiKey":"fixture-secret","mcpServers":{"other":{"command":"other-server"}}}"#).unwrap();
        fs::write(&locations.codex, "# Keep this comment\nmodel = 'fixture-model'\n[mcp_servers.other]\ncommand = 'other-server'\n").unwrap();
        fs::write(&locations.meta, r#"{"schema_version":1,"model":"fixture-model","mcpServers":{"other":{"type":"stdio","command":"other-server"}}}"#).unwrap();
        Self {
            queue: root.join("queue"),
            root,
            project,
            locations,
        }
    }
    fn request(&self, name: &str, clients: &[&str]) -> Request {
        Request {
            name: name.into(),
            worktree: self.project.clone(),
            command: vec![
                env!("CARGO_BIN_EXE_jitterbug-run").into(),
                "queue".into(),
                "--help".into(),
            ],
            memory_mib: 512,
            clients: clients.iter().map(|s| (*s).into()).collect(),
        }
    }
    fn preview(&self, name: &str, clients: &[&str]) -> manage::Preview {
        manage::prepare(
            &self.queue,
            &self.locations,
            &PathBuf::from(env!("CARGO_BIN_EXE_jitterbug-run")),
            self.request(name, clients),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
#[ignore = "Owned bounded child used for process discovery"]
fn check() {
    std::thread::sleep(std::time::Duration::from_secs(2));
}

#[test]
fn on_demand_discovery_requires_fresh_process_identity() {
    let f = Fixture::new();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "check", "--ignored"])
        .current_dir(&f.project)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let identity = queue::identity(child.id()).unwrap();
    let mut snapshot = Snapshot {
        processes: vec![jitterbug_lib::types::ProcessRecord {
            pid: child.id(),
            start_time: identity.start,
            name: "fixture".into(),
            workload_id: "fixture-workload".into(),
            memory_bytes: 100 * 1024 * 1024,
            ..Default::default()
        }],
        ..Default::default()
    };
    let selection = discovery::Selection {
        workload_id: Some("fixture-workload".into()),
        pid: Some(identity.pid),
        start_time: Some(identity.start),
        ..Default::default()
    };
    let found = discovery::discover(&snapshot, &selection).unwrap();
    assert!(found
        .candidates
        .iter()
        .any(|c| c.label == "Observed fixture"));
    snapshot.processes[0].start_time += 1;
    assert!(discovery::discover(&snapshot, &selection).is_err());
    assert!(child.wait().unwrap().success());
}

#[test]
fn revocation_cancels_waiters_but_preserves_running_work_and_denies_new_submissions() {
    let f = Fixture::new();
    manage::apply(&f.preview("approved", &[]).token).unwrap();
    let first = queue::submit(
        &f.queue,
        "approved",
        "first",
        "Synthetic run",
        "manual",
        "test",
    )
    .unwrap();
    let second = queue::submit(
        &f.queue,
        "approved",
        "second",
        "Synthetic waiter",
        "manual",
        "test",
    )
    .unwrap();
    queue::transaction(&f.queue, |s| {
        s.jobs[0].stage = queue::Stage::Running;
        Ok(())
    })
    .unwrap();
    manage::revoke_profile(&f.queue, "approved").unwrap();
    let state = queue::inspect(&f.queue).unwrap();
    assert!(state.profiles.is_empty());
    assert_eq!(
        state.jobs.iter().find(|j| j.id == first.id).unwrap().stage,
        queue::Stage::Running
    );
    assert_eq!(
        state.jobs.iter().find(|j| j.id == second.id).unwrap().stage,
        queue::Stage::Cancelled
    );
    assert!(queue::submit(&f.queue, "approved", "third", "Denied", "manual", "test").is_err());
}

#[test]
fn approval_preserves_settings_and_scopes_all_three_clients() {
    let f = Fixture::new();
    let before = fs::read(&f.locations.claude).unwrap();
    let preview = f.preview("approved-check", &connections::CLIENTS);
    assert_eq!(fs::read(&f.locations.claude).unwrap(), before);
    assert!(queue::inspect(&f.queue).unwrap().profiles.is_empty());
    assert_eq!(preview.changes.len(), 3);
    assert!(!serde_json::to_string(&preview)
        .unwrap()
        .contains("fixture-secret"));
    manage::apply(&preview.token).unwrap();
    assert_eq!(queue::inspect(&f.queue).unwrap().profiles.len(), 1);
    assert!(
        queue::inspect(&f.queue).unwrap().jobs.is_empty(),
        "approval must not execute work"
    );
    let claude: serde_json::Value =
        serde_json::from_slice(&fs::read(&f.locations.claude).unwrap()).unwrap();
    assert_eq!(claude["apiKey"], "fixture-secret");
    assert_eq!(claude["mcpServers"]["other"]["command"], "other-server");
    let codex = fs::read_to_string(&f.locations.codex).unwrap();
    assert!(codex.contains("# Keep this comment"));
    assert!(codex.contains("fixture-model"));
    let overview = manage::overview(&f.queue, &f.locations).unwrap();
    assert_eq!(overview[0].state, "managed");
    assert!(overview[0]
        .connections
        .iter()
        .all(|c| c.configured && c.last_seen.is_none()));
    assert!(
        manage::apply(&preview.token).is_err(),
        "approval tokens are single-use"
    );
    let next = f.preview("second-check", &connections::CLIENTS);
    manage::apply(&next.token).unwrap();
    for client in connections::CLIENTS {
        let receipt = connections::read_receipt(&f.queue, client).unwrap();
        assert_eq!(receipt.profiles, ["approved-check", "second-check"]);
    }
}

#[test]
fn changed_settings_or_scripts_invalidate_review_without_partial_writes() {
    let f = Fixture::new();
    let preview = f.preview("stale", &connections::CLIENTS);
    let claude_before = fs::read(&f.locations.claude).unwrap();
    fs::write(&f.locations.codex, "model = 'new-value'\n").unwrap();
    assert!(manage::apply(&preview.token)
        .unwrap_err()
        .contains("changed since preview"));
    assert_eq!(fs::read(&f.locations.claude).unwrap(), claude_before);
    assert!(queue::inspect(&f.queue).unwrap().profiles.is_empty());
    let preview = f.preview("stale", &[]);
    fs::write(f.project.join("package.json"), "{}").unwrap();
    assert!(manage::apply(&preview.token)
        .unwrap_err()
        .contains("scripts changed"));
    assert!(queue::inspect(&f.queue).unwrap().profiles.is_empty());
}

#[test]
fn connection_drift_is_detected_and_never_overwritten() {
    let f = Fixture::new();
    manage::apply(&f.preview("approved", &["codex"]).token).unwrap();
    let changed = fs::read_to_string(&f.locations.codex)
        .unwrap()
        .replace("approved", "unapproved");
    fs::write(&f.locations.codex, &changed).unwrap();
    assert!(
        !connections::status(&f.queue, &f.locations, "approved")
            .iter()
            .find(|c| c.client == "codex")
            .unwrap()
            .configured
    );
    assert!(manage::prepare(
        &f.queue,
        &f.locations,
        &PathBuf::from(env!("CARGO_BIN_EXE_jitterbug-run")),
        f.request("another", &["codex"])
    )
    .is_err());
    assert_eq!(fs::read_to_string(&f.locations.codex).unwrap(), changed);
}

#[test]
fn muse_saved_settings_use_the_versioned_file_schema_not_cli_override_names() {
    let f = Fixture::new();
    fs::remove_file(&f.locations.meta).unwrap();
    let preview = f.preview("muse-check", &["meta"]);
    assert!(preview.changes[0].description.contains("schema_version"));
    manage::apply(&preview.token).unwrap();
    let mut doc: serde_json::Value =
        serde_json::from_slice(&fs::read(&f.locations.meta).unwrap()).unwrap();
    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["mcpServers"][connections::SERVER]["type"], "stdio");
    assert!(doc.get("mcp_servers").is_none());
    doc["schema_version"] = json!(2);
    fs::write(&f.locations.meta, serde_json::to_vec(&doc).unwrap()).unwrap();
    assert!(manage::prepare(
        &f.queue,
        &f.locations,
        &PathBuf::from(env!("CARGO_BIN_EXE_jitterbug-run")),
        f.request("new", &["meta"])
    )
    .is_err());
}

#[test]
fn discovery_filters_watch_scripts_and_manual_profiles_need_review() {
    let f = Fixture::new();
    let discovery = discovery::discover(
        &Snapshot::default(),
        &discovery::Selection {
            worktree: Some(f.project.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(discovery.candidates.len(), 2);
    assert!(discovery
        .candidates
        .iter()
        .all(|c| c.command[0] == "npm" && c.memory_mib == 2048));
    assert!(discovery
        .candidates
        .iter()
        .any(|c| c.script.as_ref().unwrap().contains("precheck:")));
    assert!(queue::inspect(&f.queue).unwrap().profiles.is_empty());
    let mut request = f.request("secret", &[]);
    request.command.push("--api-key=fixture-value".into());
    assert!(manage::prepare(
        &f.queue,
        &f.locations,
        &PathBuf::from(env!("CARGO_BIN_EXE_jitterbug-run")),
        request
    )
    .unwrap_err()
    .contains("credentials"));
}

#[test]
fn real_mcp_handshake_marks_contact_and_enforces_the_saved_scope() {
    let f = Fixture::new();
    manage::apply(&f.preview("approved", &connections::CLIENTS).token).unwrap();
    for client in connections::CLIENTS {
        let receipt = connections::read_receipt(&f.queue, client).unwrap();
        let mut child = Command::new(receipt.entry["command"].as_str().unwrap())
            .args(
                receipt.entry["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap()),
            )
            .env("JITTERBUG_QUEUE_DIR", &f.queue)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"enrollment-fixture","version":"1"}}}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"validation_profiles","arguments":{}}}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"validation_submit","arguments":{"profile":"unapproved","request_id":"no","task":"no","session":"no"}}}),
        ].iter().map(|v| format!("{v}\n")).collect::<String>();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let replies: Vec<serde_json::Value> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert!(replies[0]["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("exact worktree"));
        assert_eq!(replies[2]["result"]["isError"], true);
        assert!(connections::status(&f.queue, &f.locations, "approved")
            .iter()
            .find(|c| c.client == client)
            .unwrap()
            .last_seen
            .is_some());
    }
    assert!(queue::inspect(&f.queue).unwrap().jobs.is_empty());
}

#[test]
fn desktop_executable_serves_mcp_without_starting_a_window() {
    let f = Fixture::new();
    let mut child = Command::new(env!("CARGO_BIN_EXE_jitterbug"))
        .args([
            "mcp",
            "--profiles",
            "fixture",
            "--harness",
            "codex",
            "--connection",
            "codex",
        ])
        .env("JITTERBUG_QUEUE_DIR", &f.queue)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        response["result"]["serverInfo"]["name"],
        "jitterbug-validation"
    );
}

#[cfg(unix)]
#[test]
fn linked_config_files_and_parents_require_manual_setup() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let target = f.root.join("target.json");
    fs::rename(&f.locations.claude, &target).unwrap();
    symlink(&target, &f.locations.claude).unwrap();
    assert!(manage::prepare(
        &f.queue,
        &f.locations,
        &PathBuf::from(env!("CARGO_BIN_EXE_jitterbug-run")),
        f.request("linked", &["claude"])
    )
    .is_err());
    let parent = f.root.join("linked-parent");
    symlink(&f.project, &parent).unwrap();
    assert!(manage::read_bounded(&parent.join("package.json"), 256 * 1024).is_err());
}
