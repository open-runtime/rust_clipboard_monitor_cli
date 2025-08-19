#![cfg(target_os = "macos")]

use std::collections::{HashMap, HashSet};
use std::ffi::{c_void, CStr};
use std::ptr::null_mut;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use arboard::Clipboard;

// Import the clipboard file reader module
mod clipboard_file_reader;
use clipboard_file_reader::{read_file_contents_safe, is_text_file};

use accessibility_sys::*;

// Declare external functions not exposed by accessibility_sys
extern "C" {
    fn AXUIElementGetAttributeValueCount(
        element: AXUIElementRef,
        attribute: CFStringRef,
        count: *mut CFIndex,
    ) -> AXError;
    
    fn AXUIElementCopyAttributeValues(
        element: AXUIElementRef,
        attribute: CFStringRef,
        index: CFIndex,
        maxValues: CFIndex,
        values: *mut CFTypeRef,
    ) -> AXError;
}
use clap::Parser;
use cocoa::appkit::{NSApp, NSApplication, NSApplicationActivationPolicyProhibited, NSPasteboard};
use cocoa::base::{id, nil};
use cocoa::foundation::NSAutoreleasePool;
use core_foundation::array::CFArray;
use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::runloop::{CFRunLoop, CFRunLoopRun, kCFRunLoopDefaultMode};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation_sys::runloop::{CFRunLoopAddSource, CFRunLoopRemoveSource};
use core_foundation_sys::base::{CFGetTypeID, CFTypeID, CFIndex, CFGetRetainCount};
use core_foundation_sys::string::CFStringGetTypeID;
use core_foundation_sys::array::{CFArrayRef, CFArrayGetCount, CFArrayGetValueAtIndex};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType};
use core_graphics::event_source::CGEventSource;

// Direct FFI for CGEventTap
extern "C" {
    fn CGEventTapCreate(
        tap: CGEventTapLocation,
        place: i32, // CGEventTapPlacement
        options: i32, // CGEventTapOptions
        events_of_interest: u64,
        callback: extern "C" fn(
            proxy: *mut c_void,
            event_type: CGEventType,
            event: *mut c_void,
            user_info: *mut c_void,
        ) -> *mut c_void,
        user_info: *mut c_void,
    ) -> *mut c_void;
    
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    
    fn CFMachPortCreateRunLoopSource(
        allocator: *mut c_void,
        port: *mut c_void,
        order: i64,
    ) -> *mut c_void;
    
    fn CGEventGetFlags(event: *mut c_void) -> CGEventFlags;
    
    fn CGEventGetIntegerValueField(event: *mut c_void, field: i32) -> i64;
    
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
    
    fn CGEventGetDoubleValueField(event: *mut c_void, field: i32) -> f64;
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

// CGEventTapPlacement
const kCGHeadInsertEventTap: i32 = 0;

// CGEventTapOptions
const kCGEventTapOptionDefault: i32 = 0;

// CGEventField
const kCGKeyboardEventKeycode: i32 = 9;
const kCGScrollWheelEventDeltaAxis1: i32 = 11;
const kCGScrollWheelEventDeltaAxis2: i32 = 12;
const kCGMouseEventButtonNumber: i32 = 3;
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(name = "focus-tracker", about = "Ultimate macOS artificial memory system")]
struct Cli {
    #[arg(long, default_value = "json", value_parser = ["text", "json"])]
    format: String,
    
    #[arg(long)]
    no_prompt: bool,
    
    #[arg(long, default_value_t = true, help = "Enable clipboard monitoring")]
    clipboard: bool,
    
    #[arg(long, default_value_t = true, help = "Enable network monitoring")]
    network: bool,
    
    #[arg(long, default_value_t = true, help = "Enable deep content extraction")]
    deep: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ClipboardEvent {
    timestamp: u128,
    event_type: String, // "copy", "paste", "cut"
    content: Option<String>,
    content_type: Option<String>,
    source_app: String,
    source_bundle_id: String,
    source_window: Option<String>,
    source_url: Option<String>,
    source_file: Option<String>,
    file_paths: Vec<String>, // If clipboard contains file references
    metadata: HashMap<String, String>,
    all_formats: Option<HashMap<String, serde_json::Value>>, // All clipboard formats
}

#[derive(Debug, Clone, Serialize)]
struct Context {
    // Application
    app_name: String,
    bundle_id: String,
    pid: i32,
    app_path: Option<String>,
    
    // Window
    window_title: Option<String>,
    document_path: Option<String>,
    document_modified: Option<bool>,
    
    // Web (browsers) - ENHANCED
    url: Option<String>,
    actual_url: Option<String>, // From address bar VALUE
    page_title: Option<String>,
    tab_count: Option<usize>,
    
    // IDE (Cursor/VS Code/JetBrains) - ENHANCED
    active_file: Option<String>,
    project: Option<String>,
    open_files: Vec<String>,
    git_branch: Option<String>,
    
    // Terminal - ENHANCED
    terminal_tab: Option<String>,
    terminal_cwd: Option<String>,
    terminal_command: Option<String>,
    
    // Spreadsheets
    sheet_name: Option<String>,
    
    // Finder - ENHANCED
    finder_path: Option<String>,
    selected_files: Vec<String>,
    
    // Communication apps
    channel_name: Option<String>,
    conversation: Option<String>,
    
    // Clipboard
    clipboard_text: Option<String>,
    clipboard_type: Option<String>,
    
    // Network (if enabled)
    active_connections: Vec<String>,
    
    // UI State - ENHANCED
    focused_element: Option<ElementInfo>,
    ui_path: Vec<String>,
    all_attributes: HashMap<String, String>,
    
    // Mouse/Keyboard state
    mouse_position: Option<(f64, f64)>,
    key_modifiers: Option<String>,
    
    // Timing
    timestamp: u128,
    duration_ms: Option<u128>,
    #[serde(skip)]
    started_at: Instant,
}

#[derive(Debug, Clone, Serialize)]
struct ElementInfo {
    role: Option<String>,
    title: Option<String>,
    value: Option<String>,
    description: Option<String>,
    identifier: Option<String>,
    url: Option<String>,
    selected_text: Option<String>,
    // New fields for better context
    placeholder: Option<String>,
    help: Option<String>,
    label: Option<String>,
    subrole: Option<String>,
    role_description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Event {
    event_type: String,
    timestamp: u128,
    from_context: Option<Context>,
    to_context: Context,
    trigger: String,
    transition_details: Option<String>,
}

#[derive(Debug)]
struct Tracker {
    current_context: Option<Context>,
    json_output: bool,
    current_observer: Option<usize>,
    start_time: Instant,
    url_times: HashMap<String, Duration>,
    current_url_start: Option<(String, Instant)>,
    // New tracking fields
    clipboard_monitor: bool,
    network_monitor: bool,
    deep_extraction: bool,
    context_history: Vec<Context>,
    last_clipboard: Option<String>,
    last_clipboard_change_count: i64,
    last_event_time: Option<Instant>,
    // Enhanced clipboard tracking
    clipboard_events: Vec<ClipboardEvent>,
    clipboard_thread_handle: Option<thread::JoinHandle<()>>,
    last_copy_context: Option<Context>, // Context when copy happened
    last_paste_context: Option<Context>, // Context when paste happened
    last_keyboard_shortcut: Option<(String, Instant)>, // Track Cmd+C/V/X
    // Mouse tracking
    last_mouse_position: Option<CGPoint>,
    // Scroll tracking
    last_scroll_time: Option<Instant>,
}

static STATE: OnceLock<Arc<Mutex<Tracker>>> = OnceLock::new();

// Global event tap storage using AtomicPtr for thread safety
static KEYBOARD_TAP: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static KEYBOARD_TAP_SOURCE: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static SCROLL_TAP: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static SCROLL_TAP_SOURCE: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

// Global callback for keyboard shortcuts
extern "C" fn keyboard_event_callback(
    _proxy: *mut c_void,
    event_type: CGEventType,
    event: *mut c_void,
    _user_info: *mut c_void,
) -> *mut c_void {
    unsafe {
        // Only process key down events
        if event_type as u32 != CGEventType::KeyDown as u32 {
            return event;
        }
        
        let flags = CGEventGetFlags(event);
        let keycode = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode) as u16;
        
        // Check if Command key is pressed
        let cmd_pressed = flags.bits() & (1 << 20) != 0; // Cmd flag is bit 20
        
        if cmd_pressed {
            let action = match keycode {
                8 => Some("copy"),   // C key
                9 => Some("paste"),  // V key  
                7 => Some("cut"),    // X key
                _ => None,
            };
            
            if let Some(action) = action {
                // Get mouse position when keyboard shortcut is pressed
                let mouse_pos = CGEventGetLocation(event);
                
                if let Some(state) = STATE.get() {
                    if let Ok(mut tracker) = state.lock() {
                        let now = Instant::now();
                        
                        // Debounce rapid key presses
                        if let Some((_, last_time)) = &tracker.last_keyboard_shortcut {
                            if now.duration_since(*last_time).as_millis() < 50 {
                                return event;
                            }
                        }
                        
                        tracker.last_keyboard_shortcut = Some((action.to_string(), now));
                        tracker.last_mouse_position = Some(mouse_pos);
                        tracker.handle_clipboard_shortcut_with_position(action, mouse_pos);
                    }
                }
            }
        }
        
        event
    }
}

// Global callback for scroll events
extern "C" fn scroll_event_callback(
    _proxy: *mut c_void,
    event_type: CGEventType,
    event: *mut c_void,
    _user_info: *mut c_void,
) -> *mut c_void {
    unsafe {
        if event_type as u32 != CGEventType::ScrollWheel as u32 {
            return event;
        }
        
        let delta_y = CGEventGetDoubleValueField(event, kCGScrollWheelEventDeltaAxis1);
        let delta_x = CGEventGetDoubleValueField(event, kCGScrollWheelEventDeltaAxis2);
        let mouse_pos = CGEventGetLocation(event);
        
        if delta_y.abs() > 0.1 || delta_x.abs() > 0.1 {
            if let Some(state) = STATE.get() {
                if let Ok(mut tracker) = state.lock() {
                    let now = Instant::now();
                    
                    // Debounce scroll events
                    if let Some(last_time) = tracker.last_scroll_time {
                        if now.duration_since(last_time).as_millis() < 200 {
                            return event;
                        }
                    }
                    
                    tracker.last_scroll_time = Some(now);
                    tracker.last_mouse_position = Some(mouse_pos);
                    tracker.handle_scroll_event_with_position(delta_x, delta_y, mouse_pos);
                }
            }
        }
        
        event
    }
}

// Safe CFRelease wrapper to prevent crashes
unsafe fn safe_cf_release(cf_ref: CFTypeRef) {
    if !cf_ref.is_null() {
        // Check retain count to avoid double-release
        let retain_count = CFGetRetainCount(cf_ref);
        if retain_count > 0 && retain_count < 1000 { // Sanity check
            CFRelease(cf_ref);
        }
    }
}

impl Tracker {
    fn new(cli: &Cli) -> Self {
        Self {
            current_context: None,
            json_output: cli.format == "json",
            current_observer: None,
            start_time: Instant::now(),
            url_times: HashMap::new(),
            current_url_start: None,
            clipboard_monitor: cli.clipboard,
            network_monitor: cli.network,
            deep_extraction: cli.deep,
            context_history: Vec::new(),
            last_clipboard: None,
            last_clipboard_change_count: 0,
            last_event_time: None,
            clipboard_events: Vec::new(),
            clipboard_thread_handle: None,
            last_copy_context: None,
            last_paste_context: None,
            last_keyboard_shortcut: None,
            last_mouse_position: None,
            last_scroll_time: None,
        }
    }

