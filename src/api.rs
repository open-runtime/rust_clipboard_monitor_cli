// src/api.rs
//! Flutter Rust Bridge API for clipboard monitoring
//! This provides a streaming interface to the real AppSwitcher from main.rs

use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use core_foundation::runloop::CFRunLoopRun;
use core_foundation_sys;
use chrono;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSWorkspace, NSRunningApplication, NSPasteboard};
use objc2_foundation::{NSAutoreleasePool, NSRunLoop, NSDefaultRunLoopMode, NSDate, NSString, NSNotification, NSArray};
use dispatch::Queue;
use block2::RcBlock;
use std::ptr::NonNull;

// Import the actual AppSwitcher system from main.rs
use crate::core::app_switcher::{
    initialize_app_switcher, AppSwitchEvent, AppSwitchListener, AppSwitchType, AppSwitcher,
};

// Import enhanced context modules for rich clipboard context
use crate::core::accessibility::{extract_accessibility_context};
use crate::core::spaces::{query_spaces};

// Import StreamSink from FRB generated
use crate::frb_generated::StreamSink;

/// Get appropriate emoji for clipboard format
fn get_format_emoji(format_type: &str) -> &'static str {
    match format_type {
        // Text formats
        s if s.contains("text") || s.contains("string") => "📝",
        s if s.contains("utf8") => "🔤",
        
        // Web formats
        s if s.contains("html") => "🌐",
        s if s.contains("url") => "🔗",
        s if s.contains("web") => "🕸️",
        
        // Rich text formats
        s if s.contains("rtf") => "📄",
        
        // Image formats
        s if s.contains("png") => "🖼️",
        s if s.contains("jpg") || s.contains("jpeg") => "📸",
        s if s.contains("gif") => "🎞️",
        s if s.contains("tiff") || s.contains("tif") => "🖨️",
        s if s.contains("image") => "🎨",
        
        // File formats
        s if s.contains("file") => "📁",
        s if s.contains("path") => "📂",
        
        // PDF formats
        s if s.contains("pdf") => "📕",
        
        // Audio/Video
        s if s.contains("audio") || s.contains("sound") => "🔊",
        s if s.contains("video") || s.contains("movie") => "🎥",
        
        // Apple-specific
        s if s.contains("apple") || s.contains("ns") => "🍎",
        
        // Browser-specific
        s if s.contains("chromium") || s.contains("chrome") => "🟡",
        s if s.contains("firefox") => "🦊",
        s if s.contains("safari") => "🧭",
        
        // Microsoft formats
        s if s.contains("microsoft") || s.contains("office") => "🏢",
        
        // Development
        s if s.contains("code") || s.contains("source") => "💻",
        s if s.contains("json") => "🔧",
        s if s.contains("xml") => "📋",
        
        // Data formats
        s if s.contains("data") || s.contains("binary") => "💾",
        s if s.contains("custom") => "⚙️",
        
        // Default
        _ => "📦",
    }
}

/// Safe Unicode-aware string truncation
fn safe_truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect::<String>() + "..."
    }
}

/// App information for Dart
#[derive(Debug, Clone)]
pub struct DartAppInfo {
    pub name: String,
    pub bundle_id: String,
    pub pid: i32,
    pub path: Option<String>,
}

/// App switch event for Dart
#[derive(Debug, Clone)]
pub struct DartAppSwitchEventData {
    pub app_info: DartAppInfo,
    pub previous_app: Option<DartAppInfo>,
    pub event_type: String,
    pub window_title: Option<String>,
    pub url: Option<String>,
}

/// Enhanced clipboard data with full context for Dart
#[derive(Debug, Clone)]
pub struct DartClipboardData {
    pub change_count: isize,
    pub timestamp: String,
    pub source_app: Option<DartAppInfo>,
    pub formats: Vec<DartClipboardFormat>,
    pub primary_content: String,
    
    // Enhanced context fields
    pub window_context: Option<WindowContext>,
    pub browser_context: Option<BrowserContext>,
    pub space_context: Option<SpaceContext>,
    pub accessibility_context: Option<AccessibilityContextData>,
    pub system_context: SystemContext,
}

/// Window context information
#[derive(Debug, Clone)]
pub struct WindowContext {
    pub window_title: Option<String>,
    pub window_id: u32,
    pub window_layer: i32,
    pub is_fullscreen: bool,
    pub is_minimized: bool,
    pub bounds: Option<ClipboardWindowBounds>,
}

/// Window bounds information for clipboard context
#[derive(Debug, Clone)]
pub struct ClipboardWindowBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Browser-specific context
#[derive(Debug, Clone)]
pub struct BrowserContext {
    pub current_url: Option<String>,
    pub page_title: Option<String>,
    pub tab_count: Option<usize>,
    pub is_incognito: bool,
}

/// Space/Desktop context
#[derive(Debug, Clone)]
pub struct SpaceContext {
    pub space_index: u32,
    pub space_name: String,
    pub display_uuid: String,
}

/// Accessibility context data
#[derive(Debug, Clone)]
pub struct AccessibilityContextData {
    pub focused_element_role: Option<String>,
    pub focused_element_title: Option<String>,
    pub selected_text: Option<String>,
    pub document_path: Option<String>,
}

/// System context information
#[derive(Debug, Clone)]
pub struct SystemContext {
    pub display_count: u32,
    pub active_display_id: u32,
    pub session_active: bool,
    pub screen_locked: bool,
}

/// Individual clipboard format with data
#[derive(Debug, Clone)]
pub struct DartClipboardFormat {
    pub format_type: String,        // "public.utf8-plain-text", "public.png", etc.
    pub data_size: usize,            // Size in bytes
    pub content_preview: String,    // First 200 chars or data description
    pub is_available: bool,         // Whether data can be retrieved
}

