#![cfg(target_os = "macos")]

use std::collections::HashMap;
use std::ffi::{c_void, CStr};
use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use std::process::Command;

use accessibility_sys::*;
use clap::Parser;
use cocoa::appkit::{NSApp, NSApplication, NSApplicationActivationPolicyProhibited};
use cocoa::base::{id, nil};
use cocoa::foundation::{NSAutoreleasePool, NSData, NSString};
use core_foundation::array::CFArray;
use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::runloop::{CFRunLoop, CFRunLoopRun, kCFRunLoopDefaultMode};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation_sys::runloop::{CFRunLoopAddSource, CFRunLoopRemoveSource};
use core_foundation_sys::base::{CFGetTypeID, CFTypeID, CFIndex};
use core_foundation_sys::string::CFStringGetTypeID;
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use serde::{Serialize, Deserialize};
use serde_json::json;

// External functions for extended functionality
extern "C" {
    // Accessibility API extensions
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
    
    // CGEvent for mouse/keyboard monitoring
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
    
    // File system events
    fn FSEventStreamCreate(
        allocator: *mut c_void,
        callback: extern "C" fn(*mut c_void, *mut c_void, usize, *mut *const i8, *const u32, *const u64),
        context: *mut c_void,
        paths: CFTypeRef,
        since_when: u64,
        latency: f64,
        flags: u32
    ) -> *mut c_void;
    
    fn FSEventStreamSetDispatchQueue(stream: *mut c_void, queue: *mut c_void);
    fn FSEventStreamStart(stream: *mut c_void) -> bool;
    fn FSEventStreamStop(stream: *mut c_void);
}

#[derive(Debug, Parser)]
#[command(name = "ultimate-monitor", about = "Ultimate macOS activity monitor")]
struct Cli {
    #[arg(long, default_value = "json")]
    format: String,
    
    #[arg(long)]
    no_prompt: bool,
    
    #[arg(long)]
    monitor_clipboard: bool,
    
    #[arg(long)]
    monitor_network: bool,
    
    #[arg(long)]
    monitor_files: bool,
    
