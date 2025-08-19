#![cfg(target_os = "macos")]

use std::ffi::{c_void, CStr};
use std::ptr::null_mut;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

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
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use serde::Serialize;

/// CLI flags
#[derive(Debug, Parser)]
#[command(name = "focus-track-enhanced", version, about = "Track detailed app context and user interactions on macOS")]
struct Cli {
    /// Output format: text or json
    #[arg(long, default_value = "json", value_parser = ["text", "json"])]
    format: String,

    /// Do not show the Accessibility permission prompt if not trusted
    #[arg(long)]
    no_prompt: bool,
    
    /// Enable mouse tracking (clicks, position)
    #[arg(long)]
    track_mouse: bool,
    
    /// Enable scroll tracking
    #[arg(long)]
    track_scroll: bool,
}

#[derive(Debug, Clone, Serialize)]
struct DetailedContext {
    // Application info
    app_name: String,
    bundle_id: String,
    pid: i32,
    app_path: Option<String>,
    
    // Window info
    window_title: Option<String>,
    window_position: Option<(f64, f64)>,
    window_size: Option<(f64, f64)>,
    
    // Document/file info
    document_path: Option<String>,
    document_modified: Option<bool>,
    
    // UI element details
    focused_element: Option<UIElementInfo>,
    ui_hierarchy: Vec<UIElementInfo>,
    
    // Interaction state
    last_click_position: Option<(f64, f64)>,
    #[serde(skip)]
    last_click_time: Option<Instant>,
    scroll_position: Option<f64>,
    
    // Timing
    #[serde(skip)]
    started_at: Instant,
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
}

#[derive(Debug, Clone, Serialize)]
struct ContextEvent {
    event_type: String,
    timestamp: u128,
    from_context: Option<DetailedContext>,
    to_context: DetailedContext,
    duration_ms: Option<u128>,
    trigger: Option<String>, // What caused the change
}

#[derive(Debug)]
struct EnhancedFocusTracker {
    current_context: Option<DetailedContext>,
    json_output: bool,
    current_ax_observer: Option<usize>,
    event_tap: Option<usize>,
    start_time: Instant,
}

static STATE: OnceLock<Arc<Mutex<EnhancedFocusTracker>>> = OnceLock::new();

impl EnhancedFocusTracker {
    fn new(json_output: bool) -> Self {
        Self {
            current_context: None,
            json_output,
            current_ax_observer: None,
            event_tap: None,
            start_time: Instant::now(),
        }
    }

    fn extract_ui_element_info(&self, element: AXUIElementRef) -> UIElementInfo {
        unsafe {
            let mut info = UIElementInfo {
                role: None,
                title: None,
                value: None,
                description: None,
                help: None,
                position: None,
                size: None,
                enabled: None,
                focused: None,
                selected: None,
                identifier: None,
                url: None,
            };

            // Extract role
            info.role = self.get_string_attribute(element, "AXRole");
            
            // Extract text attributes
            info.title = self.get_string_attribute(element, "AXTitle");
            info.value = self.get_string_attribute(element, "AXValue");
            info.description = self.get_string_attribute(element, "AXDescription");
            info.help = self.get_string_attribute(element, "AXHelp");
            info.identifier = self.get_string_attribute(element, "AXIdentifier");
            
            // Extract URL if available (for web content)
            info.url = self.get_string_attribute(element, "AXURL");
            
            // Extract position
            if let Some(pos_ref) = self.get_attribute(element, "AXPosition") {
                if let Some(point) = self.extract_point(pos_ref) {
                    info.position = Some(point);
                }
                CFRelease(pos_ref);
            }
            
            // Extract size
            if let Some(size_ref) = self.get_attribute(element, "AXSize") {
                if let Some(size) = self.extract_size(size_ref) {
                    info.size = Some(size);
                }
                CFRelease(size_ref);
            }
            
            // Extract boolean states
            info.enabled = self.get_bool_attribute(element, "AXEnabled");
            info.focused = self.get_bool_attribute(element, "AXFocused");
            info.selected = self.get_bool_attribute(element, "AXSelected");
            
            info
        }
    }

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
                // Check if it's a boolean
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

    fn extract_point(&self, value: CFTypeRef) -> Option<(f64, f64)> {
        // AXValueGetValue would be used here to extract CGPoint
        // For now, returning a placeholder
        None
    }

    fn extract_size(&self, value: CFTypeRef) -> Option<(f64, f64)> {
        // AXValueGetValue would be used here to extract CGSize
        None
    }

    fn get_focused_ui_element(&self, app_element: AXUIElementRef) -> Option<UIElementInfo> {
        unsafe {
            if let Some(focused_ref) = self.get_attribute(app_element, "AXFocusedUIElement") {
                let info = self.extract_ui_element_info(focused_ref as AXUIElementRef);
                CFRelease(focused_ref);
                Some(info)
            } else {
                None
            }
        }
    }