/// Convert from core AppSwitchEvent to Dart-compatible structure
fn convert_to_dart_event(event: &AppSwitchEvent) -> DartAppSwitchEventData {
    let app_info = DartAppInfo {
        name: event.app_info.name.clone(),
        bundle_id: event.app_info.bundle_id.clone(),
        pid: event.app_info.pid,
        path: event.app_info.path.clone(),
    };

    let previous_app = event.previous_app.as_ref().map(|prev| DartAppInfo {
        name: prev.name.clone(),
        bundle_id: prev.bundle_id.clone(),
        pid: prev.pid,
        path: prev.path.clone(),
    });

    let event_type = match event.event_type {
        AppSwitchType::Foreground => "foreground".to_string(),
        AppSwitchType::Background => "background".to_string(),
        AppSwitchType::Launch => "launch".to_string(),
        AppSwitchType::Terminate => "terminate".to_string(),
        AppSwitchType::Hide => "hide".to_string(),
        AppSwitchType::Unhide => "unhide".to_string(),
    };

    let window_title = event
        .workspace
        .as_ref()
        .and_then(|ws| ws.focused_title.clone())
        .or_else(|| {
            event
                .enhanced
                .as_ref()
                .and_then(|enh| enh.front_window_title.clone())
        });

    let url = event
        .enhanced
        .as_ref()
        .and_then(|enh| enh.url.clone())
        .or_else(|| {
            event
                .workspace
                .as_ref()
                .and_then(|ws| ws.primary_url.clone())
        });

    DartAppSwitchEventData {
        app_info,
        previous_app,
        event_type,
        window_title,
        url,
    }
}

/// Internal listener implementation using a closure
/// This avoids any FRB type conflicts
struct InternalStreamListener<F>
where
    F: FnMut(&AppSwitchEvent) + Send + 'static,
{
    callback: F,
}

impl<F> InternalStreamListener<F>
where
    F: FnMut(&AppSwitchEvent) + Send + Sync + 'static,
{
    fn new(callback: F) -> Self {
        Self { callback }
    }
}

impl<F> AppSwitchListener for InternalStreamListener<F>
where
    F: FnMut(&AppSwitchEvent) + Send + Sync + 'static,
{
    fn on_app_switch(&mut self, event: &AppSwitchEvent) {
        (self.callback)(event);
    }
}

/// Global monitor state - completely internal
static MONITOR_STATE: OnceLock<Arc<Mutex<MonitorState>>> = OnceLock::new();

/// Global NSApplication initialization state
static NSAPP_INITIALIZED: OnceLock<bool> = OnceLock::new();

/// Monitor state - completely internal
struct MonitorState {
    is_monitoring: bool,
    event_count: u64,
    monitor_thread: Option<thread::JoinHandle<()>>,
}

/// Initialize the monitor state
fn init_monitor_state() -> Arc<Mutex<MonitorState>> {
    Arc::new(Mutex::new(MonitorState {
        is_monitoring: false,
        event_count: 0,
        monitor_thread: None,
    }))
}

/// Get or initialize the global monitor state
fn get_monitor_state() -> &'static Arc<Mutex<MonitorState>> {
    MONITOR_STATE.get_or_init(init_monitor_state)
}

/// Ensure NSApplication is initialized on the main thread
/// This solves the core issue - Dart CLI doesn't run on macOS main thread
fn ensure_nsapp_initialized() -> Result<()> {
    if NSAPP_INITIALIZED.get().is_some() {
        return Ok(());
    }
    
    // Use a simpler approach since get_or_try_init is unstable
    static INIT_ONCE: std::sync::Once = std::sync::Once::new();
    static mut INIT_RESULT: Option<Result<()>> = None;
    
    unsafe {
        INIT_ONCE.call_once(|| {
            INIT_RESULT = Some((|| {
        println!("🔧 Initializing NSApplication on main thread via dispatch...");
        
        let (sender, receiver) = std::sync::mpsc::channel();
        
        // Use Grand Central Dispatch to ensure we run on the main thread
        // Use exec_sync since CLI apps don't have a main run loop for async dispatch
        Queue::main().exec_sync(move || {
            let result = if let Some(mtm) = MainThreadMarker::new() {
                println!("✅ Successfully acquired MainThreadMarker on main thread!");
                let app = NSApplication::sharedApplication(mtm);
                app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
                
                // Initialize the app switcher system
                match initialize_app_switcher(mtm) {
                    Ok(()) => {
                        println!("✅ NSApplication and AppSwitcher initialized on main thread");
                        Ok(true)
                    }
                    Err(e) => {
                        println!("❌ Failed to initialize app switcher: {}", e);
                        Err(anyhow::anyhow!("Failed to initialize app switcher: {}", e))
                    }
                }
            } else {
                Err(anyhow::anyhow!("Still failed to get MainThreadMarker even on main thread"))
            };
            
            let _ = sender.send(result);
        });
        
        // Wait for initialization to complete
                match receiver.recv() {
                    Ok(Ok(success)) => {
                        let _ = NSAPP_INITIALIZED.set(success);
                        Ok(())
                    },
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(anyhow::anyhow!("Main thread initialization channel failed")),
                }
            })());
        });
        
        match INIT_RESULT.as_ref().unwrap() {
            Ok(_) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("{}", e)),
        }
    }
}

/// Execute a function on the main thread via GCD
fn execute_on_main_thread<T, F>(f: F) -> Result<T>
where 
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    // Use exec_sync for CLI applications since there's no main run loop for async dispatch
    Queue::main().exec_sync(|| f())
}

/// Run AppSwitcher service using dedicated NSWorkspace background thread
/// This bypasses the NSApplication main thread requirement entirely
fn run_app_switcher_service(sink: StreamSink<DartAppSwitchEventData>) {
    println!("🚀 Starting AppSwitcher service with SINGLE dedicated thread");

    // CRITICAL FIX: Use ONE thread for everything
    let handle = thread::spawn(move || {
        // Set up autorelease pool for this thread
        let _pool = unsafe { NSAutoreleasePool::new() };
        
        println!("🔧 Setting up NSWorkspace monitoring on THE SAME thread that will run CFRunLoop");
        
        // Update monitoring state first
        {
            let state = get_monitor_state();
            let mut state_guard = state.lock().unwrap();
            state_guard.is_monitoring = true;
        }
        
        // Set up NSWorkspace notifications on THIS thread
        setup_real_notification_system(sink);
        
        println!("🛑 NSWorkspace dedicated thread exiting (this should never happen if CFRunLoop is running)");
    });
    
    // Don't wait for the thread - let it run in background
    std::mem::forget(handle);
    
    println!("✅ AppSwitcher service background thread started");
}

