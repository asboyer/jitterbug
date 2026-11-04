# Development and providers

## Setup

Follow the [README](../README.md#run-locally) for prerequisites and startup. `npm run dev` runs a
browser shell without system telemetry; `npm run tauri dev` runs the native collector and desktop app.
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
The default budget is $2, with at most $0.10 reserved per request, a 200-request rolling
24-hour limit, and a $100 absolute ceiling. The request window renews without clearing spending. Reservations persist before transmission; failed requests or
missing usage retain their reservation. This is an estimated app ledger, not provider billing,
and excludes other clients using the same key.

Monitor keeps system statistics local and does not request an automatic diagnosis when an app is
selected. Its inspector has a reusable one-sentence description and an explicit chat. Meta uses
`muse-spark-1.3` by default. Chat uses minimal reasoning, at most 1,024 output tokens, schema-constrained output,
fresh app/PID context, and optional reviewed priority/termination proposals; native identity and
permission checks remain authoritative. No model-generated command is executed.

Visible executable identities are enriched in batches of up to six, through the same serialized
inference queue. Foreground questions cancel active identity enrichment and skip queued descriptions;
background enrichment pauses while the inspector is open. Cancelled cloud requests retain their
reservation because their final usage is unknown. Identity research uses medium reasoning. Meta's
[Responses web search](https://dev.meta.ai/docs/search-grounding) researches public app identity
metadata only, with at most two tool calls, 3,072 output tokens, and a $0.10 ledger reservation.
Live telemetry, paths, command arguments, project names and chat text do not enter the search
request. Search responses use a strict JSON schema. Displayed source links must come from actual
retrieved results; progress messages are excluded from summaries. The usage ledger includes
reported tokens and estimated search charges. Other providers retain the non-search adapter.

Descriptions use a bounded 128-entry, 30-day on-device cache, invalidated by executable identity,
version and cache schema. Live diagnoses are never saved as descriptions. Inspector conversations
stay in memory, bounded to 12 contexts and 12 messages per context, with exact PID/start-time and
provider-settings keys. No investigation starts on selection or periodic telemetry updates.
The inspector collects at most 24 observations for each of up to 24 inspected identities; it does
not reconstruct activity before inspection. The native inventory is queried in pages of 40
(within the collector's 4,096-process and 512-group bounds). Each refresh preserves existing row
identities and updates readings; explicit sorting, search and refresh-order controls choose a new
order. Exited/reused PIDs cannot inherit the old selection or proposed action.
See [browser investigations](browser-investigations.md) and
[agent connections](app-and-agent-control.md) for evidence and privacy boundaries.

Browser resource questions collect supported tab/process evidence before inference and show the
largest measured processes as local, clickable rows. Firefox memory questions use fixed Mozilla
guidance (two-second fetch limit, 24-hour cache); tab titles and URLs are excluded from the model
context. On macOS, Firefox tab attribution is an optional, read-only DevTools connection on loopback port 6000.
Jitterbug never changes browser preferences or restarts it. The person must enable local remote
debugging, start Firefox with its debugger server, and choose **Show tabs**. Only `listTabs` and
`getTarget` are used, bounded by time and response size; no page scripts execute. Process IDs must
belong to the selected app. Shared-process tab memory is labeled estimated, and private/discarded
tabs are omitted. Until connected, Firefox's own **Shift+Esc** Process Manager is the immediate
alternative. The read-only connector exposes no tab-control buttons.

## Checks

```sh
npm run build
npm test
npx playwright install chromium --only-shell
npm run test:ui
# Use a separate port when another worktree is serving the app:
JITTERBUG_UI_PORT=1421 npm run test:ui
npm run format:check
npm run smoke
```

The smoke check samples live telemetry without printing private measurements. UI tests use
isolated synthetic fixtures without model calls or process actions. The product itself has no
demo mode, fake telemetry, canned chat replies, or simulated action results. Audit live behavior
in the native app using real, disposable workloads.

The optional `jitterbug-monitor-probe --live-provider` Cargo example validates researched identities
(including retained vendor sources) and an explicit PID conversation with two real Meta requests
using synthetic evidence. It respects saved privacy/provider settings and the shared spending
ledger, and caps additional reserved spend at $0.20. Its output contains synthetic descriptions,
a synthetic answer, elapsed milliseconds, pass/fail, and aggregate usage; it never collects real
processes. Add `--chat-only` to test just the foreground answer. On macOS, `jitterbug-icon-probe`
checks non-secret credential presence and OS artwork without a model call. Startup checks credential
attributes off the UI thread rather than reading passwords, so Keychain authorization cannot block
the Monitor window or its icon service.

For native action tests, use disposable fixtures only. Specialized probes in `src-tauri/examples/`
cover process actions, browser inspection, agent control, provider calls, and caching. Read each
probe's options: some create load, perform actions, or incur charges. The
[Jitter lab](jitter-lab.md) documents opt-in load scenarios, not routine checks.

Keep snapshots, logs, screenshots, credentials, and machine-specific output in ignored `.local/`.
Persistent runtime data belongs in the OS application-data directory for `com.jitterbug.desktop`.
Use conventional commits and focused PRs. Follow the checked-in Prettier configuration and rustfmt.
