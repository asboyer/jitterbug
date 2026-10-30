use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct WriteError {
    message: String,
    committed: bool,
}

impl WriteError {
    fn before_commit(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            committed: false,
        }
    }

    fn after_commit(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            committed: true,
        }
    }

    pub fn committed(&self) -> bool {
        self.committed
    }

    pub fn into_message(self) -> String {
        self.message
    }
}

pub fn read<T: DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| {
            "Local settings or spending ledger is invalid. Refusing to reset it automatically."
                .into()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(_) => Err("Cannot read local application state.".into()),
    }
}

pub fn write<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    write_detailed(path, value).map_err(WriteError::into_message)
}

pub fn write_detailed<T: Serialize>(path: &Path, value: &T) -> Result<(), WriteError> {
    write_with_parent_sync(path, value, sync_parent)
}

pub(crate) fn write_with_parent_sync<T, F>(
    path: &Path,
    value: &T,
    parent_sync: F,
) -> Result<(), WriteError>
where
    T: Serialize,
    F: FnOnce(&Path) -> Result<(), String>,
{
    let parent = path
        .parent()
        .ok_or_else(|| WriteError::before_commit("Missing application data directory."))?;
    fs::create_dir_all(parent)
        .map_err(|_| WriteError::before_commit("Cannot create application data directory."))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(|_| {
            WriteError::before_commit("Cannot restrict application data permissions.")
        })?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|_| WriteError::before_commit("Cannot serialize local application state."))?;
    let (tmp, mut file) = temporary_file(parent).map_err(WriteError::before_commit)?;
    if file
        .write_all(&bytes)
        .and_then(|()| file.sync_all())
        .is_err()
    {
        let _ = fs::remove_file(&tmp);
        return Err(WriteError::before_commit(
            "Cannot persist local application state.",
        ));
    }
    drop(file);
    if replace_with_retry(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
        return Err(WriteError::before_commit(
            "Cannot replace local application state.",
        ));
    }
    parent_sync(parent).map_err(WriteError::after_commit)
}

fn temporary_file(parent: &Path) -> Result<(PathBuf, File), String> {
    for _ in 0..100 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let tmp = parent.join(format!(
            ".jitterbug-state-{:08x}-{sequence:016x}.tmp",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&tmp) {
            Ok(file) => return Ok((tmp, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("Cannot write local application state.".into()),
        }
    }
    Err("Cannot allocate temporary local application state.".into())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

#[cfg(target_os = "windows")]
pub(crate) fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MOVE_FILE_FLAGS,
    };
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    let flags = MOVE_FILE_FLAGS(MOVEFILE_REPLACE_EXISTING.0 | MOVEFILE_WRITE_THROUGH.0);
    unsafe { MoveFileExW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr()), flags) }
        .map_err(|error| std::io::Error::other(error.to_string()))
}

pub(crate) fn replace_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last = None;
    for attempt in 0..8 {
        match replace(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last = Some(error);
                std::thread::sleep(std::time::Duration::from_millis(1 << attempt.min(4)));
            }
        }
    }
    Err(last.unwrap())
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<(), String> {
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "Cannot durably persist local application state.".into())
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;

    #[derive(Debug, Default, Deserialize, Serialize)]
    struct Value {
        writer: u64,
    }

    #[test]
    fn concurrent_writes_use_unique_private_temporary_files_and_leave_valid_json() {
        let directory = std::env::temp_dir().join(format!(
            "jitterbug-atomic-storage-{}-{}",
            std::process::id(),
            super::TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let path = Arc::new(directory.join("settings.json"));
        let threads = (0..8)
            .map(|writer| {
                let path = path.clone();
                std::thread::spawn(move || super::write(&path, &Value { writer }))
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap().unwrap();
        }
        let value: Value = super::read(&path).unwrap();
        assert!(value.writer < 8);
        assert!(std::fs::read_dir(&directory).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(&*path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
