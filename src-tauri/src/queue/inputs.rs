//! A bounded source fingerprint, never a copy of source code. Ignored files, external
//! dependencies, environment variables and files outside this Git worktree are not covered.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    pub revision: Option<String>,
    pub fingerprint: Option<String>,
}

pub fn capture(path: &Path) -> Result<Inputs, String> {
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
    };
    let head = git(&["rev-parse", "HEAD"]);
    let Ok(head) = head else {
        return Ok(Inputs::default());
    };
    if !head.status.success() {
        return Ok(Inputs::default());
    }
    let root = git(&["rev-parse", "--show-toplevel"]).map_err(|_| "Cannot inspect Git root.")?;
    let root = std::path::PathBuf::from(
        String::from_utf8(root.stdout)
            .map_err(|_| "Git root must be UTF-8.")?
            .trim(),
    );
    let listing = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "Cannot list validation inputs.")?;
    if !listing.status.success() || listing.stdout.len() > 2 * 1024 * 1024 {
        return Err("Cannot fingerprint this worktree within the input limit.".into());
    }
    let mut hash = Sha256::new();
    hash.update(&head.stdout);
    let mut total = 0u64;
    for name in listing.stdout.split(|c| *c == 0).filter(|s| !s.is_empty()) {
        let name =
            std::str::from_utf8(name).map_err(|_| "Validation input paths must be UTF-8.")?;
        hash.update(name.as_bytes());
        hash.update([0]);
        let file = root.join(name);
        let metadata = match fs::symlink_metadata(&file) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                hash.update(b"deleted");
                continue;
            }
            Err(_) => return Err("Cannot inspect a validation input.".into()),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Source fingerprints do not support symlinks or submodules yet.".into());
        }
        total = total.saturating_add(metadata.len());
        if total > 64 * 1024 * 1024 {
            return Err("Source inputs exceed the 64 MiB fingerprint limit.".into());
        }
        hash.update(metadata.len().to_le_bytes());
        let mut bytes = Vec::new();
        fs::File::open(&file)
            .map_err(|_| "Cannot open a validation input.")?
            .take(metadata.len() + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot fingerprint a validation input.")?;
        if bytes.len() as u64 != metadata.len() {
            return Err("Inputs changed while fingerprinting; retry.".into());
        }
        hash.update(bytes);
    }
    Ok(Inputs {
        revision: Some(String::from_utf8_lossy(&head.stdout).trim().into()),
        fingerprint: Some(format!("{:x}", hash.finalize())),
    })
}
