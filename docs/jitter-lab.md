# Jitter lab

Reproducible loads for checking Jitter Snapshot analysis. Run them on your own machine;
never commit their output. Each scenario names the finding the rules should produce.

## Headless lab (no desktop app)

The probe runs the real collector and pulse sampler, starts one disposable load eight
seconds before a simulated press, freezes the capture after the ten-second post-roll, and
prints only finding kinds, strengths, and whether the culprit is the load. The load is
always stopped, and the disk scenario deletes its temporary file.

```sh
cargo run --release --manifest-path src-tauri/Cargo.toml --example jitterbug-pulse-probe -- --lab pegged
cargo run --release --manifest-path src-tauri/Cargo.toml --example jitterbug-pulse-probe -- --lab contention
cargo run --release --manifest-path src-tauri/Cargo.toml --example jitterbug-pulse-probe -- --lab memory
cargo run --release --manifest-path src-tauri/Cargo.toml --example jitterbug-pulse-probe -- --lab disk
```

Each run takes about 75 seconds: a 55-second quiet baseline, the load, the press, and the
post-roll. Keep the machine otherwise idle for clean results.

| Scenario   | Load                                | Expected finding                           |
| ---------- | ----------------------------------- | ------------------------------------------ |
| pegged     | one `yes > /dev/null`               | Process spike and one saturated core       |
| contention | one `yes` per logical core          | Process spikes (up to three) on the loads  |
| memory     | `memory_pressure -l warn`           | Memory paging; strong once pressure rises  |
| disk       | `dd` writing 4 GiB from `/dev/zero` | Usually nothing found: disk is not sampled |

`/usr/bin` tools are protected processes, so the lab never offers an in-app action; the
analysis still names them as culprits.

## Manual lab (desktop app)

Start `npm run tauri dev`, keep an editor or browser in front, then in a terminal:

```sh
yes > /dev/null &                        # pegged core
for i in $(seq "$(sysctl -n hw.logicalcpu)"); do yes > /dev/null & done   # contention
memory_pressure -l warn                  # memory thrash; Ctrl+C to stop
dd if=/dev/zero of=/tmp/jitter-lab bs=1m count=8192 && rm /tmp/jitter-lab   # disk
```

Wait ten seconds, press **⌘⇧J** (Ctrl+Shift+J on Windows) with your editor in front, and
stop the load afterwards with `kill %1` (or `killall yes`). The case should name your
editor as the app you were using and the load as the culprit. In **Auto** mode, sustained
scheduler delay can also open a **Short hitch** case without a press.

## A short-lived hog inside the app in front

```sh
cargo run --release --manifest-path src-tauri/Cargo.toml --example jitterbug-hog-probe -- --lead 4
cargo run --release --manifest-path src-tauri/Cargo.toml --example jitterbug-hog-probe -- --lead 7 --exit-before 1
```

Add `--shell` to start the loop from a shell, as a terminal command would, and `--procs 3`
for a command with several busy processes. Run from a terminal inside an editor. A single-core `perl` loop starts a few seconds before
a simulated press (and optionally exits just before it), with the hosting app as the app in
front. The probe passes only when the loop is reported as a separate culprit, not as the
editor's own work. It prints finding kinds and labels without folder names.

## Measuring the sampler itself

```sh
cargo run --release --manifest-path src-tauri/Cargo.toml --example jitterbug-pulse-probe -- --seconds 120
```

Reports the sampler's own CPU time, resident-memory growth, wakeups, per-tick cost, and
the scheduler probe distribution. Budget: under 0.5% of one core and 5 MiB.
