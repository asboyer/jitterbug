# System settings baseline and rollback

**Implemented:** automatic macOS rollback for volume/mute, 19 saved Dock, keyboard/text, pointer
and Finder storage preferences, supported output nominal rate/full physical stream formats, full
display modes, and saved per-source sleep/display-sleep timers. One Rollback preview/confirmation
executes captured targets; there are no commands to copy. Power writes assume externally provided
OS authorization and report denial honestly. Permission UX/helper installation is a future branch.
Hardware current-value restoration does **not** prove reboot persistence.
This is not a full-system backup, OS downgrade, performance optimization, or a claim that writing a
preference immediately changes live behavior. Windows and other platforms remain explicitly
unsupported; no Windows baseline adapter was implemented or cross-compiled in this change.

## Exact capture/restore scope

Users can independently include Audio, Audio hardware, Displays, Power profiles, Desktop & Dock, Keyboard, Pointer, and Finder storage. Every Set Default
creates an immutable snapshot and keeps every earlier snapshot. Each snapshot includes a per-setting
coverage report, including exclusions. Existing v1 audio-only snapshots and separate legacy
provider-history files remain readable/preserved; restoring an old audio snapshot does not touch
newly supported preferences. Earlier hardware/power observations stored only in excluded coverage
reports are not promoted into writable targets; capture a new baseline to save typed hardware/power
values. Empty additive `hardware` fields are omitted, preserving old snapshot digests and history.

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
Audio/display/preference adapters request no microphone, Accessibility, or Full Disk Access grant.
Power timer writes require externally provided system authorization. **Needs permission** means an
implemented executable adapter, not unsupported; successfully captured power targets remain selectable. **Supported means a readable valid value and implemented setter, not proof of
write permission.** Managed preferences are rejected by `CFPreferencesAppValueIsForced`; denied
synchronization, unsupported native types, and unvalidated OS versions are explained individually.
No permission is requested automatically. Recheck access performs fresh discovery, including after
manual permission/management changes. There is no fake grant button.

| Additional setting             | Current behavior                                                                                                                            |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Reduce motion                  | Read-only effective value from `NSWorkspace.accessibilityDisplayShouldReduceMotion`; no baseline restoration                                |
| Reduce transparency            | Read-only effective value from `NSWorkspace.accessibilityDisplayShouldReduceTransparency`; no baseline restoration                          |
| Stage Manager                  | Unavailable; no reliable public reversible setting API implemented                                                                          |
| Display sleep timeout          | Automatic saved `displaysleep` restore per reported AC/Battery/UPS source; needs external authorization                                     |
| System idle sleep              | Automatic saved `sleep` restore per reported AC/Battery/UPS source; needs external authorization                                            |
| Display brightness             | Unavailable; no implemented verified per-display adapter across Apple Silicon and external displays                                         |
| Audio nominal rate/full format | Automatic supported nominal rate/full output ASBD restore; UID/stream identity, ranges and settable status revalidated; may interrupt audio |
| Display mode/refresh           | Automatic full-mode restore using UUID/topology and exact enumeration; requests permanent scope, verifies current modes only                |
| Low/High Power Mode            | Manual; hardware and power-source dependent, no authorized writer; undocumented numeric `powermode` values are not interpreted              |
| Login/background items         | Manual; ServiceManagement does not provide a general reversible adapter for other applications' registrations and approvals                 |
| Time Machine scheduling        | Manual: enable/disable commands exist, but no trustworthy captured enable-state getter validated; never infer from running status           |
| Pointer acceleration           | Manual, explicitly distinct from captured tracking speed; no validated separate acceleration adapter                                        |
| iCloud Optimize Mac Storage    | Manual account-managed policy; no eviction, download, or file deletion                                                                      |
| Microphone TCC grants          | Manual Privacy & Security approval; grants cannot be replayed; no discovery prompt or TCC database writes                                   |
| Spotlight exclusions           | Manual Search Privacy; no exact supported exclusion-list restore adapter, no indexing actions                                               |
| Developer Tools access         | Manual per-app privacy approval, not replayable; no security-disabling controls                                                             |

### Automatic hardware and power executors

**CoreAudio:** capture the current default output UID, optional settable nominal rate and the full
ASBD of each readable, settable physical output stream. Stream identity is output direction,
starting channel and terminal type under that exact UID; ambiguous channels fail closed. Available
nominal-rate ranges and full ranged ASBDs must support saved values. Runtime IDs come from fresh
inspection, never a persisted slot or re-resolving the default device at the setter. Busy audio fails
with guidance to stop playback/recording yourself. Changes can interrupt sound or require audio apps
to reopen the device; this impact is shown before confirmation.

Rate and physical formats form a **coupled restore target** because changing rate may change stream
ASBDs. The applying journal covers this finite sequence. A later failure leaves earlier native changes
possibly applied; no automatic undo/replay occurs. Success requires all captured format/rate values
to match fresh readback. Reconnect/reboot persistence is not verified. Nonsettable streams are not
restored. No input-device mutation, output-device selection, or stopping another application's audio.

