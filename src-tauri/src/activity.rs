//! What a coding agent is doing right now, read from the end of its own local session log:
//! Claude Code's transcript (and each subagent's), or a Codex conversation log. Only the latest
//! tool call, the latest request, and when it last did anything are kept. Logs are internal
//! formats read best-effort; anything unrecognized just leaves the activity blank.
//! Activity text stays on this computer: model prompts get only a path-free summary.
use serde::Serialize;
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// How much of a log's end is read. Recent entries are enough; logs can be hundreds of MiB.
const TAIL_BYTES: u64 = 192 * 1024;
/// A subagent whose transcript changed within this window is still at work.
const SUBAGENT_ACTIVE: u64 = 90;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Activity {
    /// "Running cargo test", "Editing Agents.svelte", "Thinking", or "Waiting for you".
    pub doing: Option<String>,
    /// The latest request from the person, shortened.
    pub request: Option<String>,
    /// Unix time of the last entry in the log.
    pub last_active: Option<u64>,
    /// The agent finished its turn and is waiting for the person.
    pub waiting_for_you: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Subagent {
    pub label: String,
    pub doing: Option<String>,
    pub last_active: u64,
}

fn modified(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The last lines of a log, oldest first. A partial first line is dropped.
fn tail_lines(path: &Path) -> Vec<String> {
    let Ok(mut file) = File::open(path) else {
        return vec![];
    };
    let length = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = length.saturating_sub(TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return vec![];
    }
    let mut bytes = Vec::new();
    if file.take(TAIL_BYTES).read_to_end(&mut bytes).is_err() {
        return vec![];
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    lines
}

fn clean(text: &str, max: usize) -> String {
    let flat: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    if flat.chars().count() <= max {
        flat
    } else {
        format!(
            "{}…",
            flat.chars().take(max - 1).collect::<String>().trim_end()
        )
    }
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string()
}

/// A shell command without directories: `cargo test --lib`, not its full paths.
fn command_summary(command: &str) -> String {
    let first = command.lines().next().unwrap_or_default();
    let words: Vec<String> = first
        .split_whitespace()
        .take(6)
        .map(|w| {
            if w.contains('/') {
                file_name(w.trim_matches(|c| c == '"' || c == '\''))
            } else {
                w.to_string()
            }
        })
        .collect();
    clean(&words.join(" "), 60)
}

/// Present-tense description of one Claude Code tool call.
fn claude_tool(name: &str, input: &Value) -> String {
    let text = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or_default();
    match name {
        "Bash" => {
            let description = text("description");
            if description.is_empty() {
                format!("Running {}", command_summary(text("command")))
            } else {
                clean(description, 70)
            }
        }
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => {
            format!("Editing {}", file_name(text("file_path")))
        }
        "Read" => format!("Reading {}", file_name(text("file_path"))),
        "Grep" | "Glob" => "Searching the code".into(),
        "Task" | "Agent" => format!("Delegating: {}", clean(text("description"), 50)),
        "WebSearch" | "WebFetch" => "Searching the web".into(),
        "TodoWrite" | "TaskCreate" | "TaskUpdate" => "Planning".into(),
        other if other.starts_with("mcp__") => {
            let server = other.split("__").nth(1).unwrap_or("a connected");
            format!("Using {} tools", server.replace(['_', '-'], " "))
        }
        other => format!("Using {}", clean(other, 30)),
    }
}

fn timestamp(value: &Value) -> Option<u64> {
    let text = value.get("timestamp")?.as_str()?;
    // RFC 3339 in UTC: 2026-11-04T04:17:29.123Z
    let date = text.get(0..19)?;
    let (day, time) = date.split_once('T')?;
    let mut d = day.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, dd) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|p| p.parse::<i64>().ok());
    let (hh, mm, ss) = (t.next()??, t.next()??, t.next()??);
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + dd - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3600 + mm * 60 + ss).ok()
}

