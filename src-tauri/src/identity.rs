//! Local, deterministic identity evidence for apps and executables. Reads only bundle metadata
//! (Info.plist), an Electron app's packaged package.json, and installed manual-page NAME lines.
//! Never spawns processes, follows network links, or reads arguments, documents, or user data.
use crate::applications::Application;
use std::{
    collections::VecDeque,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

const CACHE_LIMIT: usize = 256;
const PACKAGE_LIMIT: u64 = 64 * 1024;
const ASAR_HEADER_LIMIT: u64 = 8 * 1024 * 1024;
const MANUAL_LIMIT: u64 = 64 * 1024;

/// Evidence read from disk for one bundle or executable. Plain strings only; never paths.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Evidence {
    pub package_name: Option<String>,
    pub package_description: Option<String>,
    pub package_author: Option<String>,
    /// Human phrase for `LSApplicationCategoryType`, e.g. "a developer tool".
    pub app_category: Option<&'static str>,
    pub vendor: Option<String>,
    pub manual_summary: Option<String>,
    pub electron: bool,
    /// The bundle's main executable name, to tell its helpers apart.
    pub main_executable: Option<String>,
}

impl Evidence {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

type Entry = (PathBuf, Option<SystemTime>, Evidence);
static CACHE: Mutex<VecDeque<Entry>> = Mutex::new(VecDeque::new());

/// Evidence for an identified app or executable, cached per path and modification time.
pub fn evidence(app: &Application) -> Evidence {
    if app.path.as_os_str().is_empty() {
        return Evidence::default();
    }
    let modified = std::fs::metadata(&app.path).and_then(|m| m.modified()).ok();
    if let Ok(mut entries) = CACHE.lock() {
        if let Some(index) = entries
            .iter()
            .position(|(path, time, _)| *path == app.path && *time == modified)
        {
            let entry = entries.remove(index).expect("index is in bounds");
            let value = entry.2.clone();
            entries.push_back(entry);
            return value;
        }
    }
    let value = gather(app);
    if let Ok(mut entries) = CACHE.lock() {
        entries.retain(|(path, _, _)| *path != app.path);
        if entries.len() >= CACHE_LIMIT {
            entries.pop_front();
        }
        entries.push_back((app.path.clone(), modified, value.clone()));
    }
    value
}

fn gather(app: &Application) -> Evidence {
    let mut evidence = Evidence::default();
    let bundle = app.path.extension().is_some_and(|e| e == "app");
    if bundle {
        let contents = app.path.join("Contents");
        evidence.electron = contents
            .join("Frameworks/Electron Framework.framework")
            .exists();
        let package = read_asar_package(&contents.join("Resources/app.asar")).or_else(|| {
            read_bounded(&contents.join("Resources/app/package.json"), PACKAGE_LIMIT)
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        });
        if let Some(package) = package {
            evidence.package_name = text_field(&package["productName"])
                .or_else(|| text_field(&package["name"]).filter(|n| !n.contains('/')));
            evidence.package_description = text_field(&package["description"]);
            evidence.package_author = author(&package["author"]);
        }
        #[cfg(target_os = "macos")]
        if let Some(info) = read_info(&contents.join("Info.plist")) {
            evidence.app_category = info.category.as_deref().and_then(category_phrase);
            evidence.vendor = info
                .copyright
                .as_deref()
                .and_then(vendor_from_copyright)
                .or_else(|| info.get_info.as_deref().and_then(vendor_from_copyright));
            evidence.main_executable = info.executable;
        }
    } else {
        // Plain executables and XPC/framework daemons: the installed manual page, if any.
        evidence.manual_summary = manual_summary(&app.executable_name);
    }
    evidence
}

fn text_field(value: &serde_json::Value) -> Option<String> {
    clean(value.as_str()?, 160)
}

/// Bounded, single-line display text without control characters.
fn clean(value: &str, limit: usize) -> Option<String> {
    let text: String = value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect();
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// `author` as a string ("Name <email> (url)") or `{ "name": ... }`. Emails and URLs are dropped.
fn author(value: &serde_json::Value) -> Option<String> {
    let raw = value
        .as_str()
        .or_else(|| value["name"].as_str())?
        .to_string();
    let mut name = String::new();
    let mut depth = 0u32;
    for c in raw.chars() {
        match c {
            '<' | '(' => depth += 1,
            '>' | ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => name.push(c),
            _ => (),
        }
    }
    clean(&name, 60).filter(|n| !n.contains('@') && !n.contains("://"))
}

fn read_bounded(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

/// Read only `package.json` from an Electron `app.asar` archive. The archive starts with a
/// 16-byte pickle prefix: bytes 4..8 hold the header pickle size and bytes 12..16 the JSON
/// header length. File data begins at `8 + header pickle size`; entries give string offsets.
pub(crate) fn read_asar_package(path: &Path) -> Option<serde_json::Value> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let mut prefix = [0u8; 16];
    file.read_exact(&mut prefix).ok()?;
    let word = |at: usize| u32::from_le_bytes(prefix[at..at + 4].try_into().unwrap()) as u64;
    let (pickle, json_length) = (word(4), word(12));
    if json_length == 0 || json_length > ASAR_HEADER_LIMIT || 16 + json_length > length {
        return None;
    }
    let mut header = vec![0u8; json_length as usize];
    file.read_exact(&mut header).ok()?;
    let header: serde_json::Value = serde_json::from_slice(&header).ok()?;
    let entry = &header["files"]["package.json"];
    if entry["unpacked"].as_bool() == Some(true) {
        let unpacked = path.with_extension("asar.unpacked").join("package.json");
        return serde_json::from_slice(&read_bounded(&unpacked, PACKAGE_LIMIT)?).ok();
    }
    let size = entry["size"].as_u64()?;
    let offset: u64 = match &entry["offset"] {
        serde_json::Value::String(text) => text.parse().ok()?,
        value => value.as_u64()?,
    };
    let start = 8u64.checked_add(pickle)?.checked_add(offset)?;
    if size == 0 || size > PACKAGE_LIMIT || start.checked_add(size)? > length {
        return None;
    }
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = vec![0u8; size as usize];
    file.read_exact(&mut bytes).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn manual_summary(executable: &str) -> Option<String> {
    manual_summary_in(
        &[
            Path::new("/usr/share/man/man8"),
            Path::new("/usr/share/man/man1"),
        ],
        executable,
    )
}

/// The NAME-line summary of an installed manual page. Plain troff only; never runs `man`.
pub(crate) fn manual_summary_in(directories: &[&Path], executable: &str) -> Option<String> {
    if executable.is_empty()
        || executable.len() > 100
        || executable.contains(['/', '\\'])
        || executable.starts_with('.')
    {
        return None;
    }
    directories.iter().find_map(|directory| {
        let section = directory
            .file_name()?
            .to_str()?
            .strip_prefix("man")
            .unwrap_or("1")
            .to_string();
        let bytes = read_bounded(
            &directory.join(format!("{executable}.{section}")),
            MANUAL_LIMIT,
        )?;
        parse_manual(&String::from_utf8_lossy(&bytes))
    })
}

/// Parse `.Sh NAME` / `.Nd summary` (mdoc) or `.SH NAME` then `name \- summary` (man).
pub(crate) fn parse_manual(text: &str) -> Option<String> {
    let mut in_name = false;
    for line in text.lines().map(|l| l.trim_end_matches('\r').trim()) {
        let upper = line.to_ascii_uppercase();
        if let Some(heading) = upper.strip_prefix(".SH") {
            if in_name {
                return None;
            }
            in_name = heading
                .split("\\\"")
                .next()
                .unwrap_or_default()
                .trim()
                .trim_matches('"')
                == "NAME";
            continue;
        }
        if !in_name {
            continue;
        }
        if let Some(summary) = line.strip_prefix(".Nd ") {
            return manual_text(summary);
        }
        if line.starts_with('.') || line.starts_with('\'') {
            continue;
        }
        let summary = line
            .split_once("\\-")
            .or_else(|| line.split_once(" - "))
            .map(|(_, summary)| summary)?;
        return manual_text(summary);
    }
    None
}

fn manual_text(raw: &str) -> Option<String> {
    let mut text = raw.split("\\\"").next().unwrap_or_default().to_string();
    for (from, to) in [
        ("\\(em", "—"),
        ("\\(en", "–"),
        ("\\*(--", "—"),
        ("\\-", "-"),
        ("\\&", ""),
        ("\\fB", ""),
        ("\\fI", ""),
        ("\\fR", ""),
        ("\\fP", ""),
        ("\\e", "\\"),
    ] {
        text = text.replace(from, to);
    }
    let text = text.trim().trim_matches('"').trim().trim_end_matches('.');
    clean(text, 200).filter(|t| t.chars().any(char::is_alphabetic))
}

#[cfg(target_os = "macos")]
struct Info {
    category: Option<String>,
    copyright: Option<String>,
    get_info: Option<String>,
    executable: Option<String>,
}

#[cfg(target_os = "macos")]
fn read_info(path: &Path) -> Option<Info> {
    if std::fs::metadata(path).ok()?.len() > 1024 * 1024 {
        return None;
    }
    let value = plist::Value::from_file(path).ok()?;
    let dictionary = value.as_dictionary()?;
    let text = |key: &str| {
        dictionary
            .get(key)
            .and_then(plist::Value::as_string)
            .map(|s| s.chars().take(300).collect::<String>())
    };
    Some(Info {
        category: text("LSApplicationCategoryType"),
        copyright: text("NSHumanReadableCopyright"),
        get_info: text("CFBundleGetInfoString"),
        executable: text("CFBundleExecutable"),
    })
}

pub(crate) fn category_phrase(category: &str) -> Option<&'static str> {
    let kind = category.strip_prefix("public.app-category.")?;
    Some(match kind {
        "developer-tools" => "a developer tool",
        "productivity" => "a productivity app",
        "utilities" => "a utility",
        "music" => "a music app",
        "video" => "a video app",
        "photography" => "a photo app",
        "graphics-design" => "a design app",
        "social-networking" => "a messaging and social app",
        "business" => "a business app",
        "finance" => "a finance app",
        "education" => "an education app",
        "entertainment" => "an entertainment app",
        "news" => "a news app",
        "reference" => "a reference app",
        "lifestyle" => "a lifestyle app",
        "healthcare-fitness" => "a health and fitness app",
        "medical" => "a medical app",
        "sports" => "a sports app",
        "travel" => "a travel app",
        "navigation" => "a maps and navigation app",
        "weather" => "a weather app",
        "books" => "a reading app",
        kind if kind == "games" || kind.ends_with("-games") => "a game",
        _ => return None,
    })
}

/// Extract the rights holder from a copyright line: "Copyright © 2026 Doist. All rights
/// reserved." → "Doist". Placeholders and licence names are rejected.
pub(crate) fn vendor_from_copyright(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let end = lower.find("all rights").unwrap_or(raw.len());
    let (text, lower) = (&raw[..end], &lower[..end]);
    let marker = ["©", "(c)", "copyright"]
        .iter()
        .filter_map(|m| lower.rfind(m).map(|i| i + m.len()))
        .max();
    let rest = match marker {
        Some(at) => &text[at..],
        // Without a marker, accept only "2025 Example Corp" style lines.
        None if text
            .split_whitespace()
            .next()
            .is_some_and(|w| w.chars().all(|c| c.is_ascii_digit() || "-–,".contains(c))) =>
        {
            text
        }
        None => return None,
    };
    let rest = rest.split(" under ").next().unwrap_or_default();
    let words: Vec<_> = rest
        .split_whitespace()
        .filter(|w| {
            !w.eq_ignore_ascii_case("copyright")
                && *w != "©"
                && !w.eq_ignore_ascii_case("(c)")
                && !w.chars().all(|c| c.is_ascii_digit() || "-–—,.".contains(c))
        })
        .collect();
    let mut vendor = words
        .join(" ")
        .trim_matches(|c: char| c == ',' || c == '.' || c.is_whitespace())
        .to_string();
    if ["Inc", "Ltd", "Co", "Corp"]
        .iter()
        .any(|s| vendor.ends_with(&format!(" {s}")) || vendor.ends_with(&format!(",{s}")))
    {
        vendor.push('.');
    }
    let vendor = clean(&vendor, 60)?;
    (vendor.chars().any(char::is_alphabetic)
        && !vendor.contains('_')
        && !vendor.starts_with(|c: char| c.is_lowercase() && vendor.contains(char::is_uppercase))
        && vendor.split_whitespace().count() <= 6)
        .then_some(vendor)
}

fn system_location(path: &Path) -> bool {
    [
        "/System/",
        "/usr/libexec/",
        "/usr/sbin/",
        "/sbin/",
        "/Library/Apple/",
    ]
    .iter()
    .any(|root| path.starts_with(root.trim_end_matches('/')))
}

/// A helper's job from its executable name (Chromium/Electron conventions), or None.
fn helper_role(executable: &str, app: &str) -> Option<String> {
    let name = executable.to_lowercase();
    let role = if name.contains("widget") {
        format!("Widget for {app}, shown on the desktop or in Notification Center.")
    } else if name.ends_with("intents") || name.contains("intentsextension") {
        format!("Adds {app} actions to Shortcuts and Siri.")
    } else if name.contains("renderer") || name.contains("webcontent") {
        format!("Draws pages and the interface for {app}.")
    } else if name.contains("gpu") {
        format!("Handles graphics and video acceleration for {app}.")
    } else if name.contains("plugin") {
        format!("Runs plug-in or extension code for {app} in a separate process.")
    } else if name.contains("crashpad") || name.contains("crash") {
        format!("Collects crash reports for {app}.")
    } else if name.contains("updat") {
        format!("Checks for and installs {app} updates.")
    } else if name.contains("extension") {
        format!("Extension that brings {app} features into other parts of macOS.")
    } else if name.contains("login") || name.contains("launcher") {
        format!("Starts {app} or its background parts when you log in.")
    } else if name.contains("network") {
        format!("Handles network connections for {app}.")
    } else if ["helper", "agent", "service", "server", "daemon"]
        .iter()
        .any(|w| name.contains(w))
    {
        format!("Background helper that supports {app}.")
    } else {
        return None;
    };
    Some(role)
}

/// The product folder a bundle is installed in under `Library` or `Application Support`
/// ("…/Application Support/Example/Example Agent.app" → "Example"). Never a user folder.
fn owner_folder(bundle: &Path) -> Option<String> {
    let parts: Vec<String> = bundle
        .parent()?
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let anchor = parts
        .iter()
        .rposition(|p| p == "Application Support" || p == "Library")?;
    if parts[anchor + 1..].iter().any(|p| p == "Caches") {
        return None;
    }
    parts[anchor + 1..]
        .iter()
        .rev()
        .map(|p| p.trim_end_matches(".localized"))
        .find(|p| {
            !p.starts_with('.')
                && !p.contains('.')
                && !matches!(
                    *p,
                    "Contents"
                        | "Frameworks"
                        | "Resources"
                        | "Helpers"
                        | "MacOS"
                        | "Library"
                        | "LoginItems"
                        | "PlugIns"
                        | "XPCServices"
                        | "SharedSupport"
                        | "Applications"
                        | "Support"
                        | "Versions"
                        | "Current"
                )
        })
        .and_then(|p| clean(p, 40))
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// A one-sentence, everyday description and its source: `reference` (Jitterbug's curated
/// table) or `local` (read from this computer's bundle metadata, package or manual page).
pub fn describe(app: &Application) -> (String, &'static str) {
    let system = app.installation == "macOS system component" || system_location(&app.path);
    let bundle = app.path.extension().is_some_and(|e| e == "app");
    let evidence = evidence(app);
    let name = app.name.as_str();
    // A helper inside a bundle has its own job; describe that before the app biography.
    let helper = bundle
        && evidence
            .main_executable
            .as_deref()
            .is_some_and(|main| main != app.executable_name)
        && !app.executable_name.eq_ignore_ascii_case(name);
    if helper {
        if let Some(role) = helper_role(&app.executable_name, name) {
            return (role, "local");
        }
    }
    if let Some(text) = app
        .identifier
        .as_deref()
        .and_then(known_bundle)
        .map(|t| t.replace("{name}", name))
    {
        return (text, "reference");
    }
    // Claude Code's native install runs a version-named executable.
    let executable = crate::agent_sessions::agent_key(&app.executable_name, Some(&app.path))
        .unwrap_or(&app.executable_name);
    if let Some(text) = known_executable(executable, system) {
        return (text.into(), "reference");
    }
    if helper {
        return (format!("Background helper that supports {name}."), "local");
    }
    if let Some(text) = package_sentence(&evidence, name) {
        return (text, "local");
    }
    if bundle
        && app
            .path
            .components()
            .any(|c| c.as_os_str() == "org.sparkle-project.Sparkle")
    {
        return (
            "Sparkle updater that downloads and installs an update for one of your apps.".into(),
            "local",
        );
    }
    // A separately installed agent/helper bundle ("…_agent.app" under Application Support).
    if let Some(role) = bundle
        .then(|| owner_folder(&app.path).or_else(|| evidence.vendor.clone()))
        .flatten()
        .and_then(|owner| helper_role(name, &owner))
    {
        return (role, "local");
    }
    let electron = if evidence.electron {
        " It is built on Electron, so it runs like a web browser with its own helper processes."
    } else {
        ""
    };
    match (evidence.app_category, evidence.vendor.as_deref()) {
        (Some(category), Some(vendor)) => {
            return (
                format!("{name} is {category} from {vendor}.{electron}"),
                "local",
            )
        }
        (Some(category), None) => return (format!("{name} is {category}.{electron}"), "local"),
        (None, Some(vendor)) => {
            return (
                format!("{name} is an app from {vendor}.{electron}"),
                "local",
            )
        }
        _ => (),
    }
    if let Some(summary) = &evidence.manual_summary {
        let suffix = if system {
            " (a built-in macOS service)"
        } else {
            ""
        };
        return (format!("{}{suffix}.", capitalized(summary)), "local");
    }
    let parent = app
        .path
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "appex"))
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().into_owned());
    if let Some(extension) = parent {
        return (
            helper_role(&extension, "macOS").unwrap_or_else(|| {
                format!("{extension} extension that adds a feature to apps or the system.")
            }),
            "local",
        );
    }
    if bundle {
        let text = if system {
            format!("{name} is a built-in part of macOS.")
        } else if evidence.electron {
            format!("{name} is an installed desktop app built on Electron.")
        } else {
            format!("{name} is an installed app; it doesn't describe itself in its bundle.")
        };
        return (text, "local");
    }
    if let (Some(component), "framework" | "xpc") =
        (app.component.as_deref(), app.component_kind.as_str())
    {
        let owner = component
            .trim_end_matches(".framework")
            .trim_end_matches(".xpc");
        let text = if system {
            format!("Background helper in the {owner} macOS component.")
        } else {
            format!("Background helper in the {owner} component.")
        };
        return (text, "local");
    }
    let executable = if app.executable_name.is_empty() {
        name
    } else {
        &app.executable_name
    };
    let text = if system {
        format!("Built-in macOS background process {executable}; its specific job isn't recorded locally.")
    } else {
        format!("Command-line program {executable}; its job isn't recorded locally.")
    };
    (text, "local")
}

