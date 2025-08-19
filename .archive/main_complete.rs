#![cfg(target_os = "macos")]

use std::ffi::{c_void, CStr};
use std::ptr::null_mut;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use std::collections::HashMap;

use accessibility_sys::*;
use clap::Parser;
use cocoa::appkit::{NSApp, NSApplication, NSApplicationActivationPolicyProhibited};
use cocoa::base::{id, nil};
use cocoa::foundation::NSAutoreleasePool;
use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::runloop::{CFRunLoop, CFRunLoopRun, kCFRunLoopDefaultMode};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation_sys::runloop::{CFRunLoopAddSource, CFRunLoopRemoveSource};
use core_graphics::display::CGPoint;
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use serde::Serialize;

/// CLI flags
#[derive(Debug, Parser)]
#[command(name = "focus-tracker-complete", version, about = "Complete context tracking for macOS")]
struct Cli {
    #[arg(long, default_value = "json", value_parser = ["text", "json"])]
    format: String,
    
    #[arg(long)]
    no_prompt: bool,
}

#[derive(Debug, Clone, Serialize)]
struct CompleteContext {
    // Core app info
    app_name: String,
    bundle_id: String,
    pid: i32,
    app_path: Option<String>,
    
    // Window & document info
    window_title: Option<String>,
    window_position: Option<(f64, f64)>,
    window_size: Option<(f64, f64)>,
    document_path: Option<String>,
    document_modified: Option<bool>,
    
    // Web context (for browsers)
    url: Option<String>,
    page_title: Option<String>,
    scroll_position_web: Option<(f64, f64)>,
    
    // IDE context (for Cursor/VS Code/JetBrains)
    active_file_path: Option<String>,
    active_project: Option<String>,
    terminal_tab: Option<String>,
    terminal_content: Option<String>,
    
    // Spreadsheet context
    active_sheet: Option<String>,
    active_cell: Option<String>,
    
    // UI element details
    focused_element: Option<UIElementInfo>,
    clicked_element: Option<UIElementInfo>,
    ui_breadcrumb: Vec<String>, // Path through UI hierarchy
    
    // User interaction
    mouse_position: Option<(f64, f64)>,
    last_click: Option<ClickInfo>,
    scroll_delta: Option<(f64, f64)>,
    key_modifiers: Option<String>,
    
    // Timing & metrics
    #[serde(skip)]
    started_at: Instant,
    duration_ms: Option<u128>,
    idle_time_ms: Option<u128>,
}

#[derive(Debug, Clone, Serialize)]
struct UIElementInfo {
    role: Option<String>,
    title: Option<String>,
    value: Option<String>,
    description: Option<String>,
    help: Option<String>,
    position: Option<(f64, f64)>,
    size: Option<(f64, f64)>,
    enabled: Option<bool>,
    focused: Option<bool>,
    selected: Option<bool>,
    identifier: Option<String>,
    url: Option<String>,
    placeholder: Option<String>,
    selected_text: Option<String>,
    insertion_point: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
struct ClickInfo {
    position: (f64, f64),
    #[serde(skip)]
    time: Instant,
    button: String,
    count: u32,
}

#[derive(Debug, Clone, Serialize)]
struct ContextEvent {
    event_type: String,
    timestamp: u128,
    context: CompleteContext,
    trigger: String,
    metadata: HashMap<String, String>,
}

struct CompleteTracker {
    current_context: Option<CompleteContext>,
    context_history: Vec<CompleteContext>,
    json_output: bool,
    current_ax_observer: Option<usize>,
    event_tap: Option<*mut c_void>,
    start_time: Instant,
    last_activity: Instant,
    url_dwell_times: HashMap<String, Duration>,
    current_url_start: Option<(String, Instant)>,
}

static STATE: OnceLock<Arc<Mutex<CompleteTracker>>> = OnceLock::new();

// Event tap FFI
extern "C" {
    fn CGEventTapCreate(
        tap: i32,
        place: i32,
        options: i32,
        events_of_interest: u64,
        callback: extern "C" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> *mut c_void,
        user_info: *mut c_void,
    ) -> *mut c_void;
    
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    fn CFMachPortCreateRunLoopSource(allocator: *mut c_void, port: *mut c_void, order: isize) -> *mut c_void;
}

impl CompleteTracker {
    fn new(json_output: bool) -> Self {
        Self {
            current_context: None,
            context_history: Vec::new(),
            json_output,
            current_ax_observer: None,
            event_tap: None,
            start_time: Instant::now(),
            last_activity: Instant::now(),
            url_dwell_times: HashMap::new(),
            current_url_start: None,
        }
    }

