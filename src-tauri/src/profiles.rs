//! Reusable app descriptions. Never cache a live diagnosis, a path, a PID, or a launch claim.
use crate::{
    applications::Application,
    types::{now, Workload},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    hash::{DefaultHasher, Hash, Hasher},
    path::Path,
};

const TTL: u64 = 30 * 24 * 60 * 60;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub text: String,
    pub source: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub generated_at: Option<u64>,
    pub cached: bool,
}

pub fn reference(app: Option<&Application>, workload: &Workload) -> Profile {
    // Bundle identity is OS evidence, not publisher/signature verification.
    let known = app.and_then(|app| match app.identifier.as_deref()? {
        "com.jitterbug.desktop" => Some("Jitterbug monitors resource use and explains which apps and background tasks are active. It collects system readings locally and uses configured models for explanations and investigations."),
        "com.spotify.client" => Some("Spotify plays music and podcasts. Its helpers draw the interface, play media and handle networking; several processes are normal."),
        "com.apple.Safari" | "com.google.Chrome" | "org.mozilla.firefox" | "com.microsoft.edgemac" | "company.thebrowser.Browser" => Some("This browser runs tabs, extensions and graphics in separate processes. Those helpers belong to the same app; the process name alone does not identify a specific tab."),
        "com.microsoft.VSCode" | "com.todesktop.230313mzl4w4u92" => Some("This code editor runs the interface, extensions, code analysis and terminals. Helpers can continue working while a project is open."),
        "com.apple.Terminal" | "com.googlecode.iterm2" | "com.mitchellh.ghostty" => Some("This terminal hosts command-line sessions. Commands beneath it can run servers, builds or coding agents; their parent chain and working folder help identify the source."),
        "com.apple.finder" => Some("Finder is the macOS file browser and desktop. It handles folder windows, file operations and desktop items."),
        "com.apple.WindowManager" | "com.apple.dock" => Some("This macOS component manages windows or the Dock. It runs as part of the desktop interface."),
        "com.tinyspeck.slackmacgap" | "com.hnc.Discord" => Some("This messaging app uses helpers for conversations, calls, updates and shared media. Several processes can belong to the same application."),
        "ai.ollama.ollama" => Some("Ollama runs language models on this computer. Loaded models can use substantial memory; an idle server does not necessarily have a model loaded."),
        _ => None,
    });
    Profile {
        text: known.unwrap_or(&workload.description).into(),
        source: if known.is_some() {
            "reference"
        } else {
            "inference"
        }
        .into(),
        provider: None,
        model: None,
        generated_at: None,
        cached: known.is_some(),
    }
}

pub fn key(app: &Application) -> String {
    let mut hash = DefaultHasher::new();
    // Schema, install identity, and version prevent cross-app/stale-version reuse.
    let modified = std::fs::metadata(&app.path).and_then(|m| m.modified()).ok();
    (
        3u32,
        &app.path,
        &app.executable_name,
        &app.identifier,
        &app.version,
        modified,
    )
        .hash(&mut hash);
    format!("app-{:x}", hash.finish())
}

/// Only identity metadata crosses the provider boundary. Paths, process arguments,
/// working directories, parent chains, and live diagnoses are never cached here.
pub fn model_payload(app: &Application) -> serde_json::Value {
    let purpose = reference(Some(app), &Workload::default());
    serde_json::json!({
        "known_purpose": (purpose.source == "reference").then_some(purpose.text),
        "name": app.name,
        "executable": app.executable_name,
        "component": app.component,
        "component_kind": app.component_kind,
        "bundle_identifier": app.identifier,
        "installation": app.installation,
        "operating_system": std::env::consts::OS,
    })
}

