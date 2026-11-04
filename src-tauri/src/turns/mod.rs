//! Take turns: coding agents (Claude Code, Codex, Muse Code) ask Jitterbug before each shell
//! command through their own pre-tool hooks. Light commands pass at once. Heavy ones (test
//! suites, builds) share one slot, so several agents never run their heaviest work together;
//! agents keep reasoning and editing while a heavy command waits for its turn.
//!
//! The gate fails open: when Jitterbug is closed, turned off, or its state is unreadable, every
//! command runs as it would without Jitterbug. Nothing here starts, stops, or rewrites commands.
pub mod classify;
pub mod hooks;

use crate::{
    queue::resources::Resources,
    storage,
    types::{now, Snapshot},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::Duration,
};

pub const HARNESSES: [&str; 3] = ["claude", "codex", "muse"];
/// A waiting command keeps its place in line while its agent retries within this window.
const WAIT_TTL: u64 = 180;
/// A turn whose command never reported back is released after this long without live processes.
const QUIET_RELEASE: u64 = 20;
/// Hard ceiling for one turn, in case neither the release hook nor process tracking ends it.
const MAX_TURN: u64 = 45 * 60;
const MAX_HISTORY: usize = 32;
const MAX_WAITING: usize = 32;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Turn {
    pub id: String,
    pub harness: String,
    pub session: String,
    /// Subagent label (its description), empty for the main conversation.
    pub agent: String,
    /// Short display form of the command: program and subcommand, never full paths.
    pub command: String,
    pub folder: String,
    /// The harness process that asked, used to find the command's processes and to notice exits.
    pub owner_pid: u32,
    pub owner_start: u64,
    pub requested_at: u64,
    pub last_poll: u64,
    pub started_at: Option<u64>,
    /// Live usage of the command's processes, filled in by the desktop each sample.
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub peak_cpu_percent: f32,
    pub peak_memory_bytes: u64,
    pub last_active: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Finished {
    pub harness: String,
    pub session: String,
    pub agent: String,
    pub command: String,
    pub waited_seconds: u64,
    pub started_at: u64,
    pub ended_at: u64,
    pub peak_cpu_percent: f32,
    pub peak_memory_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub enabled: bool,
    pub slots: usize,
    pub waiting: Vec<Turn>,
    pub running: Vec<Turn>,
    pub history: Vec<Finished>,
    /// Last time each harness's hook reached Jitterbug.
    pub contacts: BTreeMap<String, u64>,
    /// Commands seen recently that were not treated as heavy, for the explanation model.
    pub light_seen: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            enabled: true,
            slots: 1,
            waiting: vec![],
            running: vec![],
            history: vec![],
            contacts: BTreeMap::new(),
            light_seen: 0,
        }
    }
}

/// What a harness asked about, extracted from its hook payload.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub harness: String,
    pub session: String,
    pub tool_use_id: String,
    pub agent_id: String,
    pub agent_type: String,
    pub transcript: Option<PathBuf>,
    pub cwd: String,
    pub command: Option<String>,
}

impl Request {
    pub fn parse(harness: &str, input: &Value) -> Self {
        let text = |key: &str| {
            input
                .get(key)
                .and_then(Value::as_str)
                .map(|s| s.chars().filter(|c| !c.is_control()).take(160).collect())
                .unwrap_or_default()
        };
        let tool = input.get("tool_input");
        let command = tool
            .and_then(|t| t.get("command").or_else(|| t.get("cmd")))
            .and_then(|c| match c {
                Value::String(s) => Some(s.clone()),
                Value::Array(parts) => {
                    let words: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
                    (!words.is_empty()).then(|| words.join(" "))
                }
                _ => None,
            })
            .filter(|c| !c.trim().is_empty() && c.len() <= 64 * 1024);
        Self {
            harness: harness.into(),
            session: text("session_id"),
            tool_use_id: text("tool_use_id"),
            agent_id: text("agent_id"),
            agent_type: text("agent_type"),
            transcript: input
                .get("transcript_path")
                .and_then(Value::as_str)
                .map(PathBuf::from),
            cwd: text("cwd"),
            command,
        }
    }

