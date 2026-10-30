//! The app side of agent control: one owner-only Unix socket, one JSON request per line.
use super::MAX_LINE;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{FileTypeExt, PermissionsExt},
        io::AsRawFd,
        net::{UnixListener, UnixStream},
    },
    path::Path,
    sync::Arc,
    time::Duration,
};

/// Answers one request from the caller with the given process ID.
pub type Handler = Arc<dyn Fn(u32, &str, &Value) -> Result<Value, String> + Send + Sync>;

/// The connecting process's user and process IDs, as reported by the kernel.
fn peer(stream: &UnixStream) -> Option<(u32, u32)> {
    let fd = stream.as_raw_fd();
    #[cfg(target_os = "macos")]
    {
        let (mut uid, mut gid) = (0, 0);
        if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
            return None;
        }
        let mut pid: libc::pid_t = 0;
        let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        let read = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_LOCAL,
                libc::LOCAL_PEERPID,
                (&mut pid as *mut libc::pid_t).cast(),
                &mut len,
            )
        };
        (read == 0 && pid > 0).then_some((uid, pid as u32))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let read = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            )
        };
        (read == 0 && cred.pid > 0).then_some((cred.uid, cred.pid as u32))
    }
}

/// Bind the socket and answer requests on a background thread.
pub fn serve(path: &Path, handler: Handler) -> Result<(), String> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        // Only a stale socket from an earlier run is replaced, never another kind of file.
        if !meta.file_type().is_socket() {
            return Err("The control socket path is occupied by another file.".into());
        }
        if UnixStream::connect(path).is_ok() {
            return Err("Another Jitterbug is already serving agent control.".into());
        }
        std::fs::remove_file(path).map_err(|_| "Cannot replace a stale control socket.")?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|_| "Cannot create the app-data directory.")?;
    }
    let listener = UnixListener::bind(path).map_err(|_| "Cannot open the control socket.")?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|_| "Cannot restrict the control socket.")?;
    std::thread::Builder::new()
        .name("jitterbug-control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let handler = handler.clone();
                let _ = std::thread::Builder::new()
                    .name("jitterbug-control-request".into())
                    .spawn(move || answer(stream, &handler));
            }
        })
        .map_err(|_| "Cannot start agent control.")?;
    Ok(())
}

fn answer(mut stream: UnixStream, handler: &Handler) {
    let me = unsafe { libc::geteuid() };
    let Some((_, pid)) = peer(&stream).filter(|(uid, _)| *uid == me) else {
        return;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut line = String::new();
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let read = BufReader::new(clone.take(MAX_LINE as u64 + 1)).read_line(&mut line);
    let reply = match read {
        Ok(n) if n > 0 && n <= MAX_LINE => match serde_json::from_str::<Value>(&line) {
            Ok(request) => {
                let method = request.get("method").and_then(Value::as_str).unwrap_or("");
                let params = request.get("params").cloned().unwrap_or(json!({}));
                match handler(pid, method, &params) {
                    Ok(value) => json!({ "ok": value }),
                    Err(error) => json!({ "error": error }),
                }
            }
            Err(_) => json!({ "error": "Invalid request." }),
        },
        _ => json!({ "error": "Request missing or too large." }),
    };
    let _ = writeln!(stream, "{reply}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_with_the_callers_pid_and_an_owner_only_socket() {
        let dir = std::env::temp_dir().join(format!("jitterbug-control-{}", std::process::id()));
        let path = dir.join("control.sock");
        let handler: Handler = Arc::new(|pid, method, params| match method {
            "echo" => Ok(json!({ "pid": pid, "params": params })),
            _ => Err("Unknown method.".into()),
        });
        serve(&path, handler.clone()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        std::env::set_var("JITTERBUG_CONTROL_SOCKET", &path);
        let reply = super::super::request("echo", &json!({ "x": 1 })).unwrap();
        // The caller's PID comes from the kernel, not from the request.
        assert_eq!(reply["pid"], std::process::id());
        assert_eq!(reply["params"]["x"], 1);
        assert_eq!(
            super::super::request("nope", &json!({})).unwrap_err(),
            "Unknown method."
        );
        // A second server refuses to steal a live socket; a plain file is never replaced.
        assert!(serve(&path, handler.clone()).is_err());
        let file = dir.join("not-a-socket");
        std::fs::write(&file, b"keep").unwrap();
        assert!(serve(&file, handler).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"keep");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
