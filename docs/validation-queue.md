# Shared validation queue

Jitterbug admits one heavy validation at a time across every participating session,
worktree, and project of the logged-in OS user. An agent can keep editing or reasoning
while its submitted validation waits. The same runner also works for a human in a
terminal; this is not restricted to subagents or one conversation.

The desktop must be open to schedule work. The queue uses the existing five-second
collector, with no inference, extra background agent, or per-waiter system scan.
It checks the **whole computer**, including browsers, local models, ordinary apps,
and background services. The dashboard's **Make room** suggestions identify measured
competitors and suggest deferring work, closing unused apps/tabs, unloading a model,
or considering a remote runner. Suggestions do not move or stop anything.

## Register a validation

Build the runner once, then use an absolute path to it in client configurations:

```sh
cargo build --manifest-path src-tauri/Cargo.toml --bin jitterbug-run

./src-tauri/target/debug/jitterbug-run queue add project-tests \
    --worktree /absolute/path/to/worktree --memory-mib 2048 \
    -- node --test --test-concurrency=1

./src-tauri/target/debug/jitterbug-run queue run project-tests \
    --task "Check search navigation" --session terminal
```

On Windows use `jitterbug-run.exe` and an absolute Windows worktree path. Arguments
are passed directly to the executable, not interpreted as a shell string. Register
a shell explicitly if your approved command needs one. A profile fixes the working
directory, executable, arguments, and estimated additional memory. Register separate
profiles for separate worktrees. `queue remove NAME` revokes a profile; waiting jobs
will not launch under a removed or changed profile.

Registration is permission to execute that fixed command as your OS user, **outside
the requesting agent's sandbox**. Review commands before registering them. Commands
can run project scripts, so only register trusted checkouts. MCP cannot create or
edit profiles, supply new command arguments, release interrupted slots, or change
scheduling policy. For human use, `queue submit` returns immediately; `queue run`
waits, prints bounded stdout/stderr, and preserves the command's exit status. A stale,
cancelled, or interrupted result exits with code 3. Ctrl-C stops waiting; cancel a
queued job explicitly with `queue cancel JOB_ID`.

## Connect a coding tool

All clients connect to the **same local queue** through a stdio MCP server. Limit each
connection to named approved profiles. Keep configurations and absolute paths local,
outside public Git. No provider key belongs in this MCP configuration.

### Claude Code, including local desktop Code sessions

Use Claude Code's local MCP configuration, or pass a private JSON file with
`--strict-mcp-config --mcp-config /path/to/jitterbug-mcp.json` for a single CLI run:

```json
{
    "mcpServers": {
        "jitterbug": {
            "command": "/absolute/path/to/jitterbug-run",
            "args": ["mcp", "--profiles", "project-tests", "--harness", "claude"]
        }
    }
}
```

