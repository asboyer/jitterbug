# App inspection and agent connections

## Inspect an app

Open **Workloads** and select an app. Its detail view separates a reusable purpose explanation
from live measurements and process origins. The parent chain is the OS's current view, not a
historical launch log. AI purpose descriptions are cached locally for up to 30 days and invalidated
when the app version or installation changes.

**Inside an app** adds two kinds of detail:

- **Chromium tabs:** choose **Show tabs** and follow the browser's remote-debugging setup instructions.
  Chrome, Edge, Brave, Chromium, and Vivaldi use a local DevTools connection. Remote debugging grants
  broader access than the metrics Jitterbug reads; enable it intentionally and turn it off in the
  browser to revoke access. Background incidents reuse an approved connection, never open one.
  Safari and Firefox tab inspection are not implemented. The current selector depends on macOS app
  bundles, so Windows inspection is not wired end to end yet.
- **Command sessions:** supported terminals, editors, and coding apps show command subtrees from
  the process inventory. Detached work may lose attribution, and helper processes can still
  obscure individual sessions. No transcripts or shell history are read.

Tab CPU is sampled on demand over one second. Renderer identity is inferred from matching browser
and tab CPU counters; shared-renderer memory is an estimate marked **≈**. **Show** focuses the tab;
**Close tab** confirms the action and rechecks its address. Closing can discard unsaved input.

On supported Unix systems, **Pause** suspends a command tree while retaining its memory. It resumes
after 15 minutes or on normal app exit; recovery and a separate watchdog cover interrupted runs.
Identity, ownership, and protected processes are checked before acting. Pause differs from holding
future work in the [validation queue](validation-queue.md).

## Register an agent task

Keep the desktop running and use the same task ID with distinct agent labels in each worktree:

```sh
cargo build --manifest-path src-tauri/Cargo.toml --bin jitterbug-run

./src-tauri/target/debug/jitterbug-run \
    --task-id EXAMPLE-1 --task-title "Improve a test workflow" \
    --agent implementation --harness claude --capture-context \
    --worktree /absolute/path/to/worktree -- claude
```

Use `--task-description` to supply intent for another harness. The wrapper preserves terminal I/O
and exit status. PID/start-time checks and ancestry establish ownership; a shared folder does not.
Peaks are sampled, and summed resident memory can double-count shared pages. Successful command exit
does not prove a feature is complete. Registration attributes work; it does not schedule it.

`--capture-context` adds session-only Claude Code hooks. It retains bounded request excerpts and
activity counts locally, not code, tool output, or transcripts. Do not combine it with an existing
`--settings` argument without merging settings yourself. `jitterbug-run hook-settings` prints
configuration for manual integration; generated local paths must stay out of Git. Codex-compatible
hook parsing does not establish per-chat ownership in a shared desktop daemon.

**Summarize task with AI** explicitly sends displayed request excerpts to the intelligent provider.
Review these first: they can contain private information despite redaction. Ordinary attribution
requires no inference. For scheduling, use the [shared validation queue](validation-queue.md), with
separate MCP tools and approved command profiles for Claude Code, Codex, and Muse Code.

## Let agents request resource actions

The control MCP server is separate from the validation queue. Build the runner above, then connect:

```sh
claude mcp add jitterbug-control -- /absolute/path/to/jitterbug-run control-mcp
```

Enable agent access in **Settings** first. The desktop app must be running.

| Tool                       | Purpose                                      |
| -------------------------- | -------------------------------------------- |
| `jitterbug_overview`       | Machine pressure, apps, and command sessions |
| `jitterbug_inspect_app`    | One app's sessions or connected browser tabs |
| `jitterbug_pause_session`  | Request a reversible session pause           |
| `jitterbug_resume_session` | Resume the pause owned by that agent request |
| `jitterbug_close_tab`      | Request closure of one browser tab           |
| `jitterbug_request_status` | Check approval and action results            |

Agent control is off by default and available on macOS/Linux, not Windows. Any program running as
your OS user can reach the owner-only socket when enabled; it is not restricted to a named harness.
Results may reach the requesting agent's provider: app/session labels, folder basenames, tab titles,
and hosts, but not full paths, URLs, command lines, or page contents.

Each action asks unless you separately allow agents to pause sessions automatically. Tab closure
always asks. An agent cannot pause itself or resume a user-initiated pause. Requests are bounded to
one pending request per target and six per hour. Snoozing holds automatic pauses; disabling access
refuses new requests.
