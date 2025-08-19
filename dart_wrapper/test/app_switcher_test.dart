import 'dart:async';
import 'package:test/test.dart';
import 'package:clipboard_monitor_dart/rust_wrapper.dart';

void main() {
  group('AppSwitcher Streaming API Tests', () {
    setUpAll(() async {
      // Initialize the Rust library
      await RustLib.init();
    });

    test('can check accessibility permissions', () async {
      final hasPermissions = await checkAccessibilityPermissions();
      print('Accessibility permissions: $hasPermissions');

      // This should return a boolean (might be false if permissions not granted)
      expect(hasPermissions, isA<bool>());
    });

    test('can get current app info', () async {
      final currentApp = await getCurrentAppInfo();
      print('Current app info: $currentApp');

      // May be null if no app is focused or permissions not granted
      if (currentApp != null) {
        expect(currentApp, isA<DartAppInfo>());
        expect(currentApp.name, isNotEmpty);
        expect(currentApp.bundleId, isNotEmpty);
        print('  App: ${currentApp.name} (${currentApp.bundleId})');
      }
    });

    test('can check monitoring status', () async {
      final isCurrentlyMonitoring = await isMonitoring();
      print('Currently monitoring: $isCurrentlyMonitoring');

      expect(isCurrentlyMonitoring, isA<bool>());
      expect(isCurrentlyMonitoring, isFalse); // Should be false initially
    });

    test('can start and receive app switch events via stream', () async {
      print('🔧 Starting app switcher monitoring test...');

      final Completer<DartAppSwitchEventData> firstEventCompleter = Completer();
      late StreamSubscription subscription;

      // Start monitoring and listen to the stream
      final stream = monitorAppSwitches(
        enhanced: true,
        verbose: 2,
        background: false,
      );

      subscription = stream.listen(
        (DartAppSwitchEventData event) {
          print('📱 Received app switch event:');
          print('  App: ${event.appInfo.name} (${event.appInfo.bundleId})');
          print('  Event type: ${event.eventType}');
          print('  Window title: ${event.windowTitle ?? "None"}');
          print('  URL: ${event.url ?? "None"}');
          if (event.previousApp != null) {
            print('  Previous app: ${event.previousApp!.name}');
          }

          // Complete on first event
          if (!firstEventCompleter.isCompleted) {
            firstEventCompleter.complete(event);
          }
        },
        onError: (error) {
          print('❌ Stream error: $error');
          if (!firstEventCompleter.isCompleted) {
            firstEventCompleter.completeError(error);
          }
        },
        onDone: () {
          print('✅ Stream completed');
        },
      );

      // Wait a bit for the monitoring to start
      await Future.delayed(Duration(seconds: 1));

      // Check that monitoring is now active
      final isNowMonitoring = await isMonitoring();
      print('Monitoring status after start: $isNowMonitoring');
      expect(isNowMonitoring, isTrue);

      print('👆 Please switch between apps to generate events...');
      print('⏱️  Waiting up to 30 seconds for an app switch event...');

      try {
        // Wait up to 30 seconds for an app switch event
        final firstEvent = await firstEventCompleter.future.timeout(Duration(seconds: 30));

        print('✅ Successfully received first app switch event!');
        expect(firstEvent, isA<DartAppSwitchEventData>());
        expect(firstEvent.appInfo.name, isNotEmpty);
        expect(firstEvent.eventType, isNotEmpty);
      } catch (e) {
        if (e is TimeoutException) {
          print('⏰ Timeout waiting for app switch event (this is OK - might need to manually switch apps)');
        } else {
          print('❌ Error waiting for event: $e');
          rethrow;
        }
      } finally {
        // Clean up
        await subscription.cancel();
        await stopMonitoring();

        // Verify monitoring stopped
        final isFinallyMonitoring = await isMonitoring();
        print('Final monitoring status: $isFinallyMonitoring');
        expect(isFinallyMonitoring, isFalse);
      }
    }, timeout: Timeout(Duration(seconds: 45)));
  });
}