Approve the submit tool through the client. For a deliberately unattended test,
Claude's `--allowedTools mcp__jitterbug__validation_submit` can approve that specific
tool. Keep your normal subscription login; `--bare` disables OAuth/keychain reads
and is not suitable for a subscription test. See [Claude's MCP documentation](https://code.claude.com/docs/en/mcp).

### Codex CLI and local desktop sessions

Add this server in the desktop MCP settings or your private Codex configuration:

```toml
[mcp_servers.jitterbug]
command = "/absolute/path/to/jitterbug-run"
args = ["mcp", "--profiles", "project-tests", "--harness", "codex"]
required = true
tool_timeout_sec = 30
```

Approve `validation_submit` when asked. For a deliberately unattended run using only
reviewed profiles, the narrowly scoped configuration below grants that tool permission;
without it, a headless run with approval policy `never` rejects submission:

```toml
[mcp_servers.jitterbug.tools.validation_submit]
approval_mode = "approve"
```

The CLI accepts the same fields through per-launch `-c` overrides; global settings
do not need to change for a test. See [Codex MCP configuration](https://learn.chatgpt.com/docs/extend/mcp).

### Meta's Muse Code

Meta currently calls its coding CLI **Muse Code**. Use the [official installation
and authentication guide](https://dev.meta.ai/docs/muse-code). For API usage, pass
`META_API_KEY` through the environment or `--api-key-stdin`; never put it in arguments,
project files, or MCP settings. Add to your private Muse settings:

```json
{
    "mcp_servers": {
        "jitterbug": {
            "transport": "stdio",
            "command": "/absolute/path/to/jitterbug-run",
            "args": ["mcp", "--profiles", "project-tests", "--harness", "meta"]
        }
    }
}
```

For per-launch overrides, address `mcp_servers.jitterbug` rather than replacing the
entire server map. [Muse's MCP and headless guide](https://dev.meta.ai/docs/muse-code/extending)
describes approval behavior and `--max-model-steps`. The queue makes no model calls;
the harness's own provider/subscription pays for its reasoning. Jitterbug's model
budget ledger does not govern an external harness.

### Tell the agent how to participate

Put an instruction like this in the project's appropriate local agent instructions:

> Submit heavy validation through Jitterbug's approved profile. Give each job a
> unique request ID and the task/session label. Do independent work while waiting.
> Read `validation_result` before reporting success; use `wait_seconds: 20` instead
> of busy polling. Do not run the same check directly while it is queued. Resubmit
> if source inputs changed.

Tools: `validation_profiles`, `validation_submit`, `validation_result`, and
`validation_cancel`. Submission is asynchronous and idempotent for a retained
request ID. Status and results survive client exit and desktop restart. Labels
identify the submitting session; they are supplied by the client, not inferred
proof of chat identity. Resource attribution uses the worker's verified PID/start
time and descendants, not a shared desktop app-server PID.

## Admission and recovery

- One slot; FIFO across sessions. This MVP does not increase concurrency automatically.
- Two healthy samples; fresh telemetry no older than 15 seconds.
- CPU below 70%, available memory at least the job estimate plus 2 GiB and at least
  20% of total RAM, no OS memory warning, no reported thermal pressure, and less
  than 128 MiB swap growth in the latest interval. Missing Windows thermal/memory
  pressure sensors are unavailable, not proof of normal conditions.
- These are admission checks, not CPU/RAM limits. Unexpected growth can still occur.
  Running work is never killed automatically by the queue.
- Pause prevents new starts; queued cancellation never stops a running process.
  Queued jobs expire after one hour. History retains up to 48 jobs, with at most
  16 pending jobs and 32 saved profiles. Each stream retains its first 32 KiB and
  reports truncation; excess output is drained without growing memory.
- If a worker disappears, the slot stays blocked. Check for surviving descendants,
  then use the desktop review control or `queue release JOB_ID --reviewed-descendants`.
  A live verified worker or direct child prevents release. Jitterbug does not guess
  that orphaned grandchildren have finished.
- Git HEAD plus tracked and nonignored untracked file contents are fingerprinted at
  submission, immediately before execution, and after completion. Changes produce
  a stale result, never an automatic pass or retry. Fingerprints are bounded to
  64 MiB; symlinks/submodules are currently rejected. Ignored files, dependencies,
  environment variables, external services, and transient edits reverted between
  checks are not covered. Non-Git directories explicitly report unverified inputs.

State, approved commands, fingerprints, and bounded outputs stay in the private OS
application-data directory. They never enter Jitterbug's cloud prompts. Requesting
a result through MCP sends that command output to the requesting harness, which may
send it to its model provider. Avoid checks that print secrets. `JITTERBUG_QUEUE_DIR`
and `JITTERBUG_AGENT_RUNS_DIR` allow isolated test registries.

Jitterbug removes its model-provider API key variables from the worker environment.
Other inherited environment variables and the user's filesystem remain accessible to
approved commands; this is a scheduler, not an isolation sandbox.

## Verification

Automated tests exercise real worker processes, simultaneous coordinators, FIFO
serialization across sessions, exit status and bounded stdout/stderr, pause/cancel,
pressure admission, removed profiles, source changes before/during execution, missing
workers, credential stripping, MCP profile scope, and the stdio handshake. Fixtures
use synthetic telemetry without stressing the host.

On macOS, real Claude Code, Codex CLI, and Muse Code sessions each submitted a fixed
Node assertion in separate Git worktrees and read a passing result over MCP. Claude
and Codex overlapped as clients while their validation jobs ran sequentially. The
desktop pause, queued cancellation, resume, and result views were checked against the
same live queue. Claude/Codex used existing subscription authentication; Muse used a
locally bounded API test. Desktop client MCP setup is documented from each vendor's
supported configuration, but separate Claude/Codex desktop-agent sessions have not
been exercised end to end. Native Windows interaction remains unverified; Windows
builds and process-level integration tests run in CI.

## Boundaries

This is cooperative admission, not a universal interception layer. A process started
through a normal shell outside the queue is not governed. Desktop clients can use
the same local MCP protocol when their surface supports it; cloud-only sessions
without local MCP access cannot. This does not establish full per-chat attribution
of a shared desktop daemon. Use finite validation commands that wait for their own
children; detached background services are not supported queue jobs.

Remote offloading needs a configured destination and a workload-specific runner.
Moving an arbitrary already-running app to another machine is not implemented.
