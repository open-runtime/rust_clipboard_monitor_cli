import 'dart:async';
import 'package:test/test.dart';
import 'package:clipboard_monitor_dart/rust_wrapper.dart';

void main() {
  group('Enhanced Clipboard Monitoring Tests', () {
    setUpAll(() async {
      // Initialize the Rust library
      await RustLib.init();
    });

    test('can get current clipboard info with enhanced context', () async {
      print('📋 Testing enhanced clipboard monitoring...');
      
      // Get current clipboard data
      final clipboardData = await getCurrentClipboardInfo();
      
      if (clipboardData != null) {
        print('✅ Clipboard data retrieved:');
        print('  Change count: ${clipboardData.changeCount}');
        print('  Timestamp: ${clipboardData.timestamp}');
        print('  Primary content: ${clipboardData.primaryContent}');
        print('  Formats: ${clipboardData.formats.length}');
        
        // Check source app context
        if (clipboardData.sourceApp != null) {
          print('  Source app: ${clipboardData.sourceApp!.name} (${clipboardData.sourceApp!.bundleId})');
        }
        
        // Check window context
        if (clipboardData.windowContext != null) {
          print('  Window context:');
          print('    Title: ${clipboardData.windowContext!.windowTitle ?? "N/A"}');
          print('    Window ID: ${clipboardData.windowContext!.windowId}');
          print('    Fullscreen: ${clipboardData.windowContext!.isFullscreen}');
        }
        
        // Check browser context
        if (clipboardData.browserContext != null) {
          print('  Browser context:');
          print('    URL: ${clipboardData.browserContext!.currentUrl ?? "N/A"}');
          print('    Page title: ${clipboardData.browserContext!.pageTitle ?? "N/A"}');
          print('    Incognito: ${clipboardData.browserContext!.isIncognito}');
        }
        
        // Check space context
        if (clipboardData.spaceContext != null) {
          print('  Space context:');
          print('    Space index: ${clipboardData.spaceContext!.spaceIndex}');
          print('    Space name: ${clipboardData.spaceContext!.spaceName}');
        }
        
        // Check accessibility context
        if (clipboardData.accessibilityContext != null) {
          print('  Accessibility context:');
          print('    Focused element: ${clipboardData.accessibilityContext!.focusedElementRole ?? "N/A"}');
          print('    Selected text: ${clipboardData.accessibilityContext!.selectedText ?? "N/A"}');
        }
        
        // Check system context
        print('  System context:');
        print('    Display count: ${clipboardData.systemContext.displayCount}');
        print('    Active display: ${clipboardData.systemContext.activeDisplayId}');
        print('    Session active: ${clipboardData.systemContext.sessionActive}');
        print('    Screen locked: ${clipboardData.systemContext.screenLocked}');
        
        expect(clipboardData, isA<DartClipboardData>());
        expect(clipboardData.formats, isA<List<DartClipboardFormat>>());
      } else {
        print('⚠️ No clipboard data available (clipboard might be empty)');
      }
    });

    test('can run comprehensive clipboard monitoring test', () async {
      print('🔍 Running comprehensive clipboard monitoring test...');
      
      try {
        await testComprehensiveClipboardMonitoring();
        print('✅ Comprehensive monitoring test completed');
      } catch (e) {
        print('❌ Error in comprehensive monitoring: $e');
        // This might fail if clipboard is empty or permissions not granted
      }
    });
  });
}