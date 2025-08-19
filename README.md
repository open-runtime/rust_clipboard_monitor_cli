# Focus Track - macOS App & Window Focus Monitor

A production-ready Rust CLI application for tracking application and window focus changes on macOS. Monitors app switches, window changes, and browser tab changes with millisecond precision.

## Features

- **Real-time app tracking** - Monitors active application changes
- **Window title tracking** - Captures window titles for all applications
- **Browser tab support** - Special handling for Chrome, Safari, and Firefox tabs
- **Multiple output formats** - Text or JSON output for easy integration
- **Low resource usage** - Efficient polling with configurable intervals
- **Privacy-focused** - Runs locally, no data leaves your machine

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/yourusername/rust_clipboard_monitor_cli.git
cd rust_clipboard_monitor_cli

# Build release version
cargo build --release

# Run the binary
./target/release/rust_clipboard_monitor_cli
```

## Usage

### Basic Usage

```bash
# Start tracking with default settings (text output, 250ms polling)
./rust_clipboard_monitor_cli

# Use JSON output for programmatic consumption
./rust_clipboard_monitor_cli --format json

# Increase polling frequency for more responsive tracking
./rust_clipboard_monitor_cli --poll-interval 100

# Enable verbose logging
./rust_clipboard_monitor_cli -v  # Info level
./rust_clipboard_monitor_cli -vv # Debug level
```

### Command Line Options

```
Options:
  -f, --format <FORMAT>                Output format: text or json [default: text]
  -v, --verbose...                     Verbosity level (-v for info, -vv for debug)
      --no-prompt                      Don't prompt for accessibility permissions
      --poll-interval <POLL_INTERVAL>  Poll interval in milliseconds [default: 250]
  -h, --help                           Print help
  -V, --version                        Print version
```

### Output Examples

#### Text Format
```
Started tracking: Cursor (com.todesktop.230313mzl4w4u92)
  Window/Tab: main.rs — rust_clipboard_monitor_cli
App switch: Cursor → Google Chrome (spent 45.2s)
  Window/Tab: GitHub - rust-lang/rust
Window change in Google Chrome: "GitHub - rust-lang/rust" → "Stack Overflow" (spent 12.3s)
```

#### JSON Format
```json
{"event_type":"start","app":{"app_name":"Cursor","bundle_id":"com.todesktop.230313mzl4w4u92","window_title":"main.rs"}}
{"event_type":"app_switch","from":{"app_name":"Cursor","bundle_id":"com.todesktop.230313mzl4w4u92","window_title":"main.rs"},"to":{"app_name":"Google Chrome","bundle_id":"com.google.Chrome","window_title":"GitHub"},"duration_ms":45234}
```

## Permissions

The application works best with accessibility permissions but can function without them:

### With Accessibility Permissions
- Full window title tracking
- Browser tab title tracking
- All application window information

### Without Accessibility Permissions
- Basic app switching detection
- Bundle ID and app name tracking
- Limited window information

To grant permissions:
1. Open System Settings → Privacy & Security → Accessibility
2. Add the application to the list
3. Enable the checkbox

## Architecture

The application uses a hybrid approach for maximum compatibility:

- **AppleScript** for reliable app and window information
- **Polling-based monitoring** for consistent tracking
- **Browser-specific handling** for tab titles
- **Efficient state management** to detect changes

## Performance

- **Memory usage**: ~5-10MB
- **CPU usage**: <1% with default polling interval
- **Polling interval**: Configurable (50ms - 5000ms)

## Privacy & Security

- All processing happens locally
- No network connections
- No data storage (output only)
- Open source for full transparency

## Use Cases

- **Productivity tracking** - Monitor time spent in different applications
- **Development workflows** - Track context switches during coding
- **Research** - Analyze application usage patterns
- **Automation** - Trigger actions based on app switches
- **Time tracking** - Accurate work time allocation

## Building from Source

### Requirements

- Rust 1.70 or later
- macOS 11.0 or later
- Xcode Command Line Tools

### Development

```bash
# Run in development mode
cargo run

# Run with verbose output
RUST_LOG=debug cargo run

# Run tests
cargo test

# Build optimized binary
cargo build --release
```

## Integration Examples

### With Shell Scripts

```bash
#!/bin/bash
# Log app usage to file
./rust_clipboard_monitor_cli --format json >> ~/app_usage.jsonl
```

### With Python

```python
import subprocess
import json

proc = subprocess.Popen(
    ['./rust_clipboard_monitor_cli', '--format', 'json'],
    stdout=subprocess.PIPE,
    text=True
)

for line in proc.stdout:
    event = json.loads(line)
    print(f"Event: {event['event_type']}")
```

## Troubleshooting

### Application not detecting window changes
- Grant accessibility permissions in System Settings
- Some applications may not expose window information

### High CPU usage
- Increase the poll interval: `--poll-interval 500`
- Check for other monitoring applications

### Permission prompt not appearing
- Run without `--no-prompt` flag
- Manually add in System Settings → Privacy & Security → Accessibility

## Contributing

Contributions are welcome! Please feel free to submit pull requests or open issues.

## License

MIT License - See LICENSE file for details

## Acknowledgments

Built with:
- [clap](https://github.com/clap-rs/clap) - Command line argument parsing
- [serde](https://serde.rs/) - Serialization framework
- [tracing](https://github.com/tokio-rs/tracing) - Application instrumentation

## Roadmap

- [ ] Event-driven monitoring using macOS notifications
- [ ] Support for multiple monitors/spaces
- [ ] Window position and size tracking
- [ ] Export to various time tracking formats
- [ ] Configuration file support
- [ ] Historical data analysis