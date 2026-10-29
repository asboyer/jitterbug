# Jitterbug

A small desktop resource assistant: see what is running, understand the evidence behind resource pressure, and choose what to do next.

## Run the desktop app

Install Node.js 22.12+ (or 24+), Rust 1.94+, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS. macOS needs Xcode command-line tools; Windows needs the MSVC build tools and WebView2.

```sh
npm ci
npm run tauri dev
```

The dashboard collects real telemetry every five seconds. Closing the window leaves monitoring running; choose **Quit Jitterbug** from the tray/menu-bar menu to stop it. Sustained pressure reveals the window and opens a contextual assistant; it does not automatically call a model. On macOS, reopening from the Dock also restores a hidden window.

```sh
npm run dev          # browser preview: synthetic data only
npm run build        # TypeScript/Svelte checks and frontend production build
npm test             # Rust unit tests
npm run smoke        # real collector smoke test; prints no private measurements
npm run format       # house-style formatting and rustfmt
npm run tauri build  # desktop package; unsigned unless signing is configured
```

## What is real

- Live process CPU, resident memory, disk read/write counters, parent relationships, available memory, and swap usage through `sysinfo`. App/workload history retains the last 24 samples (about two minutes).
- Project grouping from working directories and project markers; app grouping from executable ancestry on macOS. Descriptions are deterministic inferences, not proof of task ownership.
- Sustained-pressure detection after three consecutive samples, with a five-minute alert cooldown. macOS adds native memory-pressure and thermal-state readings. Other platforms expose unavailable native sensors explicitly.
- Ask about an app by name or a process by PID: “Why is Spotify using so much CPU?” resolves the target locally, supplies recent workload history and up to eight busy processes to the intelligent tier, and displays the measured evidence beside its diagnosis. Follow-ups retain context; ambiguous names prompt a choice. General summaries use anonymous aliases. Specific investigations include the selected app/process names; project labels, paths, command lines, environment variables, unrelated app names, and transcripts are excluded from telemetry sent to models. Your chat messages are sent with basic path/credential redaction.
- Confirm actions directly from investigation evidence: lower CPU priority for supported user-owned development processes and third-party app processes on macOS; request termination for supported development processes or an app’s main executable. App helpers offer priority changes only. PID, start time, ownership, and protected processes are rechecked before acting. Afterward, a fresh sample shows before/after CPU and memory, or that the identity is no longer observed. A measured change does not prove causation. Actions target a single process; Windows native actions remain unimplemented.
- Provider settings, native credential storage, and a persistent conservative spending ledger.

## Models and credentials

The default policy is hybrid: a tiny local model for clearly bounded definitions and Meta for intelligent analysis. Optional decision-model routing is off initially. Every tier can be configured independently; local-only and cloud-only are enforced in Rust before inference. Simple requests can escalate once to the intelligent tier. Process investigations bypass the optional decision model and go to the intelligent tier. Diagnoses never silently downgrade to the cheap tier. Explicit cheap-tier diagnosis requests return observations and explain how to select the intelligent tier. Capability questions such as “Can I ask about a particular process?” are answered locally, without an inference call. If no eligible model can answer, the app labels its fallback as deterministic observations. Tiny models can still make factual mistakes; their output is advisory.

For development, copy `.env.example` to `.env`, set `META_API_KEY`, and restrict the file to your user (`chmod 600 .env` on Unix). `.env` is ignored and is loaded by debug builds only. Never prefix secrets with `VITE_`. Packaged apps use keys entered in **Providers & policy**, stored in the native credential store. Development environment keys take precedence.

Ollama is optional and accessed only at `127.0.0.1:11434`. Install a tiny model yourself if desired:

```sh
ollama pull qwen3.5:0.8b
# Optional decision tier:
ollama pull tev1:0.8b
```

Jitterbug never downloads models. Local inference requires at least 4 GiB and 20% memory available, CPU below 75%, fresh telemetry, and no reported resource or thermal pressure. Models larger than a 1.5 GiB download are excluded. Chat requests use a 2,048-token context, up to 256 output tokens, two CPU threads, and immediate model unloading. These are conservative gates, not a guarantee about every model's peak memory. Decision models use the System One endpoint and its own model context configuration. Tev is an optional local alternative to Jev; code-based routing has the lowest overhead and remains the default.

Meta and Claude use fixed official endpoints. Jev is wired for the decision tier only and requires its own key; it is not required for the app. Cloud model IDs and prices are reviewed in `src-tauri/src/settings.rs`; unknown prices fail closed. Contributor/training-discount variants are not enabled.

Cloud calls are manual, bounded, and have no automatic HTTP retries. The default lifetime prototype budget is $2; the lower of the in-app budget and `JITTERBUG_SPEND_LIMIT_USD` applies. There is also a $0.10 reservation ceiling per call, 200-request lifetime prototype limit, and $100 absolute app limit. Conservative reservations are saved **before** requests and retained on failures or restarts. The displayed usage estimate is not a provider billing balance; external clients sharing a key are outside this app's ledger.

## What is a demo or future work

**Explore demo** and **Simulate pressure** use synthetic fixtures. In the desktop app, pressure simulation feeds three synthetic samples through the real Rust detector and delivers its incident through the real event channel. It does not generate CPU/memory load, call models, or change processes. Browser preview is always a demo. The source badge and assistant message labels make this visible.

