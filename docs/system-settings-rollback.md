# System settings baseline and rollback

**Implemented:** macOS audio plus current-user saved Dock, keyboard/text, pointer, and Finder storage
preferences (21 restore entries when all controls are available). Hardware formats and power timers
have read-only discovery, not persistent restore adapters.
This is not a full-system backup, OS downgrade, performance optimization, or a claim that writing a
preference immediately changes live behavior. Windows and other platforms remain explicitly
unsupported; no Windows baseline adapter was implemented or cross-compiled in this change.

## Exact capture/restore scope

Users can include Audio, Desktop & Dock, Keyboard, Pointer, and Finder storage independently. Every Set Default
creates an immutable snapshot and keeps every earlier snapshot. Each snapshot includes a per-setting
coverage report, including exclusions. Existing v1 audio-only snapshots and separate legacy
provider-history files remain readable/preserved; restoring an old audio snapshot does not touch
newly supported preferences.

| Group          | Setting                            | Native property / domain and key                          | Stored type                        |
| -------------- | ---------------------------------- | --------------------------------------------------------- | ---------------------------------- |
| Audio          | Default output master volume       | CoreAudio `kAudioDevicePropertyVolumeScalar`, output/main | Float32, 0–1                       |
| Audio          | Default output master mute         | CoreAudio `kAudioDevicePropertyMute`, output/main         | UInt32 Boolean                     |
| Desktop & Dock | Auto-hide                          | `com.apple.dock` / `autohide`                             | Boolean or absent                  |
| Desktop & Dock | Magnification                      | `com.apple.dock` / `magnification`                        | Boolean or absent                  |
| Desktop & Dock | Animate opening applications       | `com.apple.dock` / `launchanim`                           | Boolean or absent                  |
| Desktop & Dock | Minimize into application icon     | `com.apple.dock` / `minimize-to-application`              | Boolean or absent                  |
| Desktop & Dock | Show recent applications           | `com.apple.dock` / `show-recents`                         | Boolean or absent                  |
| Desktop & Dock | Icon size                          | `com.apple.dock` / `tilesize`                             | Integer or real, 16–256, or absent |
| Keyboard       | Repeat interval                    | global / `KeyRepeat`                                      | Integer, 0–10000, or absent        |
| Keyboard       | Initial repeat delay               | global / `InitialKeyRepeat`                               | Integer, 0–10000, or absent        |
| Keyboard       | Press-and-hold accent menu         | global / `ApplePressAndHoldEnabled`                       | Boolean or absent                  |
| Pointer        | Natural scrolling                  | global / `com.apple.swipescrolldirection`                 | Boolean or absent                  |
| Pointer        | Mouse tracking speed               | global / `com.apple.mouse.scaling`                        | Real, −1–10, or absent             |
| Pointer        | Trackpad tracking speed            | global / `com.apple.trackpad.scaling`                     | Real, −1–10, or absent             |
| Keyboard       | Automatic spelling correction      | global / `NSAutomaticSpellingCorrectionEnabled`           | Boolean or absent                  |
| Keyboard       | Automatic capitalization           | global / `NSAutomaticCapitalizationEnabled`               | Boolean or absent                  |
| Keyboard       | Automatic period substitution      | global / `NSAutomaticPeriodSubstitutionEnabled`           | Boolean or absent                  |
| Keyboard       | Smart dashes                       | global / `NSAutomaticDashSubstitutionEnabled`             | Boolean or absent                  |
| Keyboard       | Smart quotes                       | global / `NSAutomaticQuoteSubstitutionEnabled`            | Boolean or absent                  |
| Finder storage | Avoid .DS_Store on network volumes | `com.apple.desktopservices` / `DSDontWriteNetworkStores`  | Boolean or absent                  |
| Finder storage | Avoid .DS_Store on USB volumes     | `com.apple.desktopservices` / `DSDontWriteUSBStores`      | Boolean or absent                  |

The Finder entries are **future file-creation preferences**, not cleanup: existing files are never
deleted, regenerated, or restored. USB behavior is an observed preference convention, not a public
Finder API guarantee. Applications can override global text preferences. Tracking speed is not a
separately captured pointer-acceleration toggle. No predetermined values are supplied.

The preference adapter is enabled on macOS major versions **13–15** only. Exact OS-version matching
is required for restoration, including minor/patch changes. These keys are observed macOS preference
conventions, **not public Apple guarantees of live-setting semantics**. The public API guarantees
preference storage operations, not that Dock or an input driver rereads a key. Future OS versions fail
closed for preferences until reviewed. CoreAudio support is independently discovered through the
public device property API.

