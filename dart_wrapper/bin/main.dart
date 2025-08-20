import 'dart:async';
import 'dart:io';
import 'package:args/args.dart';
import '../lib/src/rust/api.dart';
import '../lib/src/rust/frb_generated.dart';

void main(List<String> arguments) async {
  final parser = ArgParser()
    ..addFlag('enhanced',
        abbr: 'e',
        defaultsTo: true,
        help: 'Extract detailed context (URLs, file paths, etc.) - requires accessibility permissions')
    ..addOption('verbose',
        abbr: 'v',
        defaultsTo: '2',
        help: 'Verbosity level for logging (0-2)')
    ..addFlag('background',
        abbr: 'b',
        defaultsTo: false,
        help: 'Run without prompting for permissions')
    ..addOption('filter',
        abbr: 'f',
        help: 'Only track specific app types: browser, ide, productivity')
    ..addFlag('check-permissions',
        abbr: 'c',
        defaultsTo: false,
        help: 'Check required permissions and exit')
    ..addFlag('clipboard',
        abbr: 'p',
        defaultsTo: false,
        help: 'Test clipboard monitoring capabilities')
    ..addFlag('clipboard-info',
        defaultsTo: false,
        help: 'Get current clipboard information')
    ..addFlag('clipboard-monitor',
        abbr: 'm',
        defaultsTo: false,
        help: 'Continuously monitor clipboard changes for 3 minutes')
    ..addFlag('silent',
        abbr: 's',
        defaultsTo: false,
        help: 'Silent monitoring - run until terminated, only output on clipboard changes with full metadata')
    ..addFlag('help',
        abbr: 'h',
        defaultsTo: false,
        help: 'Show this help message');

  late ArgResults results;
  try {
    results = parser.parse(arguments);
  } on FormatException catch (e) {
    print('Error: ${e.message}');
    print('');
    print(parser.usage);
    exit(1);
  }

  if (results['help'] as bool) {
    print('Dart CLI wrapper for Rust clipboard monitor with FRB streaming');
    print('Usage: clipboard_monitor_dart [options]');
    print('');
    print('Options:');
    print(parser.usage);
    exit(0);
  }

  try {
    // Initialize the FRB Rust library
    print('🔧 Initializing Rust library with FRB...');
    await RustLib.init();
    print('✅ Rust library initialized successfully');

    // Check permissions if requested
    if (results['check-permissions'] as bool) {
      final hasPerms = await checkAccessibilityPermissions();
      print('Accessibility permissions: ${hasPerms ? "✅ Granted" : "❌ Not granted"}');
      if (!hasPerms) {
        print('Enable in: System Settings → Privacy & Security → Accessibility');
      }
      exit(hasPerms ? 0 : 1);
    }

    // Get current clipboard info if requested
    if (results['clipboard-info'] as bool) {
      print('📋 Getting current clipboard information...');
      try {
        final clipboardData = await getCurrentClipboardInfo();
        if (clipboardData != null) {
          print('✅ Clipboard data found:');
          print('   Change Count: ${clipboardData.changeCount}');
          print('   Timestamp: ${clipboardData.timestamp}');
          print('   Primary Content: "${clipboardData.primaryContent}"');
          if (clipboardData.sourceApp != null) {
            print('   Source App: ${clipboardData.sourceApp!.name} (${clipboardData.sourceApp!.bundleId})');
          }
          print('   Available Formats: ${clipboardData.formats.length}');
          for (int i = 0; i < clipboardData.formats.length; i++) {
            final format = clipboardData.formats[i];
            final emoji = getFormatEmoji(format.formatType);
            print('     $emoji [${i + 1}] ${format.formatType}: ${format.dataSize} bytes ${format.isAvailable ? "✅" : "❌"}');
            if (format.contentPreview.isNotEmpty) {
              final preview = format.contentPreview.length > 50 
                  ? format.contentPreview.substring(0, 50) + "..."
                  : format.contentPreview;
              print('         Preview: "$preview"');
            }
          }
        } else {
          print('❌ No clipboard data available');
        }
      } catch (e) {
        print('❌ Error getting clipboard info: $e');
      }
      exit(0);
    }

    // Test comprehensive clipboard monitoring if requested
    if (results['clipboard'] as bool) {
      print('🧪 Testing comprehensive clipboard monitoring...');
      try {
        await testComprehensiveClipboardMonitoring();
        print('✅ Clipboard monitoring test completed successfully');
      } catch (e) {
        print('❌ Error testing clipboard monitoring: $e');
      }
      exit(0);
    }

    // Start continuous clipboard monitoring if requested
    if (results['clipboard-monitor'] as bool) {
      await startClipboardMonitoring();
      exit(0);
    }

    // Start silent monitoring if requested
    if (results['silent'] as bool) {
      await startSilentMonitoring();
      exit(0);
    }

    // Get configuration
    final enhanced = results['enhanced'] as bool;
    final verbose = int.parse(results['verbose'] as String);
    final background = results['background'] as bool;

    print('🚀 Starting Dart Clipboard Monitor with FRB Streaming...');
    print('Configuration: enhanced=$enhanced, verbose=$verbose, background=$background');

    // Check initial accessibility permissions
    final hasPermissions = await checkAccessibilityPermissions();
    if (!hasPermissions) {
      print('⚠️  Warning: No accessibility permissions granted');
      print('   Go to: System Preferences > Security & Privacy > Privacy > Accessibility');
      if (!background) {
        print('   Grant permissions and restart the application');
        exit(1);
      }
    }

    // Get current app info
    final currentApp = await getCurrentAppInfo();
    if (currentApp != null) {
      print('📱 Current app: ${currentApp.name} (${currentApp.bundleId})');
    }

    int eventCount = 0;
    late StreamSubscription<DartAppSwitchEventData> subscription;

    // Start the streaming AppSwitcher
    print('\n🔄 Starting real-time AppSwitcher monitoring...');
    final stream = monitorAppSwitches(
      enhanced: enhanced,
      verbose: verbose,
      background: background,
    );

    subscription = stream.listen(
      (DartAppSwitchEventData event) {
        eventCount++;
        _handleAppSwitchEvent(event, verbose, eventCount);
      },
      onError: (error) {
        print('\n❌ Stream error: $error');
        exit(1);
      },
      onDone: () {
        print('\n✅ Stream completed');
        exit(0);
      },
    );

    // Wait for monitoring to start
    await Future.delayed(Duration(seconds: 1));

    // Verify monitoring is active
    final isActive = await isMonitoring();
    print('🔍 Monitoring active: $isActive');

    if (!isActive) {
      print('❌ Failed to start monitoring');
      exit(1);
    }

    print('👀 Monitoring started. Press Ctrl+C to stop gracefully.');

    // Handle Ctrl+C gracefully
    ProcessSignal.sigint.watch().listen((signal) async {
      print('\n🛑 Stopping monitor...');
      await subscription.cancel();
      await stopMonitoring();
      print('✅ Monitor stopped (received $eventCount events)');
      exit(0);
    });

    // Keep the application running
    while (await isMonitoring()) {
      await Future.delayed(Duration(milliseconds: 500));
    }

  } catch (e, stackTrace) {
    print('❌ Error: $e');
    print('Stack trace: $stackTrace');
    exit(1);
  }
}

