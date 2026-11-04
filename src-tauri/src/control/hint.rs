//! `jitterbug-run pressure-hint`: a PreToolUse hook. Before an agent runs a heavy command (a
//! build, a full test suite, an install, a container), it asks the running app how loaded the
//! machine is. Only under pressure does it add one short note to the agent's context, naming
//! the other sessions using the most, so the agent can wait, narrow the command, or ask
//! Jitterbug to pause one. It never blocks or changes the command, and stays silent when the
//! app is closed, agent access is off, or the machine is fine.
use serde_json::{json, Value};
use std::path::Path;

/// The shell command an agent is about to run, from a Claude Code or Codex PreToolUse event.
pub fn command_of(event: &Value) -> Option<String> {
    if event["hook_event_name"] != "PreToolUse" {
        return None;
    }
    let tool = event["tool_name"].as_str()?;
    if !matches!(tool, "Bash" | "exec_command" | "shell_command" | "shell") {
        return None;
    }
    let command = &event["tool_input"]["command"];
    match command {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    }
}

fn base(word: &str) -> String {
    Path::new(word)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(word)
        .to_ascii_lowercase()
}

/// Script names worth a check when run through a package manager.
fn heavy_script(name: &str) -> bool {
    [
        "build",
        "test",
        "e2e",
        "check",
        "lint",
        "typecheck",
        "compile",
        "bundle",
        "tauri",
        "dev",
    ]
    .iter()
    .any(|k| name.contains(k))
}

fn heavy_words(words: &[&str]) -> bool {
    // Skip environment assignments and wrappers that run the real command.
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        let wrapper = matches!(
            w,
            "sudo" | "time" | "nice" | "env" | "exec" | "command" | "nohup"
        );
        if wrapper || w.starts_with('-') || w.contains('=') {
            i += 1;
        } else {
            break;
        }
    }
    let Some(first) = words.get(i) else {
        return false;
    };
    let program = base(first);
    let rest: Vec<&str> = words[i + 1..].to_vec();
    let sub = rest
        .iter()
        .find(|w| !w.starts_with('-') && !w.starts_with('+'))
        .copied()
        .unwrap_or("");
    let sub_is = |names: &[&str]| names.contains(&sub);
    match program.as_str() {
        "cargo" => sub_is(&[
            "build", "b", "test", "t", "check", "c", "clippy", "run", "r", "bench", "nextest",
            "install", "doc", "tauri",
        ]),
        "npx" | "bunx" | "pnpx" => heavy_words(&rest),
        "npm" | "pnpm" | "yarn" | "bun" => match sub {
            "install" | "i" | "ci" | "add" | "build" | "test" | "t" => true,
            "exec" | "dlx" | "x" => {
                let at = rest.iter().position(|w| *w == sub).unwrap_or(0);
                heavy_words(&rest[at + 1..])
            }
            "run" | "run-script" => rest
                .iter()
                .skip_while(|w| **w != sub)
                .nth(1)
                .is_some_and(|name| heavy_script(name)),
            // `yarn` alone installs; `yarn build`, `pnpm test:e2e`, `bun dev` run a script.
            "" => program == "yarn",
            name => program != "npm" && heavy_script(name),
        },
        "tsc" | "vitest" | "jest" | "playwright" | "webpack" | "rollup" | "turbo" | "nx"
        | "make" | "gmake" | "cmake" | "ninja" | "bazel" | "bazelisk" | "gradle" | "gradlew"
        | "mvn" | "mvnw" | "xcodebuild" | "pytest" | "tox" | "nox" | "ffmpeg" | "svelte-check" => {
            true
        }
        "vite" | "next" | "astro" | "nuxt" | "remix" => sub == "build",
        "go" => sub_is(&["build", "test", "install", "vet"]),
        "swift" => sub_is(&["build", "test"]),
        "dotnet" => sub_is(&["build", "test", "run", "restore", "publish"]),
        "docker" | "podman" => sub_is(&["build", "run", "compose", "buildx"]),
        "docker-compose" => sub_is(&["up", "build", "run"]),
        "ollama" => sub_is(&["run", "pull", "serve", "create"]),
        "pip" | "pip3" | "uv" | "poetry" | "pipenv" | "conda" | "brew" => {
            sub_is(&["install", "sync", "upgrade", "update", "add"])
        }
        "python" | "python3" => {
            rest.first() == Some(&"-m")
                && rest
                    .get(1)
                    .is_some_and(|m| matches!(*m, "pytest" | "pip" | "tox" | "build"))
        }
        _ => false,
    }
}