    fn extract_complete_ui_info(&self, element: AXUIElementRef) -> UIElementInfo {
        unsafe {
            let mut info = UIElementInfo {
                role: self.get_string_attribute(element, "AXRole"),
                title: self.get_string_attribute(element, "AXTitle"),
                value: self.get_string_attribute(element, "AXValue"),
                description: self.get_string_attribute(element, "AXDescription"),
                help: self.get_string_attribute(element, "AXHelp"),
                position: None,
                size: None,
                enabled: self.get_bool_attribute(element, "AXEnabled"),
                focused: self.get_bool_attribute(element, "AXFocused"),
                selected: self.get_bool_attribute(element, "AXSelected"),
                identifier: self.get_string_attribute(element, "AXIdentifier"),
                url: self.get_string_attribute(element, "AXURL"),
                placeholder: self.get_string_attribute(element, "AXPlaceholderValue"),
                selected_text: self.get_string_attribute(element, "AXSelectedText"),
                insertion_point: self.get_number_attribute(element, "AXInsertionPointLineNumber"),
            };
            
            // Get position and size
            if let Some(pos_ref) = self.get_attribute(element, "AXPosition") {
                info.position = self.extract_point_value(pos_ref);
                CFRelease(pos_ref);
            }
            
            if let Some(size_ref) = self.get_attribute(element, "AXSize") {
                info.size = self.extract_size_value(size_ref);
                CFRelease(size_ref);
            }
            
            info
        }
    }

    fn get_browser_url(&self, app_element: AXUIElementRef, bundle_id: &str) -> Option<String> {
        unsafe {
            // Try to get the focused window
            if let Some(window_ref) = self.get_attribute(app_element, "AXFocusedWindow") {
                let window = window_ref as AXUIElementRef;
                
                // For Chrome/Safari/Firefox, look for the URL in the toolbar
                if bundle_id.contains("chrome") || bundle_id.contains("safari") || bundle_id.contains("firefox") {
                    // Try to find the address bar
                    if let Some(toolbar_ref) = self.get_attribute(window, "AXToolbar") {
                        let toolbar = toolbar_ref as AXUIElementRef;
                        
                        // Look for text fields in toolbar (usually the address bar)
                        if let Some(children_ref) = self.get_attribute(toolbar, "AXChildren") {
                            // This would need proper CFArray handling to iterate children
                            // and find the URL text field
                            CFRelease(children_ref);
                        }
                        CFRelease(toolbar_ref);
                    }
                    
                    // Alternative: Check for AXDocument which sometimes contains the URL
                    let url = self.get_string_attribute(window, "AXDocument")
                        .or_else(|| self.get_string_attribute(window, "AXURL"));
                    
                    CFRelease(window_ref);
                    return url;
                }
                
                CFRelease(window_ref);
            }
            
            None
        }
    }

    fn get_ide_file_path(&self, app_element: AXUIElementRef, app_name: &str) -> Option<String> {
        unsafe {
            if let Some(window_ref) = self.get_attribute(app_element, "AXFocusedWindow") {
                let window = window_ref as AXUIElementRef;
                
                // For VS Code/Cursor/JetBrains IDEs
                if app_name.contains("Code") || app_name.contains("Cursor") || app_name.contains("IntelliJ") || app_name.contains("WebStorm") {
                    // Document often contains the file path
                    if let Some(doc) = self.get_string_attribute(window, "AXDocument") {
                        CFRelease(window_ref);
                        return Some(doc);
                    }
                    
                    // Try window title which often has the file path
                    if let Some(title) = self.get_string_attribute(window, "AXTitle") {
                        // Parse file path from title (usually format: "filename — project — AppName")
                        if let Some(first_part) = title.split(" — ").next() {
                            // This might be a relative path, try to resolve it
                            CFRelease(window_ref);
                            return Some(first_part.to_string());
                        }
                    }
                }
                
                CFRelease(window_ref);
            }
            
            None
        }
    }

