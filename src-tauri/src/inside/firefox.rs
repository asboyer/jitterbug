//! Read-only Firefox tab attribution through its explicitly enabled local RDP endpoint.
//! No preferences are changed, browser code evaluated, page contents read, or tabs controlled.
use super::{host, Inside, ProcessLabel, Unit};
use crate::types::*;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
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

pub fn inspect(snapshot: &Snapshot, id: &str, members: &[&ProcessRecord], connect: bool) -> Inside {
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
}
