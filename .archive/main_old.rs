#![cfg(target_os = "macos")]

use std::ffi::{c_void, CStr};
use std::ptr::null_mut;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use accessibility_sys::*;
use clap::Parser;
use cocoa::appkit::{NSApp, NSApplication, NSApplicationActivationPolicyRegular, NSRunningApplication};
use cocoa::base::{id, nil};
use cocoa::foundation::NSAutoreleasePool;
use core_foundation::array::CFArray;
use core_foundation::base::{Boolean, CFRetain, CFRelease, CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::runloop::{CFRunLoop, kCFRunLoopDefaultMode};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::display::CGMainDisplayID;
use objc::runtime::{Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use serde::Serialize;

/// CLI flags
#[derive(Debug, Parser)]
#[command(name = "focus-track", version, about = "Track active app and tab changes (macOS Accessibility)")]
struct Cli {
    /// Output format: text or json
    #[arg(long, default_value = "text", value_parser = ["text", "json"])]
    format: String,

    /// Do not show the Accessibility permission prompt if not trusted
    #[arg(long)]
    no_prompt: bool,
}

#[derive(Debug, Clone, Serialize)]
struct AppState {
    app_name: String,
    bundle_id: String,
    window_title: Option<String>,
    tab_title: Option<String>,
    #[serde(skip)]
    started_at: Instant,
}

#[derive(Debug, Clone, Serialize)]
struct FocusEvent {
    event_type: String,
    from: AppState,
    to: AppState,
    duration_ms: u128,
}

struct FocusTracker {
    current_state: Arc<Mutex<AppState>>,
    json_output: bool,
    app_observer: Option<AXObserverRef>,
    window_observer: Option<AXObserverRef>,
}

impl FocusTracker {
    fn new(json_output: bool) -> Self {
        Self {
            current_state: Arc::new(Mutex::new(AppState {
                app_name: String::new(),
                bundle_id: String::new(),
                window_title: None,
                tab_title: None,
                started_at: Instant::now(),
            })),
            json_output,
            app_observer: None,
            window_observer: None,
        }
    }

    fn get_frontmost_app(&self) -> Option<(String, String, i32)> {
        unsafe {
            // Create a new autorelease pool for each call
            let pool = NSAutoreleasePool::new(nil);
            
            let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
            // Force refresh by getting running applications first
            let _running_apps: id = msg_send![workspace, runningApplications];
            let frontmost_app: id = msg_send![workspace, frontmostApplication];
            
            if frontmost_app == nil {
                let _: () = msg_send![pool, drain];
                return None;
            }

            let bundle_id: id = msg_send![frontmost_app, bundleIdentifier];
            let name: id = msg_send![frontmost_app, localizedName];
            let pid: i32 = msg_send![frontmost_app, processIdentifier];

            let bundle_str = if bundle_id != nil {
                let cstr = CStr::from_ptr(msg_send![bundle_id, UTF8String]);
                cstr.to_string_lossy().to_string()
            } else {
                String::new()
            };

            let name_str = if name != nil {
                let cstr = CStr::from_ptr(msg_send![name, UTF8String]);
                cstr.to_string_lossy().to_string()
            } else {
                String::from("Unknown")
            };

            let _: () = msg_send![pool, drain];
            Some((name_str, bundle_str, pid))
        }
    }

    fn get_window_title(&self, app_element: AXUIElementRef) -> Option<String> {
        unsafe {
            let mut window_ref: CFTypeRef = null_mut();
            let window_attr = CFString::new("AXFocusedWindow");
            
            if AXUIElementCopyAttributeValue(
                app_element,
                window_attr.as_concrete_TypeRef() as CFStringRef,
                &mut window_ref,
            ) != kAXErrorSuccess
            {
                return None;
            }

            if window_ref.is_null() {
                return None;
            }

            let window = window_ref as AXUIElementRef;
            let mut title_ref: CFTypeRef = null_mut();
            let title_attr = CFString::new("AXTitle");

            if AXUIElementCopyAttributeValue(
                window,
                title_attr.as_concrete_TypeRef() as CFStringRef,
                &mut title_ref,
            ) == kAXErrorSuccess && !title_ref.is_null()
            {
                let title_cfstr = CFString::wrap_under_create_rule(title_ref as CFStringRef);
                let result = title_cfstr.to_string();
                CFRelease(window_ref);
                Some(result)
            } else {
                CFRelease(window_ref);
                None
            }
        }
    }

    fn get_browser_tab_title(&self, app_element: AXUIElementRef) -> Option<String> {
        unsafe {
            // Try to find tab group in the window
            let mut window_ref: CFTypeRef = null_mut();
            let window_attr = CFString::new("AXFocusedWindow");
            
            if AXUIElementCopyAttributeValue(
                app_element,
                window_attr.as_concrete_TypeRef() as CFStringRef,
                &mut window_ref,
            ) != kAXErrorSuccess || window_ref.is_null()
            {
                return None;
            }

            let window = window_ref as AXUIElementRef;
            let tab_title = self.find_selected_tab_in_element(window);
            CFRelease(window_ref);
            tab_title
        }
    }

    fn find_selected_tab_in_element(&self, element: AXUIElementRef) -> Option<String> {
        unsafe {
            // Check if this is a tab group
            let mut role_ref: CFTypeRef = null_mut();
            let role_attr = CFString::new("AXRole");
            
            if AXUIElementCopyAttributeValue(
                element,
                role_attr.as_concrete_TypeRef() as CFStringRef,
                &mut role_ref,
            ) == kAXErrorSuccess && !role_ref.is_null()
            {
                let role_cfstr = CFString::wrap_under_create_rule(role_ref as CFStringRef);
                if role_cfstr.to_string() == "AXTabGroup" {
                    // Found tab group, look for selected tab
                    return self.get_selected_tab_from_group(element);
                }
            }

            // Recurse into children
            let mut children_ref: CFTypeRef = null_mut();
            let children_attr = CFString::new("AXChildren");
            
            if AXUIElementCopyAttributeValue(
                element,
                children_attr.as_concrete_TypeRef() as CFStringRef,
                &mut children_ref,
            ) == kAXErrorSuccess && !children_ref.is_null()
            {
                let children_array = CFArray::<CFType>::wrap_under_create_rule(children_ref as core_foundation::array::CFArrayRef);
                
                for i in 0..children_array.len() {
                    if let Some(child_cf) = children_array.get(i) {
                        let child_element = child_cf.as_CFTypeRef() as AXUIElementRef;
                        if let Some(title) = self.find_selected_tab_in_element(child_element) {
                            return Some(title);
                        }
                    }
                }
            }

            None
        }
    }

    fn get_selected_tab_from_group(&self, tab_group: AXUIElementRef) -> Option<String> {
        unsafe {
            let mut children_ref: CFTypeRef = null_mut();
            let children_attr = CFString::new("AXChildren");
            
            if AXUIElementCopyAttributeValue(
                tab_group,
                children_attr.as_concrete_TypeRef() as CFStringRef,
                &mut children_ref,
            ) != kAXErrorSuccess || children_ref.is_null()
            {
                return None;
            }

            let children_array = CFArray::<CFType>::wrap_under_create_rule(children_ref as core_foundation::array::CFArrayRef);
            
            for i in 0..children_array.len() {
                if let Some(child_cf) = children_array.get(i) {
                    let child_element = child_cf.as_CFTypeRef() as AXUIElementRef;
                    
                    // Check if this tab is selected
                    let mut selected_ref: CFTypeRef = null_mut();
                    let selected_attr = CFString::new("AXSelected");
                    
                    if AXUIElementCopyAttributeValue(
                        child_element,
                        selected_attr.as_concrete_TypeRef() as CFStringRef,
                        &mut selected_ref,
                    ) == kAXErrorSuccess && !selected_ref.is_null()
                    {
                        let selected = CFBoolean::wrap_under_create_rule(selected_ref as core_foundation::boolean::CFBooleanRef);
                        
                        if selected.into() {
                            // Get the title of the selected tab
                            let mut title_ref: CFTypeRef = null_mut();
                            let title_attr = CFString::new("AXTitle");
                            
                            if AXUIElementCopyAttributeValue(
                                child_element,
                                title_attr.as_concrete_TypeRef() as CFStringRef,
                                &mut title_ref,
                            ) == kAXErrorSuccess && !title_ref.is_null()
                            {
                                let title_cfstr = CFString::wrap_under_create_rule(title_ref as CFStringRef);
                                return Some(title_cfstr.to_string());
                            }
                        }
                    }
                }
            }

            None
        }
    }

    fn update_state(&self, new_state: AppState) {
        let mut current = self.current_state.lock().unwrap();
        let old_state = current.clone();
        
        // Debug: Always log what we're detecting
        if !self.json_output && (old_state.app_name != new_state.app_name || old_state.bundle_id != new_state.bundle_id) {
            eprintln!("DEBUG: Detected app: {} ({}), PID: {}", new_state.app_name, new_state.bundle_id, 
                     new_state.started_at.elapsed().as_millis());
        }
        
        if old_state.app_name != new_state.app_name || 
           old_state.window_title != new_state.window_title ||
           old_state.tab_title != new_state.tab_title {
            
            let duration = old_state.started_at.elapsed();
            
            if !old_state.app_name.is_empty() {
                let event = FocusEvent {
                    event_type: if old_state.app_name != new_state.app_name {
                        "app_switch".to_string()
                    } else if old_state.window_title != new_state.window_title {
                        "window_change".to_string()
                    } else {
                        "tab_change".to_string()
                    },
                    from: old_state,
                    to: new_state.clone(),
                    duration_ms: duration.as_millis(),
                };

                if self.json_output {
                    println!("{}", serde_json::to_string(&event).unwrap());
                } else {
                    let from_str = format!("{} - {:?}", event.from.app_name, event.from.window_title.or(event.from.tab_title));
                    let to_str = format!("{} - {:?}", event.to.app_name, event.to.window_title.or(event.to.tab_title));
                    println!("[{}] From: {} To: {} ({}ms)", event.event_type, from_str, to_str, event.duration_ms);
                }
            }
        }
        
        *current = new_state;
    }

    fn start(&mut self) {
        // Get initial state
        if let Some((name, bundle, pid)) = self.get_frontmost_app() {
            let app_element = unsafe { AXUIElementCreateApplication(pid) };
            
            let window_title = self.get_window_title(app_element);
            let tab_title = if name.contains("Chrome") || name.contains("Safari") || name.contains("Firefox") {
                self.get_browser_tab_title(app_element)
            } else {
                None
            };

            let initial_state = AppState {
                app_name: name,
                bundle_id: bundle,
                window_title,
                tab_title,
                started_at: Instant::now(),
            };

            self.update_state(initial_state.clone());

            if !self.json_output {
                println!("Started tracking focus changes...");
                println!("Current app: {} - {:?}", initial_state.app_name, initial_state.window_title.or(initial_state.tab_title));
            }

            unsafe { CFRelease(app_element as CFTypeRef); }
        }

        // Start polling loop
        loop {
            std::thread::sleep(Duration::from_millis(250));
            
            if let Some((name, bundle, pid)) = self.get_frontmost_app() {
                let app_element = unsafe { AXUIElementCreateApplication(pid) };
                
                let window_title = self.get_window_title(app_element);
                let tab_title = if name.contains("Chrome") || name.contains("Safari") || name.contains("Firefox") {
                    self.get_browser_tab_title(app_element)
                } else {
                    None
                };

                let new_state = AppState {
                    app_name: name,
                    bundle_id: bundle,
                    window_title,
                    tab_title,
                    started_at: Instant::now(),
                };

                self.update_state(new_state);

                unsafe { CFRelease(app_element as CFTypeRef); }
            }
        }
    }
}

fn ensure_ax_trust(prompt: bool) -> bool {
    unsafe {
        if AXIsProcessTrusted() {
            return true;
        }
        
        if prompt {
            let options = core_foundation::dictionary::CFDictionary::from_CFType_pairs(&[
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
    
    // Check accessibility permissions
    let trusted = ensure_ax_trust(!cli.no_prompt);
    if !trusted {
        eprintln!(
            "Accessibility access is not granted. \
             Please enable: System Settings → Privacy & Security → Accessibility → allow this app."
        );
        std::process::exit(1);
    }

    // Initialize NSApplication (required for NSWorkspace)
    unsafe {
        let _pool = NSAutoreleasePool::new(nil);
        let app = NSApp();
        app.setActivationPolicy_(NSApplicationActivationPolicyRegular);
    }

    // Start tracking
    let mut tracker = FocusTracker::new(cli.format == "json");
    tracker.start();
}