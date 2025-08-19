import 'lib/src/rust/api.dart' as rust_api;
import 'lib/src/rust/frb_generated.dart' show RustLib;

Future<void> main() async {
  print('🧪 Testing Dart Clipboard Monitor API...');
  
  // Initialize the Flutter Rust Bridge library
  await RustLib.init();
  print('✅ Flutter Rust Bridge initialized');
  
  // Initialize the monitor
  await rust_api.initMonitor();
  print('✅ Monitor initialized');
  
  // Check if monitoring initially
  bool isMonitoring = await rust_api.isMonitoring();
  print('📊 Is monitoring: $isMonitoring');
  
  // Start monitoring
  await rust_api.startMonitoringSimple(
    enhanced: true,
    verbose: 1,
    background: false,
  );
  print('✅ Monitoring started');
  
  // Check monitoring status
  isMonitoring = await rust_api.isMonitoring();
  print('📊 Is monitoring: $isMonitoring');
  
  // Wait a bit for app detection
  await Future.delayed(Duration(seconds: 2));
  
  // Get current app
  var currentApp = await rust_api.getCurrentApp();
  if (currentApp != null) {
    print('📱 Current app: ${currentApp.appInfo.name} (${currentApp.appInfo.bundleId})');
    print('   - PID: ${currentApp.appInfo.pid}');
    print('   - Event type: ${currentApp.eventType}');
    if (currentApp.previousApp != null) {
      print('   - Previous app: ${currentApp.previousApp!.name}');
    }
  } else {
    print('⚠️ No current app detected');
  }
  
  // Check accessibility permissions
  bool hasPermissions = await rust_api.checkAccessibilityPermissions();
  print('🔐 Has accessibility permissions: $hasPermissions');
  
  print('🧪 API test completed');
  
  // Stop monitoring
  await rust_api.stopMonitoring();
  print('🛑 Monitoring stopped');
}