Preference scope is exactly `kCFPreferencesCurrentUser` + `kCFPreferencesAnyHost`. Global preferences
use `kCFPreferencesAnyApplication` for primitive read/write/synchronize calls. Per-host, per-app,
managed, and device-driver overrides are **not** erased or claimed as captured. Pointer settings here
are user preference overrides, not per-device calibration or proof that a trackpad is attached.
Integer and real property-list types remain distinct; strings, arrays, dictionaries, nonfinite values,
and out-of-range values are not captured. Keyboard integers are preserved in their native units,
not converted to milliseconds or guessed slider positions.

An absent key is a first-class typed value. Restoring absence calls `CFPreferencesSetValue` with NULL,
removing only that exact scope's override. It does not write a guessed factory value. Other scopes may
still determine effective behavior.

## Coverage, access, and permissions

Coverage & access inventories each setting as supported, needs permission, read-only, manual, or unavailable.
Manual means there is a user-controlled settings route but no validated restore adapter; unavailable
means discovery is missing/failed or a platform API is unsupported. Unknown values are never replaced
with defaults. A successful read-only observation does not make an entry selectable.
Current implemented writable adapters require no microphone, Accessibility, Full Disk Access, or
administrator grant. **Supported means a readable valid value and implemented setter, not proof of
write permission.** Managed preferences are rejected by `CFPreferencesAppValueIsForced`; denied
synchronization, unsupported native types, and unvalidated OS versions are explained individually.
No permission is requested automatically. Recheck access performs fresh discovery, including after
manual permission/management changes. There is no fake grant button.

| Additional setting             | Current behavior                                                                                                                                                             |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Reduce motion                  | Read-only effective value from `NSWorkspace.accessibilityDisplayShouldReduceMotion`; no baseline restoration                                                                 |
| Reduce transparency            | Read-only effective value from `NSWorkspace.accessibilityDisplayShouldReduceTransparency`; no baseline restoration                                                           |
| Stage Manager                  | Unavailable; no reliable public reversible setting API implemented                                                                                                           |
| Display sleep timeout          | Read-only `pmset -g custom` saved `displaysleep`, separately per reported AC/Battery/UPS source; no authorized restore                                                       |
| System idle sleep              | Read-only `pmset -g custom` saved `sleep`, separately per reported AC/Battery/UPS source; no authorized restore                                                              |
| Display brightness             | Unavailable; no implemented verified per-display adapter across Apple Silicon and external displays                                                                          |
| Audio nominal rate/full format | Read-only nominal rate, supported rate ranges, full physical-stream ASBD, matching supported format ranges, and settable status; default-device UID and stream drift checked |
| Display mode/refresh           | Read-only full CoreGraphics mode: points, pixels, refresh, mode ID, flags, desktop usability, exact supported-mode match; stable display UUID and topology drift checked     |
| Low/High Power Mode            | Manual; hardware and power-source dependent, no authorized writer; undocumented numeric `powermode` values are not interpreted                                               |
| Login/background items         | Manual; ServiceManagement does not provide a general reversible adapter for other applications' registrations and approvals                                                  |
| Time Machine scheduling        | Manual; no typed authorized schedule adapter; backup execution/deletion is excluded                                                                                          |
| Pointer acceleration           | Manual, explicitly distinct from captured tracking speed; no validated separate acceleration adapter                                                                         |
| iCloud Optimize Mac Storage    | Manual account-managed policy; no eviction, download, or file deletion                                                                                                       |
| Microphone TCC grants          | Manual Privacy & Security approval; grants cannot be replayed; no discovery prompt or TCC database writes                                                                    |
| Spotlight exclusions           | Manual Search Privacy; no exact supported exclusion-list restore adapter, no indexing actions                                                                                |
| Developer Tools access         | Manual per-app privacy approval, not replayable; no security-disabling controls                                                                                              |

### Hardware persistence gate

The public SDK explicitly documents `CGDisplaySetDisplayMode` as **application-lifetime**, reverting
on termination. `CGCompleteDisplayConfiguration(kCGConfigurePermanently)` can fall back to the login
session if the UI cannot support the configuration. Neither an immediate readback nor a mode's Hz
proves persistence. Display writes are therefore not enabled. Full modes are compared, never refresh
alone; a reported zero refresh is shown as unknown/variable, not converted to a guessed Hz. Deprecated
pixel-encoding APIs are not used. Full-mode observations are not HDR/color-profile/VRR-policy backups.

