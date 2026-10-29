# Jitterbug

A small desktop resource assistant: see what is running, understand the evidence behind resource pressure, and choose what to do next.

## Run the desktop app

Install Node.js 22.12+ (or 24+), Rust 1.94+, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS. macOS needs Xcode command-line tools; Windows needs the MSVC build tools and WebView2.

```sh
npm ci
npm run tauri dev
```

The dashboard collects real telemetry every five seconds. Closing the window leaves monitoring running; choose **Quit Jitterbug** from the tray/menu-bar menu to stop it. No model is called automatically by monitoring or alerts.

```sh
npm run dev          # browser preview: synthetic data only
npm run build        # TypeScript/Svelte checks and frontend production build
npm test             # Rust unit tests
npm run smoke        # real collector smoke test; prints no private measurements
npm run format       # house-style formatting and rustfmt
npm run tauri build  # desktop package; unsigned unless signing is configured
```

## What is real

- Live process CPU, resident memory, parent relationships, available memory, and swap usage through `sysinfo`.
- Project grouping from working directories and project markers; app grouping from executable ancestry on macOS. Descriptions are deterministic inferences, not proof of task ownership.
- Sustained-pressure detection after three consecutive samples, with a five-minute alert cooldown. macOS adds native memory-pressure and thermal-state readings. Other platforms expose unavailable native sensors explicitly.
- User-requested contextual explanations using rules or configured models. Model telemetry uses anonymous workload aliases; names, paths, command lines, and transcripts are not included. Your chat messages are sent to the selected model, with basic path/credential redaction.
- Explicit lower-priority and termination-request actions for supported user-owned development processes on macOS. PID, start time, ownership, and protected processes are checked again before acting. Actions target a single process. Windows native process actions remain unimplemented.
- Provider settings, native credential storage, and a persistent conservative spending ledger.

## Models and credentials

The default policy is hybrid: a tiny local model for easy requests and Meta for intelligent analysis. Optional decision-model routing is off initially. Every tier can be configured independently; local-only and cloud-only are enforced in Rust before inference. If no eligible model can answer, the app labels its fallback as deterministic observations.

For development, copy `.env.example` to `.env`, set `META_API_KEY`, and restrict the file to your user (`chmod 600 .env` on Unix). `.env` is ignored and is loaded by debug builds only. Never prefix secrets with `VITE_`. Packaged apps use keys entered in **Providers & policy**, stored in the native credential store. Development environment keys take precedence.

Ollama is optional and accessed only at `127.0.0.1:11434`. Install a tiny model yourself if desired:

```sh
ollama pull qwen3.5:0.8b
# Optional decision tier:
ollama pull tev1:0.8b
```

Jitterbug never downloads models. Local inference requires at least 4 GiB and 20% memory available, CPU below 75%, fresh telemetry, and no reported resource or thermal pressure. Models larger than a 1.5 GiB download are excluded. Chat requests use a 2,048-token context, up to 256 output tokens, two CPU threads, and immediate model unloading. These are conservative gates, not a guarantee about every model's peak memory. Decision models use the System One endpoint and its own model context configuration.

Meta and Claude use fixed official endpoints. Jev is wired for the decision tier only and requires its own key; it is not required for the app. Cloud model IDs and prices are reviewed in `src-tauri/src/settings.rs`; unknown prices fail closed. Contributor/training-discount variants are not enabled.

Cloud calls are manual, bounded, and have no automatic HTTP retries. The default lifetime prototype budget is $2; the lower of the in-app budget and `JITTERBUG_SPEND_LIMIT_USD` applies. There is also a $0.10 reservation ceiling per call, 200-request lifetime prototype limit, and $100 absolute app limit. Conservative reservations are saved **before** requests and retained on failures or restarts. The displayed usage estimate is not a provider billing balance; external clients sharing a key are outside this app's ledger.

## What is a demo or future work

**Explore demo** and **Simulate pressure** use synthetic fixtures. They do not generate CPU/memory load, call models, or change processes. Browser preview is always a demo. The source badge and assistant message labels make this visible.

Agent-harness hooks, transcript correlation, GPU utilization, exact per-task ownership, model-fit recommendations, arbitrary job offloading, and automatic remediation are future work. There is no hidden implementation of these features. No process history or chat transcript is persisted; metric history is bounded to 120 in-memory samples.

## Structure

```text
src/                    Svelte + TypeScript desktop UI; synthetic demo fixtures
src-tauri/src/
  collector.rs          bounded telemetry and deterministic explanations
  attribution.rs        project/app evidence and grouping
  pressure.rs           sustained incidents and local-inference resource gates
  platform/             native OS readings (macOS first)
  actions.rs            explicit process-action validation
  providers.rs          Ollama, Meta, Claude, and Jev adapters; bounded routing
  settings.rs           inference modes, model tiers, credentials, reviewed prices
  budget.rs             locked, persistent pre-request spending reservations
  storage.rs            private local settings and ledger writes
  lib.rs                Tauri IPC, background sampler, tray lifecycle
```

Application settings and spending totals live in the OS application-data directory for `com.jitterbug.desktop`. Credentials stay in the native credential store or ignored development environment. Keep local evidence, logs, screenshots, and experiments in ignored `.local/`. Never commit real process snapshots, machine details, transcripts, or credentials to this public repository.
