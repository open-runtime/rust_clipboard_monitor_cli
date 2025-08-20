# Migrating Accessibility Listeners to objc2

## Overview

Your code uses two types of listeners for application changes:
1. **AXObserver** - C-based Accessibility API for UI element changes
2. **NSWorkspace Notifications** - Objective-C notifications for app activation

Both need careful migration to work with objc2 while maintaining thread safety and memory management.

## 1. NSWorkspace Application Change Notifications

### Current Code (cocoa):
```rust
unsafe extern "C" fn workspace_callback(_this: &Object, _cmd: Sel, notification: id) {
    let user_info: id = msg_send![notification, userInfo];
    if user_info == nil { return; }
    
    let app: id = msg_send![user_info, objectForKey:ns_key];
    let bundle_id: id = msg_send![app, bundleIdentifier];
    let name: id = msg_send![app, localizedName];
    let pid: i32 = msg_send![app, processIdentifier];
}
```

### Migrated Code (objc2) - Three Approaches:

#### Approach 1: Selector-Based (Direct Migration)
```rust
use objc2::runtime::{AnyObject, Sel};
use objc2::{msg_send, msg_send_id, ClassType};
use objc2_foundation::{NSNotification, NSString, NSDictionary};
use objc2_app_kit::{NSWorkspace, NSRunningApplication};

// Define the callback with proper signature
unsafe extern "C" fn workspace_callback(
    this: &AnyObject,
    _cmd: Sel,
    notification: &AnyObject
) {
    // IMPORTANT: Wrap in autorelease pool
    NSAutoreleasePool::with(|_pool| {
        // Cast notification to proper type
        let notification = notification as *const _ as *const NSNotification;
        let notification = unsafe { &*notification };
        
        if let Some(user_info) = notification.userInfo() {
            let key = NSString::from_str("NSWorkspaceApplicationKey");
            
            if let Some(app_obj) = user_info.objectForKey(&key) {
                // Try to cast to NSRunningApplication
                if let Some(app) = app_obj.downcast_ref::<NSRunningApplication>() {
                    let pid = app.processIdentifier();
                    
                    let bundle_id = app.bundleIdentifier()
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    
                    let name = app.localizedName()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "Unknown".to_string());
                    
                    // Process the app change
                    with_state(|tracker| {
                        tracker.handle_app_change(name, bundle_id, pid);
                    });
                }
            }
        }
    });
}

// Register the observer
fn setup_workspace_observer() {
    let workspace = NSWorkspace::sharedWorkspace();
    let nc = workspace.notificationCenter();
    
    // Create observer class
    let observer_class = create_observer_class();
    let observer: Retained<AnyObject> = unsafe { 
        msg_send_id![observer_class, new] 
    };
    
    // Register for notifications
    let notification_names = [
        "NSWorkspaceDidActivateApplicationNotification",
        "NSWorkspaceDidDeactivateApplicationNotification",
        "NSWorkspaceDidLaunchApplicationNotification",
        "NSWorkspaceDidTerminateApplicationNotification",
    ];
    
    for name in &notification_names {
        let notif_name = NSNotificationName::from_str(name);
        unsafe {
            nc.addObserver_selector_name_object(
                &observer,
                sel!(workspaceDidChangeApp:),
                Some(&notif_name),
                None,
            );
        }
    }
    
    // Store observer to prevent deallocation
    self.workspace_observer = Some(observer);
}
```

#### Approach 2: Block-Based (Modern, Recommended)
```rust
use block2::ConcreteBlock;
use objc2_foundation::{NSNotificationCenter, NSNotificationName};
use std::sync::Arc;

impl Tracker {
    fn setup_workspace_observer_blocks(&mut self) {
        let workspace = NSWorkspace::sharedWorkspace();
        let nc = workspace.notificationCenter();
        
        // Create a weak reference to avoid retain cycles
        let state = Arc::downgrade(&STATE.get().unwrap());
        
        // Define notification handlers
        let notifications = [
            ("NSWorkspaceDidActivateApplicationNotification", "activate"),
            ("NSWorkspaceDidDeactivateApplicationNotification", "deactivate"),
            ("NSWorkspaceDidLaunchApplicationNotification", "launch"),
            ("NSWorkspaceDidTerminateApplicationNotification", "terminate"),
            ("NSWorkspaceDidHideApplicationNotification", "hide"),
            ("NSWorkspaceDidUnhideApplicationNotification", "unhide"),
        ];
        
        for (notification_name, event_type) in notifications {
            let state_clone = state.clone();
            let event_type = event_type.to_string();
            
            let block = ConcreteBlock::new(move |notification: &NSNotification| {
                // Try to upgrade weak reference
                if let Some(state_arc) = state_clone.upgrade() {
                    if let Ok(mut tracker) = state_arc.lock() {
                        // Extract app info from notification
                        if let Some(user_info) = notification.userInfo() {
                            let key = NSString::from_str("NSWorkspaceApplicationKey");
                            
                            if let Some(app_obj) = user_info.objectForKey(&key) {
                                if let Some(app) = app_obj.downcast_ref::<NSRunningApplication>() {
                                    let event = ApplicationEvent {
                                        event_type: event_type.clone(),
                                        timestamp: Instant::now(),
                                        pid: app.processIdentifier(),
                                        bundle_id: app.bundleIdentifier()
                                            .map(|s| s.to_string())
                                            .unwrap_or_default(),
                                        app_name: app.localizedName()
                                            .map(|s| s.to_string())
                                            .unwrap_or_else(|| "Unknown".to_string()),
                                        is_active: app.isActive(),
                                        is_hidden: app.isHidden(),
                                    };
                                    
                                    tracker.handle_application_event(event);
                                }
                            }
                        }
                    }
                }
            });
            
            let name = NSNotificationName::from_str(notification_name);
            let observer = unsafe {
                nc.addObserverForName_object_queue_usingBlock(
                    Some(&name),
                    None,
                    None,
                    &block,
                )
            };
            
            // Store observer to remove later
            self.notification_observers.push(observer);
        }
    }
}
```

