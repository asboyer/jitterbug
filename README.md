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
identifier. Other identified applications offer **Generate app explanation**, using the configured
intelligent tier only on request, subject to mode, budget, and local-resource guards. Only the
app name and bundle identifier (when available) enter that request; paths, process snapshots,
and transcripts do not. The response is labeled AI-generated and is general app knowledge,
not a cached diagnosis of current activity. Unrecognized executables and project workloads
retain explicit rule-based explanations; the contextual assistant can still investigate them.

AI descriptions persist privately in `app-profiles.json` in the OS application-data directory.
The cache holds at most 128 entries for 30 days and is invalidated by app version or installation
changes. Reopening a detail page reuses the description without inference. Icons and origins
remain local; raw paths are omitted from normal telemetry, incident serialization, action reports,
and model context. Demo origins are entirely synthetic and cannot call a provider or act on a process.

## What is real

- Live process CPU, resident memory, disk read/write counters, parent relationships, available memory, and swap usage through `sysinfo`. App/workload history retains the last 24 samples (about two minutes).
- Project grouping from working directories and project markers; app grouping from executable ancestry on macOS. Descriptions are deterministic inferences, not proof of task ownership.
- Sustained-pressure and workload anomaly detection after three consecutive samples, with configurable thresholds, recovery, cooldown, and rate limits. macOS adds native memory-pressure and thermal-state readings. Other platforms expose unavailable native sensors explicitly.
- Ask about an app by name or a process by PID: “Why is Spotify using so much CPU?” resolves the target locally, supplies recent workload history and up to eight busy processes to the intelligent tier, and displays the measured evidence beside its diagnosis. Follow-ups retain context; ambiguous names prompt a choice. General summaries use anonymous aliases. Specific investigations include the selected app/process names; project labels, paths, command lines, environment variables, unrelated app names, and transcripts are excluded from telemetry sent to models. Your chat messages are sent with basic path/credential redaction.
- Confirm actions directly from investigation evidence: lower CPU priority for supported user-owned development processes and third-party app processes on macOS; request termination for supported development processes or an app’s main executable. App helpers offer priority changes only. PID, start time, ownership, and protected processes are rechecked before acting. Afterward, a fresh sample shows before/after CPU and memory, or that the identity is no longer observed. A measured change does not prove causation. Actions target a single process; Windows native actions remain unimplemented.
- Provider settings, native credential storage, and a persistent conservative spending ledger.

## Automatic assistance

The native loop runs while the window is hidden:

1. **Detect:** three consecutive five-second samples above a configured limit. Defaults: 85% whole-machine CPU, less than 12% available memory, OS memory/thermal pressure, or one core of app CPU. A workload can also trigger after CPU rises at least 3× its baseline and 25% of one core, or memory grows at least 50% and 512 MiB. Baselines need at least nine samples. High activity is evidence to investigate, not proof of a fault.
2. **Investigate:** show the popup and ask the intelligent tier once, using the selected workload's real processes and recent history. Mode, local resource guards, and the spending ceiling still apply. Disable automatic model analysis to use observations only; unavailable models produce an explicitly labeled rules fallback.
3. **Propose:** deterministic policy can propose lowering a busy, eligible process's CPU priority. Model text is advisory and cannot execute commands. Memory incidents receive diagnosis without an irrelevant priority change.
4. **Apply:** choose **Apply once & measure**, or **Allow for this workload & apply**. The latter saves permission for future priority changes in that workload. No workloads are allowed initially. Automatic termination is never allowed. Current trigger, PID/start time, ownership, protection, and permission are rechecked immediately before the OS action.
5. **Verify:** wait for a complete fresh sampling interval, compare resources, and read back OS scheduling priority. The result explicitly reports when CPU did not improve or evidence is unavailable. Lower priority helps contention; it does not cap CPU, free RAM, or guarantee a speedup. It lasts until process exit and cannot always be reversed without privileges.

Use **Settings** to change limits, disable model analysis or automation, and remove permissions. The popup can dismiss an incident or pause new investigations/actions for 15 minutes. Closing it leaves monitoring active. Dismissal cannot cancel an action already applied. Repeated alerts require recovery and a cooldown (five minutes by default), with a global 60-second gap and at most four investigations per hour per session. Only one model request and one measured action run at a time. Up to 12 incidents, with 12 steps each, stay in memory and disappear on quit. Existing lifetime spending reservations survive restarts.

Automatic process changes currently work on **macOS only**. Windows can detect, investigate, and display evidence, but has no native action adapter yet. This is workload reprioritization, not moving arbitrary running work between machines.

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

Cloud calls come from chat requests, explicitly requested app/task explanations, or enabled automatic incident investigations. They are bounded and have no automatic HTTP retries. The default lifetime prototype budget is $2; the lower of the in-app budget and `JITTERBUG_SPEND_LIMIT_USD` applies. There is also a $0.10 reservation ceiling per call, 200-request lifetime prototype limit, and $100 absolute app limit. Conservative reservations are saved **before** requests and retained on failures or restarts. The displayed usage estimate is not a provider billing balance; external clients sharing a key are outside this app's ledger.

## What is a demo or future work

**Explore demo** and **Simulate pressure** use synthetic fixtures. In the desktop app, pressure simulation feeds three synthetic samples through the real Rust detector and delivers its incident through the real event channel. It does not generate CPU/memory load, call models, or change processes. Exploring the demo does not pause the native monitor; a real incident switches the desktop UI back to LIVE. Browser preview is always a demo. The source badge and assistant message labels make this visible.

Automatic discovery of unregistered harnesses, transcript correlation, GPU utilization, ownership of unregistered or detached work, model-fit recommendations, arbitrary job offloading, CPU caps, and app-specific automatic repairs are future work. There is no hidden implementation of these features. No process history or chat transcript is persisted. Global history is bounded to 120 samples; per-workload history to 24 samples for up to 512 workloads. The internal process inventory retains up to 4,096 processes; dashboard updates show the top 40 workloads and 160 processes. Name/PID resolution uses the wider internal inventory.

Diagnosis is evidence-based troubleshooting, not a profiler: Jitterbug cannot inspect an app’s settings, call stacks, GPU activity, or network traffic yet. It can identify the busy process and resource trend, propose a hypothesis and a test, and perform the supported process actions after confirmation. Changing Spotify settings, clearing caches, restarting apps, and other app-specific repairs remain manual suggestions, never falsely reported as completed.

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
  details.rs            local process details and bounded, identity-checked parent chains
  applications.rs       installed app identity and bounded native icon cache
  profiles.rs           built-in app reference and private, versioned AI description cache
  pressure.rs           sustained incidents and local-inference resource gates
  platform/             native OS adapters (macOS readings; explicit Windows gaps)
  automation.rs         sustained thresholds, anomaly detection, permissions, verification
  supervisor.rs         background investigation → proposal → action → measurement
  actions.rs            native process-action validation and OS priority verification
  providers.rs          Ollama, Meta, Claude, and Jev adapters; bounded routing
  settings.rs           inference modes, model tiers, credentials, reviewed prices
  budget.rs             locked, persistent pre-request spending reservations
  storage.rs            private local settings and ledger writes
  desktop.rs            template tray icon, native quick-view window, visibility-aware events
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
