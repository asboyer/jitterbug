//! Find the coding agents installed on this computer and connect them with one approval.
//! Connecting adds one Jitterbug entry to each agent's own hook settings: Claude Code's
//! `settings.json`, Codex's `config.toml`, and a local Muse Code plugin. Unrelated settings are
//! kept, a backup of each file is saved first, and disconnecting removes only Jitterbug's entry.
use super::{harness_name, State, HARNESSES};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const MUSE_PLUGIN: &str = "jitterbug-agents";
/// Seconds a harness waits for the gate before giving up on it. The gate answers well before.
const HOOK_TIMEOUT: u64 = 90;

#[derive(Clone, Debug, Serialize)]
pub struct Harness {
    pub key: String,
    pub name: String,
    pub installed: bool,
    pub version: Option<String>,
    /// Where the hook entry lives, with the home folder shortened to `~`.
    pub config: String,
    pub connected: bool,
    pub last_seen: Option<u64>,
    /// What Jitterbug adds, exactly as written.
    pub entry: String,
    pub note: String,
}

fn home() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "Home directory unavailable.".into())
}

fn short(path: &Path) -> String {
    crate::agent_sessions::display_path(path, dirs::home_dir().as_deref())
}

pub fn claude_settings() -> Result<PathBuf, String> {
    Ok(std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or(home()?.join(".claude"))
        .join("settings.json"))
}

pub fn codex_config() -> Result<PathBuf, String> {
    Ok(std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or(home()?.join(".codex"))
        .join("config.toml"))
}

fn muse_data() -> Result<PathBuf, String> {
    Ok(std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or(home()?.join(".local/share"))
        .join("muse"))
}

/// Common install locations: a desktop app's PATH does not include the user's shell setup.
fn find_binary(name: &str) -> Option<PathBuf> {
    let mut places: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = dirs::home_dir() {
        for dir in [
            ".local/bin",
            ".claude/local",
            ".npm-global/bin",
            ".bun/bin",
            ".cargo/bin",
        ] {
            places.push(home.join(dir));
        }
    }
    places.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    places
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

fn version(binary: &Path) -> Option<String> {
    let mut child = Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if child.try_wait().ok()?.is_some() {
            let output = child.wait_with_output().ok()?;
            let text = String::from_utf8_lossy(&output.stdout);
            return text
                .split_whitespace()
                .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))
                .map(|w| w.chars().take(24).collect());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().ok();
    None
}

fn gate_command(runner: &Path, action: &str, harness: &str) -> String {
    format!("\"{}\" {action} --harness {harness}", runner.display())
}

fn ours(command: &str) -> bool {
    command.contains("jitterbug-run")
        && (command.contains(" gate --harness") || command.contains(" release --harness"))
}

fn claude_entry(runner: &Path) -> Value {
    let hook = |action: &str, timeout: Option<u64>| {
        let mut hook =
            json!({"type": "command", "command": gate_command(runner, action, "claude")});
        if let Some(seconds) = timeout {
            hook["timeout"] = json!(seconds);
        }
        json!([{"matcher": "Bash", "hooks": [hook]}])
    };
    json!({
        "PreToolUse": hook("gate", Some(HOOK_TIMEOUT)),
        "PostToolUse": hook("release", None),
    })
}

fn codex_entry(runner: &Path) -> String {
    let block = |event: &str, action: &str, timeout: bool| {
        format!(
            "[[hooks.{event}]]\nmatcher = \"Bash\"\n[[hooks.{event}.hooks]]\ntype = \"command\"\ncommand = {}\n{}",
            toml_edit::Value::from(gate_command(runner, action, "codex")),
            if timeout { format!("timeout = {HOOK_TIMEOUT}\n") } else { String::new() }
        )
    };
    format!(
        "{}\n{}",
        block("PreToolUse", "gate", true),
        block("PostToolUse", "release", false)
    )
}