    fn get_terminal_info(&self, app_element: AXUIElementRef) -> (Option<String>, Option<String>) {
        unsafe {
            if let Some(window_ref) = self.get_attribute(app_element, "AXFocusedWindow") {
                let window = window_ref as AXUIElementRef;
                
                // Try to get terminal tab info
                let tab_title = self.get_string_attribute(window, "AXTitle");
                
                // Try to get terminal content from focused text area
                if let Some(focused_ref) = self.get_attribute(window, "AXFocusedUIElement") {
                    let focused = focused_ref as AXUIElementRef;
                    let content = self.get_string_attribute(focused, "AXValue");
                    CFRelease(focused_ref);
                    CFRelease(window_ref);
                    return (tab_title, content);
                }
                
                CFRelease(window_ref);
                return (tab_title, None);
            }
            
            (None, None)
        }
    }

    fn get_spreadsheet_info(&self, app_element: AXUIElementRef) -> (Option<String>, Option<String>) {
        unsafe {
            if let Some(window_ref) = self.get_attribute(app_element, "AXFocusedWindow") {
                let window = window_ref as AXUIElementRef;
                
                // Get sheet name from window title or tab
                let sheet_name = self.get_string_attribute(window, "AXTitle");
                
                // Try to get selected cell
                if let Some(focused_ref) = self.get_attribute(window, "AXFocusedUIElement") {
                    let focused = focused_ref as AXUIElementRef;
                    
                    // For spreadsheets, this might be a cell reference
                    let cell_ref = self.get_string_attribute(focused, "AXDescription")
                        .or_else(|| self.get_string_attribute(focused, "AXHelp"));
                    
                    CFRelease(focused_ref);
                    CFRelease(window_ref);
                    return (sheet_name, cell_ref);
                }
                
                CFRelease(window_ref);
                return (sheet_name, None);
            }
            
            (None, None)
        }
    }

    fn build_ui_breadcrumb(&self, element: AXUIElementRef) -> Vec<String> {
        let mut breadcrumb = Vec::new();
        unsafe {
            let mut current = element;
            let mut depth = 0;
            
            while depth < 10 {
                if let Some(parent_ref) = self.get_attribute(current, "AXParent") {
                    let parent = parent_ref as AXUIElementRef;
                    
                    if let Some(role) = self.get_string_attribute(parent, "AXRole") {
                        let title = self.get_string_attribute(parent, "AXTitle")
                            .unwrap_or_else(|| role.clone());
                        breadcrumb.push(title);
                    }
                    
                    if depth > 0 {
                        CFRelease(current as CFTypeRef);
                    }
                    current = parent;
                    depth += 1;
                } else {
                    break;
                }
            }
            
            if depth > 0 {
                CFRelease(current as CFTypeRef);
            }
        }
        
        breadcrumb.reverse();
        breadcrumb
    }