    fn extract_context(&mut self, app_name: String, bundle_id: String, pid: i32) -> Context {
        unsafe {
            let app_element = AXUIElementCreateApplication(pid);
            
            // Mine ALL attributes from the app element first
            let app_attrs = self.mine_all_attributes(app_element);
            
            let mut ctx = Context {
                app_name: app_name.clone(),
                bundle_id: bundle_id.clone(),
                pid,
                app_path: self.get_app_path(&bundle_id),
                window_title: None,
                document_path: None,
                document_modified: None,
                url: None,
                actual_url: None,
                page_title: None,
                tab_count: None,
                active_file: None,
                project: None,
                open_files: Vec::new(),
                git_branch: None,
                terminal_tab: None,
                terminal_cwd: None,
                terminal_command: None,
                sheet_name: None,
                finder_path: None,
                selected_files: Vec::new(),
                channel_name: None,
                conversation: None,
                clipboard_text: None,
                clipboard_type: None,
                active_connections: Vec::new(),
                focused_element: None,
                ui_path: Vec::new(),
                all_attributes: HashMap::new(),
                mouse_position: None,
                key_modifiers: None,
                timestamp: self.start_time.elapsed().as_millis(),
                duration_ms: None,
                started_at: Instant::now(),
            };
            
            // Get clipboard if enabled and changed - use changeCount for efficiency
            if self.clipboard_monitor {
                let current_change_count = self.get_clipboard_change_count();
                if current_change_count != self.last_clipboard_change_count {
                    // Clipboard has changed - read the new content
                    let current_clipboard = self.get_clipboard_text();
                    ctx.clipboard_text = current_clipboard.clone();
                    ctx.clipboard_type = if current_clipboard.is_some() {
                        self.get_clipboard_type()
                    } else {
                        None
                    };
                    // Update tracking
                    self.last_clipboard = current_clipboard;
                    self.last_clipboard_change_count = current_change_count;
                }
            }
            
            // Get network connections if enabled
            if self.network_monitor {
                ctx.active_connections = self.get_network_connections(pid);
            }
            
            // Try to extract URL from app attributes
            ctx.url = app_attrs.get("AXURL").cloned()
                .or_else(|| app_attrs.get("AXDocument").cloned());

            // Get window info
            if let Some(window_ref) = self.get_attribute(app_element, "AXFocusedWindow") {
                let window = window_ref as AXUIElementRef;
                
                ctx.window_title = self.get_string_attr(window, "AXTitle");
                ctx.document_modified = self.get_bool_attr(window, "AXIsDocumentEdited");
                
                // Mine ALL window attributes
                let window_attrs = self.mine_all_attributes(window);
                
                // Browser and Chrome App URL extraction - ULTIMATE SOLUTION
                if bundle_id.contains("chrome") || bundle_id.contains("Chrome") || 
                   bundle_id.contains("safari") || bundle_id.contains("Safari") ||
                   bundle_id.contains("firefox") || bundle_id.contains("edge") || 
                   bundle_id.contains("com.google") || bundle_id.contains("arc") || 
                   bundle_id.contains("brave") {
                    
                    // Safari-specific URL extraction using AppleScript
                    if bundle_id.contains("Safari") {
                        // Use AppleScript for Safari - safe and reliable
                        if let Ok(url) = self.get_safari_url_via_applescript() {
                            ctx.url = Some(url.clone());
                            ctx.actual_url = Some(url);
                        }
                        
                        // Get page title
                        if let Ok(title) = self.get_safari_title_via_applescript() {
                            ctx.page_title = Some(title);
                        }
                        
                        // Skip the problematic accessibility-based extraction
                        if false {
                        // Safari stores URL differently - need to check multiple places
                        
                        // Method 1: Check the web area directly
                        if let Some(web_area) = self.find_safari_web_area(window) {
                            let web_attrs = self.mine_all_attributes(web_area);
                            ctx.url = web_attrs.get("AXURL").cloned()
                                .or_else(|| web_attrs.get("AXDocument").cloned())
                                .or_else(|| web_attrs.get("AXTitle").cloned());
                            
                            // Get page title from web area
                            if let Some(title) = web_attrs.get("AXTitle") {
                                ctx.page_title = Some(title.clone());
                            }
                            
                            CFRelease(web_area as CFTypeRef);
                        }
                        
                        // Method 2: Find Safari's address field (it's different from Chrome)
                        if ctx.url.is_none() {
                            if let Some(toolbar) = self.find_safari_toolbar(window) {
                                let toolbar_attrs = self.mine_all_attributes(toolbar);
                                
                                // Look for the URL field in toolbar
                                if let Some(url_field) = self.find_safari_url_field(toolbar) {
                                    if let Some(url) = self.get_string_attr(url_field, "AXValue")
                                        .or_else(|| self.get_string_attr(url_field, "AXURL")) {
                                        ctx.url = Some(url);
                                    }
                                    CFRelease(url_field as CFTypeRef);
                                }
                                
                                CFRelease(toolbar as CFTypeRef);
                            }
                        }
                        
                        // Method 3: Extract from UI hierarchy
                        if ctx.url.is_none() && !ctx.ui_path.is_empty() {
                            // The ui_path contains "Sign in to Google" which might be the page title
                            for path_elem in &ctx.ui_path {
                                if path_elem.contains("http") || path_elem.contains("www.") {
                                    ctx.url = Some(path_elem.clone());
                                    break;
                                }
                            }
                        }
                        } // Close the if false block for Safari problematic code
                    }
                    
                    // CRITICAL: Get URL from browser - try multiple methods
                    // Method 1: AppleScript (most reliable for Chrome)
                    if bundle_id.contains("chrome") || bundle_id.contains("Chrome") {
                        if let Ok(url) = self.get_chrome_url_via_applescript() {
                            if !url.is_empty() && url != "missing value" {
                                ctx.url = Some(url.clone());
                                ctx.actual_url = Some(url);
                            }
                        }
                    }
                    
                    // Method 2: Accessibility API fallback
                    {
                        // First try focused element (it might be the address bar)
                        if let Some(focused_ref) = self.get_attribute(app_element, "AXFocusedUIElement") {
                            let focused = focused_ref as AXUIElementRef;
                            let focused_attrs = self.mine_all_attributes(focused);
                            
                            // Check if this is the address bar
                            if let Some(role) = focused_attrs.get("AXRole") {
                                if role == "AXTextField" || role == "AXComboBox" {
                                    if let Some(desc) = focused_attrs.get("AXDescription") {
                                        if desc.to_lowercase().contains("address") || 
                                           desc.to_lowercase().contains("search") ||
                                           desc.to_lowercase().contains("url") {
                                            // This is the address bar - get its VALUE!
                                            if let Some(actual_url) = focused_attrs.get("AXValue") {
                                                // Clean up the URL
                                                let clean_url = if !actual_url.starts_with("http") && 
                                                                   !actual_url.starts_with("file:") &&
                                                                   actual_url.contains(".") {
                                                    format!("https://{}", actual_url)
                                                } else {
                                                    actual_url.clone()
                                                };
                                                // ALWAYS override with the current address bar value
                                                ctx.actual_url = Some(clean_url.clone());
                                                ctx.url = Some(clean_url);
                                            }
                                        }
                                    }
                                }
                            }
                            
                            // Store focused element info
                            ctx.focused_element = Some(self.extract_element_info(focused));
                            CFRelease(focused_ref);
                        }
                    }
                    
                    // If we still don't have URL, try generic methods
                    if ctx.url.is_none() {
                        ctx.url = window_attrs.get("AXURL").cloned()
                            .or_else(|| window_attrs.get("AXDocument").cloned())
                            .or_else(|| self.deep_mine_for_url(window))
                            .or_else(|| self.extract_url_from_address_bar(window))
                            .or_else(|| self.extract_url_from_tab_content(window));
                    }
                    
                    // Store all window attributes for deep analysis
                    ctx.all_attributes = window_attrs.clone();
                    
                    // Track URL dwell time
                    if let Some(url) = &ctx.url {
                        self.update_url_time(url.clone());
                    }
                }
                
                // IDE file path extraction - get FULL paths
                if app_name.contains("Code") || app_name.contains("Cursor") || 
                   app_name.contains("IntelliJ") || app_name.contains("WebStorm") {
                    
                    // First try to get the document path directly
                    let doc_path = self.get_string_attr(window, "AXDocument")
                        .or_else(|| window_attrs.get("AXDocument").cloned())
                        .or_else(|| window_attrs.get("AXPath").cloned())
                        .or_else(|| window_attrs.get("AXFilename").cloned())
                        .map(|path| {
                            if path.starts_with("file://") {
                                urlencoding::decode(&path[7..])
                                    .unwrap_or_else(|_| path[7..].into())
                                    .to_string()
                            } else {
                                path
                            }
                        });
                    
                    // Extract project and filename from window title
                    if let Some(title) = &ctx.window_title {
                        let parts: Vec<&str> = title.split(" — ").collect();
                        if parts.len() >= 2 {
                            let filename = parts[0].to_string();
                            let project_name = parts[1].to_string();
                            ctx.project = Some(project_name.clone());
                            
                            // Try to construct full path
                            if let Some(doc) = doc_path {
                                ctx.active_file = Some(doc);
                            } else {
                                // Try to find project root and construct path
                                let full_path = self.construct_full_path(&filename, &project_name);
                                ctx.active_file = full_path.or(Some(filename));
                            }
                        }
                    } else {
                        ctx.active_file = doc_path;
                    }
                    
                    // Mine the focused element for file paths
                    if ctx.active_file.is_none() || !ctx.active_file.as_ref().unwrap().starts_with("/") {
                        if let Some(focused_ref) = self.get_attribute(app_element, "AXFocusedUIElement") {
                            let focused = focused_ref as AXUIElementRef;
                            let focused_attrs = self.mine_all_attributes(focused);
                            
                            if let Some(path) = focused_attrs.get("AXDocument").cloned()
                                .or_else(|| focused_attrs.get("AXPath").cloned())
                                .or_else(|| focused_attrs.get("AXFilename").cloned())
                                .or_else(|| focused_attrs.get("AXURL").cloned()) {
                                
                                if path.starts_with("file://") {
                                    let clean_path = urlencoding::decode(&path[7..])
                                        .unwrap_or_else(|_| path[7..].into())
                                        .to_string();
                                    ctx.active_file = Some(clean_path);
                                } else if path.starts_with("/") {
                                    ctx.active_file = Some(path);
                                }
                            }
                            
                            CFRelease(focused_ref);
                        }
                        
                        // ALWAYS actively search for address bar to get current URL
                        // This ensures we get the current tab's URL even if address bar isn't focused
                        {
                            // Search through all window children for text fields
                            if let Some(address_bar) = self.find_address_bar(window) {
                                if let Some(url_value) = self.get_string_attr(address_bar, "AXValue") {
                                    let clean_url = if !url_value.starts_with("http") && 
                                                       !url_value.starts_with("file:") &&
                                                       url_value.contains(".") {
                                        format!("https://{}", url_value)
                                    } else {
                                        url_value.clone()
                                    };
                                    // Always update URL with current address bar value
                                    ctx.url = Some(clean_url.clone());
                                    ctx.actual_url = Some(clean_url);
                                }
                                CFRelease(address_bar as CFTypeRef);
                            }
                        }
                    }
                }
                
                // Terminal tab
                if app_name.contains("Terminal") || app_name.contains("iTerm") {
                    ctx.terminal_tab = ctx.window_title.clone();
                }
                
                // QuickTime Player - extract file path
                if app_name.contains("QuickTime") {
                    if let Ok(file_path) = self.get_quicktime_file_path_via_applescript() {
                        ctx.active_file = Some(file_path.clone());
                        ctx.document_path = Some(file_path);
                    }
                }
                
                // Spreadsheets
                if app_name.contains("Excel") || app_name.contains("Numbers") || 
                   app_name.contains("Sheets") {
                    ctx.sheet_name = ctx.window_title.clone();
                }
                
                // Finder path extraction - get full paths, parent folder, and metadata
                if app_name == "Finder" {
                    // Try multiple methods to get the current Finder location
                    ctx.document_path = self.get_string_attr(window, "AXDocument")
                        .or_else(|| self.get_string_attr(window, "AXURL"))
                        .or_else(|| window_attrs.get("AXDocument").cloned())
                        .or_else(|| window_attrs.get("AXURL").cloned())
                        .or_else(|| window_attrs.get("AXPath").cloned())
                        .map(|path| {
                            if path.starts_with("file://") {
                                urlencoding::decode(&path[7..])
                                    .unwrap_or_else(|_| path[7..].into())
                                    .to_string()
                            } else {
                                path
                            }
                        });
                    
                    // Try to extract more Finder information through deep Accessibility API traversal
                    // Get selected items using the full Accessibility API capabilities
                    let selected_items = self.extract_finder_selection(window);
                    if !selected_items.is_empty() {
                        // Use first selected item as document path if we don't have one
                        if ctx.document_path.is_none() && !selected_items.is_empty() {
                            ctx.document_path = Some(selected_items[0].clone());
                        }
                        // Store all selected items
                        ctx.active_file = Some(format!("Selected: {}", selected_items.join(", ")));
                    }
                    
                    // Extract parent folder from window title or path  
                    if let Some(ref doc_path) = ctx.document_path {
                        if let Some(parent) = std::path::Path::new(doc_path).parent() {
                            if let Some(folder_name) = parent.file_name() {
                                ctx.project = Some(format!("Parent: {}", folder_name.to_string_lossy()));
                            }
                        }
                        
                        // Also get the full parent path
                        if let Some(parent_path) = std::path::Path::new(doc_path).parent() {
                            ctx.terminal_tab = Some(format!("Full Path: {}", parent_path.display()));
                        }
                    }
                    
                    // Try to get selected items in Finder for more context
                    if ctx.active_file.is_none() {
                        if let Some(selected_ref) = self.get_attribute(window, "AXSelectedChildren") {
                            let selected_items = self.get_selected_finder_items(selected_ref);
                            if !selected_items.is_empty() {
                                ctx.active_file = Some(format!("Selected: {}", selected_items.join(", ")));
                            }
                            CFRelease(selected_ref);
                        }
                    }
                }
                
                CFRelease(window_ref);
            }
            
            // Get focused element with deep mining
            if let Some(focused_ref) = self.get_attribute(app_element, "AXFocusedUIElement") {
                let focused = focused_ref as AXUIElementRef;
                ctx.focused_element = Some(self.extract_element_info(focused));
                ctx.ui_path = self.build_ui_path(focused);
                
                // If we still don't have a URL, try to get it from the focused element
                if ctx.url.is_none() {
                    ctx.url = self.deep_mine_for_url(focused);
                }
                
                CFRelease(focused_ref);
            }
            
            // Last resort: scan all top-level windows for URLs
            if ctx.url.is_none() && (bundle_id.contains("chrome") || bundle_id.contains("Chrome") || 
                                      bundle_id.contains("safari") || bundle_id.contains("com.google")) {
                ctx.url = self.scan_all_windows_for_url(app_element);
            }
            
            CFRelease(app_element as CFTypeRef);
            ctx
        }
    }

