# Browser slowdown investigations

A reported freeze is a symptom even when CPU is low. The assistant must investigate
the selected app instead of returning whole-machine totals or defining what a browser is.

## Evidence collected

- **Application ownership:** macOS WebKit XPC helpers are grouped through the OS
  responsibility relationship, with matching user and plausible process lifetime.
  Other applications' WebKit processes are not assigned to Safari by name.
- **Window responsiveness:** optional WindowServer queries report an app hang,
  no current hang flag, or an unavailable reading. Three consecutive samples are
  required by default. Low CPU is not an exclusion. App hangs can surface while
  an unrelated resource investigation or its 60-second gap is active; inference
  stays serialized. Snooze, recovery/cooldown, and hourly limits still apply.
- **Short stack checks:** explicit slowdown investigations sample the selected
  main process and at most one helper, sequentially, for one second at 10 ms
  intervals. Each command has a five-second wall-clock deadline and 2 MiB output
  limit. PID, start time, executable, and user are checked before and after.
  No privilege escalation, arbitrary commands, or periodic stack sampling is used.
- **Official guidance:** Safari slowdown investigations can check
  [Apple's troubleshooting article](https://support.apple.com/en-us/102564).
  Only this fixed URL is requested, with no redirects, a five-second deadline,
  and a 512 KiB response limit. Relevant sections are verified; a curated summary
  is supplied to the model. This is not a general web-search integration.
  Verified guidance is cached for 24 hours. Local-only mode never makes this
  network request; cached and offline references are labeled.

Raw stack reports use temporary files restricted to the current user and are
removed after completion, including ordinary failure/timeout paths. Only fixed
main-thread wait categories and summaries enter model context. Paths, addresses,
raw symbols, tab URLs, browser history, and page contents are not sent. A process
crash or abrupt machine shutdown can leave a temporary report in the private app
data directory; it must never be included in a public bug report.

The main-thread parser distinguishes ordinary event-loop idle waits from dominant
synchronous IPC, lock, sleep, and selected I/O patterns. A wait is not proof of a
deadlock, and an idle sample does not prove every page works. Sampling the busiest
helper is not the same as sampling every helper or identifying the affected tab.
The UI offers a focused whole-window/one-tab/recovered follow-up instead of a
list of unrelated remedies.

## Limits and platform support

The responsiveness and responsibility functions are optional, read-only private
macOS APIs resolved at runtime. Missing symbols or failed queries produce
unavailable evidence. This needs retesting on new macOS releases; it is not an
App Store compatibility promise. Windows retains resource-based detection and
reports stack/window checks unavailable. Windows UI-hang detection is not
implemented by this change.

WindowServer does not detect every stuck tab, network request, extension, or
brief freeze. Short samples may miss intermittent activity. Historical resource
readings and observed hangs are bounded to the app session. Jitterbug cannot
retroactively determine the exact cause of a freeze it did not capture.
No automatic priority proposal is made for an unresponsive app: lowering CPU
priority does not repair its blocked UI. Closing/force-quitting remains manual,
with possible loss of unsaved work.

## Failure visibility and budget accounting

Blocked or failed model requests show **AI unavailable** and the reason directly.
Local evidence is preserved; generic machine totals do not masquerade as a diagnosis.
The budget ceiling and 200-request prototype limit have separate errors. A lower
environment ceiling is identified rather than directing users only to Settings.

Each request reserves a conservative estimate before transmission. Complete
provider token usage settles only that request's reservation; missing usage,
timeouts, and legacy unknown charges remain reserved. This remains an estimate,
not a billing API balance. Existing privacy modes and all spending ceilings apply.

## Verification

Run `npm test`, `npm run test:ui`, `npm run build`, and Rust clippy. Regression
coverage includes low-CPU hang detection, unavailable readings breaking the
streak, concurrent resource alerts, snooze/hourly limits, no priority proposal,
main-thread sample classification and privacy, budget settlement/migration,
visible provider failures, scoped follow-ups, retained conversation history,
keyboard scrolling without scrollbar chrome, and small-window layout.

For native validation, use a disposable AppKit fixture whose button sleeps on its
main thread for 45 seconds and exits after 90 seconds. This simulates an unresponsive
window without burning CPU or allocating significant memory. Check that the app
appears as not responding, a sustained hang creates an incident even immediately
after a resource alert, and recovery clears the pop-up without applying a process
action. Separately verify one real provider response, the official-source/cache
label, stack-check evidence, and removal of temporary sample files. Keep all
screenshots, raw telemetry, and fixtures with local paths outside version control.

ABI references: [CGSInternal](https://github.com/NUIKit/CGSInternal/blob/master/CGSEvent.h)
and [Objective-See FileMonitor](https://github.com/objective-see/FileMonitor/blob/master/Library/Source/FileMonitor.m).
WebKit's [multiprocess architecture](https://docs.webkit.org/Deep%20Dive/Architecture/WebKit2.html)
explains why the main browser process alone is insufficient.

## Runtime origins

Opening a runtime’s details now inspects its actual entrypoint and current parent
chain. A Python process launching the Hermes gateway is described as that gateway,
with its log wrapper and macOS script launcher identified separately. Matching
requires both Hermes command syntax and an installation/working-folder match;
a folder alone does not prove which command is running. The gateway’s general
purpose comes from the [Hermes messaging documentation](https://hermes-agent.nousresearch.com/docs/user-guide/messaging).
Other Python and JavaScript processes expose their script or module entrypoint
locally. This does not recover a user’s historical request or read their scripts,
configuration, logs, messages, or transcripts.

Commands are read on demand for the selected process and its bounded parent chain,
with fresh PID/start-time/executable and same-user checks. Raw arguments are discarded.
Only parsed descriptions are cached in memory, for 30 seconds and at most 128
process identities. An AI investigation receives only recognized fixed job
categories; arbitrary script names, commands, paths and arguments remain local.
These live launch findings take precedence over a cached interpreter description.
Identity-only questions about a recognized command are answered directly from this
evidence without inference cost; resource diagnoses continue through the configured
investigation path.

When a short sample captures an interface-thread sleep, synchronization wait,
synchronous IPC or blocking I/O while macOS reports an unresponsive window, the
finding is explained directly without a model call. A normal event-loop wait is
not classified as a captured cause. A short sample is evidence about that interval,
not proof of what caused a slowdown earlier.