    fn extract_complete_context(&mut self, app_name: String, bundle_id: String, pid: i32) -> CompleteContext {
        unsafe {
            let app_element = AXUIElementCreateApplication(pid);
            
            let mut context = CompleteContext {
                app_name: app_name.clone(),
                bundle_id: bundle_id.clone(),
                pid,
                app_path: self.get_app_installation_path(&bundle_id),
                window_title: None,
                window_position: None,
                window_size: None,
                document_path: None,
                document_modified: None,
                url: None,
                page_title: None,
                scroll_position_web: None,
                active_file_path: None,
                active_project: None,
                terminal_tab: None,
                terminal_content: None,
                active_sheet: None,
                active_cell: None,
                focused_element: None,
                clicked_element: None,
                ui_breadcrumb: Vec::new(),
                mouse_position: None,
                last_click: None,
                scroll_delta: None,
                key_modifiers: None,
                started_at: Instant::now(),
                duration_ms: None,
                idle_time_ms: Some(self.last_activity.elapsed().as_millis()),
            };

            // Get window information
            if let Some(window_ref) = self.get_attribute(app_element, "AXFocusedWindow") {
                let window = window_ref as AXUIElementRef;
                
                context.window_title = self.get_string_attribute(window, "AXTitle");
                
                // Get position and size
                if let Some(pos_ref) = self.get_attribute(window, "AXPosition") {
                    context.window_position = self.extract_point_value(pos_ref);
                    CFRelease(pos_ref);
                }
                
                if let Some(size_ref) = self.get_attribute(window, "AXSize") {
                    context.window_size = self.extract_size_value(size_ref);
                    CFRelease(size_ref);
                }
                
                // Check document modified state
                context.document_modified = self.get_bool_attribute(window, "AXIsDocumentEdited");
                
                CFRelease(window_ref);
            }
            
            // Browser-specific: Get URL
            if bundle_id.contains("chrome") || bundle_id.contains("safari") || bundle_id.contains("firefox") || bundle_id.contains("edge") {
                context.url = self.get_browser_url(app_element, &bundle_id);
                context.page_title = context.window_title.clone();
                
                // Track URL dwell time
                if let Some(url) = &context.url {
                    self.update_url_dwell_time(url.clone());
                }
            }
            
            // IDE-specific: Get file paths
            if app_name.contains("Code") || app_name.contains("Cursor") || app_name.contains("IntelliJ") || app_name.contains("WebStorm") {
                context.active_file_path = self.get_ide_file_path(app_element, &app_name);
                
                // Try to extract project from window title
                if let Some(title) = &context.window_title {
                    let parts: Vec<&str> = title.split(" — ").collect();
                    if parts.len() >= 2 {
                        context.active_project = Some(parts[1].to_string());
                    }
                }
                
                // Get terminal info if in terminal
                let (tab, content) = self.get_terminal_info(app_element);
                context.terminal_tab = tab;
                context.terminal_content = content;
            }
            
            // Terminal app
            if app_name.contains("Terminal") || app_name.contains("iTerm") {
                let (tab, content) = self.get_terminal_info(app_element);
                context.terminal_tab = tab;
                context.terminal_content = content;
            }
            
            // Spreadsheet apps
            if app_name.contains("Excel") || app_name.contains("Numbers") || app_name.contains("Sheets") {
                let (sheet, cell) = self.get_spreadsheet_info(app_element);
                context.active_sheet = sheet;
                context.active_cell = cell;
            }
            
            // Finder-specific: Get current folder path
            if app_name == "Finder" {
                if let Some(window_ref) = self.get_attribute(app_element, "AXFocusedWindow") {
                    let window = window_ref as AXUIElementRef;
                    
                    // Try multiple methods to get the path
                    context.document_path = self.get_string_attribute(window, "AXDocument")
                        .or_else(|| self.get_string_attribute(window, "AXURL"));
                    
                    // Clean up file:// URLs
                    if let Some(ref mut path) = context.document_path {
                        if path.starts_with("file://") {
                            *path = path.strip_prefix("file://").unwrap().to_string();
                            *path = urlencoding::decode(path).unwrap_or_else(|_| path.clone().into()).to_string();
                        }
                    }
                    
                    CFRelease(window_ref);
                }
            }
            
            // Get focused UI element details
            if let Some(focused_ref) = self.get_attribute(app_element, "AXFocusedUIElement") {
                let focused = focused_ref as AXUIElementRef;
                context.focused_element = Some(self.extract_complete_ui_info(focused));
                context.ui_breadcrumb = self.build_ui_breadcrumb(focused);
                CFRelease(focused_ref);
            }
            
            CFRelease(app_element as CFTypeRef);
            context
        }
    }

    fn update_url_dwell_time(&mut self, url: String) {
        // End previous URL timing
        if let Some((prev_url, start)) = self.current_url_start.take() {
            let duration = start.elapsed();
            *self.url_dwell_times.entry(prev_url).or_insert(Duration::ZERO) += duration;
        }
        
        // Start new URL timing
        self.current_url_start = Some((url, Instant::now()));
    }

