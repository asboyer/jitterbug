//! One-click agent setup: registers `jitterbug-run control-mcp` with Claude Code, Codex and
//! Muse Code, and adds the `pressure-hint` hook to Claude Code, so a person never has to paste
//! commands. Muse Code takes the server as a small local plugin installed through its own CLI.
//!
//! Claude Code's MCP list lives in `~/.claude.json`, which running sessions rewrite, so it is
//! changed only through `claude mcp add|remove --scope user`. The hook goes into
//! `~/.claude/settings.json` and Codex's server into `~/.codex/config.toml`; both are edited in
//! place, keep everything else, and are replaced atomically.
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The server name agents see; matches the copy-paste setup in Settings and docs.
pub const SERVER: &str = "jitterbug-control";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Client {
    pub id: &'static str,
    pub label: &'static str,
    /// The agent is installed (its CLI or config folder was found).
    pub installed: bool,
    /// The MCP server is registered and points at this app's `jitterbug-run`.
    pub mcp: bool,
    /// The pressure hint hook is installed (Claude Code only; Codex has no such hook here).
    pub hint: Option<bool>,
}

impl Client {
    pub fn connected(&self) -> bool {
        self.mcp && self.hint != Some(false)
    }
}

/// Where each agent keeps its files, overridable for tests.
#[derive(Debug, Clone)]
pub struct Places {
    pub claude_dir: PathBuf,
    pub claude_json: PathBuf,
    pub codex_dir: PathBuf,
    pub claude_cli: Option<PathBuf>,
    pub codex_cli: Option<PathBuf>,
    pub muse_cli: Option<PathBuf>,
    /// Muse Code's data folder, where it records installed plugins.
    pub muse_data: PathBuf,
    /// Where Jitterbug writes the plugin bundle Muse Code installs from.
    pub muse_bundle: PathBuf,
}

impl Places {
    pub fn detect() -> Result<Self, String> {
        let home = dirs::home_dir().ok_or("Cannot locate your home folder.")?;
        let env_dir = |key: &str| {
            std::env::var_os(key)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        let claude_config = env_dir("CLAUDE_CONFIG_DIR");
        Ok(Self {
            claude_dir: claude_config
                .clone()
                .unwrap_or_else(|| home.join(".claude")),
            claude_json: claude_config
                .map(|d| d.join(".claude.json"))
                .unwrap_or_else(|| home.join(".claude.json")),
            codex_dir: env_dir("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
            claude_cli: find_cli(&home, "claude"),
            codex_cli: find_cli(&home, "codex"),
            muse_cli: find_cli(&home, "muse"),
            muse_data: env_dir("XDG_DATA_HOME")
                .unwrap_or_else(|| home.join(".local/share"))
                .join("muse"),
            muse_bundle: dirs::data_dir()
                .ok_or("Cannot locate the application data folder.")?
                .join("com.jitterbug.desktop/muse-control-plugin"),
        })
    }
}

/// Apps opened from Finder get a minimal PATH, so look where installers put agent CLIs too.
fn find_cli(home: &Path, name: &str) -> Option<PathBuf> {
    let mut places: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for dir in [
        ".local/bin",
        ".claude/local",
        ".npm-global/bin",
        ".bun/bin",
        ".volta/bin",
        ".cargo/bin",
        "Library/pnpm",
    ] {
        places.push(home.join(dir));
    }
    places.push("/opt/homebrew/bin".into());
    places.push("/usr/local/bin".into());
    if let Ok(versions) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        places.extend(versions.flatten().map(|v| v.path().join("bin")));
    }
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    places
        .into_iter()
        .map(|dir| dir.join(&file))
        .find(|path| path.is_file())
}

fn read_json(path: &Path) -> Result<Value, String> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(json!({})),
        Ok(text) => serde_json::from_str::<Value>(&text)
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| {
                format!(
                    "{} is not a JSON object. Fix it, or set up by hand.",
                    path.display()
                )
            }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(_) => Err(format!("Cannot read {}.", path.display())),
    }
}

