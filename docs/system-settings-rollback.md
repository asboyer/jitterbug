# System settings baseline and rollback

**Implemented:** macOS audio plus current-user saved Dock, keyboard, and pointer preferences.
This is not a full-system backup, OS downgrade, performance optimization, or a claim that writing a
preference immediately changes live behavior. Windows and other platforms remain explicitly
unsupported; no Windows baseline adapter was implemented or cross-compiled in this change.

## Exact capture/restore scope

Users can include Audio, Desktop & Dock, Keyboard, and Pointer independently. Every Set Default
creates an immutable snapshot and keeps every earlier snapshot. Each snapshot includes a per-setting
coverage report, including exclusions. Existing v1 audio-only snapshots and separate legacy
provider-history files remain readable/preserved; restoring an old audio snapshot does not touch
newly supported preferences.

| Group          | Setting                        | Native property / domain and key                          | Stored type                        |
| -------------- | ------------------------------ | --------------------------------------------------------- | ---------------------------------- |
| Audio          | Default output master volume   | CoreAudio `kAudioDevicePropertyVolumeScalar`, output/main | Float32, 0–1                       |
| Audio          | Default output master mute     | CoreAudio `kAudioDevicePropertyMute`, output/main         | UInt32 Boolean                     |
| Desktop & Dock | Auto-hide                      | `com.apple.dock` / `autohide`                             | Boolean or absent                  |
| Desktop & Dock | Magnification                  | `com.apple.dock` / `magnification`                        | Boolean or absent                  |
| Desktop & Dock | Animate opening applications   | `com.apple.dock` / `launchanim`                           | Boolean or absent                  |
| Desktop & Dock | Minimize into application icon | `com.apple.dock` / `minimize-to-application`              | Boolean or absent                  |
| Desktop & Dock | Show recent applications       | `com.apple.dock` / `show-recents`                         | Boolean or absent                  |
| Desktop & Dock | Icon size                      | `com.apple.dock` / `tilesize`                             | Integer or real, 16–256, or absent |
| Keyboard       | Repeat interval                | global / `KeyRepeat`                                      | Integer, 0–10000, or absent        |
| Keyboard       | Initial repeat delay           | global / `InitialKeyRepeat`                               | Integer, 0–10000, or absent        |
| Keyboard       | Press-and-hold accent menu     | global / `ApplePressAndHoldEnabled`                       | Boolean or absent                  |
| Pointer        | Natural scrolling              | global / `com.apple.swipescrolldirection`                 | Boolean or absent                  |
| Pointer        | Mouse tracking speed           | global / `com.apple.mouse.scaling`                        | Real, −1–10, or absent             |
| Pointer        | Trackpad tracking speed        | global / `com.apple.trackpad.scaling`                     | Real, −1–10, or absent             |

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

Coverage & access inventories each setting as supported, needs permission, read-only, or unavailable.
Current implemented writable adapters require no microphone, Accessibility, Full Disk Access, or
administrator grant. **Supported means a readable valid value and implemented setter, not proof of
write permission.** Managed preferences are rejected by `CFPreferencesAppValueIsForced`; denied
synchronization, unsupported native types, and unvalidated OS versions are explained individually.
No permission is requested automatically. Recheck access performs fresh discovery, including after
manual permission/management changes. There is no fake grant button.

| Additional setting    | Current behavior                                                                                                     |
| --------------------- | -------------------------------------------------------------------------------------------------------------------- |
| Reduce motion         | Read-only effective value from `NSWorkspace.accessibilityDisplayShouldReduceMotion`; no baseline restoration         |
| Reduce transparency   | Read-only effective value from `NSWorkspace.accessibilityDisplayShouldReduceTransparency`; no baseline restoration   |
| Stage Manager         | Unavailable; no reliable public reversible setting API implemented                                                   |
| Display sleep timeout | Unavailable for capture/restore; administrator-managed power preferences, no privileged authorization adapter        |
| System idle sleep     | Unavailable for capture/restore; hardware/power-source-dependent power profiles, no privileged authorization adapter |
| Display brightness    | Unavailable; no implemented verified per-display adapter across Apple Silicon and external displays                  |

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
last native call cannot be ruled out. Operations have finite setting lists, no command execution,
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

The read-only native probe on macOS 15 discovered all 14 allowlisted capture/restore entries, the two
read-only accessibility queries, and four explicitly unavailable categories. It exercised capture to
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