    /// A stable key for a command that is retried after being asked to wait.
    fn key(&self, command: &str) -> String {
        format!(
            "{}|{}|{}|{}",
            self.harness, self.session, self.agent_id, command
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// Not heavy, or Jitterbug is not able to schedule right now: run as normal.
    Pass,
    /// Heavy, and this command now holds a slot.
    Started,
    /// Heavy and waiting; tell the agent who is ahead.
    Wait {
        position: usize,
        ahead: Vec<String>,
        reason: String,
    },
}

pub fn directory() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("JITTERBUG_TURNS_DIR") {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err("JITTERBUG_TURNS_DIR must be absolute.".into());
        }
        return Ok(path);
    }
    dirs::data_dir()
        .map(|p| p.join("com.jitterbug.desktop").join("turns"))
        .ok_or_else(|| "Cannot locate the private turns directory.".into())
}

fn lock(directory: &Path) -> Result<File, String> {
    fs::create_dir_all(directory).map_err(|_| "Cannot create the turns directory.")?;
    if fs::symlink_metadata(directory)
        .map_err(|_| "Cannot inspect the turns directory.")?
        .file_type()
        .is_symlink()
    {
        return Err("Turns directory must not be a symbolic link.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot restrict turns permissions.")?;
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(directory.join("turns.lock"))
        .map_err(|_| "Cannot open the turns lock.")?;
    for _ in 0..200 {
        match fs2::FileExt::try_lock_exclusive(&lock) {
            Ok(()) => return Ok(lock),
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return Err("Cannot lock turns state.".into()),
        }
    }
    Err("Turns state busy.".into())
}

pub fn transaction<T>(
    directory: &Path,
    operation: impl FnOnce(&mut State) -> T,
) -> Result<T, String> {
    let _lock = lock(directory)?;
    let path = directory.join("turns.json");
    if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file() || m.len() > 1024 * 1024) {
        return Err("Invalid or oversized turns state.".into());
    }
    let mut state: State = storage::read(&path)?;
    let before = serde_json::to_vec(&state).map_err(|_| "Cannot encode turns state.")?;
    let result = operation(&mut state);
    let after = serde_json::to_vec(&state).map_err(|_| "Cannot encode turns state.")?;
    if after != before {
        storage::write(&path, &state)?;
    }
    Ok(result)
}

pub fn inspect(directory: &Path) -> Result<State, String> {
    let _lock = lock(directory)?;
    storage::read(&directory.join("turns.json"))
}

impl State {
    fn forget_stale(&mut self, at: u64) {
        self.waiting.retain(|t| t.last_poll + WAIT_TTL >= at);
        let (keep, done): (Vec<_>, Vec<_>) = self
            .running
            .drain(..)
            .partition(|t| t.started_at.is_none_or(|s| s + MAX_TURN >= at));
        self.running = keep;
        for turn in done {
            self.finish(turn, at);
        }
    }

    fn finish(&mut self, turn: Turn, at: u64) {
        let started = turn.started_at.unwrap_or(at);
        self.history.push(Finished {
            harness: turn.harness,
            session: turn.session,
            agent: turn.agent,
            command: turn.command,
            waited_seconds: started.saturating_sub(turn.requested_at),
            started_at: started,
            ended_at: at,
            peak_cpu_percent: turn.peak_cpu_percent,
            peak_memory_bytes: turn.peak_memory_bytes,
        });
        if self.history.len() > MAX_HISTORY {
            let excess = self.history.len() - MAX_HISTORY;
            self.history.drain(..excess);
        }
    }