/// Write next to the target, then rename, keeping the original file's permissions.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let dir = path.parent().ok_or("Bad settings path.")?;
    std::fs::create_dir_all(dir).map_err(|_| format!("Cannot create {}.", dir.display()))?;
    let temp = dir.join(format!(
        ".{}.jitterbug-{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("settings"),
        std::process::id()
    ));
    std::fs::write(&temp, text).map_err(|_| format!("Cannot write {}.", path.display()))?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&temp, meta.permissions());
    }
    std::fs::rename(&temp, path).map_err(|_| {
        let _ = std::fs::remove_file(&temp);
        format!("Cannot replace {}.", path.display())
    })
}

/// A hook command this app installed, at any launcher path (a rebuilt or moved app included).
fn is_hint_command(command: &str) -> bool {
    command.ends_with(" pressure-hint") && command.contains("jitterbug-run")
}

/// The PreToolUse entries with every Jitterbug hint hook taken out. Other hooks in a shared
/// entry are kept.
fn without_hint(entries: &[Value]) -> Vec<Value> {
    entries
        .iter()
        .filter_map(|entry| {
            let Some(hooks) = entry["hooks"].as_array() else {
                return Some(entry.clone());
            };
            let kept: Vec<Value> = hooks
                .iter()
                .filter(|h| !h["command"].as_str().is_some_and(is_hint_command))
                .cloned()
                .collect();
            if kept.is_empty() {
                return None;
            }
            let mut entry = entry.clone();
            entry["hooks"] = Value::Array(kept);
            Some(entry)
        })
        .collect()
}

fn claude_settings(places: &Places) -> PathBuf {
    places.claude_dir.join("settings.json")
}

fn hint_installed(places: &Places, runner: &Path) -> bool {
    let Ok(want) = crate::control::hint::entry(runner) else {
        return false;
    };
    read_json(&claude_settings(places)).is_ok_and(|settings| {
        settings["hooks"]["PreToolUse"]
            .as_array()
            .is_some_and(|entries| {
                entries.iter().any(|e| {
                    e["matcher"] == want["matcher"]
                        && e["hooks"].as_array().is_some_and(|h| {
                            h.iter()
                                .any(|h| h["command"] == want["hooks"][0]["command"])
                        })
                })
            })
    })
}

/// Add (or, with `runner` None, remove) the hint hook in Claude Code's user settings.
fn set_hint(places: &Places, runner: Option<&Path>) -> Result<(), String> {
    let path = claude_settings(places);
    let mut settings = read_json(&path)?;
    let existing = settings["hooks"]["PreToolUse"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut entries = without_hint(&existing);
    if let Some(runner) = runner {
        entries.push(crate::control::hint::entry(runner)?);
    } else if entries == existing {
        return Ok(());
    }
    if !settings["hooks"].is_object() {
        settings["hooks"] = json!({});
    }
    if entries.is_empty() {
        if let Some(hooks) = settings["hooks"].as_object_mut() {
            hooks.remove("PreToolUse");
        }
        if settings["hooks"].as_object().is_some_and(|h| h.is_empty()) {
            if let Some(settings) = settings.as_object_mut() {
                settings.remove("hooks");
            }
        }
    } else {
        settings["hooks"]["PreToolUse"] = Value::Array(entries);
    }
    let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    write_atomic(&path, &(text + "\n"))
}

fn same_server(server: &Value, runner: &Path) -> bool {
    server["command"].as_str() == runner.to_str() && server["args"] == json!(["control-mcp"])
}

/// The user-scope server Claude Code has for `jitterbug-control`, if any.
fn claude_server(places: &Places) -> Option<Value> {
    let config = read_json(&places.claude_json).ok()?;
    let server = &config["mcpServers"][SERVER];
    (!server.is_null()).then(|| server.clone())
}

fn codex_config(places: &Places) -> PathBuf {
    places.codex_dir.join("config.toml")
}

fn codex_document(places: &Places) -> Result<toml_edit::DocumentMut, String> {
    let path = codex_config(places);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return Err(format!("Cannot read {}.", path.display())),
    };
    text.parse::<toml_edit::DocumentMut>().map_err(|_| {
        format!(
            "{} is not valid TOML. Fix it, or set up by hand.",
            path.display()
        )
    })
}

