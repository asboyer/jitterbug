#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    if jitterbug_lib::queue::worker_entry() {
        return;
    }
    // The installed desktop executable also serves local MCP and terminal queue clients.
    // Do this before starting Tauri: no window or collector for a client connection.
    if let Some(result) = jitterbug_lib::queue::manage::client_entry() {
        match result {
            Ok(code) => std::process::exit(code),
            Err(error) => {
                eprintln!("Jitterbug: {error}");
                std::process::exit(2);
            }
        }
    }
    jitterbug_lib::run()
}