    /// Ask for a turn. `headroom` is `None` when the machine has room for one more heavy
    /// command, otherwise the reason it does not.
    pub fn ask(
        &mut self,
        request: &Request,
        command: &str,
        agent: &str,
        owner: (u32, u64),
        headroom: Option<String>,
        at: u64,
    ) -> Decision {
        self.contacts.insert(request.harness.clone(), at);
        self.forget_stale(at);
        let key = request.key(command);
        if let Some(index) = self.running.iter().position(|t| t.id == key) {
            // A retried command that already holds the slot keeps it.
            self.running[index].last_poll = at;
            return Decision::Started;
        }
        let index = match self.waiting.iter().position(|t| t.id == key) {
            Some(index) => index,
            None => {
                if self.waiting.len() >= MAX_WAITING {
                    return Decision::Pass;
                }
                self.waiting.push(Turn {
                    id: key.clone(),
                    harness: request.harness.clone(),
                    session: request.session.clone(),
                    agent: agent.into(),
                    command: command.into(),
                    folder: folder_name(&request.cwd),
                    owner_pid: owner.0,
                    owner_start: owner.1,
                    requested_at: at,
                    ..Default::default()
                });
                self.waiting.len() - 1
            }
        };
        self.waiting[index].last_poll = at;
        let free = self.running.len() < self.slots.max(1);
        if index == 0 && free && headroom.is_none() {
            let mut turn = self.waiting.remove(0);
            turn.started_at = Some(at);
            turn.last_active = at;
            self.running.push(turn);
            return Decision::Started;
        }
        let ahead: Vec<String> = self
            .running
            .iter()
            .chain(self.waiting[..index].iter())
            .map(describe)
            .collect();
        let reason = if !free || index > 0 {
            format!(
                "{} ahead of this one",
                if ahead.len() == 1 {
                    "1 heavy command is".to_string()
                } else {
                    format!("{} heavy commands are", ahead.len())
                }
            )
        } else {
            headroom.unwrap_or_default()
        };
        Decision::Wait {
            position: index + 1,
            ahead,
            reason,
        }
    }

    /// The command finished (the harness's post-tool hook), or it was cancelled.
    pub fn release(&mut self, request: &Request, command: Option<&str>, at: u64) -> bool {
        self.contacts.insert(request.harness.clone(), at);
        let position = command
            .map(|c| request.key(c))
            .and_then(|key| self.running.iter().position(|t| t.id == key));
        match position {
            Some(index) => {
                let turn = self.running.remove(index);
                self.finish(turn, at);
                true
            }
            None => false,
        }
    }
}

fn describe(turn: &Turn) -> String {
    let who = if turn.agent.is_empty() {
        harness_name(&turn.harness).to_string()
    } else {
        format!("{} ({})", turn.agent, harness_name(&turn.harness))
    };
    format!("{} from {}", turn.command, who)
}

pub fn harness_name(key: &str) -> &str {
    match key {
        "claude" => "Claude Code",
        "codex" => "Codex",
        "muse" => "Muse Code",
        other => other,
    }
}

fn folder_name(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .chars()
        .take(60)
        .collect()
}

/// The message an agent reads when its heavy command has to wait.
pub fn wait_message(command: &str, position: usize, ahead: &[String], reason: &str) -> String {
    let mut text = format!(
        "Jitterbug is pacing heavy commands on this computer so agents don't all run them at once. `{command}` is #{position} in line: {reason}."
    );
    if let Some(first) = ahead.first() {
        text.push_str(&format!(" Running now: {first}."));
    }
    text.push_str(" Keep working on something that doesn't need this command, then run it again in about a minute; if there's nothing else to do, wait in the foreground (for example `sleep 60`) and retry. It keeps its place in line. Don't work around this by running the same command another way.");
    text
}

