# Jitterbug

A small desktop resource assistant: see what is running, understand the evidence behind resource pressure, and choose what to do next.

## Run the desktop app

Install Node.js 22.12+ (or 24+), Rust 1.94+, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS. macOS needs Xcode command-line tools; Windows needs the MSVC build tools and WebView2.

```sh
npm ci
npm run tauri dev
```

The dashboard collects real telemetry every five seconds. Closing the window leaves monitoring running; choose **Quit Jitterbug** from the tray/menu-bar menu to stop it. Sustained pressure or unusual workload activity reveals the window and opens the automatic assistant. It investigates through the configured intelligent tier, proposes a supported optimization, and measures its result after approval or an existing workload permission. On macOS, reopening from the Dock also restores a hidden window.

```sh
npm run dev          # browser preview: synthetic data only
npm run build        # TypeScript/Svelte checks and frontend production build
npm test             # Rust unit and process-level integration tests
npm run smoke        # real collector smoke test; prints no private measurements
npm run format       # house-style formatting and rustfmt
npm run tauri build  # desktop package; unsigned unless signing is configured
```

## Desktop interface and native readings

The macOS menu bar uses a monochrome template image: macOS supplies white or dark tint to
match the menu bar. Click it for a compact quick view; right-click for Open, Settings, and Quit.
Use **Window → Quick View** (⇧⌘M on macOS) for keyboard access, and ⌘, for Settings.
The panel uses native macOS popover vibrancy, closes on Escape or focus loss, and opens the
selected workload directly in its detail view. The shared Svelte interface uses system
fonts, light/dark appearance, compact controls, and native window chrome. It remains a Tauri
webview interface, not a rewrite in AppKit.

The quick view reuses the same five-second Rust sampler. Its webview is created only on first
use and receives at most five workloads with no process inventory. Hidden/minimized windows
stop receiving telemetry events; reopening delivers the latest sample. Monitoring and the
policy-controlled incident loop continue independently.

On macOS, **Core activity & memory details** adds per-core usage from the existing CPU poll,
plus app, wired, compressed, and cached memory via `host_statistics64`. Swap-in/out values are
page-counter deltas for the last sample interval, not allocated swap size or a claim about any
particular process. Missing/first samples are unavailable; counter resets establish a new
baseline. macOS available memory is total minus native used memory (app + wired + compressed),
including reclaimable cache. The same conservative headroom feeds local-inference guards;
`sysinfo`'s macOS non-compressed-memory estimate includes active app pages and is not used
as allocation headroom. CPU values elsewhere still represent total machine capacity; each core bar is a
percentage of that one logical core. Native memory detail is unavailable on Windows.