fn codex_registered(places: &Places, runner: &Path) -> bool {
    codex_document(places).is_ok_and(|doc| {
        let Some(server) = doc.get("mcp_servers").and_then(|s| s.get(SERVER)) else {
            return false;
        };
        server.get("command").and_then(|c| c.as_str()) == runner.to_str()
            && server
                .get("args")
                .and_then(|a| a.as_array())
                .is_some_and(|a| {
                    a.len() == 1 && a.get(0).and_then(|v| v.as_str()) == Some("control-mcp")
                })
    })
}

fn set_codex(places: &Places, runner: Option<&Path>) -> Result<(), String> {
    let mut doc = codex_document(places)?;
    match runner {
        Some(runner) => {
            let command = runner.to_str().ok_or("The launcher path must be UTF-8.")?;
            if !doc.contains_table("mcp_servers") {
                let mut servers = toml_edit::Table::new();
                servers.set_implicit(true);
                doc.insert("mcp_servers", toml_edit::Item::Table(servers));
            }
            let servers = doc["mcp_servers"]
                .as_table_mut()
                .ok_or("mcp_servers in Codex's config.toml is not a table.")?;
            let mut server = toml_edit::Table::new();
            server["command"] = toml_edit::value(command);
            let mut args = toml_edit::Array::new();
            args.push("control-mcp");
            server["args"] = toml_edit::value(args);
            servers.insert(SERVER, toml_edit::Item::Table(server));
        }
        None => {
            let Some(servers) = doc
                .get_mut("mcp_servers")
                .and_then(toml_edit::Item::as_table_mut)
            else {
                return Ok(());
            };
            if servers.remove(SERVER).is_none() {
                return Ok(());
            }
        }
    }
    write_atomic(&codex_config(places), &doc.to_string())
}

/// The plugin manifest that gives Muse Code the control server, and nothing else.
fn muse_manifest(runner: &Path) -> Value {
    json!({
        "schemaVersion": 1,
        "name": SERVER,
        "version": "0.1.0",
        "description": "Lets agents ask Jitterbug how busy this computer is before heavy work.",
        "compat": {"source": "native", "manifestDir": ".muse-plugin"},
        "capabilities": {
            "skills": [], "commands": [], "reminders": [], "hooks": [],
            "mcpServers": [{"id": SERVER, "command": [runner.display().to_string(), "control-mcp"]}]
        }
    })
}

/// Muse Code's record of the installed plugin, when there is one.
fn muse_installed(places: &Places) -> Option<Value> {
    let record = read_json(&places.muse_data.join("plugins/installed.json")).ok()?;
    let plugin = &record["plugins"][SERVER];
    (!plugin.is_null()).then(|| plugin.clone())
}

/// Installed, enabled, and its cached copy starts this app's launcher.
fn muse_registered(places: &Places, runner: &Path) -> bool {
    let Some(plugin) = muse_installed(places).filter(|p| p["enabled"] != false) else {
        return false;
    };
    let Some(cache) = plugin["cache_path"].as_str().filter(|c| !c.contains("..")) else {
        return false;
    };
    read_json(
        &places
            .muse_data
            .join(cache)
            .join(".muse-plugin/plugin.json"),
    )
    .is_ok_and(|manifest| manifest == muse_manifest(runner))
}

fn set_muse(places: &Places, runner: Option<&Path>) -> Result<(), String> {
    let installed = muse_installed(places).is_some();
    let Some(runner) = runner else {
        if installed {
            let cli = places
                .muse_cli
                .as_deref()
                .ok_or("Muse Code's `muse` command was not found.")?;
            run_cli(cli, &["plugins", "remove", SERVER, "--json"])?;
        }
        return Ok(());
    };
    let cli = places
        .muse_cli
        .as_deref()
        .ok_or("Muse Code's `muse` command was not found. Install Muse Code, or set up by hand.")?;
    let manifest =
        serde_json::to_string_pretty(&muse_manifest(runner)).map_err(|e| e.to_string())?;
    write_atomic(
        &places.muse_bundle.join(".muse-plugin/plugin.json"),
        &(manifest + "\n"),
    )?;
    let bundle = places
        .muse_bundle
        .to_str()
        .ok_or("The plugin path must be UTF-8.")?;
    if installed {
        run_cli(cli, &["plugins", "update", SERVER, "--json"])?;
    } else {
        run_cli(
            cli,
            &["plugins", "install", bundle, "--scope", "user", "--json"],
        )?;
    }
    // Trust the server so Muse Code does not ask again before its first use.
    run_cli(cli, &["plugins", "approve", SERVER, "--json"]).ok();
    Ok(())
}

