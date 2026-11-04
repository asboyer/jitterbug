//! Check OS app artwork locally; no telemetry, credentials or model calls.
#[cfg(target_os = "macos")]
fn main() {
    let started = std::time::Instant::now();
    // Presence only; this must not read a secret or show a Keychain authorization prompt.
    let configured =
        jitterbug_lib::settings::credential_present(&jitterbug_lib::settings::Provider::Meta);
    println!(
        "Credential presence checked: {configured} ({} ms)",
        started.elapsed().as_millis()
    );
    let path = std::path::Path::new("/System/Library/CoreServices/Finder.app");
    let icon = jitterbug_lib::applications::Icons::default().get(path);
    println!(
        "OS icon available: {}, bytes: {}",
        icon.is_some(),
        icon.as_ref().map_or(0, |s| s.len())
    );
    assert!(icon.is_some());
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("This smoke check uses macOS application artwork.");
}
