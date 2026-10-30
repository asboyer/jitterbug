//! Agent control: a private local socket served by the running app, and a stdio MCP server
//! (`jitterbug-run control-mcp`) that forwards tool calls to it. Agents see which app, tab,
//! or command session is using resources and can ask for one granular action: pause or
//! resume a session, or close a tab. Requests become approvals in Jitterbug's own popup.
//!
//! The socket lives in the private app-data directory with owner-only permissions; every
//! connection is checked against this user's ID, and the caller's PID comes from the kernel,
//! so an agent can never pause the session it runs in.
pub mod handlers;
pub mod mcp;
#[cfg(unix)]
pub mod server;

use serde_json::Value;
use std::path::PathBuf;

/// Requests are single JSON lines no larger than this.
pub const MAX_LINE: usize = 64 * 1024;
/// Answers are single JSON lines no larger than this (a browser can list 100 tabs).
pub const MAX_REPLY: usize = 1024 * 1024;

pub fn socket_path() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("JITTERBUG_CONTROL_SOCKET") {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err("JITTERBUG_CONTROL_SOCKET must be absolute.".into());
        }
        return Ok(path);
    }
    dirs::data_dir()
        .map(|p| p.join("com.jitterbug.desktop").join("control.sock"))
        .ok_or_else(|| "Cannot locate the private app-data directory.".into())
}

/// Send one request to the running app and wait for its answer.
#[cfg(unix)]
pub fn request(method: &str, params: &Value) -> Result<Value, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let path = socket_path()?;
    let mut stream = UnixStream::connect(&path).map_err(|_| {
        "Jitterbug is not running, or agent control is unavailable. Start the Jitterbug app."
            .to_string()
    })?;
    // With a socket path in a shared folder, another user could be listening there instead.
    let me = unsafe { libc::geteuid() };
    if server::peer(&stream).is_none_or(|(uid, _)| uid != me) {
        return Err("The control socket is not served by Jitterbug running as you.".into());
    }
    // A browser may wait up to a minute for the person to allow a tab connection.
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(90)))
        .map_err(|e| e.to_string())?;
    let line = serde_json::json!({ "method": method, "params": params }).to_string();
    if line.len() > MAX_LINE {
        return Err("Request too large.".into());
    }
    writeln!(stream, "{line}").map_err(|_| "Jitterbug closed the connection.")?;
    let mut reply = String::new();
    BufReader::new(std::io::Read::take(stream, MAX_REPLY as u64 + 1))
        .read_line(&mut reply)
        .map_err(|_| "Jitterbug did not answer in time.")?;
    let value: Value =
        serde_json::from_str(&reply).map_err(|_| "Invalid answer from Jitterbug.")?;
    match value.get("error").and_then(Value::as_str) {
        Some(error) => Err(error.to_string()),
        None => Ok(value.get("ok").cloned().unwrap_or(Value::Null)),
    }
}

#[cfg(not(unix))]
pub fn request(_method: &str, _params: &Value) -> Result<Value, String> {
    Err("Agent control is not available on this platform yet.".into())
}
