# Testing the Enhanced Block Variant

The enhanced block variant provides superior app switching monitoring with comprehensive data collection. Here's how to test it:

## Option 1: Simple Test (Recommended)

Run the simple test example:

```bash
cargo run --example simple_enhanced_test
```

This will:
- Start the enhanced monitoring
- Show detailed app switch events as you switch between applications
- Display window info, process data, desktop state, and more

## Option 2: Build and Run with Feature Flag

If you want to test with the feature flag:

```bash
cargo run --features enhanced_block --example simple_enhanced_test
```

## Option 3: Integration Test

To test how it would integrate with the main application, modify `src/main.rs` temporarily:

1. Replace the standard AppSwitcher creation:

```rust
// Replace this line in main.rs:
let app_switcher = Arc::new(Mutex::new(AppSwitcher::new()));

// With:
use research_assistant_tracker::core::app_switcher_enhanced_block::EnhancedAppSwitcher;
let enhanced_switcher = Arc::new(Mutex::new(EnhancedAppSwitcher::new()));
```

2. Then run the main application:

```bash
cargo run
```

## Option 4: Direct Testing

For direct testing without examples, create a minimal test:

```rust
use objc2::MainThreadMarker;
use research_assistant_tracker::core::app_switcher_enhanced_block::{
    EnhancedAppSwitcher, DebugListener
};

fn main() {
    let mtm = MainThreadMarker::new().unwrap();
    let mut switcher = EnhancedAppSwitcher::new();
    switcher.add_listener(DebugListener);
    switcher.start_monitoring(mtm).unwrap();
    
    // Switch apps and observe output
    std::thread::sleep(std::time::Duration::from_secs(30));
    
    switcher.stop_monitoring();
}
```

## What to Test

### 1. App Switching
- Switch between different applications
- Notice the rich data: CPU usage, memory, window titles, bounds

### 2. Event Coalescing
- Rapidly switch between apps (Cmd+Tab quickly)
- Watch how events are coalesced into single foreground events

### 3. Desktop State Changes
- Lock/unlock screen
- Change displays
- Notice idle time tracking

### 4. Window Information
- Switch to apps with different window titles
- Open/close windows
- Resize windows

### 5. Process Information
- Switch to CPU-intensive apps
- Notice real-time CPU and memory tracking

## Expected Output

You should see detailed output like:

```
🔄 App Switch Event:
  Type: Foreground
  App: Google Chrome (com.google.Chrome)
  PID: 1234
  Windows: 3
  Front Window: GitHub - Enhanced App Switcher
  CPU: 12.5%
  Memory: 543.2 MB
  Activation #: 5
  Trigger: EventCoalescing
  Confidence: 95%
  Desktop State:
    Session Active: true
    Screen Locked: false
    User: yourname
    Idle: 2.3s
```

## Performance Comparison

To compare with the standard variant, the enhanced block provides:

- **6x more data points** per event
- **Real-time process monitoring** (CPU, memory, threads)
- **Window metadata** (titles, bounds, layers)
- **Desktop state tracking** (session, lock, displays, idle)
- **Event coalescing** for cleaner event streams
- **Higher confidence scoring** with multi-layer validation

## Troubleshooting

### Permission Issues
If you see permission errors:
- Grant accessibility permissions in System Settings
- Run from a terminal that has developer permissions

### Build Issues
```bash
# Clean and rebuild
cargo clean
cargo build

# Or with specific features
cargo build --features enhanced_block
```

### Runtime Issues
- Ensure you're running on macOS
- Check that you're calling from the main thread
- Verify NSWorkspace notifications are working

## Next Steps

Once testing is successful, you can:
1. Integrate into main.rs permanently
2. Add custom listeners for your specific use case
3. Export data to files or databases
4. Build analysis tools on top of the rich data