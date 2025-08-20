# Compilation Fixes Needed

## Summary of Issues

After researching objc2 0.6.x migration and attempting fixes, here are the remaining issues:

### 1. extern_methods! Attribute Syntax (CRITICAL)
**Issue**: The `#[method(...)]` attribute is now unsafe and requires `#[unsafe(method(...))]` syntax
**Files Affected**: 
- `app_switcher_enhanced.rs` 
- `app_switcher_workspace.rs`

**Fix Required**: Replace all instances of:
```rust
#[method(selectorName:)]
```
with:
```rust
#[unsafe(method(selectorName:))]
```

### 2. Missing AnyThread Trait Import
**Issue**: `no function or associated item named 'alloc' found`
**Fix**: Add import:
```rust
use objc2::AnyThread;
```

### 3. CFDictionary find() Method Issues
**Issue**: The `find()` method returns `ItemRef<'_, T>` not raw pointers
**Fix**: Need to dereference the ItemRef or use proper extraction methods:
```rust
// Instead of:
dict.find(key).and_then(|n| Some(CFNumber::from_void(n)))

// Use:
dict.find(key).and_then(|n| Some(CFNumber::from_void(*n)))
```

### 4. sysinfo API Changes
**Issue**: `refresh_process_specifics` doesn't exist
**Fix**: Use the new API:
```rust
// Instead of:
system.refresh_process_specifics(pid, ProcessRefreshKind::new())

// Use:
system.refresh_process(pid)
```

### 5. Retained API Changes
**Issue**: `cast()` is deprecated, `retain()` doesn't exist as a method
**Fix**: 
```rust
// Instead of:
notification_obj.cast::<AnyObject>().retain()

// Use:
notification_obj.cast_unchecked::<AnyObject>()
```

### 6. CGSession and CGWindow Functions
**Issue**: Functions need proper unsafe blocks around calls
**Fix**: Wrap CGWindowListCopyWindowInfo and CGSessionCopyCurrentDictionary calls in unsafe blocks

### 7. Core Foundation Type Conversions
**Issue**: CFString::from("...").as_concrete_TypeRef() returns wrong type
**Fix**: Use proper conversion:
```rust
// Create the CFString
let cf_str = CFString::from("string");
// Get the raw pointer (may need casting depending on context)
let cf_ref = cf_str.as_CFTypeRef() as CFStringRef;
```

## Next Steps

1. Fix all `#[method(...)]` to `#[unsafe(method(...))]` in extern_methods!
2. Add missing trait imports (AnyThread)
3. Fix all CFDictionary extraction patterns
4. Update sysinfo API calls
5. Fix Retained API usage
6. Add unsafe blocks where needed
7. Test compilation after each fix

## Files to Update
1. `/src/core/app_switcher_enhanced.rs`
2. `/src/core/app_switcher_workspace.rs`
3. `/src/core/time_tracker.rs` (minor fixes already done)
4. `/src/core/event_tap.rs` (pending)
5. `/src/main.rs` (integration pending)

## Research Documents Created
- `OBJC2_RESEARCH_SUMMARY.md` - High-level findings
- `OBJC2_0.6_MACRO_RESEARCH.md` - Detailed macro patterns
- `OBJC2_MIGRATION_GUIDE.md` - Migration guide
- `CFSTRING_CONVERSION_RESEARCH.md` - CFString conversion patterns