fn muse_manifest(runner: &Path) -> Value {
    let hook = |id: &str, event: &str, action: &str| {
        json!({
            "id": id,
            "event": event,
            "command": [runner.display().to_string(), action, "--harness", "muse"],
            "timeoutMs": HOOK_TIMEOUT * 1000,
            "statusMessage": "Checking with Jitterbug"
        })
    };
    json!({
        "schemaVersion": 1,
        "name": MUSE_PLUGIN,
        "version": "0.1.0",
        "description": "Jitterbug paces heavy commands (tests, builds) so agents take turns.",
        "compat": {"source": "native", "manifestDir": ".muse-plugin"},
        "capabilities": {
            "skills": [], "commands": [], "mcpServers": [], "reminders": [],
            "hooks": [
                hook("gate", "PreToolUse", "gate"),
                hook("release", "PostToolUse", "release"),
            ]
        }
    })
}

fn read_json(path: &Path) -> Result<Value, String> {
    match fs::read(path) {
        Ok(bytes) if bytes.len() > 4 * 1024 * 1024 => {
            Err("Settings file is unexpectedly large.".into())
        }
        Ok(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => Ok(json!({})),
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| {
            format!(
                "{} is not valid JSON; fix it before connecting.",
                short(path)
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(_) => Err(format!("Cannot read {}.", short(path))),
    }
}

fn claude_connected(settings: &Value) -> bool {
    settings
        .pointer("/hooks/PreToolUse")
        .and_then(Value::as_array)
        .is_some_and(|groups| {
            groups.iter().any(|g| {
                g.get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|hooks| {
                        hooks.iter().any(|h| {
                            h.get("command")
                                .and_then(Value::as_str)
                                .is_some_and(|c| ours(c) && c.contains(" gate "))
                        })
                    })
            })
        })
}

/// Remove Jitterbug's hook groups from Claude-style settings, keeping everything else.
fn strip_claude(settings: &mut Value) {
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    for event in ["PreToolUse", "PostToolUse"] {
        if let Some(groups) = hooks.get_mut(event).and_then(Value::as_array_mut) {
            groups.retain(|g| {
                !g.get("hooks").and_then(Value::as_array).is_some_and(|hs| {
                    hs.iter()
                        .any(|h| h.get("command").and_then(Value::as_str).is_some_and(ours))
                })
            });
            if groups.is_empty() {
                hooks.remove(event);
            }
        }
    }
    if hooks.is_empty() {
        settings.as_object_mut().map(|o| o.remove("hooks"));
    }
}

fn add_claude(settings: &mut Value, runner: &Path) -> Result<(), String> {
    strip_claude(settings);
    let object = settings
        .as_object_mut()
        .ok_or("Claude Code settings must be a JSON object.")?;
    let hooks = object.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks
        .as_object_mut()
        .ok_or("Claude Code's hooks setting must be an object.")?;
    for (event, groups) in claude_entry(runner).as_object().into_iter().flatten() {
        let list = hooks.entry(event.clone()).or_insert_with(|| json!([]));
        let list = list
            .as_array_mut()
            .ok_or("Claude Code's hook lists must be arrays.")?;
        list.extend(groups.as_array().cloned().unwrap_or_default());
    }
    Ok(())
}

fn codex_doc(path: &Path) -> Result<toml_edit::DocumentMut, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return Err(format!("Cannot read {}.", short(path))),
    };
    text.parse::<toml_edit::DocumentMut>().map_err(|_| {
        format!(
            "{} is not valid TOML; fix it before connecting.",
            short(path)
        )
    })
}

