# Tab Switching Detection: Implementation Summary

## Quick Answer: It's Complex but Doable

Tab switching within applications is **significantly harder** than app switching because:
- **No universal API** exists for tabs across all applications
- Each app implements tabs differently  
- Browser security prevents direct content access
- Requires multiple detection strategies

## The Five Approaches (Ranked by Practicality)

### 1. 🥇 **Hybrid Approach** (Recommended)
Combine multiple methods based on the active application:
- **Browsers**: Use AppleScript/JXA
- **IDEs**: Use Accessibility API
- **Others**: Use keyboard monitoring as fallback

**Pros**: Best coverage, good accuracy
**Cons**: More complex implementation

### 2. 🥈 **AppleScript/JXA** (Best for Browsers)
Direct scripting access to Safari, Chrome, and other scriptable apps.

**Pros**: Accurate tab info including URLs
**Cons**: High latency (~500ms), requires permissions

### 3. 🥉 **Accessibility API** (Most Universal)
Works for any app with standard tab UI elements.

**Pros**: Works across most apps
**Cons**: No URL access, requires UI hierarchy traversal

### 4. **Keyboard Shortcuts** (Fallback)
Monitor Cmd+Shift+], Cmd+Shift+[, Cmd+1-9, etc.

**Pros**: Low latency, minimal overhead
**Cons**: Misses mouse clicks, false positives

### 5. **Browser Extensions** (Most Accurate)
Native messaging with browser extensions.

**Pros**: 100% accurate, real-time
**Cons**: Requires extension installation per browser

## Implementation Roadmap

### Phase 1: Basic Tab Detection (1-2 days)
```rust
// Add to your existing main.rs
mod tab_monitor;
use tab_monitor::TabMonitor;

// In your Tracker struct
tab_monitor: Option<TabMonitor>,

// When app switches
self.tab_monitor.set_current_app(app_name, pid);
let tabs = self.tab_monitor.get_current_tabs();
```

### Phase 2: Browser Support (2-3 days)
- Implement AppleScript execution for Safari/Chrome
- Add URL extraction
- Cache results for performance

### Phase 3: Enhanced Detection (3-5 days)
- Add keyboard shortcut monitoring
- Implement tab click detection
- Add IDE-specific handling

### Phase 4: Production Ready (1 week)
- Error handling and recovery
- Performance optimization
- Configuration system

## Key Implementation Details

### For Browsers (Safari/Chrome)
```javascript
// JXA script executed via osascript
const Safari = Application('Safari');
const tabs = Safari.windows[0].tabs();
tabs.forEach(tab => {
    console.log(tab.name(), tab.url());
});
```

### For Generic Apps (Accessibility)
```rust
// Find tab groups in window
let tab_groups = find_elements_by_role(window, "AXTabGroup");

// Monitor for changes
AXObserverAddNotification(
    observer,
    tab_group,
    "AXSelectedChildrenChanged", // Tab switched
    null
);
```

### Performance Considerations
- **Cache aggressively**: Tab info doesn't change that often
- **Use async where possible**: Don't block on AppleScript calls
- **Rate limit**: Don't poll more than 2x per second
- **Be selective**: Only monitor apps user cares about

## Security & Privacy

### Required Permissions
1. **Accessibility**: System Settings → Privacy & Security → Accessibility
2. **Automation**: For AppleScript (per-app basis)
3. **Input Monitoring**: For keyboard shortcuts (optional)

### Privacy Best Practices
- Don't track incognito/private windows
- Exclude banking/medical sites
- Allow user to disable for specific apps
- Don't store full URLs with query parameters

## What You Can and Can't Get

### ✅ **Can Get**
- Tab titles
- Tab count
- Active tab index
- Tab order
- URLs (browsers via AppleScript)
- File paths (IDEs via Accessibility)

### ❌ **Can't Get** (easily)
- Tab content/DOM
- Password fields
- Incognito tab URLs
- Cross-origin iframe content
- Tabs in Electron apps (Discord, Slack)

## Common Gotchas

1. **Safari's Radio Groups**: Safari uses AXRadioGroup not AXTabGroup
2. **Chrome's Delays**: Chrome AppleScript can be slow with many tabs
3. **Firefox Limitations**: Limited AppleScript support
4. **VS Code Complexity**: Multiple tab groups (editors, terminals, etc.)
5. **Permission Popups**: Users will see automation permission requests

## Quick Start Code

Add this to your existing `main.rs`:

```rust
// When handling app change
fn handle_app_change(&mut self, name: String, bundle: String, pid: i32) {
    // ... existing code ...
    
    // Add tab monitoring
    if self.tab_monitor.is_none() {
        self.tab_monitor = Some(TabMonitor::new());
    }
    
    if let Some(tab_mon) = &mut self.tab_monitor {
        tab_mon.set_current_app(name.clone(), pid);
        
        // Get tabs after short delay (let UI settle)
        thread::sleep(Duration::from_millis(100));
        
        let tabs = tab_mon.get_current_tabs();
        if let Some(active) = tabs.iter().find(|t| t.is_active) {
            println!("Active tab: {}", active.title);
            if let Some(url) = &active.url {
                println!("URL: {}", url);
            }
        }
    }
}
```

## Testing Commands

```bash
# Test Safari AppleScript
osascript -l JavaScript -e "Application('Safari').windows[0].currentTab.name()"

# Test Chrome AppleScript  
osascript -l JavaScript -e "Application('Google Chrome').windows[0].activeTab.title()"

# Check Accessibility permissions
tccutil reset Accessibility com.yourapp.bundle

# Monitor Accessibility notifications
log stream --predicate 'eventMessage contains "AX"'
```

## Decision Matrix

| If you need... | Use this approach |
|---------------|-------------------|
| Browser URLs | AppleScript/JXA |
| IDE file paths | Accessibility API |
| Real-time accuracy | Browser extension |
| Low latency | Keyboard monitoring |
| Broad app support | Hybrid solution |

## Bottom Line

**Start with**: Accessibility API + AppleScript for browsers
**Add if needed**: Keyboard monitoring for better responsiveness
**Consider later**: Browser extensions for production use

The implementation in `src/tab_monitor.rs` provides a solid foundation that:
- Works with your existing objc2 migration
- Handles browsers and IDEs
- Gracefully degrades when permissions are missing
- Can be extended with additional detection methods

Most importantly: **Don't try to implement everything at once**. Start with basic tab detection for your most-used apps and expand from there.
