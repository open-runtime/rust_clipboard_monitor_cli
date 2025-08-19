#!/usr/bin/env dart

import 'dart:async';
import 'dart:io';
import '../lib/rust_wrapper.dart';

/// Simple CLI test for the AppSwitcher streaming API
/// This demonstrates the real AppSwitcher functionality piped through FRB streams
Future<void> main(List<String> args) async {
  print('🔧 Initializing Rust AppSwitcher library...');
  
  try {
    // Initialize the Rust library
    await RustLib.init();
    print('✅ Rust library initialized successfully');
    
    // Check accessibility permissions
    print('\n📋 Checking accessibility permissions...');
    final hasPermissions = await checkAccessibilityPermissions();
    print('Accessibility permissions: $hasPermissions');
    
    if (!hasPermissions) {
      print('⚠️  Note: You may need to grant accessibility permissions in System Preferences');
      print('   Go to: System Preferences > Security & Privacy > Privacy > Accessibility');
    }
    
    // Get current app info
    print('\n📱 Getting current app info...');
    final currentApp = await getCurrentAppInfo();
    if (currentApp != null) {
      print('Current app: ${currentApp.name} (${currentApp.bundleId})');
      print('PID: ${currentApp.pid}');
      if (currentApp.path != null) {
        print('Path: ${currentApp.path}');
      }
    } else {
      print('No current app detected');
    }
    
    // Check initial monitoring status
    print('\n🔍 Checking initial monitoring status...');
    final initialStatus = await isMonitoring();
    print('Initially monitoring: $initialStatus');
    
    // Start monitoring with streaming
    print('\n🚀 Starting real AppSwitcher monitoring with streaming...');
    print('This uses the same AppSwitcher from main.rs but as a singleton service');
    print('Events will be streamed in real-time via Flutter Rust Bridge');
    
    int eventCount = 0;
    final completer = Completer<void>();
    
    // Start the streaming API
    final stream = monitorAppSwitches(
      enhanced: true,
      verbose: 2,
      background: false,
    );
    
    // Listen to the stream
    late StreamSubscription subscription;
    subscription = stream.listen(
      (DartAppSwitchEventData event) {
        eventCount++;
        print('\n📱 App Switch Event #$eventCount:');
        print('  App: ${event.appInfo.name} (${event.appInfo.bundleId})');
        print('  Event: ${event.eventType}');
        print('  PID: ${event.appInfo.pid}');
        
        if (event.windowTitle != null) {
          print('  Window: ${event.windowTitle}');
        }
        
        if (event.url != null) {
          print('  URL: ${event.url}');
        }
        
        if (event.previousApp != null) {
          print('  Previous: ${event.previousApp!.name}');
        }
        
        if (event.appInfo.path != null) {
          print('  Path: ${event.appInfo.path}');
        }
        
        print('  ─────────────────────────────────────');
        
        // Stop after 10 events or if user presses Ctrl+C
        if (eventCount >= 10) {
          print('\n✅ Received 10 events, stopping...');
          completer.complete();
        }
      },
      onError: (error) {
        print('\n❌ Stream error: $error');
        completer.completeError(error);
      },
      onDone: () {
        print('\n✅ Stream completed');
        if (!completer.isCompleted) {
          completer.complete();
        }
      },
    );
    
    // Set up Ctrl+C handler
    ProcessSignal.sigint.watch().listen((signal) async {
      print('\n⏹️  Received SIGINT, stopping monitoring...');
      if (!completer.isCompleted) {
        completer.complete();
      }
    });
    
    // Wait a moment for monitoring to start
    await Future.delayed(Duration(seconds: 1));
    
    // Verify monitoring is active
    final monitoringStatus = await isMonitoring();
    print('Monitoring active: $monitoringStatus');
    
    if (monitoringStatus) {
      print('\n👆 Switch between applications to see real-time events!');
      print('   Press Ctrl+C to stop monitoring');
      print('   Or wait for 10 events to be automatically collected\n');
    } else {
      print('❌ Failed to start monitoring');
      exit(1);
    }
    
    // Wait for completion or user interrupt
    await completer.future.timeout(
      Duration(minutes: 5),
      onTimeout: () {
        print('\n⏰ Timeout after 5 minutes');
      },
    );
    
    // Clean up
    print('\n🧹 Cleaning up...');
    await subscription.cancel();
    await stopMonitoring();
    
    // Verify monitoring stopped
    final finalStatus = await isMonitoring();
    print('Final monitoring status: $finalStatus');
    
    print('\n✅ AppSwitcher streaming test completed!');
    print('Total events received: $eventCount');
    
  } catch (e, stackTrace) {
    print('\n❌ Error: $e');
    print('Stack trace: $stackTrace');
    exit(1);
  }
}