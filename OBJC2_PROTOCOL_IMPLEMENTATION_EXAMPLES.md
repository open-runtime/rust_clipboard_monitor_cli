# objc2 0.6.x Protocol Implementation Examples

## Complete Working Examples for Common Protocols

### 1. NSApplicationDelegate Implementation

```rust
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSApplicationActivationPolicy};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol};

define_class!(
    #[unsafe(super(NSObject))]
    #[derive(Debug)]
    pub struct AppDelegate;
    
    // Must implement NSObjectProtocol as base
    unsafe impl NSObjectProtocol for AppDelegate {}
    
    // NSApplicationDelegate implementation
    unsafe impl NSApplicationDelegate for AppDelegate {
        #[method(applicationDidFinishLaunching:)]
        fn application_did_finish_launching(&self, notification: &NSNotification) {
            println!("App finished launching");
            // Your initialization code here
        }
        
        #[method(applicationWillTerminate:)]
        fn application_will_terminate(&self, notification: &NSNotification) {
            println!("App will terminate");
            // Cleanup code here
        }
        
        #[method(applicationDidBecomeActive:)]
        fn application_did_become_active(&self, notification: &NSNotification) {
            println!("App became active");
        }
        
        #[method(applicationDidResignActive:)]
        fn application_did_resign_active(&self, notification: &NSNotification) {
            println!("App resigned active");
        }
        
        #[method(applicationShouldTerminateAfterLastWindowClosed:)]
        fn should_terminate_after_last_window_closed(&self, sender: &NSApplication) -> bool {
            true // Return true to quit when last window closes
        }
    }
);

// Implementation methods outside the macro
impl AppDelegate {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe {
            msg_send![Self::alloc(), init]
        }
    }
}
```

### 2. NSWorkspace Notification Observer

```rust
use objc2::{define_class, msg_send, sel, MainThreadMarker};
use objc2::rc::Retained;
use objc2_app_kit::{NSWorkspace, NSRunningApplication};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol};

define_class!(
    #[unsafe(super(NSObject))]
    pub struct WorkspaceObserver;
    
    unsafe impl NSObjectProtocol for WorkspaceObserver {}
    
    // Custom notification handling methods
    unsafe impl WorkspaceObserver {
        #[method(workspaceDidActivateApplication:)]
        fn workspace_did_activate_application(&self, notification: &NSNotification) {
            // Extract the NSRunningApplication from the notification
            println!("Application activated");
        }
        
        #[method(workspaceDidDeactivateApplication:)]
        fn workspace_did_deactivate_application(&self, notification: &NSNotification) {
            println!("Application deactivated");
        }
    }
);

impl WorkspaceObserver {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe {
            let observer: Retained<Self> = msg_send![Self::alloc(), init];
            
            // Register for notifications
            let workspace = NSWorkspace::sharedWorkspace();
            let nc = workspace.notificationCenter();
            
            // Add observer for app activation
            let _: () = msg_send![
                nc,
                addObserver: &*observer,
                selector: sel!(workspaceDidActivateApplication:),
                name: Some(&*NSWorkspaceDidActivateApplicationNotification),
                object: None
            ];
            
            observer
        }
    }
}
```

### 3. Custom View with Event Handling

```rust
use objc2::{define_class, msg_send, sel, MainThreadMarker};
use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSEvent, NSResponder};
use objc2_foundation::{NSObject, NSObjectProtocol, CGRect};

define_class!(
    #[unsafe(super(NSView))]
    pub struct CustomView;
    
    unsafe impl NSObjectProtocol for CustomView {}
    
    // NSResponder methods for event handling
    unsafe impl CustomView {
        #[method(mouseDown:)]
        fn mouse_down(&self, event: &NSEvent) {
            println!("Mouse clicked at: {:?}", event.locationInWindow());
        }
        
        #[method(keyDown:)]
        fn key_down(&self, event: &NSEvent) {
            println!("Key pressed: {:?}", event.keyCode());
        }
        
        #[method(drawRect:)]
        fn draw_rect(&self, dirty_rect: CGRect) {
            // Custom drawing code
            unsafe {
                // Call super's drawRect first
                let _: () = msg_send![super(self), drawRect: dirty_rect];
            }
            // Your drawing code here
        }
    }
);
```