    fn handle_app_change(&mut self, new_app_name: String, new_bundle_id: String, new_pid: i32) {
        // Clean up old observer
        if let Some(old_observer_addr) = self.current_ax_observer.take() {
            unsafe {
                let old_observer = old_observer_addr as AXObserverRef;
                let source = AXObserverGetRunLoopSource(old_observer);
                CFRunLoopRemoveSource(
                    CFRunLoop::get_current().as_concrete_TypeRef(),
                    source,
                    kCFRunLoopDefaultMode as CFStringRef
                );
                CFRelease(old_observer as CFTypeRef);
            }
        }

        let mut new_context = self.extract_complete_context(new_app_name, new_bundle_id, new_pid);
        
        // Calculate duration for previous context
        if let Some(ref old_context) = self.current_context {
            new_context.duration_ms = Some(old_context.started_at.elapsed().as_millis());
        }
        
        let event = ContextEvent {
            event_type: "app_switch".to_string(),
            timestamp: self.start_time.elapsed().as_millis(),
            context: new_context.clone(),
            trigger: "user_action".to_string(),
            metadata: HashMap::new(),
        };

        self.log_event(event);
        
        // Store in history
        if let Some(old) = self.current_context.take() {
            self.context_history.push(old);
            // Keep only last 100 contexts
            if self.context_history.len() > 100 {
                self.context_history.remove(0);
            }
        }
        
        self.setup_ax_observer_for_app(new_pid);
        self.current_context = Some(new_context);
        self.last_activity = Instant::now();
    }

    fn handle_ui_change(&mut self, notification_type: &str) {
        if let Some(context) = self.current_context.take() {
            let mut new_context = self.extract_complete_context(
                context.app_name.clone(),
                context.bundle_id.clone(),
                context.pid,
            );
            
            // Preserve interaction state
            new_context.last_click = context.last_click;
            new_context.mouse_position = context.mouse_position;
            new_context.scroll_delta = context.scroll_delta;
            
            let mut metadata = HashMap::new();
            metadata.insert("notification".to_string(), notification_type.to_string());
            
            let event = ContextEvent {
                event_type: "ui_change".to_string(),
                timestamp: self.start_time.elapsed().as_millis(),
                context: new_context.clone(),
                trigger: notification_type.to_string(),
                metadata,
            };
            
            self.log_event(event);
            self.current_context = Some(new_context);
            self.last_activity = Instant::now();
        }
    }

    fn handle_mouse_event(&mut self, event_type: CGEventType, location: CGPoint, button: i64) {
        if let Some(ref mut context) = self.current_context {
            context.mouse_position = Some((location.x, location.y));
            
            if matches!(event_type as u32, 1 | 2 | 3 | 4) { // Mouse down/up events
                let button_str = match button {
                    0 => "left",
                    1 => "right",
                    2 => "middle",
                    _ => "other",
                }.to_string();
                
                context.last_click = Some(ClickInfo {
                    position: (location.x, location.y),
                    time: Instant::now(),
                    button: button_str.clone(),
                    count: 1,
                });
                
                // Try to get the element at click position
                if let Ok(element) = self.element_at_position(location) {
                    context.clicked_element = Some(self.extract_complete_ui_info(element));
                    unsafe { CFRelease(element as CFTypeRef); }
                }
                
                let mut metadata = HashMap::new();
                metadata.insert("button".to_string(), button_str);
                metadata.insert("x".to_string(), location.x.to_string());
                metadata.insert("y".to_string(), location.y.to_string());
                
                let event = ContextEvent {
                    event_type: "mouse_click".to_string(),
                    timestamp: self.start_time.elapsed().as_millis(),
                    context: context.clone(),
                    trigger: format!("mouse_{:?}", event_type),
                    metadata,
                };
                
                self.log_event(event);
            }
            
            self.last_activity = Instant::now();
        }
    }

    fn handle_scroll_event(&mut self, delta_x: f64, delta_y: f64) {
        if let Some(ref mut context) = self.current_context {
            context.scroll_delta = Some((delta_x, delta_y));
            
            // For web contexts, try to get scroll position
            if context.url.is_some() {
                // This would need JavaScript injection to get actual scroll position
                // For now, we track the delta
                let (prev_x, prev_y) = context.scroll_position_web.unwrap_or((0.0, 0.0));
                context.scroll_position_web = Some((prev_x + delta_x, prev_y + delta_y));
            }
            
            let mut metadata = HashMap::new();
            metadata.insert("delta_x".to_string(), delta_x.to_string());
            metadata.insert("delta_y".to_string(), delta_y.to_string());
            
            let event = ContextEvent {
                event_type: "scroll".to_string(),
                timestamp: self.start_time.elapsed().as_millis(),
                context: context.clone(),
                trigger: "user_scroll".to_string(),
                metadata,
            };
            
            self.log_event(event);
            self.last_activity = Instant::now();
        }
    }

