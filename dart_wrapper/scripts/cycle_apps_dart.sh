#!/usr/bin/env bash
set -euo pipefail

# Simple AppleScript-driven app switcher for testing focus events with Dart CLI.
# Usage:
#   scripts/cycle_apps_dart.sh [App Name 1] [App Name 2] ...
#   CYCLES=3 DELAY=1.0 MINIMIZE=0 FULLSCREEN=0 scripts/cycle_apps_dart.sh Safari "Google Chrome" Terminal Finder
#
# Env vars:
#   CYCLES     - how many loops to run (default: 3)
#   DELAY      - seconds to wait after each switch (default: 1.0)
#   MINIMIZE   - if 1, send Cmd+M after activating each app (default: 0)
#   FULLSCREEN - if 1, send Ctrl+Cmd+F after activating each app (default: 0)

CYCLES=${CYCLES:-3}
DELAY=${DELAY:-1.0}
MINIMIZE=${MINIMIZE:-0}
FULLSCREEN=${FULLSCREEN:-0}

if [ "$#" -gt 0 ]; then
  APPS=("$@")
else
  # Default set of common apps; adjust to your machine
  APPS=(
    "Finder"
    "Safari"
    "Google Chrome"
    "Terminal"
    "Notes"
    "Visual Studio Code"
    "TextEdit"
  )
fi

echo "Cycling through apps for Dart tracker: ${APPS[*]}" >&2
echo "CYCLES=$CYCLES DELAY=$DELAY MINIMIZE=$MINIMIZE FULLSCREEN=$FULLSCREEN" >&2

activate_app() {
  /usr/bin/osascript <<EOF
try
  tell application "$1"
    activate
  end tell
  return true
on error
  return false
end try
EOF
}

keystroke_minimize() {
  /usr/bin/osascript <<'EOF'
tell application "System Events"
  keystroke "m" using {command down}
end tell
EOF
}

keystroke_fullscreen() {
  /usr/bin/osascript <<'EOF'
tell application "System Events"
  keystroke "f" using {control down, command down}
end tell
EOF
}

for (( c=1; c<=CYCLES; c++ )); do
  echo "[Dart Cycle $c/$CYCLES]" >&2
  for app in "${APPS[@]}"; do
    echo "  → Activating: $app" >&2
    if activate_app "$app"; then
      sleep "$DELAY"
      if [ "$MINIMIZE" = "1" ]; then
        echo "  → Minimizing: $app" >&2
        keystroke_minimize || true
        sleep "$DELAY"
      fi
      if [ "$FULLSCREEN" = "1" ]; then
        echo "  → Toggling fullscreen: $app" >&2
        keystroke_fullscreen || true
        sleep "$DELAY"
      fi
    else
      echo "  ⚠ Could not activate: $app (not installed?)" >&2
    fi
  done
done

echo "✅ Dart app cycling completed." >&2