use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

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
    let parent = path.parent().ok_or("Missing application data directory.")?;
    fs::create_dir_all(parent).map_err(|_| "Cannot create application data directory.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot restrict application data permissions.")?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp)
        .map_err(|_| "Cannot write local application state.")?;
    file.write_all(
        &serde_json::to_vec_pretty(value)
            .map_err(|_| "Cannot serialize local application state.")?,
    )
    .map_err(|_| "Cannot write local application state.")?;
    file.sync_all()
        .map_err(|_| "Cannot persist local application state.")?;
    fs::rename(&tmp, path).map_err(|_| "Cannot replace local application state.".into())
}