fn package_sentence(evidence: &Evidence, name: &str) -> Option<String> {
    let author = evidence
        .package_author
        .as_deref()
        .or(evidence.vendor.as_deref())
        .map(|a| format!(" from {a}"))
        .unwrap_or_default();
    let framework = if evidence.electron {
        ", built on Electron"
    } else {
        ""
    };
    let description = evidence.package_description.as_deref().filter(|d| {
        !d.eq_ignore_ascii_case(name)
            && evidence
                .package_name
                .as_deref()
                .is_none_or(|n| !d.eq_ignore_ascii_case(n))
            && d.chars().count() >= 8
    });
    if let Some(description) = description {
        let description = capitalized(description.trim_end_matches('.'));
        return Some(format!("{description}{author}{framework}."));
    }
    (evidence.package_name.is_some() && evidence.electron && !author.is_empty())
        .then(|| format!("{name} desktop app{author}{framework}."))
}

/// Curated descriptions for popular apps, keyed by bundle identifier. `{name}` is replaced
/// with the bundle name. Identity is OS metadata, not publisher/signature verification.
fn known_bundle(identifier: &str) -> Option<&'static str> {
    const BROWSER: &str =
        "{name} is a web browser; tabs, extensions and graphics each run in their own processes.";
    const TERMINAL: &str = "{name} is a terminal app that hosts command-line sessions such as shells, dev servers, builds and coding agents.";
    Some(match identifier {
        "com.jitterbug.desktop" => "Jitterbug explains what is running on your computer and helps you manage its resource use.",
        "com.apple.Safari" | "com.google.Chrome" | "com.google.Chrome.canary" | "org.mozilla.firefox"
        | "org.mozilla.firefoxdeveloperedition" | "com.microsoft.edgemac" | "company.thebrowser.Browser"
        | "company.thebrowser.dia" | "com.brave.Browser" | "com.vivaldi.Vivaldi" | "com.operasoftware.Opera"
        | "com.duckduckgo.macos.browser" | "org.torproject.torbrowser" | "app.zen-browser.zen"
        | "com.kagi.kagimacOS" | "org.chromium.Chromium" | "com.openai.atlas" => BROWSER,
        "com.apple.Terminal" | "com.googlecode.iterm2" | "com.mitchellh.ghostty" | "dev.warp.Warp-Stable"
        | "com.github.wez.wezterm" | "org.alacritty" | "net.kovidgoyal.kitty" | "co.zeit.hyper" => TERMINAL,
        "com.microsoft.VSCode" | "com.microsoft.VSCodeInsiders" | "com.vscodium" => "{name} is a code editor; extensions, language servers and terminals run as separate helper processes.",
        "com.todesktop.230313mzl4w4u92" => "Cursor is an AI code editor built on VS Code; its helpers run extensions, language servers and AI features.",
        "com.exafunction.windsurf" => "Windsurf is an AI code editor built on VS Code.",
        "dev.zed.Zed" | "dev.zed.Zed-Preview" => "Zed is a fast code editor with built-in AI and collaboration features.",
        "com.sublimetext.4" | "com.sublimetext.3" => "Sublime Text is a lightweight text and code editor.",
        "com.apple.dt.Xcode" => "Xcode is Apple's app for building, testing and debugging software for Apple devices.",
        "com.github.GitHubClient" => "GitHub Desktop lets you review, commit and sync code changes with GitHub.",
        "com.jitterbug.desktop" => "Jitterbug is this app: it explains what's running on your Mac and steps in when it slows down.",
        "com.anthropic.claudefordesktop" => "Claude is Anthropic's AI assistant app for chat, writing, coding and research.",
        "com.openai.chat" => "ChatGPT is OpenAI's AI assistant app for chat, writing and research.",
        "com.openai.codex" => "OpenAI's desktop app for ChatGPT and Codex, which can write and run code for you.",
        "com.electron.wispr-flow" => "Wispr Flow turns your speech into text in any app as you dictate.",
        "com.spotify.client" => "Spotify streams music, podcasts and audiobooks.",
        "com.tinyspeck.slackmacgap" => "Slack is a work messaging app for team channels, direct messages, huddles and shared files.",
        "com.hnc.Discord" => "Discord is a messaging app for voice, video and text chat in communities and with friends.",
        "us.zoom.xos" => "Zoom runs video meetings, calls and screen sharing.",
        "com.microsoft.teams2" | "com.microsoft.teams" => "Microsoft Teams runs work chat, meetings and calls.",
        "net.whatsapp.WhatsApp" | "desktop.WhatsApp" => "WhatsApp sends messages and makes calls from your computer.",
        "ru.keepcoder.Telegram" | "org.telegram.desktop" => "Telegram is a messaging app for chats, groups and channels.",
        "org.whispersystems.signal-desktop" => "Signal is a private, end-to-end encrypted messaging app.",
        "notion.id" => "Notion is a workspace for notes, documents, wikis and project tracking.",
        "com.linear" => "Linear tracks software issues, projects and roadmaps for product teams.",
        "com.figma.Desktop" => "Figma is a collaborative design tool for interfaces, graphics and prototypes.",
        "com.todoist.mac.Todoist" => "Todoist keeps your to-do lists, tasks and reminders.",
        "com.culturedcode.ThingsMac" => "Things organizes your to-dos, projects and daily plans.",
        "md.obsidian" => "Obsidian is a note-taking app that keeps linked Markdown notes on your computer.",
        "com.raycast.macos" => "Raycast is a launcher for opening apps, searching, running quick commands and AI actions from the keyboard.",
        "com.runningwithcrayons.Alfred" => "Alfred is a keyboard launcher for apps, files, web searches and workflows.",
        "com.1password.1password" | "com.agilebits.onepassword7" => "1Password stores your passwords, passkeys and secure notes and fills them in for you.",
        "com.bitwarden.desktop" => "Bitwarden stores your passwords and fills them in for you.",
        "dev.kdrag0n.MacVirt" => "OrbStack runs Docker containers and Linux machines in a lightweight virtual machine.",
        "com.docker.docker" => "Docker Desktop runs containers inside a Linux virtual machine on your Mac.",
        "io.tailscale.ipn.macsys" | "io.tailscale.ipn.macos" => "Tailscale connects your devices into a private network (VPN) so they can reach each other securely.",
        "com.flexibits.fantastical2.mac" => "Fantastical is a calendar app for events, reminders and scheduling.",
        "ai.ollama.ollama" | "com.electron.ollama" => "Ollama runs AI language models directly on your computer.",
        "ai.elementlabs.lmstudio" => "LM Studio downloads and runs AI language models on your computer.",
        "pl.maketheweb.cleanshotx" => "CleanShot X takes screenshots and screen recordings and lets you annotate them.",
        "com.knollsoft.Rectangle" => "Rectangle moves and resizes windows with keyboard shortcuts or by dragging.",
        "com.google.drivefs" => "Google Drive syncs your Google Drive files with this computer.",
        "com.getdropbox.dropbox" => "Dropbox syncs your Dropbox files with this computer.",
        "com.microsoft.OneDrive" => "OneDrive syncs your Microsoft OneDrive files with this computer.",
        "com.microsoft.Word" => "Microsoft Word writes and edits documents.",
        "com.microsoft.Excel" => "Microsoft Excel works with spreadsheets.",
        "com.microsoft.Powerpoint" => "Microsoft PowerPoint creates and presents slide decks.",
        "com.microsoft.Outlook" => "Microsoft Outlook handles email, calendar and contacts.",
        "org.videolan.vlc" => "VLC plays almost any video or audio file.",
        "com.obsproject.obs-studio" => "OBS Studio records and live-streams your screen, camera and audio.",
        "com.utmapp.UTM" => "UTM runs virtual machines for other operating systems.",
        "com.grammarly.ProjectLlama" => "Grammarly checks your writing as you type in other apps.",
        "com.loom.desktop" => "Loom records and shares quick screen videos.",
        "com.postmanlabs.mac" => "Postman sends and tests web API requests.",
        "com.valvesoftware.steam" => "Steam is a store and launcher for PC games.",
        "org.python.python" => "Python runs scripts and tools written in Python.",
        "com.apple.finder" => "Finder lets you browse, organize and move files on your Mac.",
        "com.apple.dock" => "The Dock shows your apps, switches between them, and draws Mission Control and Launchpad.",
        "com.apple.WindowManager" => "Manages Stage Manager and window arrangement on your desktop.",
        "com.apple.controlcenter" => "Control Center runs the menu-bar controls for Wi-Fi, Bluetooth, sound, display and Focus.",
        "com.apple.notificationcenterui" => "Shows notifications and widgets in Notification Center.",
        "com.apple.systemuiserver" => "Runs menu-bar status items such as the clock and some system menus.",
        "com.apple.loginwindow" => "Handles logging in, locking the screen and logging out.",
        "com.apple.Spotlight" => "Spotlight searches your apps, files and the web from the menu bar.",
        "com.apple.Sharing.AirDropUI" => "Shows AirDrop sharing windows and transfers.",
        "com.apple.MenuBarAgent" => "Manages the arrangement of menu-bar items.",
        "com.apple.TextInputMenuAgent" => "Runs the keyboard and input-language menu in the menu bar.",
        "com.apple.AirPlayUIAgent" => "Shows AirPlay controls for streaming to TVs and speakers.",
        "com.apple.coreservices.uiagent" => "Shows system prompts such as warnings when opening downloaded apps.",
        "com.apple.screencaptureui" => "Draws the screenshot and screen-recording tools.",
        "com.apple.ScreenTimeWidgetApplication" => "Shows Screen Time reports and limits.",
        "com.apple.universalcontrol" => "Universal Control lets one keyboard and mouse work across your Mac and iPad.",
        "com.apple.wallpaper.agent" => "Draws your desktop wallpaper, including dynamic and video wallpapers.",
        "com.apple.Siri" | "com.apple.siri.launcher" => "Siri answers voice and typed requests.",
        "com.apple.MobileSMS" => "Messages sends iMessages and text messages.",
        "com.apple.FaceTime" => "FaceTime makes video and audio calls.",
        "com.apple.mail" => "Mail sends, receives and organizes your email.",
        "com.apple.Music" => "Music plays your music library and Apple Music.",
        "com.apple.Photos" => "Photos stores, organizes and edits your photos and videos.",
        "com.apple.Notes" => "Notes keeps your notes, checklists and sketches.",
        "com.apple.iCal" => "Calendar keeps your events and schedules.",
        "com.apple.reminders" => "Reminders keeps your to-do lists.",
        "com.apple.Preview" => "Preview opens and marks up PDFs and images.",
        "com.apple.news" => "News shows stories from news publications.",
        "com.apple.findmy" => "Find My locates your devices, items and friends.",
        "com.apple.Home" => "Home controls your smart-home accessories.",
        "com.apple.podcasts" => "Podcasts plays and downloads podcast episodes.",
        "com.apple.weather" => "Weather shows forecasts and conditions.",
        "com.apple.journal" => "Journal is a private app for writing about your day.",
        "com.apple.Passwords" => "Passwords stores and fills in your saved passwords and passkeys.",
        "com.apple.shortcuts" => "Shortcuts builds and runs automations that combine steps across apps.",
        "com.apple.ActivityMonitor" => "Activity Monitor shows the apps and processes using your Mac's resources.",
        "com.apple.systempreferences" => "System Settings changes your Mac's preferences.",
        "com.apple.AppStore" => "The App Store downloads and updates apps.",
        "com.apple.TV" => "TV plays shows and movies from Apple TV and your library.",
        "com.apple.Maps" => "Maps shows directions, places and traffic.",
        "com.apple.iBooksX" => "Books is a reader for ebooks and audiobooks.",
        "com.apple.AddressBook" => "Contacts stores people's phone numbers, emails and addresses.",
        "com.apple.freeform" => "Freeform is a whiteboard for sketching and collecting ideas.",
        "com.apple.TextEdit" => "TextEdit edits plain and rich text documents.",
        "com.apple.QuickTimePlayerX" => "QuickTime Player plays and records video and audio.",
        "com.apple.TestFlight" => "TestFlight installs beta versions of apps.",
        "com.apple.Console" => "Console shows system and app logs.",
        "com.apple.ScreenContinuity" => "iPhone Mirroring shows and controls your iPhone from your Mac.",
        _ => return None,
    })
}