    fn get_ui_hierarchy(&self, app_element: AXUIElementRef, max_depth: usize) -> Vec<UIElementInfo> {
        let mut hierarchy = Vec::new();
        self.traverse_ui_tree(app_element, &mut hierarchy, 0, max_depth);
        hierarchy
    }

    fn traverse_ui_tree(&self, element: AXUIElementRef, hierarchy: &mut Vec<UIElementInfo>, depth: usize, max_depth: usize) {
        if depth >= max_depth {
            return;
        }

        unsafe {
            let info = self.extract_ui_element_info(element);
            hierarchy.push(info);

            // Get children
            if let Some(children_ref) = self.get_attribute(element, "AXChildren") {
                // Cast to CFArray and iterate
                // This would need proper CFArray handling
                CFRelease(children_ref);
            }
        }
    }

    fn get_app_installation_path(&self, bundle_id: &str) -> Option<String> {
        unsafe {
            let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
            let bundle_id_str: id = msg_send![class!(NSString), alloc];
            let bundle_id_str: id = msg_send![bundle_id_str, initWithBytes:bundle_id.as_ptr() 
                                             length:bundle_id.len() 
                                             encoding:4]; // UTF8
            
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

    fn get_finder_path(&self, window_element: AXUIElementRef) -> Option<String> {
        // For Finder, try to get the path from the window's document attribute
        unsafe {
            // First check if this is Finder
            if let Some(doc) = self.get_string_attribute(window_element, "AXDocument") {
                // Document often contains the file:// URL for Finder windows
                if doc.starts_with("file://") {
                    let path = doc.strip_prefix("file://").unwrap();
                    return Some(urlencoding::decode(path).unwrap_or_else(|_| path.to_string().into()).to_string());
                }
            }
            
            // Try to get URL attribute
            if let Some(url) = self.get_string_attribute(window_element, "AXURL") {
                if url.starts_with("file://") {
                    let path = url.strip_prefix("file://").unwrap();
                    return Some(urlencoding::decode(path).unwrap_or_else(|_| path.to_string().into()).to_string());
                }
            }
            
            None
        }
    }

    fn get_document_path(&self, app_element: AXUIElementRef) -> Option<String> {
        unsafe {
            // Try to get the document path from the focused window
            if let Some(window_ref) = self.get_attribute(app_element, "AXFocusedWindow") {
                let window = window_ref as AXUIElementRef;
                
                // Check for document attribute
                let doc_path = self.get_string_attribute(window, "AXDocument");
                
                CFRelease(window_ref);
                doc_path
            } else {
                None
            }
        }
    }

    fn extract_detailed_context(&self, app_name: String, bundle_id: String, pid: i32) -> DetailedContext {
        unsafe {
            let app_element = AXUIElementCreateApplication(pid);
            
            let mut context = DetailedContext {
                app_name: app_name.clone(),
                bundle_id: bundle_id.clone(),
                pid,
                app_path: self.get_app_installation_path(&bundle_id),
                window_title: None,
                window_position: None,
                window_size: None,
                document_path: None,
                document_modified: None,
                focused_element: None,
                ui_hierarchy: Vec::new(),
                last_click_position: None,
                last_click_time: None,
                scroll_position: None,
                started_at: Instant::now(),
            };

            // Get window information
            if let Some(window_ref) = self.get_attribute(app_element, "AXFocusedWindow") {
                let window = window_ref as AXUIElementRef;
                
                context.window_title = self.get_string_attribute(window, "AXTitle");
                
                // Get window position and size
                if let Some(pos_ref) = self.get_attribute(window, "AXPosition") {
                    context.window_position = self.extract_point(pos_ref);
                    CFRelease(pos_ref);
                }
                
                if let Some(size_ref) = self.get_attribute(window, "AXSize") {
                    context.window_size = self.extract_size(size_ref);
                    CFRelease(size_ref);
                }
                
                // Special handling for Finder
                if app_name == "Finder" {
                    context.document_path = self.get_finder_path(window);
                } else {
                    // Try to get document path for other apps
                    context.document_path = self.get_string_attribute(window, "AXDocument");
                }
                
                // Check if document is modified
                context.document_modified = self.get_bool_attribute(window, "AXIsDocumentEdited");
                
                CFRelease(window_ref);
            }
            
            // Get focused UI element information
            context.focused_element = self.get_focused_ui_element(app_element);
            
            // Get a shallow UI hierarchy (limit depth for performance)
            context.ui_hierarchy = self.get_ui_hierarchy(app_element, 3);
            
            CFRelease(app_element as CFTypeRef);
            
            context
        }
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

        let new_context = self.extract_detailed_context(new_app_name, new_bundle_id, new_pid);
        
        if let Some(ref old_context) = self.current_context {
            if old_context.app_name != new_context.app_name {
                let event = ContextEvent {
                    event_type: "app_switch".to_string(),
                    timestamp: self.start_time.elapsed().as_millis(),
                    from_context: Some(old_context.clone()),
                    to_context: new_context.clone(),
                    duration_ms: Some(old_context.started_at.elapsed().as_millis()),
                    trigger: Some("user_action".to_string()),
                };

                self.log_event(event);
            }
        } else if !self.json_output {
            println!("Started tracking enhanced context...");
            println!("Current app: {} at {:?}", new_context.app_name, new_context.app_path);
        }

        // Set up AX observer for the new app
        self.setup_ax_observer_for_app(new_pid);
        self.current_context = Some(new_context);
    }

    fn handle_ui_change(&mut self, notification_type: &str) {
        if let Some(context) = self.current_context.take() {
            let old_context = context.clone();
            
            // Re-extract context to get updated information
            let new_context = self.extract_detailed_context(
                context.app_name.clone(),
                context.bundle_id.clone(),
                context.pid,
            );
            
            // Preserve interaction state
            let mut updated_context = new_context;
            updated_context.last_click_position = context.last_click_position;
            updated_context.last_click_time = context.last_click_time;
            updated_context.scroll_position = context.scroll_position;
            
            let event = ContextEvent {
                event_type: "ui_change".to_string(),
                timestamp: self.start_time.elapsed().as_millis(),
                from_context: Some(old_context.clone()),
                to_context: updated_context.clone(),
                duration_ms: Some(old_context.started_at.elapsed().as_millis()),
                trigger: Some(notification_type.to_string()),
            };
            
            self.log_event(event);
            self.current_context = Some(updated_context);
        }
    }

    fn handle_mouse_event(&mut self, x: f64, y: f64, event_type: &str) {
        if let Some(ref mut context) = self.current_context {
            context.last_click_position = Some((x, y));
            context.last_click_time = Some(Instant::now());
            
            let event = ContextEvent {
                event_type: "interaction".to_string(),
                timestamp: self.start_time.elapsed().as_millis(),
                from_context: None,
                to_context: context.clone(),
                duration_ms: None,
                trigger: Some(format!("{}_at_{},{}", event_type, x, y)),
            };
            
            self.log_event(event);
        }
    }

    fn handle_scroll_event(&mut self, delta: f64) {
        if let Some(ref mut context) = self.current_context {
            context.scroll_position = Some(context.scroll_position.unwrap_or(0.0) + delta);
            
            // Log scroll events in batches to avoid spam
            // This is simplified - in production you'd want to batch these
        }
    }

    fn log_event(&self, event: ContextEvent) {
        if self.json_output {
            println!("{}", serde_json::to_string(&event).unwrap());
        } else {
            println!("[{}] {} → {} (trigger: {:?})", 
                event.event_type,
                event.from_context.as_ref().map(|c| c.app_name.as_str()).unwrap_or("none"),
                event.to_context.app_name,
                event.trigger
            );
            
            if let Some(path) = &event.to_context.document_path {
                println!("  Document: {}", path);
            }
            if let Some(elem) = &event.to_context.focused_element {
                println!("  Focused: {:?} - {:?}", elem.role, elem.title);
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
                    "AXTitleChanged",
                    "AXFocusedUIElementChanged",
                    "AXValueChanged",
                    "AXSelectedChildrenChanged",
                    "AXSelectedTextChanged",
                    "AXMenuItemSelected",
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
}

// AX Observer callback
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

// Helper function to access global state
fn with_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut EnhancedFocusTracker) -> R,
{
    let state = STATE.get().expect("State not initialized");
    let mut tracker = state.lock().unwrap();
    f(&mut *tracker)
}

// NSWorkspace observer
extern "C" fn workspace_did_activate_app(_this: &Object, _cmd: Sel, notification: id) {
    unsafe {
        let user_info: id = msg_send![notification, userInfo];
        if user_info == nil {
            return;
        }

        let key_str = "NSWorkspaceApplicationKey";
        let ns_key: id = msg_send![class!(NSString), alloc];
        let ns_key: id = msg_send![ns_key, initWithBytes:key_str.as_ptr() 
                                           length:key_str.len() 
                                           encoding:4];
        
        let app: id = msg_send![user_info, objectForKey:ns_key];
        let _: () = msg_send![ns_key, release];
        
        if app == nil {
            return;
        }

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

fn create_workspace_observer_class() -> *const Class {
    let superclass = class!(NSObject);
    let mut decl = ClassDecl::new("EnhancedFocusTrackerObserver", superclass).unwrap();
    
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

fn main() {
    let cli = Cli::parse();
    
    if !ensure_ax_trust(!cli.no_prompt) {
        eprintln!("Accessibility access is not granted.");
        eprintln!("Please enable: System Settings → Privacy & Security → Accessibility → allow this app.");
        std::process::exit(1);
    }

    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        
        let app = NSApp();
        app.setActivationPolicy_(NSApplicationActivationPolicyProhibited);
        
        STATE.set(Arc::new(Mutex::new(EnhancedFocusTracker::new(cli.format == "json"))))
            .expect("Failed to initialize state");
        
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
        
        if !cli.format.contains("json") {
            println!("Monitoring enhanced context and user interactions (press Ctrl+C to stop)...");
        }
        
        CFRunLoopRun();
        
        let _: () = msg_send![pool, drain];
    }
}