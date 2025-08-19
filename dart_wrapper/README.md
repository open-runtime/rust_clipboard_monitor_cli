# Clipboard Monitor Dart CLI

A lightweight Dart CLI wrapper for the Rust clipboard monitor functionality using Flutter Rust Bridge.

## Features

- **App Switching Detection**: Monitor when applications gain/lose focus
- **Enhanced Context**: Extract URLs, window titles, and file paths (requires accessibility permissions)
- **Cross-Platform**: Built with Rust for performance, exposed through Dart for ease of use
- **Configurable**: Multiple output formats and verbosity levels

## Prerequisites

- Dart SDK 3.0.0 or higher
- Rust toolchain (for building the native library)
- macOS (the underlying Rust library is macOS-specific)

## Installation

1. Clone this repository
2. Run the build script:
   ```bash
   cd .. && ./build_dart_wrapper.sh
   ```

## Usage

```bash
# Basic usage
./clipboard_monitor_dart

# Check permissions
./clipboard_monitor_dart --check-permissions

# Run with enhanced context extraction
./clipboard_monitor_dart --enhanced

# Run with verbose output
./clipboard_monitor_dart --verbose 2

# Run in background mode (no prompts)
./clipboard_monitor_dart --background

# Show help
./clipboard_monitor_dart --help
```

## Command Line Options

- `--enhanced` / `-e`: Extract detailed context (requires accessibility permissions) [default: true]
- `--verbose` / `-v`: Verbosity level (0-2) [default: 0]
- `--background` / `-b`: Run without prompting for permissions [default: false]
- `--filter` / `-f`: Only track specific app types (browser, ide, productivity)
- `--check-permissions` / `-c`: Check required permissions and exit
- `--help` / `-h`: Show help message

## Permissions

This application requires **Accessibility** permissions to function properly:

1. Open **System Settings**
2. Go to **Privacy & Security** → **Accessibility**
3. Add this application to the list
4. Enable the checkbox

You can check your current permissions with:
```bash
./clipboard_monitor_dart --check-permissions
```

## Example Output

```
🚀 Starting Dart Clipboard Monitor...
Configuration: enhanced=true, verbose=0
👀 Monitoring started. Press Ctrl+C to stop gracefully.

🔥 SWITCHED TO: Visual Studio Code (com.microsoft.VSCode)
   From: Safari (pid: 1234)
   Window: main.dart - clipboard_monitor_dart
   URL: file:///Users/username/project/main.dart
```

## Architecture

This Dart CLI is a wrapper around the Rust clipboard monitor library:

- **Rust Core**: High-performance system monitoring using objc2 and macOS APIs
- **FFI Layer**: Flutter Rust Bridge provides type-safe bindings
- **Dart CLI**: User-friendly command-line interface

## Development

To modify or extend the wrapper:

1. Edit Rust code in `../src/ffi_api.rs`
2. Modify Dart code in `lib/` or `bin/`
3. Rebuild with `../build_dart_wrapper.sh`

## Troubleshooting

### Library Loading Issues
If you get library loading errors:
1. Ensure the Rust library is built: `cd .. && cargo build --release`
2. Check that the library exists: `ls -la ../target/release/libresearch_assistant_tracker.dylib`

### Permission Errors
If accessibility features don't work:
1. Check permissions: `./clipboard_monitor_dart --check-permissions`
2. Grant accessibility permissions in System Settings
3. Restart the application

### Build Issues
If the build script fails:
1. Ensure Dart SDK is installed: `dart --version`
2. Ensure Rust is installed: `cargo --version`
3. Try building components separately