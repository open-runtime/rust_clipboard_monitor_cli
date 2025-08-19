#!/usr/bin/env bash
set -euo pipefail

# Runs the Dart clipboard monitor, then cycles through apps via AppleScript to generate focus/window events.
# Usage examples:
#   scripts/run_and_cycle_dart.sh "Safari" "Google Chrome" "Terminal" "Finder"
#   CYCLES=2 DELAY=1.2 scripts/run_and_cycle_dart.sh Safari Terminal
#   ENHANCED=1 VERBOSE=1 scripts/run_and_cycle_dart.sh "Safari" "Google Chrome"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DART_WRAPPER_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ROOT_DIR="$(cd "$DART_WRAPPER_DIR/.." && pwd)"

cd "$DART_WRAPPER_DIR"

# Config via env vars
ENHANCED=${ENHANCED:-1}    # default to enhanced on; set 0 to disable
VERBOSE=${VERBOSE:-1}      # 0|1|2
CYCLES=${CYCLES:-3}
DELAY=${DELAY:-1.0}
MINIMIZE=${MINIMIZE:-0}
FULLSCREEN=${FULLSCREEN:-0}

APPS=("$@")
if [ ${#APPS[@]} -eq 0 ]; then
  APPS=("Finder" "Safari" "Google Chrome" "Terminal" "Notes")
fi

echo "[run_and_cycle_dart] Building Dart CLI…" >&2
if [ ! -f "./clipboard_monitor_dart" ]; then
  echo "Building Dart executable..."
  dart compile exe bin/main.dart -o clipboard_monitor_dart
fi

# Build Rust library if needed
if [ ! -f "../target/release/libresearch_assistant_tracker.dylib" ]; then
  echo "Building Rust library..."
  cd "$ROOT_DIR"
  cargo build --lib --release
  cd "$DART_WRAPPER_DIR"
fi

TRACKER_ARGS=()
if [ "$ENHANCED" = "0" ]; then
  TRACKER_ARGS+=(--no-enhanced)
fi
if [ "$VERBOSE" -gt 0 ]; then
  TRACKER_ARGS+=(--verbose "$VERBOSE")
fi

LOG_FILE="${DART_WRAPPER_DIR}/dart_tracker_run.log"
rm -f "$LOG_FILE"
echo "[run_and_cycle_dart] Launching Dart tracker… (logs: $LOG_FILE)" >&2
(
  ./clipboard_monitor_dart "${TRACKER_ARGS[@]}" 2>&1 | tee "$LOG_FILE"
) &
TRACKER_PID=$!

cleanup() {
  echo "[run_and_cycle_dart] Stopping Dart tracker (pid=$TRACKER_PID)…" >&2
  kill "$TRACKER_PID" 2>/dev/null || true
}
trap cleanup EXIT

# Give the tracker a moment to initialize
sleep 2

echo "[run_and_cycle_dart] Cycling apps: ${APPS[*]} (CYCLES=$CYCLES DELAY=$DELAY MINIMIZE=$MINIMIZE FULLSCREEN=$FULLSCREEN)" >&2
CYCLES=$CYCLES DELAY=$DELAY MINIMIZE=$MINIMIZE FULLSCREEN=$FULLSCREEN \
  ./scripts/cycle_apps_dart.sh "${APPS[@]}"

echo "[run_and_cycle_dart] Done cycling. Leaving tracker running for 3s to flush events…" >&2
sleep 3