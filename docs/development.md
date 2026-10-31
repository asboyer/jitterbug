# Development and providers

## Setup

Follow the [README](../README.md#run-locally) for prerequisites and startup. `npm run dev` runs a
synthetic browser preview; `npm run tauri dev` runs the native collector and desktop app.
On macOS, install Xcode command-line tools. Windows needs MSVC build tools and WebView2.

The Svelte/TypeScript UI lives in `src/`. Rust in `src-tauri/src/` owns telemetry, attribution,
diagnostics, providers, policy, and actions. OS-specific code lives in `platform/`; `inside/`
handles app views, `queue/` schedules validation, and `control/` serves agent control.
`supervisor.rs` coordinates investigation, proposal, action, and verification.

## Providers and credentials

Configure decision, cheap, and intelligent tiers in **Settings**. Code-based routing is the default;
the optional decision model adds another request. Intelligent investigations default to Meta.
Ollama, Claude, and optional Jev adapters are also available. Local-only and cloud-only restrictions
are enforced before inference. Unavailable AI is labeled rather than presented as a model diagnosis.

Packaged apps store keys in the native credential store. For development:

```sh
cp .env.example .env
chmod 600 .env  # macOS / Unix
```

Edit `.env` privately. Debug builds load it; its keys take precedence over stored credentials.
Never prefix a secret with `VITE_`. Restrict file access to your user on Windows too.

Ollama is optional and installed separately. Jitterbug uses `127.0.0.1:11434`, never downloads models,
and guards inference using current memory, CPU, and thermal readings. Requests are serialized,
small, and unload the model afterwards. A tiny model is still fallible, and resource gates cannot
guarantee every model's peak memory usage.

Set the in-app cloud budget before using AI. The lower of it and `JITTERBUG_SPEND_LIMIT_USD` applies.
The prototype defaults to $2, reserves at most $0.10 per request, and has a 200-request lifetime
limit and $100 absolute ceiling. Reservations persist before transmission; failed requests or
missing usage retain their reservation. This is an estimated app ledger, not provider billing,
and excludes other clients using the same key.

AI can run for uncached process descriptions, investigations, explicit task summaries, or enabled
automatic incidents. General purpose descriptions are cached; current diagnoses are not reused as
app descriptions. Process actions are policy-controlled, never executed from model-generated text.
See [browser investigations](browser-investigations.md) and
[agent connections](app-and-agent-control.md) for evidence and privacy boundaries.

## Checks

```sh
npm run build
npm test
npx playwright install chromium --only-shell
npm run test:ui
npm run format:check
npm run smoke
```

The smoke check samples live telemetry without printing private measurements. UI tests use
synthetic fixtures without model calls or process actions. **Demo & testing** offers the same
kind of exploration in the desktop, including a delayed action-popup preview.

For native action tests, use disposable fixtures only. Specialized probes in `src-tauri/examples/`
cover process actions, browser inspection, agent control, provider calls, and caching. Read each
probe's options: some create load, perform actions, or incur charges. The
[Jitter lab](jitter-lab.md) documents opt-in load scenarios, not routine checks.

Keep snapshots, logs, screenshots, credentials, and machine-specific output in ignored `.local/`.
Persistent runtime data belongs in the OS application-data directory for `com.jitterbug.desktop`.
Use conventional commits and focused PRs. Follow the checked-in Prettier configuration and rustfmt.
