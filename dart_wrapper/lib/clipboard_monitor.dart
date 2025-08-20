import 'dart:async';

// Import the Flutter Rust Bridge generated API
import 'src/rust/api.dart' as rust_api;
import 'src/rust/frb_generated.dart' show RustLib;

// Configuration class
class MonitorConfig {
  final bool enhanced;
  final int verbose;
  final bool background;
  final String? filter;

  const MonitorConfig({
    required this.enhanced,
    required this.verbose,
    required this.background,
    this.filter,
  });
}

// Re-export the Rust types for convenience
typedef AppInfo = rust_api.DartAppInfo;
typedef AppSwitchEventData = rust_api.DartAppSwitchEventData;

class ClipboardMonitor {
  Function(AppSwitchEventData)? onAppSwitch;

  Future<void> initialize() async {
    // Initialize the Flutter Rust Bridge library
    await RustLib.init();
    print('✅ Flutter Rust Bridge initialized successfully');
  }

  Future<void> startMonitoring(MonitorConfig config) async {
    try {
      // Initialize the monitor first
      await rust_api.initMonitor();
      print('✅ macOS monitor initialized');

      // Start monitoring with the configuration
      await rust_api.startMonitoringSimple(
        enhanced: config.enhanced,
        verbose: config.verbose,
        background: config.background,
      );

      print('✅ App monitoring started successfully');
    } catch (e) {
      throw Exception('Failed to start monitoring: $e');
    }
  }

  Future<void> stopMonitoring() async {
    try {
      await rust_api.stopMonitoring();
      print('🛑 Monitoring stopped');
    } catch (e) {
      print('Warning: Error stopping monitoring: $e');
    }
  }

  Future<bool> isMonitoring() async {
    try {
      return await rust_api.isMonitoring();
    } catch (e) {
      print('Error checking monitoring status: $e');
      return false;
    }
  }

  Future<bool> checkAccessibilityPermissions() async {
    try {
      return await rust_api.checkAccessibilityPermissions();
    } catch (e) {
      print('Error checking accessibility permissions: $e');
      return false;
    }
  }

  Future<AppSwitchEventData?> getCurrentApp() async {
    try {
      return await rust_api.getCurrentApp();
    } catch (e) {
      print('Error getting current app: $e');
      return null;
    }
  }
}