/// Whether a command does the kind of work that loads the whole machine. Quick commands,
/// reads, and edits are never worth a check.
pub fn heavy(command: &str) -> bool {
    command
        .split(['\n', ';', '|', '&', '(', ')'])
        .map(|segment| segment.split_whitespace().collect::<Vec<_>>())
        .any(|words| heavy_words(&words))
}

fn mib_text(mib: u64) -> String {
    if mib >= 1024 {
        format!("{:.1} GB", mib as f64 / 1024.0)
    } else {
        format!("{mib} MB")
    }
}

fn session_text(s: &Value) -> String {
    let title = s["title"].as_str().unwrap_or("Untitled session");
    let memory = mib_text(s["memory_mib"].as_u64().unwrap_or(0));
    let mut text = match s["agent_status"].as_str() {
        Some(status) => format!("{title} is {status} at {memory}"),
        None => format!("{title} is using {memory}"),
    };
    let cpu = s["cpu_percent_of_machine"]
        .as_f64()
        .unwrap_or_default()
        .round();
    if cpu >= 1.0 {
        text.push_str(&format!(", {cpu}% CPU"));
    }
    format!(
        "{text} (unit_id {})",
        s["unit_id"].as_str().unwrap_or_default()
    )
}

/// The note added to the agent's context, from the app's `pressure_hint` answer, e.g.
/// "Jitterbug: this machine is under memory strain (1.1 GB free, CPU 40%). TEST2 is idle at
/// 1.2 GB (unit_id …). Before more heavy commands, …consider jitterbug_pause_session…or waiting."
pub fn message(answer: &Value) -> Option<String> {
    if answer["hint"] != json!(true) {
        return None;
    }
    let strain = match answer["strain"].as_str() {
        Some("memory") => "memory strain",
        Some("cpu") => "CPU strain",
        Some(_) => "memory and CPU strain",
        None => "sustained pressure",
    };
    let mut text = format!(
        "Jitterbug: this machine is under {strain} ({} GB memory free, CPU {}%).",
        answer["memory_available_gib"].as_f64().unwrap_or_default(),
        answer["cpu_percent"].as_f64().unwrap_or_default().round()
    );
    let sessions: Vec<String> = answer["sessions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(session_text)
        .collect();
    if sessions.is_empty() {
        text.push_str(" Before more heavy commands, mention it in one line, then wait or run narrower commands where that would do.");
    } else {
        text.push_str(&format!(
            " {}. Before more heavy commands, mention it in one line, then consider asking Jitterbug to pause the least urgent idle session with jitterbug_pause_session (jitterbug-control MCP), running narrower commands, or waiting. Never pause work the person is waiting on.",
            sessions.join("; ")
        ));
    }
    Some(text)
}

/// Run the hook on one event. Returns the JSON to print, or nothing to stay silent.
pub fn hook(input: &str) -> Option<String> {
    let event: Value = serde_json::from_str(input).ok()?;
    // Subagents share the main session's machine; one note to the main session is enough.
    if event.get("agent_id").is_some_and(|v| !v.is_null()) {
        return None;
    }
    if !heavy(&command_of(&event)?) {
        return None;
    }
    let params = json!({ "session_id": event["session_id"].as_str().unwrap_or("") });
    let answer = super::request("pressure_hint", &params).ok()?;
    let note = message(&answer)?;
    Some(
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "additionalContext": note,
            }
        })
        .to_string(),
    )
}

/// The PreToolUse entry that runs the hook. Only shell commands trigger it, and quick ones
/// return without contacting the app.
pub fn entry(binary: &Path) -> Result<Value, String> {
    let command = crate::harness::hook_command(binary, "pressure-hint")?;
    Ok(json!({
        "matcher": "Bash",
        "hooks": [{ "type": "command", "command": command, "timeout": 5 }]
    }))
}