/// Read a Claude Code transcript (the main conversation or one subagent).
pub fn claude(transcript: &Path) -> Activity {
    let mut activity = Activity {
        last_active: modified(transcript),
        ..Default::default()
    };
    for line in tail_lines(transcript).iter().rev() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = entry
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind == "last-prompt" && activity.request.is_none() {
            activity.request = entry
                .get("lastPrompt")
                .and_then(Value::as_str)
                .map(|p| clean(p, 140))
                .filter(|p| !p.is_empty());
            continue;
        }
        if activity.doing.is_some() {
            if activity.request.is_some() {
                break;
            }
            continue;
        }
        let content = entry.pointer("/message/content");
        match kind {
            "assistant" => {
                let items = content
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if let Some(tool) = items
                    .iter()
                    .rev()
                    .find(|i| i.get("type").and_then(Value::as_str) == Some("tool_use"))
                {
                    let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
                    let input = tool.get("input").cloned().unwrap_or(Value::Null);
                    activity.doing = Some(claude_tool(name, &input));
                } else if items
                    .iter()
                    .any(|i| i.get("type").and_then(Value::as_str) == Some("thinking"))
                {
                    activity.doing = Some("Thinking".into());
                } else if items
                    .iter()
                    .any(|i| i.get("type").and_then(Value::as_str) == Some("text"))
                {
                    activity.doing = Some("Writing a reply".into());
                }
            }
            "user" => {
                // A fresh request with nothing after it yet: the agent is starting on it.
                let is_result = content.and_then(Value::as_array).is_some_and(|items| {
                    items
                        .iter()
                        .any(|i| i.get("type").and_then(Value::as_str) == Some("tool_result"))
                });
                activity.doing = Some(
                    if is_result {
                        "Thinking"
                    } else {
                        "Reading your request"
                    }
                    .into(),
                );
            }
            _ => {}
        }
    }
    activity
}

/// Claude Code keeps each session's transcript under a folder named after its working
/// directory. The session may have moved folders, so look in every project for its ID.
pub fn claude_transcript(session_id: &str) -> Option<PathBuf> {
    let projects = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude")))?
        .join("projects");
    let name = format!("{session_id}.jsonl");
    fs::read_dir(projects)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(&name))
        .filter(|path| path.is_file())
        .max_by_key(|path| modified(path).unwrap_or(0))
}

/// Subagents of a Claude Code session that are working right now.
pub fn claude_subagents(transcript: &Path) -> Vec<Subagent> {
    let (Some(dir), Some(stem)) = (transcript.parent(), transcript.file_stem()) else {
        return vec![];
    };
    let Ok(entries) = fs::read_dir(dir.join(stem).join("subagents")) else {
        return vec![];
    };
    let at = now();
    let mut out: Vec<Subagent> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|path| {
            let changed = modified(&path)?;
            if changed + SUBAGENT_ACTIVE < at {
                return None;
            }
            let label = fs::read(path.with_extension("meta.json"))
                .ok()
                .filter(|b| b.len() < 64 * 1024)
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| v.get("description")?.as_str().map(|d| clean(d, 80)))
                .unwrap_or_else(|| "Subagent".into());
            Some(Subagent {
                label,
                doing: claude(&path).doing,
                last_active: changed,
            })
        })
        .collect();
    out.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    out.truncate(8);
    out
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
}

