use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
};
use sysinfo::{Process, System};

#[derive(Default)]
pub struct Attribution {
    projects: HashMap<PathBuf, Option<PathBuf>>,
}

pub struct Label {
    pub id: String,
    pub label: String,
    pub category: String,
    pub description: String,
    pub evidence: Vec<String>,
    pub confidence: String,
}

pub fn category(name: &str) -> (&'static str, &'static str) {
    let n = name.to_lowercase();
    if n == "jitterbug-worker" || n == "jitterbug-worker.exe" {
        ("test", "Opt-in Jitterbug verification fixture: bounded memory allocation and a short CPU pulse.")
    } else if n.contains("ollama") || n.contains("llama") {
        (
            "model",
            "Local model runtime. Loaded model weights and inference can use memory and compute.",
        )
    } else if n.contains("rustc")
        || n.contains("cargo")
        || n.contains("clang")
        || n.contains("ninja")
    {
        (
            "build",
            "Compilation or build tooling. High CPU can be expected during a build.",
        )
    } else if n.contains("python") {
        (
            "python",
            "Python workload. Its executable alone does not identify the script or agent task.",
        )
    } else if n.contains("node") || n.contains("bun") || n.contains("deno") {
        (
            "javascript",
            "JavaScript runtime, commonly a development server, build tool, or agent helper.",
        )
    } else if n.contains("code") || n.contains("cursor") || n.contains("claude") {
        (
            "editor",
            "Editor or coding-agent application. One application may host multiple tasks.",
        )
    } else if n.contains("safari")
        || n.contains("chrome")
        || n.contains("firefox")
        || n.contains("webkit")
    {
        ("browser", "Browser or embedded web content. Helper processes may belong to different tabs or apps.")
    } else if n.contains("jitterbug") {
        (
            "jitterbug",
            "Jitterbug's own desktop application and resource monitoring.",
        )
    } else {
        ("other", "Application or operating-system process. Purpose is not established from its name alone.")
    }
}

fn project_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .take(4)
        .find(|p| {
            [
                ".git",
                "package.json",
                "Cargo.toml",
                "pyproject.toml",
                "go.mod",
            ]
            .iter()
            .any(|m| p.join(m).exists())
        })
        .map(Path::to_path_buf)
}

fn opaque_id(value: &str) -> String {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    format!("w-{:x}", h.finish())
}

pub fn application_bundle(executable: &Path) -> Option<&Path> {
    // Helpers are often nested .app bundles. Attribute them to the outer application.
    executable
        .ancestors()
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
        .last()
}

impl Attribution {
    pub fn label(&mut self, p: &Process, system: &System) -> Label {
        let name = p.name().to_string_lossy();
        let (cat, description) = category(&name);
        let project = p.cwd().and_then(|cwd| {
            if self.projects.len() > 512 {
                self.projects.clear();
            }
            self.projects
                .entry(cwd.to_path_buf())
                .or_insert_with(|| project_root(cwd))
                .clone()
        });
        if let Some(root) =
            project.filter(|_| matches!(cat, "build" | "python" | "javascript" | "test"))
        {
            let label = root
                .file_name()
                .map(|x| x.to_string_lossy().chars().take(60).collect())
                .unwrap_or_else(|| "Development project".into());
            return Label { id: opaque_id(&format!("project:{}", root.display())), label, category: "project".into(), description: "Development processes sharing a project working directory. This does not prove they belong to the same agent task.".into(), evidence: vec!["Working directory contains a project manifest or .git directory.".into(), format!("Executable category: {cat}.")], confidence: "Project-directory match".into() };
        }
        // An app bundle is stronger evidence than a shared runtime name.
        let mut ancestor = p;
        for _ in 0..8 {
            if let Some(exe) = ancestor.exe() {
                if let Some(app) = application_bundle(exe) {
                    let label: String = app
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .chars()
                        .take(60)
                        .collect();
                    let (app_category, app_description) = category(&label);
                    return Label {
                        id: opaque_id(&format!("app:{}", app.display())),
                        label,
                        category: app_category.into(),
                        description: app_description.into(),
                        evidence: vec![
                            "Executable or ancestor belongs to this application bundle.".into(),
                        ],
                        confidence: "Application ancestry".into(),
                    };
                }
            }
            let Some(parent) = ancestor.parent().and_then(|id| system.process(id)) else {
                break;
            };
            if parent.pid() == ancestor.pid() || parent.start_time() > ancestor.start_time() {
                break;
            }
            ancestor = parent;
        }
        // WebKit's XPC services can have launchd as their parent. The kernel's
        // responsibility relationship distinguishes Safari from other WebKit clients.
        // Never group all WebKit processes by name or assume the only browser owns them.
        if name.starts_with("com.apple.WebKit.") && crate::platform::same_user(p) {
            if let Some(owner) = crate::platform::responsible_pid(p.pid().as_u32())
                .and_then(|pid| system.process(sysinfo::Pid::from_u32(pid)))
                .filter(|owner| {
                    owner.start_time() <= p.start_time() && owner.user_id() == p.user_id()
                })
            {
                if let Some(app) = owner.exe().and_then(|exe| application_bundle(exe)) {
                    let label: String = app
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .chars()
                        .take(60)
                        .collect();
                    let (category, description) = category(&label);
                    return Label {
                        id: opaque_id(&format!("app:{}", app.display())), label,
                        category: category.into(), description: description.into(),
                        evidence: vec!["macOS reports this application as responsible for the WebKit helper. This does not identify a tab.".into()],
                        confidence: "macOS responsible application".into(),
                    };
                }
            }
        }
        Label {
            id: format!("p-{}-{}", p.pid(), p.start_time()),
            label: name.chars().take(60).collect(),
            category: cat.into(),
            description: description.into(),
            evidence: vec!["Executable name only; task ownership is unknown.".into()],
            confidence: "Name-based inference".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_is_not_a_task_claim() {
        assert!(category("python3").1.contains("does not identify"));
        assert_eq!(category("ollama").0, "model");
    }
    #[test]
    fn project_identifiers_do_not_contain_paths() {
        assert!(!opaque_id("/private/example/project").contains("private"));
    }
    #[test]
    fn nested_helpers_share_the_outer_application() {
        let executable = Path::new(
            "/Applications/Example.app/Contents/Frameworks/Helper.app/Contents/MacOS/helper",
        );
        assert_eq!(
            application_bundle(executable),
            Some(Path::new("/Applications/Example.app"))
        );
    }
}
