#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    if jitterbug_lib::queue::worker_entry() {
        return;
    }
    jitterbug_lib::run()
}