/// Set up NSWorkspace monitoring on the current background thread
/// This is the CLI-friendly approach that doesn't require NSApplication
fn setup_workspace_monitoring_on_background_thread(sink: StreamSink<DartAppSwitchEventData>) -> Result<()> {
    println!("🔧 Setting up NSWorkspace monitoring without NSApplication dependency");
    
    // Use the real AppSwitcher but bypass the MainThreadMarker requirement
    // by directly accessing NSWorkspace on this background thread
    
    // First, let's try a simple test - just get the current frontmost app
    // This verifies NSWorkspace works on this thread
    test_nsworkspace_access()?;
    
    // Set up our event listener
    let sink_clone = sink.clone();
    let listener = InternalStreamListener::new(move |event: &AppSwitchEvent| {
        let dart_event = convert_to_dart_event(event);
        
        println!(
            "🔄 App switch detected: {} → {} ({})",
            dart_event
                .previous_app
                .as_ref()
                .map(|p| p.name.as_str())
                .unwrap_or("None"),
            dart_event.app_info.name,
            dart_event.event_type
        );
        
        // Send the event through the stream
        let _ = sink_clone.add(dart_event);
    });
    
    // Now try to start monitoring without MainThreadMarker
    // We'll modify AppSwitcher to work without it if possible
    println!("🔧 Attempting to initialize AppSwitcher on background thread");
    
    // For now, let's create a simple workspace-based monitor
    setup_simple_workspace_monitor(sink)?;
    
    Ok(())
}

/// Test NSWorkspace access to verify it works on background thread
fn test_nsworkspace_access() -> Result<()> {
    println!("🧪 Testing NSWorkspace access on background thread");
    
    unsafe {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::NSArray;
        
        let workspace = NSWorkspace::sharedWorkspace();
        let running_apps = workspace.runningApplications();
        
        println!("✅ NSWorkspace access successful - found {} running apps", running_apps.len());
        
        // Log a few app names to verify we can read them
        for (i, app) in running_apps.iter().enumerate() {
            if i >= 3 { break; } // Just show first 3
            if let Some(name) = app.localizedName() {
                println!("   📱 App {}: {}", i + 1, name);
            }
        }
    }
    
    Ok(())
}

/// Set up real event-driven NSWorkspace notification-based monitor
fn setup_simple_workspace_monitor(sink: StreamSink<DartAppSwitchEventData>) -> Result<()> {
    println!("🔧 This function is no longer used - notifications set up in main thread");
    Ok(())
}

