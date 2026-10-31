# Jitterbug

Get more out of your hardware while running parallel agents, local models, and everyday apps.
Jitterbug explains resource use, investigates slowdowns, and schedules approved checks around you.

Built with **Tauri, Rust, Svelte, and TypeScript**. Early-stage; macOS has the fullest support.
Windows has live telemetry, with gaps in native diagnostics and actions.

## What it does

- **Understand workloads:** app icons, process origins, resource history, and cached explanations.
- **Catch a slowdown:** press **⌘⇧J** on Mac or **Ctrl+Shift+J** on Windows for a Jitter Snapshot.
- **Act on the cause:** review an optimization, pause a command session, or inspect supported
  Chromium tabs. Pausing retains memory; it does not close the command.
- **Coordinate agents:** learn check costs and share spare capacity across sessions, with room for your apps.
- **Save system settings:** capture a Mac settings baseline and preview a rollback.

Rules handle monitoring and scheduling. AI explains unfamiliar processes and investigates incidents
through interchangeable local/cloud providers. Hybrid, local-only, and cloud-only modes are available.
Process actions are checked in Rust; a model cannot execute arbitrary commands. Automatic changes
require explicit saved permission, and closing apps or tabs always needs approval.

## Run locally

Install **Node.js 22.12+**, **Rust 1.94+**, and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```sh
npm ci
npm run tauri dev
```

Add a provider key in **Settings** for AI investigations. Ollama is optional; Jitterbug never downloads
models. Development keys can go in an ignored `.env` copied from `.env.example`. Never use a `VITE_`
prefix for secrets. Set a cloud budget before enabling inference; the ledger is an estimate, not your
provider's billing balance. See [development and providers](docs/development.md).

For a no-load preview, run `npm run dev` or choose **Demo & testing** in the app. Demo data is labeled
and cannot change real processes. Closing the desktop window keeps monitoring active; use
**Quit Jitterbug** to exit.

## Develop

```sh
npm run build        # Svelte/TypeScript checks + frontend build
npm test             # Rust unit and integration tests
npm run test:ui      # isolated Playwright scenarios
npm run tauri build  # desktop package
```

The UI lives in `src/`; shared collection, policy, providers, and OS adapters live in
`src-tauri/src/`. Follow [AGENTS.md](AGENTS.md) and the checked-in Prettier configuration.
Keep credentials, real telemetry, transcripts, and screenshots out of Git; use ignored `.local/`.

## Guides

- [App inspection and agent connections](docs/app-and-agent-control.md)
- [Shared validation queue and workflow controls](docs/validation-queue.md)
- [Browser slowdown investigations](docs/browser-investigations.md)
- [Jitter Snapshot verification](docs/jitter-lab.md)
- [System settings rollback](docs/system-settings-rollback.md)
- [Development, providers, and testing](docs/development.md)
- [Third-party notices](THIRD_PARTY_NOTICES.md)

Jitterbug cannot move an arbitrary running process to the cloud, identify every stalled browser tab,
or reconstruct activity it did not capture. Measurements, inferred explanations, and demos are
distinguished throughout the app.