    fn handle_key_event(&mut self, flags: CGEventFlags) {
        if let Some(ref mut context) = self.current_context {
            let mut modifiers = Vec::new();
            if flags.contains(CGEventFlags::CGEventFlagMaskCommand) {
                modifiers.push("cmd");
            }
            if flags.contains(CGEventFlags::CGEventFlagMaskShift) {
                modifiers.push("shift");
            }
            if flags.contains(CGEventFlags::CGEventFlagMaskControl) {
                modifiers.push("ctrl");
            }
            if flags.contains(CGEventFlags::CGEventFlagMaskAlternate) {
                modifiers.push("alt");
            }
            
            context.key_modifiers = if modifiers.is_empty() {
                None
            } else {
                Some(modifiers.join("+"))
            };
            
            self.last_activity = Instant::now();
        }
    }

    fn element_at_position(&self, point: CGPoint) -> Result<AXUIElementRef, ()> {
        unsafe {
            let mut element: AXUIElementRef = null_mut();
            let result = AXUIElementCopyElementAtPosition(
                AXUIElementCreateSystemWide(),
                point.x as f32,
                point.y as f32,
                &mut element
            );
            
            if result == kAXErrorSuccess && !element.is_null() {
                Ok(element)
            } else {
                Err(())
            }
        }
    }

    fn log_event(&self, event: ContextEvent) {
        if self.json_output {
            println!("{}", serde_json::to_string(&event).unwrap());
        } else {
            println!("[{}] {}: {}", 
                event.event_type,
                event.context.app_name,
                event.context.window_title.as_deref().unwrap_or("no window")
            );
            
            if let Some(url) = &event.context.url {
                println!("  URL: {}", url);
            }
            if let Some(path) = &event.context.active_file_path {
                println!("  File: {}", path);
            }
            if let Some(doc) = &event.context.document_path {
                println!("  Document: {}", doc);
            }
            if !event.context.ui_breadcrumb.is_empty() {
                println!("  UI Path: {}", event.context.ui_breadcrumb.join(" > "));
            }
            if let Some((x, y)) = event.context.mouse_position {
                println!("  Mouse: ({:.0}, {:.0})", x, y);
            }
        }
    }

    fn setup_ax_observer_for_app(&mut self, pid: i32) {
        unsafe {
            let mut observer: AXObserverRef = null_mut();
            
            if AXObserverCreate(pid, ax_observer_callback, &mut observer) == kAXErrorSuccess {
                let app_element = AXUIElementCreateApplication(pid);
                
                // Register for comprehensive notifications
                let notifications = [
                    "AXFocusedWindowChanged",
                    "AXMainWindowChanged",
                    "AXWindowCreated",
                    "AXWindowMoved",
                    "AXWindowResized",
                    "AXWindowMiniaturized",
                    "AXWindowDeminiaturized",
                    "AXTitleChanged",
                    "AXFocusedUIElementChanged",
                    "AXValueChanged",
                    "AXSelectedChildrenChanged",
                    "AXSelectedTextChanged",
                    "AXMenuItemSelected",
                    "AXApplicationActivated",
                    "AXApplicationDeactivated",
                    "AXApplicationHidden",
                    "AXApplicationShown",
                    "AXRowCountChanged",
                    "AXSelectedRowsChanged",
                    "AXSelectedCellsChanged",
                ];
                
                for notif_name in &notifications {
                    let notif = CFString::new(notif_name);
                    AXObserverAddNotification(
                        observer,
                        app_element,
                        notif.as_concrete_TypeRef() as CFStringRef,
                        null_mut()
                    );
                }
                
                let source = AXObserverGetRunLoopSource(observer);
                CFRunLoopAddSource(
                    CFRunLoop::get_current().as_concrete_TypeRef(),
                    source,
                    kCFRunLoopDefaultMode as CFStringRef
                );
                
                self.current_ax_observer = Some(observer as usize);
                CFRelease(app_element as CFTypeRef);
            }
        }
    }