#[derive(Default, Serialize, Deserialize)]
pub struct Cache {
    entries: VecDeque<(String, Profile)>,
}
impl Cache {
    pub fn read(directory: &Path) -> Self {
        let path = directory.join("app-profiles.json");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() <= 512 * 1024) {
            if let Ok(mut cache) = crate::storage::read::<Self>(&path) {
                cache.prune();
                return cache;
            }
        }
        Self::default()
    }
    fn prune(&mut self) {
        self.entries.retain(|(_, p)| {
            p.source == "model"
                && p.text.len() <= 4000
                && p.generated_at
                    .is_some_and(|t| t <= now() && t.saturating_add(TTL) > now())
        });
        while self.entries.len() > 128 {
            self.entries.pop_front();
        }
    }
    pub fn get(&mut self, key: &str) -> Option<Profile> {
        self.prune();
        let index = self.entries.iter().position(|(id, _)| id == key)?;
        let (id, mut profile) = self.entries.remove(index)?;
        profile.cached = true;
        self.entries.push_back((id, profile.clone()));
        Some(profile)
    }
    pub fn insert_memory(&mut self, key: String, profile: Profile) {
        self.entries.retain(|(id, _)| id != &key);
        self.entries.push_back((key, profile));
        self.prune();
    }
    pub fn insert(
        &mut self,
        key: String,
        profile: Profile,
        directory: &Path,
    ) -> Result<(), String> {
        self.entries.retain(|(id, _)| id != &key);
        self.entries.push_back((key, profile));
        self.prune();
        crate::storage::write(&directory.join("app-profiles.json"), self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_identity_payload_excludes_paths_and_cache_keys_separate_executables() {
        let mut app =
            crate::applications::identify(Path::new("/example/private-project/bin/worker"));
        let payload = model_payload(&app).to_string();
        assert!(payload.contains("worker"));
        assert!(!payload.contains("private-project"));
        assert!(!payload.contains("/example"));
        let first = key(&app);
        app.path = "/example/another-install/bin/worker".into();
        assert_ne!(key(&app), first);
        app.path = "/example/private-project/bin/worker".into();
        app.executable_name = "different-helper".into();
        assert_ne!(key(&app), first);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn framework_daemons_have_component_context_without_trusting_their_name() {
        let installed = crate::applications::identify(Path::new(
            "/System/Library/PrivateFrameworks/ExampleAnalysis.framework/Versions/A/exampled",
        ));
        assert_eq!(installed.name, "exampled");
        assert_eq!(
            installed.component.as_deref(),
            Some("ExampleAnalysis.framework")
        );
        assert_eq!(installed.component_kind, "framework");
        assert_eq!(installed.installation, "macOS system component");
        let impostor = crate::applications::identify(Path::new("/example/downloads/exampled"));
        assert_eq!(impostor.installation, "Standalone executable");
        assert_ne!(key(&installed), key(&impostor));
        assert!(!model_payload(&installed).to_string().contains("/System/"));
    }

    #[test]
    fn version_changes_invalidate_app_profiles_and_references_need_bundle_identity() {
        let mut app = Application {
            name: "Spotify".into(),
            identifier: Some("com.spotify.client".into()),
            version: Some("1".into()),
            path: "/Applications/Example.app".into(),
            identity_source: "test".into(),
            ..Default::default()
        };
        assert_eq!(
            reference(Some(&app), &Workload::default()).source,
            "reference"
        );
        let old = key(&app);
        app.version = Some("2".into());
        assert_ne!(key(&app), old);
        app.identifier = None;
        assert_eq!(
            reference(Some(&app), &Workload::default()).source,
            "inference"
        );
    }
    #[test]
    fn cache_is_bounded_and_expired_descriptions_are_discarded() {
        let mut cache = Cache::default();
        let profile = Profile {
            text: "General purpose only.".into(),
            source: "model".into(),
            provider: Some("meta".into()),
            model: Some("test".into()),
            generated_at: Some(now()),
            cached: false,
        };
        for id in 0..200 {
            cache.entries.push_back((id.to_string(), profile.clone()));
        }
        assert!(cache.get("199").unwrap().cached);
        assert_eq!(cache.entries.len(), 128);
        assert!(cache.get("0").is_none());
        cache.entries[0].1.generated_at = Some(now() - TTL);
        cache.prune();
        assert_eq!(cache.entries.len(), 127);
    }
    #[test]
    fn cache_survives_restart_and_corruption_is_only_a_cache_miss() {
        let directory =
            std::env::temp_dir().join(format!("jitterbug-profile-test-{}", std::process::id()));
        let mut cache = Cache::default();
        let profile = Profile {
            text: "Synthetic app description.".into(),
            source: "model".into(),
            provider: Some("meta".into()),
            model: Some("test".into()),
            generated_at: Some(now()),
            cached: false,
        };
        cache
            .insert("test-app".into(), profile, &directory)
            .unwrap();
        let reloaded = Cache::read(&directory).get("test-app").unwrap();
        assert!(reloaded.cached);
        assert_eq!(reloaded.text, "Synthetic app description.");
        let path = directory.join("app-profiles.json");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::write(&path, "invalid cache").unwrap();
        assert!(Cache::read(&directory).get("test-app").is_none());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
