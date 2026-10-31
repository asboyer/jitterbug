//! Inside an app: split one workload into the units a person can act on (browser tabs,
//! terminal/editor/agent sessions) with each unit's measured share. Local and on demand only:
//! titles, URLs, and working directories reach the detail view, never events or model prompts.
pub mod chrome;
pub mod options;
pub mod sessions;

use crate::types::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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
    /// Processes this unit's numbers cover (a session's whole tree; a tab's renderer).
    pub process_count: usize,
    /// Other units measured in the same process(es); their split is estimated.
    pub shared_with: usize,
    pub pid: Option<u32>,
    pub start_time: Option<u64>,
    pub actions: Vec<String>,
    pub evidence: Vec<String>,
    pub confidence: String,
    /// When a paused session resumes by itself.
    pub paused_until: Option<u64>,
    /// A coding agent's own state: `working` or `idle` (waiting for the person).
    #[serde(default)]
    pub agent_status: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Remainder {
    pub label: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub process_count: usize,
}

/// What one process of the workload is, in a person's terms: the tab or session it runs, or its
/// role in the app. Local-only like unit titles; never sent to events, logs, or model prompts.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ProcessLabel {
    pub pid: u32,
    pub start_time: u64,
    pub title: String,
    /// Secondary context: a tab's host or what a role covers. May be empty.
    pub subtitle: String,
}

impl ProcessLabel {
    fn new(p: &ProcessRecord, title: impl Into<String>, subtitle: impl Into<String>) -> Self {
        Self {
            pid: p.pid,
            start_time: p.start_time,
            title: title.into(),
            subtitle: subtitle.into(),
        }
    }
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
    /// Names for the workload's processes; processes with nothing better are left out.
    pub labels: Vec<ProcessLabel>,
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

/// Blocking: a browser view samples for about one second and may make the browser ask once.
pub fn inspect(snapshot: &Snapshot, workload_id: &str) -> Result<Inside, String> {
    inspect_with(snapshot, workload_id, true)
}

/// Without `connect`, a browser is only measured through a connection the person already
/// allowed, so automatic labeling never makes the browser ask.
pub fn inspect_with(
    snapshot: &Snapshot,
    workload_id: &str,
    connect: bool,
) -> Result<Inside, String> {
    let members = crate::details::members(snapshot, workload_id)?;
    let lens = lens(&members);
    let mut inside = match lens {
        Lens::Browser if connect => chrome::inspect(snapshot, workload_id, &members),
        Lens::Browser if chrome::connected() => {
            chrome::inspect_connected(snapshot, workload_id, &members)
        }
        Lens::Browser => Inside::status(
            workload_id,
            chrome::SOURCE,
            "setup",
            "Choose Show tabs to see which tab each process runs.",
        ),
        Lens::Sessions => sessions::inspect(snapshot, workload_id, &members),
        Lens::None => Inside::status(
            workload_id,
            "None",
            "unavailable",
            "No inside view for this app yet. Browsers built on Chromium, terminals, editors, and coding-agent apps are supported.",
        ),
    };
    let browser = matches!(lens, Lens::Browser);
    if !matches!(lens, Lens::None) {
        for p in &members {
            if inside.labels.iter().any(|l| l.pid == p.pid) {
                continue;
            }
            if let Some((title, subtitle)) = role(p, browser) {
                inside.labels.push(ProcessLabel::new(p, title, subtitle));
            }
        }
    }
    Ok(inside)
}

/// A helper's role from its name, for processes no tab or session explains. Only processes
/// inside an app bundle: a command's own name is already the best name for it.
fn role(p: &ProcessRecord, browser: bool) -> Option<(&'static str, &'static str)> {
    let executable = p.executable.as_deref()?;
    let bundle = crate::attribution::application_bundle(executable)?;
    let main = executable.parent() == Some(bundle.join("Contents/MacOS").as_path());
    let name = p.name.to_lowercase();
    Some(if name.contains("crashpad") {
        ("Crash reporter", "Sends a report if the app crashes")
    } else if name.ends_with("(renderer)") && browser {
        (
            "Tab or extension (not identified)",
            "Choose Show tabs to identify it",
        )
    } else if name.ends_with("(renderer)") {
        ("Window or extension", "Draws a window, panel, or web view")
    } else if name.ends_with("(gpu)") && browser {
        chrome::role("GPU").1
    } else if name.ends_with("(gpu)") {
        ("Graphics", "Shared by all windows")
    } else if name.ends_with("(plugin)") {
        ("Extensions", "Extension host or plug-in")
    } else if name.ends_with("(alerts)") {
        ("Notifications", "Shows the app's alerts")
    } else if name.contains("helper") && browser {
        chrome::role("utility").1
    } else if name.contains("helper") {
        ("Background helper", "Services the app's windows")
    } else if !main {
        // An updater or other bundled tool: its own name says more than a guess.
        return None;
    } else if browser {
        chrome::role("browser").1
    } else {
        ("App window", "The app's main process")
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
    fn helpers_get_plain_roles_without_ever_contacting_the_browser() {
        let bundle = "/Applications/Google Chrome.app/Contents";
        let helper = |pid: u32, name: &str, exe: String| ProcessRecord {
            pid,
            start_time: pid as u64,
            name: name.into(),
            executable: Some(exe.into()),
            workload_id: "chrome".into(),
            ..Default::default()
        };
        let frameworks = format!("{bundle}/Frameworks/F.framework/Helpers");
        let snapshot = Snapshot {
            source: "live".into(),
            captured_at: now(),
            sample_ready: true,
            processes: vec![
                helper(1, "Google Chrome", format!("{bundle}/MacOS/Google Chrome")),
                helper(
                    2,
                    "Google Chrome Helper (Renderer)",
                    format!("{frameworks}/R.app/Contents/MacOS/r"),
                ),
                helper(
                    3,
                    "Google Chrome Helper (Renderer)",
                    format!("{frameworks}/R.app/Contents/MacOS/r"),
                ),
                helper(
                    4,
                    "Google Chrome Helper (GPU)",
                    format!("{frameworks}/G.app/Contents/MacOS/g"),
                ),
                helper(
                    5,
                    "Google Chrome Helper",
                    format!("{frameworks}/H.app/Contents/MacOS/h"),
                ),
                helper(
                    6,
                    "chrome_crashpad_handler",
                    format!("{frameworks}/crashpad"),
                ),
                helper(7, "some-tool", "/usr/local/bin/some-tool".into()),
            ],
            ..Default::default()
        };
        // No connection was ever allowed in tests, so this must not measure or prompt.
        let inside = inspect_with(&snapshot, "chrome", false).unwrap();
        assert_eq!(inside.status, "setup");
        assert!(inside.units.is_empty());
        let title = |pid: u32| {
            inside
                .labels
                .iter()
                .find(|l| l.pid == pid)
                .map(|l| l.title.as_str())
        };
        assert_eq!(title(1), Some("Browser window"));
        assert_eq!(title(2), Some("Tab or extension (not identified)"));
        assert_eq!(title(3), Some("Tab or extension (not identified)"));
        assert_eq!(title(4), Some("Graphics (shared by all tabs)"));
        assert_eq!(title(5), Some("Network & storage"));
        assert_eq!(title(6), Some("Crash reporter"));
        // Outside an app bundle, or a bundled tool, the own name is already the best name.
        assert_eq!(title(7), None);
        assert_eq!(title(8), None);
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