### 4. NSPasteboard Change Observer

```rust
use objc2::{define_class, msg_send, sel};
use objc2::rc::Retained;
use objc2_app_kit::NSPasteboard;
use objc2_foundation::{NSObject, NSObjectProtocol, NSTimer};

define_class!(
    #[unsafe(super(NSObject))]
    pub struct PasteboardObserver {
        last_change_count: u64,
    }
    
    unsafe impl NSObjectProtocol for PasteboardObserver {}
    
    unsafe impl PasteboardObserver {
        #[method(checkPasteboard:)]
        fn check_pasteboard(&mut self, _timer: &NSTimer) {
            let pasteboard = NSPasteboard::generalPasteboard();
            let current_count = pasteboard.changeCount();
            
            if current_count != self.last_change_count {
                self.last_change_count = current_count;
                println!("Pasteboard changed!");
                // Handle pasteboard change
            }
        }
    }
);
```

### 5. Using extern_methods! (New Pattern)

```rust
use objc2::{extern_methods, msg_send, sel};
use objc2::rc::Retained;
use objc2_foundation::{NSObject, NSString};

// The impl is OUTSIDE the macro now
unsafe impl NSString {
    extern_methods!(
        #[method_id(stringWithUTF8String:)]
        pub fn stringWithUTF8String(string: *const u8) -> Option<Retained<Self>>;
        
        #[method(UTF8String)]
        pub fn UTF8String(&self) -> *const u8;
    );
}
```

## Key Patterns to Remember

### 1. Protocol Implementation Location
- **Inside define_class!**: Protocol implementations
- **Outside define_class!**: Regular impl blocks for your methods

### 2. Method Attributes
- `#[method(selector:)]` - Regular instance methods
- `#[method_id(selector)]` - Methods returning Retained<T>
- `#[method(selector:with:params:)]` - Methods with multiple parameters

### 3. Calling Super
```rust
unsafe {
    let _: () = msg_send![super(self), methodName: param];
}
```

### 4. Creating Instances
```rust
pub fn new() -> Retained<Self> {
    unsafe {
        msg_send![Self::alloc(), init]
    }
}
```

### 5. Class Registration
The `define_class!` macro automatically handles class registration with the Objective-C runtime. You don't need to manually register classes.

## Common Gotchas

1. **Protocol methods must match Objective-C signatures exactly**
   - Parameter types must be correct
   - Return types must match
   - Method names must match selectors

2. **MainThreadMarker for UI classes**
   - UI-related classes often require MainThreadMarker
   - This ensures thread safety for UI operations

3. **Memory management**
   - Use Retained<T> for owned references
   - Use & references for borrowed values
   - The framework handles retain/release automatically

4. **Notification names**
   - Use the predefined constants from objc2_app_kit/objc2_foundation
   - These are typically static strings like NSWorkspaceDidActivateApplicationNotification

## Testing Your Implementation

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use objc2::MainThreadMarker;
    
    #[test]
    fn test_app_delegate_creation() {
        // MainThreadMarker is only available on the main thread
        // In tests, you might need to handle this differently
        if let Some(mtm) = MainThreadMarker::new() {
            let delegate = AppDelegate::new(mtm);
            assert!(!delegate.is_null());
        }
    }
}
```

## Debugging Tips

1. **Use println! in methods to verify they're being called**
2. **Check selector names carefully - they must match exactly**
3. **Verify protocol conformance with Objective-C runtime tools**
4. **Use RUST_BACKTRACE=1 for detailed error messages**

This guide should help you implement any Objective-C protocol in Rust using objc2 0.6.x!