fn run_cli(cli: &Path, args: &[&str]) -> Result<(), String> {
    // npm installs are `#!/usr/bin/env node` scripts; give them the folder they came from.
    let mut path = cli
        .parent()
        .map(PathBuf::from)
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(current) = std::env::var_os("PATH") {
        path.extend(std::env::split_paths(&current));
    }
    path.extend([
        "/opt/homebrew/bin".into(),
        "/usr/local/bin".into(),
        "/usr/bin".into(),
        "/bin".into(),
    ]);
    let output = Command::new(cli)
        .args(args)
        .env(
            "PATH",
            std::env::join_paths(path).map_err(|e| e.to_string())?,
        )
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|_| format!("Cannot run {}.", cli.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    Err(format!(
        "`{} {}` failed{}",
        cli.file_name().and_then(|n| n.to_str()).unwrap_or("agent"),
        args.first().copied().unwrap_or(""),
        if detail.is_empty() {
            ".".to_string()
        } else {
            format!(": {}", detail.chars().take(200).collect::<String>())
        }
    ))
}

pub fn status(places: &Places, runner: Option<&Path>) -> Vec<Client> {
    let claude_mcp = match (runner, claude_server(places)) {
        (Some(runner), Some(server)) => same_server(&server, runner),
        _ => false,
    };
    vec![
        Client {
            id: "claude",
            label: "Claude Code",
            installed: places.claude_cli.is_some(),
            mcp: claude_mcp,
            hint: Some(runner.is_some_and(|r| hint_installed(places, r))),
        },
        Client {
            id: "codex",
            label: "Codex",
            installed: places.codex_cli.is_some() || places.codex_dir.is_dir(),
            mcp: runner.is_some_and(|r| codex_registered(places, r)),
            hint: None,
        },
        Client {
            id: "muse",
            label: "Muse Code",
            installed: places.muse_cli.is_some(),
            mcp: runner.is_some_and(|r| muse_registered(places, r)),
            hint: None,
        },
    ]
}

/// Register Jitterbug with one agent. Re-running repairs a stale launcher path.
pub fn connect(places: &Places, runner: &Path, id: &str) -> Result<(), String> {
    let command = runner.to_str().ok_or("The launcher path must be UTF-8.")?;
    match id {
        "claude" => {
            let cli = places
                .claude_cli
                .as_deref()
                .ok_or("Claude Code's `claude` command was not found. Install Claude Code, or set up by hand.")?;
            match claude_server(places) {
                Some(server) if same_server(&server, runner) => {}
                Some(_) => {
                    run_cli(cli, &["mcp", "remove", "--scope", "user", SERVER])?;
                    run_cli(
                        cli,
                        &[
                            "mcp",
                            "add",
                            "--scope",
                            "user",
                            SERVER,
                            "--",
                            command,
                            "control-mcp",
                        ],
                    )?;
                }
                None => run_cli(
                    cli,
                    &[
                        "mcp",
                        "add",
                        "--scope",
                        "user",
                        SERVER,
                        "--",
                        command,
                        "control-mcp",
                    ],
                )?,
            }
            set_hint(places, Some(runner))
        }
        "codex" => set_codex(places, Some(runner)),
        "muse" => set_muse(places, Some(runner)),
        _ => Err("Unknown agent.".into()),
    }
}

/// Remove what `connect` added. Other servers and hooks are left alone.
pub fn disconnect(places: &Places, id: &str) -> Result<(), String> {
    match id {
        "claude" => {
            set_hint(places, None)?;
            if claude_server(places).is_some() {
                let cli = places
                    .claude_cli
                    .as_deref()
                    .ok_or("Claude Code's `claude` command was not found.")?;
                run_cli(cli, &["mcp", "remove", "--scope", "user", SERVER])?;
            }
            Ok(())
        }
        "codex" => set_codex(places, None),
        "muse" => set_muse(places, None),
        _ => Err("Unknown agent.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places(root: &Path) -> Places {
        Places {
            claude_dir: root.join(".claude"),
            claude_json: root.join(".claude.json"),
            codex_dir: root.join(".codex"),
            claude_cli: None,
            codex_cli: None,
            muse_cli: None,
            muse_data: root.join("muse"),
            muse_bundle: root.join("bundle"),
        }
    }

    #[test]
    fn muse_counts_as_connected_only_when_its_cached_plugin_starts_this_launcher() {
        let root = temp("muse");
        let p = places(&root);
        let runner = Path::new("/Applications/Jitterbug.app/Contents/MacOS/jitterbug-run");
        assert!(!muse_registered(&p, runner));
        let cache = "plugins/cache/local/jitterbug-control/abc/package";
        std::fs::create_dir_all(p.muse_data.join(cache).join(".muse-plugin")).unwrap();
        std::fs::write(
            p.muse_data.join("plugins/installed.json"),
            json!({"plugins":{SERVER:{"enabled":true,"cache_path":cache}}}).to_string(),
        )
        .unwrap();
        let manifest = p.muse_data.join(cache).join(".muse-plugin/plugin.json");
        std::fs::write(&manifest, muse_manifest(runner).to_string()).unwrap();
        assert!(muse_registered(&p, runner));
        assert!(!muse_registered(&p, Path::new("/elsewhere/jitterbug-run")));
        let client = status(&p, Some(runner))
            .into_iter()
            .find(|c| c.id == "muse")
            .unwrap();
        assert!(client.mcp && !client.installed && client.connected());
        // Without the CLI there is nothing to run, but setup says so instead of guessing.
        assert!(connect(&p, runner, "muse")
            .unwrap_err()
            .contains("not found"));
        let _ = std::fs::remove_dir_all(root);
    }

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("jitterbug-setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    #[cfg(not(windows))]
    fn hint_hook_merges_into_claude_settings_and_comes_out_cleanly() {
        let root = temp("hint");
        let p = places(&root);
        std::fs::create_dir_all(&p.claude_dir).unwrap();
        let original = json!({
            "theme": "dark",
            "permissions": {"allow": ["Bash(ls:*)"]},
            "hooks": {
                "PreToolUse": [
                    {"matcher": "Edit", "hooks": [{"type": "command", "command": "lint-it"}]},
                    {"matcher": "Bash", "hooks": [
                        {"type": "command", "command": "'/old/place/jitterbug-run' pressure-hint"},
                        {"type": "command", "command": "audit-bash"}
                    ]}
                ],
                "Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]
            }
        });
        std::fs::write(claude_settings(&p), original.to_string()).unwrap();
        let runner = Path::new("/Apps/Jitterbug/jitterbug-run");
        assert!(!hint_installed(&p, runner));
        set_hint(&p, Some(runner)).unwrap();
        // Twice is the same as once, and a stale path from an older build is replaced.
        set_hint(&p, Some(runner)).unwrap();
        assert!(hint_installed(&p, runner));
        let saved = read_json(&claude_settings(&p)).unwrap();
        // The person's own keys keep their order.
        assert!(std::fs::read_to_string(claude_settings(&p))
            .unwrap()
            .starts_with("{\n  \"theme\": \"dark\",\n  \"permissions\""));
        assert_eq!(saved["theme"], "dark");
        assert_eq!(saved["permissions"], original["permissions"]);
        assert_eq!(saved["hooks"]["Stop"], original["hooks"]["Stop"]);
        let pre = saved["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 3);
        assert_eq!(
            pre[1]["hooks"],
            json!([{"type": "command", "command": "audit-bash"}])
        );
        assert_eq!(
            pre[2]["hooks"][0]["command"],
            "'/Apps/Jitterbug/jitterbug-run' pressure-hint"
        );
        set_hint(&p, None).unwrap();
        assert!(!hint_installed(&p, runner));
        let removed = read_json(&claude_settings(&p)).unwrap();
        assert_eq!(removed["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
        // A broken file is never overwritten.
        std::fs::write(claude_settings(&p), "{ not json").unwrap();
        assert!(set_hint(&p, Some(runner)).is_err());
        assert_eq!(
            std::fs::read_to_string(claude_settings(&p)).unwrap(),
            "{ not json"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn hint_hook_creates_missing_settings() {
        let root = temp("fresh");
        let p = places(&root);
        let runner = Path::new("/Apps/jitterbug-run");
        set_hint(&p, Some(runner)).unwrap();
        assert!(hint_installed(&p, runner));
        set_hint(&p, None).unwrap();
        assert_eq!(read_json(&claude_settings(&p)).unwrap(), json!({}));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_server_is_added_beside_existing_config_and_removed() {
        let root = temp("codex");
        let p = places(&root);
        std::fs::create_dir_all(&p.codex_dir).unwrap();
        let original =
            "# my settings\nmodel = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"other-mcp\"\n";
        std::fs::write(codex_config(&p), original).unwrap();
        let runner = Path::new("/Apps/Jitterbug/jitterbug-run");
        assert!(!status(&p, Some(runner))[1].mcp);
        assert!(status(&p, Some(runner))[1].installed);
        connect(&p, runner, "codex").unwrap();
        connect(&p, runner, "codex").unwrap();
        let text = std::fs::read_to_string(codex_config(&p)).unwrap();
        assert!(text.starts_with("# my settings\nmodel = \"gpt-5\"\n"));
        assert!(text.contains("[mcp_servers.other]\ncommand = \"other-mcp\""));
        assert_eq!(text.matches("[mcp_servers.jitterbug-control]").count(), 1);
        assert!(
            text.contains("command = \"/Apps/Jitterbug/jitterbug-run\"\nargs = [\"control-mcp\"]")
        );
        assert!(status(&p, Some(runner))[1].connected());
        // Another build's path counts as not connected until set up again.
        assert!(!status(&p, Some(Path::new("/elsewhere/jitterbug-run")))[1].mcp);
        disconnect(&p, "codex").unwrap();
        assert_eq!(std::fs::read_to_string(codex_config(&p)).unwrap(), original);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn claude_needs_its_cli_and_reads_the_user_server() {
        let root = temp("claude");
        let p = places(&root);
        let runner = Path::new("/Apps/jitterbug-run");
        assert!(connect(&p, runner, "claude")
            .unwrap_err()
            .contains("not found"));
        std::fs::write(
            &p.claude_json,
            json!({"mcpServers": {SERVER: {"type": "stdio", "command": "/Apps/jitterbug-run", "args": ["control-mcp"], "env": {}}}}).to_string(),
        )
        .unwrap();
        let claude = &status(&p, Some(runner))[0];
        assert!(claude.mcp && claude.hint == Some(false) && !claude.connected());
        assert!(connect(&p, runner, "gemini").is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn claude_registration_goes_through_its_cli() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp("cli");
        let mut p = places(&root);
        // A fake `claude` that records its arguments and writes the server like the real one.
        let log = root.join("calls");
        let cli = root.join("claude");
        std::fs::write(
            &cli,
            format!(
                "#!/bin/sh\necho \"$@\" >> '{log}'\nif [ \"$2\" = add ]; then printf '{{\"mcpServers\":{{\"{SERVER}\":{{\"command\":\"%s\",\"args\":[\"control-mcp\"]}}}}}}' \"$7\" > '{json}'; else echo '{{}}' > '{json}'; fi\n",
                log = log.display(),
                json = p.claude_json.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.claude_cli = Some(cli);
        let runner = Path::new("/Apps/jitterbug-run");
        connect(&p, runner, "claude").unwrap();
        assert!(status(&p, Some(runner))[0].connected());
        // Already registered: nothing to run. A moved app re-registers.
        connect(&p, runner, "claude").unwrap();
        connect(&p, Path::new("/New/jitterbug-run"), "claude").unwrap();
        disconnect(&p, "claude").unwrap();
        assert!(!status(&p, Some(runner))[0].mcp);
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            format!(
                "mcp add --scope user {SERVER} -- /Apps/jitterbug-run control-mcp\n\
                 mcp remove --scope user {SERVER}\n\
                 mcp add --scope user {SERVER} -- /New/jitterbug-run control-mcp\n\
                 mcp remove --scope user {SERVER}\n"
            )
        );
        assert!(!hint_installed(&p, runner));
        let _ = std::fs::remove_dir_all(root);
    }
}