Agent-harness hooks, transcript correlation, GPU utilization, exact per-task ownership, model-fit recommendations, arbitrary job offloading, and automatic remediation are future work. There is no hidden implementation of these features. No process history or chat transcript is persisted. Global history is bounded to 120 samples; per-workload history to 24 samples for up to 512 workloads. The internal process inventory retains up to 4,096 processes; dashboard updates show the top 40 workloads and 160 processes. Name/PID resolution uses the wider internal inventory.

Diagnosis is evidence-based troubleshooting, not a profiler: Jitterbug cannot inspect an app’s settings, call stacks, GPU activity, or network traffic yet. It can identify the busy process and resource trend, propose a hypothesis and a test, and perform the supported process actions after confirmation. Changing Spotify settings, clearing caches, restarting apps, and other app-specific repairs remain manual suggestions, never falsely reported as completed.

## Structure

```text
src/                    Svelte + TypeScript desktop UI; synthetic demo fixtures
src-tauri/src/
  collector.rs          bounded telemetry and deterministic explanations
  attribution.rs        project/app evidence and grouping
  diagnostics.rs        app/PID resolution and scoped investigation evidence
  pressure.rs           sustained incidents and local-inference resource gates
  platform/             native OS adapters (macOS readings; explicit Windows gaps)
  actions.rs            explicit process-action validation
  providers.rs          Ollama, Meta, Claude, and Jev adapters; bounded routing
  settings.rs           inference modes, model tiers, credentials, reviewed prices
  budget.rs             locked, persistent pre-request spending reservations
  storage.rs            private local settings and ledger writes
  lib.rs                Tauri IPC, background sampler, tray lifecycle
```

Application settings and spending totals live in the OS application-data directory for `com.jitterbug.desktop`. Credentials stay in the native credential store or ignored development environment. Keep local evidence, logs, screenshots, and experiments in ignored `.local/`. Never commit real process snapshots, machine details, transcripts, or credentials to this public repository.

## Verification and feedback loop

CI checks formatting, TypeScript/Svelte, Rust unit tests, Clippy, and native compilation on macOS and Windows. No API credentials or machine telemetry are used in CI. Windows interactive behavior still needs a teammate's hands-on check; Windows process actions and native pressure/thermal sensors are not implemented.

On macOS, the native desktop feedback loop has been exercised with live collection, workload filtering, simulated sustained pressure, a real Meta answer, provider-mode persistence, a zero-budget cloud denial, and a confirmation-driven stop of a disposable worker. The worker was observed in telemetry and disappeared after the action. Separate live probes exercise priority changes and stale process identity rejection.

```sh
# Safe by default: sample real telemetry, without logging private values.
npm run smoke

# Build opt-in verification tools (not included in the desktop bundle).
cargo build --manifest-path src-tauri/Cargo.toml --examples

# macOS only: creates one disposable worker, observes its real CPU/RAM,
# lowers its priority, then terminates only that worker. Repeats with an app-bundle fixture.
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-probe -- --actions

# Uses a synthetic credential in a separate test service, then deletes it.
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-probe -- --credential-store

# Optional real inference checks: require the configured model or credential.
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-probe -- --local-model
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-probe -- --decision-model
cargo run --manifest-path src-tauri/Cargo.toml --example jitterbug-probe -- --meta-model
```

The action fixture allocates 32 MiB only when at least 4 GiB is available, produces a three-second single-thread CPU pulse, and exits within 90 seconds even without an action. Model probes retain responses only in ignored `.local/verification/` and use a separate $0.25 conservative lifetime ledger. Local Qwen and Tev routing and Meta generation have been exercised against real endpoints. Claude and hosted Jev adapters have protocol tests but have not been tested with live credentials. Jev remains optional and no Jev request is made without an explicit configuration and key.

To repeat the UI check, launch the desktop app, confirm **LIVE**, filter/select a workload, to open its assistant. Ask “Can I ask you about a particular process?”, then name an app or PID. Verify the resolved target, model tier, history, and process evidence. For actions, use only the disposable worker; confirm an action and check the follow-up measurement. Use **Simulate pressure** to exercise the alert and mocked chat without stressing the host, then **Return to live**. In Providers, save each privacy mode and verify excluded tiers return observations with a routing explanation. Set the budget to zero and verify a cloud request is blocked without increasing the ledger; restore the intended mode and budget afterward.

## Next increments

Add one opt-in harness integration that supplies a session identifier, project, and root PID. Keep transcript parsing local and attach task ownership only when that evidence exists. Then implement Windows-specific sensors and process actions, measure release-build overhead across representative machines, and evaluate small-model accuracy before broadening local responsibilities. Offloading requires explicit job/runtime integrations; an arbitrary running process cannot simply be moved to the cloud.

Provider references: [Meta Chat Completions](https://dev.meta.ai/docs/protocols/chat-completions), [Meta pricing](https://dev.meta.ai/docs/pricing-rate-limits), [Claude pricing](https://platform.claude.com/docs/en/about-claude/pricing), [Ollama Chat API](https://docs.ollama.com/api/chat), [Tev](https://ollama.com/library/tev1:0.8b), and [Jev System One](https://docs.typesafe.ai/introduction/quickstart). Provider account settings determine retention and any free-credit data terms; Jitterbug does not change those settings.