    fn extract_element_info(&self, element: AXUIElementRef) -> ElementInfo {
        // Mine ALL attributes from the element
        let attrs = self.mine_all_attributes(element);
        
        ElementInfo {
            role: attrs.get("AXRole").cloned()
                .or_else(|| attrs.get("AXSubrole").cloned()),
            title: attrs.get("AXTitle").cloned()
                .or_else(|| attrs.get("AXLabel").cloned()),
            value: attrs.get("AXValue").cloned()
                .or_else(|| attrs.get("AXStringValue").cloned()),
            description: attrs.get("AXDescription").cloned()
                .or_else(|| attrs.get("AXRoleDescription").cloned()),
            identifier: attrs.get("AXIdentifier").cloned()
                .or_else(|| attrs.get("AXDOMIdentifier").cloned()),
            url: attrs.get("AXURL").cloned()
                .or_else(|| attrs.get("AXDocument").cloned())
                .or_else(|| attrs.get("AXPath").cloned())
                .or_else(|| attrs.get("AXFilename").cloned()),
            selected_text: attrs.get("AXSelectedText").cloned()
                .or_else(|| attrs.get("AXSelectedTextRange").cloned()),
            placeholder: attrs.get("AXPlaceholderValue").cloned(),
            help: attrs.get("AXHelp").cloned(),
            label: attrs.get("AXLabel").cloned(),
            subrole: attrs.get("AXSubrole").cloned(),
            role_description: attrs.get("AXRoleDescription").cloned(),
        }
    }

    fn build_ui_path(&self, element: AXUIElementRef) -> Vec<String> {
        let mut path = Vec::new();
        unsafe {
            let mut current = element;
            for _ in 0..5 {
                if let Some(parent_ref) = self.get_attribute(current, "AXParent") {
                    let parent = parent_ref as AXUIElementRef;
                    if let Some(role) = self.get_string_attr(parent, "AXRole") {
                        let title = self.get_string_attr(parent, "AXTitle")
                            .unwrap_or_else(|| role.clone());
                        path.push(title);
                    }
                    if current != element {
                        CFRelease(current as CFTypeRef);
                    }
                    current = parent;
                } else {
                    break;
                }
            }
            if current != element {
                CFRelease(current as CFTypeRef);
            }
        }
        path.reverse();
        path
    }

    fn update_url_time(&mut self, url: String) {
        if let Some((prev_url, start)) = self.current_url_start.take() {
            let duration = start.elapsed();
            *self.url_times.entry(prev_url).or_insert(Duration::ZERO) += duration;
        }
        self.current_url_start = Some((url, Instant::now()));
    }

    fn handle_app_change(&mut self, name: String, bundle: String, pid: i32) {
        // Clean up old observer
        if let Some(old_obs) = self.current_observer.take() {
            unsafe {
                let observer = old_obs as AXObserverRef;
                let source = AXObserverGetRunLoopSource(observer);
                CFRunLoopRemoveSource(
                    CFRunLoop::get_current().as_concrete_TypeRef(),
                    source,
                    kCFRunLoopDefaultMode as CFStringRef
                );
                CFRelease(observer as CFTypeRef);
            }
        }

        let mut new_ctx = self.extract_context(name, bundle, pid);
        
        // Calculate duration for old context
        let old_ctx = self.current_context.take();
        if let Some(ref old) = old_ctx {
            new_ctx.duration_ms = Some(old.started_at.elapsed().as_millis());
        }
        
        // Determine transition details
        let transition = self.determine_transition(&old_ctx, &new_ctx);
        
        let event = Event {
            event_type: "app_switch".to_string(),
            timestamp: self.start_time.elapsed().as_millis(),
            from_context: old_ctx,
            to_context: new_ctx.clone(),
            trigger: "user".to_string(),
            transition_details: transition,
        };

        self.log_event(event);
        self.setup_observer(pid);
        self.current_context = Some(new_ctx);
    }

    fn handle_ui_change(&mut self, notification: &str) {
        // Debounce rapid events
        let now = Instant::now();
        if let Some(last) = self.last_event_time {
            if now.duration_since(last).as_millis() < 50 {
                // Skip events that are too close together (within 50ms)
                return;
            }
        }
        
        // Handle different notification types
        let should_update = match notification {
            // Major context changes - always handle
            "AXFocusedWindowChanged" | "AXMainWindowChanged" | "AXWindowCreated" => true,
            
            // Title changes often indicate tab switches
            "AXTitleChanged" => true,
            
            // UI element changes - handle for browsers and IDEs
            "AXFocusedUIElementChanged" | "AXValueChanged" => {
                if let Some(ctx) = &self.current_context {
                    ctx.bundle_id.contains("browser") || 
                    ctx.bundle_id.contains("chrome") ||
                    ctx.bundle_id.contains("safari") ||
                    ctx.bundle_id.contains("firefox") ||
                    ctx.bundle_id.contains("cursor") ||
                    ctx.bundle_id.contains("code") ||
                    ctx.bundle_id.contains("jetbrains")
                } else {
                    false
                }
            }
            
            // Selection changes in lists/tables (tabs, files)
            "AXSelectedChildrenChanged" | "AXSelectedRowsChanged" => true,
            
            // Application state changes
            "AXApplicationActivated" | "AXApplicationDeactivated" => true,
            
            // Text selection - only for certain apps
            "AXSelectedTextChanged" => {
                if let Some(ctx) = &self.current_context {
                    ctx.bundle_id.contains("terminal") ||
                    ctx.bundle_id.contains("iterm")
                } else {
                    false
                }
            }
            
            _ => false
        };
        
        if should_update {
            self.last_event_time = Some(now);
            
            if let Some(old_ctx) = self.current_context.take() {
                let mut new_ctx = self.extract_context(
                    old_ctx.app_name.clone(),
                    old_ctx.bundle_id.clone(),
                    old_ctx.pid,
                );
                
                new_ctx.duration_ms = Some(old_ctx.started_at.elapsed().as_millis());
                
                // For tab/title changes, always emit even if subtle
                let force_emit = notification == "AXTitleChanged" || 
                                notification == "AXSelectedChildrenChanged" ||
                                notification == "AXSelectedRowsChanged";
                
                // Only emit if there's a meaningful change or forced
                if force_emit || 
                   new_ctx.window_title != old_ctx.window_title || 
                   new_ctx.url != old_ctx.url || 
                   new_ctx.actual_url != old_ctx.actual_url ||
                   new_ctx.active_file != old_ctx.active_file ||
                   new_ctx.document_path != old_ctx.document_path {
                    
                    let transition = self.determine_transition(&Some(old_ctx.clone()), &new_ctx);
                    
                    let event_type = match notification {
                        "AXTitleChanged" => "tab_change",
                        "AXSelectedChildrenChanged" | "AXSelectedRowsChanged" => "selection_change",
                        "AXFocusedUIElementChanged" | "AXValueChanged" => "focus_change",
                        _ => "window_change"
                    };
                    
                    let event = Event {
                        event_type: event_type.to_string(),
                        timestamp: self.start_time.elapsed().as_millis(),
                        from_context: Some(old_ctx),
                        to_context: new_ctx.clone(),
                        trigger: notification.to_string(),
                        transition_details: transition,
                    };
                    
                    self.log_event(event);
                }
                
                self.current_context = Some(new_ctx);
            }
        }
    }
    
    fn determine_transition(&self, from: &Option<Context>, to: &Context) -> Option<String> {
        if let Some(from_ctx) = from {
            let mut details = Vec::new();
            
            // Check what changed
            if from_ctx.app_name != to.app_name {
                details.push(format!("App: {} → {}", from_ctx.app_name, to.app_name));
            }
            
            if from_ctx.url != to.url {
                if let (Some(from_url), Some(to_url)) = (&from_ctx.url, &to.url) {
                    details.push(format!("URL: {} → {}", from_url, to_url));
                }
            }
            
            if from_ctx.active_file != to.active_file {
                if let (Some(from_file), Some(to_file)) = (&from_ctx.active_file, &to.active_file) {
                    details.push(format!("File: {} → {}", from_file, to_file));
                }
            }
            
            if from_ctx.document_path != to.document_path {
                if let (Some(from_path), Some(to_path)) = (&from_ctx.document_path, &to.document_path) {
                    details.push(format!("Path: {} → {}", from_path, to_path));
                }
            }
            
            if !details.is_empty() {
                Some(details.join(", "))
            } else {
                None
            }
        } else {
            None
        }
    }
    
    fn mine_all_attributes(&self, element: AXUIElementRef) -> HashMap<String, String> {
        let mut attrs = HashMap::new();
        unsafe {
            // List of all possible attributes to check
            let attribute_names = [
                "AXRole", "AXRoleDescription", "AXTitle", "AXDescription",
                "AXValue", "AXHelp", "AXURL", "AXDocument", "AXFilename",
                "AXPath", "AXIdentifier", "AXLabel", "AXPlaceholderValue",
                "AXSelectedText", "AXSelectedTextRange", "AXVisibleCharacterRange",
                "AXNumberOfCharacters", "AXSharedTextUIElements", "AXSharedCharacterRange",
                "AXInsertionPointLineNumber", "AXLinkedUIElements", "AXServesAsTitleForUIElements",
                "AXTitleUIElement", "AXMenuItemMarkChar", "AXMenuItemCmdChar",
                "AXMenuItemCmdVirtualKey", "AXMenuItemCmdGlyph", "AXMenuItemCmdModifiers",
                "AXAlternateUIVisible", "AXSubrole", "AXColumnHeaderUIElements",
                "AXRowHeaderUIElements", "AXContents", "AXHeader", "AXIndex",
                "AXRowIndexRange", "AXColumnIndexRange", "AXHorizontalScrollBar",
                "AXVerticalScrollBar", "AXSortDirection", "AXDisclosureLevel",
                "AXAccessKey", "AXRowCount", "AXColumnCount", "AXOrderedByRow",
                "AXWarningValue", "AXCriticalValue", "AXSelectedCells", "AXVisibleCells",
                "AXRowHeaderUIElements", "AXColumnHeaderUIElements",
            ];
            
            for attr_name in &attribute_names {
                if let Some(value) = self.get_string_attr(element, attr_name) {
                    attrs.insert(attr_name.to_string(), value);
                }
            }
        }
        attrs
    }
    
    fn deep_mine_for_url(&self, element: AXUIElementRef) -> Option<String> {
        unsafe {
            // Try to find URL by traversing the entire tree
            let mut url: Option<String> = None;
            
            // Check this element
            url = url.or_else(|| self.get_string_attr(element, "AXURL"));
            url = url.or_else(|| self.get_string_attr(element, "AXDocument"));
            
            if url.is_some() {
                return url;
            }
            
            // Get all children and search recursively (limited depth)
            if let Some(children_ref) = self.get_attribute(element, "AXChildren") {
                // We need to handle CFArray properly here
                // For now, try to find specific child types
                CFRelease(children_ref);
            }
            
            // Try to find AXWebArea, AXScrollArea, AXGroup children
            for child_type in &["AXWebArea", "AXScrollArea", "AXGroup", "AXTextField", "AXStaticText"] {
                if let Some(child) = self.find_child_by_role(element, child_type) {
                    url = url.or_else(|| self.get_string_attr(child, "AXURL"));
                    url = url.or_else(|| self.get_string_attr(child, "AXDocument"));
                    url = url.or_else(|| self.get_string_attr(child, "AXValue"));
                    
                    // Check if value contains a URL
                    if let Some(value) = self.get_string_attr(child, "AXValue") {
                        if value.starts_with("http://") || value.starts_with("https://") {
                            url = Some(value);
                        }
                    }
                    
                    CFRelease(child as CFTypeRef);
                    if url.is_some() {
                        return url;
                    }
                }
            }
            
            url
        }
    }
    
