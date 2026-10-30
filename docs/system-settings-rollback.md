# System settings baseline and rollback

Jitterbug’s system baseline is intentionally narrower than a system backup. On macOS it captures
only settable master volume and mute values exposed by the current default audio output device.
Windows and other platforms report the feature as unsupported.

## Public native API review

The implementation was checked against the declarations and documentation shipped in the installed
macOS SDK’s `CoreAudio.framework/Headers/AudioHardware.h` and `AudioHardwareBase.h`:

- `kAudioHardwarePropertyDefaultOutputDevice` discovers the current default output device.
- `kAudioDevicePropertyDeviceUID` provides the device’s persistent opaque identity.
- `kAudioDevicePropertyVolumeScalar` is a `Float32` from 0 through 1.
- `kAudioDevicePropertyMute` is a `UInt32` Boolean control.
- `AudioObjectHasProperty` and `AudioObjectIsPropertySettable` gate every included control.
- `AudioObjectGetPropertyData` reads device identity and actual values.
- `AudioObjectSetPropertyData` is the only native mutation mechanism in this feature.
- `geteuid(2)` and the macOS `gethostuuid(3)` declaration provide compatibility-only user and
  machine identities; only keyed hashes are persisted.

No undocumented preference domains, `defaults` writes, daemon restarts, process kills, elevated
commands, privacy/security/network controls, or output-device selection are used.

## Safety and persistence

Snapshots live in the new `system-settings-defaults-v1` storage namespace. Existing
`settings-default-history` provider files are never read, migrated, overwritten, or deleted by this
feature. Every baseline is a create-only file; the current pointer is separate. Compatibility
bindings hash the effective user, machine, and output-device identities with a private local random
key. Raw identities are not persisted.

A rollback preview binds the target snapshot and actual current values, expires after five minutes,
and is consumed once. Confirmation re-reads compatibility and values. Before each individual write,
Jitterbug checks them again. A create-only, fsynced intent journal exists before the first write;
each write and readback outcome is then durably recorded. Interrupted journals are reported but are
never replayed automatically. Partial failures stop remaining writes and do not silently undo an
earlier verified change.

After the applying journal is fsynced, the native adapter rechecks both identity and expected
values and passes the exact inspected device ID to the setter, never a newly resolved default.
CoreAudio provides no atomic compare-and-set: an external change during the final native calls
cannot be excluded. Verification uses immediate fresh readback, not an unbounded retry loop.
Asynchronous device updates that are not yet visible are reported as a readback mismatch and stop
remaining writes; a read error returns unknown current values, never the expected values.

The shared supervisor action lock prevents rollback from overlapping Jitterbug automation or process
actions. Native writes are permitted only for the two typed allowlist entries. Models cannot add a
setting or issue a command.

## Verification record

During final repository verification, the manual read-only native probe successfully discovered
supported controls through CoreAudio and validated their values and compatibility identity. The
probe deliberately omitted identities and values from its output. No native settings were changed.

The following write paths were **not exercised** during automated or manual verification:

- `AudioObjectSetPropertyData(device, kAudioDevicePropertyVolumeScalar, output/main, Float32)`
- `AudioObjectSetPropertyData(device, kAudioDevicePropertyMute, output/main, UInt32)`

All mutation tests use injected in-memory adapters. Passing readback means the requested preference
value was observed through CoreAudio; it is not evidence that performance, resource usage, or audible
behavior improved.