    fn setup_event_tap(&mut self) {
        unsafe {
            // Monitor mouse, keyboard, and scroll events
            let event_mask: u64 = 
                (1 << 1) | (1 << 2) | // Left mouse down/up
                (1 << 3) | (1 << 4) | // Right mouse down/up  
                (1 << 5) | (1 << 6) | (1 << 7) | // Mouse movement/dragging
                (1 << 10) | (1 << 11) | // Key down/up
                (1 << 12) | // Flags changed (modifiers)
                (1 << 22); // Scroll wheel
            
            let tap = CGEventTapCreate(
                0, // HID event tap
                0, // Head insert
                1, // Listen only
                event_mask,
                event_tap_callback,
                null_mut()
            );
            
            if !tap.is_null() {
                let source = CFMachPortCreateRunLoopSource(null_mut(), tap, 0);
                CFRunLoopAddSource(
                    CFRunLoop::get_current().as_concrete_TypeRef(),
                    source,
                    kCFRunLoopDefaultMode as CFStringRef
                );
                CGEventTapEnable(tap, true);
                self.event_tap = Some(tap);
            }
        }
    }

    // Helper methods
    fn get_attribute(&self, element: AXUIElementRef, attr_name: &str) -> Option<CFTypeRef> {
        unsafe {
            let mut value: CFTypeRef = null_mut();
            let attr = CFString::new(attr_name);
            
            if AXUIElementCopyAttributeValue(
                element,
                attr.as_concrete_TypeRef() as CFStringRef,
                &mut value,
            ) == kAXErrorSuccess && !value.is_null() {
                Some(value)
            } else {
                None
            }
        }
    }

    fn get_string_attribute(&self, element: AXUIElementRef, attr_name: &str) -> Option<String> {
        unsafe {
            if let Some(value) = self.get_attribute(element, attr_name) {
                let cfstr = CFString::wrap_under_create_rule(value as CFStringRef);
                let result = cfstr.to_string();
                if !result.is_empty() {
                    Some(result)
                } else {
                    None
                }
            } else {
                None
            }
        }
    }

    fn get_bool_attribute(&self, element: AXUIElementRef, attr_name: &str) -> Option<bool> {
        unsafe {
            if let Some(value) = self.get_attribute(element, attr_name) {
                let cfbool = value as *const _ as *const u8;
                if !cfbool.is_null() {
                    Some(*cfbool != 0)
                } else {
                    None
                }
            } else {
                None
            }
        }
    }

    fn get_number_attribute(&self, element: AXUIElementRef, attr_name: &str) -> Option<i32> {
        unsafe {
            if let Some(value) = self.get_attribute(element, attr_name) {
                // This would need proper CFNumber extraction
                None
            } else {
                None
            }
        }
    }

    fn extract_point_value(&self, _value: CFTypeRef) -> Option<(f64, f64)> {
        // Would use AXValueGetValue to extract CGPoint
        None
    }

    fn extract_size_value(&self, _value: CFTypeRef) -> Option<(f64, f64)> {
        // Would use AXValueGetValue to extract CGSize
        None
    }

    fn get_app_installation_path(&self, bundle_id: &str) -> Option<String> {
        unsafe {
            let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
            let bundle_id_str: id = msg_send![class!(NSString), alloc];
            let bundle_id_str: id = msg_send![bundle_id_str, initWithBytes:bundle_id.as_ptr() 
                                             length:bundle_id.len() 
                                             encoding:4];
            
            let url: id = msg_send![workspace, URLForApplicationWithBundleIdentifier:bundle_id_str];
            let _: () = msg_send![bundle_id_str, release];
            
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
}

// Callbacks
extern "C" fn ax_observer_callback(
    _observer: AXObserverRef,
    _element: AXUIElementRef,
    notification: CFStringRef,
    _user_data: *mut c_void,
) {
    unsafe {
        let notif_str = CFString::wrap_under_get_rule(notification).to_string();
        with_state(|tracker| {
            tracker.handle_ui_change(&notif_str);
        });
    }
}

extern "C" fn event_tap_callback(
    _proxy: *mut c_void,
    event_type: u32,
    event: *mut c_void,
    _user_info: *mut c_void,
) -> *mut c_void {
    unsafe {
        let cg_event = event as *mut CGEvent;
        
        with_state(|tracker| {
            match event_type {
                1..=4 => { // Mouse events
                    let location = (*cg_event).location();
                    let button = (*cg_event).integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER);
                    tracker.handle_mouse_event(CGEventType::from(event_type), location, button);
                }
                22 => { // Scroll
                    let delta_x = (*cg_event).double_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_2);
                    let delta_y = (*cg_event).double_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1);
                    tracker.handle_scroll_event(delta_x, delta_y);
                }
                10..=12 => { // Keyboard
                    let flags = (*cg_event).get_flags();
                    tracker.handle_key_event(flags);
                }
                _ => {}
            }
        });
    }
    