    fn find_child_by_role(&self, parent: AXUIElementRef, role: &str) -> Option<AXUIElementRef> {
        unsafe {
            // This is a simplified search - in production you'd properly iterate CFArray
            if let Some(children_ref) = self.get_attribute(parent, "AXChildren") {
                // Would need proper CFArray iteration here
                CFRelease(children_ref);
            }
            None
        }
    }
    
    fn scan_all_windows_for_url(&self, app_element: AXUIElementRef) -> Option<String> {
        unsafe {
            // Get all windows
            if let Some(windows_ref) = self.get_attribute(app_element, "AXWindows") {
                // Would iterate through all windows here
                CFRelease(windows_ref);
            }
            
            // Try main window
            if let Some(main_window) = self.get_attribute(app_element, "AXMainWindow") {
                let url = self.deep_mine_for_url(main_window as AXUIElementRef);
                CFRelease(main_window);
                if url.is_some() {
                    return url;
                }
            }
            
            None
        }
    }
    
    fn construct_full_path(&self, filename: &str, project_name: &str) -> Option<String> {
        // Common project locations
        let possible_roots = vec![
            format!("/Users/{}/Development/{}", std::env::var("USER").unwrap_or_default(), project_name),
            format!("/Users/{}/Projects/{}", std::env::var("USER").unwrap_or_default(), project_name),
            format!("/Users/{}/Documents/{}", std::env::var("USER").unwrap_or_default(), project_name),
            format!("/Users/{}/Desktop/{}", std::env::var("USER").unwrap_or_default(), project_name),
            format!("/Users/{}/Code/{}", std::env::var("USER").unwrap_or_default(), project_name),
            format!("/Users/{}/dev/{}", std::env::var("USER").unwrap_or_default(), project_name),
            format!("/Users/{}/src/{}", std::env::var("USER").unwrap_or_default(), project_name),
            format!("/Users/{}/Development/GitHub/{}", std::env::var("USER").unwrap_or_default(), project_name),
            format!("/Users/{}/Development/GitHub/open-runtime/{}", std::env::var("USER").unwrap_or_default(), project_name),
        ];
        
        // Check if any of these directories exist and contain the file
        for root in possible_roots {
            let full_path = format!("{}/{}", root, filename);
            if std::path::Path::new(&full_path).exists() {
                return Some(full_path);
            }
            
            // Also check common subdirectories
            for subdir in &["src", "lib", "app", "pages", "components", "test", "tests"] {
                let path_with_subdir = format!("{}/{}/{}", root, subdir, filename);
                if std::path::Path::new(&path_with_subdir).exists() {
                    return Some(path_with_subdir);
                }
            }
        }
        
        None
    }

    fn log_event(&self, event: Event) {
        if self.json_output {
            println!("{}", serde_json::to_string(&event).unwrap());
        } else {
            let ctx = &event.to_context;
            println!("\n[{}] at {}ms | trigger: {}", 
                event.event_type,
                event.timestamp,
                event.trigger
            );
            println!("════════════════════════════════════════════════════════");
            
            // Show FROM context if app switch
            if let Some(from) = &event.from_context {
                println!("FROM:");
                println!("  App: {} ({})", from.app_name, from.bundle_id);
                if let Some(url) = &from.url {
                    println!("  URL: {}", url);
                }
                if let Some(file) = &from.active_file {
                    println!("  File: {}", file);
                }
                if let Some(path) = &from.document_path {
                    println!("  Path: {}", path);
                }
                if let Some(duration) = ctx.duration_ms {
                    println!("  Time spent: {}ms", duration);
                }
                println!("TO:");
            }
            
            // Application info
            println!("App: {} ({})", ctx.app_name, ctx.bundle_id);
            println!("PID: {} | Path: {}", ctx.pid, ctx.app_path.as_deref().unwrap_or("unknown"));
            
            // Window info
            if let Some(window) = &ctx.window_title {
                println!("Window: {}", window);
            }
            
            // All specific context info - ONLY show what's relevant
            if let Some(url) = &ctx.url {
                println!("🌐 URL: {}", url);
            } else if let Some(actual_url) = &ctx.actual_url {
                println!("📍 URL: {}", actual_url);
            }
            
            if let Some(file) = &ctx.active_file {
                println!("📄 Active File: {}", file);
                if let Some(proj) = &ctx.project {
                    println!("   Project: {}", proj);
                }
            }
            
            if let Some(path) = &ctx.finder_path {
                println!("📂 Finder: {}", path);
                if !ctx.selected_files.is_empty() && ctx.selected_files.len() <= 3 {
                    println!("   Selected: {}", ctx.selected_files.join(", "));
                } else if ctx.selected_files.len() > 3 {
                    println!("   Selected: {} items", ctx.selected_files.len());
                }
            }
            
            if let Some(tab) = &ctx.terminal_tab {
                println!("💻 Terminal: {}", tab);
                if let Some(cwd) = &ctx.terminal_cwd {
                    println!("   CWD: {}", cwd);
                }
            }
            
            // Only show clipboard if there's new content
            if let Some(clipboard) = &ctx.clipboard_text {
                if let Some(ctype) = &ctx.clipboard_type {
                    let preview = if clipboard.len() > 50 {
                        format!("{}...", &clipboard[..50])
                    } else {
                        clipboard.clone()
                    };
                    println!("📋 Clipboard ({}): {}", ctype, preview);
                }
            }
            
            // Show network connections only if meaningful
            if !ctx.active_connections.is_empty() && ctx.active_connections.len() <= 5 {
                println!("🌐 Connected to: {}", ctx.active_connections.join(", "));
            } else if ctx.active_connections.len() > 5 {
                println!("🌐 Connected to: {} services", ctx.active_connections.len());
            }
            
            // Only show focused element if it has meaningful content
            if let Some(elem) = &ctx.focused_element {
                // Only show if there's selected text or a meaningful value
                if let Some(text) = &elem.selected_text {
                    if !text.trim().is_empty() {
                        println!("✏️  Selected: {}", text);
                    }
                } else if let Some(value) = &elem.value {
                    // Only show value if it's meaningful (not empty, not just whitespace)
                    if !value.trim().is_empty() && value.len() < 100 {
                        if let Some(role) = &elem.role {
                            if role == "AXTextField" || role == "AXTextArea" {
                                println!("✏️  Typing: {}", value.trim());
                            }
                        }
                    }
                }
            }
            
            // Transition details
            if let Some(details) = &event.transition_details {
                println!("\nTransition: {}", details);
            }
            
            println!("────────────────────────────────────────────────────────");
        }
    }

    fn setup_observer(&mut self, pid: i32) {
        unsafe {
            let mut observer: AXObserverRef = null_mut();
            
            if AXObserverCreate(pid, ax_callback, &mut observer) == kAXErrorSuccess {
                let app = AXUIElementCreateApplication(pid);
                
                // Only track major window changes, not UI element changes
                // Comment out fine-grained UI tracking to focus on app switches
                let notifications = [
                    "AXFocusedWindowChanged",
                    "AXMainWindowChanged",
                    "AXWindowCreated",
                    "AXTitleChanged",
                    "AXFocusedUIElementChanged",
                    "AXValueChanged",
                    "AXSelectedChildrenChanged",
                    "AXSelectedTextChanged",
                    "AXMenuItemSelected",
                    "AXSelectedRowsChanged",
                    "AXRowCountChanged",
                    "AXApplicationActivated",
                    "AXApplicationDeactivated",
                    "AXApplicationShown",
                    "AXApplicationHidden",
                    "AXWindowMiniaturized",
                    "AXWindowDeminiaturized",
                    "AXWindowMoved",
                    "AXWindowResized",
                ];
                
                for notif in &notifications {
                    let cfstr = CFString::new(notif);
                    AXObserverAddNotification(
                        observer,
                        app,
                        cfstr.as_concrete_TypeRef() as CFStringRef,
                        null_mut()
                    );
                }
                
                let source = AXObserverGetRunLoopSource(observer);
                CFRunLoopAddSource(
                    CFRunLoop::get_current().as_concrete_TypeRef(),
                    source,
                    kCFRunLoopDefaultMode as CFStringRef
                );
                
                self.current_observer = Some(observer as usize);
                CFRelease(app as CFTypeRef);
            }
        }
    }

    fn get_attribute(&self, element: AXUIElementRef, attr: &str) -> Option<CFTypeRef> {
        unsafe {
            let mut value: CFTypeRef = null_mut();
            let cfstr = CFString::new(attr);
            
            if AXUIElementCopyAttributeValue(
                element,
                cfstr.as_concrete_TypeRef() as CFStringRef,
                &mut value,
            ) == kAXErrorSuccess && !value.is_null() {
                Some(value)
            } else {
                None
            }
        }
    }

    fn get_string_attr(&self, element: AXUIElementRef, attr: &str) -> Option<String> {
        unsafe {
            self.get_attribute(element, attr).and_then(|value| {
                // Check if it's actually a string type
                let type_id = CFGetTypeID(value);
                let string_type_id = CFStringGetTypeID();
                
                if type_id == string_type_id {
                    let cfstr = CFString::wrap_under_create_rule(value as CFStringRef);
                    let result = cfstr.to_string();
                    if !result.is_empty() {
                        Some(result)
                    } else {
                        None
                    }
                } else {
                    // Not a string, release it and return None
                    CFRelease(value);
                    None
                }
            })
        }
    }

    fn get_bool_attr(&self, element: AXUIElementRef, attr: &str) -> Option<bool> {
        unsafe {
            self.get_attribute(element, attr).map(|value| {
                let ptr = value as *const _ as *const u8;
                !ptr.is_null() && *ptr != 0
            })
        }
    }

    fn get_clipboard_change_count(&self) -> i64 {
        unsafe {
            let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            msg_send![pasteboard, changeCount]
        }
    }
    
    fn setup_keyboard_tap(&mut self) {
        unsafe {
            // Create event tap for keyboard events
            let mask = (1u64 << CGEventType::KeyDown as u32);
            
            let tap = CGEventTapCreate(
                CGEventTapLocation::Session,
                kCGHeadInsertEventTap,
                kCGEventTapOptionDefault,
                mask,
                keyboard_event_callback,
                null_mut(),
            );
            
            if !tap.is_null() {
                let source = CFMachPortCreateRunLoopSource(null_mut(), tap, 0);
                if !source.is_null() {
                    let run_loop = CFRunLoop::get_current();
                    CFRunLoopAddSource(
                        run_loop.as_concrete_TypeRef(),
                        source as *mut _,
                        kCFRunLoopDefaultMode,
                    );
                    CGEventTapEnable(tap, true);
                    
                    KEYBOARD_TAP.store(tap, Ordering::SeqCst);
                    KEYBOARD_TAP_SOURCE.store(source, Ordering::SeqCst);
                    
                    if !self.json_output {
                        println!("⌨️  Keyboard shortcut monitoring enabled (Cmd+C/V/X)");
                    }
                }
            }
        }
    }
    
    fn setup_scroll_tap(&mut self) {
        unsafe {
            // Create event tap for scroll events
            let mask = (1u64 << CGEventType::ScrollWheel as u32);
            
            let tap = CGEventTapCreate(
                CGEventTapLocation::Session,
                kCGHeadInsertEventTap,
                kCGEventTapOptionDefault,
                mask,
                scroll_event_callback,
                null_mut(),
            );
            
            if !tap.is_null() {
                let source = CFMachPortCreateRunLoopSource(null_mut(), tap, 0);
                if !source.is_null() {
                    let run_loop = CFRunLoop::get_current();
                    CFRunLoopAddSource(
                        run_loop.as_concrete_TypeRef(),
                        source as *mut _,
                        kCFRunLoopDefaultMode,
                    );
                    CGEventTapEnable(tap, true);
                    
                    SCROLL_TAP.store(tap, Ordering::SeqCst);
                    SCROLL_TAP_SOURCE.store(source, Ordering::SeqCst);
                    
                    if !self.json_output {
                        println!("📜 Scroll event monitoring enabled");
                    }
                }
            }
        }
    }
    
