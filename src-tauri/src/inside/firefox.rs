//! Read-only Firefox tab attribution through its explicitly enabled local RDP endpoint.
//! No preferences are changed, browser code evaluated, page contents read, or tabs controlled.
use super::{host, Inside, ProcessLabel, Unit};
use crate::types::*;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

const SOURCE: &str = "Firefox DevTools";
static CONNECTION: Mutex<Option<Rdp>> = Mutex::new(None);
struct Rdp {
    stream: TcpStream,
    deadline: Instant,
    bytes: usize,
}

impl Rdp {
    fn read_exact(&mut self, mut bytes: &mut [u8]) -> Result<(), String> {
        while !bytes.is_empty() {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Firefox did not respond in time.".into());
            }
            self.stream
                .set_read_timeout(Some(remaining))
                .map_err(|_| "Firefox connection unavailable.")?;
            let count = self
                .stream
                .read(bytes)
                .map_err(|_| "Firefox connection closed or timed out.")?;
            if count == 0 {
                return Err("Firefox connection closed.".into());
            }
            bytes = &mut bytes[count..];
        }
        Ok(())
    }

    fn read(&mut self) -> Result<Value, String> {
        let mut prefix = Vec::new();
        loop {
            let mut byte = [0];
            self.read_exact(&mut byte)?;
            if byte[0] == b':' {
                break;
            }
            if prefix.len() >= 8 || !byte[0].is_ascii_digit() {
                return Err("Invalid Firefox response.".into());
            }
            prefix.push(byte[0]);
        }
        let length = std::str::from_utf8(&prefix)
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .ok_or("Invalid Firefox response size.")?;
        self.bytes = self.bytes.saturating_add(length);
        if length > 512 * 1024 || self.bytes > 4 * 1024 * 1024 {
            return Err("Firefox response exceeded the local evidence limit.".into());
        }
        let mut bytes = vec![0; length];
        self.read_exact(&mut bytes)?;
        serde_json::from_slice(&bytes).map_err(|_| "Invalid Firefox response.".into())
    }
    fn call(&mut self, actor: &str, kind: &str) -> Result<Value, String> {
        // Caller supplies only actors from the current connection and these two read methods.
        if !matches!(kind, "listTabs" | "getTarget") || actor.len() > 300 {
            return Err("Unsupported Firefox request.".into());
        }
        let body = json!({"to":actor,"type":kind}).to_string();
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Firefox did not respond in time.".into());
        }
        self.stream
            .set_write_timeout(Some(remaining.min(Duration::from_secs(1))))
            .map_err(|_| "Firefox connection unavailable.")?;
        self.stream
            .write_all(format!("{}:{body}", body.len()).as_bytes())
            .map_err(|_| "Firefox connection closed.")?;
        for _ in 0..32 {
            let value = self.read()?;
            if value["from"] == actor {
                if value.get("error").is_some() {
                    return Err("Firefox could not identify this tab.".into());
                }
                return Ok(value);
            }
        }
        Err("Firefox sent too many notifications.".into())
    }
}

pub fn matches(members: &[&ProcessRecord]) -> bool {
    members
        .iter()
        .filter_map(|p| p.executable.as_deref())
        .filter_map(crate::attribution::application_bundle)
        .filter_map(|p| p.file_stem())
        .any(|name| {
            matches!(
                name.to_str(),
                Some("Firefox" | "Firefox Developer Edition" | "Firefox Nightly")
            )
        })
}

fn setup(id: &str) -> Inside {
    let mut view = Inside::status(
        id,
        SOURCE,
        "setup",
        "Firefox has not shared its tab-to-process mapping yet.",
    );
    view.setup = vec![
        "For a quick check, open Firefox’s Process Manager with Shift+Esc; its memory column identifies the largest tab processes.".into(),
        "To show those tabs here, enable remote debugging in Firefox’s Developer Tools settings, keep devtools.debugger.force-local set to true, then start Firefox with --start-debugger-server 6000 after saving your work. This grants a local debugging connection; Jitterbug does not change these settings or restart Firefox.".into(),
        "Choose Show tabs or Refresh here, then allow Firefox’s connection prompt. Only tab identities and process IDs are read; no scripts run and no browsing titles or URLs are sent to AI.".into(),
    ];
    view
}

