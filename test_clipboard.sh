#!/bin/bash

echo "🧪 Testing Enhanced Clipboard Monitoring"
echo "========================================="
echo ""

# Copy some test data
echo "Test 1: Plain text" | pbcopy
sleep 1

echo "📋 Current clipboard content:"
pbpaste
echo ""

echo "🔍 Starting clipboard monitor (press Ctrl+C to stop)..."
echo ""

# Run the monitor to see the enhanced context
cargo run --bin research-tracker 2>/dev/null