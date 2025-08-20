import 'dart:async';
import 'lib/src/rust/api.dart';
import 'lib/src/rust/frb_generated.dart';

void main() async {
  print('🔧 Testing Enhanced Context Extraction...\n');
  
  // Initialize FRB
  await RustLib.init();
  print('✅ Rust library initialized\n');
  
  // Get clipboard data with full context
  final clipboardData = await getCurrentClipboardInfoSilent();
  
  if (clipboardData == null) {
    print('❌ No clipboard data available');
    return;
  }
  
  print('📋 CLIPBOARD DATA WITH FULL CONTEXT:');
  print('=====================================\n');
  
  // Basic clipboard info
  print('📊 BASIC INFO:');
  print('   Change Count: ${clipboardData.changeCount}');
  print('   Timestamp: ${clipboardData.timestamp}');
  print('   Primary Content Length: ${clipboardData.primaryContent.length} chars');
  
  // Source app info
  if (clipboardData.sourceApp != null) {
    print('\n📱 SOURCE APP:');
    print('   Name: ${clipboardData.sourceApp!.name}');
    print('   Bundle ID: ${clipboardData.sourceApp!.bundleId}');
    print('   PID: ${clipboardData.sourceApp!.pid}');
    if (clipboardData.sourceApp!.path != null) {
      print('   Path: ${clipboardData.sourceApp!.path}');
    }
  }
  
  // Window context
  if (clipboardData.windowContext != null) {
    print('\n🪟 WINDOW CONTEXT:');
    final wc = clipboardData.windowContext!;
    print('   Title: ${wc.windowTitle ?? "N/A"}');
    print('   Window ID: ${wc.windowId}');
    print('   Layer: ${wc.windowLayer}');
    print('   Fullscreen: ${wc.isFullscreen}');
    print('   Minimized: ${wc.isMinimized}');
    if (wc.bounds != null) {
      print('   Bounds: (${wc.bounds!.x}, ${wc.bounds!.y}) ${wc.bounds!.width}x${wc.bounds!.height}');
    }
  } else {
    print('\n🪟 WINDOW CONTEXT: Not available');
  }
  
  // Browser context
  if (clipboardData.browserContext != null) {
    print('\n🌐 BROWSER CONTEXT:');
    final bc = clipboardData.browserContext!;
    print('   URL: ${bc.currentUrl ?? "N/A"}');
    print('   Page Title: ${bc.pageTitle ?? "N/A"}');
    print('   Tab Count: ${bc.tabCount ?? "N/A"}');
    print('   Incognito: ${bc.isIncognito}');
  } else {
    print('\n🌐 BROWSER CONTEXT: Not available');
  }
  
  // Space context
  if (clipboardData.spaceContext != null) {
    print('\n🖥️ SPACE/DESKTOP CONTEXT:');
    final sc = clipboardData.spaceContext!;
    print('   Space Index: ${sc.spaceIndex}');
    print('   Space Name: ${sc.spaceName}');
    print('   Display UUID: ${sc.displayUuid}');
  } else {
    print('\n🖥️ SPACE/DESKTOP CONTEXT: Not available');
  }
  
  // Accessibility context
  if (clipboardData.accessibilityContext != null) {
    print('\n♿ ACCESSIBILITY CONTEXT:');
    final ac = clipboardData.accessibilityContext!;
    print('   Focused Element: ${ac.focusedElementRole ?? "N/A"}');
    print('   Element Title: ${ac.focusedElementTitle ?? "N/A"}');
    print('   Selected Text: ${ac.selectedText ?? "N/A"}');
    print('   Document Path: ${ac.documentPath ?? "N/A"}');
  } else {
    print('\n♿ ACCESSIBILITY CONTEXT: Not available');
  }
  
  // System context
  print('\n💻 SYSTEM CONTEXT:');
  final sys = clipboardData.systemContext;
  print('   Display Count: ${sys.displayCount}');
  print('   Active Display ID: ${sys.activeDisplayId}');
  print('   Session Active: ${sys.sessionActive}');
  print('   Screen Locked: ${sys.screenLocked}');
  
  // Available formats
  print('\n📊 CLIPBOARD FORMATS (${clipboardData.formats.length}):');
  for (int i = 0; i < clipboardData.formats.length; i++) {
    final format = clipboardData.formats[i];
    if (format.isAvailable) {
      print('   [${i+1}] ${format.formatType}: ${format.dataSize} bytes');
    }
  }
  
  print('\n✅ Context extraction test complete!');
}