fn codex_groups(doc: &toml_edit::DocumentMut, event: &str) -> Vec<bool> {
    doc.get("hooks")
        .and_then(|h| h.get(event))
        .and_then(|e| e.as_array_of_tables())
        .map(|groups| {
            groups
                .iter()
                .map(|g| {
                    g.get("hooks")
                        .and_then(|h| h.as_array_of_tables())
                        .is_some_and(|hs| {
                            hs.iter().any(|h| {
                                h.get("command").and_then(|c| c.as_str()).is_some_and(ours)
                            })
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn strip_codex(doc: &mut toml_edit::DocumentMut) {
    for event in ["PreToolUse", "PostToolUse"] {
        let flags = codex_groups(doc, event);
        // `Table::get_mut` never inserts; `Item::get_mut` would add an empty entry.
        if let Some(groups) = doc
            .get_mut("hooks")
            .and_then(|h| h.as_table_mut())
            .and_then(|h| h.get_mut(event))
            .and_then(|e| e.as_array_of_tables_mut())
        {
            for (index, mine) in flags.iter().enumerate().rev() {
                if *mine {
                    groups.remove(index);
                }
            }
            if groups.is_empty() {
                if let Some(table) = doc.get_mut("hooks").and_then(|h| h.as_table_mut()) {
                    table.remove(event);
                }
            }
        }
    }
}

fn add_codex(doc: &mut toml_edit::DocumentMut, runner: &Path) -> Result<(), String> {
    strip_codex(doc);
    let hooks = doc
        .entry("hooks")
        .or_insert_with(|| {
            let mut table = toml_edit::Table::new();
            table.set_implicit(true);
            toml_edit::Item::Table(table)
        })
        .as_table_mut()
        .ok_or("Codex's hooks setting must be a table.")?;
    for (event, action, timeout) in [
        ("PreToolUse", "gate", true),
        ("PostToolUse", "release", false),
    ] {
        let mut hook = toml_edit::Table::new();
        hook["type"] = toml_edit::value("command");
        hook["command"] = toml_edit::value(gate_command(runner, action, "codex"));
        if timeout {
            hook["timeout"] = toml_edit::value(HOOK_TIMEOUT as i64);
        }
        let mut inner = toml_edit::ArrayOfTables::new();
        inner.push(hook);
        let mut group = toml_edit::Table::new();
        group["matcher"] = toml_edit::value("Bash");
        group["hooks"] = toml_edit::Item::ArrayOfTables(inner);
        let list = hooks
            .entry(event)
            .or_insert_with(|| toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()))
            .as_array_of_tables_mut()
            .ok_or("Codex's hook lists must be arrays of tables.")?;
        list.push(group);
    }
    Ok(())
}

fn muse_connected() -> bool {
    muse_data()
        .ok()
        .and_then(|dir| fs::read(dir.join("plugins/installed.json")).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|value| value.to_string().contains(&format!("\"{MUSE_PLUGIN}\"")))
}

fn backup(directory: &Path, source: &Path, name: &str) -> Result<(), String> {
    if !source.is_file() {
        return Ok(());
    }
    let target = directory.join("backups");
    fs::create_dir_all(&target).map_err(|_| "Cannot create the backup folder.")?;
    let stamp = crate::types::now();
    fs::copy(source, target.join(format!("{name}-{stamp}")))
        .map_err(|_| format!("Cannot back up {} before changing it.", short(source)))?;
    Ok(())
}

fn write_text(path: &Path, text: &str) -> Result<(), String> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!(
            "{} is a symbolic link; add the hook manually.",
            short(path)
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| format!("Cannot create {}.", short(parent)))?;
    }
    let temp = path.with_extension("jitterbug-tmp");
    fs::write(&temp, text).map_err(|_| format!("Cannot write {}.", short(path)))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)
            .map(|m| m.permissions().mode())
            .unwrap_or(0o600);
        fs::set_permissions(&temp, fs::Permissions::from_mode(mode & 0o777)).ok();
    }
    fs::rename(&temp, path).map_err(|_| format!("Cannot update {}.", short(path)))
}

fn muse(binary: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new(binary)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| "Cannot run Muse Code.")?;
    if output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stderr);
    Err(text
        .lines()
        .last()
        .unwrap_or("Muse Code refused the plugin.")
        .chars()
        .take(200)
        .collect())
}