/// Curated descriptions for common command-line programs and built-in macOS services, keyed
/// by executable name. Service names are trusted only in system locations.
fn known_executable(executable: &str, system: bool) -> Option<&'static str> {
    // Versioned runtime names ("python3.14") share their runtime's description.
    let runtime = if executable.starts_with("python") || executable.starts_with("Python") {
        "python"
    } else {
        executable
    };
    let anywhere = match runtime {
        "node" => Some("Node.js runs JavaScript programs such as dev servers, build tools and coding-agent helpers."),
        "bun" => Some("Bun runs JavaScript and TypeScript programs, package installs and dev servers."),
        "deno" => Some("Deno runs JavaScript and TypeScript programs."),
        "python" | "python3" | "Python" => Some("Python runs scripts and tools written in Python."),
        "ruby" => Some("Ruby runs scripts and tools written in Ruby."),
        "java" => Some("Java runs programs built for the Java virtual machine."),
        "cargo" => Some("Cargo builds, tests and runs Rust projects."),
        "rustc" => Some("The Rust compiler turns Rust source code into programs."),
        "rust-analyzer" => Some("Rust language server that powers code completion and errors in your editor."),
        "clang" | "cc" | "gcc" => Some("A C/C++ compiler turning source code into programs."),
        "swift-frontend" => Some("The Swift compiler building Swift code."),
        "git" => Some("Git tracks changes to code and syncs repositories."),
        "ssh" => Some("SSH keeps a secure connection open to another computer."),
        "zsh" | "bash" | "fish" | "sh" => Some("A shell running commands in a terminal session."),
        "tmux" => Some("tmux keeps terminal sessions running in panes and windows you can detach from."),
        "claude" => Some("Claude Code is Anthropic's command-line coding agent that reads, edits and runs code in a project."),
        "codex" => Some("Codex is OpenAI's command-line coding agent that reads, edits and runs code in a project."),
        "gemini" => Some("Gemini CLI is Google's command-line coding agent."),
        "cursor-agent" => Some("Cursor Agent is Cursor's command-line coding agent."),
        "opencode" => Some("OpenCode is a command-line coding agent."),
        "aider" => Some("Aider is a command-line AI pair-programming agent."),
        "ollama" => Some("Ollama runs AI language models directly on your computer."),
        "docker" | "com.docker.backend" => Some("Docker runs and manages containers."),
        "postgres" => Some("PostgreSQL database server."),
        "redis-server" => Some("Redis in-memory database server."),
        "nginx" => Some("nginx web server."),
        "esbuild" => Some("esbuild bundles JavaScript and TypeScript for development servers and builds."),
        "tsserver" => Some("TypeScript language server that powers code completion and errors in your editor."),
        _ => None,
    };
    if anywhere.is_some() || !system {
        return anywhere;
    }
    Some(match executable {
        "WindowServer" => "Draws everything on your screen and composes app windows; its CPU use rises with animations, displays and video.",
        "launchd" => "The first process macOS starts; it launches and supervises every other service and app.",
        "mds" => "Runs the Spotlight index that lets search find your files.",
        "mds_stores" => "Stores and updates the Spotlight search index for your files.",
        "mdworker" | "mdworker_shared" | "mdbulkimport" => "Indexes files so Spotlight search can find them.",
        "corespotlightd" => "Indexes app content (messages, notes, mail) so Spotlight can find it.",
        "spotlightknowledged" => "Builds Spotlight suggestions from your activity on this Mac.",
        "sharingd" => "Powers AirDrop, Handoff, Instant Hotspot and other sharing between your devices.",
        "suggestd" => "Finds suggestions such as contacts and events in your mail and messages.",
        "duetexpertd" => "Learns your usage patterns to predict apps, contacts and suggestions.",
        "usernoted" => "Delivers notifications from apps to Notification Center.",
        "bird" => "Syncs files in iCloud Drive.",
        "cloudd" => "Syncs app data with iCloud (CloudKit).",
        "photoanalysisd" => "Analyzes your photo library for people, places and Memories, mostly while the Mac is idle.",
        "mediaanalysisd" => "Analyzes photos and videos to recognize objects, text and faces for search.",
        "photolibraryd" => "Manages access to your Photos library.",
        "cloudphotod" => "Syncs your photo library with iCloud Photos.",
        "coreaudiod" => "Plays and records all sound on your Mac.",
        "logd" => "Collects system and app log messages.",
        "trustd" => "Checks security certificates for websites and apps.",
        "nsurlsessiond" => "Downloads and uploads data in the background for apps and macOS.",
        "softwareupdated" => "Checks for and installs macOS updates.",
        "backupd" | "backupd-helper" => "Runs Time Machine backups.",
        "syspolicyd" => "Checks that apps are safe to open (Gatekeeper).",
        "opendirectoryd" => "Looks up user accounts and groups.",
        "configd" => "Manages network configuration.",
        "powerd" => "Manages sleep, power and battery settings.",
        "bluetoothd" => "Runs Bluetooth connections to keyboards, headphones and other devices.",
        "airportd" | "wifid" | "wifianalyticsd" => "Manages Wi-Fi connections.",
        "locationd" => "Provides location services to apps.",
        "cfprefsd" => "Reads and saves app and system preferences.",
        "distnoted" => "Passes notifications between apps and system services.",
        "UserEventAgent" => "Runs system plug-ins that react to events such as devices connecting.",
        "coreservicesd" | "lsd" => "Keeps track of installed apps and which app opens each kind of file.",
        "runningboardd" => "Manages which apps and services are running and how much they may use.",
        "securityd" | "secd" | "secinitd" => "Handles keychain passwords and security checks.",
        "accountsd" => "Manages your internet accounts (iCloud, mail, calendars).",
        "identityservicesd" => "Keeps iMessage and FaceTime connected to Apple's servers.",
        "imagent" => "Sends and receives iMessages for the Messages app.",
        "apsd" => "Maintains the push-notification connection to Apple's servers.",
        "rapportd" => "Lets your nearby Apple devices discover and talk to each other.",
        "remindd" => "Syncs reminders.",
        "calaccessd" => "Gives apps access to your calendars.",
        "contactsd" => "Gives apps access to your contacts.",
        "knowledge-agent" | "knowledgeconstructiond" => "Builds an on-device record of your activity to power suggestions.",
        "biomesyncd" => "Syncs on-device activity data between your Apple devices.",
        "routined" => "Learns places you visit to power location-based suggestions.",
        "fileproviderd" => "Connects cloud storage apps (iCloud Drive, Dropbox, Google Drive) to Finder.",
        "revisiond" => "Saves document versions so you can revert changes.",
        "diskarbitrationd" => "Mounts and unmounts disks and drives.",
        "fseventsd" => "Records file changes so apps and backups know what changed.",
        "hidd" => "Handles keyboard, mouse and trackpad input.",
        "coreduetd" => "Tracks device activity to schedule background work at good times.",
        "symptomsd" => "Monitors network health and data usage.",
        "analyticsd" => "Collects diagnostic and usage data if you share it with Apple.",
        "ReportCrash" => "Writes crash reports when an app quits unexpectedly.",
        "spindump" => "Records what a slow or hanging app was doing.",
        "tccd" => "Manages privacy permissions such as camera, microphone and file access.",
        "amfid" => "Verifies app code signatures before code runs.",
        "mobileassetd" => "Downloads system assets such as fonts, voices and language models.",
        "triald" => "Downloads and runs Apple feature experiments and model updates.",
        "intelligenceplatformd" => "Runs Apple Intelligence features on this Mac.",
        "siriknowledged" | "siriactionsd" | "assistantd" => "Powers Siri and its suggestions.",
        "avconferenced" => "Runs audio and video for FaceTime calls.",
        "callservicesd" => "Manages phone and FaceTime calls.",
        "nearbyd" => "Finds nearby Apple devices for features like AirDrop and Find My.",
        "universalaccessd" => "Runs accessibility features such as zoom and VoiceOver.",
        "AppleSpell" => "Checks spelling and grammar as you type.",
        "chronod" => "Updates the widgets on your desktop and in Notification Center.",
        "corebrightnessd" => "Adjusts display brightness and Night Shift.",
        "thermalmonitord" => "Monitors temperature and slows the Mac down to keep it cool.",
        "kernelmanagerd" => "Loads system extensions and drivers.",
        "installd" => "Installs apps and software packages.",
        "deleted" => "Frees disk space by removing purgeable files when needed.",
        "mediaremoted" => "Powers Now Playing media controls.",
        "akd" => "Handles Apple Account sign-in.",
        "storagekitd" => "Manages disks for Disk Utility.",
        "replayd" => "Records screen video for screen recordings and sharing.",
        "WindowManager" => "Manages Stage Manager and window arrangement on your desktop.",
        "com.apple.WebKit.WebContent" => "Draws a web page for Safari or another app that shows web content.",
        "com.apple.WebKit.Networking" => "Loads web content over the network for Safari and other apps.",
        "com.apple.WebKit.GPU" => "Handles graphics and video for web content in Safari and other apps.",
        "com.apple.appkit.xpc.openAndSavePanelService" => "Shows the Open and Save file dialogs for apps.",
        "LocalAuthenticationRemoteService" => "Shows Touch ID and password prompts for apps.",
        "ShareSheetUI" => "Shows the Share menu for apps.",
        "QuickLookUIService" | "QuickLookSatellite" => "Draws Quick Look previews of files.",
        "AutoFillPanelService" => "Shows password and contact AutoFill suggestions.",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("jitterbug-identity-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn asar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut entries = serde_json::Map::new();
        let mut data = Vec::new();
        for (name, bytes) in files {
            entries.insert(
                (*name).into(),
                serde_json::json!({"size": bytes.len(), "offset": data.len().to_string()}),
            );
            data.extend_from_slice(bytes);
        }
        let header = serde_json::json!({ "files": entries })
            .to_string()
            .into_bytes();
        let padded = header.len().div_ceil(4) * 4;
        let mut out = Vec::new();
        out.extend_from_slice(&4u32.to_le_bytes());
        out.extend_from_slice(&((padded + 8) as u32).to_le_bytes());
        out.extend_from_slice(&((padded + 4) as u32).to_le_bytes());
        out.extend_from_slice(&(header.len() as u32).to_le_bytes());
        out.extend_from_slice(&header);
        out.resize(16 + padded, 0);
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn electron_package_identifies_a_rebranded_app_without_paths() {
        let root = temp("asar");
        let bundle = root.join("Example Bot.app");
        let resources = bundle.join("Contents/Resources");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::create_dir_all(bundle.join("Contents/Frameworks/Electron Framework.framework"))
            .unwrap();
        let package = br#"{"productName":"Example Bot","description":"Example Bot desktop agent","author":{"name":"Example Labs"},"homepage":"https://example.com"}"#;
        std::fs::write(
            resources.join("app.asar"),
            asar(&[("main.js", b"console.log(1)"), ("package.json", package)]),
        )
        .unwrap();
        let read = read_asar_package(&resources.join("app.asar")).unwrap();
        assert_eq!(read["productName"], "Example Bot");
        let app = Application {
            name: "Example Bot".into(),
            executable_name: "Example Bot".into(),
            component_kind: "app".into(),
            installation: "Installed application".into(),
            identifier: Some("com.example.unlisted".into()),
            path: bundle.clone(),
            ..Default::default()
        };
        let found = evidence(&app);
        assert!(found.electron);
        assert_eq!(found.package_author.as_deref(), Some("Example Labs"));
        let (text, source) = describe(&app);
        assert_eq!(source, "local");
        assert_eq!(
            text,
            "Example Bot desktop agent from Example Labs, built on Electron."
        );
        assert!(!text.contains('/'));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_or_oversized_archives_are_ignored() {
        let root = temp("bad-asar");
        let path = root.join("app.asar");
        std::fs::write(&path, [0u8; 8]).unwrap();
        assert!(read_asar_package(&path).is_none());
        let mut bytes = asar(&[("package.json", b"{}")]);
        bytes[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(read_asar_package(&path).is_none());
        let big = vec![b' '; (PACKAGE_LIMIT + 1) as usize];
        std::fs::write(&path, asar(&[("package.json", &big)])).unwrap();
        assert!(read_asar_package(&path).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn author_strings_drop_email_and_links() {
        assert_eq!(
            author(&serde_json::json!(
                "Example Labs <dev@example.com> (https://example.com)"
            )),
            Some("Example Labs".into())
        );
        assert_eq!(author(&serde_json::json!("dev@example.com")), None);
    }

    #[test]
    fn manual_pages_in_both_macro_styles_and_crlf() {
        let mdoc = ".Dd 2020\r\n.Dt EXAMPLED 8\r\n.Sh NAME                 \\\" Section Header\r\n.Nm exampled\r\n.Nd Example Daemon that syncs widgets.\r\n.Sh SYNOPSIS\r\n";
        assert_eq!(
            parse_manual(mdoc).as_deref(),
            Some("Example Daemon that syncs widgets")
        );
        let man =
            ".TH EXAMPLE 1\n.SH \"NAME\"\nexample \\- the example \\fBtool\\fR\n.SH SYNOPSIS\n";
        assert_eq!(parse_manual(man).as_deref(), Some("the example tool"));
        assert_eq!(parse_manual(".SH NAME\n.SH SYNOPSIS\nx \\- y\n"), None);
        let root = temp("man");
        let section = root.join("man8");
        std::fs::create_dir_all(&section).unwrap();
        std::fs::write(section.join("exampled.8"), mdoc).unwrap();
        assert_eq!(
            manual_summary_in(&[section.as_path()], "exampled").as_deref(),
            Some("Example Daemon that syncs widgets")
        );
        assert!(manual_summary_in(&[section.as_path()], "../exampled").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn copyright_lines_yield_vendors_and_reject_placeholders() {
        for (line, vendor) in [
            ("Copyright © 2026 Example Doist", Some("Example Doist")),
            (
                "Copyright © 2026 Example Orbit, Inc.",
                Some("Example Orbit, Inc."),
            ),
            (
                "© Example Technologies Ltd. 2019-2026. All Rights Reserved.",
                Some("Example Technologies Ltd."),
            ),
            ("(c) 2012-2026 Example Person", Some("Example Person")),
            ("2025 Example PBC", Some("Example PBC")),
            (
                "Copyright © 2024 Example Person under GNU GPL 3.0",
                Some("Example Person"),
            ),
            ("GPL v2", None),
            ("HUMAN_READABLE_COPYRIGHT", None),
            (
                "Copyright © 2002–2025 Example Inc.\n All Rights Reserved.",
                Some("Example Inc."),
            ),
        ] {
            assert_eq!(vendor_from_copyright(line).as_deref(), vendor, "{line}");
        }
        assert_eq!(
            category_phrase("public.app-category.developer-tools"),
            Some("a developer tool")
        );
        assert_eq!(
            category_phrase("public.app-category.word-games"),
            Some("a game")
        );
        assert_eq!(category_phrase("other"), None);
    }

    #[test]
    fn last_resorts_are_useful_and_services_are_trusted_only_in_system_locations() {
        let mut app = Application {
            name: "exampletool".into(),
            executable_name: "exampletool".into(),
            component_kind: "executable".into(),
            installation: "Standalone executable".into(),
            path: "/example/bin/exampletool".into(),
            ..Default::default()
        };
        let (text, source) = describe(&app);
        assert_eq!(source, "local");
        assert!(text.contains("Command-line program exampletool"));
        app.executable_name = "mds".into();
        assert!(!describe(&app).0.contains("Spotlight"));
        app.installation = "macOS system component".into();
        assert_eq!(describe(&app).1, "reference");
        assert!(describe(&app).0.contains("Spotlight"));
        app.executable_name = "exampled".into();
        app.component = Some("ExampleKit.framework".into());
        app.component_kind = "framework".into();
        assert_eq!(
            describe(&app).0,
            "Background helper in the ExampleKit macOS component."
        );
    }

    #[test]
    fn helpers_and_widgets_describe_their_role_within_the_parent_app() {
        assert_eq!(
            helper_role("ExampleWidgetExtension", "Example").as_deref(),
            Some("Widget for Example, shown on the desktop or in Notification Center.")
        );
        assert!(helper_role("Example Helper (Renderer)", "Example")
            .unwrap()
            .starts_with("Draws pages"));
        assert!(helper_role("Example", "Example").is_none());
    }
}
