/// Flutter Rust Bridge Wrapper for AppSwitcher
/// 
/// This library provides a Dart interface to the Rust AppSwitcher functionality
/// for real-time app switching notifications on macOS.
library rust_wrapper;

// Export the generated API
export 'src/rust/api.dart';
export 'src/rust/frb_generated.dart' show RustLib;