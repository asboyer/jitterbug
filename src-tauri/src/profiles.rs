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
        "com.spotify.client" => Some("Spotify plays music and podcasts. Its main process coordinates the app; helper processes can render the interface, decode media, and handle networking. Multiple helpers are normal. This describes the app's purpose; it does not establish why a particular process is busy right now."),
        "com.apple.Safari" | "com.google.Chrome" | "org.mozilla.firefox" | "com.microsoft.edgemac" | "company.thebrowser.Browser" => Some("This is a web browser. Separate processes can run tabs, websites, extensions, graphics, and networking. A helper's name alone cannot identify the tab responsible. Resource history and a fresh investigation are needed to explain current activity."),
        "com.microsoft.VSCode" | "com.todesktop.230313mzl4w4u92" => Some("This is a code editor. Its helpers can run extensions, language servers, searches, terminals, and the interface. A child runtime may be doing project work, but process ancestry alone does not identify the specific coding task."),
        "com.apple.Terminal" | "com.googlecode.iterm2" | "com.mitchellh.ghostty" => Some("This is a terminal application. It hosts shells that can start commands, development servers, builds, and coding agents. Follow a child process's parent chain and working directory to see which shell or project it is associated with."),
        "com.apple.finder" => Some("Finder is the macOS file browser and desktop. It handles folder windows, file operations, and desktop items. Its presence is expected; resource use by itself does not establish a fault."),
        "com.apple.WindowManager" | "com.apple.dock" => Some("This is part of the macOS desktop interface. It manages windows or the Dock and works with other system services. Treat it as a system component rather than an ordinary background job."),
        "com.tinyspeck.slackmacgap" | "com.hnc.Discord" => Some("This is a messaging and collaboration app. Helpers can render conversations, receive updates, and handle calls or shared media. Several processes can belong to the same application; a fresh investigation is needed to explain unusual resource use."),
        "ai.ollama.ollama" => Some("Ollama runs local language models. Model weights, context, and inference can use substantial memory and compute. A running server does not by itself prove a model is loaded; check the local model runtime before drawing that conclusion."),
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
    (1u32, &app.path, &app.identifier, &app.version, modified).hash(&mut hash);
    format!("app-{:x}", hash.finish())
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
    fn version_changes_invalidate_app_profiles_and_references_need_bundle_identity() {
        let mut app = Application {
            name: "Spotify".into(),
            identifier: Some("com.spotify.client".into()),
            version: Some("1".into()),
            path: "/Applications/Example.app".into(),
            identity_source: "test".into(),
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
