//! Fixed allowlisted power-profile capture/restore; authorization must be provided externally.
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
    if let Err(error) = collect() {
        for item in items
            .iter_mut()
            .filter(|v| ["system_sleep", "display_sleep"].contains(&v.id.as_str()))
        {
            item.status = super::CapabilityStatus::Unavailable;
            item.detail = format!("Power profile capture unavailable: {error}. The allowlisted automatic timer executor requires a successfully captured value and external authorization.");
        }
    }
}

pub(super) fn collect() -> Result<BTreeMap<u16, super::HardwareValue>, String> {
    let profiles = read_custom()?;
    let mut values = BTreeMap::new();
    for (i, source) in ["AC Power", "Battery Power", "UPS Power"]
        .iter()
        .enumerate()
    {
        for (j, key) in ["sleep", "displaysleep"].iter().enumerate() {
            if let Some(minutes) = profiles.get(*source).and_then(|v| v.get(*key)) {
                let slot = 100 + i as u16 * 2 + j as u16;
                let value = super::HardwareValue::Power {
                    source: source.to_string(),
                    key: key.to_string(),
                    minutes: *minutes,
                };
                value.validate(slot)?;
                values.insert(slot, value);
            }
        }
    }
    Ok(values)
}

fn write_with(
    slot: u16,
    expected: &super::HardwareValue,
    target: &super::HardwareValue,
    read: impl FnOnce() -> Result<super::HardwareValue, String>,
    run: impl FnOnce([String; 3]) -> Result<(), String>,
) -> Result<(), String> {
    target.validate(slot)?;
    if !expected.same_identity(target) || read()? != *expected {
        return Err("Power source or saved timer changed before write.".into());
    }
    let super::HardwareValue::Power {
        source,
        key,
        minutes,
    } = target
    else {
        return Err("Wrong power target type".into());
    };
    let flag = match source.as_str() {
        "AC Power" => "-c",
        "Battery Power" => "-b",
        "UPS Power" => "-u",
        _ => return Err("Unknown power source".into()),
    };
    run([flag.into(), key.clone(), minutes.to_string()])
}

pub(super) fn write(
    slot: u16,
    expected: &super::HardwareValue,
    target: &super::HardwareValue,
) -> Result<(), String> {
    write_with(
        slot,
        expected,
        target,
        || {
            collect()?
                .remove(&slot)
                .ok_or("Power timer is unavailable".into())
        },
        |args| {
            use std::{
                process::{Command, Stdio},
                time::{Duration, Instant},
            };
            // Closed typed arguments, fixed executable, no shell, prompt, privilege escalation, or helper installation.
            let mut child = Command::new("/usr/bin/pmset")
                .args(args)
                .env_clear()
                .env("LC_ALL", "C")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("Cannot start power timer executor: {e}"))?;
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => return Err(format!("pmset rejected the saved timer (exit {status}). External authorization is required to modify system power profiles, or hardware/policy rejected it. Arrange authorization outside Jitterbug and re-preview. No grant was requested; do not run the GUI as root.")),
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
                result => {
                    let _ = child.kill(); let _ = child.wait();
                    return Err(format!("Power timer executor timed out or failed ({result:?}); state may have changed. Recheck before retrying."));
                }
            }
            }
        },
    )?;
    if collect()?.get(&slot) != Some(target) {
        return Err("Saved power timer readback mismatch".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writer_is_allowlisted_rechecks_and_propagates_authorization_failure() {
        use super::super::HardwareValue;
        let old = HardwareValue::Power {
            source: "Battery Power".into(),
            key: "displaysleep".into(),
            minutes: 5,
        };
        let target = HardwareValue::Power {
            source: "Battery Power".into(),
            key: "displaysleep".into(),
            minutes: 0,
        };
        let mut called = false;
        let error = write_with(
            103,
            &old,
            &target,
            || Ok(old.clone()),
            |args| {
                called = true;
                assert_eq!(args, ["-b", "displaysleep", "0"]);
                Err("External authorization required".into())
            },
        )
        .unwrap_err();
        assert!(called && error.contains("authorization"));
        assert!(write_with(
            103,
            &old,
            &target,
            || Ok(target.clone()),
            |_| panic!("stale")
        )
        .is_err());
        let invalid = HardwareValue::Power {
            source: "Battery Power".into(),
            key: "sleep; touch /tmp/x".into(),
            minutes: 0,
        };
        assert!(write_with(
            103,
            &old,
            &invalid,
            || Ok(old.clone()),
            |_| panic!("injection")
        )
        .is_err());
    }
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
