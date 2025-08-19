#!/bin/bash

set -e

echo "🦀 Building Rust library..."
cargo build --release --lib

echo "📱 Installing Dart dependencies..."
cd dart_wrapper
dart pub get

echo "🔧 Generating Flutter Rust Bridge bindings..."
cd ..
dart run flutter_rust_bridge_codegen generate

echo "🎯 Building Dart CLI..."
cd dart_wrapper
dart compile exe bin/main.dart -o clipboard_monitor_dart

echo "✅ Build complete!"
echo "📍 Dart executable: dart_wrapper/clipboard_monitor_dart"
echo "📍 Rust library: target/release/libresearch_assistant_tracker.dylib"
echo ""
echo "🚀 To run: ./dart_wrapper/clipboard_monitor_dart --help"