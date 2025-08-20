# Window State Tracking Enhancements Documentation

## Current State Analysis

### What We Currently Track
Based on analysis of `src/core/app_switcher_enhanced.rs`:

1. **Basic Window Properties**
   - Window ID (`kCGWindowNumber`)
   - Title (`kCGWindowName`)
   - Layer (`kCGWindowLayer`)
   - Alpha/transparency (`kCGWindowAlpha`)
   - Is on screen (`kCGWindowIsOnscreen`)
   - Window bounds (position and size)

2. **Application Events via NSWorkspace**
   - App activation/deactivation
   - App launch/termination
   - App hide/unhide
   - Space changes (partial)
   - Session changes
   - Wake from sleep

3. **Missing Critical Tracking**
   - ❌ Fullscreen state detection
   - ❌ Minimized window detection
   - ❌ Window close/open events
   - ❌ Space/desktop assignment for windows
   - ❌ Mission Control state
   - ❌ Window stacking order changes
   - ❌ Multi-monitor window movement

## Required Enhancements

### 1. Fullscreen State Detection

#### Method A: Window Bounds Comparison
```swift
// Check if window bounds match screen bounds
let screens = NSScreen.screens
for screen in screens {
    if window.frame == screen.frame {
        // Window is fullscreen on this screen
    }
}
```

#### Method B: NSWindow Style Mask (if we have window reference)
```swift
if window.styleMask.contains(.fullScreen) {
    // Window is in fullscreen mode
}
```

#### Method C: CGWindowListCopyWindowInfo Properties
```objc
// Check kCGWindowBounds against display bounds
CGDirectDisplayID displayID = CGMainDisplayID();
CGRect displayBounds = CGDisplayBounds(displayID);
// Compare with window bounds
```

### 2. Minimized Window Detection

#### Key Indicators:
- **kCGWindowIsOnscreen**: Will be `false` for minimized windows
- **kCGWindowStoreType**: Check if equals to `kCGBackingStoreBuffered`
- **Window Layer**: Minimized windows often have layer = 0 or negative
- **Alpha Value**: May be 0.0 for minimized windows

```rust
// Enhanced window state detection
pub enum WindowState {
    Normal,
    Minimized,
    Fullscreen,
    Hidden,
    Offscreen,
}

impl WindowInfo {
    pub fn detect_state(&self, screen_bounds: &[ScreenBounds]) -> WindowState {
        // Check if minimized
        if !self.is_onscreen && self.layer <= 0 {
            return WindowState::Minimized;
        }
        
        // Check if fullscreen
        for screen in screen_bounds {
            if self.bounds.matches_screen(screen) {
                return WindowState::Fullscreen;
            }
        }
        
        // Check if hidden
        if self.alpha == 0.0 {
            return WindowState::Hidden;
        }
        
        // Check if offscreen
        if !self.is_onscreen {
            return WindowState::Offscreen;
        }
        
        WindowState::Normal
    }
}
```

### 3. Space/Desktop Assignment Tracking

#### Private CGS API (Use with caution)
```c
// Private API declarations
extern int CGSGetWindowWorkspace(int cid, int wid, int *workspace);
extern int CGSGetWorkspaceWindowList(int cid, int workspace, int count, int *list, int *outCount);
extern int CGSGetWorkspaceWindowCount(int cid, int workspace, int *count);
```

#### Alternative: Track Space Changes
```rust
// Track which windows are visible before/after space change
struct SpaceTracker {
    spaces: HashMap<u32, Vec<u32>>, // space_id -> window_ids
    current_space: u32,
    window_to_space: HashMap<u32, u32>, // window_id -> space_id
}
```

### 4. Enhanced Event Detection

#### Window-Level Events to Add:
```rust
pub enum WindowEvent {
    Created(WindowInfo),
    Destroyed(u32), // window_id
    Moved(u32, WindowBounds),
    Resized(u32, WindowBounds),
    MinimizedStateChanged(u32, bool),
    FullscreenStateChanged(u32, bool),
    SpaceChanged(u32, u32, u32), // window_id, from_space, to_space
    OrderChanged(Vec<u32>), // new stacking order
}
```

### 5. CGWindow Enhanced Properties to Extract