/// Every supported agent, whether it is installed and whether Jitterbug is connected to it.
pub fn detect(runner: &Path, state: &State) -> Vec<Harness> {
    HARNESSES
        .iter()
        .map(|key| {
            let binary = find_binary(key);
            let (config, connected, entry) = match *key {
                "claude" => {
                    let path = claude_settings().unwrap_or_default();
                    let connected = read_json(&path).is_ok_and(|v| claude_connected(&v));
                    let entry =
                        serde_json::to_string_pretty(&json!({"hooks": claude_entry(runner)}))
                            .unwrap_or_default();
                    (short(&path), connected, entry)
                }
                "codex" => {
                    let path = codex_config().unwrap_or_default();
                    let connected = codex_doc(&path)
                        .is_ok_and(|doc| codex_groups(&doc, "PreToolUse").contains(&true));
                    (short(&path), connected, codex_entry(runner))
                }
                _ => (
                    format!("Muse plugin \"{MUSE_PLUGIN}\""),
                    muse_connected(),
                    serde_json::to_string_pretty(&muse_manifest(runner)["capabilities"]["hooks"])
                        .unwrap_or_default(),
                ),
            };
            let installed = binary.is_some()
                || match *key {
                    "claude" => {
                        claude_settings().is_ok_and(|p| p.parent().is_some_and(Path::is_dir))
                    }
                    "codex" => codex_config().is_ok_and(|p| p.parent().is_some_and(Path::is_dir)),
                    _ => muse_data().is_ok_and(|p| p.is_dir()),
                };
            Harness {
                key: key.to_string(),
                name: harness_name(key).into(),
                installed,
                version: binary.as_deref().and_then(version),
                config,
                connected,
                last_seen: state.contacts.get(*key).copied(),
                entry,
                note: match *key {
                    "muse" if binary.is_none() && installed => {
                        "Muse Code's command isn't on a standard path.".into()
                    }
                    // Codex keeps its own trust record for each hook; Jitterbug never writes it.
                    "codex" if connected && !state.contacts.contains_key("codex") => {
                        "Open Codex and type /hooks to trust Jitterbug's hook. Until then Codex skips it.".into()
                    }
                    _ => String::new(),
                },
            }
        })
        .collect()
}

/// Add Jitterbug's hooks to each requested agent. Returns the agents that changed.
pub fn connect(directory: &Path, runner: &Path, keys: &[String]) -> Result<Vec<String>, String> {
    if !runner.is_file() {
        return Err("Jitterbug's command-line helper is missing next to the app.".into());
    }
    let mut changed = vec![];
    for key in keys {
        match key.as_str() {
            "claude" => {
                let path = claude_settings()?;
                let mut settings = read_json(&path)?;
                backup(directory, &path, "claude-settings.json")?;
                add_claude(&mut settings, runner)?;
                let text = serde_json::to_string_pretty(&settings)
                    .map_err(|_| "Cannot encode Claude Code settings.")?;
                write_text(&path, &(text + "\n"))?;
            }
            "codex" => {
                let path = codex_config()?;
                let mut doc = codex_doc(&path)?;
                backup(directory, &path, "codex-config.toml")?;
                add_codex(&mut doc, runner)?;
                write_text(&path, &doc.to_string())?;
            }
            "muse" => {
                let binary = find_binary("muse").ok_or("Muse Code isn't installed.")?;
                let bundle = directory.join("muse-plugin");
                let manifest_dir = bundle.join(".muse-plugin");
                fs::create_dir_all(&manifest_dir).map_err(|_| "Cannot create the Muse plugin.")?;
                let manifest = serde_json::to_string_pretty(&muse_manifest(runner))
                    .map_err(|_| "Cannot encode the Muse plugin.")?;
                write_text(&manifest_dir.join("plugin.json"), &manifest)?;
                let bundle_text = bundle.display().to_string();
                if muse_connected() {
                    muse(&binary, &["plugins", "update", MUSE_PLUGIN, "--json"])?;
                } else {
                    muse(
                        &binary,
                        &[
                            "plugins",
                            "install",
                            &bundle_text,
                            "--scope",
                            "user",
                            "--json",
                        ],
                    )?;
                }
                // Trust the hooks so they run without a separate prompt in Muse.
                muse(&binary, &["plugins", "approve", MUSE_PLUGIN, "--json"]).ok();
            }
            _ => return Err("Unsupported agent.".into()),
        }
        changed.push(key.clone());
    }
    Ok(changed)
}