/// Subagent label for a request: Claude Code keeps each subagent's description next to the
/// session transcript. Other harnesses report only the agent type.
pub fn agent_label(request: &Request) -> String {
    if request.agent_id.is_empty() {
        return String::new();
    }
    if let Some(transcript) = &request.transcript {
        let safe = request
            .agent_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if let (true, Some(dir), Some(stem)) = (
            safe,
            transcript.parent(),
            transcript.file_stem().and_then(|s| s.to_str()),
        ) {
            let meta = dir
                .join(stem)
                .join("subagents")
                .join(format!("agent-{}.meta.json", request.agent_id));
            if let Some(description) = fs::read(&meta)
                .ok()
                .filter(|b| b.len() < 64 * 1024)
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| v.get("description")?.as_str().map(str::to_string))
            {
                return description
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(80)
                    .collect();
            }
        }
    }
    if request.agent_type.is_empty() {
        "Subagent".into()
    } else {
        request.agent_type.clone()
    }
}

/// Whether the desktop has published fresh headroom. Without it nobody is watching, so the gate
/// lets commands run instead of waiting for a scheduler that is not there.
pub fn headroom() -> Result<Resources, String> {
    let resources: Resources = storage::read(&crate::queue::directory()?.join("headroom.json"))?;
    if !resources.ready || resources.captured_at + 20 < now() {
        return Err("Jitterbug is not sampling right now.".into());
    }
    Ok(resources)
}

/// Desktop bookkeeping each sample: attach live usage to running turns and release turns whose
/// harness exited or whose command's processes are gone.
pub fn tick(directory: &Path, snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.source != "live" || !snapshot.sample_ready {
        return Ok(());
    }
    let at = now();
    let mut children: HashMap<u32, Vec<&crate::types::ProcessRecord>> = HashMap::new();
    for p in &snapshot.processes {
        if let Some(parent) = p.parent_pid {
            children.entry(parent).or_default().push(p);
        }
    }
    let alive = |pid: u32, start: u64| {
        snapshot
            .processes
            .iter()
            .any(|p| p.pid == pid && (start == 0 || p.start_time == start))
    };
    transaction(directory, |state| {
        state.forget_stale(at);
        let mut done = vec![];
        for (index, turn) in state.running.iter_mut().enumerate() {
            let started = turn.started_at.unwrap_or(at);
            if turn.owner_pid != 0 && !alive(turn.owner_pid, turn.owner_start) {
                done.push(index);
                continue;
            }
            let (cpu, memory) = usage(&children, turn.owner_pid, started);
            turn.cpu_percent = cpu;
            turn.memory_bytes = memory;
            turn.peak_cpu_percent = turn.peak_cpu_percent.max(cpu);
            turn.peak_memory_bytes = turn.peak_memory_bytes.max(memory);
            if cpu > 1.0 || memory > 64 * 1024 * 1024 {
                turn.last_active = at;
            }
            if turn.owner_pid != 0 && turn.last_active + QUIET_RELEASE < at && started + 10 < at {
                done.push(index);
            }
        }
        for index in done.into_iter().rev() {
            let turn = state.running.remove(index);
            state.finish(turn, at);
        }
    })
}