    #[arg(long)]
    deep_chrome_inspect: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UltimateContext {
    // Application basics
    app_name: String,
    bundle_id: String,
    pid: i32,
    app_path: Option<String>,
    
    // Window and document
    window_title: Option<String>,
    document_path: Option<String>,
    document_modified: Option<bool>,
    
    // Web content
    url: Option<String>,
    page_title: Option<String>,
    cookies: Option<Vec<Cookie>>,
    
    // IDE and development
    active_file: Option<String>,
    project: Option<String>,
    git_branch: Option<String>,
    open_files: Vec<String>,
    
    // Terminal
    terminal_tab: Option<String>,
    terminal_cwd: Option<String>,
    terminal_command: Option<String>,
    
    // Finder
    finder_path: Option<String>,
    selected_files: Vec<String>,
    
    // Clipboard
    clipboard_text: Option<String>,
    clipboard_type: Option<String>,
    
    // Network activity
    active_connections: Vec<NetworkConnection>,
    
    // File system activity
    recent_file_changes: Vec<FileChange>,
    
    // UI details
    focused_element: Option<UIElement>,
    ui_hierarchy: Vec<String>,
    
    // Chrome DevTools data
    chrome_tabs: Vec<ChromeTab>,
    console_logs: Vec<String>,
    
    // System
    timestamp: u128,
    duration_ms: Option<u128>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cookie {
    name: String,
    value: String,
    domain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NetworkConnection {
    host: String,
    port: u16,
    state: String,
    bytes_sent: u64,
    bytes_received: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileChange {
    path: String,
    change_type: String,
    timestamp: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UIElement {
    role: Option<String>,
    title: Option<String>,
    value: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChromeTab {
    id: u32,
    url: String,
    title: String,
    active: bool,
}

struct UltimateMonitor {
    current_context: Option<UltimateContext>,
    json_output: bool,
    start_time: Instant,
    clipboard_monitor: Option<ClipboardMonitor>,
    network_monitor: Option<NetworkMonitor>,
    file_monitor: Option<FileMonitor>,
    chrome_debugger: Option<ChromeDebugger>,
}

struct ClipboardMonitor;
struct NetworkMonitor;
struct FileMonitor;
struct ChromeDebugger;

impl ClipboardMonitor {
    fn new() -> Self {
        Self
    }
    
    fn get_clipboard_content(&self) -> (Option<String>, Option<String>) {
        unsafe {
            let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            let types: id = msg_send![pasteboard, types];
            
            if types != nil {
                // Check for text
                let string_type: id = msg_send![class!(NSString), stringWithUTF8String: "public.utf8-plain-text".as_ptr()];
                let has_string: bool = msg_send![types, containsObject: string_type];
                
                if has_string {
                    let string: id = msg_send![pasteboard, stringForType: string_type];
                    if string != nil {
                        let c_str: *const i8 = msg_send![string, UTF8String];
                        let content = CStr::from_ptr(c_str).to_string_lossy().to_string();
                        return (Some(content), Some("text".to_string()));
                    }
                }
                
                // Check for files
                let file_type: id = msg_send![class!(NSString), stringWithUTF8String: "public.file-url".as_ptr()];
                let has_file: bool = msg_send![types, containsObject: file_type];
                
                if has_file {
                    let urls: id = msg_send![pasteboard, readObjectsForClasses: nil options: nil];
                    if urls != nil {
                        // Extract file paths
                        return (Some("Files copied".to_string()), Some("files".to_string()));
                    }
                }
            }
            
            (None, None)
        }
    }
}

impl NetworkMonitor {
    fn new() -> Self {
        Self
    }
    
    fn get_app_connections(&self, pid: i32) -> Vec<NetworkConnection> {
        // Use lsof to get network connections for a specific PID
        let output = Command::new("lsof")
            .args(&["-i", "-n", "-P", "-p", &pid.to_string()])
            .output()
            .ok();
        
        let mut connections = Vec::new();
        
        if let Some(output) = output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines().skip(1) {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 9 && parts[7].contains("->") {
                        let conn_parts: Vec<&str> = parts[8].split("->").collect();
                        if conn_parts.len() == 2 {
                            let dest_parts: Vec<&str> = conn_parts[1].split(':').collect();
                            if dest_parts.len() == 2 {
                                connections.push(NetworkConnection {
                                    host: dest_parts[0].to_string(),
                                    port: dest_parts[1].parse().unwrap_or(0),
                                    state: parts[9].to_string(),
                                    bytes_sent: 0,
                                    bytes_received: 0,
                                });
                            }
                        }
                    }
                }
            }
        }
        
        connections
    }
}

impl FileMonitor {
    fn new() -> Self {
        Self
    }
    
    fn monitor_path(&self, path: &str) -> Vec<FileChange> {
        // This would use FSEvents API to monitor file changes
        // For now, return empty vector
        Vec::new()
    }
}

impl ChromeDebugger {
    fn new() -> Self {
        Self
    }
    
    fn get_chrome_tabs(&self) -> Vec<ChromeTab> {
        // Try to connect to Chrome DevTools Protocol
        // This requires Chrome to be started with --remote-debugging-port=9222
        let output = Command::new("curl")
            .args(&["-s", "http://localhost:9222/json/list"])
            .output()
            .ok();
        
        let mut tabs = Vec::new();
        
        if let Some(output) = output {
            if output.status.success() {
                if let Ok(json_tabs) = serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout) {
                    for tab in json_tabs {
                        if let (Some(id), Some(url), Some(title)) = (
                            tab["id"].as_str(),
                            tab["url"].as_str(),
                            tab["title"].as_str()
                        ) {
                            tabs.push(ChromeTab {
                                id: tabs.len() as u32,
                                url: url.to_string(),
                                title: title.to_string(),
                                active: tab["active"].as_bool().unwrap_or(false),
                            });
                        }
                    }
                }
            }
        }
        
        tabs
    }
    
    fn get_console_logs(&self) -> Vec<String> {
        // Would connect to Chrome DevTools and retrieve console logs
        Vec::new()
    }
}

impl UltimateMonitor {
    fn new(cli: &Cli) -> Self {
        let clipboard = if cli.monitor_clipboard {
            Some(ClipboardMonitor::new())
        } else {
            None
        };
        
        let network = if cli.monitor_network {
            Some(NetworkMonitor::new())
        } else {
            None
        };
        
        let files = if cli.monitor_files {
            Some(FileMonitor::new())
        } else {
            None
        };
        
        let chrome = if cli.deep_chrome_inspect {
            Some(ChromeDebugger::new())
        } else {
            None
        };
        
        Self {
            current_context: None,
            json_output: cli.format == "json",
            start_time: Instant::now(),
            clipboard_monitor: clipboard,
            network_monitor: network,
            file_monitor: files,
            chrome_debugger: chrome,
        }
    }
    
    fn extract_ultimate_context(&mut self, app_name: String, bundle_id: String, pid: i32) -> UltimateContext {
        unsafe {
            let app_element = AXUIElementCreateApplication(pid);
            
            let mut ctx = UltimateContext {
                app_name: app_name.clone(),
                bundle_id: bundle_id.clone(),
                pid,
                app_path: self.get_app_path(&bundle_id),
                window_title: None,
                document_path: None,
                document_modified: None,
                url: None,
                page_title: None,
                cookies: None,
                active_file: None,
                project: None,
                git_branch: None,
                open_files: Vec::new(),
                terminal_tab: None,
                terminal_cwd: None,
                terminal_command: None,
                finder_path: None,
                selected_files: Vec::new(),
                clipboard_text: None,
                clipboard_type: None,
                active_connections: Vec::new(),
                recent_file_changes: Vec::new(),
                focused_element: None,
                ui_hierarchy: Vec::new(),
                chrome_tabs: Vec::new(),
                console_logs: Vec::new(),
                timestamp: self.start_time.elapsed().as_millis(),
                duration_ms: None,
            };
            
            // Get clipboard content
            if let Some(ref clipboard) = self.clipboard_monitor {
                let (content, ctype) = clipboard.get_clipboard_content();
                ctx.clipboard_text = content;
                ctx.clipboard_type = ctype;
            }
            
            // Get network connections
            if let Some(ref network) = self.network_monitor {
                ctx.active_connections = network.get_app_connections(pid);
            }
            
            // Get Chrome DevTools data
            if let Some(ref chrome) = self.chrome_debugger {
                if bundle_id.contains("chrome") || bundle_id.contains("Chrome") {
                    ctx.chrome_tabs = chrome.get_chrome_tabs();
                    ctx.console_logs = chrome.get_console_logs();
                }
            }
            
            // Deep mine accessibility tree
            if let Some(window_ref) = self.get_attribute(app_element, "AXFocusedWindow") {
                let window = window_ref as AXUIElementRef;
                
                ctx.window_title = self.get_string_attr(window, "AXTitle");
                ctx.document_modified = self.get_bool_attr(window, "AXIsDocumentEdited");
                
                // Extract based on app type
                if bundle_id.contains("chrome") || bundle_id.contains("Chrome") || 
                   bundle_id.contains("safari") || bundle_id.contains("firefox") {
                    // Browser extraction
                    ctx.url = self.deep_extract_browser_content(window);
                    ctx.page_title = ctx.window_title.clone();
                    
                    // Try to get cookies from Chrome if debugging enabled
                    if let Some(ref chrome) = self.chrome_debugger {
                        // Would extract cookies here
                    }
                }
                
                if app_name.contains("Code") || app_name.contains("Cursor") {
                    // IDE extraction
                    ctx.active_file = self.deep_extract_ide_content(window);
                    ctx.project = self.extract_project_info(&app_name, &ctx.window_title);
                    ctx.git_branch = self.get_git_branch(&ctx.project);
                    ctx.open_files = self.extract_open_files(window);
                }
                
                if app_name.contains("Terminal") || app_name.contains("iTerm") {
                    // Terminal extraction
                    ctx.terminal_tab = ctx.window_title.clone();
                    ctx.terminal_cwd = self.extract_terminal_cwd(window);
                    ctx.terminal_command = self.extract_terminal_command(window);
                }
                
                if app_name == "Finder" {
                    // Finder extraction
                    ctx.finder_path = self.extract_finder_path(window);
                    ctx.selected_files = self.extract_finder_selection(window);
                }
                
                // Get focused element details
                if let Some(focused_ref) = self.get_attribute(app_element, "AXFocusedUIElement") {
                    let focused = focused_ref as AXUIElementRef;
                    ctx.focused_element = Some(self.extract_ui_element(focused));
                    ctx.ui_hierarchy = self.build_ui_hierarchy(focused);
                    CFRelease(focused_ref);
                }
                
                CFRelease(window_ref);
            }
            
            CFRelease(app_element as CFTypeRef);
            ctx
        }
    }
    
    fn deep_extract_browser_content(&self, window: AXUIElementRef) -> Option<String> {
        unsafe {
            // Try multiple methods to get URL
            
            // Method 1: Direct URL attribute
            if let Some(url) = self.get_string_attr(window, "AXURL") {
                return Some(url);
            }
            
            // Method 2: Search for web area
            if let Some(web_area) = self.find_web_area(window) {
                if let Some(url) = self.get_string_attr(web_area, "AXURL") {
                    CFRelease(web_area as CFTypeRef);
                    return Some(url);
                }
                CFRelease(web_area as CFTypeRef);
            }
            
            // Method 3: Search address bar
            if let Some(address_bar) = self.find_address_bar(window) {
                if let Some(url) = self.get_string_attr(address_bar, "AXValue") {
                    CFRelease(address_bar as CFTypeRef);
                    return Some(url);
                }
                CFRelease(address_bar as CFTypeRef);
            }
            
            // Method 4: Parse from window title
            if let Some(title) = self.get_string_attr(window, "AXTitle") {
                if title.contains("http://") || title.contains("https://") {
                    if let Some(url_start) = title.find("http") {
                        let url_part: String = title.chars().skip(url_start).collect();
                        return Some(url_part);
                    }
                }
            }
            
            None
        }
    }
    
    fn deep_extract_ide_content(&self, window: AXUIElementRef) -> Option<String> {
        unsafe {
            // Try multiple methods to get file path
            
            // Method 1: Document attribute
            if let Some(doc) = self.get_string_attr(window, "AXDocument") {
                if doc.starts_with("file://") {
                    return Some(urlencoding::decode(&doc[7..])
                        .unwrap_or_else(|_| doc[7..].into())
                        .to_string());
                } else if doc.starts_with("/") {
                    return Some(doc);
                }
            }
            
            // Method 2: Mine all attributes
            let attrs = self.mine_all_attributes(window);
            for (key, value) in attrs {
                if key.contains("Path") || key.contains("File") || key.contains("Document") {
                    if value.starts_with("/") {
                        return Some(value);
                    }
                }
            }
            
            // Method 3: Search tabs
            if let Some(tab_group) = self.find_tab_group(window) {
                if let Some(path) = self.extract_tab_path(tab_group) {
                    CFRelease(tab_group as CFTypeRef);
                    return Some(path);
                }
                CFRelease(tab_group as CFTypeRef);
            }
            
            None
        }
    }
    
    fn extract_terminal_cwd(&self, window: AXUIElementRef) -> Option<String> {
        // Would extract current working directory from terminal
        // This might require reading the terminal content or using other methods
        None
    }
    
    fn extract_terminal_command(&self, window: AXUIElementRef) -> Option<String> {
        // Would extract current/last command from terminal
        None
    }
    
    fn extract_finder_path(&self, window: AXUIElementRef) -> Option<String> {
        unsafe {
            // Try to get current folder path
            self.get_string_attr(window, "AXDocument")
                .or_else(|| self.get_string_attr(window, "AXURL"))
                .map(|path| {
                    if path.starts_with("file://") {
                        urlencoding::decode(&path[7..])
                            .unwrap_or_else(|_| path[7..].into())
                            .to_string()
                    } else {
                        path
                    }
                })
        }
    }
    
    fn extract_finder_selection(&self, window: AXUIElementRef) -> Vec<String> {
        // Implementation from previous code
        Vec::new()
    }
    
    fn extract_project_info(&self, app_name: &str, window_title: &Option<String>) -> Option<String> {
        if let Some(title) = window_title {
            // Parse project name from window title
            let parts: Vec<&str> = title.split(" — ").collect();
            if parts.len() >= 2 {
                return Some(parts[1].to_string());
            }
        }
        None
    }
    
    fn get_git_branch(&self, project: &Option<String>) -> Option<String> {
        // Would execute git command to get current branch
        None
    }
    
    fn extract_open_files(&self, window: AXUIElementRef) -> Vec<String> {
        // Would extract list of open files from IDE tabs
        Vec::new()
    }
    
    fn find_web_area(&self, element: AXUIElementRef) -> Option<AXUIElementRef> {
        // Search for web area in accessibility tree
        None
    }
    
    fn find_address_bar(&self, element: AXUIElementRef) -> Option<AXUIElementRef> {
        // Search for address bar text field
        None
    }
    
    fn find_tab_group(&self, element: AXUIElementRef) -> Option<AXUIElementRef> {
        // Search for tab group in accessibility tree
        None
    }
    
    fn extract_tab_path(&self, tab_group: AXUIElementRef) -> Option<String> {
        // Extract file path from selected tab
        None
    }
    
    fn extract_ui_element(&self, element: AXUIElementRef) -> UIElement {
        UIElement {
            role: self.get_string_attr(element, "AXRole"),
            title: self.get_string_attr(element, "AXTitle"),
            value: self.get_string_attr(element, "AXValue"),
            url: self.get_string_attr(element, "AXURL"),
        }
    }
    
    fn build_ui_hierarchy(&self, element: AXUIElementRef) -> Vec<String> {
        let mut hierarchy = Vec::new();
        unsafe {
            let mut current = element;
            for _ in 0..10 {
                if let Some(role) = self.get_string_attr(current, "AXRole") {
                    hierarchy.push(role);
                }
                
                if let Some(parent_ref) = self.get_attribute(current, "AXParent") {
                    current = parent_ref as AXUIElementRef;
                    // Don't release parent as we're still using it
                } else {
                    break;
                }
            }
        }
        hierarchy.reverse();
        hierarchy
    }
    
    fn mine_all_attributes(&self, element: AXUIElementRef) -> HashMap<String, String> {
        let mut attrs = HashMap::new();
        
        let attribute_names = [
            "AXRole", "AXRoleDescription", "AXTitle", "AXDescription",
            "AXValue", "AXHelp", "AXURL", "AXDocument", "AXFilename",
            "AXPath", "AXIdentifier", "AXLabel", "AXPlaceholderValue",
            "AXSelectedText", "AXSelectedTextRange", "AXNumberOfCharacters",
            "AXVisibleCharacterRange", "AXInsertionPointLineNumber",
            "AXMenuItemCmdChar", "AXMenuItemCmdVirtualKey",
        ];
        
        for attr_name in &attribute_names {
            if let Some(value) = self.get_string_attr(element, attr_name) {
                attrs.insert(attr_name.to_string(), value);
            }
        }
        
        attrs
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
                let result = !ptr.is_null() && *ptr != 0;
                CFRelease(value);
                result
            })
        }
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
    
    fn emit_event(&self, context: &UltimateContext) {
        if self.json_output {
            println!("{}", serde_json::to_string(context).unwrap());
        } else {
            println!("\n╔══════════════════════════════════════════════════════╗");
            println!("║ {} ({}) ", context.app_name, context.bundle_id);
            println!("╚══════════════════════════════════════════════════════╝");
            
            if let Some(ref title) = context.window_title {
                println!("📋 Window: {}", title);
            }
            
            if let Some(ref url) = context.url {
                println!("🌐 URL: {}", url);
            }
            
            if let Some(ref file) = context.active_file {
                println!("📄 File: {}", file);
            }
            
            if let Some(ref project) = context.project {
                println!("📁 Project: {}", project);
            }
            
            if let Some(ref branch) = context.git_branch {
                println!("🌿 Git Branch: {}", branch);
            }
            
            if let Some(ref path) = context.finder_path {
                println!("📂 Finder: {}", path);
            }
            
            if !context.selected_files.is_empty() {
                println!("📎 Selected: {}", context.selected_files.join(", "));
            }
            
            if let Some(ref clipboard) = context.clipboard_text {
                println!("📋 Clipboard ({}): {}", 
                    context.clipboard_type.as_ref().unwrap_or(&"unknown".to_string()),
                    if clipboard.len() > 50 {
                        format!("{}...", &clipboard[..50])
                    } else {
                        clipboard.clone()
                    }
                );
            }
            
            if !context.active_connections.is_empty() {
                println!("🌐 Network Connections:");
                for conn in &context.active_connections {
                    println!("   → {}:{} ({})", conn.host, conn.port, conn.state);
                }
            }
            
            if !context.chrome_tabs.is_empty() {
                println!("🔖 Chrome Tabs:");
                for tab in &context.chrome_tabs {
                    println!("   {} {} - {}", 
                        if tab.active { "▶" } else { " " },
                        tab.title,
                        tab.url
                    );
                }
            }
            
            if let Some(ref elem) = context.focused_element {
                if let Some(ref role) = elem.role {
                    println!("🎯 Focused: {} - {}", 
                        role, 
                        elem.value.as_ref().unwrap_or(&"".to_string())
                    );
                }
            }
        }
    }
}

static STATE: OnceLock<Arc<Mutex<UltimateMonitor>>> = OnceLock::new();

fn with_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut UltimateMonitor) -> R,
{
    let state = STATE.get().expect("State not initialized");
    let mut monitor = state.lock().unwrap();
    f(&mut *monitor)
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
            let c_str: *const i8 = msg_send![bundle_id, UTF8String];
            CStr::from_ptr(c_str).to_string_lossy().to_string()
        } else {
            "unknown".to_string()
        };
        
        let name_str = if name != nil {
            let c_str: *const i8 = msg_send![name, UTF8String];
            CStr::from_ptr(c_str).to_string_lossy().to_string()
        } else {
            "Unknown".to_string()
        };
        
        with_state(|monitor| {
            let context = monitor.extract_ultimate_context(name_str, bundle_str, pid);
            monitor.emit_event(&context);
            monitor.current_context = Some(context);
        });
    }
}