void _handleAppSwitchEvent(DartAppSwitchEventData event, int verbose, int eventCount) {
  final timestamp = DateTime.now().toIso8601String();
  
  print('\n🔥 [$eventCount] SWITCHED TO: ${event.appInfo.name} (${event.appInfo.bundleId})');
  
  if (event.previousApp != null) {
    print('   From: ${event.previousApp!.name} (pid: ${event.previousApp!.pid})');
  }
  
  if (event.windowTitle != null && event.windowTitle!.isNotEmpty) {
    print('   Window: ${event.windowTitle}');
  }
  
  if (event.url != null && event.url!.isNotEmpty) {
    print('   URL: ${event.url}');
  }
  
  if (verbose > 0) {
    print('   PID: ${event.appInfo.pid}');
    print('   Event: ${event.eventType}');
    print('   Time: $timestamp');
    
    if (event.appInfo.path != null) {
      print('   Path: ${event.appInfo.path}');
    }
  }
  
  print('   ─────────────────────────────────────');
}

/// Get appropriate emoji for clipboard format
String getFormatEmoji(String formatType) {
  final format = formatType.toLowerCase();
  
  // Text formats
  if (format.contains('text') || format.contains('string')) return '📝';
  if (format.contains('utf8')) return '🔤';
  
  // Web formats
  if (format.contains('html')) return '🌐';
  if (format.contains('url')) return '🔗';
  if (format.contains('web')) return '🕸️';
  
  // Rich text formats
  if (format.contains('rtf')) return '📄';
  
  // Image formats
  if (format.contains('png')) return '🖼️';
  if (format.contains('jpg') || format.contains('jpeg')) return '📸';
  if (format.contains('gif')) return '🎞️';
  if (format.contains('tiff') || format.contains('tif')) return '🖨️';
  if (format.contains('image')) return '🎨';
  
  // File formats
  if (format.contains('file')) return '📁';
  if (format.contains('path')) return '📂';
  
  // PDF formats
  if (format.contains('pdf')) return '📕';
  
  // Audio/Video
  if (format.contains('audio') || format.contains('sound')) return '🔊';
  if (format.contains('video') || format.contains('movie')) return '🎥';
  
  // Apple-specific
  if (format.contains('apple') || format.contains('ns')) return '🍎';
  
  // Browser-specific
  if (format.contains('chromium') || format.contains('chrome')) return '🟡';
  if (format.contains('firefox')) return '🦊';
  if (format.contains('safari')) return '🧭';
  
  // Microsoft formats
  if (format.contains('microsoft') || format.contains('office')) return '🏢';
  
  // Development
  if (format.contains('code') || format.contains('source')) return '💻';
  if (format.contains('json')) return '🔧';
  if (format.contains('xml')) return '📋';
  
  // Data formats
  if (format.contains('data') || format.contains('binary')) return '💾';
  if (format.contains('custom')) return '⚙️';
  
  // Default
  return '📦';
}

