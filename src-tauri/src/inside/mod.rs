//! Inside an app: split one workload into the units a person can act on (browser tabs,
//! terminal/editor/agent sessions) with each unit's measured share. Local and on demand only:
//! titles, URLs, and working directories reach the detail view, never events or model prompts.
pub mod chrome;
pub mod sessions;

use crate::types::*;
use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Unit {
    /// Tab target ID or `pid-start` of a session root.
    pub id: String,
    pub kind: String,
    pub title: String,
    /// Local-only context: a tab URL or a session working directory.
    pub detail: String,
    /// Shareable summary (a host or command name) suitable for prompts.
    pub host: Option<String>,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    /// Other units measured in the same process(es); their split is estimated.
    pub shared_with: usize,
    pub pid: Option<u32>,
    pub start_time: Option<u64>,
    pub actions: Vec<String>,
    pub evidence: Vec<String>,
    pub confidence: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Remainder {
    pub label: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub process_count: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Inside {
    pub workload_id: String,
    pub source: String,
    /// `ready`, `setup` (needs a one-time user step), or `unavailable`.
    pub status: String,
    pub message: String,
    pub setup: Vec<String>,
    pub units: Vec<Unit>,
    pub remainder: Vec<Remainder>,
    pub measured_seconds: f32,
    pub captured_at: u64,
    pub limits: String,
}

impl Inside {
    fn status(workload_id: &str, source: &str, status: &str, message: impl Into<String>) -> Self {
        Self {
            workload_id: workload_id.into(),
            source: source.into(),
            status: status.into(),
            message: message.into(),
            captured_at: now(),
            ..Default::default()
        }
    }
}

pub enum Lens {
    Browser,
    Sessions,
    None,
}

/// Pick a lens from the workload's executables; labels alone can be renamed by the app.
pub fn lens(members: &[&ProcessRecord]) -> Lens {
    let bundles: Vec<String> = members
        .iter()
        .filter_map(|p| p.executable.as_deref())
        .filter_map(crate::attribution::application_bundle)
        .filter_map(|b| b.file_stem())
        .map(|s| s.to_string_lossy().to_lowercase())
        .collect();
    let any = |names: &[&str]| bundles.iter().any(|b| names.iter().any(|n| b == n));
    if any(chrome::BROWSERS) {
        Lens::Browser
    } else if any(sessions::HOSTS) {
        Lens::Sessions
    } else {
        Lens::None
    }
}

/// Blocking: a browser view samples for about one second.
pub fn inspect(snapshot: &Snapshot, workload_id: &str) -> Result<Inside, String> {
    let members = crate::details::members(snapshot, workload_id)?;
    Ok(match lens(&members) {
        Lens::Browser => chrome::inspect(snapshot, workload_id, &members),
        Lens::Sessions => sessions::inspect(snapshot, workload_id, &members),
        Lens::None => Inside::status(
            workload_id,
            "None",
            "unavailable",
            "No inside view for this app yet. Browsers built on Chromium, terminals, editors, and coding-agent apps are supported.",
        ),
    })
}

pub fn size(bytes: u64) -> String {
    let mib = bytes as f64 / 1_048_576.0;
    if mib >= 1024.0 {
        format!("{:.1} GiB", mib / 1024.0)
    } else {
        format!("{mib:.0} MiB")
    }
}

/// Host part of a URL without scheme, credentials, port, path, or query.
pub fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    (!host.is_empty()).then(|| host.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_strips_everything_private() {
        assert_eq!(
            host("https://user:pw@meet.google.com:443/abc-defg?x=1").as_deref(),
            Some("meet.google.com")
        );
        assert_eq!(host("about:blank"), None);
        assert_eq!(host("chrome://newtab/").as_deref(), Some("newtab"));
    }

    #[test]
    fn lens_follows_the_bundle_not_the_label() {
        let chrome = ProcessRecord {
            executable: Some("/Applications/Google Chrome.app/Contents/Frameworks/X/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/r".into()),
            ..Default::default()
        };
        assert!(matches!(lens(&[&chrome]), Lens::Browser));
        let code = ProcessRecord {
            executable: Some("/Users/x/IDE/Visual Studio Code.app/Contents/MacOS/Code".into()),
            ..Default::default()
        };
        assert!(matches!(lens(&[&code]), Lens::Sessions));
        let other = ProcessRecord {
            executable: Some("/Applications/Spotify.app/Contents/MacOS/Spotify".into()),
            ..Default::default()
        };
        assert!(matches!(lens(&[&other]), Lens::None));
    }
}
