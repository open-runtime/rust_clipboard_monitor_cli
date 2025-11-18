#!/bin/bash
# Start the monitor in background
./target/release/rust_clipboard_monitor_cli --format text 2>&1 &
PID=$!

sleep 2

# Switch to Finder
osascript -e 'tell application "Finder" to activate' 2>/dev/null
sleep 2

# Switch to Terminal  
osascript -e 'tell application "Terminal" to activate' 2>/dev/null
sleep 2

# Switch to Safari
osascript -e 'tell application "Safari" to activate' 2>/dev/null
sleep 2

# Back to Terminal
osascript -e 'tell application "Terminal" to activate' 2>/dev/null
sleep 2

# Kill the monitor
kill $PID 2>/dev/null
wait $PID 2>/dev/null || true
