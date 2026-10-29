use super::*;
use serde_json::{json, Value};

pub const SERVER: &str = "jitterbug-managed";
pub const CLIENTS: [&str; 3] = ["claude", "codex", "meta"];

#[derive(Clone)]
pub struct Locations {
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub meta: PathBuf,
}

impl Locations {
    pub fn current() -> Result<Self, String> {
        let home = dirs::home_dir().ok_or("Home directory unavailable.")?;
        if std::env::var_os("CLAUDE_CONFIG_DIR").is_some() {
            return Err("Custom CLAUDE_CONFIG_DIR needs manual MCP configuration for now.".into());
        }
        Ok(Self {
            claude: home.join(".claude.json"),
            codex: std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"))
                .join("config.toml"),
            meta: std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("muse/settings.json"),
        })
    }
    pub fn path(&self, client: &str) -> Result<&Path, String> {
        match client {
            "claude" => Ok(&self.claude),
            "codex" => Ok(&self.codex),
            "meta" => Ok(&self.meta),
            _ => Err("Unsupported client.".into()),
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Receipt {
    pub client: String,
    pub profiles: Vec<String>,
    pub entry: Value,
}

#[derive(Clone, Serialize)]
pub struct ConnectionStatus {
    pub client: String,
    pub configured: bool,
    pub last_seen: Option<u64>,
    pub message: String,
}

pub fn receipt_path(directory: &Path, client: &str) -> PathBuf {
    directory.join(format!("managed-{client}.json"))
}

pub fn read_receipt(directory: &Path, client: &str) -> Result<Receipt, String> {
    match read_bounded(&receipt_path(directory, client), 64 * 1024)? {
        Some(v) => {
            serde_json::from_slice(&v).map_err(|_| "Invalid managed connection receipt.".into())
        }
        None => Ok(Receipt::default()),
    }
}

fn json_entry(bytes: Option<&[u8]>, client: &str) -> Result<Value, String> {
    if client == "codex" {
        let doc = std::str::from_utf8(bytes.unwrap_or_default())
            .map_err(|_| "Config must be UTF-8.")?
            .parse::<toml_edit::DocumentMut>()
            .map_err(|_| "Existing Codex TOML is invalid; fix it before connecting.")?;
        let Some(table) = doc.get("mcp_servers").and_then(|s| s.get(SERVER)) else {
            return Ok(Value::Null);
        };
        // Only compare our owned entry; unrelated settings and comments remain intact.
        let command = table.get("command").and_then(|v| v.as_str());
        let args: Option<Vec<&str>> = table
            .get("args")
            .and_then(|v| v.as_array())
            .and_then(|a| a.iter().map(|v| v.as_str()).collect());
        let count = table.as_table_like().map_or(0, |t| t.len());
        if count != 2 {
            return Err("Jitterbug's Codex connection has additional settings. Review it manually before reconnecting.".into());
        }
        return Ok(json!({"command":command,"args":args}));
    }
    let doc: Value = match bytes {
        Some(b) => serde_json::from_slice(b)
            .map_err(|_| "Existing client JSON is invalid; fix it before connecting.")?,
        None => json!({}),
    };
    if !doc.is_object() {
        return Err("Client settings must be a JSON object.".into());
    }
    if client == "meta"
        && bytes.is_some()
        && doc.get("schema_version").and_then(Value::as_u64) != Some(1)
    {
        return Err("Muse settings require schema_version 1. Repair the existing settings file before connecting.".into());
    }
    let key = "mcpServers";
    if doc.get(key).is_some_and(|v| !v.is_object()) {
        return Err("MCP settings must be an object.".into());
    }
    Ok(doc
        .get(key)
        .and_then(|m| m.get(SERVER))
        .cloned()
        .unwrap_or(Value::Null))
}

pub fn prepare(
    directory: &Path,
    locations: &Locations,
    client: &str,
    profile: &str,
    executable: &Path,
) -> Result<Vec<Edit>, String> {
    if !CLIENTS.contains(&client) {
        return Err("Unsupported client.".into());
    }
    let path = locations.path(client)?;
    let before = read_bounded(path, 2 * 1024 * 1024)?;
    let existing = json_entry(before.as_deref(), client)?;
    let receipt = read_receipt(directory, client)?;
    if !existing.is_null() && (receipt.client != client || receipt.entry != existing) {
        return Err(format!("The {client} connection was changed outside Jitterbug. Review it manually; no settings were overwritten."));
    }
    let state = super::super::inspect(directory)?;
    let mut profiles: Vec<_> = receipt
        .profiles
        .into_iter()
        .filter(|name| state.profiles.iter().any(|p| &p.name == name))
        .collect();
    if !profiles.iter().any(|p| p == profile) {
        profiles.push(profile.into());
    }
    profiles.sort();
    profiles.dedup();
    let args = vec![
        "mcp".to_string(),
        "--profiles".into(),
        profiles.join(","),
        "--harness".into(),
        client.into(),
        "--connection".into(),
        client.into(),
    ];
    let mut entry = json!({"command": executable, "args": args});
    if client == "claude" {
        entry["type"] = json!("stdio");
    }
    if client == "meta" {
        entry["type"] = json!("stdio");
    }
    let (after, preview) = if client == "codex" {
        let mut doc = std::str::from_utf8(before.as_deref().unwrap_or_default())
            .map_err(|_| "Config must be UTF-8.")?
            .parse::<toml_edit::DocumentMut>()
            .map_err(|_| "Invalid TOML.")?;
        let mut server = toml_edit::Table::new();
        server["command"] =
            toml_edit::value(executable.to_str().ok_or("App location must be UTF-8.")?);
        server["args"] = toml_edit::value(args.iter().collect::<toml_edit::Array>());
        if doc.get("mcp_servers").is_none() {
            doc["mcp_servers"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        if !doc["mcp_servers"].is_table() {
            return Err(
                "Codex MCP config needs a regular TOML table; inline tables require manual setup."
                    .into(),
            );
        }
        doc["mcp_servers"][SERVER] = toml_edit::Item::Table(server.clone());
        (
            doc.to_string().into_bytes(),
            format!("[mcp_servers.{SERVER}]\n{server}"),
        )
    } else {
        let mut doc: Value = before
            .as_ref()
            .map(|b| serde_json::from_slice(b))
            .transpose()
            .map_err(|_| "Invalid JSON.")?
            .unwrap_or(json!({}));
        if client == "meta" {
            doc["schema_version"] = json!(1);
        }
        let key = "mcpServers";
        if doc.get(key).is_none() {
            doc[key] = json!({});
        }
        doc[key][SERVER] = entry.clone();
        (
            serde_json::to_vec_pretty(&doc).map_err(|_| "Cannot encode client settings.")?,
            serde_json::to_string_pretty(&if client == "meta" {
                json!({"schema_version":1,key:{SERVER:entry.clone()}})
            } else {
                json!({key:{SERVER:entry.clone()}})
            })
            .map_err(|_| "Cannot encode preview.")?,
        )
    };
    let saved = Receipt {
        client: client.into(),
        profiles,
        entry,
    };
    Ok(vec![
        Edit {
            path: path.into(),
            before,
            after: Some(after),
            preview: format!(
                "Replace only the {SERVER} MCP entry. Other settings are preserved.\n\n{preview}"
            ),
            visible: true,
        },
        Edit {
            path: receipt_path(directory, client),
            before: read_bounded(&receipt_path(directory, client), 64 * 1024)?,
            after: Some(serde_json::to_vec_pretty(&saved).map_err(|_| "Cannot encode receipt.")?),
            preview: String::new(),
            visible: false,
        },
    ])
}

pub fn status(directory: &Path, locations: &Locations, profile: &str) -> Vec<ConnectionStatus> {
    statuses(directory, locations, &[profile.to_owned()])
        .remove(profile)
        .unwrap_or_default()
}

pub fn statuses(
    directory: &Path,
    locations: &Locations,
    profiles: &[String],
) -> HashMap<String, Vec<ConnectionStatus>> {
    let mut result: HashMap<String, Vec<ConnectionStatus>> =
        profiles.iter().map(|p| (p.clone(), Vec::new())).collect();
    // Read each client file once per refresh, regardless of the number of approved profiles.
    for client in CLIENTS {
        let check = || -> Result<(Receipt, bool, Value), String> {
            let receipt = read_receipt(directory, client)?;
            if receipt.profiles.is_empty() {
                return Ok((receipt, false, Value::Null));
            }
            let current = read_bounded(locations.path(client)?, 2 * 1024 * 1024)?;
            let configured = !receipt.entry.is_null()
                && json_entry(current.as_deref(), client)? == receipt.entry
                && receipt
                    .entry
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|p| Path::new(p).is_file());
            let seen: Value =
                read_bounded(&directory.join(format!("contact-{client}.json")), 16 * 1024)?
                    .map(|b| serde_json::from_slice(&b))
                    .transpose()
                    .map_err(|_| "Invalid client contact record.")?
                    .unwrap_or(Value::Null);
            Ok((receipt, configured, seen))
        };
        let checked = check();
        for (profile, statuses) in &mut result {
            let status = match &checked {
                Ok((receipt, valid, seen)) => {
                    let configured = *valid && receipt.profiles.contains(profile);
                    let last_seen = seen
                        .get("profiles")
                        .and_then(Value::as_array)
                        .filter(|p| p.iter().any(|v| v.as_str() == Some(profile)))
                        .and_then(|_| seen.get("at").and_then(Value::as_u64));
                    ConnectionStatus { client: client.into(), configured, last_seen,
                        message: if configured { if last_seen.is_some() { "Client has contacted this queue. Restart after configuration changes." } else { "Configured. Restart the client and approve its MCP tools; no contact observed yet." } } else { "Not connected, or configuration changed." }.into() }
                }
                Err(error) => ConnectionStatus {
                    client: client.into(),
                    configured: false,
                    last_seen: None,
                    message: error.clone(),
                },
            };
            statuses.push(status);
        }
    }
    result
}

pub fn contact(directory: &Path, client: &str, profiles: &[String]) -> Result<(), String> {
    if !CLIENTS.contains(&client) {
        return Err("Unsupported connection label.".into());
    }
    // Contact means a client initialized MCP, not that every session participates.
    super::super::transaction(directory, |_| {
        storage::write(
            &directory.join(format!("contact-{client}.json")),
            &json!({"at":crate::types::now(),"profiles":profiles}),
        )
    })
}