/// Remove only Jitterbug's entries from each requested agent.
pub fn disconnect(directory: &Path, keys: &[String]) -> Result<(), String> {
    for key in keys {
        match key.as_str() {
            "claude" => {
                let path = claude_settings()?;
                let mut settings = read_json(&path)?;
                if claude_connected(&settings) || settings.pointer("/hooks/PostToolUse").is_some() {
                    backup(directory, &path, "claude-settings.json")?;
                    strip_claude(&mut settings);
                    let text = serde_json::to_string_pretty(&settings)
                        .map_err(|_| "Cannot encode Claude Code settings.")?;
                    write_text(&path, &(text + "\n"))?;
                }
            }
            "codex" => {
                let path = codex_config()?;
                let mut doc = codex_doc(&path)?;
                backup(directory, &path, "codex-config.toml")?;
                strip_codex(&mut doc);
                write_text(&path, &doc.to_string())?;
            }
            "muse" => {
                if let Some(binary) = find_binary("muse") {
                    if muse_connected() {
                        muse(&binary, &["plugins", "remove", MUSE_PLUGIN, "--json"])?;
                    }
                }
            }
            _ => return Err("Unsupported agent.".into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_hooks_are_added_once_and_removed_cleanly() {
        let runner = Path::new("/Applications/Jitterbug.app/Contents/MacOS/jitterbug-run");
        let mut settings = json!({
            "model": "opus",
            "hooks": {"PreToolUse": [{"matcher": "Edit", "hooks": [{"type": "command", "command": "lint"}]}]}
        });
        add_claude(&mut settings, runner).unwrap();
        add_claude(&mut settings, runner).unwrap();
        assert!(claude_connected(&settings));
        assert_eq!(settings["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
        assert_eq!(
            settings["hooks"]["PostToolUse"].as_array().unwrap().len(),
            1
        );
        strip_claude(&mut settings);
        assert!(!claude_connected(&settings));
        assert_eq!(
            settings,
            json!({"model": "opus", "hooks": {"PreToolUse": [{"matcher": "Edit", "hooks": [{"type": "command", "command": "lint"}]}]}})
        );
    }

    #[test]
    fn codex_hooks_keep_unrelated_settings_and_comments() {
        let runner = Path::new("/opt/jitterbug-run");
        let original = "# my config\nmodel = \"gpt-5\"\n\n[hooks.state]\nfoo = 1\n";
        let mut doc = original.parse::<toml_edit::DocumentMut>().unwrap();
        add_codex(&mut doc, runner).unwrap();
        add_codex(&mut doc, runner).unwrap();
        let text = doc.to_string();
        assert!(text.contains("# my config"));
        assert_eq!(codex_groups(&doc, "PreToolUse"), vec![true]);
        assert!(text.contains("gate --harness codex"));
        strip_codex(&mut doc);
        assert!(codex_groups(&doc, "PreToolUse").is_empty());
        assert!(doc.to_string().contains("foo = 1"));
    }

    #[test]
    fn muse_plugin_runs_the_gate_for_every_tool() {
        let manifest = muse_manifest(Path::new("/opt/jitterbug-run"));
        let hooks = manifest["capabilities"]["hooks"].as_array().unwrap();
        assert_eq!(hooks[0]["event"], "PreToolUse");
        assert_eq!(hooks[0]["command"][1], "gate");
        assert_eq!(manifest["schemaVersion"], 1);
    }
}