CoreAudio `AudioObjectIsPropertySettable` describes driver writability, not persistence across reboot
or reconnect. No device-specific persistence contract has been validated for nominal rate or physical
format, so their writers are also gated. The physical ASBD observation retains sample rate, format ID,
flags, bytes/packet, frames/packet, bytes/frame, channels, bits/channel, and reserved field; supported
rate ranges alone do not validate a full format. Per-stream errors remain explicitly unknown.
These observations are saved only in the **excluded coverage report**, not `SystemValues` and not
restore targets. Stable hardware identities are checked in memory, not exposed in report text.

The fixed `/usr/bin/pmset -g custom` probe is read-only, bounded to two seconds and 16 KiB, with no
caller-controlled executable or arguments. Missing sources/keys remain unknown; zero means never.
This is saved profile state, not effective sleep behavior (assertions, lid state, and hardware can
override it). The `pmset(1)` manual requires root to modify settings and documents per-system
persistence. No safe native authorized helper exists in this app; there is no fake grant or writable
power adapter. Timeout cleanup reaps only the app's own probe subprocess, never user workloads.

### Excluded one-time actions

This baseline never offers purge, cache deletion, DNS flushing, quarantine removal, sleepimage deletion,
process termination, temporary renice, session keep-awake assertions, index rebuilds, bulk tweaks,
or security-disabling commands. Those are not persistent settings that this history could restore.
No generalized command runner or automatic optimization values were added.

Power settings are not presented as grantable just because an administrator password could exist.
Jitterbug does not use `sudo`, ask for passwords, install a persistent privileged helper, or use a
custom authorization dialog. The public IOKit graphics SDK includes `IODisplayGetFloatParameter` /
`IODisplaySetFloatParameter`, but that does not establish support on every display; no private
DisplayServices or DDC command fallback is claimed. Universal-access preference writes and UI scripting
are not substituted for missing public accessibility setters.

User-triggered Open System Settings buttons use `NSWorkspace.openURL` and a fixed allowlist of
`x-apple.systempreferences` destinations. Callers cannot supply a URL or command. Opening a pane is
not a grant and does not restore a setting. If a macOS build does not resolve its pane link, the UI
provides the setting/category name for manual navigation. No security, networking, updates, device
selection, process termination, or arbitrary setting writes are included.

## Native API review and verification semantics

- CoreAudio SDK `AudioHardware.h` / `AudioHardwareBase.h`: default output discovery, device UID,
  `AudioObjectHasProperty`, `AudioObjectIsPropertySettable`, `AudioObjectGetPropertyData`, and
  `AudioObjectSetPropertyData`. Audio verification means fresh live property readback, not audible
  output or improved performance.
- CoreFoundation SDK `CFPreferences.h`: `CFPreferencesCopyValue` searches only the exact scope;
  `CFPreferencesSetValue(NULL)` deletes the override; `CFPreferencesSynchronize` persists/reloads it.
  Saved preference verification never means live-effect verification.
- **App-level CFPreferences APIs forbid `kCFPreferencesAnyApplication`.** Global managed-state checks
  use `kCFPreferencesCurrentApplication` and its global search chain; Dock checks use its bundle ID.
- AppKit SDK `NSAccessibility.h`: the two NSWorkspace display-accessibility queries are read-only;
  they do not prompt for Accessibility permission and have no corresponding supported setter here.
- CoreGraphics SDK `CGDirectDisplay.h` documents the application lifetime of `CGDisplaySetDisplayMode`;
  `CGDisplayConfiguration.h` documents the permanent-to-session fallback. Read-only discovery uses
  `CGDisplayCreateUUIDFromDisplayID`, `CGDisplayCopyDisplayMode`, `CGDisplayCopyAllDisplayModes`, and
  nondeprecated full-mode accessors. No display setter is linked by the new adapter.
- CoreAudio SDK `AudioHardwareBase.h` defines `kAudioDevicePropertyNominalSampleRate`,
  `kAudioDevicePropertyAvailableNominalSampleRates`, `kAudioDevicePropertyStreams`,
  `kAudioStreamPropertyPhysicalFormat`, and `kAudioStreamPropertyAvailablePhysicalFormats`.
  The new format observer links only read/size/settable queries, not a format setter.
- `pmset(1)` sections SETTING, GETTING, and FILES document root authorization, `-g custom`, per-source
  timers, and per-system preference persistence. No private power plist is written.
