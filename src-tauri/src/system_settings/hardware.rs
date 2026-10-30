//! Typed, bounded restore targets. Slot numbers are storage keys, never native object IDs.
use super::*;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioFormat {
    pub sample_rate: f64,
    pub format_id: u32,
    pub flags: u32,
    pub bytes_per_packet: u32,
    pub frames_per_packet: u32,
    pub bytes_per_frame: u32,
    pub channels: u32,
    pub bits_per_channel: u32,
    pub reserved: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioStream {
    pub starting_channel: u32,
    pub terminal_type: u32,
    pub format: AudioFormat,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayMode {
    pub id: i32,
    pub width: usize,
    pub height: usize,
    pub pixel_width: usize,
    pub pixel_height: usize,
    pub refresh_hz: f64,
    pub flags: u32,
    pub desktop_usable: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayState {
    pub uuid: String,
    pub x: f64,
    pub y: f64,
    pub rotation: f64,
    pub main: bool,
    pub active: bool,
    pub mirror: Option<String>,
    pub mode: DisplayMode,
}
impl DisplayState {
    pub fn same_topology(&self, other: &Self) -> bool {
        self.uuid == other.uuid
            && self.x == other.x
            && self.y == other.y
            && self.rotation == other.rotation
            && self.main == other.main
            && self.active == other.active
            && self.mirror == other.mirror
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HardwareValue {
    Displays {
        displays: Vec<DisplayState>,
    },
    Audio {
        uid: String,
        rate: Option<f64>,
        streams: Vec<AudioStream>,
    },
    Power {
        source: String,
        key: String,
        minutes: u32,
    },
}
/// An inspected native handle is never rebound after the journal boundary. No retry of writes.
#[cfg(any(target_os = "macos", test))]
pub(super) fn execute_verified<T>(
    before: &HardwareValue,
    target: &HardwareValue,
    inspect: impl FnOnce() -> Result<(T, HardwareValue), String>,
    set: impl FnOnce(T) -> Result<(), String>,
    mut read: impl FnMut() -> Result<HardwareValue, String>,
    mut wait: impl FnMut(),
    attempts: usize,
) -> Result<(), String> {
    let (handle, fresh) = inspect()?;
    if fresh != *before || !before.same_identity(target) {
        return Err("Hardware identity or value changed immediately before write".into());
    }
    set(handle)?;
    for _ in 0..attempts.min(40) {
        let actual = read()?;
        if !actual.same_identity(target) {
            return Err("Hardware identity changed during readback; no further writes".into());
        }
        if actual == *target {
            return Ok(());
        }
        wait();
    }
    Err("Bounded native readback did not match saved target; state may have changed".into())
}

impl HardwareValue {
    pub fn group(&self) -> &'static str {
        match self {
            Self::Displays { .. } => "Displays",
            Self::Audio { .. } => "Audio hardware",
            Self::Power { .. } => "Power profiles",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Displays { .. } => "Display resolution and refresh modes",
            Self::Audio { .. } => "Output nominal rate and physical stream formats",
            Self::Power { .. } => "Saved power-source timer",
        }
    }
    pub fn verification(&self) -> &'static str {
        match self {
            Self::Displays {..} => "Current display modes restored by full descriptor readback; reboot persistence not verified. Permanent configuration requested; macOS can fall back to login-session scope. Screens may blank or windows move.",
            Self::Audio {..} => "Live CoreAudio rate/full ASBD readback; reboot/reconnect persistence not verified. Changing formats can interrupt audio, stop playback/recording, or require audio applications to reopen their device.",
            Self::Power {..} => "Saved pmset profile readback; effective sleep not verified. Requires externally provided authorization.",
        }
    }
    pub fn description(&self) -> String {
        match self {
            Self::Displays { displays } => displays
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    let m = &d.mode;
                    let rate = if m.refresh_hz == 0.0 { "unknown/variable refresh (0 reported)".into() } else { format!("{} Hz", m.refresh_hz) };
                    format!("Display {}: {}×{} points, {}×{} pixels, {rate}, mode ID {}, flags {:#x}, desktop usable {}", i + 1, m.width, m.height, m.pixel_width, m.pixel_height, m.id, m.flags, m.desktop_usable)
                })
                .collect::<Vec<_>>()
                .join("; "),
            Self::Audio { rate, streams, .. } => {
                let rate = rate.map_or_else(|| "not captured (not settable)".into(), |r| format!("{r} Hz"));
                let formats = streams.iter().map(|s| {
                    let f = s.format;
                    format!("output channel {} / terminal {:#x}: ASBD rate={} Hz, format={:#x}, flags={:#x}, bytes/packet={}, frames/packet={}, bytes/frame={}, channels={}, bits/channel={}, reserved={}", s.starting_channel, s.terminal_type, f.sample_rate, f.format_id, f.flags, f.bytes_per_packet, f.frames_per_packet, f.bytes_per_frame, f.channels, f.bits_per_channel, f.reserved)
                }).collect::<Vec<_>>().join("; ");
                format!("Nominal rate: {rate}; captured physical output formats: {formats}")
            },
            Self::Power {
                source,
                key,
                minutes,
            } => format!("{source} / {key}: {minutes} min (0 = never)"),
        }
    }
    pub fn validate(&self, slot: u16) -> Result<(), String> {
        match self {
            Self::Displays { displays } => {
                let mut ids = std::collections::BTreeSet::new();
                if slot != 1 || displays.is_empty() || displays.len() > 31 {
                    return Err("Invalid display inventory".into());
                }
                for d in displays {
                    if d.uuid.is_empty()
                        || d.uuid.len() > 128
                        || !ids.insert(&d.uuid)
                        || !d.x.is_finite()
                        || !d.y.is_finite()
                        || !d.rotation.is_finite()
                        || d.mode.width == 0
                        || d.mode.height == 0
                        || d.mode.pixel_width == 0
                        || d.mode.pixel_height == 0
                        || !d.mode.refresh_hz.is_finite()
                        || d.mode.refresh_hz < 0.0
                    {
                        return Err("Invalid full display descriptor/topology".into());
                    }
                }
                if displays
                    .iter()
                    .any(|d| d.mirror.as_ref().is_some_and(|m| !ids.contains(m)))
                {
                    return Err("Missing mirror display".into());
                }
            }
            Self::Audio { uid, rate, streams } => {
                if slot != 0
                    || uid.is_empty()
                    || uid.len() > 1024
                    || streams.len() > 16
                    || (rate.is_none() && streams.is_empty())
                    || rate.is_some_and(|r| !r.is_finite() || r <= 0.0)
                {
                    return Err("Invalid audio format target".into());
                }
                let mut channels = std::collections::BTreeSet::new();
                for stream in streams {
                    if stream.starting_channel == 0
                        || !channels.insert(stream.starting_channel)
                        || stream.format.channels == 0
                        || !stream.format.sample_rate.is_finite()
                        || stream.format.sample_rate <= 0.0
                    {
                        return Err("Invalid or ambiguous output stream".into());
                    }
                }
            }
            Self::Power {
                source,
                key,
                minutes,
            } => {
                let source_index = ["AC Power", "Battery Power", "UPS Power"]
                    .iter()
                    .position(|s| s == source)
                    .ok_or("Unknown power source")?;
                let key_index = ["sleep", "displaysleep"]
                    .iter()
                    .position(|s| s == key)
                    .ok_or("Unknown power key")?;
                if slot != 100 + source_index as u16 * 2 + key_index as u16
                    || *minutes > i32::MAX as u32
                {
                    return Err("Invalid power timer or slot".into());
                }
            }
        }
        Ok(())
    }
    pub fn same_identity(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Displays { displays: a }, Self::Displays { displays: b }) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.same_topology(b))
            }
            (
                Self::Audio {
                    uid: a,
                    rate: ar,
                    streams: as_,
                },
                Self::Audio {
                    uid: b,
                    rate: br,
                    streams: bs,
                },
            ) => {
                a == b
                    && ar.is_some() == br.is_some()
                    && as_.len() == bs.len()
                    && as_.iter().zip(bs).all(|(a, b)| {
                        a.starting_channel == b.starting_channel
                            && a.terminal_type == b.terminal_type
                    })
            }
            (
                Self::Power {
                    source: a, key: b, ..
                },
                Self::Power {
                    source: c, key: d, ..
                },
            ) => a == c && b == d,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn power(minutes: u32) -> HardwareValue {
        HardwareValue::Power {
            source: "AC Power".into(),
            key: "sleep".into(),
            minutes,
        }
    }
    #[test]
    fn descriptions_label_unknown_refresh_and_omit_hardware_identity() {
        let value = HardwareValue::Displays {
            displays: vec![DisplayState {
                uuid: "private-uuid".into(),
                x: 0.0,
                y: 0.0,
                rotation: 0.0,
                main: true,
                active: true,
                mirror: None,
                mode: DisplayMode {
                    id: 1,
                    width: 100,
                    height: 100,
                    pixel_width: 200,
                    pixel_height: 200,
                    refresh_hz: 0.0,
                    flags: 3,
                    desktop_usable: true,
                },
            }],
        };
        let text = value.description();
        assert!(text.contains("unknown/variable"));
        assert!(text.contains("100×100 points") && text.contains("200×200 pixels"));
        assert!(!text.contains("private-uuid"));
        let value = HardwareValue::Audio {
            uid: "private-uid".into(),
            rate: None,
            streams: vec![],
        };
        assert!(value.description().contains("not captured"));
        assert!(!value.description().contains("private-uid"));
    }
    #[test]
    fn executor_uses_inspected_handle_and_waits_for_async_readback() {
        let before = power(1);
        let target = power(2);
        let mut reads = 0;
        execute_verified(
            &before,
            &target,
            || Ok((42_u32, before.clone())),
            |handle| {
                assert_eq!(handle, 42);
                Ok(())
            },
            || {
                reads += 1;
                Ok(if reads < 3 {
                    before.clone()
                } else {
                    target.clone()
                })
            },
            || {},
            4,
        )
        .unwrap();
        assert_eq!(reads, 3);
        assert!(execute_verified(
            &before,
            &target,
            || Ok((42, target.clone())),
            |_| panic!("drift"),
            || unreachable!(),
            || {},
            4
        )
        .is_err());
        assert!(execute_verified(
            &before,
            &target,
            || Ok((42, before.clone())),
            |_| Err("busy".into()),
            || panic!("setter denied"),
            || {},
            4
        )
        .unwrap_err()
        .contains("busy"));
        assert!(execute_verified(
            &before,
            &target,
            || Ok((42, before.clone())),
            |_| Ok(()),
            || Ok(before.clone()),
            || {},
            4
        )
        .unwrap_err()
        .contains("readback"));
        assert!(execute_verified(
            &before,
            &target,
            || Ok((42, before.clone())),
            |_| Ok(()),
            || Err("removed".into()),
            || {},
            4
        )
        .unwrap_err()
        .contains("removed"));
    }
}