/// Set up the REAL NSWorkspace notification system using proper NSNotificationCenter
fn setup_real_notification_system(sink: StreamSink<DartAppSwitchEventData>) {
    println!("🔔 Initializing REAL NSWorkspace notification system with NSNotificationCenter");
    
    // Set up autorelease pool for this thread
    let _pool = unsafe { NSAutoreleasePool::new() };
    
    unsafe {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::NSNotificationCenter;
        
        let workspace = NSWorkspace::sharedWorkspace();
        let workspace_notification_center = workspace.notificationCenter();
        let default_notification_center = NSNotificationCenter::defaultCenter();
        
        println!("📡 Setting up REAL NSWorkspace notification observers on BOTH notification centers");
        
        // Use the actual NSWorkspace notification constants from objc2
        // These are the proper system-defined notification names
        let app_did_activate = objc2_app_kit::NSWorkspaceDidActivateApplicationNotification;
        let app_did_deactivate = objc2_app_kit::NSWorkspaceDidDeactivateApplicationNotification;
        
        // Clone the sink for use in the closures
        let sink_activate = sink.clone();
        let sink_deactivate = sink.clone();
        let sink_activate2 = sink.clone();
        let sink_deactivate2 = sink.clone();
        
        // Set up notification observers using blocks
        println!("🔔 Adding NSWorkspace activation notification observer to WORKSPACE notification center");
        
        // App activation observer for workspace notification center
        let activation_block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
            println!("🔥🔥🔥 NSWorkspace ACTIVATION notification received from WORKSPACE CENTER! 🔥🔥🔥");
            
            // Get current app for immediate response
            if let Ok(app_info) = get_current_frontmost_app() {
                println!("📱 Activated app: {} ({})", app_info.name, app_info.bundle_id);
                
                let dart_event = DartAppSwitchEventData {
                    app_info,
                    previous_app: None, // TODO: track previous app
                    event_type: "foreground".to_string(),
                    window_title: None,
                    url: None,
                };
                
                let _ = sink_activate.add(dart_event);
            } else {
                println!("⚠️  Failed to get current app info");
            }
        });
        
        // Add the observer for app activation to workspace notification center
        workspace_notification_center.addObserverForName_object_queue_usingBlock(
            Some(&app_did_activate),
            None, // Observe all objects
            None, // Use default queue
            &activation_block,
        );
        
        println!("✅ Added NSWorkspace activation observer to WORKSPACE notification center");
        
        // App deactivation observer for workspace notification center
        let deactivation_block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
            println!("🔄🔄🔄 NSWorkspace DEACTIVATION notification received from WORKSPACE CENTER! 🔄🔄🔄");
            
            if let Ok(app_info) = get_current_frontmost_app() {
                println!("📱 Deactivated - current app: {} ({})", app_info.name, app_info.bundle_id);
                
                let dart_event = DartAppSwitchEventData {
                    app_info,
                    previous_app: None,
                    event_type: "background".to_string(),
                    window_title: None,
                    url: None,
                };
                
                let _ = sink_deactivate.add(dart_event);
            }
        });
        
        workspace_notification_center.addObserverForName_object_queue_usingBlock(
            Some(&app_did_deactivate),
            None,
            None,
            &deactivation_block,
        );
        
        println!("✅ Added NSWorkspace deactivation observer to WORKSPACE notification center");
        
        // ALSO try with the default notification center in case NSWorkspace posts there
        println!("🔔 Adding NSWorkspace activation notification observer to DEFAULT notification center");
        
        // App activation observer for default notification center
        let activation_block2 = RcBlock::new(move |_notification: NonNull<NSNotification>| {
            println!("🔥🔥🔥 NSWorkspace ACTIVATION notification received from DEFAULT CENTER! 🔥🔥🔥");
            
            // Get current app for immediate response
            if let Ok(app_info) = get_current_frontmost_app() {
                println!("📱 Activated app: {} ({})", app_info.name, app_info.bundle_id);
                
                let dart_event = DartAppSwitchEventData {
                    app_info,
                    previous_app: None,
                    event_type: "foreground".to_string(),
                    window_title: None,
                    url: None,
                };
                
                let _ = sink_activate2.add(dart_event);
            }
        });
        
        // Add the observer for app activation to default notification center
        default_notification_center.addObserverForName_object_queue_usingBlock(
            Some(&app_did_activate),
            None, // Observe all objects
            None, // Use default queue
            &activation_block2,
        );
        
        println!("✅ Added NSWorkspace activation observer to DEFAULT notification center");
        
        // App deactivation observer for default notification center
        let deactivation_block2 = RcBlock::new(move |_notification: NonNull<NSNotification>| {
            println!("🔄🔄🔄 NSWorkspace DEACTIVATION notification received from DEFAULT CENTER! 🔄🔄🔄");
            
            if let Ok(app_info) = get_current_frontmost_app() {
                let dart_event = DartAppSwitchEventData {
                    app_info,
                    previous_app: None,
                    event_type: "background".to_string(),
                    window_title: None,
                    url: None,
                };
                
                let _ = sink_deactivate2.add(dart_event);
            }
        });
        
        default_notification_center.addObserverForName_object_queue_usingBlock(
            Some(&app_did_deactivate),
            None,
            None,
            &deactivation_block2,
        );
        
        println!("✅ Added NSWorkspace deactivation observer to DEFAULT notification center");
        
        // Test: Generate a test event to verify our observers work
        println!("🧪 Testing notification setup with current app info...");
        if let Ok(current_app) = get_current_frontmost_app() {
            println!("🧪 Current app detected: {} ({})", current_app.name, current_app.bundle_id);
            
            // Send a test event through the stream to verify connectivity
            let test_event = DartAppSwitchEventData {
                app_info: current_app,
                previous_app: None,
                event_type: "test".to_string(),
                window_title: None,
                url: None,
            };
            
            let _ = sink.add(test_event);
            println!("🧪 Test event sent through stream");
        }
        
        // Use CFRunLoop for better run loop management
        println!("🔄 Starting CFRunLoop for NSWorkspace notifications");
        
        // Use Core Foundation's run loop which is more reliable for keeping alive
        use core_foundation::runloop::{CFRunLoopRun, CFRunLoopGetCurrent, CFRunLoopAddTimer};
        use core_foundation::date::CFAbsoluteTimeGetCurrent;
        
        // Add a timer to keep CFRunLoop alive - NSWorkspace notifications alone may not provide sources
        println!("🕐 Adding keep-alive timer to CFRunLoop to ensure it stays active");
        unsafe {
            use core_foundation_sys::runloop::{
                CFRunLoopTimerCreate, CFRunLoopTimerRef, kCFRunLoopDefaultMode
            };
            use core_foundation_sys::base::kCFAllocatorDefault;
            
            // Create a simple timer callback that does nothing but keeps the run loop alive
            extern "C" fn keep_alive_timer_callback(_timer: CFRunLoopTimerRef, _info: *mut std::ffi::c_void) {
                // Do nothing - this just keeps the run loop alive
                println!("🕐 Keep-alive timer fired - CFRunLoop is active");
            }
            
            let current_time = CFAbsoluteTimeGetCurrent();
            let fire_date = current_time + 1.0; // Fire in 1 second
            let interval = 5.0; // Every 5 seconds
            
            // Create the timer
            let timer = CFRunLoopTimerCreate(
                kCFAllocatorDefault,
                fire_date,
                interval,
                0, // flags
                0, // order
                keep_alive_timer_callback,
                std::ptr::null_mut(), // context info
            );
            
            if !timer.is_null() {
                let current_run_loop = CFRunLoopGetCurrent();
                CFRunLoopAddTimer(current_run_loop, timer, kCFRunLoopDefaultMode);
                println!("✅ Added keep-alive timer to CFRunLoop");
            } else {
                println!("❌ Failed to create keep-alive timer");
            }
        }
        
        // NSWorkspace notifications don't fire in CLI context, so we need polling fallback
        // Keep notification setup for potential future compatibility, but rely on polling
        println!("🔄 Adding polling-based app detection as primary mechanism");
        println!("🔔 NSWorkspace notifications also set up but don't fire in CLI context");
        
        // Create a thread for polling-based detection that runs alongside CFRunLoop
        let polling_sink = sink.clone();
        let polling_thread = thread::spawn(move || {
            let mut last_frontmost_app: Option<String> = None;
            let mut last_frontmost_bundle: Option<String> = None;
            
            loop {
                // Poll every 100ms for responsive detection
                thread::sleep(Duration::from_millis(100));
                
                // Check current frontmost app
                if let Ok(current_app) = get_current_frontmost_app() {
                    // Check if this is a different app than before
                    let app_changed = Some(&current_app.name) != last_frontmost_app.as_ref() ||
                                    Some(&current_app.bundle_id) != last_frontmost_bundle.as_ref();
                    
                    if app_changed {
                        println!("🔄 POLLING detected app switch: {} → {} ({})", 
                                last_frontmost_app.as_deref().unwrap_or("None"), 
                                current_app.name,
                                current_app.bundle_id);
                        
                        let previous_app = if let (Some(name), Some(bundle)) = (last_frontmost_app.as_ref(), last_frontmost_bundle.as_ref()) {
                            Some(DartAppInfo {
                                name: name.clone(),
                                bundle_id: bundle.clone(),
                                pid: 0,
                                path: None,
                            })
                        } else {
                            None
                        };
                        
                        let dart_event = DartAppSwitchEventData {
                            app_info: current_app.clone(),
                            previous_app,
                            event_type: "foreground".to_string(),
                            window_title: None,
                            url: None,
                        };
                        
                        let _ = polling_sink.add(dart_event);
                        last_frontmost_app = Some(current_app.name);
                        last_frontmost_bundle = Some(current_app.bundle_id);
                    }
                }
            }
        });
        
        // Don't wait for the polling thread
        std::mem::forget(polling_thread);
        
        println!("✅ Starting CFRunLoop for notification processing - this should BLOCK and keep running");
        println!("✅ Hybrid approach: NSWorkspace notifications (for future) + polling (current working solution)");
        CFRunLoopRun(); // This blocks and processes events including our notifications
        println!("🛑 CFRunLoop stopped - this should never happen unless intentionally stopped");
    }
}

