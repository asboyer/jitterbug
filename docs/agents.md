# Agents and taking turns

The **Agents** tab lists every coding agent running on this computer (Claude Code, Codex, and
Muse Code), what each one is using, and the heavy commands it runs. Agents are found from the
process tree, so they appear without setup.

## Taking turns

Connecting an agent adds one hook to its own settings. Before each shell command the agent asks
`jitterbug-run gate`:

- Light commands (git, ls, edits, installs) pass immediately.
- Heavy commands (test suites, builds, type checks) share one slot. The first runs at full speed;
  the others wait. The gate holds a waiting command for up to 45 seconds, then hands it back
  with a short message telling the agent what is ahead of it and to keep working and retry.
  The command keeps its place in line for three minutes.
- After the command, `jitterbug-run release` frees the slot. If that never arrives, the desktop
  frees it when the harness exits or the command's processes have been quiet for 20 seconds.

A heavy command also waits while the whole computer is busy (CPU at 70% or more, low memory,
OS memory pressure, or heat). The gate fails open: when Jitterbug is closed, turned off, or its
state is unreadable, every command runs as it would without Jitterbug. Nothing is started,
stopped, or rewritten. Agents that ignore the message are not forced.

Heavy commands are recognized by program and subcommand (`cargo test`, `npm run build`,
`pytest`, `go test`, `make`, and similar); see `src-tauri/src/turns/classify.rs`. Long-running
servers such as `npm run dev` pass.

## Connecting

**Connect** in the Agents tab shows exactly what is added to each file and saves a backup in
Jitterbug's private data folder first. Unrelated settings are kept. **Disconnect** removes only
Jitterbug's entries.

| Agent       | Where the hook goes                                                       | Trust                                    |
| ----------- | ------------------------------------------------------------------------- | ---------------------------------------- |
| Claude Code | `~/.claude/settings.json` (`PreToolUse`/`PostToolUse`, matcher `Bash`)    | None needed                              |
| Codex       | `~/.codex/config.toml` (`[[hooks.PreToolUse]]`/`[[hooks.PostToolUse]]`)   | Trust it once: open Codex, type `/hooks` |
| Muse Code   | A local plugin, `jitterbug-agents`, installed with `muse plugins install` | Approved with `muse plugins approve`     |

Open sessions pick up the hook when restarted. Each agent shows **heard from it** once its first
command reaches Jitterbug.

## Subagents

Claude Code includes the subagent's ID in its hook events. Jitterbug reads that subagent's
description from the `.meta.json` file next to the session transcript and shows it as the
agent's name. Other harnesses report the agent type where available.

## Explanations

The headline and per-agent notes come from the intelligent model tier (Meta by default). They
are requested only when the agents or their heavy commands change. The model receives harness
names, subagent descriptions, short command names, states, and CPU/memory, never paths,
prompts, transcripts, or session names. Mode restrictions and the cloud budget apply.

## Verification

Unit tests cover classification, the line, payload parsing for each harness, subagent naming,
and adding/removing hooks in all three formats. `scripts/turns-smoke.sh` drives the real
`jitterbug-run` binary through gate and release with throwaway state. Headless Claude Code
sessions using session-only `--settings` were verified to take turns: three `make test` runs from
three sessions ran one at a time, the waiting session received the message and retried, and a
subagent's run was labeled with its description. Muse Code's plugin format and deny handling
were checked with `muse plugins validate` and `muse plugins hook test`. Live Codex and Muse Code
sessions have not yet been run through the gate. Codex silently skips the hook until it is trusted
with `/hooks`; Muse Code needs `muse login` or `META_API_KEY` before it can run.