#### Approach 3: Distributed Notifications (System-Wide)
```rust
use objc2_foundation::{NSDistributedNotificationCenter, NSNotificationName};

fn setup_distributed_notifications(&mut self) {
    // For system-wide notifications
    let dnc = NSDistributedNotificationCenter::defaultCenter();
    
    // Monitor screen lock/unlock
    let screen_notifications = [
        "com.apple.screenIsLocked",
        "com.apple.screenIsUnlocked",
        "com.apple.screensaver.didstart",
        "com.apple.screensaver.didstop",
    ];
    
    for notif_name in screen_notifications {
        let name = NSNotificationName::from_str(notif_name);
        let block = ConcreteBlock::new(move |notification: &NSNotification| {
            println!("System event: {}", notif_name);
            // Handle system event
        });
        
        unsafe {
            dnc.addObserverForName_object_queue_usingBlock(
                Some(&name),
                None,
                None,
                &block,
            );
        }
    }
}
```

## 2. AXObserver for Accessibility Events

The AXObserver API is C-based, so it doesn't change much, but integration with objc2 requires care:

### Enhanced AXObserver Setup with objc2:
```rust
use accessibility_sys::{
    AXObserverRef, AXObserverCreate, AXObserverAddNotification,
    AXObserverGetRunLoopSource, AXUIElementRef, AXUIElementCreateApplication,
    kAXErrorSuccess
};
use objc2_foundation::NSAutoreleasePool;

impl Tracker {
    fn setup_ax_observer(&mut self, pid: i32) {
        unsafe {
            let mut observer: AXObserverRef = null_mut();
            
            // Create observer with enhanced callback
            if AXObserverCreate(pid, ax_callback_enhanced, &mut observer) == kAXErrorSuccess {
                let app = AXUIElementCreateApplication(pid);
                
                // Comprehensive notification list
                let notifications = [
                    // Window notifications
                    "AXFocusedWindowChanged",
                    "AXMainWindowChanged",
                    "AXWindowCreated",
                    "AXWindowMoved",
                    "AXWindowResized",
                    "AXWindowMiniaturized",
                    "AXWindowDeminiaturized",
                    
                    // Application state
                    "AXApplicationActivated",
                    "AXApplicationDeactivated",
                    "AXApplicationShown",
                    "AXApplicationHidden",
                    
                    // UI element changes
                    "AXFocusedUIElementChanged",
                    "AXValueChanged",
                    "AXTitleChanged",
                    "AXSelectedChildrenChanged",
                    "AXSelectedTextChanged",
                    "AXSelectedRowsChanged",
                    
                    // Menu events
                    "AXMenuOpened",
                    "AXMenuClosed",
                    "AXMenuItemSelected",
                    
                    // Document changes
                    "AXDocumentChanged",
                    "AXUnitsChanged",
                    "AXSelectedPagesChanged",
                ];
                
                for notif in &notifications {
                    let cfstr = CFString::new(notif);
                    AXObserverAddNotification(
                        observer,
                        app,
                        cfstr.as_concrete_TypeRef() as CFStringRef,
                        null_mut()
                    );
                }
                
                // Add to run loop
                let source = AXObserverGetRunLoopSource(observer);
                CFRunLoopAddSource(
                    CFRunLoop::get_current().as_concrete_TypeRef(),
                    source,
                    kCFRunLoopDefaultMode as CFStringRef
                );
                
                // Store observer
                self.ax_observers.insert(pid, observer as usize);
                CFRelease(app as CFTypeRef);
            }
        }
    }
}

// Enhanced callback with proper memory management
extern "C" fn ax_callback_enhanced(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    user_data: *mut c_void,
) {
    // CRITICAL: Create autorelease pool for the callback
    NSAutoreleasePool::with(|_pool| {
        unsafe {
            let notif = CFString::wrap_under_get_rule(notification).to_string();
            
            // Extract context from the element
            let context = extract_ax_context(element, &notif);
            
            with_state(|tracker| {
                tracker.handle_ax_notification(&notif, context);
            });
        }
    });
}

// Helper to extract context from AX element
fn extract_ax_context(element: AXUIElementRef, notification: &str) -> AXContext {
    unsafe {
        let mut context = AXContext {
            notification: notification.to_string(),
            timestamp: Instant::now(),
            ..Default::default()
        };
        
        // Get window title
        if let Some(title_ref) = get_ax_attribute(element, "AXTitle") {
            if let Some(title) = cfstring_to_string(title_ref) {
                context.window_title = Some(title);
            }
            CFRelease(title_ref);
        }
        
        // Get role
        if let Some(role_ref) = get_ax_attribute(element, "AXRole") {
            if let Some(role) = cfstring_to_string(role_ref) {
                context.element_role = Some(role);
            }
            CFRelease(role_ref);
        }
        
        // Get focused state
        if let Some(focused_ref) = get_ax_attribute(element, "AXFocused") {
            context.is_focused = cfboolean_to_bool(focused_ref);
            CFRelease(focused_ref);
        }
        
        context
    }
}
```