**CoreGraphics:** capture online displays' stable UUIDs, full modes (ID, points/pixels, refresh, flags,
desktop usability), origins, rotation, active/main state and mirror relationships. Revalidate UUIDs
and topology, enumerate and retain exact supported mode objects, and submit changed modes in one
configuration transaction. Request `kCGConfigurePermanently`, not application-lifetime
`CGDisplaySetDisplayMode`; cancel uncommitted transactions on error. Report **current modes restored;
reboot persistence not verified**: macOS documents permanent-to-login-session fallback. Readback
must match all captured modes and unchanged topology. Automatic layout/mirror adjustments cause a
reported mismatch, never fabricated success. This is not an arrangement/rotation/mirroring/HDR/ICC/
brightness/VRR-policy restore. Screens may blank and windows move. Zero refresh stays unknown/variable.

**Power:** fixed `/usr/bin/pmset -g custom` discovery is bounded to two seconds/16 KiB. Capture only
reported `sleep` and `displaysleep`, separately for AC/Battery/UPS. Missing fields stay unknown; zero
means never. Restore uses fixed `/usr/bin/pmset`, `-c`/`-b`/`-u`, one allowlisted key, and a validated
decimal integer (0 through signed 32-bit maximum), via `Command::args`, never a shell. Each source/key
has its own journal outcome. Recheck the exact source/key/value before execution and read saved
profiles afterward. Exit failure/timeout stops further writes with an authorization/hardware-policy
error. No sudo, password, helper installation or interactive input. Do not run the GUI as root.
Externally granted execution privilege is an integration assumption, not something this branch grants
or bypasses. The manual documents per-system persistence; timers do not prove effective sleep.

**Other requested controls researched:** installed `tmutil(8)` documents `enable`/`disable` (root +
Full Disk Access), but not a trustworthy automatic-enable-state getter. Backup-running status is not
scheduling state; absent/private preferences are not guessed as enabled/disabled. No Time Machine
restore target is captured. The installed `pmset(1)` lacks a validated per-source Low/High Power policy
value contract; opaque `powermode` numbers are not interpreted. Apple's
[ProcessInfo.isLowPowerModeEnabled](https://developer.apple.com/documentation/foundation/processinfo/islowpowermodeenabled)
is explicitly get-only, reporting effective state rather than a reversible per-source policy. These
stay manual rather than inventing an API. No backups, security grants or daemons are manipulated.

### Excluded one-time actions

This baseline never offers purge, cache deletion, DNS flushing, quarantine removal, sleepimage deletion,
process termination, temporary renice, session keep-awake assertions, index rebuilds, bulk tweaks,
or security-disabling commands. Those are not persistent settings that this history could restore.
No generalized command runner or automatic optimization values were added.

Power timers with captured values are executable but need externally provided system authorization.
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
  nondeprecated full-mode accessors. The executor uses `CGBeginDisplayConfiguration`, `CGConfigureDisplayWithDisplayMode`,
  `CGCompleteDisplayConfiguration(..., kCGConfigurePermanently)`, and cancellation on pre-commit error.
- CoreAudio SDK `AudioHardwareBase.h` defines `kAudioDevicePropertyNominalSampleRate`,
  `kAudioDevicePropertyAvailableNominalSampleRates`, `kAudioDevicePropertyStreams`,
  `kAudioStreamPropertyPhysicalFormat`, and `kAudioStreamPropertyAvailablePhysicalFormats`.
  The executor uses `AudioObjectSetPropertyData` for nominal rate and physical ASBD after
  UID/stream identity, support, settable and busy checks.
- `pmset(1)` sections SETTING, GETTING, and FILES document root authorization, `-g custom`, per-source
  timers, and per-system preference persistence. No private power plist is written.
- `geteuid(2)` and `gethostuuid(3)` supply compatibility identity. User/machine/default-device compatibility uses locally keyed hashes. New private hardware payloads
  additionally store exact audio UID/stream descriptors and display UUID/topology for rebinding;
  those identities are not rendered in coverage or test logs.

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
last native call cannot be ruled out. Restore operations have finite typed setting lists and no arbitrary command execution or write retries.
Hardware readback polls at most 40 times with 50 ms waits; nominal-rate convergence has its own
two-second polling budget before physical formats. Synchronous native APIs have no cancellable
per-call deadline. Each fixed pmset writer has a three-second process deadline and null standard
input/output/error; cleanup kills/reaps only its owned child. The identity query has a five-second timeout.

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

Tests use injected adapters/setters and isolated temporary history/journals, never real host writes.
Controller tests cover automatic power/audio/display targets, selected scopes, legacy snapshots,
drift/identity mismatch, unsupported-target preflight, durable applying boundaries, partial failures,
readback errors, serialization, interrupted recovery, and bounded storage. The native executor is
exercised with inspected-handle binding, delayed readback, timeout, removal, and busy/denied setters.
Power tests cover allowlisted source/key arguments, injection rejection, zero, missing values and
authorization errors. UI tests use mocked IPC.

The opt-in read-only native probe validates discovered rate ranges/full ASBD support, enumerated
display modes, saved profile timers, capture to temporary storage, capability reporting and no-change
preview. It prints no hardware identities or values and invokes no native setter:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib real_macos_discovery_is_read_only -- --ignored --nocapture
```

Intentionally untested on this host: CoreAudio writes (volume/mute/rate/physical formats), CoreGraphics
configuration commit/fallback, pmset writes/real privilege grants, and CFPreferences writes/deletions.
Also unverified: reboot/reconnect persistence, audible output, post-change display usability, effective
sleep, actual busy/permission errors, and performance gains. Windows remains unsupported. No test
changes actual user OS settings or launches the GUI as root.