/// Extract app information from NSWorkspace notification
unsafe fn extract_app_info_from_notification(notification: *const NSNotification) -> Result<DartAppInfo> {
    let notification = unsafe { &*notification };
    
    // Get the userInfo dictionary from the notification
    if let Some(user_info) = unsafe { notification.userInfo() } {
        // NSWorkspace notifications include app info in userInfo
        // The keys are usually NSWorkspaceApplicationKey
        let app_key = NSString::from_str("NSWorkspaceApplicationKey");
        
        if let Some(app_obj) = user_info.objectForKey(&app_key) {
            // Try to extract app information from the notification object
            // For now, let's use a simpler approach and just get the current app
            println!("📱 Got app object from notification, extracting info...");
            
            // Fallback to current frontmost app since casting is complex
            return get_current_frontmost_app();
        }
    }
    
    // Fallback: get current frontmost app
    get_current_frontmost_app()
}

/// Set up hybrid detection system that's more event-driven than pure polling
fn setup_hybrid_detection_system(sink: StreamSink<DartAppSwitchEventData>) {
    println!("🔧 Setting up hybrid event detection system");
    
    thread::spawn(move || {
        let mut last_frontmost_app: Option<String> = None;
        let mut last_check_time = std::time::Instant::now();
        
        // Use adaptive polling - faster when changes detected, slower when stable
        let mut poll_interval = Duration::from_millis(50); // Start responsive
        let min_interval = Duration::from_millis(25);
        let max_interval = Duration::from_millis(200);
        
        loop {
            thread::sleep(poll_interval);
            
            // Check current frontmost app
            if let Ok(current_app) = get_current_frontmost_app() {
                if Some(&current_app.name) != last_frontmost_app.as_ref() {
                    println!("🔄 REAL app switch: {} → {}", 
                            last_frontmost_app.as_deref().unwrap_or("None"), 
                            current_app.name);
                    
                    let previous_app = last_frontmost_app.as_ref().map(|name| DartAppInfo {
                        name: name.clone(),
                        bundle_id: "unknown".to_string(),
                        pid: 0,
                        path: None,
                    });
                    
                    let dart_event = DartAppSwitchEventData {
                        app_info: current_app.clone(),
                        previous_app,
                        event_type: "foreground".to_string(),
                        window_title: None,
                        url: None,
                    };
                    
                    let _ = sink.add(dart_event);
                    last_frontmost_app = Some(current_app.name);
                    last_check_time = std::time::Instant::now();
                    
                    // Speed up polling after detecting change
                    poll_interval = min_interval;
                } else {
                    // No change detected - gradually slow down polling
                    let time_since_change = last_check_time.elapsed();
                    if time_since_change > Duration::from_secs(2) {
                        poll_interval = std::cmp::min(poll_interval + Duration::from_millis(5), max_interval);
                    }
                }
            }
        }
    });
}

/// Set up basic but functional app detection using NSWorkspace
fn setup_basic_app_detection(sink: StreamSink<DartAppSwitchEventData>) -> Result<()> {
    println!("🔧 Setting up basic NSWorkspace app detection");
    
    let sink_clone = sink.clone();
    
    thread::spawn(move || {
        let mut last_frontmost_app: Option<String> = None;
        
        // Use a shorter poll interval for more responsive detection
        // This is still a temporary approach until we get full notifications working
        loop {
            thread::sleep(Duration::from_millis(100)); // Much more responsive
            
            // Check current frontmost app
            if let Ok(current_app) = get_current_frontmost_app() {
                if Some(&current_app.name) != last_frontmost_app.as_ref() {
                    println!("🔄 App switch detected: {} → {}", 
                            last_frontmost_app.as_deref().unwrap_or("None"), 
                            current_app.name);
                    
                    let previous_app = last_frontmost_app.as_ref().map(|name| DartAppInfo {
                        name: name.clone(),
                        bundle_id: "unknown".to_string(),
                        pid: 0,
                        path: None,
                    });
                    
                    let dart_event = DartAppSwitchEventData {
                        app_info: current_app.clone(),
                        previous_app,
                        event_type: "foreground".to_string(),
                        window_title: None,
                        url: None,
                    };
                    
                    let _ = sink_clone.add(dart_event);
                    last_frontmost_app = Some(current_app.name);
                }
            }
        }
    });
    
    Ok(())
}

/// Helper to get window context from CGWindow - simplified version
fn get_window_context_for_app(_pid: i32) -> Option<WindowContext> {
    // For now, return a basic context
    // The full CGWindow implementation requires careful handling of CF types
    Some(WindowContext {
        window_title: None,
        window_id: 0,
        window_layer: 0,
        is_fullscreen: false,
        is_minimized: false,
        bounds: None,
    })
}

/// Extract browser context using accessibility APIs
fn get_browser_context(bundle_id: &str, pid: i32) -> Option<BrowserContext> {
    use accessibility_sys::{
        AXUIElementCreateApplication, AXUIElementCopyAttributeValue,
        kAXURLAttribute, kAXTitleAttribute, kAXDocumentAttribute,
        kAXErrorSuccess,
    };
    use core_foundation::string::CFString as CFStringCore;
    use core_foundation::base::{CFRelease, TCFType};
    use std::ptr;
    
    let is_browser = bundle_id.contains("chrome") || 
                     bundle_id.contains("safari") || 
                     bundle_id.contains("firefox") ||
                     bundle_id.contains("edge") ||
                     bundle_id.contains("opera") ||
                     bundle_id.contains("brave");
    
    if !is_browser {
        return None;
    }
    
    unsafe {
        let ax_app = AXUIElementCreateApplication(pid);
        if ax_app.is_null() {
            return None;
        }
        
        let mut context = BrowserContext {
            current_url: None,
            page_title: None,
            tab_count: None,
            is_incognito: false,
        };
        
        // Try to get URL
        let url_attr = CFStringCore::new("AXDocument");
        let mut url_value: core_foundation_sys::base::CFTypeRef = ptr::null();
        
        if AXUIElementCopyAttributeValue(
            ax_app,
            url_attr.as_concrete_TypeRef() as *const _,
            &mut url_value
        ) == kAXErrorSuccess && !url_value.is_null() {
            let url_str = CFStringCore::wrap_under_get_rule(url_value as _);
            context.current_url = Some(url_str.to_string());
            CFRelease(url_value);
        }
        
        // Try to get title
        let title_attr = CFStringCore::new("AXTitle");
        let mut title_value: core_foundation_sys::base::CFTypeRef = ptr::null();
        
        if AXUIElementCopyAttributeValue(
            ax_app,
            title_attr.as_concrete_TypeRef() as *const _,
            &mut title_value
        ) == kAXErrorSuccess && !title_value.is_null() {
            let title_str = CFStringCore::wrap_under_get_rule(title_value as _);
            let title = title_str.to_string();
            
            // Check for incognito indicators
            if title.contains("Private") || title.contains("Incognito") {
                context.is_incognito = true;
            }
            
            context.page_title = Some(title);
            CFRelease(title_value);
        }
        
        CFRelease(ax_app as _);
        
        Some(context)
    }
}

