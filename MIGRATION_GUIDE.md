# Migration Guide: cocoa to objc2

## Import Changes

### Old (cocoa/objc):
```rust
use cocoa::appkit::{NSApp, NSApplication, NSApplicationActivationPolicyProhibited, NSPasteboard};
use cocoa::base::{id, nil};
use cocoa::foundation::NSAutoreleasePool;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
```

### New (objc2):
```rust
use objc2::runtime::{AnyClass, AnyObject, Sel, ProtocolObject};
use objc2::{msg_send, msg_send_id, sel, ClassType};
use objc2::rc::{Id, Retained};
use objc2_foundation::{NSString, NSArray, NSThread, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSPasteboard, NSWorkspace, NSRunningApplication};
use block2::Block;
```

## Key Type Changes

### 1. `id` type replacement
- **Old**: `id` (type alias for `*mut Object`)
- **New**: `&AnyObject` or `Retained<T>` for owned objects

### 2. `nil` replacement
- **Old**: `nil` constant
- **New**: Use `None` with `Option<&T>` or `Option<Retained<T>>`

## Specific Code Migrations

### NSAutoreleasePool

**Old (cocoa):**
```rust
unsafe {
    let pool = NSAutoreleasePool::new(nil);
    // ... your code ...
    let _: () = msg_send![pool, drain];
}
```

**New (objc2):**
```rust
use objc2_foundation::NSAutoreleasePool;

unsafe {
    let pool = NSAutoreleasePool::new();
    // ... your code ...
    // pool is automatically drained when dropped
}

// Or use the convenience method:
NSAutoreleasePool::with(|_pool| {
    // ... your code ...
});
```

### NSApplication (NSApp)

**Old (cocoa):**
```rust
unsafe {
    let app = NSApp();
    app.setActivationPolicy_(NSApplicationActivationPolicyProhibited);
}
```

**New (objc2):**
```rust
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

unsafe {
    let app = NSApplication::sharedApplication();
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
}
```

### NSPasteboard

**Old (cocoa):**
```rust
unsafe {
    let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
    let change_count: i64 = msg_send![pasteboard, changeCount];
    let string: id = msg_send![pasteboard, stringForType: nil];
}
```

**New (objc2):**
```rust
use objc2_app_kit::{NSPasteboard, NSPasteboardType};
use objc2_foundation::NSString;

unsafe {
    let pasteboard = NSPasteboard::generalPasteboard();
    let change_count = pasteboard.changeCount();
    
    // For string content:
    let string_type = NSPasteboardType::string();
    if let Some(string) = pasteboard.stringForType(&string_type) {
        let rust_string = string.to_string();
    }
}
```

### NSWorkspace

**Old (cocoa):**
```rust
unsafe {
    let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
    let frontmost: id = msg_send![workspace, frontmostApplication];
    let bundle_id: id = msg_send![frontmost, bundleIdentifier];
}
```

**New (objc2):**
```rust
use objc2_app_kit::{NSWorkspace, NSRunningApplication};

unsafe {
    let workspace = NSWorkspace::sharedWorkspace();
    if let Some(frontmost) = workspace.frontmostApplication() {
        if let Some(bundle_id) = frontmost.bundleIdentifier() {
            let rust_string = bundle_id.to_string();
        }
    }
}
```

### Message Sending

**Old (objc):**
```rust
let result: id = msg_send![object, method];
let value: i32 = msg_send![object, intValue];
let _: () = msg_send![object, performAction];
```

**New (objc2):**
```rust
// For methods returning objects (retained):
let result = msg_send_id![object, method];

// For methods returning primitives:
let value: i32 = msg_send![object, intValue];

// For void methods:
msg_send![object, performAction];
```

### Creating Classes and Observers

**Old (cocoa/objc):**
```rust
use objc::declare::ClassDecl;

fn create_observer_class() -> *const Class {
    let superclass = class!(NSObject);
    let mut decl = ClassDecl::new("FocusObserver", superclass).unwrap();
    
    unsafe {
        decl.add_method(
            sel!(workspaceDidActivateApp:),
            workspace_callback as extern "C" fn(&Object, Sel, id),
        );
    }
    
    decl.register()
}
```