/// Hook settings for Claude Code (`~/.claude/settings.json`) or a Codex hooks file.
pub fn settings(binary: &Path) -> Result<Value, String> {
    Ok(json!({ "hooks": { "PreToolUse": [entry(binary)?] } }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_machine_loading_commands_are_heavy() {
        for command in [
            "cargo test --manifest-path src-tauri/Cargo.toml --lib",
            "cargo +nightly build",
            "cd app && npm install",
            "npm run build",
            "npm run test:e2e",
            "pnpm test",
            "yarn",
            "bun dev",
            "npx playwright test",
            "npx -y vitest run",
            "pnpm exec tsc --noEmit",
            "RUST_LOG=debug cargo run",
            "time make -j8",
            "./gradlew assembleDebug",
            "docker compose up",
            "python3 -m pytest tests",
            "uv sync",
            "go test ./...",
            "git status && npm ci",
        ] {
            assert!(heavy(command), "{command} should be heavy");
        }
        for command in [
            "ls -la",
            "git status",
            "git commit -m 'run the build'",
            "cat package.json | grep build",
            "rg cargo src",
            "npm view react version",
            "cargo fmt",
            "docker ps",
            "python3 script.py",
            "echo cargo build",
            "vite",
            "",
        ] {
            assert!(!heavy(command), "{command} should not be heavy");
        }
    }

    #[test]
    fn reads_claude_and_codex_shell_events_only() {
        let claude = json!({"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"cargo build"}});
        assert_eq!(command_of(&claude).as_deref(), Some("cargo build"));
        let codex = json!({"hook_event_name":"PreToolUse","tool_name":"exec_command","tool_input":{"command":["npm","test"]}});
        assert_eq!(command_of(&codex).as_deref(), Some("npm test"));
        let edit = json!({"hook_event_name":"PreToolUse","tool_name":"Edit","tool_input":{"command":"cargo build"}});
        assert!(command_of(&edit).is_none());
        let post = json!({"hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"cargo build"}});
        assert!(command_of(&post).is_none());
        // Quick commands and subagents never reach the app.
        assert!(hook(&claude.to_string().replace("cargo build", "ls")).is_none());
        let mut sub = claude.clone();
        sub["agent_id"] = json!("child");
        assert!(hook(&sub.to_string()).is_none());
        assert!(hook("not json").is_none());
    }

    #[test]
    fn notes_appear_only_when_the_app_says_so_and_name_other_sessions() {
        assert!(message(&json!({"enabled": false, "hint": false})).is_none());
        assert!(message(&json!({"enabled": true, "hint": false, "level": "high"})).is_none());
        let note = message(&json!({
            "enabled": true, "hint": true, "level": "high", "strain": "memory",
            "cpu_percent": 41.4, "memory_available_gib": 1.1,
            "sessions": [
                {"unit_id": "session:42-7", "title": "TEST2", "agent_status": "idle", "memory_mib": 1229, "cpu_percent_of_machine": 0.4},
                {"unit_id": "session:51-9", "title": "api", "memory_mib": 700, "cpu_percent_of_machine": 12.0}
            ]
        }))
        .unwrap();
        assert!(note.starts_with(
            "Jitterbug: this machine is under memory strain (1.1 GB memory free, CPU 41%)."
        ));
        assert!(note.contains(
            "TEST2 is idle at 1.2 GB (unit_id session:42-7); api is using 700 MB, 12% CPU (unit_id session:51-9)."
        ));
        assert!(note.contains("jitterbug_pause_session"));
        assert!(note.contains("or waiting"));
        let cpu = message(&json!({"hint": true, "level": "high", "strain": "cpu", "cpu_percent": 93.0, "memory_available_gib": 6.0, "sessions": []})).unwrap();
        assert!(cpu.contains("under CPU strain"));
        assert!(!cpu.contains("jitterbug_pause_session"));
    }

    #[test]
    #[cfg(not(windows))]
    fn settings_run_the_hook_for_shell_commands_only() {
        let value = settings(Path::new("/opt/it's/jitterbug-run")).unwrap();
        let entry = &value["hooks"]["PreToolUse"][0];
        assert_eq!(entry["matcher"], "Bash");
        assert_eq!(
            entry["hooks"][0]["command"],
            "'/opt/it'\"'\"'s/jitterbug-run' pressure-hint"
        );
    }
}