/// Silent monitoring with MAXIMUM context extraction
Future<void> startSilentMonitoring() async {
  print('🔇 ENHANCED SILENT CLIPBOARD MONITORING');
  print('==========================================');
  print('📍 Running for 30 minutes or until Ctrl+C');
  print('🔍 ONLY outputs when clipboard CHANGES');
  print('📊 Extracting MAXIMUM possible context:');
  print('   • Source app, bundle ID, PID, path');
  print('   • Window title, document path, tab name');
  print('   • Browser URL, page title, incognito mode');
  print('   • Space/desktop, display info');
  print('   • Accessibility focus, selected text');
  print('   • System state, session info');
  print('   • All clipboard formats and metadata');
  print('────────────────────────────────────────\n');

  int lastChangeCount = -1;
  int changeCount = 0;
  DartAppInfo? currentApp;
  DartAppInfo? lastClipboardSourceApp;
  String? lastWindowTitle;
  String? lastUrl;
  DateTime? lastAppSwitch;
  bool shouldStop = false;
  final startTime = DateTime.now();
  final maxDuration = Duration(minutes: 30);

  // Handle Ctrl+C gracefully
  ProcessSignal.sigint.watch().listen((signal) {
    print('\n🛑 Silent monitoring stopped by user');
    shouldStop = true;
  });

  // Start app switching monitoring in parallel
  late StreamSubscription appSwitchSubscription;
  
  try {
    final appSwitchStream = monitorAppSwitches(
      enhanced: true,
      verbose: 2,
      background: true,
    );
    
    appSwitchSubscription = appSwitchStream.listen(
      (appSwitchEvent) {
        currentApp = appSwitchEvent.appInfo;
        lastWindowTitle = appSwitchEvent.windowTitle;
        lastUrl = appSwitchEvent.url;
        lastAppSwitch = DateTime.now();
        
        // Debug: Track app switches (silent mode, so minimal output)
        // print('🔄 App switch: ${currentApp?.name} (${currentApp?.bundleId})');
      },
      onError: (error) {
        print('⚠️  App monitoring error: $error');
      },
    );
  } catch (e) {
    print('⚠️  Could not start app monitoring: $e');
  }

  // Monitor clipboard changes every 250ms for high precision
  Timer.periodic(Duration(milliseconds: 250), (timer) async {
    try {
      // Check if we should stop (user interrupt or timeout)
      if (shouldStop || DateTime.now().difference(startTime) > maxDuration) {
        timer.cancel();
        await appSwitchSubscription.cancel();
        if (!shouldStop) {
          print('\n⏰ 30-minute monitoring period completed');
          print('📊 Total clipboard changes detected: $changeCount');
        }
        shouldStop = true;
        return;
      }

      // Get current clipboard info silently (no debug output)
      final clipboardData = await getCurrentClipboardInfoSilent();
      if (clipboardData != null) {
        final currentChangeCount = clipboardData.changeCount;
        
        // Check if clipboard changed
        if (currentChangeCount != lastChangeCount && lastChangeCount != -1) {
          changeCount++;
          final timestamp = DateTime.now();
          
          // Determine most likely source app
          DartAppInfo? sourceApp = clipboardData.sourceApp;
          
          // If we have recent app switch info and no direct source, use current app
          if (sourceApp == null && currentApp != null) {
            sourceApp = currentApp;
          }
          
          // Check if clipboard is empty
          final isEmpty = clipboardData.primaryContent.trim().isEmpty && 
                         clipboardData.formats.isEmpty;
          
          if (isEmpty) {
            print('\n📋 CLIPBOARD EMPTIED');
          } else {
            print('\n🔥 CLIPBOARD CHANGE DETECTED #$changeCount');
          }
          
          print('⏰ Timestamp: ${timestamp.toIso8601String()}');
          print('🔢 Change Count: $lastChangeCount → $currentChangeCount');
          
          // Source application metadata
          if (sourceApp != null || clipboardData.sourceApp != null) {
            final app = sourceApp ?? clipboardData.sourceApp!;
            print('\n📱 SOURCE APPLICATION:');
            print('   Name: ${app.name}');
            print('   Bundle ID: ${app.bundleId}');
            print('   Process ID: ${app.pid}');
            if (app.path != null) {
              print('   App Path: ${app.path}');
            }
          }
          
          // Window context from enhanced clipboard data
          if (clipboardData.windowContext != null) {
            final wc = clipboardData.windowContext!;
            print('\n🪟 WINDOW CONTEXT:');
            if (wc.windowTitle != null && wc.windowTitle!.isNotEmpty) {
              print('   Title: "${wc.windowTitle}"');
            }
            print('   Window ID: ${wc.windowId}');
            print('   Layer: ${wc.windowLayer}');
            print('   Fullscreen: ${wc.isFullscreen}');
            print('   Minimized: ${wc.isMinimized}');
            if (wc.bounds != null) {
              final b = wc.bounds!;
              print('   Position: (${b.x}, ${b.y})');
              print('   Size: ${b.width} x ${b.height}');
            }
          } else if (lastWindowTitle != null && lastWindowTitle!.isNotEmpty) {
            print('\n🪟 Window Title: "$lastWindowTitle"');
          }
          
          // Browser context from enhanced clipboard data
          if (clipboardData.browserContext != null) {
            final bc = clipboardData.browserContext!;
            print('\n🌐 BROWSER CONTEXT:');
            if (bc.currentUrl != null && bc.currentUrl!.isNotEmpty) {
              print('   URL: ${bc.currentUrl}');
            }
            if (bc.pageTitle != null && bc.pageTitle!.isNotEmpty) {
              print('   Page Title: "${bc.pageTitle}"');
            }
            if (bc.tabCount != null) {
              print('   Tab Count: ${bc.tabCount}');
            }
            print('   Incognito Mode: ${bc.isIncognito}');
          } else if (lastUrl != null && lastUrl!.isNotEmpty) {
            print('🌐 URL Context: $lastUrl');
          }
          
          // Space context
          if (clipboardData.spaceContext != null) {
            final sc = clipboardData.spaceContext!;
            print('\n🖥️ SPACE/DESKTOP CONTEXT:');
            print('   Space Index: ${sc.spaceIndex}');
            print('   Space Name: ${sc.spaceName}');
            print('   Display UUID: ${sc.displayUuid}');
          }
          
          // Accessibility context
          if (clipboardData.accessibilityContext != null) {
            final ac = clipboardData.accessibilityContext!;
            print('\n♿ ACCESSIBILITY CONTEXT:');
            if (ac.focusedElementRole != null) {
              print('   Focused Element: ${ac.focusedElementRole}');
            }
            if (ac.focusedElementTitle != null) {
              print('   Element Title: "${ac.focusedElementTitle}"');
            }
            if (ac.selectedText != null && ac.selectedText!.isNotEmpty) {
              final preview = ac.selectedText!.length > 100
                  ? ac.selectedText!.substring(0, 100) + '...'
                  : ac.selectedText!;
              print('   Selected Text: "$preview"');
            }
            if (ac.documentPath != null) {
              print('   Document Path: ${ac.documentPath}');
            }
          }
          
          // System context
          final sys = clipboardData.systemContext;
          print('\n💻 SYSTEM CONTEXT:');
          print('   Display Count: ${sys.displayCount}');
          print('   Active Display ID: ${sys.activeDisplayId}');
          print('   Session Active: ${sys.sessionActive}');
          print('   Screen Locked: ${sys.screenLocked}');
          
          if (lastAppSwitch != null) {
            final switchDelta = timestamp.difference(lastAppSwitch!);
            print('⏱️  App Switch Timing: ${switchDelta.inMilliseconds}ms ago');
          }
          
          // Clipboard content metadata
          if (!isEmpty) {
            print('📝 Content Length: ${clipboardData.primaryContent.length} characters');
            
            // Content preview (first 100 chars, safely truncated)
            final contentPreview = clipboardData.primaryContent.length > 100 
                ? clipboardData.primaryContent.substring(0, 100) + '...'
                : clipboardData.primaryContent;
            print('👁️  Content Preview: "$contentPreview"');
            
            // Format analysis
            print('📊 Available Formats (${clipboardData.formats.length}):');
            for (int i = 0; i < clipboardData.formats.length; i++) {
              final format = clipboardData.formats[i];
              if (format.isAvailable) {
                final emoji = getFormatEmoji(format.formatType);
                print('   $emoji ${format.formatType}: ${format.dataSize} bytes');
                
                // Show preview for text-like formats
                if (format.contentPreview.isNotEmpty && 
                    (format.formatType.contains('text') || format.formatType.contains('html'))) {
                  final preview = format.contentPreview.length > 60 
                      ? format.contentPreview.substring(0, 60) + '...'
                      : format.contentPreview;
                  print('      Preview: "$preview"');
                }
              }
            }
            
            // Analyze content type
            final content = clipboardData.primaryContent.toLowerCase();
            if (content.startsWith('http://') || content.startsWith('https://')) {
              print('🔗 Content Type: URL');
            } else if (content.contains('@') && content.contains('.')) {
              print('📧 Content Type: Likely Email');
            } else if (content.startsWith('/') || content.contains('\\')) {
              print('📂 Content Type: Likely File Path');
            } else if (content.split('\n').length > 5) {
              print('📄 Content Type: Multi-line Text (${content.split('\n').length} lines)');
            } else {
              print('📝 Content Type: Single-line Text');
            }
          }
          
          print('─' * 60);
          lastClipboardSourceApp = sourceApp;
        }
        
        lastChangeCount = currentChangeCount;
      }
    } catch (e) {
      print('❌ Error monitoring clipboard: $e');
    }
  });

  // Keep running until stopped
  while (!shouldStop) {
    await Future.delayed(Duration(milliseconds: 100));
  }
  
  // Cleanup
  try {
    await appSwitchSubscription.cancel();
  } catch (e) {
    // Ignore cleanup errors
  }
}