## 3. Thread Safety and Memory Management

### Key Principles:

1. **Always wrap callbacks in NSAutoreleasePool**:
```rust
extern "C" fn any_callback(...) {
    NSAutoreleasePool::with(|_pool| {
        // Your callback code here
        // All autoreleased objects will be cleaned up
    });
}
```

2. **Use weak references to avoid retain cycles**:
```rust
let state_weak = Arc::downgrade(&self.state);
let block = ConcreteBlock::new(move |notification| {
    if let Some(state) = state_weak.upgrade() {
        // Use state
    }
});
```

3. **Ensure main thread for UI operations**:
```rust
use dispatch::Queue;

fn ensure_main_thread<F, R>(f: F) -> R 
where 
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    if NSThread::isMainThread() {
        f()
    } else {
        Queue::main().sync(f)
    }
}
```

## 4. Complete Example: Unified Event System

```rust
use std::collections::HashMap;
use objc2_foundation::{NSAutoreleasePool, NSNotificationCenter};
use objc2_app_kit::{NSWorkspace, NSRunningApplication};

#[derive(Debug, Clone, Serialize)]
pub enum SystemEvent {
    ApplicationActivated {
        pid: i32,
        bundle_id: String,
        name: String,
    },
    ApplicationDeactivated {
        pid: i32,
        bundle_id: String,
    },
    WindowFocusChanged {
        app_name: String,
        window_title: Option<String>,
    },
    ScreenLocked,
    ScreenUnlocked,
    UserSwitched {
        from_user: Option<String>,
        to_user: String,
    },
}

pub struct UnifiedEventMonitor {
    workspace_observers: Vec<Retained<NSObject>>,
    ax_observers: HashMap<i32, AXObserverRef>,
    event_queue: Arc<Mutex<VecDeque<SystemEvent>>>,
    is_running: Arc<AtomicBool>,
}

impl UnifiedEventMonitor {
    pub fn new() -> Self {
        Self {
            workspace_observers: Vec::new(),
            ax_observers: HashMap::new(),
            event_queue: Arc::new(Mutex::new(VecDeque::new())),
            is_running: Arc::new(AtomicBool::new(false)),
        }
    }
    
    pub fn start(&mut self) {
        self.is_running.store(true, Ordering::SeqCst);
        
        // Setup all monitors
        self.setup_workspace_monitors();
        self.setup_accessibility_monitors();
        self.setup_system_monitors();
        
        // Start event processing thread
        self.start_event_processor();
    }
    
    fn setup_workspace_monitors(&mut self) {
        let workspace = NSWorkspace::sharedWorkspace();
        let nc = workspace.notificationCenter();
        
        // Application lifecycle events
        let app_events = [
            ("NSWorkspaceDidActivateApplicationNotification", Self::handle_app_activate),
            ("NSWorkspaceDidDeactivateApplicationNotification", Self::handle_app_deactivate),
            ("NSWorkspaceDidLaunchApplicationNotification", Self::handle_app_launch),
            ("NSWorkspaceDidTerminateApplicationNotification", Self::handle_app_terminate),
        ];
        
        for (notif_name, handler) in app_events {
            let event_queue = self.event_queue.clone();
            
            let block = ConcreteBlock::new(move |notification: &NSNotification| {
                NSAutoreleasePool::with(|_pool| {
                    if let Some(event) = handler(notification) {
                        event_queue.lock().unwrap().push_back(event);
                    }
                });
            });
            
            let name = NSNotificationName::from_str(notif_name);
            let observer = unsafe {
                nc.addObserverForName_object_queue_usingBlock(
                    Some(&name),
                    None,
                    None,
                    &block,
                )
            };
            
            self.workspace_observers.push(observer);
        }
    }
    
    fn handle_app_activate(notification: &NSNotification) -> Option<SystemEvent> {
        if let Some(user_info) = notification.userInfo() {
            let key = NSString::from_str("NSWorkspaceApplicationKey");
            
            if let Some(app_obj) = user_info.objectForKey(&key) {
                if let Some(app) = app_obj.downcast_ref::<NSRunningApplication>() {
                    return Some(SystemEvent::ApplicationActivated {
                        pid: app.processIdentifier(),
                        bundle_id: app.bundleIdentifier()
                            .map(|s| s.to_string())
                            .unwrap_or_default(),
                        name: app.localizedName()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "Unknown".to_string()),
                    });
                }
            }
        }
        None
    }
    
    fn start_event_processor(&self) {
        let event_queue = self.event_queue.clone();
        let is_running = self.is_running.clone();
        
        thread::spawn(move || {
            while is_running.load(Ordering::SeqCst) {
                // Process events in batches
                let events: Vec<SystemEvent> = {
                    let mut queue = event_queue.lock().unwrap();
                    queue.drain(..).collect()
                };
                
                for event in events {
                    // Process each event
                    Self::process_event(event);
                }
                
                thread::sleep(Duration::from_millis(100));
            }
        });
    }
    
    fn process_event(event: SystemEvent) {
        match event {
            SystemEvent::ApplicationActivated { pid, bundle_id, name } => {
                println!("App activated: {} ({}) [{}]", name, bundle_id, pid);
                // Update tracking state
            }
            SystemEvent::WindowFocusChanged { app_name, window_title } => {
                println!("Window focus: {} - {:?}", app_name, window_title);
                // Track window changes
            }
            _ => {
                // Handle other events
            }
        }
    }
}

impl Drop for UnifiedEventMonitor {
    fn drop(&mut self) {
        // Clean up observers
        self.is_running.store(false, Ordering::SeqCst);
        
        // Remove AX observers
        for (_, observer) in self.ax_observers.drain() {
            unsafe {
                let source = AXObserverGetRunLoopSource(observer as AXObserverRef);
                CFRunLoopRemoveSource(
                    CFRunLoop::get_current().as_concrete_TypeRef(),
                    source,
                    kCFRunLoopDefaultMode as CFStringRef
                );
                CFRelease(observer as CFTypeRef);
            }
        }
        
        // Workspace observers are automatically cleaned up when dropped
    }
}
```