/// Tabs from Firefox's debugger when the person connected it; otherwise the open tabs from its
/// local session file, and every helper process named by the kind Firefox launched it as.
pub fn inspect(snapshot: &Snapshot, id: &str, members: &[&ProcessRecord], connect: bool) -> Inside {
    let mut view = inspect_debugger(snapshot, id, members, connect);
    let (kinds, profile) = launched_as(members);
    if view.status == "setup" {
        if let Some(tabs) = profile.as_deref().and_then(session_tabs) {
            view = local(id, view.setup, tabs);
        }
    }
    for p in members {
        if view.labels.iter().any(|l| l.pid == p.pid) {
            continue;
        }
        if let Some((title, subtitle)) = kinds.get(&p.pid).and_then(|k| kind_label(k)) {
            view.labels.push(ProcessLabel::new(p, title, subtitle));
        }
    }
    view
}

fn inspect_debugger(
    snapshot: &Snapshot,
    id: &str,
    members: &[&ProcessRecord],
    connect: bool,
) -> Inside {
    let Ok(mut connection) = CONNECTION.try_lock() else {
        return Inside::status(
            id,
            SOURCE,
            "unavailable",
            "Firefox is already being read. Try again shortly.",
        );
    };
    if connection.is_none() {
        if !connect {
            return setup(id);
        }
        let address = SocketAddr::from(([127, 0, 0, 1], 6000));
        let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_millis(300)) else {
            return setup(id);
        };
        let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
        let mut rdp = Rdp {
            stream,
            // Explicit Show tabs can trigger Firefox's permission prompt. Give the person
            // time to answer; automatic chat evidence never opens this connection.
            deadline: Instant::now() + Duration::from_secs(30),
            bytes: 0,
        };
        let Ok(greeting) = rdp.read() else {
            return setup(id);
        };
        if greeting["applicationType"] != "browser" {
            return setup(id);
        }
        *connection = Some(rdp);
    }
    let rdp = connection.as_mut().unwrap();
    rdp.deadline = Instant::now() + Duration::from_secs(3);
    rdp.bytes = 0;
    match read_tabs(rdp, snapshot, id, members) {
        Ok(view) => view,
        Err(_) => {
            *connection = None;
            setup(id)
        }
    }
}

fn read_tabs(
    rdp: &mut Rdp,
    snapshot: &Snapshot,
    id: &str,
    members: &[&ProcessRecord],
) -> Result<Inside, String> {
    let response = rdp.call("root", "listTabs")?;
    let tabs = response["tabs"]
        .as_array()
        .ok_or("Firefox returned no tab inventory.")?;
    let mut observations = Vec::new();
    for tab in tabs.iter().take(100) {
        // Do not wake discarded tabs merely to identify them, or inspect private contexts.
        if tab["isZombieTab"] == true || tab["isPrivate"] == true {
            continue;
        }
        let Some(actor) = tab["actor"].as_str() else {
            continue;
        };
        let target = rdp.call(actor, "getTarget")?;
        if target["frame"]["isPrivate"] == true {
            continue;
        }
        let Some(pid) = target["frame"]["processID"]
            .as_u64()
            .and_then(|p| u32::try_from(p).ok())
        else {
            continue;
        };
        observations.push((tab.clone(), pid));
    }
    Ok(attributed(snapshot, id, members, observations))
}

