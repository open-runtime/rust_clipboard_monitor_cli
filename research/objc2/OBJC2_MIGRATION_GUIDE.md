# objc2 Migration Guide: Fixing Import and Compatibility Issues

## Summary of Issues Found

After extensive research into objc2-app-kit, objc2-foundation, and related crates, I've identified the following key issues with your migration:

### 1. Cargo.toml Feature Configuration

**Problem:** The objc2 crates don't use individual class names as features. Features like `NSApplicationDelegate`, `NSPasteboardType` don't exist.

**Solution:** Features in objc2 crates are module-based, not class-based. I've updated your Cargo.toml to use the `all` feature for comprehensive coverage.

### 2. Type Import Issues

**Problem:** `CFStringRef` and `CFTypeRef` are not directly exported from objc2-core-foundation.

**Solution:** In objc2-core-foundation 0.3.x:
- Use `CFString` instead of `CFStringRef`
- Use `CFType` instead of `CFTypeRef`
- These are the actual struct types, not type aliases

### 3. Accessibility API Changes

**Problem:** `objc2-application-services` and `objc2-accessibility` don't exist in the expected versions.

**Solution:** Use `accessibility-sys` version 0.2 for accessibility APIs.

## Required Code Changes

### 1. Fix imports in `src/core/accessibility.rs`

Change:
```rust
use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFString, CFStringRef, CFTypeRef};
```

To:
```rust
use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFString, CFType};
```

### 2. Fix imports in `src/core/app_switcher.rs`

Change:
```rust
use objc2_core_foundation::{
    CFRunLoop, CFRunLoopAddSource, CFRunLoopGetCurrent, CFString, CFStringRef,
};
```

To:
```rust
use objc2_core_foundation::{
    CFRunLoop, CFRunLoopAddSource, CFRunLoopGetCurrent, CFString,
};
```

### 3. Type Usage Updates

When using these types in your code:
- Replace `*const CFStringRef` with `&CFString` or `*const CFString`
- Replace `CFTypeRef` with `&CFType` or `*const CFType`
- Use `.as_ptr()` or similar methods when needing raw pointers

### 4. objc2 Macro Syntax Issues

The error about `no rules expected keyword fn` in app_switcher.rs line 637 suggests incorrect macro usage.

For objc2 0.6.x, the `define_class!` macro syntax has changed. Methods should be defined with:

```rust
unsafe impl ProtocolName for ClassName {
    #[method(methodName:)]
    unsafe fn method_name(&self, param: &NSObject) {
        // implementation
    }
}
```

Not as regular `fn` definitions outside the unsafe impl block.

## Updated Dependencies Configuration

Your Cargo.toml should now have:

```toml
[dependencies]
# Core objc2 ecosystem
objc2 = "0.6.2"
objc2-foundation = { version = "0.3.1", features = ["all"] }
objc2-app-kit = { version = "0.3.1", features = ["all"] }
accessibility-sys = "0.2"
objc2-core-graphics = { version = "0.3.1", features = ["all"] }
objc2-core-foundation = { version = "0.3.1", features = ["all"] }
```

## Additional Considerations

1. **Feature Granularity**: Using `features = ["all"]` includes all available types but may increase compile time. Once your code is working, you can optimize by identifying specific features you need.

2. **API Stability**: The objc2 ecosystem is still evolving. Version 0.3.1 of the framework crates is relatively stable but may have breaking changes in future versions.

3. **Documentation**: The objc2 crates are generated from Apple's headers, so documentation may be sparse. Refer to Apple's official Core Foundation and AppKit documentation for detailed API information.

4. **Memory Management**: objc2 handles reference counting automatically in most cases, but be careful with raw pointers and manual CFRelease calls.

## Next Steps

1. Apply the import fixes to your source files
2. Update the objc2 macro usage to match the 0.6.x syntax
3. Test compilation incrementally
4. Review any remaining type compatibility issues

## Resources

- [objc2 GitHub Repository](https://github.com/madsmtm/objc2)
- [objc2 Documentation](https://docs.rs/objc2)
- [Apple Core Foundation Documentation](https://developer.apple.com/documentation/corefoundation)
- [Apple AppKit Documentation](https://developer.apple.com/documentation/appkit)
