# Complex Migration Patterns: Event Taps and Accessibility APIs

## Challenge Areas in Your Code

Your code has several complex areas that require special attention during migration:

1. **CGEventTap callbacks** - C function pointers with objc2
2. **Accessibility API integration** - Mixing C and Objective-C APIs
3. **Global state management** - Thread-safe access patterns
4. **NSWorkspace notifications** - Observer pattern migration
5. **Dynamic class creation** - Runtime class registration

## 1. CGEventTap with objc2

The CGEventTap APIs are C-based, not Objective-C, so they remain largely unchanged. However, the integration with NSAutoreleasePool needs updating:

### Current Code Pattern:
```rust
extern "C" fn keyboard_event_callback(
    _proxy: *mut c_void,
    event_type: CGEventType,
    event: *mut c_void,
    _user_info: *mut c_void,
) -> *mut c_void {
    unsafe {
        // No autorelease pool here - potential memory leak!
        // ... handle event ...
    }
    event
}
```

### Migrated Pattern:
```rust
extern "C" fn keyboard_event_callback(
    _proxy: *mut c_void,
    event_type: CGEventType,
    event: *mut c_void,
    _user_info: *mut c_void,
) -> *mut c_void {
    // Create autorelease pool for callback
    NSAutoreleasePool::with(|_pool| {
        unsafe {
            // ... handle event ...
            // Any Objective-C objects created here will be properly released
        }
    });
    event
}
```

## 2. Mixing Accessibility API with objc2

Your code uses `accessibility-sys` with objc2. This requires careful type conversion:

### Pattern for AXUIElement to objc2:
```rust
use objc2::rc::Retained;
use objc2::runtime::AnyObject;

unsafe fn ax_element_to_objc2(element: AXUIElementRef) -> Option<Retained<AnyObject>> {
    if element.is_null() {
        return None;
    }
    
    // AXUIElementRef is already retained, so we use retain_autoreleased
    let obj = element as *mut AnyObject;
    Some(Retained::retain_autoreleased(obj))
}
```

## 3. Thread-Safe Global State

Your current pattern using `OnceLock` and `Arc<Mutex<T>>` remains valid, but can be enhanced:

### Enhanced Pattern with objc2:
```rust
use std::sync::OnceLock;
use objc2_foundation::{MainThreadMarker, is_main_thread};

static STATE: OnceLock<Arc<Mutex<Tracker>>> = OnceLock::new();

// Ensure main thread for UI operations
fn with_state_on_main<F, R>(f: F) -> R
where
    F: FnOnce(&mut Tracker, MainThreadMarker) -> R + Send + 'static,
    R: Send + 'static,
{
    if is_main_thread() {
        let mtm = MainThreadMarker::new().unwrap();
        let state = STATE.get().expect("State not initialized");
        let mut tracker = state.lock().unwrap();
        f(&mut *tracker, mtm)
    } else {
        // Dispatch to main thread
        dispatch::Queue::main().sync(|| {
            let mtm = MainThreadMarker::new().unwrap();
            let state = STATE.get().expect("State not initialized");
            let mut tracker = state.lock().unwrap();
            f(&mut *tracker, mtm)
        })
    }
}
```

## 4. NSWorkspace Notifications with Blocks

The most modern approach uses blocks instead of selectors:

### Old Selector-Based:
```rust
let _: () = msg_send![nc,
    addObserver:observer
    selector:sel!(workspaceDidActivateApp:)
    name:notif_name
    object:nil
];
```

### New Block-Based:
```rust
use block2::ConcreteBlock;
use objc2_foundation::{NSNotificationCenter, NSNotificationName};

let nc = NSNotificationCenter::defaultCenter();
let name = NSNotificationName::from_str("NSWorkspaceDidActivateApplicationNotification");

let block = ConcreteBlock::new(|notification: &NSNotification| {
    // Handle notification in closure
    if let Some(user_info) = notification.userInfo() {
        // Process user info
    }
});

let observer = nc.addObserverForName_object_queue_usingBlock(
    Some(&name),
    None,
    None,
    &block,
);

// Store observer to remove later
self.notification_observer = Some(observer);
```

## 5. Dynamic Class Creation

Creating custom Objective-C classes is more type-safe with objc2:

### Complete Example:
```rust
use objc2::declare::{ClassBuilder, Ivar, IvarType};
use objc2::runtime::{AnyClass, NSObject};
use objc2::{sel, msg_send, ClassType};

#[repr(C)]
struct FocusObserverIvars {
    tracker_ptr: *mut c_void,
}

unsafe impl IvarType for FocusObserverIvars {
    type Type = FocusObserverIvars;
    const ENCODING: objc2::Encoding = objc2::Encoding::Struct(
        "FocusObserverIvars",
        &[objc2::Encoding::Pointer(&objc2::Encoding::Void)],
    );
}

fn create_observer_class() -> &'static AnyClass {
    static REGISTER: OnceLock<&'static AnyClass> = OnceLock::new();
    
    REGISTER.get_or_init(|| {
        let mut builder = ClassBuilder::new("FocusObserver", NSObject::class())
            .expect("Class already registered");
        
        // Add ivar to store tracker pointer
        builder.add_ivar::<FocusObserverIvars>("tracker");
        
        // Add methods
        unsafe {
            builder.add_method(
                sel!(init),
                init_observer as unsafe extern "C" fn(&mut AnyObject, Sel) -> Option<&mut AnyObject>,
            );
            
            builder.add_method(
                sel!(workspaceDidActivateApp:),
                workspace_callback as unsafe extern "C" fn(&AnyObject, Sel, &NSNotification),
            );
        }
        
        builder.register()
    })
}

unsafe extern "C" fn init_observer(
    this: &mut AnyObject,
    _cmd: Sel,
) -> Option<&mut AnyObject> {
    // Call super init
    let this: Option<&mut AnyObject> = msg_send![super(this, NSObject::class()), init];
    this
}
```

