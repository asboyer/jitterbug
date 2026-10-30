//! `jitterbug-run control-mcp`: a stdio MCP server for agents. It holds no state and makes
//! no model calls; each tool call is one request to the running Jitterbug app.
use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};

const INSTRUCTIONS: &str = "Jitterbug sees which app, browser tab, or command session (such as another coding-agent session in a terminal) is using this computer's CPU and memory, and can pause a session or close a tab. Call jitterbug_overview first; call jitterbug_inspect_app for one app's tabs or sessions. When the machine is under pressure, prefer pausing the session using the most resources that is least urgent: pausing freezes it and everything it started without losing work, and it resumes when asked or after 15 minutes. Never pause your own session (marked is_your_session). Closing a tab ends what runs in it. Every action is a request the person approves in Jitterbug unless they allowed agents to pause without asking; follow it with jitterbug_request_status, and resume sessions you paused once the pressure clears. Results are measurements, not proof of what a session is doing.";

fn tool(
    name: &str,
    description: &str,
    properties: Value,
    required: &[&str],
    read_only: bool,
) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required, "additionalProperties": false},
        "annotations": {"readOnlyHint": read_only, "destructiveHint": name == "jitterbug_close_tab", "openWorldHint": false},
    })
}

pub fn tools() -> Value {
    let unit = json!({"type": "string", "description": "unit_id from jitterbug_overview or jitterbug_inspect_app"});
    let reason = json!({"type": "string", "description": "One sentence shown to the person, e.g. why this session should wait"});
    json!({"tools": [
        tool("jitterbug_overview", "Machine pressure, the busiest apps, and the command sessions inside terminals, editors, and agent apps, with what each uses. Read-only.", json!({}), &[], true),
        tool("jitterbug_inspect_app", "Everything running inside one app: its command sessions, or for a browser its tabs measured over one second (the first call may ask the person to allow the browser connection). Read-only.", json!({"workload_id": {"type": "string"}}), &["workload_id"], true),
        tool("jitterbug_pause_session", "Ask to pause one command session and everything it started. Nothing is closed and no work is lost; it uses no CPU until resumed (automatically after 15 minutes). Usually waits for the person's approval.", json!({"unit_id": unit.clone(), "reason": reason.clone()}), &["unit_id", "reason"], false),
        tool("jitterbug_resume_session", "Resume a paused session immediately.", json!({"unit_id": unit.clone()}), &["unit_id"], false),
        tool("jitterbug_close_tab", "Ask the person to close one browser tab. Whatever runs in it ends, such as a call or unsaved input. Always waits for approval.", json!({"unit_id": unit, "reason": reason}), &["unit_id", "reason"], false),
        tool("jitterbug_request_status", "Whether a pause or close request is still waiting for the person, applied, or declined, with the measured result.", json!({"request_id": {"type": "integer"}}), &["request_id"], true),
    ]})
}

/// Map a tool call to one control request.
pub fn call(name: &str, args: &Value) -> Result<Value, String> {
    let method = match name {
        "jitterbug_overview" => "overview",
        "jitterbug_inspect_app" => "inspect_app",
        "jitterbug_pause_session" => "pause_session",
        "jitterbug_resume_session" => "resume_session",
        "jitterbug_close_tab" => "close_tab",
        "jitterbug_request_status" => "request_status",
        _ => return Err("Unknown tool.".into()),
    };
    super::request(method, args)
}

pub fn serve() -> Result<i32, String> {
    let input = std::io::stdin();
    let mut reader = input.lock();
    let output = std::io::stdout();
    let mut writer = output.lock();
    loop {
        let mut line = String::new();
        let count = reader
            .by_ref()
            .take(super::MAX_LINE as u64 + 1)
            .read_line(&mut line)
            .map_err(|_| "Cannot read MCP input.")?;
        if count == 0 {
            return Ok(0);
        }
        if count > super::MAX_LINE {
            return Err("MCP request exceeds 64 KiB.".into());
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Err(_) => Some(
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Invalid JSON"}}),
            ),
            Ok(request) => respond(&request),
        };
        if let Some(response) = response {
            writeln!(writer, "{response}").map_err(|_| "Cannot write MCP response.")?;
            writer.flush().map_err(|_| "Cannot flush MCP response.")?;
        }
    }
}

/// One JSON-RPC message; notifications (no id) get no response.
pub fn respond(request: &Value) -> Option<Value> {
    let id = request.get("id")?;
    let result = match request.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "jitterbug-control", "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS,
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools()),
        "tools/call" => {
            let params = &request["params"];
            let value = call(
                params["name"].as_str().unwrap_or(""),
                params.get("arguments").unwrap_or(&json!({})),
            );
            Ok(match value {
                Ok(value) => {
                    json!({"content": [{"type": "text", "text": value.to_string()}], "isError": false})
                }
                Err(error) => {
                    json!({"content": [{"type": "text", "text": error}], "isError": true})
                }
            })
        }
        _ => Err(json!({"code": -32601, "message": "Method not found"})),
    };
    Some(match result {
        Ok(value) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
        Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_tools_answers_initialize_and_ignores_notifications() {
        let init = respond(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"})).unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "jitterbug-control");
        assert!(
            respond(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none()
        );
        let list = respond(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
        let names: Vec<_> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"jitterbug_pause_session".to_string()));
        assert!(names.contains(&"jitterbug_close_tab".to_string()));
        let unknown = respond(
            &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "rm"}}),
        )
        .unwrap();
        assert_eq!(unknown["result"]["isError"], true);
    }
}