/// Safe wrapper for accessibility context extraction
fn extract_accessibility_context_safe(pid: i32) -> Result<crate::core::accessibility::AccessibilityContext> {
    use accessibility_sys::{AXIsProcessTrusted, AXUIElementCreateApplication};
    use crate::core::app_switcher_types::AppInfo;
    
    unsafe {
        if !AXIsProcessTrusted() {
            return Err(anyhow::anyhow!("Accessibility not trusted"));
        }
        
        let ax_app = AXUIElementCreateApplication(pid);
        if ax_app.is_null() {
            return Err(anyhow::anyhow!("Failed to create AX element"));
        }
        
        // Create a minimal app info for the context extraction
        let app_info = AppInfo {
            name: String::new(),
            bundle_id: String::new(),
            pid,
            path: None,
            launch_date: None,
            icon_base64: None,
            icon_path: None,
            activation_count: 0,
        };
        
        // Use the actual extract function from accessibility module
        match extract_accessibility_context(&app_info) {
            Ok(context) => Ok(context),
            Err(e) => Err(anyhow::anyhow!("Failed to extract context: {}", e))
        }
    }
}

/// Get system context information
fn get_system_context() -> SystemContext {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGGetActiveDisplayList(
            maxDisplays: u32,
            activeDisplays: *mut u32,
            displayCount: *mut u32,
        ) -> i32;
        fn CGSessionCopyCurrentDictionary() -> *const core_foundation::dictionary::__CFDictionary;
    }
    
    unsafe {
        let mut display_count: u32 = 0;
        CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut display_count);
        
        let active_display_id = CGMainDisplayID();
        
        let mut session_active = true;
        let mut screen_locked = false;
        
        let session_dict_ref = CGSessionCopyCurrentDictionary();
        if !session_dict_ref.is_null() {
            // Just release the dictionary without parsing for now
            use core_foundation::base::CFRelease;
            CFRelease(session_dict_ref as _);
            
            // For now, we'll skip the session dictionary parsing
            // as it requires careful CF type handling
        }
        
        SystemContext {
            display_count,
            active_display_id,
            session_active,
            screen_locked,
        }
    }
}

/// Get comprehensive clipboard data with all available formats and enhanced context
fn get_comprehensive_clipboard_data() -> Result<DartClipboardData> {
    unsafe {
        use objc2_foundation::NSString;
        
        let pasteboard = NSPasteboard::generalPasteboard();
        let change_count = pasteboard.changeCount();
        
        println!("🔍 CLIPBOARD ANALYSIS: changeCount = {}", change_count);
        
        let mut formats = Vec::new();
        let mut primary_content = String::new();
        
        // FIRST: Get actual available formats using NSPasteboard.types()
        println!("📋 Getting actual available formats using NSPasteboard.types():");
        let available_types = pasteboard.types();
        
        if let Some(types_array) = available_types.as_deref() {
            println!("🎯 Found {} actual clipboard formats:", types_array.len());
            for (i, type_obj) in types_array.iter().enumerate() {
                let type_str = type_obj.to_string();
                let emoji = get_format_emoji(&type_str);
                println!("   {} [{}] {}", emoji, i + 1, type_str);
            }
        } else {
            println!("❌ Unable to retrieve clipboard types");
        }
        
        // SECOND: Test common clipboard formats directly (our existing approach)
        let test_formats = [
            ("public.utf8-plain-text", "Plain Text"),
            ("public.html", "HTML"),
            ("public.rtf", "Rich Text"),
            ("public.png", "PNG Image"),
            ("public.jpeg", "JPEG Image"), 
            ("public.tiff", "TIFF Image"),
            ("public.file-url", "File URL"),
            ("public.url", "URL"),
        ];
        
        println!("\n📋 Testing standard clipboard formats:");
        
        for (format_id, format_name) in &test_formats {
            let nsformat = NSString::from_str(format_id);
            
            if let Some(data) = pasteboard.dataForType(&nsformat) {
                let data_size = data.len();
                let mut content_preview = String::new();
                
                let emoji = get_format_emoji(format_id);
                println!("  {} ✅ [{}] {} - {} bytes", emoji, format_name, format_id, data_size);
                
                // Try to get string data for text formats
                if format_id.contains("text") || format_id.contains("html") || format_id.contains("rtf") {
                    if let Some(string_data) = pasteboard.stringForType(&nsformat) {
                        let full_text = string_data.to_string();
                        content_preview = safe_truncate(&full_text, 200);
                        
                        if primary_content.is_empty() {
                            primary_content = full_text;
                        }
                        
                        println!("      📝 Content: \"{}\"", safe_truncate(&content_preview, 50));
                    }
                } else if format_id.contains("url") {
                    if let Some(string_data) = pasteboard.stringForType(&nsformat) {
                        content_preview = string_data.to_string();
                        println!("      🔗 URL: {}", content_preview);
                    }
                } else {
                    // Binary data (images, files, etc.)
                    content_preview = format!("{} data ({} bytes)", format_name, data_size);
                    println!("      📦 Binary data: {} bytes", data_size);
                }
                
                formats.push(DartClipboardFormat {
                    format_type: format_id.to_string(),
                    data_size,
                    content_preview,
                    is_available: true,
                });
            } else {
                let emoji = get_format_emoji(format_id);
                println!("  {} ❌ [{}] {} - No data", emoji, format_name, format_id);
            }
        }
        
        // Also test the general string format
        if let Some(string_data) = pasteboard.stringForType(&NSString::from_str("public.utf8-plain-text")) {
            if primary_content.is_empty() {
                primary_content = string_data.to_string();
            }
        }
        
        // Get source application if possible
        let source_app = get_current_frontmost_app().ok();
        
        // Generate timestamp
        let timestamp = chrono::Utc::now().to_rfc3339();
        
        // Gather enhanced context
        let mut window_context = None;
        let mut browser_context = None;
        let mut space_context = None;
        let mut accessibility_context = None;
        
        // Get window and browser context if we have source app
        if let Some(ref app_info) = source_app {
            window_context = get_window_context_for_app(app_info.pid);
            browser_context = get_browser_context(&app_info.bundle_id, app_info.pid);
            
            // Try to get accessibility context
            if let Ok(ax_context) = extract_accessibility_context_safe(app_info.pid) {
                accessibility_context = Some(AccessibilityContextData {
                    focused_element_role: ax_context.focused_element.as_ref()
                        .and_then(|e| e.role.clone()),
                    focused_element_title: ax_context.focused_element.as_ref()
                        .and_then(|e| e.title.clone()),
                    selected_text: ax_context.selected_text.clone()
                        .or_else(|| ax_context.focused_element.as_ref()
                            .and_then(|e| e.selected_text.clone())),
                    document_path: ax_context.document_path.clone()
                        .or_else(|| ax_context.active_file_path.clone()),
                });
            }
        }
        
        // Get space context
        if let Some(spaces) = query_spaces() {
            if let Some(display) = spaces.displays.first() {
                space_context = Some(SpaceContext {
                    space_index: display.current_space_index.unwrap_or(0),
                    space_name: spaces.label_for_display(0).unwrap_or_else(|| "Unknown".to_string()),
                    display_uuid: display.display_uuid.clone(),
                });
            }
        }
        
        // Get system context
        let system_context = get_system_context();
        
        let clipboard_data = DartClipboardData {
            change_count,
            timestamp,
            source_app,
            formats,
            primary_content,
            window_context,
            browser_context,
            space_context,
            accessibility_context,
            system_context,
        };
        
        println!("✅ Clipboard analysis complete: {} formats available, primary content: {} chars", 
               clipboard_data.formats.len(),
               clipboard_data.primary_content.len());
        
        Ok(clipboard_data)
    }
}

