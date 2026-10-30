//! CoreAudio rate/full physical-format restore. Readback is not a persistence promise.
use super::*;
use crate::system_settings::hardware::{AudioFormat as Format, AudioStream};
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Range {
    min: f64,
    max: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct RangedFormat {
    format: Format,
    rate: Range,
}
fn supports(format: Format, available: RangedFormat) -> bool {
    format.sample_rate.is_finite()
        && available.rate.min.is_finite()
        && available.rate.max.is_finite()
        && format.sample_rate > 0.0
        && available.rate.min > 0.0
        && (available.rate.min..=available.rate.max).contains(&format.sample_rate)
        && Format {
            sample_rate: 0.0,
            ..format
        } == Format {
            sample_rate: 0.0,
            ..available.format
        }
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectGetPropertyDataSize(
        object: AudioObjectId,
        address: *const PropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
    ) -> OsStatus;
}
fn array<T: Copy + Default>(
    device: AudioObjectId,
    property: &PropertyAddress,
) -> Result<Vec<T>, String> {
    let mut size = 0;
    // SAFETY: public read-only size query, initialized address and valid UInt32 output.
    status(
        unsafe { AudioObjectGetPropertyDataSize(device, property, 0, ptr::null(), &mut size) },
        "size supported audio formats",
    )?;
    if size == 0 || size as usize > 65536 || !(size as usize).is_multiple_of(size_of::<T>()) {
        return Err("Unsupported audio inventory size.".into());
    }
    let expected = size;
    let mut values = vec![T::default(); size as usize / size_of::<T>()];
    // SAFETY: private call sites use exact SDK property types; initialized capacity matches size.
    status(
        unsafe {
            AudioObjectGetPropertyData(
                device,
                property,
                0,
                ptr::null(),
                &mut size,
                values.as_mut_ptr().cast(),
            )
        },
        "read supported audio formats",
    )?;
    if size != expected {
        return Err("Audio inventory changed during discovery.".into());
    }
    Ok(values)
}
pub(in crate::system_settings) fn annotate(items: &mut [crate::system_settings::Capability]) {
    if let Some(item) = items.iter_mut().find(|v| v.id == "audio_format") {
        item.status = crate::system_settings::CapabilityStatus::Unavailable;
        item.detail = match collect() {
            Err(error) => format!("Automatic executor cannot capture this configuration: {error}. Recheck hardware support and access; no guessed values are saved."),
            Ok(_) => "Hardware changed during discovery. Recheck before capture.".into(),
        };
        item.verification =
            "Not captured; automatic executor requires readable supported native descriptors"
                .into();
    }
}

// Native IDs live only in an inspected handle, never in persisted slots.
struct Handle {
    device: u32,
    streams: Vec<(u32, u32, u32)>,
    inventory: Vec<u32>,
}
fn inspect() -> Result<(Handle, HardwareValue), String> {
    let device = default_device()?;
    let uid = device_uid(device)?;
    let rate_property = address(fourcc(b"nsrt"), GLOBAL);
    let rate = if settable(device, &rate_property)? {
        let rate = read_value(device, &rate_property, "read nominal sample rate", 0.0_f64)?;
        check_rate(device, rate)?;
        Some(rate)
    } else {
        None
    };
    let ids: Vec<u32> = array(device, &address(fourcc(b"stm#"), OUTPUT))?;
    if ids.len() > 16 {
        return Err("Audio stream inventory exceeds safe bound".into());
    }
    let mut streams = Vec::new();
    let mut handles = Vec::new();
    for id in &ids {
        let property = address(fourcc(b"pft "), GLOBAL);
        if !settable(*id, &property)? {
            continue;
        }
        let channel = read_value(
            *id,
            &address(fourcc(b"schn"), GLOBAL),
            "read stream starting channel",
            0_u32,
        )?;
        let terminal = read_value(
            *id,
            &address(fourcc(b"term"), GLOBAL),
            "read stream terminal type",
            0_u32,
        )?;
        let direction = read_value(
            *id,
            &address(fourcc(b"sdir"), GLOBAL),
            "read stream direction",
            1_u32,
        )?;
        if direction != 0
            || read_value(
                *id,
                &address(fourcc(b"stdv"), GLOBAL),
                "read stream owner",
                0_u32,
            )? != device
        {
            return Err("Expected output stream direction".into());
        }
        let format = read_value(*id, &property, "read physical format", Format::default())?;
        check_format(*id, format)?;
        streams.push(AudioStream {
            starting_channel: channel,
            terminal_type: terminal,
            format,
        });
        handles.push((*id, channel, terminal));
    }
    streams.sort_by_key(|s| s.starting_channel);
    handles.sort_by_key(|s| s.1);
    let value = HardwareValue::Audio {
        uid: uid.clone(),
        rate,
        streams,
    };
    value.validate(0)?;
    if default_device()? != device
        || device_uid(device)? != uid
        || array::<u32>(device, &address(fourcc(b"stm#"), OUTPUT))? != ids
    {
        return Err("Audio identity/topology changed during inspection".into());
    }
    let handle = Handle {
        device,
        streams: handles,
        inventory: ids,
    };
    if read_bound(&handle, &value)? != value {
        return Err("Audio rate/format changed during inspection".into());
    }
    Ok((handle, value))
}
fn check_rate(device: u32, rate: f64) -> Result<(), String> {
    let ranges: Vec<Range> = array(device, &address(fourcc(b"nsr#"), GLOBAL))?;
    if !rate.is_finite()
        || rate <= 0.0
        || !ranges.iter().any(|r| {
            r.min.is_finite() && r.max.is_finite() && r.min > 0.0 && (r.min..=r.max).contains(&rate)
        })
    {
        return Err("Saved nominal rate is outside advertised supported ranges".into());
    }
    Ok(())
}
fn check_format(stream: u32, format: Format) -> Result<(), String> {
    let available: Vec<RangedFormat> = array(stream, &address(fourcc(b"pfta"), GLOBAL))?;
    if !available.iter().any(|v| supports(format, *v)) {
        return Err("Saved full ASBD is not supported by this output stream".into());
    }
    Ok(())
}
fn check_inventory(handle: &Handle, ids: &[u32]) -> Result<(), String> {
    if handle.inventory != ids {
        return Err("Output stream inventory changed before format write".into());
    }
    Ok(())
}
fn validate_handle(handle: &Handle, target: &HardwareValue) -> Result<(), String> {
    let HardwareValue::Audio { uid, streams, .. } = target else {
        return Err("Wrong audio target type".into());
    };
    if default_device()? != handle.device || device_uid(handle.device)? != *uid {
        return Err("Output device changed before format write".into());
    }
    let ids: Vec<u32> = array(handle.device, &address(fourcc(b"stm#"), OUTPUT))?;
    check_inventory(handle, &ids)?;
    for ((id, channel, terminal), saved) in handle.streams.iter().zip(streams) {
        if !ids.contains(id)
            || read_value(
                *id,
                &address(fourcc(b"stdv"), GLOBAL),
                "revalidate stream owner",
                0_u32,
            )? != handle.device
            || *channel != saved.starting_channel
            || *terminal != saved.terminal_type
            || read_value(
                *id,
                &address(fourcc(b"schn"), GLOBAL),
                "revalidate stream channel",
                0_u32,
            )? != *channel
            || read_value(
                *id,
                &address(fourcc(b"term"), GLOBAL),
                "revalidate stream terminal",
                0_u32,
            )? != *terminal
            || read_value(
                *id,
                &address(fourcc(b"sdir"), GLOBAL),
                "revalidate stream direction",
                1_u32,
            )? != 0
        {
            return Err("Output stream identity changed".into());
        }
    }
    Ok(())
}
fn read_bound(handle: &Handle, shape: &HardwareValue) -> Result<HardwareValue, String> {
    validate_handle(handle, shape)?;
    let mut actual = shape.clone();
    let HardwareValue::Audio { rate, streams, .. } = &mut actual else {
        return Err("Wrong audio target type".into());
    };
    if rate.is_some() {
        let property = address(fourcc(b"nsrt"), GLOBAL);
        if !settable(handle.device, &property)? {
            return Err("Nominal rate became read-only".into());
        }
        *rate = Some(read_value(
            handle.device,
            &property,
            "recheck bound nominal rate",
            0.0_f64,
        )?);
    }
    for ((id, _, _), stream) in handle.streams.iter().zip(streams) {
        let property = address(fourcc(b"pft "), GLOBAL);
        if !settable(*id, &property)? {
            return Err("Physical format became read-only".into());
        }
        stream.format = read_value(
            *id,
            &property,
            "recheck bound physical format",
            Format::default(),
        )?;
    }
    validate_handle(handle, &actual)?;
    Ok(actual)
}
fn require_idle(device: u32) -> Result<(), String> {
    let property = address(fourcc(b"gone"), GLOBAL);
    if unsafe { AudioObjectHasProperty(device, &property) } != 0
        && read_value(device, &property, "inspect device busy state", 0_u32)? != 0
    {
        return Err("Audio device is busy. Stop playback/recording in audio applications, then re-preview; Jitterbug does not stop them.".into());
    }
    Ok(())
}
pub(in crate::system_settings) fn collect() -> Result<HardwareValue, String> {
    inspect().map(|(_, value)| value)
}
pub(in crate::system_settings) fn preflight(target: &HardwareValue) -> Result<(), String> {
    target.validate(0)?;
    let (handle, live) = inspect()?;
    if !live.same_identity(target) {
        return Err("Saved output device/stream identity unavailable".into());
    }
    let HardwareValue::Audio { rate, streams, .. } = target else {
        return Err("Wrong audio target type".into());
    };
    if let Some(rate) = rate {
        check_rate(handle.device, *rate)?;
    }
    for ((id, _, _), stream) in handle.streams.iter().zip(streams) {
        check_format(*id, stream.format)?;
    }
    Ok(())
}
pub(in crate::system_settings) fn write(
    expected: &HardwareValue,
    target: &HardwareValue,
) -> Result<(), String> {
    target.validate(0)?;
    hardware::execute_verified(
        expected,
        target,
        || {
            let (handle, live) = inspect()?;
            let HardwareValue::Audio { rate, streams, .. } = target else {
                return Err("Wrong audio target type".into());
            };
            if !live.same_identity(target) {
                return Err("Saved audio device or stream identity no longer matches".into());
            }
            if let Some(rate) = rate {
                check_rate(handle.device, *rate)?;
            }
            for ((id, _, _), stream) in handle.streams.iter().zip(streams) {
                check_format(*id, stream.format)?;
            }
            require_idle(handle.device)?;
            Ok((handle, live))
        },
        |handle| {
            let HardwareValue::Audio { rate, streams, .. } = target else {
                unreachable!()
            };
            validate_handle(&handle, target)?;
            if read_bound(&handle, expected)? != *expected {
                return Err("Audio values changed immediately before first format write".into());
            }
            require_idle(handle.device)?;
            if let Some(rate) = rate {
                let property = address(fourcc(b"nsrt"), GLOBAL);
                if !settable(handle.device, &property)? {
                    return Err("Nominal rate is no longer settable".into());
                }
                if read_value(handle.device, &property, "recheck nominal rate", 0.0_f64)? != *rate {
                    write_value(
                        handle.device,
                        &property,
                        "set nominal sample rate (driver may be busy)",
                        rate,
                    )?;
                    let mut matched = false;
                    for _ in 0..40 {
                        validate_handle(&handle, target)?;
                        if read_value(handle.device, &property, "verify nominal rate", 0.0_f64)?
                            == *rate
                        {
                            matched = true;
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    if !matched {
                        return Err(
                            "Nominal rate asynchronous readback timed out; state may have changed"
                                .into(),
                        );
                    }
                }
            }
            for ((id, _, _), stream) in handle.streams.iter().zip(streams) {
                validate_handle(&handle, target)?;
                let property = address(fourcc(b"pft "), GLOBAL);
                if !settable(*id, &property)? {
                    return Err("Physical stream format is no longer settable; earlier changes may have applied".into());
                }
                check_format(*id, stream.format)?;
                if read_value(*id, &property, "recheck physical format", Format::default())?
                    != stream.format
                {
                    write_value(
                        *id,
                        &property,
                        "set physical stream ASBD (driver may be busy)",
                        &stream.format,
                    ).map_err(|error| format!("Physical output stream at channel {} failed: {error} Earlier rate/format changes may already have applied; no automatic undo.", stream.starting_channel))?;
                }
            }
            Ok(())
        },
        collect,
        || std::thread::sleep(Duration::from_millis(50)),
        40,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_audio_inspection_remains_bound_to_exact_stream_inventory() {
        let handle = Handle {
            device: 1,
            streams: vec![(2, 1, 1)],
            inventory: vec![2, 3],
        };
        assert!(check_inventory(&handle, &[2, 3]).is_ok());
        assert!(check_inventory(&handle, &[2, 4]).is_err());
        assert!(check_inventory(&handle, &[2, 3, 4]).is_err());
    }
    #[test]
    fn supported_format_requires_full_asbd_not_just_sample_rate() {
        let format = Format {
            sample_rate: 48000.0,
            format_id: fourcc(b"lpcm"),
            flags: 1,
            bytes_per_packet: 8,
            frames_per_packet: 1,
            bytes_per_frame: 8,
            channels: 2,
            bits_per_channel: 32,
            reserved: 0,
        };
        let available = RangedFormat {
            format: Format {
                sample_rate: 0.0,
                ..format
            },
            rate: Range {
                min: 44100.0,
                max: 96000.0,
            },
        };
        assert!(supports(format, available));
        assert!(!supports(
            Format {
                channels: 1,
                ..format
            },
            available
        ));
        assert!(!supports(Format { flags: 2, ..format }, available));
        assert!(!supports(
            Format {
                sample_rate: 192000.0,
                ..format
            },
            available
        ));
        assert!(!supports(
            Format {
                sample_rate: f64::NAN,
                ..format
            },
            available
        ));
        assert_eq!(size_of::<Format>(), 40);
        assert_eq!(size_of::<RangedFormat>(), 56);
    }
}
