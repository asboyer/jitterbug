//! Names and status of running Claude Code sessions, from the per-process files Claude Code
//! keeps in `~/.claude/sessions/<pid>.json` (the name set with /rename, busy or idle, folder).
//! This is Claude Code's internal format, not a documented API: it is read best-effort, and a
//! file counts only when its PID matches and it was written within a minute of that process
//! starting, so a reused PID never borrows another session's name.
use serde_json::Value;
use std::path::PathBuf;

/// The longest name shown; longer names are cut.
const NAME_LIMIT: usize = 60;
/// A session file is written moments after its process starts.
const START_WINDOW: u64 = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeSession {
    pub name: Option<String>,
    /// `working` or `idle` (waiting for the person).
    pub status: Option<String>,
}

fn directory() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".claude")))
        .map(|dir| dir.join("sessions"))
}

/// The Claude Code session run by this process, when its file matches the process.
pub fn lookup(pid: u32, start_time: u64) -> Option<ClaudeSession> {
    let text = std::fs::read_to_string(directory()?.join(format!("{pid}.json"))).ok()?;
    parse(&text, pid, start_time)
}

pub fn parse(text: &str, pid: u32, start_time: u64) -> Option<ClaudeSession> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("pid")?.as_u64()? != u64::from(pid) {
        return None;
    }
    let started = value.get("startedAt")?.as_u64()? / 1000;
    if !(start_time..=start_time.saturating_add(START_WINDOW)).contains(&started) {
        return None;
    }
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .map(|name| {
            name.chars()
                .filter(|c| !c.is_control())
                .take(NAME_LIMIT)
                .collect::<String>()
                .trim()
                .to_string()
        })
        .filter(|name| !name.is_empty());
    let status = match value.get("status").and_then(Value::as_str) {
        Some("busy") => Some("working".to_string()),
        Some("idle") => Some("idle".to_string()),
        _ => None,
    };
    Some(ClaudeSession { name, status })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(pid: u32, started_ms: u64, name: &str, status: &str) -> String {
        serde_json::json!({
            "pid": pid, "startedAt": started_ms, "name": name, "status": status,
            "sessionId": "x", "cwd": "/example",
        })
        .to_string()
    }

    #[test]
    fn a_matching_file_names_the_session_and_its_status() {
        let text = file(213, 1_000_000_004_000, "TEST2", "idle");
        assert_eq!(
            parse(&text, 213, 1_000_000_000),
            Some(ClaudeSession {
                name: Some("TEST2".into()),
                status: Some("idle".into())
            })
        );
        let busy = file(213, 1_000_000_004_000, "TEST1", "busy");
        assert_eq!(
            parse(&busy, 213, 1_000_000_000).unwrap().status.as_deref(),
            Some("working")
        );
    }

    #[test]
    fn another_process_or_a_reused_pid_never_borrows_the_name() {
        let text = file(213, 1_000_000_004_000, "TEST2", "idle");
        assert_eq!(parse(&text, 214, 1_000_000_000), None);
        // Same PID, but this process started long after the file was written.
        assert_eq!(parse(&text, 213, 1_000_100_000), None);
        // Or long before it.
        assert_eq!(parse(&text, 213, 999_000_000), None);
        assert_eq!(parse("not json", 213, 1_000_000_000), None);
    }

    #[test]
    fn names_are_cleaned_and_bounded() {
        let long = format!("{}\u{7}", "n".repeat(100));
        let text = file(1, 5_000, &long, "unknown");
        let session = parse(&text, 1, 4).unwrap();
        assert_eq!(session.name.unwrap().len(), NAME_LIMIT);
        assert_eq!(session.status, None);
        let blank = file(1, 5_000, "  ", "idle");
        assert_eq!(parse(&blank, 1, 4).unwrap().name, None);
    }
}
