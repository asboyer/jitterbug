//! Opt-in harness events. Keep bounded intent and tool categories, never transcripts or tool I/O.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

pub const MAX_EVENT_BYTES: u64 = 64 * 1024;
const TOOLS: &[&str] = &["read", "edit", "command", "other"];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Context {
    pub session_id: String,
    pub request: String,
    pub latest_request: String,
    pub source: String,
    pub updated_at: u64,
    pub last_activity: String,
    pub last_activity_at: u64,
    pub completed_tools: BTreeMap<String, u32>,
}

impl Context {
    pub fn valid(&self) -> bool {
        self.session_id.len() <= 128
            && self
                .session_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            && self.request.chars().count() <= 600
            && self.latest_request.chars().count() <= 600
            && !self.latest_request.chars().any(char::is_control)
            && !self.request.chars().any(char::is_control)
            && matches!(self.source.as_str(), "" | "launcher" | "hook")
            && matches!(
                self.last_activity.as_str(),
                "" | "thinking" | "read" | "edit" | "command" | "other" | "idle" | "ended"
            )
            && self.completed_tools.len() <= TOOLS.len()
            && self
                .completed_tools
                .iter()
                .all(|(role, count)| TOOLS.contains(&role.as_str()) && *count <= 1_000_000)
    }

    pub fn supplied(request: &str) -> Self {
        Self {
            request: excerpt(request),
            source: "launcher".into(),
            updated_at: crate::types::now(),
            ..Default::default()
        }
    }
}

/// Best-effort display hygiene, not a guarantee that arbitrary prose contains no secrets.
/// Excerpts remain local unless the user explicitly asks to send them for a summary.
pub fn excerpt(input: &str) -> String {
    let mut result = String::new();
    for word in input.split_whitespace() {
        let token = word.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '`'));
        let lower = token.to_ascii_lowercase();
        let value = if token.contains("sk-")
            || token.contains("LLM_")
            || token.contains("ghp_")
            || token.contains("github_pat_")
            || lower.contains("api_key=")
            || lower.contains("token=")
        {
            "[credential]"
        } else if token.starts_with('/') || token.starts_with("~/") || token.contains(":\\") {
            "[path]"
        } else if token.starts_with("https://") || token.starts_with("http://") {
            "[link]"
        } else {
            word
        };
        if !result.is_empty() {
            result.push(' ');
        }
        result.extend(value.chars().filter(|c| !c.is_control()));
        if result.chars().count() > 600 {
            result = result.chars().take(599).collect();
            result.push('…');
            break;
        }
    }
    result
}

/// Claude Code and Codex expose the same common lifecycle fields. Unknown fields (including
/// transcript_path, tool_input and tool_response) are deliberately ignored, not persisted.
pub fn apply(context: &mut Context, event: &Value, at: u64) -> Result<bool, String> {
    let name = event["hook_event_name"]
        .as_str()
        .ok_or("Missing hook event name.")?;
    if !matches!(
        name,
        "SessionStart" | "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "Stop" | "SessionEnd"
    ) {
        return Ok(false);
    }
    // A nested agent must not replace the main session's assignment.
    if event.get("agent_id").is_some_and(|v| !v.is_null()) {
        return Ok(false);
    }
    let id = event["session_id"]
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 128
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        })
        .ok_or("Invalid harness session identifier.")?;
    if !context.session_id.is_empty() && context.session_id != id {
        return Ok(false);
    }
    context.session_id = id.into();
    let activity = match name {
        "UserPromptSubmit" => {
            let request = event["prompt"].as_str().ok_or("Missing task request.")?;
            let request = excerpt(request);
            if !request.is_empty() {
                if context.request.is_empty() {
                    context.request = request;
                    context.source = "hook".into();
                } else if context.request != request {
                    context.latest_request = request;
                } else {
                    context.latest_request.clear();
                }
                context.updated_at = at;
            }
            "thinking"
        }
        "PreToolUse" | "PostToolUse" => {
            let role = match event["tool_name"].as_str().unwrap_or_default() {
                "Read" | "Glob" | "Grep" => "read",
                "Write" | "Edit" | "MultiEdit" | "apply_patch" => "edit",
                "Bash" | "exec_command" | "shell_command" | "shell" => "command",
                _ => "other",
            };
            if name == "PostToolUse" {
                let count = context.completed_tools.entry(role.into()).or_default();
                *count = count.saturating_add(1).min(1_000_000);
                "thinking"
            } else {
                role
            }
        }
        "Stop" => "idle",
        "SessionEnd" => "ended",
        _ => "thinking",
    };
    context.last_activity = activity.into();
    context.last_activity_at = at;
    Ok(true)
}

/// Nothing is installed globally. Claude accepts this object through --settings; Codex users
/// can merge it into their hook settings and review/trust it through the harness UI.
pub fn hook_settings(binary: &Path) -> Result<Value, String> {
    let binary = binary
        .to_str()
        .ok_or("Hook executable path must be UTF-8.")?;
    #[cfg(not(windows))]
    let command = format!("'{}' event", binary.replace('\'', "'\"'\"'"));
    #[cfg(windows)]
    let command = {
        // cmd.exe expands percent variables even inside quotes. Fail closed for unusual paths.
        if binary.contains(['%', '!', '"', '\n', '\r']) {
            return Err("Unsupported hook executable path.".into());
        }
        format!("\"{binary}\" event")
    };
    let mut hooks = serde_json::Map::new();
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "Stop",
        "SessionEnd",
    ] {
        hooks.insert(
            event.into(),
            json!([{ "hooks": [{ "type": "command", "command": command, "timeout": 3 }] }]),
        );
    }
    Ok(json!({ "hooks": hooks }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_events_retain_intent_and_categories_never_payloads() {
        let mut context = Context::default();
        apply(&mut context, &json!({"hook_event_name":"UserPromptSubmit", "session_id":"test-1", "prompt":"Build a search screen in /private/work using sk-secret", "transcript_path":"private-log"}), 10).unwrap();
        apply(&mut context, &json!({"hook_event_name":"PostToolUse", "session_id":"test-1", "tool_name":"Edit", "tool_input":{"new_string":"private-code"}, "tool_response":"private-output"}), 11).unwrap();
        let text = serde_json::to_string(&context).unwrap();
        for secret in [
            "/private",
            "sk-secret",
            "private-log",
            "private-code",
            "private-output",
        ] {
            assert!(!text.contains(secret));
        }
        assert_eq!(context.completed_tools["edit"], 1);
        assert_eq!(
            context.request,
            "Build a search screen in [path] using [credential]"
        );
        assert!(context.valid());
        assert!(excerpt(&"é".repeat(1000)).chars().count() <= 600);
    }
    #[test]
    fn followups_invalidate_intent_but_other_sessions_and_subagents_do_not() {
        let mut c = Context::default();
        let event = |id: &str, prompt: &str| json!({"hook_event_name":"UserPromptSubmit","session_id":id,"prompt":prompt});
        apply(&mut c, &event("first", "Build search"), 1).unwrap();
        assert!(!apply(&mut c, &event("nested", "Unrelated"), 2).unwrap());
        let mut sub = event("first", "Subagent work");
        sub["agent_id"] = json!("child");
        assert!(!apply(&mut c, &sub, 3).unwrap());
        assert_eq!(c.request, "Build search");
        apply(&mut c, &event("first", "Add search tests"), 4).unwrap();
        assert_eq!(c.request, "Build search");
        assert_eq!(c.latest_request, "Add search tests");
        assert_eq!(c.updated_at, 4);
    }
}
