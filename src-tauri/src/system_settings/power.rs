//! Read-only power-profile discovery. No privileged writer or arbitrary-command interface.
use std::collections::BTreeMap;

type Profiles = BTreeMap<String, BTreeMap<String, u32>>;

fn parse_custom(text: &str) -> Result<Profiles, String> {
    let mut result = Profiles::new();
    let mut source = None;
    for line in text.lines().map(str::trim).filter(|v| !v.is_empty()) {
        if let Some(name) = line.strip_suffix(':') {
            if !["AC Power", "Battery Power", "UPS Power"].contains(&name)
                || result.insert(name.into(), BTreeMap::new()).is_some()
            {
                return Err("Unknown or duplicate power-source section.".into());
            }
            source = Some(name.to_string());
        } else {
            let profile = result
                .get_mut(source.as_deref().ok_or("Missing power-source section.")?)
                .unwrap();
            let mut fields = line.split_whitespace();
            let key = fields.next().unwrap();
            // No interpretation of undocumented powermode numbers or other settings.
            if !["sleep", "displaysleep"].contains(&key) {
                continue;
            }
            let value = fields
                .next()
                .ok_or("Missing power timer.")?
                .parse::<u32>()
                .map_err(|_| "Invalid power timer.")?;
            if fields.next().is_some() || profile.insert(key.into(), value).is_some() {
                return Err("Ambiguous power timer.".into());
            }
        }
    }
    if result.is_empty() {
        return Err("No power-source profiles reported.".into());
    }
    Ok(result)
}

/// The executable and arguments are constants; callers cannot supply commands.
fn read_custom() -> Result<Profiles, String> {
    use std::{
        io::Read,
        os::fd::AsRawFd,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut child = Command::new("/usr/bin/pmset")
        .args(["-g", "custom"])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Power profile discovery unavailable: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("Missing power profile output.")?;
    // SAFETY: owned live pipe descriptor; nonblocking reads let us enforce deadline and size.
    if unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } == -1 {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Could not bound power profile discovery.".into());
    }
    let start = Instant::now();
    let mut bytes = Vec::new();
    let result = loop {
        let mut chunk = [0; 4096];
        match stdout.read(&mut chunk) {
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => (),
            Err(e) => break Err(format!("Power profile read failed: {e}")),
        }
        if bytes.len() > 16 * 1024 || start.elapsed() > Duration::from_secs(2) {
            break Err("Power profile discovery exceeded its size/time limit.".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    break Err("macOS denied power profile discovery.".into());
                }
                let mut remainder = Vec::new();
                if stdout
                    .take(16 * 1024 + 1)
                    .read_to_end(&mut remainder)
                    .is_err()
                {
                    break Err("Incomplete power profile output.".into());
                }
                bytes.extend(remainder);
                if bytes.len() > 16 * 1024 {
                    break Err("Power profile output too large.".into());
                }
                break String::from_utf8(bytes)
                    .map_err(|_| "Invalid power profile encoding.".into())
                    .and_then(|s| parse_custom(&s));
            }
            Err(e) => break Err(format!("Power profile process failed: {e}")),
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    // Only reap/terminate this owned read-only probe, never user processes.
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

pub(super) fn annotate(items: &mut [super::Capability]) {
    let profiles = read_custom();
    for (id, key) in [("system_sleep", "sleep"), ("display_sleep", "displaysleep")] {
        let Some(item) = items.iter_mut().find(|v| v.id == id) else {
            continue;
        };
        item.included = false;
        match &profiles {
            Ok(profiles) => {
                let values: Vec<_> = profiles
                    .iter()
                    .map(|(source, values)| {
                        format!(
                            "{source}: {}",
                            values.get(key).map_or_else(
                                || "unknown/not reported".into(),
                                |v| format!("{v} min (0 = never)")
                            )
                        )
                    })
                    .collect();
                item.status = super::CapabilityStatus::ReadOnly;
                item.detail = format!("Read-only saved power-source timers: {}. Not a prediction of effective sleep: assertions, hardware, and policy can override timers. Restore requires root; no native authorized helper is installed. Change manually in System Settings. Never included in rollback.", values.join("; "));
                item.verification =
                    "pmset -g custom observation only; not a restorable baseline value".into();
            }
            Err(error) => {
                item.status = super::CapabilityStatus::Unavailable;
                item.detail = format!(
                    "Unknown: {error} No privileged write adapter; use System Settings manually."
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separates_power_sources_preserves_zero_and_never_guesses_missing_values() {
        let result = parse_custom("Battery Power:\n sleep 3\n displaysleep 0\nAC Power:\n sleep 0\n powermode 2\nUPS Power:\n displaysleep 5\n").unwrap();
        assert_eq!(result["Battery Power"]["sleep"], 3);
        assert_eq!(result["Battery Power"]["displaysleep"], 0);
        assert_eq!(result["AC Power"]["sleep"], 0);
        assert!(!result["AC Power"].contains_key("displaysleep"));
        assert_eq!(result["UPS Power"]["displaysleep"], 5);
        assert!(!result["AC Power"].contains_key("powermode"));
    }
    #[test]
    fn rejects_ambiguous_or_malformed_profiles() {
        for text in [
            "",
            "sleep 3",
            "Mystery:\n sleep 3",
            "AC Power:\n sleep -1",
            "AC Power:\n sleep 2 extra",
            "AC Power:\n sleep 1\n sleep 2",
            "AC Power:\n sleep 1\nAC Power:\n sleep 2",
        ] {
            assert!(parse_custom(text).is_err(), "{text}");
        }
    }
}
