#!/bin/sh
# Exercises `jitterbug-run gate/release` like three agents would, against throwaway state.
# Usage: scripts/turns-smoke.sh /path/to/jitterbug-run
set -eu
RUNNER="$1"
STATE="$(mktemp -d)"
trap 'rm -rf "$STATE"' EXIT
export JITTERBUG_TURNS_DIR="$STATE/turns" JITTERBUG_QUEUE_DIR="$STATE/queue" JITTERBUG_TURN_WAIT_SECONDS=2
mkdir -p "$JITTERBUG_QUEUE_DIR"
NOW=$(date +%s)
printf '{"captured_at":%s,"ready":true,"cpu_percent":20,"available_bytes":17179869184,"total_bytes":34359738368,"thermal":null,"memory_pressure":null,"swap_growth_bytes":0,"consecutive_good":3,"suggestions":[]}' "$NOW" \
    > "$JITTERBUG_QUEUE_DIR/headroom.json"

event() { printf '{"session_id":"%s","tool_use_id":"%s","cwd":"/w/app","tool_input":{"command":"%s"}}' "$1" "$2" "$3"; }

echo "1. light command from Claude passes:"
event a t1 "git status" | "$RUNNER" gate --harness claude; echo "   (no output = allowed)"
echo "2. heavy command from Claude takes the slot:"
event a t2 "cargo test --lib" | "$RUNNER" gate --harness claude; echo "   (no output = started)"
echo "3. heavy command from Codex waits, then is handed back with a reason:"
event b t3 "npm test" | "$RUNNER" gate --harness codex
echo "4. Claude's command finishes:"
event a t2 "cargo test --lib" | "$RUNNER" release --harness claude
echo "5. Codex retries and gets the slot:"
event b t4 "npm test" | "$RUNNER" gate --harness codex; echo "   (no output = started)"
echo "6. state:"
cat "$JITTERBUG_TURNS_DIR/turns.json"; echo