Memory accounting is adapted from [Stats](https://github.com/exelban/stats) under its MIT license;
see [third-party notices](THIRD_PARTY_NOTICES.md), also included in the app's Settings.
The adapter uses the actual kernel page size and makes no shell calls. Existing cached
`sysinfo` process collection is retained; no Stats private SMC APIs, privileged helpers, or
repeated `top` subprocesses are imported.

## Application icons and process origins

**Manage this workload** turns an observed finite command or a project check into a reviewed
validation profile. Preview the command, memory estimate, and optional Claude/Codex/Muse MCP
changes before approving it. Run, inspect, or revoke profiles from the desktop. See the
[shared validation setup guide](docs/validation-queue.md#from-the-desktop).

Workloads show artwork from the installed app: `NSWorkspace` on macOS and the Windows shell
on Windows. Jitterbug rasterizes small PNGs on demand, caches at most 128 icons in memory,
and limits frontend requests to two at a time. No icon search, download, or model call occurs.
Projects and inaccessible executables use a generic fallback. The menu-bar icon stays monochrome.

Click a workload for a dedicated detail page, then select a member process to inspect its
parent chain, executable, working directory, start time, and recent CPU, memory, and disk use.
Details use the full internal inventory, display up to 200 members, and trace at most 16 ancestors.
Parent start times and process identity are checked; missing, reused, and cyclic parents stop
the trace. This is a current OS parent chain, not a historical launch log: a parent such as
`launchd` does not prove whether the app came from the Dock, a login item, or a coding agent.
**Investigate this process** opens the contextual assistant for that PID. Actions keep their
existing confirmation and fresh-identity checks.

Common macOS apps, including Spotify, have built-in descriptions matched to their bundle
identifier. Opening any other process with an observed executable automatically requests a
short explanation from the intelligent tier, subject to mode, budget, and local-resource guards.
This includes background services, framework executables, and project runtimes. macOS framework
and XPC metadata supply component context instead of relying only on a filename. The detail
page shows the installed component and observed parent beside the explanation, with an origin
evidence link. General runtime descriptions do not identify the particular script or agent task.

Only the executable/app name, component name and type, bundle identifier, OS, and installation
category enter this request. Full paths, command arguments, parent chains, files, snapshots,
and transcripts stay local. AI descriptions are general reference knowledge, not cached
diagnoses of current activity. A failed or blocked request shows a reason and an explicit retry;
telemetry refreshes do not repeatedly call the model. Demo views never request explanations.

AI descriptions persist privately in `app-profiles.json` in the OS application-data directory.
The cache holds at most 128 entries for 30 days and is invalidated by app version or installation
changes. Reopening a detail page reuses the description without inference. Icons and origins
remain local; raw paths are omitted from normal telemetry, incident serialization, action reports,
and model context. Demo origins are entirely synthetic and cannot call a provider or act on a process.

## Inside an app

A workload's detail page can split an app into the things you can actually act on.

- **Browser tabs** (Chrome, Edge, Brave, Chromium, Vivaldi): choose **Show tabs** to measure each
  tab for one second and see its CPU, memory, and host, plus what the browser itself, the GPU
  process, and extensions use. **Show** brings a tab forward; **Close tab…** needs a second click
  and first checks that the tab still shows the same address. This needs a one-time browser
  setting: open `chrome://inspect/#remote-debugging` and allow remote debugging for the running
  browser. The browser asks once to allow Jitterbug's connection, which stays local (loopback
  only) and is kept open so you are not asked again. Turn the setting off to revoke access.
- **How tab memory is found:** DevTools lists tabs but never says which renderer process hosts
  each one. Each tab does report its renderer's cumulative CPU time, and the browser reports the
  same counter per PID. Jitterbug reads both twice, a second apart and bracketed in time, so a
  tab maps to exactly one renderer. Memory is that renderer's resident memory from the normal
  sample. When several tabs share one renderer, the split is an estimate (JavaScript heap and
  main-thread time) and is marked with ≈. Sleeping/discarded tabs have no renderer.
- **Terminals, editors, and coding-agent apps** (Terminal, iTerm2, Ghostty, VS Code, Cursor,
  Zed, Claude, JetBrains IDEs, …): each command started inside the app, such as an agent session,
  a dev server, or a test run, becomes one row with its whole process subtree, working folder,
  and busiest child. Sessions registered with `jitterbug-run` show their task. Rows reuse the
  five-second inventory, so no extra sampling happens. Each row can be **paused**, as well as
  the existing process actions with the same identity checks and confirmation.
- **Pause, not quit:** pausing freezes a command and everything it started, such as an agent
  session with its test runs. It uses no CPU until you choose **Resume**. Nothing is closed,
  and an agent picks up where it was. Memory stays allocated, but it stops growing, and macOS
  can compress or swap it for your other sessions. Pauses resume by themselves after 15
  minutes, when Jitterbug quits, and on the next launch after a crash. A small detached timer
  also resumes them shortly after the 15 minutes if Jitterbug was killed and not started
  again. A plain stop signal would make the terminal's shell report the command as suspended
  and take the terminal away from it. For a command that owns its terminal, Jitterbug stops
  that launching shell first and continues it last, so the command keeps its terminal; a
  background job is paused alone, so the prompt stays usable. Only your own commands can be
  paused; apps (a frozen window cannot redraw), jobs you already suspended, and anything
  containing Jitterbug itself are refused. This was exercised
  with a real coding-agent CLI session paused for 20 seconds in the middle of a tool call. It used
  no CPU while paused, then finished its task.
- **Incidents offer the smallest fix:** when an app stays busy or memory runs short, the
  out-of-app popup offers the most specific action before quitting anything:
    - **Close the heaviest tab**, for a browser you already connected with **Show tabs**. A
      background alert never opens a browser connection by itself.
    - **Pause the heaviest command session** inside a terminal or editor. It is offered when that
      session carries at least 30% of the app's use, or when it launched the busy project.
    - Otherwise, the existing stop.

    Every option still needs your explicit click and is never applied automatically.

Titles, addresses, and folders stay on this computer except the tab or session name shown in
an incident. No page content, history, or cookies are read.

```sh
# Opt-in check against an isolated headless Chrome profile: one busy and two idle tabs.
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-inside-probe
# Commands inside running terminal/editor apps, by command name only.
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-inside-probe -- --sessions
# Pause one command's tree for 20 seconds, confirm it used no CPU, then resume it.
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-pause-probe -- --pid <pid> --seconds 20
```

## Agent control over MCP

An agent can see what each app is running and ask Jitterbug to act on one part of it.
For example, an orchestrating agent can find which of five coding-agent sessions in your
editor's terminals is using the memory, then pause just that session instead of quitting
the editor. Connect any MCP client to the stdio server:

```sh
cargo build --manifest-path src-tauri/Cargo.toml --bin jitterbug-run
claude mcp add jitterbug-control -- /absolute/path/to/jitterbug-run control-mcp
```

| Tool                       | What it does                                                                   |
| -------------------------- | ------------------------------------------------------------------------------ |
| `jitterbug_overview`       | Machine pressure, the busiest apps, and the command sessions inside each one   |
| `jitterbug_inspect_app`    | One app's sessions, or a browser's tabs measured over one second               |
| `jitterbug_pause_session`  | Ask to pause a session and everything it started (reversible; 15-minute limit) |
| `jitterbug_resume_session` | Resume a session that an agent's request paused                                |
| `jitterbug_close_tab`      | Ask to close one browser tab                                                   |
| `jitterbug_request_status` | Whether a request is waiting for you, applied, or declined, with the result    |

- **Off until you turn it on.** Any program running as you can reach the control socket, so
  agent control starts disabled. Turn on **Settings → Let connected agents see what each app
  runs** to use it.
- **You stay in charge.** Every pause or close is a request that appears in the popup (and in
  the app) with the agent's session and its stated reason. Nothing happens until you approve.
  **Settings → Agents may pause sessions without asking** lets reversible pauses apply at
  once, except while you have paused actions from the popup. Closing a tab always asks. Agents
  can resume only what an agent's request paused, never a session you paused. Turning off
  agent control refuses every request.
- **An agent can never pause itself.** Its caller's process ID comes from the kernel, not
  from the request, and its own session is marked `is_your_session`.
- **Bounded:** one open request per target and at most six agent requests an hour, so a
  looping agent cannot crowd out real incidents.
- **Local and private:** the server holds no state. Each call goes over an owner-only Unix
  socket in the app-data folder to the running app, which checks that the caller is your user.
  Results go to the agent's model provider. They include app labels, session titles and
  folder names, tab titles, and hosts. They never include full paths, URLs, command lines,
  or page content.
- **Exercised end to end:** a coding-agent CLI session ran a single-core busy loop in a separate
  Terminal window. A second coding agent, connected only through this MCP server, picked
  that session from the overview (12.6% of the machine) and asked to pause it. After the
  approval, the session used no CPU. When resumed, it finished its task.

Agent control is macOS and Linux only for now; Windows reports it as unavailable.

```sh
# Opt-in headless server for checks without the desktop UI (--approve stands in for your click).
JITTERBUG_CONTROL_SOCKET=/absolute/short/path.sock \
    cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-control-probe -- --approve
```

## What is real

- Live process CPU, resident memory, disk read/write counters, parent relationships, available memory, and swap usage through `sysinfo`. App/workload history retains the last 24 samples (about two minutes).
- Project grouping from working directories and project markers; app grouping from executable ancestry on macOS. Descriptions are deterministic inferences, not proof of task ownership.
- Sustained-pressure and workload anomaly detection with consecutive samples, with configurable thresholds, recovery, cooldown, and rate limits. App CPU alerts require at least 10% of total machine capacity by default; an increase from idle alone does not trigger a pop-up. This minimum is configurable in Automatic assistance and also applies to anomaly alerts. macOS adds native memory-pressure and thermal-state readings. Other platforms expose unavailable native sensors explicitly.
- Ask about an app by name or a process by PID: “Why is Spotify using so much CPU?” resolves the target locally, supplies recent workload history and up to eight busy processes to the intelligent tier, and displays the measured evidence beside its diagnosis. Follow-ups retain context; ambiguous names prompt a choice. General summaries use anonymous aliases. Specific investigations include the selected app/process names; project labels, paths, command lines, environment variables, unrelated app names, and transcripts are excluded from telemetry sent to models. Your chat messages are sent with basic path/credential redaction.
- Confirm actions directly from investigation evidence: lower CPU priority for supported user-owned development processes and third-party app processes on macOS; request termination for supported development processes or an app’s main executable. App helpers offer priority changes only. PID, start time, ownership, and protected processes are rechecked before acting. Afterward, a fresh sample shows before/after CPU and memory, or that the identity is no longer observed. A measured change does not prove causation. Actions target a single process; Windows native actions remain unimplemented.
- Provider settings, native credential storage, and a persistent conservative spending ledger.

## Automatic assistance

The native loop runs while the window is hidden:

1. **Detect:** CPU and thermal alerts require three consecutive five-second samples by default. Defaults: 85% whole-machine CPU, or app CPU above both 10% of total capacity and one logical core. CPU anomalies also require 3× the recent baseline and a five-percentage-point rise, with at least nine history samples. App memory growth alone never triggers an investigation or pop-up. Memory alerts require OS warning/critical pressure, swap growth above 256 MiB per sample, or (when native pressure is unavailable) available memory below both the configured percentage (12% by default) and 2 GiB. Normal memory warnings must last at least 30 seconds; critical OS memory pressure keeps a 10-second minimum. The configured sample count also applies. Growth of at least 50% and 512 MiB helps select an app to investigate during an existing memory-pressure episode; it does not establish a leak or causation.
2. **Investigate:** show the popup and ask the intelligent tier once, using the selected workload's real processes and recent history. Mode, local resource guards, and the spending ceiling still apply. Disable automatic model analysis to use observations only; unavailable models produce an explicitly labeled rules fallback.
3. **Propose:** deterministic policy can propose lowering a busy, eligible process's CPU priority. Model text is advisory and cannot execute commands. Memory incidents receive diagnosis without an irrelevant priority change.
4. **Apply:** choose **Apply once & measure**, or **Allow for this workload & apply**. The latter saves permission for future priority changes in that workload. No workloads are allowed initially. Automatic termination is never allowed. Current trigger, PID/start time, ownership, protection, and permission are rechecked immediately before the OS action.
5. **Verify:** wait for a complete fresh sampling interval, compare resources, and read back OS scheduling priority. The result explicitly reports when CPU did not improve or evidence is unavailable. Lower priority helps contention; it does not cap CPU, free RAM, or guarantee a speedup. It lasts until process exit and cannot always be reversed without privileges.

Use **Settings** to change limits, disable model analysis or automation, and remove permissions. The popup can dismiss an incident or pause new investigations/actions for 15 minutes. Closing it leaves monitoring active. Dismissal cannot cancel an action already applied. Repeated alerts require recovery and a cooldown (five minutes by default). Memory alerts share one machine-wide episode, require 60 seconds of observed recovery, and have a cooldown of at least 15 minutes. Sampling gaps and pausing do not erase cooldowns or count as recovery. Finished memory prompts close when fresh telemetry shows pressure has cleared; their findings remain in Inbox. All alerts have a global 60-second gap and at most four investigations per hour per session. Only one model request and one measured action run at a time. Up to 12 incidents, with 12 steps each, stay in memory and disappear on quit. Existing lifetime spending reservations survive restarts.

Automatic process changes currently work on **macOS only**. Windows can detect, investigate, and display evidence, but has no native action adapter yet. This is workload reprioritization, not moving arbitrary running work between machines.

## Jitter Snapshot

Felt a hitch? Press **⌘⇧J** (Ctrl+Shift+J on Windows), choose **Jitter Snapshot** from the
menu-bar menu, or use the quick view button. Jitterbug records the app in front (the
**victim**), keeps sampling for 10 seconds, then freezes the window from 60 seconds before
to 10 seconds after the press. The case shows a one-second timeline with your press and the
suspect window (−15 s to +10 s against a −60 s to −15 s baseline), then **What happened**,
**Likely cause**, **What you can do**, and **If it happens again**.

- **Signals:** a separate one-second pulse sampler reads per-core CPU, CPU time for every
  process your user can read (macOS; elsewhere the 16 busiest from the last full sample),
  keeping the 16 busiest each second, system swap/compressor/page-in counters, OS memory
  pressure, thermal state, and how late a 10 ms sleep wakes up (scheduler delay). It keeps
  120 samples in memory only. The five-second collector is unchanged. GPU, display, disk,
  and network activity are not measured, and processes owned by other users have no
  per-process CPU without privileges. Because every process is read each second, a hog
  that starts a few seconds before your press, or exits just before it, is still named. A
  process that starts after the last five-second sample is rated over its own lifetime on
  its first tick.
- **Findings** are deterministic, labeled observed or inferred, and rated strong, possible,
  or unexplained: a process spike (at least 3× baseline and over 25% of one core), one
  saturated core even when total CPU is low, memory paging with rising pressure, thermal
  escalation, scheduler delay with no visible cause, or nothing found.
- **Victim vs culprit:** actions target culprits only. If the app in front was itself busy,
  the case says it was limited by its own work. A command running _inside_ the app in front,
  such as a script in an editor's terminal or a coding-agent session, is a separate suspect:
  the case names it ("claude in Visual Studio Code") and its busiest process, not the editor.
  A command is grouped from the outermost non-shell process below the app, so an agent
  session and the tools it runs count together, and the investigation looks at its busiest
  process. Folder names in these labels stay local. An app in front that was itself started
  from a terminal without an app bundle stays the victim. CPU culprits can get **Lower priority**;
  strong memory findings can offer **Stop process** only where actions are already
  permitted. Priority is never suggested as a memory fix. The model only explains the
  findings and cannot add actions. Its request includes the victim and culprit app names and
  the bounded findings; project labels, paths, and PIDs stay local.
- **Always your approval:** snapshot and short-hitch cases never auto-apply, even with a
  saved workload permission. A manual snapshot bypasses the automatic cooldown and hourly
  cap and runs even when automatic assistance is off or paused; one-request-at-a-time,
  privacy modes, and spending limits still apply. Verification is an OS priority readback;
  current CPU says nothing about a past spike.
- **Recurrence:** a private tally in the app-data directory (`jitter-tally.json`) keeps up
  to 50 workload names with counts, no paths. When a workload appears in three of your last
  six snapshots, its review offers the existing per-workload lower-priority permission.
  Clear it in **Settings**.
- **Modes (Settings → Automatic assistance):** **Auto** (default) samples continuously and
  also opens a **Short hitch** case when scheduler delay lasts three seconds, or a saturated
  core coincides with delay, subject to the same pause, global gap, per-reason cooldown, and
  hourly cap as sustained incidents. **Snapshot only** runs no background sampler; a press
  records the next 15 seconds and compares against five-second workload history.

Measured cost of Auto mode on an Apple silicon Mac with about 830 processes (60 s, release
build): about 0.3% of one core (up to 0.4% while other work keeps the machine busy), +0.7 MiB
resident, about two wakeups per second (tick plus probe), and 3 to 4 ms of work per tick. Reading every process each second costs more than
the earlier 16-process sampler (0.06%) but stays inside the 0.5% budget; names and parents
are read once per process launch, and unreadable processes are retried every 30 seconds. While sampling, Jitterbug holds a macOS activity assertion so
App Nap timer coalescing is not misread as scheduler delay; this keeps the process from
napping (idle system sleep is still allowed). Snapshot only avoids that cost. The global
shortcut is registered from Rust; the webview has no shortcut permission. Another app that
already owns ⌘⇧J keeps it, and the menu-bar and quick-view triggers still work.

See the [jitter lab](docs/jitter-lab.md) for reproducible loads and the headless probe.

## Registered agent workflows

Heavy validation can now use a [shared validation queue](docs/validation-queue.md):
one resource-gated slot across sessions, local MCP tools for Claude/Codex/Muse Code,
fixed approved commands, source-change checks, and a dashboard with competing-workload
suggestions. The original task wrapper below still provides attribution without scheduling.

Use the opt-in `jitterbug-run` wrapper to associate parallel agent sessions and their child processes with one task. Keep the desktop app running while the task runs. The **Agent workflows** panel shows current and sampled peak CPU/memory, individual sessions, worktree locations, observed tool roles, and evidence for efficiency suggestions. Attribution and recommendations use local rules and require no model calls.

```sh
cargo build --manifest-path src-tauri/Cargo.toml --bin jitterbug-run

# Run independently in each worktree, using the same task ID and distinct agent labels.
./src-tauri/target/debug/jitterbug-run \
    --task-id EXAMPLE-1 --task-title "Improve a test workflow" \
    --agent implementation --harness claude --capture-context \
    --worktree /absolute/path/to/worktree \
    -- claude

# Also associate a separate validation run with the task.
./src-tauri/target/debug/jitterbug-run \
    --task-id EXAMPLE-1 --task-title "Improve a test workflow" \
    --agent validation --harness shell --role test --worktree /absolute/path/to/worktree \
    -- npm test
```

The wrapper preserves stdin/stdout/stderr and command exit status. By default it stores only bounded private launch metadata. `--capture-context` opts a Claude Code launch into **session-only hooks**, without changing user or project settings. Do not combine it with an existing `--settings` argument; merge the generated hook settings yourself in that case. Hooks keep the first request and latest follow-up (up to 600 characters each), a harness session identifier, last reported activity, and counts of completed tool categories. They never store command arguments, code, tool output, or transcripts. No transcript is opened. Common path/key patterns are redacted, but arbitrary prose may still contain private information; excerpts stay local by default. `--task-description "Build a search screen"` supplies intent for any harness without hooks. A matching wrapper PID and OS start time, plus a live parent chain, establish ownership. Task titles are supplied by the caller; captured request excerpts supply the intended assignment. A shared directory alone never proves ownership, and detached or reparented children lose attribution. Existing application groups and action permissions remain independent of task registration.

The task card leads with **What this task is building**, **Where the resources went**, and an **Efficiency tip**. Tool activity comes from harness reports; CPU and memory come from verified process ancestry. Completed tasks retain the role breakdown from the same sample as their peak CPU for the current app session. A process exiting successfully does not prove a feature is complete. Older registrations without intent show their launcher title and explicitly report missing context; there is no invented reconstruction.

**Summarize task with AI** is optional and sends only the displayed request excerpts (up to four sessions) to the configured intelligent provider. The button discloses that this can be cloud. No task IDs, session IDs, worktree paths, command lines, tool output, or process inventory are added to that request. Normal chat and automatic incident calls still exclude all task excerpts. The AI rewrite describes intended work only; measured resource explanations and tips remain local rules. Rewrites are cached in memory for the app session, reused while the request set is unchanged, and invalidated on follow-ups. Mode, one-request-at-a-time, local-headroom, and persistent spending controls apply. A failed request leaves the local explanation usable. No local model is started automatically for this feature.

### Other harnesses and hook setup

`jitterbug-run hook-settings` prints hook JSON using this launcher's installed absolute path. Merge its `hooks` entries into your harness settings rather than overwriting existing configuration. The bridge accepts the common `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, and `SessionEnd` inputs documented by [Claude Code](https://code.claude.com/docs/en/hooks) and [Codex](https://learn.chatgpt.com/docs/hooks). Codex requires reviewing and trusting custom hooks through `/hooks`. The generated output contains a local executable path: keep it out of public repositories.

Launch the harness through `jitterbug-run` so hooks inherit `JITTERBUG_RUN_ID` and the registry location. Each event must run beneath the exact live wrapper PID/start-time. The first harness session binds the context; nested/different sessions cannot replace its task request. Invalid, oversized (over 64 KiB), or unowned events are skipped without blocking the harness or injecting model instructions. Counts describe received tool events and can miss skipped events. Only the original request and latest follow-up are retained, not conversation history.

**Integration boundary:** automatic hook injection is implemented and exercised for Claude Code CLI. Codex-compatible event parsing is tested, but an existing desktop/shared-daemon session is not necessarily a wrapper descendant and is deliberately not attributed. Installing hooks does not establish per-chat resource ownership in a shared server. Automatic desktop chat discovery and per-thread server attribution remain future work; use explicit descriptions for currently registered validation runs. Hook activity is context, not a profiler or proof of duplicate work.

Advice covers overlapping validation, worker fan-out, memory headroom, and separating agent assignments before one integrated full check. Tool roles are inferred from a small allowlist; overlap does not prove duplicate tests. Suggestions are advisory and marked as current or observed earlier. This wrapper does not schedule agents, cap their resources, change configuration, or offload jobs. Use the underlying harness's own budget and permission controls.

Launch records live under `com.jitterbug.desktop/agent-runs` in the OS application-data directory. `JITTERBUG_AGENT_RUNS_DIR` can select an absolute private directory for isolated development/testing; the app and launcher must use the same directory. Reads are bounded to 128 files, tracking to 64 sessions and 16 tasks, and each task history to 24 samples. Intent/activity metadata shares the same bounded 8 KiB registration and expires with it. AI task summaries are memory-only. Completed records expire after a day; measured peaks and advice remain in memory for the app session. Sampling may miss short commands and peaks; summed RSS may double-count shared pages. The Quick View excludes this detail. Cloud context receives only anonymous aggregate counts and metrics, never registered task labels, worktree paths, session identifiers, or request excerpts. The explicit task-summary button is the only exception for the previewed excerpts.

The harness feedback loop has been exercised with a real Claude Code CLI session creating a small helper and tests: request capture, tool-category events, live ancestry, completion, and native task-card rendering. Two bounded concurrent Node test runs exercised retained overlap advice. A real Meta task rewrite and cache restoration were verified through the native UI without a second request on reopening. Integration tests launch real descendant hook processes and reject events from outside the registered tree. Windows compilation/tests run in CI; interactive hook installation on Windows and Codex desktop integration remain unverified.

## Models and credentials

The default policy is hybrid: a tiny local model for clearly bounded definitions and Meta for intelligent analysis. Optional decision-model routing is off initially. Every tier can be configured independently; local-only and cloud-only are enforced in Rust before inference. Simple requests can escalate once to the intelligent tier. Process investigations bypass the optional decision model and go to the intelligent tier. Diagnoses never silently downgrade to the cheap tier. Explicit cheap-tier diagnosis requests return observations and explain how to select the intelligent tier. Capability questions such as “Can I ask about a particular process?” are answered locally, without an inference call. If no eligible model can answer, the app labels its fallback as deterministic observations. Tiny models can still make factual mistakes; their output is advisory.

For development, copy `.env.example` to `.env`, set `META_API_KEY`, and restrict the file to your user (`chmod 600 .env` on Unix). `.env` is ignored and is loaded by debug builds only. Never prefix secrets with `VITE_`. Packaged apps use keys entered in **Settings**, stored in the native credential store. Development environment keys take precedence.

Ollama is optional and accessed only at `127.0.0.1:11434`. Install a tiny model yourself if desired:

```sh
ollama pull qwen3.5:0.8b
# Optional decision tier:
ollama pull tev1:0.8b
```

Jitterbug never downloads models. Local inference requires at least 4 GiB and 20% memory available, CPU below 75%, fresh telemetry, and no reported resource or thermal pressure. Models larger than a 1.5 GiB download are excluded. Chat requests use a 2,048-token context, up to 256 output tokens, two CPU threads, and immediate model unloading. These are conservative gates, not a guarantee about every model's peak memory. Decision models use the System One endpoint and its own model context configuration. Tev is an optional local alternative to Jev; code-based routing has the lowest overhead and remains the default.

Meta and Claude use fixed official endpoints. Jev is wired for the decision tier only and requires its own key; it is not required for the app. Cloud model IDs and prices are reviewed in `src-tauri/src/settings.rs`; unknown prices fail closed. Contributor/training-discount variants are not enabled.

Cloud calls come from activity checks, chat requests, opening uncached process details, explicitly requested task summaries, or enabled automatic incident investigations. They are bounded and have no automatic HTTP retries. The default lifetime prototype budget is $2; the lower of the in-app budget and `JITTERBUG_SPEND_LIMIT_USD` applies. There is also a $0.10 reservation ceiling per call, 200-request lifetime prototype limit, and $100 absolute app limit. Conservative reservations are saved **before** requests. Successful calls with complete token usage settle their own reservation to the estimated charge; failures and unknown usage keep the reservation across restarts. Legacy reservations remain conservative. The displayed usage estimate is not a provider billing balance; external clients sharing a key are outside this app's ledger.

## System settings baseline and rollback

**Set Default** captures macOS audio and saved desktop/input preferences before optimization.
Choose Audio, Desktop & Dock, Keyboard, Pointer, and Finder storage categories. The typed allowlist covers master
volume/mute plus 19 saved preferences: Dock auto-hide, magnification, launch animation, minimizing
into the application icon, recent apps, icon size; keyboard repeat interval/delay and accent menu;
natural scrolling and mouse/trackpad tracking speed (not acceleration); spelling correction,
capitalization, period/dash/quote substitution; and .DS_Store creation preferences on network/USB
volumes. Existing files are never deleted. Preference storage support is limited to macOS 13–15.
Each immutable capture retains prior history and includes per-setting coverage. Windows and other
platforms remain unsupported.

Coverage & access distinguishes supported, read-only, manual, and unavailable settings and offers explicit
System Settings links and Recheck access. Reduce motion/transparency are read-only public queries;
CoreAudio nominal rate/full physical stream formats and CoreGraphics full display modes are observed
with supported-range/mode and stable-identity checks, but **not restorable**: driver writability does
not prove persistence, and display mode APIs can be application/session scoped. Saved power-source
sleep/display-sleep timers are read using only `pmset -g custom`; no root writer is installed.
Low/high power, login/background items, Time Machine scheduling, pointer acceleration, iCloud storage,
microphone privacy, Spotlight exclusions, and Developer Tools access have precise manual coverage.
Stage Manager and brightness are not falsely presented as restorable.
There is no automatic permission request, password handling, sudo, or privileged helper.

Audio is verified through live CoreAudio readback. Preferences are verified in saved CFPreferences
storage only; live effects may require app relaunch, sign-out, or restart by the user. Jitterbug never
restarts Dock/SystemUIServer. Absence of an override is preserved, not replaced with a guessed default.
History and coverage are shown in bounded, keyboard-accessible views.

Rollback starts with an exact before/after preview and explicit confirmation. Its one-use token binds
the selected snapshot to fresh actual values and expires after five minutes. Compatibility is limited
to the same OS version, effective user, machine, and default output device. Jitterbug rechecks actual
state before each typed native write, records a durable intent first, verifies native readback after
each write, and stops on external drift or partial failure. Interrupted journals are reported and
never replayed automatically. Capture and rollback share the supervisor action lock with automation.

This is not an OS upgrade/downgrade, disk backup, provider-configuration history, output-device
switcher, or performance claim. It never changes credentials, privacy/security/network controls,
processes, or arbitrary preferences. All one-time actions are excluded: purge/cache deletion, DNS
flush, quarantine/sleepimage removal, process kills, temporary renice, session assertions, and
security-disabling commands. Legacy provider-history files remain untouched in their old
namespace but are no longer shown in Settings. See
[system settings rollback](docs/system-settings-rollback.md) for native API scope and verification.

## What is a demo or future work

**Explore demo** and **Simulate pressure** use synthetic fixtures. In the desktop app, pressure simulation feeds three synthetic samples through the real Rust detector and delivers its incident through the real event channel. It does not generate CPU/memory load, call models, or change processes. Exploring the demo does not pause the native monitor; a real incident switches the desktop UI back to LIVE. Browser preview is always a demo. The source badge and assistant message labels make this visible.

Automatic discovery of unregistered harnesses, transcript correlation, GPU utilization, ownership of unregistered or detached work, model-fit recommendations, arbitrary job offloading, CPU caps, and app-specific automatic repairs are future work. There is no hidden implementation of these features. No process history or chat transcript is persisted. Global history is bounded to 120 samples; per-workload history to 24 samples for up to 512 workloads. The internal process inventory retains up to 4,096 processes; dashboard updates show the top 40 workloads and 160 processes. Name/PID resolution uses the wider internal inventory.

Reported freezes trigger a bounded macOS stack check of the selected process/application and at most one helper. Only fixed main-thread wait categories reach the model; raw samples stay local and are removed after the check. macOS window responsiveness and WebKit ownership also inform investigations. These checks can distinguish a sampled wait from CPU activity, but cannot reconstruct a past freeze or identify every stalled tab. App settings, GPU activity, tab contents, and network traffic are not inspected. See [browser investigations](docs/browser-investigations.md) for coverage, privacy, and verification. Changing Spotify settings, clearing caches, restarting apps, and other app-specific repairs remain manual suggestions, never falsely reported as completed.

## Structure

```text
src/                    Svelte + TypeScript desktop UI; synthetic demo fixtures
src-tauri/src/
  collector.rs          bounded telemetry and deterministic explanations
  attribution.rs        project/app evidence and grouping
  workflows.rs          private launch registration, verified task ownership, efficiency advice
  harness.rs            bounded opt-in request/activity events; no transcript reading
  workflow_summary.rs   plain-English intent, resource explanations, and efficiency tips
  queue/                shared validation admission, private profiles/results, stdio MCP
  bin/jitterbug-run.rs   opt-in wrapper and harness hook event bridge
  diagnostics.rs        app/PID resolution and scoped investigation evidence
  inside/               per-tab (browser DevTools) and per-command (process tree) app views
  pause.rs              pause/resume a command tree without its shell noticing; expiry, recovery
  control/              agent control: owner-only socket server, request handlers, stdio MCP
  details.rs            local process details and bounded, identity-checked parent chains
  applications.rs       installed app identity and bounded native icon cache
  profiles.rs           built-in app reference and private, versioned AI description cache
  pressure.rs           sustained incidents and local-inference resource gates
  platform/             native OS adapters (macOS readings; explicit Windows gaps)
  automation.rs         sustained thresholds, anomaly detection, permissions, verification
  pulse.rs              one-second in-memory sampler and scheduler-delay probe
  jitter.rs             deterministic Jitter Snapshot findings, victim/culprit, recurrence tally
  capture.rs            global shortcut, snapshot triggers, and hand-off to the runtime
  supervisor.rs         background investigation → proposal → action → measurement
  actions.rs            native process-action validation and OS priority verification
  providers.rs          Ollama, Meta, Claude, and Jev adapters; bounded routing
  settings.rs           inference modes, model tiers, credentials, reviewed prices
  settings_history.rs   preserved legacy provider-history backend
  system_settings.rs    native baseline history, journals, guarded audio/preference rollback
  budget.rs             locked, persistent pre-request spending reservations
  storage.rs            private local settings and ledger writes
  desktop.rs            template tray icon, native quick-view window, visibility-aware events
  popup.rs              out-of-app approval popup: display decision, non-activating window
  lib.rs                Tauri IPC, background sampler, lifecycle
```

Application settings and spending totals live in the OS application-data directory for `com.jitterbug.desktop`. Credentials stay in the native credential store or ignored development environment. Keep local evidence, logs, screenshots, and experiments in ignored `.local/`. Never commit real process snapshots, machine details, transcripts, or credentials to this public repository.

## Verification and feedback loop

CI checks formatting, TypeScript/Svelte, Rust unit tests, Clippy, and native compilation on macOS and Windows. No API credentials or machine telemetry are used in CI. Windows interactive behavior still needs a teammate's hands-on check; Windows process actions and native pressure/thermal sensors are not implemented.

On macOS, the native desktop feedback loop has been exercised with live collection, workload filtering, simulated sustained pressure, a real Meta answer, provider-mode persistence, a zero-budget cloud denial, and a confirmation-driven stop of a disposable worker. The worker was observed in telemetry and disappeared after the action. The automatic workflow has also been exercised with a bounded real app fixture: anomaly detection, automatic Meta investigation, saved workload permission, priority application, and fresh verification. The displayed result correctly reported no clear CPU reduction. Pause/resume and permission removal were checked through the desktop UI. A separate native probe exercises the complete automatic flow with an existing permission, rules-only investigation, and no cloud calls. The macOS UI refresh was checked in the packaged desktop app: native memory detail, Quick View keyboard access, workload-to-assistant navigation, Escape dismissal, focus-loss dismissal, and Settings access. The collector probe now checks native VM accounting, interval swap counters, and local process details without printing private values. Application icons and details were checked against real macOS applications, including Spotify helper-to-parent tracing, scoped assistant handoff, and a Meta-generated app explanation reused from the local cache. Windows icon extraction is compiled in CI; interactive Windows checks remain outstanding.

```sh
# Safe by default: sample real telemetry, without logging private values.
npm run smoke

# Build opt-in verification tools (not included in the desktop bundle).
cargo build --manifest-path src-tauri/Cargo.toml --examples

# macOS only: creates one disposable worker, observes its real CPU/RAM,
# lowers its priority, then terminates only that worker. Repeats with an app-bundle fixture.
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-probe -- --actions

# macOS only: runs the real supervisor against an owned 32 MiB fixture,
# observes a sustained CPU anomaly, automatically lowers its allowed priority,
# then verifies OS priority and a fresh sample. No model/API requests.
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-autopilot-probe

# Opt-in: explain a live executable with the saved intelligent provider, then prove
# cached reuse makes no extra request. Uses the app ledger and at most $0.05 more
# reserved spend; no process actions. Output is private local verification data.
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-profile-probe -- --process NAME

# Uses a synthetic credential in a separate test service, then deletes it.
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-probe -- --credential-store

# Optional real inference checks: require the configured model or credential.
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-probe -- --local-model
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-probe -- --decision-model
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-probe -- --meta-model
```

The automation fixture first waits 35 seconds for a baseline, then uses a 70% duty cycle on one thread, rechecks memory/thermal limits, and exits within 115 seconds. Its probe scopes the real inventory to its own child and retains private evidence only in ignored `.local/verification/automation/`.

The action fixture allocates 32 MiB only when at least 4 GiB is available, produces a three-second single-thread CPU pulse, and exits within 90 seconds even without an action. Model probes retain responses only in ignored `.local/verification/` and use a separate $0.25 conservative lifetime ledger. Local Qwen and Tev routing and Meta generation have been exercised against real endpoints. Claude and hosted Jev adapters have protocol tests but have not been tested with live credentials. Jev remains optional and no Jev request is made without an explicit configuration and key.

To repeat the UI check, launch the desktop app, confirm **LIVE**, filter/select a workload to open its details. Select a member, verify its parent chain, and choose **Investigate this process** to open its assistant. Ask “Can I ask you about a particular process?”, then name an app or PID. Verify the resolved target, model tier, history, and process evidence. For actions, use only the disposable worker; confirm an action and check the follow-up measurement. Use **Simulate pressure** to exercise the alert and mocked chat without stressing the host, then **Return to live**. In Settings, save each privacy mode and verify excluded tiers return observations with a routing explanation. Set the budget to zero and verify a cloud request is blocked without increasing the ledger; restore the intended mode and budget afterward.

## Next increments

Extend cooperative queue integration with harness-specific enforcement and verify desktop-agent setup end to end. Keep any transcript parsing local and preserve identity checks. Implement Windows-specific sensors and process actions, measure release-build overhead across representative machines, and evaluate small-model accuracy before broadening local responsibilities. Offloading requires explicit job/runtime integrations; an arbitrary running process cannot simply be moved to the cloud.

Provider references: [Meta Chat Completions](https://dev.meta.ai/docs/protocols/chat-completions), [Meta pricing](https://dev.meta.ai/docs/pricing-rate-limits), [Claude pricing](https://platform.claude.com/docs/en/about-claude/pricing), [Ollama Chat API](https://docs.ollama.com/api/chat), [Tev](https://ollama.com/library/tev1:0.8b), and [Jev System One](https://docs.typesafe.ai/introduction/quickstart). Provider account settings determine retention and any free-credit data terms; Jitterbug does not change those settings.

### Interface regression checks

The opening **Workflows** hub brings approved checks, cooperative checkpoint sessions,
and observed agent tasks together. Hold a check, prioritize the next one, protect
foreground work for 20 minutes, or request an AI scheduling plan. See the
[controls and checkpoint protocol](docs/validation-queue.md#workflow-hub).
The **Activity** view focuses on computer health and current resource users.
Use the sidebar for the full **Workloads** list, **Agent tasks**, **Validation queue**,
and the **Inbox** of proposed optimizations and incident history. Open a task to see
its request, suggestions, resource measurements, and session evidence. Disclosures
stay in the state you choose during live refreshes. Completed task records are green;
that status does not independently prove the requested product change was correct.

The incident assistant leads with the observed change, proposed action, and result.
Full model output, routing, measurements, and action logs remain expandable. New
incidents do not replace an open workload, setup dialog, chat, or specialist view.

Run the isolated UI scenarios with synthetic fixtures (no models or process actions):

```sh
npx playwright install chromium --only-shell
npm run test:ui
```

These checks cover navigation, task and session disclosures across telemetry refreshes,
completed status, simulated pressure, disabled demo actions, and the minimum window
width. Desktop visual checks also exercise the packaged WebKit UI and real telemetry.
Screenshots and test output stay ignored. **Demo & testing** in the app provides the
same synthetic scenarios without generating resource load or inference requests.

**Take a tour** in the sidebar starts a six-step overlay using synthetic examples.
It covers Activity, Workloads, reviewing a suggestion, agent tasks, the queue, and
settings. Back, Next, Skip, and Escape work without making model calls or applying
process changes. Exiting restores the previous page/workload and live/demo mode.
The first-use invitation can be dismissed; only that local UI preference is saved.

Incident cards keep explanations behind the **ⓘ** button and **View details**.
**Review change** shows the effects and an unchecked option to allow future automatic
priority changes. Opening a review grants no permission. Confirmation still passes
through the existing native process-identity, pressure, and permission checks.
Synthetic reviews can be explored, but their apply controls stay disabled.

Agent tasks now lead with visible cards: the intended task, recognized resource consumers, and an
applicable next step. Recent completed tasks have no active-management prompt; tasks without samples
show one empty-state sentence instead of empty charts. View task opens a persistent detail view with
measurements, original requests, and sessions available on demand.

New sustained incidents surface a compact “Hey, I noticed…” prompt across the main views without
replacing the selected task. The native window is shown and requests attention when unfocused;
Later leaves the incident in Inbox and suppresses repeats for that case. The existing sustained
sample, cooldown, snooze, and hourly-budget rules still control detection.

When a suggestion needs approval while you are in another app, a small **action popup** appears at
the top right of the display under the pointer, on every Space and over full-screen apps, without
taking focus. It shows the same title, observation, and proposed change as the in-app review, with
**Apply once & measure**, **Allow for this workload & apply**, **Details**, and **Later**. Buttons
call the existing approval commands, so every identity, ownership, trigger, and permission recheck
still applies. After approval it follows progress and verification, then hides about 8 seconds after
the result (pointing at it keeps it open); Escape acts as Later. Only one popup shows at a time: the
newest request, with a “+N more” count. It stays hidden while the main window is focused, for cases
you chose Later on (in the popup or in-app), while automation is paused, and when **Settings →
Show action popups over other apps** is off. The payload carries no paths, PIDs, or evidence.
**Demo & testing → Preview action popup** shows a synthetic request after 5 seconds, giving you time
to switch apps; its apply flow is played locally and nothing is changed. The browser preview renders
the same component at `?view=popup`.

In the demo, **See Jitterbug step in** exercises the real native monitor with three in-memory synthetic
samples (the browser preview uses fixtures). Investigation and proposal stages are explicitly
simulated; no model or real process action runs. **Preview managed checks** illustrates two future
submitted checks taking turns. It does not pause existing tests or promise measured resource savings.

### Process insights

Workload search covers the full collected inventory, including quiet processes outside the dashboard.
Opening a workload automatically identifies its executable and caches a short purpose explanation.
The process page keeps that reusable explanation separate from **Check activity**, which uses a fresh
sample and recent history to return a conclusion, explanation and one next step inline. Measurements,
parent PIDs, provider information and action controls are available behind disclosures.

An already-selected workload resolves duplicate application names locally, without a model call or a
second picker. An explicitly named different process still changes the target; genuinely ambiguous
unselected names still need a choice. Model responses use a bounded structured format; invalid or
overlong answers fall back to measured observations. Explanations use model knowledge and installed
metadata, not automatic web research. Current activity is never reused as a cached purpose statement.