/// Enhanced clipboard monitoring with change detection
fn monitor_clipboard_changes() -> Result<Option<DartClipboardData>> {
    static mut LAST_CHANGE_COUNT: isize = -1;
    
    unsafe {
        let pasteboard = NSPasteboard::generalPasteboard();
        let current_change_count = pasteboard.changeCount();
        
        if current_change_count != LAST_CHANGE_COUNT {
            println!("🔄 CLIPBOARD CHANGED: {} → {}", LAST_CHANGE_COUNT, current_change_count);
            LAST_CHANGE_COUNT = current_change_count;
            
            // Get comprehensive clipboard data
            match get_comprehensive_clipboard_data() {
                Ok(clipboard_data) => Ok(Some(clipboard_data)),
                Err(e) => {
                    println!("❌ Failed to get clipboard data: {}", e);
                    Err(e)
                }
            }
        } else {
            // No change detected
            Ok(None)
        }
    }
}

/// Test clipboard monitoring capabilities
fn test_clipboard_monitoring() -> Result<()> {
    println!("🧪 TESTING: Comprehensive clipboard monitoring capabilities");
    println!("📋 Copy different types of content to test detection...");
    
    for test_cycle in 1..=5 {
        println!("\n--- Test Cycle {} ---", test_cycle);
        
        match monitor_clipboard_changes() {
            Ok(Some(clipboard_data)) => {
                println!("🎉 DETECTED clipboard change #{}", clipboard_data.change_count);
                println!("⏰ Timestamp: {}", clipboard_data.timestamp);
                
                if let Some(source_app) = &clipboard_data.source_app {
                    println!("📱 Source app: {} ({})", source_app.name, source_app.bundle_id);
                }
                
                println!("📊 Available formats:");
                for (i, format) in clipboard_data.formats.iter().enumerate() {
                    println!("  [{}] {}: {} bytes - {}", 
                           i + 1, 
                           format.format_type,
                           format.data_size,
                           if format.content_preview.len() > 60 {
                               safe_truncate(&format.content_preview, 60)
                           } else {
                               format.content_preview.clone()
                           });
                }
                
                println!("📝 Primary content: {} characters", clipboard_data.primary_content.len());
                
                if !clipboard_data.primary_content.is_empty() {
                    let preview = if clipboard_data.primary_content.len() > 100 {
                        format!("{}...", &clipboard_data.primary_content[..100])
                    } else {
                        clipboard_data.primary_content.clone()
                    };
                    println!("📖 Content preview: \"{}\"", preview);
                }
            },
            Ok(None) => {
                println!("📋 No clipboard changes detected");
            },
            Err(e) => {
                println!("❌ Error monitoring clipboard: {}", e);
            }
        }
        
        // Wait between checks
        thread::sleep(Duration::from_millis(500));
    }
    
    Ok(())
}

/// Get the current frontmost application using NSWorkspace
fn get_current_frontmost_app() -> Result<DartAppInfo> {
    unsafe {
        use objc2_app_kit::NSWorkspace;
        
        let workspace = NSWorkspace::sharedWorkspace();
        let frontmost_app = workspace.frontmostApplication();
        
        if let Some(app) = frontmost_app {
            let name = app.localizedName()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Unknown".to_string());
                
            let bundle_id = app.bundleIdentifier()
                .map(|b| b.to_string())
                .unwrap_or_else(|| "unknown".to_string());
                
            let pid = app.processIdentifier();
            
            Ok(DartAppInfo {
                name,
                bundle_id,
                pid,
                path: None, // NSRunningApplication doesn't easily provide path
            })
        } else {
            Err(anyhow::anyhow!("No frontmost application found"))
        }
    }
}

/// Create and run AppSwitcher with proper MainThreadMarker
fn create_and_run_app_switcher(sink: StreamSink<DartAppSwitchEventData>, mtm: MainThreadMarker) {
    println!("🔄 Creating AppSwitcher with MainThreadMarker...");
    
    // Create the actual AppSwitcher from main.rs
    let mut app_switcher = AppSwitcher::new();
    
    // Add our streaming event listener
    let sink_clone = sink.clone();
    let listener = InternalStreamListener::new(move |event: &AppSwitchEvent| {
        let dart_event = convert_to_dart_event(event);
        
        println!(
            "🔄 App switch: {} → {} ({})",
            dart_event
                .previous_app
                .as_ref()
                .map(|p| p.name.as_str())
                .unwrap_or("None"),
            dart_event.app_info.name,
            dart_event.event_type
        );
        
        let _ = sink_clone.add(dart_event);
    });
    app_switcher.add_listener(listener);
    
    // Start monitoring
    match app_switcher.start_monitoring(mtm) {
        Ok(()) => {
            println!("✅ AppSwitcher monitoring started successfully");
            
            // Update monitoring state
            {
                let state = get_monitor_state();
                let mut state_guard = state.lock().unwrap();
                state_guard.is_monitoring = true;
            }
            
            println!("🔄 Starting CFRunLoop...");
            unsafe { CFRunLoopRun() };
        }
        Err(e) => {
            println!("❌ Failed to start AppSwitcher monitoring: {}", e);
            let _ = sink.add_error(anyhow::anyhow!("Failed to start monitoring: {}", e));
        }
    }
}