fn attributed(
    snapshot: &Snapshot,
    id: &str,
    members: &[&ProcessRecord],
    tabs: Vec<(Value, u32)>,
) -> Inside {
    let mut view = Inside::status(
        id,
        SOURCE,
        "ready",
        "Tabs matched to Firefox’s own process IDs.",
    );
    view.captured_at = snapshot.captured_at;
    view.limits = "Memory and CPU are OS process readings. Tabs sharing a process show an estimated split; cross-site subframes can run in additional processes. Private and discarded tabs are omitted. Tab titles and addresses stay on this computer.".into();
    let mut counts = HashMap::new();
    for (_, pid) in &tabs {
        *counts.entry(*pid).or_insert(0usize) += 1;
    }
    for (tab, pid) in tabs {
        let Some(process) = members.iter().find(|p| p.pid == pid) else {
            continue;
        };
        let count = counts[&pid].max(1);
        let title: String = tab["title"]
            .as_str()
            .unwrap_or("Untitled tab")
            .chars()
            .take(180)
            .collect();
        let address: String = tab["url"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(400)
            .collect();
        view.units.push(Unit {
            id: tab["actor"].as_str().unwrap_or_default().into(), kind: "tab".into(),
            title, host: host(&address), detail: address,
            cpu_percent: process.cpu_percent / count as f32,
            memory_bytes: process.memory_bytes / count as u64,
            process_count: 1, shared_with: count - 1,
            pid: Some(pid), start_time: Some(process.start_time),
            confidence: "Firefox-reported process ID".into(),
            evidence: vec!["Firefox’s tab target reported this PID; it belongs to the selected app in the current OS inventory.".into()],
            actions: vec![], ..Default::default()
        });
    }
    for p in members {
        let tabs: Vec<_> = view.units.iter().filter(|t| t.pid == Some(p.pid)).collect();
        if tabs.is_empty() {
            continue;
        }
        let title = if tabs.len() == 1 {
            tabs[0].title.clone()
        } else {
            format!(
                "{} tabs: {}",
                tabs.len(),
                tabs.iter()
                    .take(3)
                    .map(|t| t.title.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        view.labels.push(ProcessLabel::new(
            p,
            title,
            tabs.iter()
                .take(3)
                .filter_map(|t| t.host.as_deref())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    view.units
        .sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes));
    view
}

/// The kind Firefox gave each child process (its last launch argument), and the profile folder.
/// Arguments are read for these processes only and dropped here; only allowlisted words remain.
fn launched_as(members: &[&ProcessRecord]) -> (HashMap<u32, String>, Option<PathBuf>) {
    let pids: Vec<_> = members
        .iter()
        .filter(|p| p.name == "plugin-container" || p.name == "firefox")
        .take(256)
        .map(|p| sysinfo::Pid::from_u32(p.pid))
        .collect();
    let mut kinds = HashMap::new();
    let mut profile = None;
    if pids.is_empty() {
        return (kinds, profile);
    }
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&pids),
        true,
        sysinfo::ProcessRefreshKind::nothing().with_cmd(sysinfo::UpdateKind::Always),
    );
    for p in system.processes().values() {
        let args: Vec<String> = p
            .cmd()
            .iter()
            .take(96)
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        if profile.is_none() {
            profile = args
                .iter()
                .position(|a| a == "-profile")
                .and_then(|i| args.get(i + 1))
                .map(PathBuf::from)
                .filter(|path| path.is_absolute());
        }
        if let Some(kind) = args.last().filter(|k| {
            matches!(
                k.as_str(),
                "tab" | "gpu" | "rdd" | "socket" | "utility" | "forkserver" | "gmplugin"
            )
        }) {
            kinds.insert(p.pid().as_u32(), kind.clone());
        }
    }
    (kinds, profile.or_else(default_profile))
}

fn kind_label(kind: &str) -> Option<(&'static str, &'static str)> {
    Some(match kind {
        "tab" => ("Web content", "Pages from one site or a few tabs"),
        "gpu" => ("Graphics", "Draws every tab on screen"),
        "rdd" => ("Media decoding", "Decodes audio and video"),
        "socket" => ("Network", "Firefox’s network connections"),
        "utility" => (
            "Utility services",
            "Audio, file access, and other shared services",
        ),
        "gmplugin" => ("Media plug-in", "Plays protected video"),
        "forkserver" => ("Process launcher", "Starts Firefox’s other processes"),
        _ => return None,
    })
}

/// The most recently used Firefox profile, for when no process names its own.
fn default_profile() -> Option<PathBuf> {
    let root = dirs::data_dir()?.join("Firefox/Profiles");
    std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file = entry.path().join("sessionstore-backups/recovery.jsonlz4");
            let modified = std::fs::metadata(&file).ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// One open tab from the session file: what it shows now, never its history.
#[derive(Debug, PartialEq)]
struct OpenTab {
    title: String,
    url: String,
}

/// Open, visible, non-private tabs from Firefox's own crash-recovery file. Read-only; Firefox
/// rewrites it every few seconds, so this is current to within about 15 seconds.
fn session_tabs(profile: &Path) -> Option<Vec<OpenTab>> {
    let file = profile.join("sessionstore-backups/recovery.jsonlz4");
    if std::fs::metadata(&file).ok()?.len() > 64 * 1024 * 1024 {
        return None;
    }
    let bytes = std::fs::read(file).ok()?;
    let json = moz_lz4(&bytes)?;
    let session: Value = serde_json::from_slice(&json).ok()?;
    Some(open_tabs(&session))
}

fn open_tabs(session: &Value) -> Vec<OpenTab> {
    let mut tabs = Vec::new();
    for window in session["windows"].as_array().into_iter().flatten() {
        if window["isPrivate"] == true {
            continue;
        }
        for tab in window["tabs"].as_array().into_iter().flatten() {
            if tab["hidden"] == true {
                continue;
            }
            let entries = tab["entries"].as_array();
            let index = tab["index"].as_u64().unwrap_or(1).max(1) as usize - 1;
            let Some(entry) = entries.and_then(|e| e.get(index).or(e.last())) else {
                continue;
            };
            let url = entry["url"].as_str().unwrap_or_default();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                continue;
            }
            let title = entry["title"].as_str().filter(|t| !t.trim().is_empty());
            tabs.push(OpenTab {
                title: title.unwrap_or(url).chars().take(180).collect(),
                url: url.chars().take(400).collect(),
            });
            if tabs.len() >= 300 {
                return tabs;
            }
        }
    }
    tabs
}

/// Decode Mozilla's `mozLz40` container: an 8-byte magic, the decoded size, one LZ4 block.
fn moz_lz4(bytes: &[u8]) -> Option<Vec<u8>> {
    let body = bytes.strip_prefix(b"mozLz40\0")?;
    let size = u32::from_le_bytes(body.get(..4)?.try_into().ok()?) as usize;
    if size > 128 * 1024 * 1024 {
        return None;
    }
    lz4_block(body.get(4..)?, size)
}

fn lz4_block(input: &[u8], size: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(size);
    let mut i = 0;
    let length = |i: &mut usize, mut n: usize| -> Option<usize> {
        if n == 15 {
            loop {
                let byte = *input.get(*i)?;
                *i += 1;
                n += byte as usize;
                if byte != 255 {
                    break;
                }
            }
        }
        Some(n)
    };
    while i < input.len() {
        let token = input[i];
        i += 1;
        let literals = length(&mut i, (token >> 4) as usize)?;
        out.extend_from_slice(input.get(i..i.checked_add(literals)?)?);
        i += literals;
        if i >= input.len() {
            break;
        }
        let offset = u16::from_le_bytes([*input.get(i)?, *input.get(i + 1)?]) as usize;
        i += 2;
        let count = length(&mut i, (token & 15) as usize)? + 4;
        if offset == 0 || offset > out.len() || out.len() + count > size {
            return None;
        }
        let start = out.len() - offset;
        for k in 0..count {
            out.push(out[start + k]);
        }
    }
    (out.len() == size).then_some(out)
}

/// Open tabs grouped by site. Firefox runs each site in its own processes but does not say
/// which, so no tab gets a process or a number here.
fn local(id: &str, setup: Vec<String>, tabs: Vec<OpenTab>) -> Inside {
    let mut sites: Vec<(String, Vec<OpenTab>)> = Vec::new();
    for tab in tabs {
        let site = host(&tab.url)
            .map(|h| h.trim_start_matches("www.").to_string())
            .unwrap_or_else(|| "Other".into());
        match sites.iter_mut().find(|(s, _)| *s == site) {
            Some((_, list)) => list.push(tab),
            None => sites.push((site, vec![tab])),
        }
    }
    sites.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    let count: usize = sites.iter().map(|(_, t)| t.len()).sum();
    let mut view = Inside::status(
        id,
        "Firefox session",
        "partial",
        format!(
            "{count} open tab{} across {} site{}.",
            if count == 1 { "" } else { "s" },
            sites.len(),
            if sites.len() == 1 { "" } else { "s" }
        ),
    );
    view.setup = setup;
    view.limits = "Read from Firefox’s own session file on this Mac. Firefox keeps each site in separate web content processes but doesn’t say which process belongs to which tab; connect its debugger to match them. Private windows are never read. Titles and addresses stay on this computer.".into();
    for (site, tabs) in sites {
        for (i, tab) in tabs.into_iter().enumerate() {
            view.units.push(Unit {
                id: format!("{site}#{i}"),
                kind: "open-tab".into(),
                title: tab.title,
                host: Some(site.clone()),
                detail: tab.url,
                confidence: "Firefox session file".into(),
                evidence: vec![
                    "Listed in Firefox’s session file; Firefox does not report its process.".into(),
                ],
                ..Default::default()
            });
        }
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    fn connection() -> (Rdp, TcpStream) {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (peer, _) = listener.accept().unwrap();
        (
            Rdp {
                stream,
                deadline: Instant::now() + Duration::from_secs(1),
                bytes: 0,
            },
            peer,
        )
    }

    #[test]
    fn protocol_reads_framed_replies_and_skips_notifications() {
        let (mut client, mut server) = connection();
        let notice = json!({"from":"other","type":"tabNavigated"}).to_string();
        let reply = json!({"from":"root","tabs":[]}).to_string();
        server
            .write_all(format!("{}:{notice}{}:{reply}", notice.len(), reply.len()).as_bytes())
            .unwrap();
        assert_eq!(client.call("root", "listTabs").unwrap()["tabs"], json!([]));
        assert!(client.call("root", "evaluateJSAsync").is_err());
    }

    #[test]
    fn protocol_rejects_oversized_packets_and_expired_deadlines() {
        let (mut client, mut server) = connection();
        server.write_all(b"999999:").unwrap();
        assert!(client.read().unwrap_err().contains("evidence limit"));
        client.deadline = Instant::now();
        assert!(client.read().unwrap_err().contains("in time"));
    }

    #[test]
    fn only_current_app_processes_are_attributed_and_shared_memory_is_not_double_counted() {
        let p = ProcessRecord {
            pid: 42,
            start_time: 7,
            memory_bytes: 1000,
            ..Default::default()
        };
        let tabs = vec![
            (
                json!({"actor":"a","title":"One","url":"https://example.com/a"}),
                42,
            ),
            (
                json!({"actor":"b","title":"Two","url":"https://example.com/b"}),
                42,
            ),
            (json!({"actor":"c","title":"Other app"}), 999),
        ];
        let view = attributed(&Snapshot::default(), "app", &[&p], tabs);
        assert_eq!(view.units.len(), 2);
        assert_eq!(view.units.iter().map(|u| u.memory_bytes).sum::<u64>(), 1000);
        assert!(view
            .units
            .iter()
            .all(|u| u.shared_with == 1 && u.actions.is_empty() && u.start_time == Some(7)));
        assert_eq!(view.labels.len(), 1);
        assert!(view.labels[0].title.starts_with("2 tabs:"));
    }

    #[test]
    fn lz4_blocks_decode_literals_and_overlapping_matches() {
        // "abcabcabcabc!": 3 literals, a 9-byte match at offset 3, then a final literal.
        let block = [0x35, b'a', b'b', b'c', 3, 0, 0x10, b'!'];
        assert_eq!(lz4_block(&block, 13).unwrap(), b"abcabcabcabc!");
        let mut file = b"mozLz40\0".to_vec();
        file.extend_from_slice(&13u32.to_le_bytes());
        file.extend_from_slice(&block);
        assert_eq!(moz_lz4(&file).unwrap(), b"abcabcabcabc!");
        assert!(lz4_block(&[0x05, b'a', 9, 0], 9).is_none());
        assert!(moz_lz4(b"not firefox").is_none());
    }

    #[test]
    fn session_lists_current_pages_and_skips_private_hidden_and_internal_tabs() {
        let session = json!({"windows":[
            {"tabs":[
                {"index":2,"entries":[{"url":"https://old.example/","title":"Old"},{"url":"https://www.youtube.com/watch","title":"Video"}]},
                {"index":1,"entries":[{"url":"about:newtab","title":"New Tab"}]},
                {"hidden":true,"index":1,"entries":[{"url":"https://hidden.example/"}]},
                {"index":1,"entries":[{"url":"https://mail.example.com/inbox","title":""}]}
            ]},
            {"isPrivate":true,"tabs":[{"index":1,"entries":[{"url":"https://secret.example/"}]}]}
        ]});
        let tabs = open_tabs(&session);
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].title, "Video");
        assert_eq!(tabs[1].title, "https://mail.example.com/inbox");
        let view = local("app", vec![], tabs);
        assert_eq!(view.status, "partial");
        assert!(view
            .units
            .iter()
            .all(|u| u.pid.is_none() && u.memory_bytes == 0));
        assert_eq!(view.message, "2 open tabs across 2 sites.");
    }

    #[test]
    fn helper_kinds_have_plain_names() {
        assert_eq!(kind_label("tab").unwrap().0, "Web content");
        assert!(kind_label("org.mozilla.machname.1").is_none());
    }
}
