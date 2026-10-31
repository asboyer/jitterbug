//! Development Lab: one window showing the Inside panel, the action popup, and an agent console
//! that speaks MCP through the real control socket, so actions and wording can be checked on
//! live data. Debug builds open it at launch; release builds never do.
use serde_json::Value;
use tauri::Manager;

pub const LABEL: &str = "lab";

pub fn open(app: &tauri::AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(LABEL) {
        window.show()?;
        return window.set_focus();
    }
    tauri::WebviewWindowBuilder::new(
        app,
        LABEL,
        tauri::WebviewUrl::App("index.html?view=lab".into()),
    )
    .title("Jitterbug Lab")
    .inner_size(1480.0, 940.0)
    .min_inner_size(1100.0, 700.0)
    .build()?;
    Ok(())
}

/// One JSON-RPC message, answered by the same MCP server an agent runs. Tool calls go over the
/// real control socket, so agent control settings and approvals apply as they would for an agent.
#[tauri::command]
pub async fn lab_mcp(request: Value) -> Result<Option<Value>, String> {
    if !cfg!(debug_assertions) {
        return Err("The Lab is only available in development builds.".into());
    }
    tauri::async_runtime::spawn_blocking(move || crate::control::mcp::respond(&request))
        .await
        .map_err(|e| e.to_string())
}
