# Compilation Fix Instructions

## Summary of Issues

Both `accessibility.rs` and `app_switcher.rs` have compilation issues related to:
1. Type mismatches between CFStringRef and &str
2. Missing or incorrect FFI bindings
3. Deprecated or missing methods in objc2 crates
4. Thread safety and unsafe block requirements

## Quick Fixes to Make It Compile

### For accessibility.rs:

1. **CFString issues**: Many methods expect `CFStringRef` but get `&str`. The simplest fix is to comment out complex extraction logic temporarily.

2. **CFDictionary creation**: The `from_pairs` method doesn't exist. For now, pass null pointers.

3. **String conversion**: The `from_ptr` methods don't exist on CFString/CFBoolean. Use placeholder values.

### For app_switcher.rs:

1. **NSNotificationCenter**: Missing `removeObserver_name_object` method - just skip cleanup for now.

2. **ProtocolObject type annotations**: Need explicit casting to `&dyn NSObjectProtocol`.

3. **AppDelegate creation**: The `alloc` method needs MainThreadMarker parameter.

4. **CFRunLoop**: Use raw FFI bindings instead of objc2 wrappers.

## Simplified Working Version

Here's what you should do to get it compiling:

1. **Comment out complex accessibility extraction** - Focus on basic app switching first
2. **Use placeholder values** where type conversions are complex
3. **Skip observer cleanup** - Let them be cleaned up on drop
4. **Use raw pointers** for CFDictionary creation

## Next Steps

Once it compiles:
1. Gradually re-enable accessibility features
2. Implement proper CFString/CFDictionary handling
3. Add proper observer cleanup
4. Test with real macOS applications

## Key Learning

The objc2 ecosystem is still evolving, and not all Core Foundation types have complete Rust bindings. Sometimes you need to:
- Use raw FFI for missing functionality
- Create your own safe wrappers
- Accept that some features need placeholder implementations initially