**New (objc2):**
```rust
use objc2::declare::{ClassBuilder, Ivar, IvarType};
use objc2::runtime::AnyClass;

fn create_observer_class() -> &'static AnyClass {
    let mut builder = ClassBuilder::new("FocusObserver", NSObject::class()).unwrap();
    
    unsafe {
        builder.add_method(
            sel!(workspaceDidActivateApp:),
            workspace_callback as unsafe extern "C" fn(&AnyObject, Sel, &AnyObject),
        );
    }
    
    builder.register()
}
```

### String Handling

**Old:**
```rust
unsafe {
    let ns_str: id = msg_send![class!(NSString), alloc];
    let ns_str: id = msg_send![ns_str, initWithBytes:str.as_ptr() 
                                       length:str.len() 
                                       encoding:4]; // NSUTF8StringEncoding
    let c_str: *const i8 = msg_send![ns_str, UTF8String];
    let rust_str = CStr::from_ptr(c_str).to_string_lossy().to_string();
}
```

**New:**
```rust
use objc2_foundation::NSString;

// Creating NSString from Rust string:
let ns_string = NSString::from_str("Hello, World!");

// Getting Rust string from NSString:
let rust_string = ns_string.to_string();
```

## Memory Management

### Old (manual retain/release):
```rust
unsafe {
    let obj: id = msg_send![class, new];
    let _: () = msg_send![obj, retain];
    // ... use obj ...
    let _: () = msg_send![obj, release];
}
```

### New (automatic with Retained):
```rust
unsafe {
    // Retained<T> automatically manages retain/release
    let obj: Retained<AnyObject> = msg_send_id![class, new];
    // obj is automatically released when dropped
}
```

## Error Handling

The new objc2 provides better error handling:

```rust
// Old: No error handling in message sends
let result: id = msg_send![object, methodThatMightFail];

// New: Can use Result types with proper error handling
use objc2_foundation::NSError;

let result: Result<Retained<AnyObject>, Retained<NSError>> = 
    unsafe { object.methodThatMightFail() };

match result {
    Ok(value) => { /* handle success */ },
    Err(error) => { /* handle error */ },
}
```

## Common Patterns

### 1. Getting Current Application
```rust
// Old
let app = NSApp();

// New
let app = NSApplication::sharedApplication();
```

### 2. Working with NSNotificationCenter
```rust
// New approach with objc2
use objc2_foundation::{NSNotificationCenter, NSNotificationName};

unsafe {
    let nc = NSNotificationCenter::defaultCenter();
    let notification_name = NSNotificationName::from_str("MyNotification");
    
    // Add observer with block
    let observer = nc.addObserverForName_object_queue_usingBlock(
        Some(&notification_name),
        None,
        None,
        &block2::ConcreteBlock::new(|notification| {
            // Handle notification
        }),
    );
}
```

### 3. Type Casting
```rust
// Old
let window: id = msg_send![app, mainWindow];

// New - with proper type safety
use objc2_app_kit::NSWindow;

let window: Option<Retained<NSWindow>> = unsafe {
    msg_send_id![&app, mainWindow]
};
```

## Benefits of Migration

1. **Type Safety**: Stronger typing prevents runtime errors
2. **Memory Safety**: Automatic reference counting through `Retained<T>`
3. **Better Documentation**: objc2 crates have better inline documentation
4. **Active Maintenance**: objc2 is actively maintained and updated
5. **Rust Idioms**: More idiomatic Rust code with Option, Result, etc.
6. **Compile-time Checks**: Many errors caught at compile time instead of runtime

## Migration Checklist

- [ ] Update Cargo.toml dependencies
- [ ] Replace all `use cocoa::*` imports with `objc2_*` equivalents
- [ ] Replace `id` types with appropriate `&AnyObject` or `Retained<T>`
- [ ] Replace `nil` with `None` or proper null checks
- [ ] Update NSAutoreleasePool usage
- [ ] Update message sending syntax
- [ ] Test thoroughly on macOS
- [ ] Remove unused imports
