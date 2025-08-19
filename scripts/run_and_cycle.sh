#!/usr/bin/env bash
set -euo pipefail

# Runs the tracker, then cycles through apps via AppleScript to generate focus/window events.
# Usage examples:
#   scripts/run_and_cycle.sh "Safari" "Google Chrome" "Terminal" "Finder"
#   CYCLES=2 DELAY=1.2 scripts/run_and_cycle.sh Safari Terminal
#   ENHANCED=1 FORMAT=json VERBOSE=1 scripts/run_and_cycle.sh "Safari" "Google Chrome"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")"/.. && pwd)"
cd "$ROOT_DIR"

# Config via env vars
ENHANCED=${ENHANCED:-1}    # default to enhanced on; set 0 to disable
FORMAT=${FORMAT:-human}    # human|json|research
VERBOSE=${VERBOSE:-1}      # 0|1|2
CYCLES=${CYCLES:-3}
DELAY=${DELAY:-1.0}
MINIMIZE=${MINIMIZE:-0}
FULLSCREEN=${FULLSCREEN:-0}

APPS=("$@")
if [ ${#APPS[@]} -eq 0 ]; then
  APPS=("Finder" "Safari" "Google Chrome" "Terminal" "Cursor")
fi

echo "[run_and_cycle] Building…" >&2
cargo build -q

TRACKER_ARGS=(--format "${FORMAT}")
# enhanced defaults to on via CLI default; only disable if explicitly set to 0
if [ "$ENHANCED" = "0" ]; then
  # pass a flag to disable if we later add it; for now, no-op as default is on
  :
fi
if [ "$VERBOSE" -gt 0 ]; then
  TRACKER_ARGS+=(--verbose)
fi

LOG_FILE="${ROOT_DIR}/target/tracker_run.log"
rm -f "$LOG_FILE"
echo "[run_and_cycle] Launching tracker… (logs: $LOG_FILE)" >&2
BIN="${ROOT_DIR}/target/debug/research-tracker"
"$BIN" "${TRACKER_ARGS[@]}" > "$LOG_FILE" 2>&1 &
TRACKER_PID=$!
# Stream logs to console
tail -n +1 -f "$LOG_FILE" &
TAIL_PID=$!

cleanup() {
  echo "[run_and_cycle] Stopping tracker (pid=$TRACKER_PID)…" >&2
  kill "$TRACKER_PID" 2>/dev/null || true
  [ -n "${TAIL_PID-}" ] && kill "$TAIL_PID" 2>/dev/null || true
}
trap cleanup EXIT

# Give the tracker a moment to subscribe observers
sleep 1.5

echo "[run_and_cycle] Cycling apps: ${APPS[*]} (CYCLES=$CYCLES DELAY=$DELAY MINIMIZE=$MINIMIZE FULLSCREEN=$FULLSCREEN)" >&2
CYCLES=$CYCLES DELAY=$DELAY MINIMIZE=$MINIMIZE FULLSCREEN=$FULLSCREEN \
  ./scripts/cycle_apps.sh "${APPS[@]}"

echo "[run_and_cycle] Done cycling. Leaving tracker running for 3s to flush events…" >&2
sleep 3

# Ensure we stop the tracker before exiting
cleanup