/// CPU and memory of processes under the harness that started after the turn did: the command
/// the harness launched once the gate let it through.
fn usage(
    children: &HashMap<u32, Vec<&crate::types::ProcessRecord>>,
    owner: u32,
    started: u64,
) -> (f32, u64) {
    let mut cpu = 0.0;
    let mut memory = 0;
    let mut stack: Vec<(u32, bool)> = vec![(owner, false)];
    let mut seen = 0;
    while let Some((pid, inside)) = stack.pop() {
        seen += 1;
        if seen > 4096 {
            break;
        }
        for child in children.get(&pid).map(Vec::as_slice).unwrap_or_default() {
            let inside = inside || child.start_time + 2 >= started;
            if inside {
                cpu += child.cpu_percent;
                memory += child.memory_bytes;
            }
            stack.push((child.pid, inside));
        }
    }
    (cpu, memory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(session: &str, command: &str) -> Request {
        Request::parse(
            "claude",
            &json!({"session_id": session, "tool_input": {"command": command}, "cwd": "/w/app"}),
        )
    }

    #[test]
    fn heavy_commands_take_turns_in_arrival_order() {
        let mut state = State::default();
        let a = request("a", "cargo test");
        let b = request("b", "npm test");
        assert_eq!(
            state.ask(&a, "cargo test", "", (1, 1), None, 100),
            Decision::Started
        );
        let Decision::Wait {
            position, ahead, ..
        } = state.ask(&b, "npm test", "", (2, 1), None, 101)
        else {
            panic!("second heavy command should wait");
        };
        assert_eq!(position, 1);
        assert!(ahead[0].starts_with("cargo test from Claude Code"));
        assert!(state.release(&a, Some("cargo test"), 130));
        assert_eq!(
            state.ask(&b, "npm test", "", (2, 1), None, 131),
            Decision::Started
        );
        assert_eq!(state.history[0].command, "cargo test");
    }

    #[test]
    fn a_waiting_command_keeps_its_place_but_not_forever() {
        let mut state = State::default();
        let a = request("a", "cargo test");
        state.ask(&a, "cargo test", "", (1, 1), None, 100);
        let b = request("b", "npm test");
        let c = request("c", "pytest");
        state.ask(&b, "npm test", "", (2, 1), None, 101);
        state.ask(&c, "pytest", "", (3, 1), None, 102);
        // b retries later and is still first in line.
        let Decision::Wait { position, .. } = state.ask(&b, "npm test", "", (2, 1), None, 150)
        else {
            panic!()
        };
        assert_eq!(position, 1);
        // c never retried within the window and loses its place.
        state.ask(&b, "npm test", "", (2, 1), None, 101 + WAIT_TTL + 5);
        assert!(state.waiting.iter().all(|t| t.session != "c"));
    }

    #[test]
    fn a_busy_machine_holds_even_a_free_slot() {
        let mut state = State::default();
        let a = request("a", "cargo build");
        let busy = Some("Waiting for total CPU use to fall below 70%.".to_string());
        let Decision::Wait { reason, .. } = state.ask(&a, "cargo build", "", (1, 1), busy, 100)
        else {
            panic!()
        };
        assert!(reason.contains("CPU"));
        assert_eq!(
            state.ask(&a, "cargo build", "", (1, 1), None, 105),
            Decision::Started
        );
    }

    #[test]
    fn hook_payloads_from_each_harness_parse() {
        let codex = Request::parse(
            "codex",
            &json!({"session_id":"s","tool_input":{"command":["bash","-lc","npm test"]}}),
        );
        assert_eq!(codex.command.as_deref(), Some("bash -lc npm test"));
        let muse = Request::parse("muse", &json!({"tool_input":{"cmd":"pytest -x"}}));
        assert_eq!(muse.command.as_deref(), Some("pytest -x"));
        let other = Request::parse("claude", &json!({"tool_input":{"file_path":"/x"}}));
        assert_eq!(other.command, None);
    }

    #[test]
    fn claude_subagents_are_named_by_their_description() {
        let dir = std::env::temp_dir().join(format!("jb-turns-{}", std::process::id()));
        let sub = dir.join("sess").join("subagents");
        fs::create_dir_all(&sub).unwrap();
        fs::write(
            sub.join("agent-a1.meta.json"),
            r#"{"agentType":"general-purpose","description":"Run the test suite"}"#,
        )
        .unwrap();
        let request = Request::parse(
            "claude",
            &json!({"agent_id":"a1","transcript_path": dir.join("sess.jsonl")}),
        );
        assert_eq!(agent_label(&request), "Run the test suite");
        let unknown = Request::parse("claude", &json!({"agent_id":"zz","agent_type":"Explore"}));
        assert_eq!(agent_label(&unknown), "Explore");
        assert_eq!(agent_label(&Request::default()), "");
        fs::remove_dir_all(dir).ok();
    }
}