- `geteuid(2)` and `gethostuuid(3)` supply compatibility identity. Only locally keyed hashes are saved.

Reference documentation and observed key semantics:

- [CFPreferencesCopyValue](<https://developer.apple.com/documentation/corefoundation/cfpreferencescopyvalue(_:_:_:_:)>)
- [CFPreferencesSetValue](<https://developer.apple.com/documentation/corefoundation/cfpreferencessetvalue(_:_:_:_:_:)>)
- [CFPreferencesSynchronize](<https://developer.apple.com/documentation/corefoundation/cfpreferencessynchronize(_:_:_:)>)
- [Dock preference examples](https://macos-defaults.com/dock/)
- [Press-and-hold behavior and relaunch requirements](https://macos-defaults.com/keyboard/applepressandholdenabled.html)
- [Mouse scaling and restart requirements](https://macos-defaults.com/mouse/scaling.html)

The community examples often restart Dock; Jitterbug deliberately does not. Preference consumers may
cache values until application relaunch, sign-out, or a Mac restart. The user must decide when to do
that. No Dock/SystemUIServer restart is performed or offered as an implicit side effect.

## Conflict protection, partial results, and persistence

Snapshots remain in `system-settings-defaults-v1`, with additive defaulted fields for compatibility.
Immutable files and the current pointer are separate. Legacy provider history stays in its original
namespace and is neither migrated nor deleted. Files are private, size bounded, and durably fsynced.
History is cursor-paginated; both the history dialog and coverage views have bounded, keyboard-focusable
scroll regions.

A five-minute one-use preview binds exact live values, target digest, OS, user, machine, and default
audio device. Category selection limits writes, not drift detection. Before each individual write,
the controller rereads all supported values and compatibility. After the applying journal is fsynced,
the adapter checks again. Audio passes the exact inspected object ID to its setter; preferences use
only a typed allowlist and recheck the exact key immediately before setting it. Old audio baselines
and selected-category baselines leave other preferences unchanged. Audio unavailability does not
prevent independent preference capture; its compatibility sentinel prevents later silent rebinding.

There is no atomic compare-and-set across macOS preference/audio APIs. An external change during the
last native call cannot be ruled out. Restore operations have finite setting lists, no command execution,
no unbounded retries, and immediate readback; native synchronous APIs do not provide a cancellable
per-call deadline. The identity query has a five-second timeout.

Intent and each outcome are durable before further writes. A denial, error, mismatch, or drift stops
remaining writes. Results explicitly show verified, failed, and not-attempted entries. A failed write
may already have changed native state; earlier successful writes are not automatically undone.
Interrupted journals are reported and **never replayed**. CoreFoundation owns an in-process cache;
a synchronization failure is not an assurance that nothing changed. No automatic retry/recovery
writes are made by the controller.

Capture and rollback use the supervisor's shared native-action lock and reject active incident /
Jitter Snapshot stages. Awaiting-approval and terminal stages do not leave a stale block. Snapshot
reports and optimizer incidents are not baseline snapshots and never replace the user's default.

## Verification record and limits

The read-only native probe on macOS 15 discovered all 21 allowlisted capture/restore entries, read-only
audio-format/display-mode/power-timer and accessibility observations, plus explicit manual/unavailable
coverage. It exercised capture to
isolated temporary app storage and no-op preview, then removed that test storage. Identities and actual
values were not printed or committed. This verifies discovery, type checks, serialization, capability
reporting, and no-change preview, **not native writes**.

Injected tests cover every preference in both present/absent directions, exact native scalar conversion,
wrong types/ranges, category selection, old audio files, repeated history, denied access and subsequent
recheck, stale permissions/previews, OS changes, readback mismatch, partial apply, external drift,
journaling, and the shared action lock. Browser UI tests use mocked IPC, not real OS mutations.

The following remain **unexercised on the host**, intentionally, because no consent was given to change
real settings:

- CoreAudio volume/mute setters.
- CFPreferences setters, deletion, persistence after a setter, and actual consumers applying changes.
- Real managed-policy/permission denials and privilege grants (injected, not granted on the host).
- System Settings pane navigation (allowlist and IPC are tested, not real UI navigation).
- Windows compilation/runtime support.

Run the opt-in read-only probe with:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib real_macos_discovery_is_read_only -- --ignored --nocapture
```

No test invokes the native setting setters. Normal unit tests use in-memory adapters; native scalar
conversion tests allocate CoreFoundation objects without persisting preferences. Baseline verification
confirms captured/storage values only within its stated scope, never a performance improvement.