## 5. Best Practices

### DO:
- ✅ Always use NSAutoreleasePool in callbacks
- ✅ Use weak references to prevent retain cycles
- ✅ Check thread safety for UI operations
- ✅ Handle nil/None cases explicitly
- ✅ Clean up observers in Drop implementation
- ✅ Use blocks for new code (more modern)

### DON'T:
- ❌ Forget autorelease pools in callbacks
- ❌ Store strong references in blocks
- ❌ Assume you're on the main thread
- ❌ Mix old and new patterns inconsistently
- ❌ Ignore memory management

## 6. Debugging Tips

```rust
// Debug helper for tracking observer lifecycle
#[cfg(debug_assertions)]
impl Debug for UnifiedEventMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UnifiedEventMonitor {{\n")?;
        write!(f, "  workspace_observers: {} active\n", self.workspace_observers.len())?;
        write!(f, "  ax_observers: {} active\n", self.ax_observers.len())?;
        write!(f, "  event_queue: {} pending\n", self.event_queue.lock().unwrap().len())?;
        write!(f, "  is_running: {}\n", self.is_running.load(Ordering::SeqCst))?;
        write!(f, "}}")
    }
}

// Log all notifications for debugging
fn debug_notification(notification: &NSNotification) {
    if let Some(name) = notification.name() {
        eprintln!("[DEBUG] Notification: {}", name);
    }
    if let Some(user_info) = notification.userInfo() {
        eprintln!("[DEBUG] UserInfo keys: {:?}", user_info.allKeys());
    }
}
```

## Summary

The migration of accessibility listeners requires:
1. **Proper memory management** with NSAutoreleasePool
2. **Type safety** with objc2's strongly typed APIs
3. **Thread safety** considerations
4. **Modern patterns** like blocks over selectors
5. **Comprehensive cleanup** in Drop implementations

The objc2 migration actually makes the code safer and more maintainable, even though the initial conversion requires careful attention to these details.