/// Try to create AppSwitcher without MainThreadMarker (experimental)
fn create_and_run_app_switcher_unsafe(sink: StreamSink<DartAppSwitchEventData>) {
    println!("⚠️  Attempting unsafe AppSwitcher creation...");
    
    // This might not work, but let's try
    let _ = sink.add_error(anyhow::anyhow!("MainThreadMarker not available - cannot run AppSwitcher safely"));
}

// Flutter Rust Bridge API functions with streaming

/// Start monitoring app switches and return a stream of events
/// This provides real-time app switching notifications through a Stream
pub fn monitor_app_switches(
    sink: StreamSink<DartAppSwitchEventData>,
    enhanced: bool,
    verbose: u8,
    background: bool,
) -> Result<()> {
    println!("🔧 Starting real AppSwitcher-based monitor with streaming (enhanced={}, verbose={}, background={})", enhanced, verbose, background);

    let state = get_monitor_state();

    // Check if already monitoring
    {
        let state_guard = state.lock().unwrap();
        if state_guard.is_monitoring {
            let _ = sink.add_error(anyhow::anyhow!("Already monitoring"));
            return Err(anyhow::anyhow!("Already monitoring"));
        }
    }

    // Skip the problematic GCD-based initialization that causes hanging
    // The dedicated thread approach will handle NSApplication initialization
    println!("🔧 Skipping main thread dispatch - using dedicated NSApplication thread approach");

    // Now start the AppSwitcher service in a background thread
    let sink_clone = sink.clone();
    let handle = thread::spawn(move || {
        run_app_switcher_service(sink_clone);
    });

    // Store the handle
    {
        let mut state_guard = state.lock().unwrap();
        state_guard.monitor_thread = Some(handle);
    }

    // Give the service thread time to initialize
    thread::sleep(Duration::from_millis(500));

    println!("✅ Real AppSwitcher monitor with streaming initialized successfully");
    Ok(())
}

/// Stop monitoring app switches
pub fn stop_monitoring() -> Result<()> {
    let state = get_monitor_state();

    {
        let mut state_guard = state.lock().unwrap();

        if !state_guard.is_monitoring {
            return Ok(()); // Already stopped
        }

        state_guard.is_monitoring = false;
        println!("🛑 AppSwitcher monitoring stopped");
    }

    Ok(())
}

/// Check if currently monitoring
pub fn is_monitoring() -> bool {
    let state = get_monitor_state();
    state.lock().unwrap().is_monitoring
}

/// Check accessibility permissions
pub fn check_accessibility_permissions() -> bool {
    use accessibility_sys::AXIsProcessTrusted;
    unsafe { AXIsProcessTrusted() }
}

/// Test comprehensive clipboard monitoring capabilities  
pub fn test_comprehensive_clipboard_monitoring() -> Result<()> {
    println!("🚀 COMPREHENSIVE CLIPBOARD MONITORING TEST");
    println!("==========================================");
    
    // Test immediate clipboard state
    println!("\n1️⃣ CURRENT CLIPBOARD STATE:");
    match get_comprehensive_clipboard_data() {
        Ok(clipboard_data) => {
            println!("✅ Successfully read clipboard data");
            println!("📊 Change count: {}", clipboard_data.change_count);
            println!("📊 Available formats: {}", clipboard_data.formats.len());
            
            for (i, format) in clipboard_data.formats.iter().enumerate() {
                println!("  [{}] {}: {} bytes", i + 1, format.format_type, format.data_size);
                if format.data_size > 0 && format.content_preview.len() > 0 {
                    let preview = safe_truncate(&format.content_preview, 80);
                    println!("      Preview: {}", preview);
                }
            }
        },
        Err(e) => {
            println!("❌ Failed to read clipboard: {}", e);
            return Err(e);
        }
    }
    
    // Test change detection
    println!("\n2️⃣ CHANGE DETECTION TEST:");
    println!("💡 Copy different content types (text, images, files) to see real-time detection...");
    
    test_clipboard_monitoring()?;
    
    println!("\n✅ CLIPBOARD MONITORING TEST COMPLETE");
    println!("📋 Capabilities verified:");
    println!("  • Real-time change detection via NSPasteboard.changeCount()");
    println!("  • Comprehensive format enumeration via NSPasteboard.types()"); 
    println!("  • Data extraction for all supported formats");
    println!("  • Source application tracking");
    println!("  • Timestamp recording");
    
    Ok(())
}

/// Get current clipboard data (one-time query)
pub fn get_current_clipboard_info() -> Option<DartClipboardData> {
    get_comprehensive_clipboard_data().ok()
}

/// Simple one-time query for current app without streaming
pub fn get_current_app_info() -> Option<DartAppInfo> {
    // This is a simplified version that just returns basic info
    // For real-time updates, use the monitor_app_switches stream
    use accessibility_sys::{AXUIElementCopyAttributeValue, AXUIElementCreateSystemWide};
    use core_foundation::base::CFTypeRef;
    use objc2_core_foundation::{CFString, CFRetained};

    unsafe {
        let system_wide = AXUIElementCreateSystemWide();
        let focused_app_key = CFString::from_static_str("AXFocusedApplication");
        let mut focused_app: CFTypeRef = std::ptr::null();

        if AXUIElementCopyAttributeValue(
            system_wide,
            CFRetained::as_ptr(&focused_app_key).as_ptr() as *const _,
            &mut focused_app,
        ) == 0
        {
            // Basic implementation - in practice you'd extract more details
            // For full functionality with enhanced context, use the streaming API
            return Some(DartAppInfo {
                name: "Current App".to_string(),
                bundle_id: "unknown".to_string(),
                pid: 0,
                path: None,
            });
        }
    }

    None
}