## 6. Clipboard Operations Deep Dive

The NSPasteboard operations become much cleaner:

### Complex Clipboard Reading:
```rust
use objc2_app_kit::{NSPasteboard, NSPasteboardType};
use objc2_foundation::{NSString, NSArray, NSURL, NSData};

impl Tracker {
    fn get_all_clipboard_formats(&self) -> HashMap<String, serde_json::Value> {
        let mut formats = HashMap::new();
        
        let pasteboard = NSPasteboard::generalPasteboard();
        
        // Get all available types
        if let Some(types) = unsafe { pasteboard.types() } {
            for type_name in types.iter() {
                match type_name.as_str() {
                    "public.utf8-plain-text" => {
                        if let Some(string) = unsafe { 
                            pasteboard.stringForType(&type_name) 
                        } {
                            formats.insert(
                                "text".to_string(), 
                                serde_json::Value::String(string.to_string())
                            );
                        }
                    }
                    "public.html" => {
                        if let Some(data) = unsafe { 
                            pasteboard.dataForType(&type_name) 
                        } {
                            if let Ok(html) = String::from_utf8(data.bytes().to_vec()) {
                                formats.insert(
                                    "html".to_string(),
                                    serde_json::Value::String(html)
                                );
                            }
                        }
                    }
                    "public.file-url" => {
                        // Handle file URLs
                        let url_class = NSURL::class();
                        let classes = NSArray::from_slice(&[url_class]);
                        
                        if let Some(urls) = unsafe {
                            pasteboard.readObjectsForClasses_options(&classes, None)
                        } {
                            let paths: Vec<String> = urls.iter()
                                .filter_map(|item| {
                                    item.downcast::<NSURL>()
                                        .and_then(|url| url.path())
                                        .map(|path| path.to_string())
                                })
                                .collect();
                            
                            formats.insert(
                                "files".to_string(),
                                serde_json::json!(paths)
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        
        formats
    }
}
```

## 7. Error Handling Patterns

objc2 enables proper error handling:

### Old Pattern (No Error Handling):
```rust
let result: id = msg_send![object, methodThatMightFail];
// Hope it worked...
```

### New Pattern (With Error Handling):
```rust
use objc2_foundation::NSError;

// Method that returns Result
let result: Result<Retained<NSString>, Retained<NSError>> = unsafe {
    object.methodThatMightFail()
};

match result {
    Ok(string) => {
        println!("Success: {}", string);
    }
    Err(error) => {
        eprintln!("Error: {} (Code: {})", 
            error.localizedDescription(),
            error.code()
        );
    }
}
```

## 8. Performance-Critical Sections

For performance-critical sections, you can still use direct message sending:

```rust
use objc2::{msg_send, msg_send_id};

// When you need maximum performance and know the types
unsafe {
    // Direct message send for primitives
    let count: NSInteger = msg_send![array, count];
    
    // Direct message send for objects (with retain)
    let object: Retained<AnyObject> = msg_send_id![array, objectAtIndex: 0];
}
```

## 9. Debugging Tips

### Enable Debug Assertions:
```toml
[dependencies]
objc2 = { version = "0.5", features = ["debug-assertions"] }
```

### Use Type Checking:
```rust
// Verify object is expected type
if let Some(window) = object.downcast::<NSWindow>() {
    // Safe to use as NSWindow
} else {
    eprintln!("Object is not an NSWindow!");
}
```

### Memory Leak Detection:
```rust
// Use Instruments or these debug helpers
#[cfg(debug_assertions)]
{
    let retain_count: NSUInteger = unsafe { msg_send![object, retainCount] };
    eprintln!("Retain count: {}", retain_count);
}
```

## 10. Common Pitfalls and Solutions

### Pitfall 1: Forgetting MainThreadMarker
**Problem**: UI operations crash when not on main thread
**Solution**: Use `MainThreadMarker` to enforce main thread

### Pitfall 2: Double-Release
**Problem**: Using `Retained::from_raw` on already-retained objects
**Solution**: Use `Retained::retain_autoreleased` for autoreleased objects

### Pitfall 3: Missing Autorelease Pool
**Problem**: Memory leaks in callbacks
**Solution**: Wrap callbacks in `NSAutoreleasePool::with`

### Pitfall 4: Wrong Encoding
**Problem**: Type mismatch in message sends
**Solution**: Use proper types and let objc2 handle encoding

## Summary

The migration to objc2 for complex patterns requires:

1. **Careful type conversion** between C and Objective-C APIs
2. **Proper memory management** with autorelease pools
3. **Thread safety** considerations for UI operations
4. **Modern patterns** like blocks instead of selectors
5. **Error handling** with Result types

The initial effort pays off with:
- Fewer runtime crashes
- Better performance
- Cleaner code
- Easier maintenance