Future<void> startClipboardMonitoring() async {
  print('📋 CONTINUOUS CLIPBOARD MONITORING');
  print('=====================================');
  print('⏰ Monitoring for 30 minutes or until Ctrl+C...');
  print('📝 Copy different content to see real-time detection');
  print('🔍 ONLY outputs when clipboard CHANGES\n');

  int changeCount = 0;
  int lastChangeCount = -1;
  final startTime = DateTime.now();
  final duration = Duration(minutes: 30);
  bool shouldStop = false;

  // Handle Ctrl+C gracefully
  ProcessSignal.sigint.watch().listen((signal) {
    print('\n🛑 Ctrl+C detected. Stopping clipboard monitoring...');
    shouldStop = true;
  });

  // Monitor clipboard changes every 500ms
  Timer.periodic(Duration(milliseconds: 500), (timer) async {
    try {
      // Check if we should stop
      if (shouldStop || DateTime.now().difference(startTime) >= duration) {
        timer.cancel();
        final elapsed = DateTime.now().difference(startTime);
        print('\n✅ MONITORING COMPLETE');
        print('📊 Session Summary:');
        print('   Duration: ${elapsed.inMinutes}m ${elapsed.inSeconds % 60}s');
        print('   Clipboard changes detected: $changeCount');
        print('   Final change count: $lastChangeCount');
        return;
      }

      // Get current clipboard info (with debug output for regular monitoring)
      final clipboardData = await getCurrentClipboardInfo();
      if (clipboardData != null) {
        final currentChangeCount = clipboardData.changeCount;
        
        // Check if clipboard changed
        if (currentChangeCount != lastChangeCount && lastChangeCount != -1) {
          changeCount++;
          final timestamp = DateTime.now().toIso8601String();
          
          print('🔥 CLIPBOARD CHANGE #$changeCount (Count: $currentChangeCount)');
          print('   ⏰ Time: $timestamp');
          print('   📝 Content: "${clipboardData.primaryContent}"');
          
          if (clipboardData.sourceApp != null) {
            print('   📱 Source: ${clipboardData.sourceApp!.name} (${clipboardData.sourceApp!.bundleId})');
          }
          
          print('   📊 Formats (${clipboardData.formats.length}):');
          for (int i = 0; i < clipboardData.formats.length; i++) {
            final format = clipboardData.formats[i];
            if (format.isAvailable) {
              final preview = format.contentPreview.length > 40 
                  ? format.contentPreview.substring(0, 40) + "..."
                  : format.contentPreview;
              final emoji = getFormatEmoji(format.formatType);
              print('      $emoji [${i + 1}] ${format.formatType}: ${format.dataSize} bytes');
              if (preview.isNotEmpty) {
                print('          Preview: "$preview"');
              }
            }
          }
          print('   ─────────────────────────────────────\n');
        }
        
        lastChangeCount = currentChangeCount;
      }
    } catch (e) {
      print('❌ Error monitoring clipboard: $e');
    }
  });

  // Wait for monitoring to complete
  while (!shouldStop && DateTime.now().difference(startTime) < duration) {
    await Future.delayed(Duration(milliseconds: 100));
  }
}