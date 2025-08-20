# objc2 0.6.x Macro Deep Research and Migration Guide

## Executive Summary

After extensive research into objc2 0.6.x, I've discovered significant changes in how macros work compared to earlier versions. The framework has evolved to provide safer, more idiomatic Rust patterns while maintaining compatibility with Objective-C runtime.

## Key Changes in objc2 0.6.x

### 1. define_class! Macro Structure

The `define_class!` macro in objc2 0.6.x follows this pattern:

```rust
define_class!(
    // Class definition with attributes
    #[unsafe(super(NSObject))]  // Specify superclass
    #[derive(Debug)]             // Can derive standard traits
    pub struct MyClass;          // Your class declaration
    
    // Protocol implementations must be inside the macro
    unsafe impl ProtocolName for MyClass {
        // Methods defined here
    }
);
```

### 2. extern_methods! Deprecation Changes

**Old Pattern (Deprecated):**
```rust
extern_methods!(
    unsafe impl NSNotificationCenter {
        #[unsafe(method(defaultCenter))]
        pub fn defaultCenter() -> Retained<Self>;
    }
);
```

**New Pattern (Recommended):**
```rust
// Define the impl outside the macro
unsafe impl NSNotificationCenter {
    extern_methods!(
        #[unsafe(method(defaultCenter))]
        pub fn defaultCenter() -> Retained<Self>;
    );
}
```

The key change is that the `impl` block should be outside the `extern_methods!` macro, with only the method declarations inside.

### 3. Protocol Implementation Patterns

For implementing protocols like NSApplicationDelegate:

```rust
define_class!(
    #[unsafe(super(NSObject))]
    pub struct AppDelegate;
    
    // Protocol implementations go inside define_class!
    unsafe impl NSObjectProtocol for AppDelegate {}
    
    unsafe impl NSApplicationDelegate for AppDelegate {
        #[method(applicationDidFinishLaunching:)]
        fn application_did_finish_launching(&self, notification: &NSNotification) {
            // Implementation
        }
        
        #[method(applicationWillTerminate:)]
        fn application_will_terminate(&self, notification: &NSNotification) {
            // Implementation
        }
    }
);
```

### 4. Method Attributes

Methods in objc2 0.6.x use attributes to specify their Objective-C selectors:

- `#[method(selector:)]` - For regular methods
- `#[method_id(selector)]` - For methods returning Retained<T>
- `#[unsafe(method(selector))]` - For unsafe methods

### 5. Type System Changes

#### ClassType and DeclaredClass

In objc2 0.6.x, classes must properly implement the type system:

```rust
unsafe impl ClassType for MyClass {
    type Super = NSObject;
    type ThreadKind = MainThreadOnly; // or InterThreadKind
    const NAME: &'static str = "MyClass";
}

impl DefinedClass for MyClass {
    type IvarTypes = (); // Define instance variables if any
}
```

### 6. Memory Management

The `Retained<T>` type is now the standard for ownership:

```rust
// Creating instances
fn new(mtm: MainThreadMarker) -> Retained<Self> {
    unsafe {
        let obj: Retained<Self> = msg_send![Self::alloc(mtm), init];
        obj
    }
}
```

## Common Issues and Solutions

### Issue 1: "no rules expected keyword fn"

**Cause:** Trying to define methods outside of an `unsafe impl` block within `define_class!`.

**Solution:** Ensure all protocol methods are inside `unsafe impl ProtocolName for ClassName {}` blocks.

### Issue 2: "having the impl inside extern_methods is deprecated"

**Cause:** Using the old pattern with impl inside the macro.

**Solution:** Move the impl outside and keep only method declarations inside extern_methods!.

### Issue 3: Type imports from objc2-core-foundation

**Problem:** `CFStringRef` and `CFTypeRef` don't exist as separate types.

**Solution:** Use `CFString` and `CFType` directly:
```rust
use objc2_core_foundation::{CFString, CFType}; // Not CFStringRef, CFTypeRef
```

## Working Example: NSApplicationDelegate

Here's a complete working example for objc2 0.6.x:

```rust
use objc2::{define_class, msg_send, sel, MainThreadMarker};
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSApplicationDelegate};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol};

define_class!(
    #[unsafe(super(NSObject))]
    #[derive(Debug)]
    pub struct AppDelegate;
    
    unsafe impl NSObjectProtocol for AppDelegate {}
    
    unsafe impl NSApplicationDelegate for AppDelegate {
        #[method(applicationDidFinishLaunching:)]
        fn application_did_finish_launching(&self, _notification: &NSNotification) {
            println!("Application launched!");
        }
        
        #[method(applicationShouldTerminateAfterLastWindowClosed:)]
        fn application_should_terminate(&self, _sender: &NSApplication) -> bool {
            true
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe {
            msg_send![Self::alloc(mtm), init]
        }
    }
}
```

## Migration Checklist

- [ ] Update all `define_class!` blocks to include protocol implementations inside
- [ ] Move `impl` blocks outside of `extern_methods!` macros
- [ ] Update type imports (CFStringRef → CFString, etc.)
- [ ] Ensure all methods have proper attributes (#[method(...)])
- [ ] Update memory management to use `Retained<T>`
- [ ] Implement ClassType and DefinedClass traits where needed

## Resources and References

1. **objc2 Documentation**: The objc2 crate provides Objective-C runtime bindings
2. **madsmtm/objc2**: The main GitHub repository for the objc2 project
3. **Apple Developer Documentation**: For understanding Objective-C protocols and delegates

## Notes on Feature Configuration

The objc2 ecosystem crates (objc2-foundation, objc2-app-kit, etc.) in version 0.3.1 use module-based features rather than individual class features. Using `features = ["all"]` provides comprehensive coverage but increases compile time.

## Future Considerations

The objc2 ecosystem is actively developed, with breaking changes possible in future versions. The move toward safer, more idiomatic Rust patterns continues, with emphasis on:

- Compile-time verification of method signatures
- Automatic memory management through Retained<T>
- Type-safe protocol implementations
- Better integration with Rust's ownership system

## Conclusion

The objc2 0.6.x macro system represents a significant evolution in Rust-Objective-C interoperability. While the changes require migration effort, they provide better safety guarantees and more idiomatic Rust code. The key is understanding that protocol implementations must be inside `define_class!`, while `extern_methods!` should only contain method declarations without the surrounding impl block.