    fn handle_clipboard_shortcut_with_position(&mut self, action: &str, position: CGPoint) {
        // Update the clipboard event with the keyboard shortcut action and mouse position
        let context = self.current_context.clone();
        
        match action {
            "copy" | "cut" => {
                // Mark this as the copy source
                self.last_copy_context = context.clone();
                
                // Schedule delayed clipboard read (50ms) to ensure clipboard is updated
                let state_clone = STATE.get().unwrap().clone();
                let action_clone = action.to_string();
                let context_clone = context.clone();
                let position_clone = position;
                
                thread::spawn(move || {
                    // Wait for clipboard to update
                    thread::sleep(Duration::from_millis(50));
                    
                    // Now read the clipboard after delay
                    let mut tracker = state_clone.lock().unwrap();
                    
                    // Get file paths first
                    let file_paths = tracker.get_clipboard_file_paths();
                    let mut content = tracker.get_clipboard_text();
                    
                    // If files were copied and no text content, try to read file contents
                    if content.is_none() && !file_paths.is_empty() {
                        // Single text file - read its contents
                        if file_paths.len() == 1 {
                            let path = &file_paths[0];
                            if is_text_file(path) {
                                if let Some(file_content) = read_file_contents_safe(path, 50_000) {
                                    // 50KB limit for clipboard display
                                    content = Some(file_content);
                                }
                            }
                        } else {
                            // Multiple files - just show count
                            content = Some(format!("{} files copied", file_paths.len()));
                        }
                    }
                    
                    // Create a clipboard event with updated content
                    let event = ClipboardEvent {
                        timestamp: tracker.start_time.elapsed().as_millis(),
                        event_type: action_clone.clone(),
                        content,
                        content_type: tracker.get_clipboard_type(),
                        source_app: context_clone.as_ref().map(|c| c.app_name.clone()).unwrap_or_default(),
                        source_bundle_id: context_clone.as_ref().map(|c| c.bundle_id.clone()).unwrap_or_default(),
                        source_window: context_clone.as_ref().and_then(|c| c.window_title.clone()),
                        source_url: context_clone.as_ref().and_then(|c| c.url.clone()),
                        source_file: context_clone.as_ref().and_then(|c| c.active_file.clone()),
                        file_paths: file_paths.clone(),
                        metadata: {
                            let mut meta = tracker.get_clipboard_metadata();
                            meta.insert("mouse_x".to_string(), format!("{:.1}", position_clone.x));
                            meta.insert("mouse_y".to_string(), format!("{:.1}", position_clone.y));
                            meta.insert("delayed_read".to_string(), "true".to_string());
                            if !file_paths.is_empty() {
                                meta.insert("file_count".to_string(), file_paths.len().to_string());
                                if file_paths.len() == 1 && is_text_file(&file_paths[0]) {
                                    meta.insert("file_read".to_string(), "true".to_string());
                                }
                            }
                            meta
                        },
                        all_formats: Some(tracker.get_all_clipboard_content()),
                    };
                    
                    tracker.clipboard_events.push(event.clone());
                    
                    if !tracker.json_output {
                        println!("\n⌨️  KEYBOARD SHORTCUT (delayed): Cmd+{} ({}) at ({:.0}, {:.0})", 
                            if action_clone == "copy" { "C" } else { "X" }, 
                            action_clone.to_uppercase(),
                            position_clone.x,
                            position_clone.y
                        );
                        
                        if let Some(content) = &event.content {
                            let preview = if content.len() > 50 {
                                format!("{}...", &content[..50])
                            } else {
                                content.clone()
                            };
                            println!("   Content: {}", preview);
                        }
                    }
                });
                
                // Immediate feedback (before clipboard is ready)
                if !self.json_output {
                    println!("\n⌨️  KEYBOARD SHORTCUT: Cmd+{} ({}) detected at ({:.0}, {:.0})", 
                        if action == "copy" { "C" } else { "X" }, 
                        action.to_uppercase(),
                        position.x,
                        position.y
                    );
                    println!("   Source: {}", 
                        context.as_ref().map(|c| c.app_name.clone()).unwrap_or("Unknown".to_string())
                    );
                }
            }
            "paste" => {
                // Track paste destination
                self.last_paste_context = context.clone();
                
                if !self.json_output {
                    println!("\n⌨️  KEYBOARD SHORTCUT: Cmd+V (PASTE) at ({:.0}, {:.0})", position.x, position.y);
                    
                    // Show copy→paste flow
                    if let (Some(from_ctx), Some(to_ctx)) = (&self.last_copy_context, &context) {
                        println!("   📋 Flow: {} → {}", 
                            from_ctx.app_name, 
                            to_ctx.app_name
                        );
                        
                        if let Some(content) = self.get_clipboard_text() {
                            let preview = if content.len() > 50 {
                                format!("{}...", &content[..50])
                            } else {
                                content.clone()
                            };
                            println!("   Content: {}", preview);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    
    fn handle_clipboard_shortcut(&mut self, action: &str) {
        // Update the clipboard event with the keyboard shortcut action
        let context = self.current_context.clone();
        
        match action {
            "copy" | "cut" => {
                // Mark this as the copy source
                self.last_copy_context = context.clone();
                
                // Create a clipboard event
                let event = ClipboardEvent {
                    timestamp: self.start_time.elapsed().as_millis(),
                    event_type: action.to_string(),
                    content: self.get_clipboard_text(),
                    content_type: self.get_clipboard_type(),
                    source_app: context.as_ref().map(|c| c.app_name.clone()).unwrap_or_default(),
                    source_bundle_id: context.as_ref().map(|c| c.bundle_id.clone()).unwrap_or_default(),
                    source_window: context.as_ref().and_then(|c| c.window_title.clone()),
                    source_url: context.as_ref().and_then(|c| c.url.clone()),
                    source_file: context.as_ref().and_then(|c| c.active_file.clone()),
                    file_paths: self.get_clipboard_file_paths(),
                    metadata: self.get_clipboard_metadata(),
                    all_formats: Some(self.get_all_clipboard_content()),
                };
                
                self.clipboard_events.push(event.clone());
                
                if !self.json_output {
                    println!("\n⌨️  KEYBOARD SHORTCUT: Cmd+{} ({})", 
                        if action == "copy" { "C" } else { "X" }, 
                        action.to_uppercase()
                    );
                    println!("   Source: {}", 
                        context.as_ref().map(|c| c.app_name.clone()).unwrap_or("Unknown".to_string())
                    );
                }
            }
            "paste" => {
                // Track paste destination
                self.last_paste_context = context.clone();
                
                if !self.json_output {
                    println!("\n⌨️  KEYBOARD SHORTCUT: Cmd+V (PASTE)");
                    
                    // Show copy→paste flow
                    if let (Some(from_ctx), Some(to_ctx)) = (&self.last_copy_context, &context) {
                        println!("   📋 Flow: {} → {}", 
                            from_ctx.app_name, 
                            to_ctx.app_name
                        );
                        
                        if let Some(content) = self.get_clipboard_text() {
                            let preview = if content.len() > 50 {
                                format!("{}...", &content[..50])
                            } else {
                                content.clone()
                            };
                            println!("   Content: {}", preview);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    
    fn handle_scroll_event_with_position(&mut self, delta_x: f64, delta_y: f64, position: CGPoint) {
        if !self.json_output {
            println!("📜 Scroll detected: Δx={:.1}, Δy={:.1} at ({:.0}, {:.0})", 
                delta_x, delta_y, position.x, position.y);
        }
        
        // Could extract visible content here if needed
        // For now, track scrolling with position
    }
    
    fn handle_scroll_event(&mut self, delta_x: f64, delta_y: f64) {
        if !self.json_output {
            println!("📜 Scroll detected: Δx={:.1}, Δy={:.1}", delta_x, delta_y);
        }
        
        // You could extract visible content here if needed
        // For now, just track that scrolling happened
    }
    
    fn start_clipboard_monitor(&mut self) {
        if !self.clipboard_monitor {
            return;
        }
        
        // Start 100ms polling thread for clipboard
        let state_clone = STATE.get().unwrap().clone();
        
        self.clipboard_thread_handle = Some(thread::spawn(move || {
            let mut last_change_count: i64 = 0;
            let mut last_content: Option<String> = None;
            
            loop {
                thread::sleep(Duration::from_millis(100)); // Poll every 100ms
                
                // Get current clipboard state
                let (current_change_count, current_content, current_context) = {
                    let tracker = state_clone.lock().unwrap();
                    (
                        tracker.get_clipboard_change_count(),
                        tracker.get_clipboard_text(),
                        tracker.current_context.clone(),
                    )
                };
                
                // Check if clipboard changed
                if current_change_count != last_change_count {
                    // Clipboard has changed!
                    let mut tracker = state_clone.lock().unwrap();
                    
                    // Get all clipboard formats
                    let all_formats = tracker.get_all_clipboard_content();
                    let file_paths = tracker.get_clipboard_file_paths();
                    
                    // If no text content but files were copied, try to read file contents
                    let mut enhanced_content = current_content.clone();
                    if enhanced_content.is_none() && !file_paths.is_empty() {
                        if file_paths.len() == 1 {
                            let path = &file_paths[0];
                            if is_text_file(path) {
                                if let Some(file_content) = read_file_contents_safe(path, 50_000) {
                                    enhanced_content = Some(file_content);
                                }
                            }
                        }
                    }
                    
                    // Create clipboard event with full metadata
                    let event = ClipboardEvent {
                        timestamp: tracker.start_time.elapsed().as_millis(),
                        event_type: "copy".to_string(), // We'll enhance this with keyboard detection
                        content: enhanced_content,
                        content_type: tracker.get_clipboard_type(),
                        source_app: current_context.as_ref().map(|c| c.app_name.clone()).unwrap_or_default(),
                        source_bundle_id: current_context.as_ref().map(|c| c.bundle_id.clone()).unwrap_or_default(),
                        source_window: current_context.as_ref().and_then(|c| c.window_title.clone()),
                        source_url: current_context.as_ref().and_then(|c| c.url.clone()),
                        source_file: current_context.as_ref().and_then(|c| c.active_file.clone()),
                        file_paths: file_paths.clone(),
                        metadata: tracker.get_clipboard_metadata(),
                        all_formats: Some(all_formats),
                    };
                    
                    // Store the copy context
                    tracker.last_copy_context = current_context.clone();
                    tracker.clipboard_events.push(event.clone());
                    
                    // Log the clipboard event
                    if tracker.json_output {
                        println!("{}", serde_json::to_string(&event).unwrap());
                    } else {
                        println!("\n📋 CLIPBOARD CHANGE DETECTED");
                        println!("   Type: {}", event.event_type);
                        
                        // Show content based on type
                        if !event.file_paths.is_empty() {
                            println!("   📁 FILES COPIED FROM FINDER:");
                            for path in &event.file_paths {
                                println!("      - {}", path);
                            }
                        } else if let Some(content) = &event.content {
                            let preview = if content.len() > 100 {
                                format!("{}...", &content[..100])
                            } else {
                                content.clone()
                            };
                            println!("   Content: {}", preview);
                        }
                        
                        // Show all available formats
                        if let Some(all_formats) = &event.all_formats {
                            if all_formats.contains_key("image") {
                                if let Some(image_info) = all_formats.get("image") {
                                    println!("   🖼️  IMAGE DATA: {}", image_info);
                                }
                            }
                            if all_formats.contains_key("html") {
                                println!("   📄 HTML content available");
                            }
                            if all_formats.contains_key("rtf_base64") {
                                println!("   📝 Rich Text (RTF) content available");
                            }
                        }
                        
                        println!("   Source: {} ({})", event.source_app, event.source_bundle_id);
                        if let Some(window) = &event.source_window {
                            println!("   Window: {}", window);
                        }
                        if let Some(url) = &event.source_url {
                            println!("   URL: {}", url);
                        }
                        if let Some(file) = &event.source_file {
                            println!("   File: {}", file);
                        }
                        
                        // Show available types
                        if !event.metadata.is_empty() {
                            if let Some(types) = event.metadata.get("available_types") {
                                println!("   Available formats: {}", types);
                            }
                        }
                    }
                    
                    // Update tracking
                    last_change_count = current_change_count;
                    last_content = current_content;
                }
            }
        }));
    }
    
    fn get_clipboard_file_paths(&self) -> Vec<String> {
        unsafe {
            let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            let mut paths = Vec::new();
            
            // Try NSFilenamesPboardType first (Finder file copy)
            let filenames_type: id = msg_send![class!(NSString), stringWithUTF8String: "NSFilenamesPboardType".as_ptr()];
            let filenames: id = msg_send![pasteboard, propertyListForType: filenames_type];
            
            if filenames != nil {
                let count: usize = msg_send![filenames, count];
                for i in 0..count {
                    let filename: id = msg_send![filenames, objectAtIndex: i];
                    if filename != nil {
                        let c_str: *const i8 = msg_send![filename, UTF8String];
                        paths.push(CStr::from_ptr(c_str).to_string_lossy().to_string());
                    }
                }
            }
            
            // Also check for file URLs (public.file-url)
            if paths.is_empty() {
                let file_url_type: id = msg_send![class!(NSString), stringWithUTF8String: "public.file-url".as_ptr()];
                let urls: id = msg_send![pasteboard, propertyListForType: file_url_type];
                
                if urls != nil {
                    let count: usize = msg_send![urls, count];
                    for i in 0..count {
                        let url: id = msg_send![urls, objectAtIndex: i];
                        if url != nil {
                            let path_str: id = msg_send![url, path];
                            if path_str != nil {
                                let c_str: *const i8 = msg_send![path_str, UTF8String];
                                paths.push(CStr::from_ptr(c_str).to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
            
            paths
        }
    }
    
    fn get_clipboard_metadata(&self) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        
        unsafe {
            let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            
            // Get available types
            let types: id = msg_send![pasteboard, types];
            if types != nil {
                let count: usize = msg_send![types, count];
                metadata.insert("type_count".to_string(), count.to_string());
                
                // List all available types
                let mut type_list = Vec::new();
                for i in 0..count.min(10) { // Limit to 10 types
                    let type_str: id = msg_send![types, objectAtIndex: i];
                    if type_str != nil {
                        let c_str: *const i8 = msg_send![type_str, UTF8String];
                        type_list.push(CStr::from_ptr(c_str).to_string_lossy().to_string());
                    }
                }
                metadata.insert("available_types".to_string(), type_list.join(", "));
            }
            
            // Get change count
            let change_count: i64 = msg_send![pasteboard, changeCount];
            metadata.insert("change_count".to_string(), change_count.to_string());
        }
        
        metadata
    }
    
    fn get_clipboard_text(&self) -> Option<String> {
        // Try NSPasteboard first
        let ns_result = self.get_clipboard_text_nspasteboard();
        
        // Try arboard as fallback/verification
        let arboard_result = self.get_clipboard_text_arboard();
        
        // If both have content, prefer the one with more data
        match (ns_result, arboard_result) {
            (Some(ns), Some(ar)) => {
                // Return whichever has more content (likely more complete)
                if ns.len() >= ar.len() {
                    Some(ns)
                } else {
                    if !self.json_output && ns != ar {
                        println!("   [Clipboard Verification: arboard has different/better content]");
                    }
                    Some(ar)
                }
            }
            (Some(ns), None) => Some(ns),
            (None, Some(ar)) => {
                if !self.json_output {
                    println!("   [Clipboard: Using arboard fallback]");
                }
                Some(ar)
            }
            (None, None) => None,
        }
    }
    
    fn get_clipboard_text_nspasteboard(&self) -> Option<String> {
        unsafe {
            let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            
            // First try the simple approach
            let string: id = msg_send![pasteboard, stringForType: nil];
            if string != nil {
                let c_str: *const i8 = msg_send![string, UTF8String];
                if !c_str.is_null() {
                    return Some(CStr::from_ptr(c_str).to_string_lossy().to_string());
                }
            }
            
            // Try with specific types using NSPasteboardType constants
            let ns_string = class!(NSString);
            
            // Try NSStringPboardType constant
            let string_type: id = msg_send![ns_string, 
                stringWithCString: "NSStringPboardType\0".as_ptr() 
                encoding: 4]; // NSUTF8StringEncoding = 4
            
            if string_type != nil {
                let string: id = msg_send![pasteboard, stringForType: string_type];
                if string != nil {
                    let c_str: *const i8 = msg_send![string, UTF8String];
                    if !c_str.is_null() {
                        return Some(CStr::from_ptr(c_str).to_string_lossy().to_string());
                    }
                }
            }
            
            // Try public.utf8-plain-text
            let utf8_type: id = msg_send![ns_string,
                stringWithCString: "public.utf8-plain-text\0".as_ptr()
                encoding: 4];
            
            if utf8_type != nil {
                let string: id = msg_send![pasteboard, stringForType: utf8_type];
                if string != nil {
                    let c_str: *const i8 = msg_send![string, UTF8String];
                    if !c_str.is_null() {
                        return Some(CStr::from_ptr(c_str).to_string_lossy().to_string());
                    }
                }
            }
            
            None
        }
    }
    
    fn get_clipboard_text_arboard(&self) -> Option<String> {
        // Use arboard for robust clipboard access
        match Clipboard::new() {
            Ok(mut clipboard) => {
                match clipboard.get_text() {
                    Ok(text) => Some(text),
                    Err(_) => None,
                }
            }
            Err(_) => None,
        }
    }
    
    fn get_clipboard_html(&self) -> Option<String> {
        unsafe {
            let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            let html_type: id = msg_send![class!(NSString), stringWithUTF8String: "public.html".as_ptr()];
            let html: id = msg_send![pasteboard, stringForType: html_type];
            
            if html != nil {
                let c_str: *const i8 = msg_send![html, UTF8String];
                Some(CStr::from_ptr(c_str).to_string_lossy().to_string())
            } else {
                None
            }
        }
    }
    
    fn get_clipboard_rtf(&self) -> Option<Vec<u8>> {
        unsafe {
            let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            let rtf_type: id = msg_send![class!(NSString), stringWithUTF8String: "public.rtf".as_ptr()];
            let rtf_data: id = msg_send![pasteboard, dataForType: rtf_type];
            
            if rtf_data != nil {
                let length: usize = msg_send![rtf_data, length];
                let bytes: *const u8 = msg_send![rtf_data, bytes];
                let mut vec = Vec::with_capacity(length);
                vec.extend_from_slice(std::slice::from_raw_parts(bytes, length));
                Some(vec)
            } else {
                None
            }
        }
    }
    
    fn get_clipboard_image_info(&self) -> Option<HashMap<String, String>> {
        unsafe {
            let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            let mut info = HashMap::new();
            
            // Check for various image types
            let image_types = [
                ("public.png", "PNG"),
                ("public.jpeg", "JPEG"),
                ("public.tiff", "TIFF"),
                ("com.apple.pict", "PICT"),
                ("public.pdf", "PDF")
            ];
            
            for (type_str, name) in &image_types {
                let image_type: id = msg_send![class!(NSString), stringWithUTF8String: type_str.as_ptr()];
                let image_data: id = msg_send![pasteboard, dataForType: image_type];
                
                if image_data != nil {
                    let length: usize = msg_send![image_data, length];
                    info.insert("format".to_string(), name.to_string());
                    info.insert("size_bytes".to_string(), length.to_string());
                    return Some(info);
                }
            }
            
            None
        }
    }
    
    fn get_all_clipboard_content(&self) -> HashMap<String, serde_json::Value> {
        let mut content = HashMap::new();
        
        // Get text content
        if let Some(text) = self.get_clipboard_text() {
            content.insert("text".to_string(), serde_json::Value::String(text));
        }
        
        // Get HTML content
        if let Some(html) = self.get_clipboard_html() {
            content.insert("html".to_string(), serde_json::Value::String(html));
        }
        
        // Get RTF content (as base64)
        if let Some(rtf) = self.get_clipboard_rtf() {
            use base64::{Engine as _, engine::general_purpose};
            let encoded = general_purpose::STANDARD.encode(rtf);
            content.insert("rtf_base64".to_string(), serde_json::Value::String(encoded));
        }
        
        // Get file paths
        let file_paths = self.get_clipboard_file_paths();
        if !file_paths.is_empty() {
            content.insert("file_paths".to_string(), serde_json::json!(file_paths));
        }
        
        // Get image info
        if let Some(image_info) = self.get_clipboard_image_info() {
            content.insert("image".to_string(), serde_json::json!(image_info));
        }
        
        // Get metadata
        let metadata = self.get_clipboard_metadata();
        if !metadata.is_empty() {
            content.insert("metadata".to_string(), serde_json::json!(metadata));
        }
        
        content
    }
    
    fn get_clipboard_type(&self) -> Option<String> {
        unsafe {
            let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            let types: id = msg_send![pasteboard, types];
            
            if types != nil {
                let string_type: id = msg_send![class!(NSString), stringWithUTF8String: "public.utf8-plain-text".as_ptr()];
                let has_string: bool = msg_send![types, containsObject: string_type];
                
                if has_string {
                    return Some("text".to_string());
                }
                
                let file_type: id = msg_send![class!(NSString), stringWithUTF8String: "public.file-url".as_ptr()];
                let has_file: bool = msg_send![types, containsObject: file_type];
                
                if has_file {
                    return Some("files".to_string());
                }
                
                Some("unknown".to_string())
            } else {
                None
            }
        }
    }
    
    fn get_network_connections(&self, pid: i32) -> Vec<String> {
        use std::process::Command;
        use std::collections::HashSet;
        
        let output = Command::new("lsof")
            .args(&["-i", "-n", "-P", "-p", &pid.to_string()])
            .output()
            .ok();
        
        let mut connections = Vec::new();
        let mut domains = HashSet::new();
        
        if let Some(output) = output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines().skip(1) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 9 {
                        if let Some(conn) = parts.get(8) {
                            if conn.contains("->") {
                                let conn_parts: Vec<&str> = conn.split("->").collect();
                                if conn_parts.len() == 2 {
                                    let dest = conn_parts[1];
                                    
                                    // Filter out localhost/internal connections
                                    if !dest.starts_with("127.0.0.1") && 
                                       !dest.starts_with("::1") &&
                                       !dest.starts_with("localhost") {
                                        
                                        // Extract domain/IP and port
                                        let dest_parts: Vec<&str> = dest.split(':').collect();
                                        if dest_parts.len() == 2 {
                                            let host = dest_parts[0];
                                            let port = dest_parts[1];
                                            
                                            // Group by domain for common services
                                            let domain = if host.contains('.') {
                                                // Try to identify service by IP/domain
                                                if host.contains("1e100.net") || host.contains("google") {
                                                    "Google"
                                                } else if host.contains("amazonaws") || host.starts_with("52.") || host.starts_with("54.") {
                                                    "AWS"
                                                } else if host.contains("cloudflare") || host.starts_with("104.") {
                                                    "Cloudflare"
                                                } else if host.contains("github") || host.contains("140.82") {
                                                    "GitHub"
                                                } else if host.contains("anthropic") || host.contains("160.79") {
                                                    "Anthropic"
                                                } else {
                                                    // Use first two parts of IP or domain
                                                    let parts: Vec<&str> = host.split('.').collect();
                                                    if parts.len() >= 2 {
                                                        &host[..host.rfind('.').unwrap_or(host.len())]
                                                    } else {
                                                        host
                                                    }
                                                }
                                            } else {
                                                host
                                            };
                                            
                                            // Only add if we haven't seen this domain yet
                                            if domains.insert(domain.to_string()) {
                                                connections.push(format!("{}:{}", domain, port));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // Limit to 10 most relevant connections
        connections.truncate(10);
        connections
    }
    
    fn get_app_path(&self, bundle_id: &str) -> Option<String> {
        unsafe {
            let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
            let ns_str: id = msg_send![class!(NSString), alloc];
            let ns_str: id = msg_send![ns_str, initWithBytes:bundle_id.as_ptr() 
                                       length:bundle_id.len() 
                                       encoding:4];
            
            let url: id = msg_send![workspace, URLForApplicationWithBundleIdentifier:ns_str];
            let _: () = msg_send![ns_str, release];
            
            if url != nil {
                let path: id = msg_send![url, path];
                if path != nil {
                    let c_str: *const i8 = msg_send![path, UTF8String];
                    Some(CStr::from_ptr(c_str).to_string_lossy().to_string())
                } else {
                    None
                }
            } else {
                None
            }
        }
    }
    
    fn get_selected_finder_items(&self, selected_children_ref: CFTypeRef) -> Vec<String> {
        unsafe {
            let mut items = Vec::new();
            
            // Cast to CFArray and iterate
            // Selected children should be an array
            let cfarray = core_foundation::array::CFArray::<CFTypeRef>::wrap_under_get_rule(selected_children_ref as *const _);
            let count = cfarray.len();
            
            for i in 0..count.min(10) { // Limit to 10 items
                if let Some(child_ref) = cfarray.get(i) {
                    let child = *child_ref as AXUIElementRef;
                    
                    // Try to get the URL or title of the selected item
                    if let Some(url) = self.get_string_attr(child, "AXURL")
                        .or_else(|| self.get_string_attr(child, "AXTitle"))
                        .or_else(|| self.get_string_attr(child, "AXFilename"))
                        .or_else(|| self.get_string_attr(child, "AXValue")) {
                        
                        let cleaned = if url.starts_with("file://") {
                            urlencoding::decode(&url[7..])
                                .unwrap_or_else(|_| url[7..].into())
                                .to_string()
                        } else {
                            url
                        };
                        items.push(cleaned);
                    }
                }
            }
            
            items
        }
    }
    
    fn get_chrome_url_via_applescript(&self) -> Result<String, String> {
        // Use AppleScript to get current Chrome URL - most reliable method
        let script = r#"tell application "Google Chrome" to get URL of active tab of front window"#;
        
        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| format!("Failed to run osascript: {}", e))?;
        
        if output.status.success() {
            let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(url)
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            Err(format!("AppleScript failed: {}", error))
        }
    }
    
    fn get_safari_url_via_applescript(&self) -> Result<String, String> {
        // Use AppleScript to get current Safari URL
        let script = r#"tell application "Safari" to get URL of current tab of front window"#;
        
        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| format!("Failed to run osascript: {}", e))?;
        
        if output.status.success() {
            let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(url)
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            Err(format!("AppleScript failed: {}", error))
        }
    }
    
    fn get_safari_title_via_applescript(&self) -> Result<String, String> {
        // Use AppleScript to get current Safari page title
        let script = r#"tell application "Safari" to get name of current tab of front window"#;
        
        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| format!("Failed to run osascript: {}", e))?;
        
        if output.status.success() {
            let title = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(title)
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            Err(format!("AppleScript failed: {}", error))
        }
    }
    
    fn get_quicktime_file_path_via_applescript(&self) -> Result<String, String> {
        // Use AppleScript to get QuickTime file path - most reliable method
        let script = r#"
            tell application "QuickTime Player"
                if (count documents) > 0 then
                    set doc to front document
                    try
                        -- Get the file path
                        set filePath to file of doc as string
                        -- Convert HFS path to POSIX
                        set posixPath to POSIX path of filePath
                        return posixPath
                    on error
                        -- For unsaved or streaming content
                        return name of doc
                    end try
                else
                    return ""
                end if
            end tell
        "#;
        
        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| format!("Failed to run osascript: {}", e))?;
        
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() && path != "missing value" {
                Ok(path)
            } else {
                Err("No document open".to_string())
            }
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            Err(format!("AppleScript failed: {}", error))
        }
    }
    
    fn find_address_bar(&self, window: AXUIElementRef) -> Option<AXUIElementRef> {
        // Actively search for the address bar in the window
        self.find_address_bar_recursive(window, 0)
    }
    
    fn find_address_bar_recursive(&self, element: AXUIElementRef, depth: usize) -> Option<AXUIElementRef> {
        if depth > 5 {  // Limit recursion depth
            return None;
        }
        
        unsafe {
            // Get children
            if let Some(children_ref) = self.get_attribute(element, "AXChildren") {
                let children = core_foundation::array::CFArray::<CFTypeRef>::wrap_under_get_rule(children_ref as *const _);
                
                for i in 0..children.len().min(100) {  // Limit to 100 children
                    if let Some(child_ref) = children.get(i) {
                        let child = *child_ref as AXUIElementRef;
                        
                        // Check if this is a text field with address-like description
                        if let Some(role) = self.get_string_attr(child, "AXRole") {
                            if role == "AXTextField" || role == "AXComboBox" {
                                // Check description - include more patterns based on research
                                if let Some(desc) = self.get_string_attr(child, "AXDescription") {
                                    let desc_lower = desc.to_lowercase();
                                    if desc_lower.contains("address") || 
                                       desc_lower.contains("search") ||
                                       desc_lower.contains("url") ||
                                       desc_lower.contains("location") ||
                                       desc_lower.contains("omnibox") ||
                                       desc_lower.contains("type web address") ||
                                       desc_lower.contains("search bar") {
                                        CFRelease(children_ref);
                                        return Some(child);
                                    }
                                }
                                
                                // Also check if it has a URL-like value
                                if let Some(value) = self.get_string_attr(child, "AXValue") {
                                    if value.starts_with("http") || value.starts_with("file:") || 
                                       (value.contains(".") && (value.contains("com") || value.contains("org") || value.contains("net"))) {
                                        CFRelease(children_ref);
                                        return Some(child);
                                    }
                                }
                            }
                        }
                        
                        // Recursively search
                        if let Some(result) = self.find_address_bar_recursive(child, depth + 1) {
                            CFRelease(children_ref);
                            return Some(result);
                        }
                    }
                }
                CFRelease(children_ref);
            }
            None
        }
    }
    
    fn find_browser_view(&self, window: AXUIElementRef) -> Option<AXUIElementRef> {
        // Find the browser content view within a window using Accessibility API
        unsafe {
            // Try to find an element with AXWebArea role (common for browser content)
            if let Some(children_ref) = self.get_attribute(window, "AXChildren") {
                {
                    let children = core_foundation::array::CFArray::<CFTypeRef>::wrap_under_get_rule(children_ref as *const _);
                    for i in 0..children.len() {
                        if let Some(child_ref) = children.get(i) {
                            let child = *child_ref as AXUIElementRef;
                            
                            // Check if this is a web area or browser content
                            if let Some(role) = self.get_string_attr(child, "AXRole") {
                                if role == "AXWebArea" || role == "AXScrollArea" || role == "AXGroup" {
                                    // Try to get URL from this element
                                    if self.get_string_attr(child, "AXURL").is_some() {
                                        CFRelease(children_ref);
                                        return Some(child);
                                    }
                                    
                                    // Recursively search deeper
                                    if let Some(web_view) = self.find_browser_view(child) {
                                        CFRelease(children_ref);
                                        return Some(web_view);
                                    }
                                }
                            }
                        }
                    }
                }
                CFRelease(children_ref);
            }
            None
        }
    }
    
    fn extract_finder_selection(&self, window: AXUIElementRef) -> Vec<String> {
        // Extract selected items in Finder using comprehensive Accessibility API
        unsafe {
            let mut items = Vec::new();
            
            // Get all children of the window using proper count/values API
            let attr = CFString::new("AXChildren");
            let mut count: CFIndex = 0;
            
            if AXUIElementGetAttributeValueCount(window, attr.as_concrete_TypeRef() as CFStringRef, &mut count) == kAXErrorSuccess && count > 0 {
                // Allocate array for children
                let mut children_array: Vec<CFTypeRef> = vec![null_mut(); count as usize];
                
                if AXUIElementCopyAttributeValues(
                    window,
                    attr.as_concrete_TypeRef() as CFStringRef,
                    0,
                    count,
                    children_array.as_mut_ptr()
                ) == kAXErrorSuccess {
                    
                    // Process each child element
                    for child_ref in children_array.iter() {
                        if !child_ref.is_null() {
                            let child = *child_ref as AXUIElementRef;
                            
                            // Check role
                            if let Some(role) = self.get_string_attr(child, "AXRole") {
                                // Look for list, browser, or scroll area
                                if role == "AXList" || role == "AXBrowser" || role == "AXScrollArea" || role == "AXOutline" {
                                    // Try multiple selection attributes
                                    for selection_attr in &["AXSelectedRows", "AXSelectedChildren", "AXSelectedCells"] {
                                        if let Some(selected_ref) = self.get_attribute(child, selection_attr) {
                                            // Process selected items
                                            let selected_items = self.extract_paths_from_selection(selected_ref);
                                            items.extend(selected_items);
                                            CFRelease(selected_ref);
                                        }
                                    }
                                    
                                    // If no selection, try to get current folder from AXDocument
                                    if items.is_empty() {
                                        if let Some(doc) = self.get_string_attr(child, "AXDocument")
                                            .or_else(|| self.get_string_attr(child, "AXURL")) {
                                            let cleaned = if doc.starts_with("file://") {
                                                urlencoding::decode(&doc[7..])
                                                    .unwrap_or_else(|_| doc[7..].into())
                                                    .to_string()
                                            } else {
                                                doc
                                            };
                                            items.push(cleaned);
                                        }
                                    }
                                    
                                    // Also recursively check child elements
                                    let child_items = self.extract_finder_selection(child);
                                    items.extend(child_items);
                                }
                            }
                            
                            // Clean up
                            CFRelease(*child_ref);
                        }
                    }
                }
            }
            
            // Remove duplicates and return
            items.sort();
            items.dedup();
            items
        }
    }
    
    fn extract_url_from_address_bar(&self, window: AXUIElementRef) -> Option<String> {
        unsafe {
            // Search for the address bar (usually a text field with specific role)
            if let Some(toolbar_ref) = self.get_attribute(window, "AXToolbar") {
                // Look for text field in toolbar
                if let Some(children_ref) = self.get_attribute(toolbar_ref as AXUIElementRef, "AXChildren") {
                    {
                        let children = core_foundation::array::CFArray::<CFTypeRef>::wrap_under_get_rule(children_ref as *const _);
                        for i in 0..children.len() {
                            if let Some(child_ref) = children.get(i) {
                                let child = *child_ref as AXUIElementRef;
                                if let Some(role) = self.get_string_attr(child, "AXRole") {
                                    if role == "AXTextField" || role == "AXComboBox" {
                                        if let Some(url) = self.get_string_attr(child, "AXValue") {
                                            CFRelease(children_ref);
                                            CFRelease(toolbar_ref);
                                            return Some(url);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    CFRelease(children_ref);
                }
                CFRelease(toolbar_ref);
            }
            None
        }
    }
    
    fn extract_url_from_tab_content(&self, window: AXUIElementRef) -> Option<String> {
        unsafe {
            // Look for the current tab's web area
            if let Some(children_ref) = self.get_attribute(window, "AXChildren") {
                {
                    let children = core_foundation::array::CFArray::<CFTypeRef>::wrap_under_get_rule(children_ref as *const _);
                    for i in 0..children.len() {
                        if let Some(child_ref) = children.get(i) {
                            let child = *child_ref as AXUIElementRef;
                            if let Some(role) = self.get_string_attr(child, "AXRole") {
                                // Look for tab content area
                                if role == "AXTabGroup" || role == "AXGroup" {
                                    // Get selected tab
                                    if let Some(selected_ref) = self.get_attribute(child, "AXSelectedChildren") {
                                        {
                                            let selected = core_foundation::array::CFArray::<CFTypeRef>::wrap_under_get_rule(selected_ref as *const _);
                                            if let Some(tab_ref) = selected.get(0) {
                                                let tab = *tab_ref as AXUIElementRef;
                                                // Try to get URL from tab
                                                if let Some(url) = self.get_string_attr(tab, "AXURL")
                                                    .or_else(|| self.get_string_attr(tab, "AXValue"))
                                                    .or_else(|| self.get_string_attr(tab, "AXDescription")) {
                                                    CFRelease(selected_ref);
                                                    CFRelease(children_ref);
                                                    return Some(url);
                                                }
                                            }
                                        }
                                        CFRelease(selected_ref);
                                    }
                                }
                            }
                        }
                    }
                }
                CFRelease(children_ref);
            }
            None
        }
    }
    
    fn extract_url_from_text(&self, text: &str) -> Option<String> {
        // Extract URL from text using simple pattern matching
        if let Some(start) = text.find("http://").or_else(|| text.find("https://")) {
            let url_part: String = text[start..].chars()
                .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
                .collect();
            if !url_part.is_empty() {
                return Some(url_part);
            }
        }
        None
    }
    
    fn extract_paths_from_selection(&self, selected_ref: CFTypeRef) -> Vec<String> {
        unsafe {
            let mut paths = Vec::new();
            
            // Try treating it as an array first
            {
                let array = core_foundation::array::CFArray::<CFTypeRef>::wrap_under_get_rule(selected_ref as *const _);
                for i in 0..array.len().min(20) { // Limit to 20 items
                    if let Some(item_ref) = array.get(i) {
                        let item = *item_ref as AXUIElementRef;
                        
                        // Try multiple attributes to get the path
                        if let Some(path) = self.get_string_attr(item, "AXURL")
                            .or_else(|| self.get_string_attr(item, "AXDocument"))
                            .or_else(|| self.get_string_attr(item, "AXPath"))
                            .or_else(|| self.get_string_attr(item, "AXFilename"))
                            .or_else(|| self.get_string_attr(item, "AXTitle"))
                            .or_else(|| self.get_string_attr(item, "AXValue"))
                            .or_else(|| self.get_string_attr(item, "AXDescription")) {
                            
                            let cleaned = if path.starts_with("file://") {
                                urlencoding::decode(&path[7..])
                                    .unwrap_or_else(|_| path[7..].into())
                                    .to_string()
                            } else {
                                path
                            };
                            
                            // Only add if it looks like a path
                            if cleaned.starts_with("/") || cleaned.contains("/") {
                                paths.push(cleaned);
                            }
                        }
                    }
                }
            }
            
            paths
        }
    }
    
    // Safari-specific helper functions
    fn find_safari_web_area(&self, window: AXUIElementRef) -> Option<AXUIElementRef> {
        self.find_safari_web_area_recursive(window, 0)
    }
    
    fn find_safari_web_area_recursive(&self, element: AXUIElementRef, depth: usize) -> Option<AXUIElementRef> {
        // Prevent infinite recursion
        if depth > 10 {
            return None;
        }
        
        unsafe {
            // Get children of element
            let children_attr = CFString::new("AXChildren");
            let mut children_value: CFTypeRef = std::ptr::null();
            
            if AXUIElementCopyAttributeValue(element, children_attr.as_concrete_TypeRef(), &mut children_value) != kAXErrorSuccess {
                return None;
            }
            
            if children_value.is_null() {
                return None;
            }
            
            let children = children_value as CFArrayRef;
            let count = CFArrayGetCount(children);
            
            // Limit children to prevent issues
            let max_children = count.min(50);
            
            for i in 0..max_children {
                let child = CFArrayGetValueAtIndex(children, i);
                if child.is_null() {
                    continue;
                }
                
                let child_element = child as AXUIElementRef;
                
                // Check if this is a WebArea
                let role_attr = CFString::new("AXRole");
                let mut role_value: CFTypeRef = std::ptr::null();
                
                if AXUIElementCopyAttributeValue(child_element, role_attr.as_concrete_TypeRef(), &mut role_value) == kAXErrorSuccess {
                    if !role_value.is_null() {
                        let role_type = CFGetTypeID(role_value);
                        if role_type == CFStringGetTypeID() {
                            let role_str = role_value as CFStringRef;
                            let role = CFString::wrap_under_get_rule(role_str).to_string();
                            
                            if role == "AXWebArea" {
                                CFRelease(role_value);
                                CFRelease(children_value);
                                return Some(child_element);
                            }
                        }
                        CFRelease(role_value);
                    }
                }
                
                // Recursively search in children
                if let Some(web_area) = self.find_safari_web_area_recursive(child_element, depth + 1) {
                    CFRelease(children_value);
                    return Some(web_area);
                }
            }
            CFRelease(children_value);
            None
        }
    }
    
    fn find_safari_toolbar(&self, window: AXUIElementRef) -> Option<AXUIElementRef> {
        unsafe {
            // Get children of window
            let children_attr = CFString::new("AXChildren");
            let mut children_value: CFTypeRef = std::ptr::null();
            
            if AXUIElementCopyAttributeValue(window, children_attr.as_concrete_TypeRef(), &mut children_value) == kAXErrorSuccess {
                if children_value.is_null() {
                    return None;
                }
                
                let children = children_value as CFArrayRef;
                let count = CFArrayGetCount(children).min(50);
                
                for i in 0..count {
                    let child = CFArrayGetValueAtIndex(children, i);
                    if child.is_null() {
                        continue;
                    }
                    let element = child as AXUIElementRef;
                    
                    // Check if this is a toolbar
                    let role_attr = CFString::new("AXRole");
                    let mut role_value: CFTypeRef = std::ptr::null();
                    
                    if AXUIElementCopyAttributeValue(element, role_attr.as_concrete_TypeRef(), &mut role_value) == kAXErrorSuccess {
                        let role_type = CFGetTypeID(role_value);
                        if role_type == CFStringGetTypeID() {
                            let role_str = role_value as CFStringRef;
                            let role = CFString::wrap_under_get_rule(role_str).to_string();
                            
                            if role == "AXToolbar" || role == "AXGroup" {
                                // Check if it contains address-related elements
                                let desc_attr = CFString::new("AXDescription");
                                let mut desc_value: CFTypeRef = std::ptr::null();
                                
                                if AXUIElementCopyAttributeValue(element, desc_attr.as_concrete_TypeRef(), &mut desc_value) == kAXErrorSuccess {
                                    let desc_type = CFGetTypeID(desc_value);
                                    if desc_type == CFStringGetTypeID() {
                                        let desc_str = desc_value as CFStringRef;
                                        let desc = CFString::wrap_under_get_rule(desc_str).to_string();
                                        
                                        if desc.to_lowercase().contains("navigation") || desc.to_lowercase().contains("toolbar") {
                                            CFRelease(desc_value);
                                            CFRelease(role_value);
                                            CFRelease(children_value);
                                            return Some(element);
                                        }
                                    }
                                    CFRelease(desc_value);
                                }
                                
                                // Also return if it's just a toolbar
                                if role == "AXToolbar" {
                                    CFRelease(role_value);
                                    CFRelease(children_value);
                                    return Some(element);
                                }
                            }
                        }
                        CFRelease(role_value);
                    }
                }
                CFRelease(children_value);
            }
            None
        }
    }
    
    fn find_safari_url_field(&self, toolbar: AXUIElementRef) -> Option<AXUIElementRef> {
        unsafe {
            // Get children of toolbar
            let children_attr = CFString::new("AXChildren");
            let mut children_value: CFTypeRef = std::ptr::null();
            
            if AXUIElementCopyAttributeValue(toolbar, children_attr.as_concrete_TypeRef(), &mut children_value) == kAXErrorSuccess {
                if children_value.is_null() {
                    return None;
                }
                
                let children = children_value as CFArrayRef;
                let count = CFArrayGetCount(children).min(50);
                
                for i in 0..count {
                    let child = CFArrayGetValueAtIndex(children, i);
                    if child.is_null() {
                        continue;
                    }
                    let element = child as AXUIElementRef;
                    
                    // Check if this is a text field
                    let role_attr = CFString::new("AXRole");
                    let mut role_value: CFTypeRef = std::ptr::null();
                    
                    if AXUIElementCopyAttributeValue(element, role_attr.as_concrete_TypeRef(), &mut role_value) == kAXErrorSuccess {
                        let role_type = CFGetTypeID(role_value);
                        if role_type == CFStringGetTypeID() {
                            let role_str = role_value as CFStringRef;
                            let role = CFString::wrap_under_get_rule(role_str).to_string();
                            
                            if role == "AXTextField" || role == "AXComboBox" {
                                // Check description for address/URL indication
                                let desc_attr = CFString::new("AXDescription");
                                let mut desc_value: CFTypeRef = std::ptr::null();
                                
                                if AXUIElementCopyAttributeValue(element, desc_attr.as_concrete_TypeRef(), &mut desc_value) == kAXErrorSuccess {
                                    let desc_type = CFGetTypeID(desc_value);
                                    if desc_type == CFStringGetTypeID() {
                                        let desc_str = desc_value as CFStringRef;
                                        let desc = CFString::wrap_under_get_rule(desc_str).to_string();
                                        
                                        if desc.to_lowercase().contains("address") || 
                                           desc.to_lowercase().contains("search") || 
                                           desc.to_lowercase().contains("url") {
                                            CFRelease(desc_value);
                                            CFRelease(role_value);
                                            CFRelease(children_value);
                                            return Some(element);
                                        }
                                    }
                                    CFRelease(desc_value);
                                }
                                
                                // Even without description, a text field in toolbar is likely the URL bar
                                CFRelease(role_value);
                                CFRelease(children_value);
                                return Some(element);
                            }
                        }
                        CFRelease(role_value);
                    }
                    
                    // Don't recurse to avoid issues
                }
                CFRelease(children_value);
            }
            None
        }
    }
}

extern "C" fn ax_callback(
    _observer: AXObserverRef,
    _element: AXUIElementRef,
    notification: CFStringRef,
    _user_data: *mut c_void,
) {
    unsafe {
        let notif = CFString::wrap_under_get_rule(notification).to_string();
        with_state(|tracker| {
            tracker.handle_ui_change(&notif);
        });
    }
}

extern "C" fn workspace_callback(_this: &Object, _cmd: Sel, notification: id) {
    unsafe {
        let user_info: id = msg_send![notification, userInfo];
        if user_info == nil { return; }

        let key = "NSWorkspaceApplicationKey";
        let ns_key: id = msg_send![class!(NSString), alloc];
        let ns_key: id = msg_send![ns_key, initWithBytes:key.as_ptr() 
                                          length:key.len() 
                                          encoding:4];
        
        let app: id = msg_send![user_info, objectForKey:ns_key];
        let _: () = msg_send![ns_key, release];
        
        if app == nil { return; }

        let bundle_id: id = msg_send![app, bundleIdentifier];
        let name: id = msg_send![app, localizedName];
        let pid: i32 = msg_send![app, processIdentifier];

        let bundle_str = if bundle_id != nil {
            CStr::from_ptr(msg_send![bundle_id, UTF8String])
                .to_string_lossy()
                .to_string()
        } else {
            String::new()
        };

        let name_str = if name != nil {
            CStr::from_ptr(msg_send![name, UTF8String])
                .to_string_lossy()
                .to_string()
        } else {
            String::from("Unknown")
        };

        with_state(|tracker| {
            tracker.handle_app_change(name_str, bundle_str, pid);
        });
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        unsafe {
            // Clean up keyboard tap
            let tap = KEYBOARD_TAP.load(Ordering::SeqCst);
            if !tap.is_null() {
                CGEventTapEnable(tap, false);
            }
            
            let source = KEYBOARD_TAP_SOURCE.load(Ordering::SeqCst);
            if !source.is_null() {
                let run_loop = CFRunLoop::get_current();
                CFRunLoopRemoveSource(
                    run_loop.as_concrete_TypeRef(),
                    source as *mut _,
                    kCFRunLoopDefaultMode,
                );
                CFRelease(source);
            }
            
            // Clean up scroll tap  
            let tap = SCROLL_TAP.load(Ordering::SeqCst);
            if !tap.is_null() {
                CGEventTapEnable(tap, false);
            }
            
            let source = SCROLL_TAP_SOURCE.load(Ordering::SeqCst);
            if !source.is_null() {
                let run_loop = CFRunLoop::get_current();
                CFRunLoopRemoveSource(
                    run_loop.as_concrete_TypeRef(),
                    source as *mut _,
                    kCFRunLoopDefaultMode,
                );
                CFRelease(source);
            }
        }
    }
}

fn with_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut Tracker) -> R,
{
    let state = STATE.get().expect("State not initialized");
    let mut tracker = state.lock().unwrap();
    f(&mut *tracker)
}

fn create_observer_class() -> *const Class {
    let superclass = class!(NSObject);
    let mut decl = ClassDecl::new("FocusObserver", superclass).unwrap();
    
    unsafe {
        decl.add_method(
            sel!(workspaceDidActivateApp:),
            workspace_callback as extern "C" fn(&Object, Sel, id),
        );
    }
    
    decl.register()
}

fn ensure_ax_trust(prompt: bool) -> bool {
    unsafe {
        if AXIsProcessTrusted() {
            return true;
        }
        
        if prompt {
            let options = CFDictionary::from_CFType_pairs(&[
                (
                    CFString::new("AXTrustedCheckOptionPrompt").as_CFType(),
                    CFBoolean::from(true).as_CFType(),
                ),
            ]);
            AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef());
        }
        
        AXIsProcessTrusted()
    }
}

fn main() {
    let cli = Cli::parse();
    
    if !ensure_ax_trust(!cli.no_prompt) {
        eprintln!("Accessibility access required.");
        eprintln!("Enable in: System Settings → Privacy & Security → Accessibility");
        std::process::exit(1);
    }

    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        
        let app = NSApp();
        app.setActivationPolicy_(NSApplicationActivationPolicyProhibited);
        
        STATE.set(Arc::new(Mutex::new(Tracker::new(&cli))))
            .expect("Failed to initialize");
        
        // Start clipboard monitoring thread
        {
            let mut tracker = STATE.get().unwrap().lock().unwrap();
            tracker.start_clipboard_monitor();
            tracker.setup_keyboard_tap();
            tracker.setup_scroll_tap();
        }
        
        // Get initial app
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let frontmost: id = msg_send![workspace, frontmostApplication];
        
        if frontmost != nil {
            let bundle_id: id = msg_send![frontmost, bundleIdentifier];
            let name: id = msg_send![frontmost, localizedName];
            let pid: i32 = msg_send![frontmost, processIdentifier];

            let bundle_str = if bundle_id != nil {
                CStr::from_ptr(msg_send![bundle_id, UTF8String])
                    .to_string_lossy()
                    .to_string()
            } else {
                String::new()
            };

            let name_str = if name != nil {
                CStr::from_ptr(msg_send![name, UTF8String])
                    .to_string_lossy()
                    .to_string()
            } else {
                String::from("Unknown")
            };
            
            with_state(|tracker| {
                tracker.handle_app_change(name_str, bundle_str, pid);
            });
        }
        
        // Register workspace observer
        let observer_class = create_observer_class();
        let observer: id = msg_send![observer_class, new];
        
        let nc: id = msg_send![workspace, notificationCenter];
        let notif_name: id = msg_send![class!(NSString), alloc];
        let notif_name: id = msg_send![notif_name, initWithBytes:"NSWorkspaceDidActivateApplicationNotification".as_ptr()
                                                   length:"NSWorkspaceDidActivateApplicationNotification".len()
                                                   encoding:4];
        
        let _: () = msg_send![nc,
            addObserver:observer
            selector:sel!(workspaceDidActivateApp:)
            name:notif_name
            object:nil
        ];
        
        let _: () = msg_send![notif_name, release];
        
        if cli.format != "json" {
            println!("Tracking app context, URLs, files, and UI state...");
        }
        
        CFRunLoopRun();
        
        let _: () = msg_send![pool, drain];
    }
}