fn main() {
    let cli = Cli::parse();
    
    unsafe {
        let pool: id = NSAutoreleasePool::new(nil);
        
        let monitor = UltimateMonitor::new(&cli);
        STATE.set(Arc::new(Mutex::new(monitor)))
            .expect("Failed to initialize state");
        
        // Setup workspace observer
        let observer_class = create_observer_class();
        let observer: id = msg_send![observer_class, alloc];
        let observer: id = msg_send![observer, init];
        
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let nc: id = msg_send![workspace, notificationCenter];
        
        let notif_name: id = msg_send![class!(NSString), alloc];
        let notif_name: id = msg_send![notif_name, 
            initWithBytes:"NSWorkspaceDidActivateApplicationNotification".as_ptr()
            length:"NSWorkspaceDidActivateApplicationNotification".len()
            encoding:4];
        
        let _: () = msg_send![nc,
            addObserver:observer
            selector:sel!(workspaceDidActivateApp:)
            name:notif_name
            object:nil
        ];
        
        let _: () = msg_send![notif_name, release];
        
        if !cli.json_output {
            println!("🚀 Ultimate Monitor Started!");
            println!("Monitoring: Apps ✓ | Clipboard {} | Network {} | Files {} | Chrome Debug {}",
                if cli.monitor_clipboard { "✓" } else { "✗" },
                if cli.monitor_network { "✓" } else { "✗" },
                if cli.monitor_files { "✓" } else { "✗" },
                if cli.deep_chrome_inspect { "✓" } else { "✗" }
            );
        }
        
        CFRunLoopRun();
        
        let _: () = msg_send![pool, drain];
    }
}

fn create_observer_class() -> *const Class {
    let superclass = class!(NSObject);
    let mut decl = ClassDecl::new("WorkspaceObserver", superclass).unwrap();
    
    unsafe {
        decl.add_method(
            sel!(workspaceDidActivateApp:),
            workspace_callback as extern "C" fn(&Object, Sel, id),
        );
    }
    
    decl.register()
}