```rust
// Additional properties to extract from CGWindowListCopyWindowInfo
const ADDITIONAL_WINDOW_KEYS: &[&str] = &[
    "kCGWindowStoreType",      // Window backing store type
    "kCGWindowSharingState",   // Screen sharing state
    "kCGWindowMemoryUsage",    // Memory usage by window
    "kCGWindowWorkspace",      // Space/desktop ID (if available)
    "kCGWindowOwnerName",      // Process name
    "kCGWindowIsOnscreen",     // Already tracked but verify
    "kCGWindowBackingLocationVideoMemory", // GPU acceleration
];
```

### 6. Multi-Monitor Support

```rust
pub struct MonitorInfo {
    pub display_id: u32,
    pub bounds: ScreenBounds,
    pub name: String,
    pub is_main: bool,
    pub is_builtin: bool,
    pub refresh_rate: f64,
    pub scale_factor: f64,
}

impl EnhancedAppMonitor {
    fn get_all_monitors() -> Vec<MonitorInfo> {
        // Use NSScreen or CGDisplay APIs
        // Track windows per monitor
    }
    
    fn get_window_monitor(&self, window: &WindowInfo) -> Option<u32> {
        // Determine which monitor contains the window
    }
}
```

### 7. Mission Control Detection

```rust
// Detect Mission Control/Exposé state
fn is_mission_control_active() -> bool {
    // Check for Dock.app windows with specific properties
    // Or use accessibility APIs to check system UI state
}
```

## Implementation Priority

### Phase 1: Core Window State (HIGH PRIORITY)
1. ✅ Detect minimized windows (check `is_onscreen` + layer)
2. ✅ Detect fullscreen windows (compare bounds with screen)
3. ✅ Track window close/open events via polling

### Phase 2: Space Management (MEDIUM PRIORITY)
1. Track active space changes (already have notification)
2. Attempt to track window-to-space assignment
3. Handle multi-desktop scenarios

### Phase 3: Advanced Features (LOW PRIORITY)
1. Mission Control state
2. Window animation states
3. Screen recording optimization
4. Detailed GPU/memory tracking

## Code Changes Required

### 1. Update WindowInfo Structure
```rust
pub struct WindowInfo {
    // ... existing fields ...
    pub state: WindowState,
    pub space_id: Option<u32>,
    pub monitor_id: Option<u32>,
    pub backing_store_type: Option<String>,
    pub is_minimized: bool,
    pub is_fullscreen: bool,
}
```

### 2. Add Polling for Window State Changes
```rust
impl EnhancedAppMonitor {
    fn start_window_polling(&mut self) {
        // Poll every 100ms for window changes
        // Compare with previous state
        // Emit WindowEvent for changes
    }
}
```

### 3. Enhanced Notification Registration
```rust
// Additional notifications to register
const SCREEN_PARAMS_CHANGED: &str = "NSApplicationDidChangeScreenParametersNotification";
const WINDOW_DID_MOVE: &str = "NSWindowDidMoveNotification";
const WINDOW_DID_RESIZE: &str = "NSWindowDidResizeNotification";
const WINDOW_DID_MINIATURIZE: &str = "NSWindowDidMiniaturizeNotification";
const WINDOW_DID_DEMINIATURIZE: &str = "NSWindowDidDeminiaturizeNotification";
const WINDOW_DID_ENTER_FULLSCREEN: &str = "NSWindowDidEnterFullScreenNotification";
const WINDOW_DID_EXIT_FULLSCREEN: &str = "NSWindowDidExitFullScreenNotification";
```

## Testing Strategy

1. **Minimization Test**: Minimize windows and verify detection
2. **Fullscreen Test**: Enter/exit fullscreen and verify
3. **Space Switch Test**: Switch spaces and track window visibility
4. **Multi-Monitor Test**: Move windows between monitors
5. **Rapid State Change Test**: Quickly change states to test coalescing

## Performance Considerations

- Polling frequency: Balance between latency and CPU usage
- Cache window states to avoid redundant CGWindow calls
- Use event coalescing for rapid changes
- Consider background thread for window polling

## Security & Permissions

- Screen Recording permission may be needed for some window details
- Accessibility permission required for detailed UI information
- Some private APIs may require entitlements or be rejected from App Store

## References

- [CGWindow Reference](https://developer.apple.com/documentation/coregraphics/quartz_window_services)
- [NSWorkspace Notifications](https://developer.apple.com/documentation/appkit/nsworkspace)
- [Window Management Best Practices](https://developer.apple.com/design/human-interface-guidelines/macos/windows-and-views/window-anatomy/)