    event
}

extern "C" fn workspace_did_activate_app(_this: &Object, _cmd: Sel, notification: id) {
    unsafe {
        let user_info: id = msg_send![notification, userInfo];
        if user_info == nil { return; }

        let key_str = "NSWorkspaceApplicationKey";
        let ns_key: id = msg_send![class!(NSString), alloc];
        let ns_key: id = msg_send![ns_key, initWithBytes:key_str.as_ptr() 
                                          length:key_str.len() 
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

fn with_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut CompleteTracker) -> R,
{
    let state = STATE.get().expect("State not initialized");
    let mut tracker = state.lock().unwrap();
    f(&mut *tracker)
}

fn create_workspace_observer_class() -> *const Class {
    let superclass = class!(NSObject);
    let mut decl = ClassDecl::new("CompleteTrackerObserver", superclass).unwrap();
    
    unsafe {
        decl.add_method(
            sel!(workspaceDidActivateApp:),
            workspace_did_activate_app as extern "C" fn(&Object, Sel, id),
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

// CGEventFlags extensions
impl CGEventFlags {
    const CGEventFlagMaskCommand: CGEventFlags = CGEventFlags(1 << 20);
    const CGEventFlagMaskShift: CGEventFlags = CGEventFlags(1 << 17);
    const CGEventFlagMaskControl: CGEventFlags = CGEventFlags(1 << 18);
    const CGEventFlagMaskAlternate: CGEventFlags = CGEventFlags(1 << 19);
}

// Event field constants
mod EventField {
    pub const MOUSE_EVENT_BUTTON_NUMBER: i32 = 3;
    pub const SCROLL_WHEEL_EVENT_DELTA_AXIS_1: i32 = 11;
    pub const SCROLL_WHEEL_EVENT_DELTA_AXIS_2: i32 = 12;
}

fn main() {
    let cli = Cli::parse();
    
    if !ensure_ax_trust(!cli.no_prompt) {
        eprintln!("Accessibility access is not granted.");
        eprintln!("Please enable: System Settings → Privacy & Security → Accessibility");
        std::process::exit(1);
    }

    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        
        let app = NSApp();
        app.setActivationPolicy_(NSApplicationActivationPolicyProhibited);
        
        STATE.set(Arc::new(Mutex::new(CompleteTracker::new(cli.format == "json"))))
            .expect("Failed to initialize state");
        
        // Set up event tap for mouse/keyboard/scroll tracking
        with_state(|tracker| {
            tracker.setup_event_tap();
        });
        
        // Get initial app state
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
        
        // Create and register workspace observer
        let observer_class = create_workspace_observer_class();
        let observer: id = msg_send![observer_class, new];
        
        let notification_center: id = msg_send![workspace, notificationCenter];
        let notif_name_str = "NSWorkspaceDidActivateApplicationNotification";
        let notif_name: id = msg_send![class!(NSString), alloc];
        let notif_name: id = msg_send![notif_name, initWithBytes:notif_name_str.as_ptr()
                                                   length:notif_name_str.len()
                                                   encoding:4];
        
        let _: () = msg_send![notification_center,
            addObserver:observer
            selector:sel!(workspaceDidActivateApp:)
            name:notif_name
            object:nil
        ];
        
        let _: () = msg_send![notif_name, release];
        
        if cli.format != "json" {
            println!("Complete context tracker running...");
            println!("Tracking: apps, URLs, files, clicks, scrolls, UI elements");
        }
        
        CFRunLoopRun();
        
        let _: () = msg_send![pool, drain];
    }
}