/// Codex conversation logs changed within the last day, newest first, with their folder.
pub fn codex_conversations() -> Vec<(PathBuf, String)> {
    let Some(root) = codex_home().map(|h| h.join("sessions")) else {
        return vec![];
    };
    let at = now();
    let mut found = vec![];
    // sessions/YYYY/MM/DD/rollout-*.jsonl; only the two newest days can hold recent logs.
    let mut days: Vec<PathBuf> = vec![];
    for year in read_sorted(&root).into_iter().rev().take(1) {
        for month in read_sorted(&year).into_iter().rev().take(2) {
            days.extend(read_sorted(&month).into_iter().rev().take(2));
        }
    }
    for day in days {
        for path in read_sorted(&day) {
            let Some(changed) = modified(&path) else {
                continue;
            };
            if changed + 86_400 < at || path.extension().is_none_or(|x| x != "jsonl") {
                continue;
            }
            match codex_meta(&path) {
                // A finished headless run (`codex exec`) is nobody's open conversation.
                Some((_, origin)) if origin.contains("exec") && changed + 120 < at => {}
                Some((cwd, _)) => found.push((changed, path, cwd)),
                None => {}
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().take(12).map(|(_, p, c)| (p, c)).collect()
}

fn read_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map(|r| r.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.sort();
    paths
}

/// A Codex conversation's folder and where it came from (`Codex Desktop`, `codex_exec`, …).
fn codex_meta(path: &Path) -> Option<(String, String)> {
    let mut first = String::new();
    let file = File::open(path).ok()?;
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(file.take(256 * 1024)),
        &mut first,
    )
    .ok()?;
    let value: Value = serde_json::from_str(&first).ok()?;
    let text = |key: &str| {
        value
            .pointer(&format!("/payload/{key}"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    Some((
        text("cwd")?,
        text("originator").unwrap_or_default().to_lowercase(),
    ))
}

/// The command inside a Codex tool call: plain arguments, or a script calling `exec_command`.
fn codex_command(call: &Value) -> Option<String> {
    let raw = call
        .get("input")
        .or_else(|| call.get("arguments"))
        .and_then(Value::as_str)?;
    if let Ok(args) = serde_json::from_str::<Value>(raw) {
        match args.get("command").or_else(|| args.get("cmd")) {
            Some(Value::Array(parts)) => {
                let words: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
                let words = if words.len() >= 3 && words[1] == "-lc" {
                    words[2..].to_vec()
                } else {
                    words
                };
                return Some(command_summary(&words.join(" ")));
            }
            Some(Value::String(s)) => return Some(command_summary(s)),
            _ => {}
        }
    }
    let start = raw.find("cmd:\"").or_else(|| raw.find("cmd: \""))? + 5;
    let rest = raw[start..].trim_start_matches(['"', ' ']);
    let end = rest.find('"').unwrap_or(rest.len());
    Some(command_summary(&rest[..end].replace("\\n", "\n")))
}

/// What the person typed, from either form Codex logs: a `user_message` event (CLI) or a user
/// message item whose text parts include injected context (desktop app).
fn codex_user_text(payload: &Value) -> Option<String> {
    let text = match payload.get("type").and_then(Value::as_str)? {
        "user_message" => payload.get("message")?.as_str()?.to_string(),
        "message" if payload.get("role").and_then(Value::as_str) == Some("user") => payload
            .get("content")?
            .as_array()?
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("input_text"))
            .filter_map(|part| part.get("text")?.as_str())
            .map(str::trim)
            .rfind(|t| !t.is_empty() && !t.starts_with('#') && !t.starts_with('<'))?
            .to_string(),
        _ => return None,
    };
    Some(clean(&text, 140)).filter(|t| !t.is_empty())
}

/// The latest request in a Codex log. Logs grow to many MiB and requests can be far from the
/// end, so each log is scanned once and then only from where the previous scan stopped.
fn codex_request(log: &Path) -> Option<String> {
    use std::collections::HashMap;
    use std::io::BufRead;
    use std::sync::Mutex;
    /// Log → (bytes already scanned, latest request found so far).
    type Scanned = HashMap<PathBuf, (u64, Option<String>)>;
    static SCANNED: Mutex<Option<Scanned>> = Mutex::new(None);
    let length = fs::metadata(log).ok()?.len();
    let mut guard = SCANNED.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    let (offset, mut request) = cache.get(log).cloned().unwrap_or((0, None));
    let offset = if offset > length { 0 } else { offset };
    if offset < length {
        let mut file = File::open(log).ok()?;
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut reader = std::io::BufReader::new(file);
        let mut consumed = offset;
        let mut line = String::new();
        loop {
            line.clear();
            let read = reader.read_line(&mut line).ok()?;
            if read == 0 || !line.ends_with('\n') {
                break;
            }
            consumed += read as u64;
            if line.contains("\"user_message\"") || line.contains("\"role\":\"user\"") {
                if let Some(text) = serde_json::from_str::<Value>(&line)
                    .ok()
                    .and_then(|v| codex_user_text(v.get("payload")?))
                {
                    request = Some(text);
                }
            }
        }
        if cache.len() > 64 {
            cache.clear();
        }
        cache.insert(log.to_path_buf(), (consumed, request.clone()));
    }
    request
}

pub fn codex(log: &Path) -> Activity {
    let mut activity = Activity {
        last_active: modified(log),
        ..Default::default()
    };
    for line in tail_lines(log).iter().rev() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let payload = entry.get("payload").cloned().unwrap_or(Value::Null);
        let kind = payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if activity.request.is_none() {
            activity.request = codex_user_text(&payload);
        }
        if activity.doing.is_none() {
            activity.doing = match kind {
                "task_complete" | "turn_complete" => {
                    activity.waiting_for_you = true;
                    Some("Waiting for you".into())
                }
                "custom_tool_call" | "function_call" | "local_shell_call" => {
                    Some(match codex_command(&payload) {
                        Some(command) if !command.is_empty() => format!("Running {command}"),
                        _ => "Using a tool".into(),
                    })
                }
                "reasoning" => Some("Thinking".into()),
                "agent_message" => Some("Writing a reply".into()),
                _ => None,
            };
        }
        if activity.doing.is_some() && activity.request.is_some() {
            break;
        }
    }
    if activity.request.is_none() {
        activity.request = codex_request(log);
    }
    if activity.last_active.is_none() {
        activity.last_active = tail_lines(log)
            .last()
            .and_then(|l| serde_json::from_str::<Value>(l).ok())
            .and_then(|v| timestamp(&v));
    }
    activity
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(lines: &[Value]) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "jb-activity-{}-{}.jsonl",
            std::process::id(),
            lines.len()
        ));
        let text: Vec<String> = lines.iter().map(Value::to_string).collect();
        fs::write(&path, text.join("\n")).unwrap();
        path
    }

    #[test]
    fn claude_reports_the_latest_tool_and_request() {
        let path = write(&[
            json!({"type":"user","message":{"content":"run the tests"}}),
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test --manifest-path /Users/x/app/Cargo.toml"}}]}}),
            json!({"type":"last-prompt","lastPrompt":"run the tests please"}),
        ]);
        let activity = claude(&path);
        assert_eq!(
            activity.doing.as_deref(),
            Some("Running cargo test --manifest-path Cargo.toml")
        );
        assert_eq!(activity.request.as_deref(), Some("run the tests please"));
        fs::remove_file(path).ok();
    }

    #[test]
    fn tool_calls_read_as_plain_actions() {
        assert_eq!(
            claude_tool("Edit", &json!({"file_path":"/a/b/Agents.svelte"})),
            "Editing Agents.svelte"
        );
        assert_eq!(
            claude_tool(
                "Bash",
                &json!({"command":"x","description":"Run the UI tests"})
            ),
            "Run the UI tests"
        );
        assert_eq!(
            claude_tool("mcp__computer-use__app_click", &json!({})),
            "Using computer use tools"
        );
    }

    #[test]
    fn codex_commands_are_found_in_plain_and_scripted_calls() {
        let plain = json!({"type":"function_call","arguments":"{\"command\":[\"bash\",\"-lc\",\"npm test\"]}"});
        assert_eq!(codex_command(&plain).as_deref(), Some("npm test"));
        let scripted = json!({"type":"custom_tool_call","input":"text(await tools.exec_command({cmd:\"rg -n foo src/app.ts\"}))"});
        assert_eq!(
            codex_command(&scripted).as_deref(),
            Some("rg -n foo app.ts")
        );
        let path = write(&[
            json!({"type":"event_msg","payload":{"type":"user_message","message":"fix the build"}}),
            json!({"type":"response_item","payload":plain}),
        ]);
        let activity = codex(&path);
        assert_eq!(activity.doing.as_deref(), Some("Running npm test"));
        assert_eq!(activity.request.as_deref(), Some("fix the build"));
        fs::remove_file(path).ok();
    }

    #[test]
    fn timestamps_parse_as_utc() {
        assert_eq!(
            timestamp(&json!({"timestamp":"1970-01-02T00:00:01.5Z"})),
            Some(86_401)
        );
    }
}
