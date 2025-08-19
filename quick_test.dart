#!/usr/bin/env dart

import 'dart:async';
import 'dart:io';
import 'lib/src/rust/api.dart';
import 'lib/src/rust/frb_generated.dart';

Future<void> main() async {
  print('🔧 Testing new FRB streaming AppSwitcher API...');
  
  try {
    // Initialize the Rust library
    await RustLib.init();
    print('✅ Rust library initialized');
    
    // Check accessibility permissions
    final hasPermissions = await checkAccessibilityPermissions();
    print('📋 Accessibility permissions: $hasPermissions');
    
    // Get current app
    final currentApp = await getCurrentAppInfo();
    if (currentApp != null) {
      print('📱 Current app: ${currentApp.name} (${currentApp.bundleId})');
    }
    
    // Start streaming
    print('\n🚀 Starting streaming AppSwitcher...');
    int eventCount = 0;
    
    final stream = monitorAppSwitches(enhanced: true, verbose: 2, background: false);
    final subscription = stream.listen(
      (event) {
        eventCount++;
        print('\n📱 Event #$eventCount: ${event.appInfo.name} (${event.eventType})');
        if (event.windowTitle != null) print('   Window: ${event.windowTitle}');
        if (event.url != null) print('   URL: ${event.url}');
        
        if (eventCount >= 5) {
          print('✅ Got 5 events, stopping...');
          exit(0);
        }
      },
      onError: (e) => print('❌ Error: $e'),
    );
    
    // Wait for monitoring to start
    await Future.delayed(Duration(seconds: 1));
    
    final isActive = await isMonitoring();
    print('🔍 Monitoring active: $isActive');
    
    if (isActive) {
      print('\n👆 Please switch between apps! Waiting for 5 events...');
      
      // Keep alive for 60 seconds max
      Timer(Duration(seconds: 60), () {
        print('\n⏰ Timeout after 60s');
        exit(0);
      });
      
      // Keep the program running
      while (true) {
        await Future.delayed(Duration(seconds: 1));
      }
    } else {
      print('❌ Failed to start monitoring');
      exit(1);
    }
    
  } catch (e) {
    print('❌ Error: $e');
    exit(1);
  }
}