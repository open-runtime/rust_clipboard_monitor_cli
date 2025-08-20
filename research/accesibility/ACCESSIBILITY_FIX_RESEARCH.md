# Accessibility.rs Fix Research Report

## Summary of Issues Found

After deep research into objc2 patterns and accessibility-sys usage, here are the key issues and solutions:

### 1. AutoreleasePool Usage

**Issue:** `AutoreleasePool::new()` is private and expects 1 argument.

**Solution:** Use the `objc2::rc::autoreleasepool` function instead:
```rust
use objc2::rc::autoreleasepool;

// Instead of:
let pool = AutoreleasePool::new();

// Use:
autoreleasepool(|pool| {
    // Your code here
});
```

### 2. Accessibility Attribute Type Mismatches

**Issue:** Functions expect `CFStringRef` but getting `&'static str` from accessibility-sys constants.

**Solution:** The accessibility-sys constants like `kAXTitleAttribute` are already `CFStringRef` types. They should be used directly without conversion.

### 3. CFString Creation and Conversion

**Issue:** `CFString::from_ptr` doesn't exist in objc2-core-foundation.

**Solution:** Use proper patterns for CFString:
```rust
// To create CFString from &str:
let cf_str = CFString::from_str("your string");

// To convert from CFStringRef to Rust String:
unsafe {
    if !cf_string_ref.is_null() {
        // Cast to CFString and convert
        let cf_str = CFString::wrap_under_get_rule(cf_string_ref);
        Some(cf_str.to_string())
    } else {
        None
    }
}
```

### 4. as_concrete_TypeRef Deprecated

**Issue:** `as_concrete_TypeRef()` method is deprecated.

**Solution:** Use `.as_CFType()` or cast directly:
```rust
// Instead of:
cf_str.as_concrete_TypeRef()

// Use:
cf_str.as_CFTypeRef() as CFStringRef
```

### 5. CFBoolean Handling

**Issue:** `CFBoolean::from_ptr` doesn't exist.

**Solution:** Use proper CFBoolean patterns:
```rust
// Cast and wrap CFBoolean
unsafe {
    let cf_bool = CFBoolean::wrap_under_get_rule(boolean_ptr);
    cf_bool.as_bool()
}
```

## Required Import Changes

Add/modify these imports:
```rust
use objc2::rc::autoreleasepool;
use objc2_core_foundation::{base::TCFType, CFBoolean, CFString, string::CFStringRef};
use core_foundation_sys::base::CFTypeRef;
```

## Key Pattern Changes

### 1. Autorelease Pool Pattern
```rust
// Old:
let pool = AutoreleasePool::new();
// ... code ...
drop(pool);

// New:
autoreleasepool(|pool| {
    // ... code ...
    // Pool is automatically dropped at end of closure
});
```

### 2. Getting String Attributes
```rust
fn get_string_attribute(&self, element: AXUIElement, attribute: CFStringRef) -> Option<String> {
    unsafe {
        let mut value: CFTypeRef = std::ptr::null_mut();
        let result = AXUIElementCopyAttributeValue(element, attribute, &mut value);
        
        if result == kAXErrorSuccess && !value.is_null() {
            // Wrap the CFString and convert to Rust String
            let cf_string = CFString::wrap_under_create_rule(value as CFStringRef);
            Some(cf_string.to_string())
        } else {
            None
        }
    }
}
```

### 3. Creating CFDictionary for Options
```rust
use objc2_core_foundation::dictionary::CFDictionary;
use objc2_core_foundation::base::FromVoid;

// Create options dictionary
let keys = vec![CFString::from_str("AXTrustedCheckOptionPrompt")];
let values = vec![CFBoolean::true_value()];
let options = CFDictionary::from_CFType_pairs(&keys, &values);
```

## Complete Fixed Implementation Pattern

Here's how the key methods should be restructured:

```rust
pub fn extract_context(&mut self, app_info: &AppInfo) -> Result<AccessibilityContext, String> {
    // ... validation code ...
    
    // Use autoreleasepool function
    autoreleasepool(|pool| {
        // Create accessibility element
        let ax_app = unsafe { AXUIElementCreateApplication(app_info.pid) };
        if ax_app.is_null() {
            return Err(format!("Failed to create AXUIElement for PID {}", app_info.pid));
        }
        
        // ... rest of the extraction logic ...
        
        Ok(context)
    })
}
```

## Verification Steps

1. Check that all accessibility-sys constants are used directly as CFStringRef
2. Ensure autoreleasepool closure pattern is used consistently
3. Verify CFString wrapping uses proper ownership rules (wrap_under_create_rule vs wrap_under_get_rule)
4. Test that type conversions compile correctly

## References

- objc2 documentation: https://docs.rs/objc2
- objc2-core-foundation: https://docs.rs/objc2-core-foundation
- accessibility-sys: https://docs.rs/accessibility-sys

## Note on Memory Management

When using `wrap_under_create_rule`, the CFString takes ownership and will release the underlying object when dropped. Use this for values returned by Copy functions.

When using `wrap_under_get_rule`, no ownership is taken, suitable for borrowed references.
