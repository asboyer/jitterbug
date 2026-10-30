//! Live driver format observations only; no unvalidated persistence promise or setter.
use super::*;
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Format {
    sample_rate: f64,
    format_id: u32,
    flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels: u32,
    bits_per_channel: u32,
    reserved: u32,
}
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
fn stream_format(stream: AudioObjectId) -> Result<String, String> {
    let property = address(fourcc(b"pft "), GLOBAL);
    let format: Format = read_value(
        stream,
        &property,
        "read physical stream format",
        Format::default(),
    )?;
    if !format.sample_rate.is_finite() || format.sample_rate <= 0.0 || format.channels == 0 {
        return Err("Invalid physical stream format.".into());
    }
    let available: Vec<RangedFormat> = array(stream, &address(fourcc(b"pfta"), GLOBAL))?;
    let supported = available.iter().any(|v| supports(format, *v));
    let writable = settable(stream, &property)?;
    let after = read_value(
        stream,
        &property,
        "recheck physical stream format",
        Format::default(),
    )?;
    if after != format {
        return Err("Physical format changed during discovery.".into());
    }
    Ok(format!("ASBD rate={} Hz, format={:#x}, flags={:#x}, bytes/packet={}, frames/packet={}, bytes/frame={}, channels={}, bits/channel={}, reserved={}; matches supported range={}, settable={} (not persistence)", format.sample_rate, format.format_id, format.flags, format.bytes_per_packet, format.frames_per_packet, format.bytes_per_frame, format.channels, format.bits_per_channel, format.reserved, supported, writable))
}
fn observe() -> Result<String, String> {
    let device = default_device()?;
    let uid = device_uid(device)?;
    let property = address(fourcc(b"nsrt"), GLOBAL);
    let rate: f64 = read_value(device, &property, "read nominal sample rate", 0.0)?;
    if !rate.is_finite() || rate <= 0.0 {
        return Err("Invalid nominal sample rate.".into());
    }
    let ranges: Vec<Range> = array(device, &address(fourcc(b"nsr#"), GLOBAL))?;
    let supported = ranges.iter().any(|v| {
        v.min.is_finite() && v.max.is_finite() && v.min > 0.0 && (v.min..=v.max).contains(&rate)
    });
    let streams: Vec<u32> = array(device, &address(fourcc(b"stm#"), OUTPUT))?;
    if streams.len() > 16 {
        return Err("Audio stream inventory exceeds safe bound.".into());
    }
    let writable = settable(device, &property)?;
    let mut details = vec![format!("Nominal rate {rate} Hz; within advertised range={supported}; settable={writable} (not persistence)")];
    for (index, stream) in streams.iter().enumerate() {
        details.push(format!(
            "Output stream {}: {}",
            index + 1,
            stream_format(*stream).unwrap_or_else(|e| format!("unknown: {e}"))
        ));
    }
    let after: Vec<u32> = array(device, &address(fourcc(b"stm#"), OUTPUT))?;
    if after != streams
        || default_device()? != device
        || device_uid(device)? != uid
        || read_value(device, &property, "recheck nominal sample rate", 0.0_f64)? != rate
    {
        return Err(
            "Audio device identity, streams, or nominal rate changed during discovery.".into(),
        );
    }
    Ok(details.join("; "))
}
pub(in crate::system_settings) fn annotate(items: &mut [Capability]) {
    let Some(item) = items.iter_mut().find(|v| v.id == "audio_format") else {
        return;
    };
    match observe() {
        Ok(detail) => {
            item.status = CapabilityStatus::ReadOnly;
            item.detail = format!("Live observation only: {detail}. {}", item.detail);
            item.verification = "CoreAudio nominal rate/full physical ASBD observation; not a restorable persistent value".into();
        }
        Err(error) => {
            item.status = CapabilityStatus::Unavailable;
            item.detail = format!("Unknown: {error} {}", item.detail);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
