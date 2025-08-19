#![cfg(target_os = "macos")]

use std::ffi::{c_void, CStr};
use std::ptr::null_mut;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use accessibility_sys::*;
use clap::Parser;
use cocoa::appkit::{NSApp, NSApplication, NSApplicationActivationPolicyProhibited};
use cocoa::base::{id, nil, BOOL, YES};
use cocoa::foundation::{NSAutoreleasePool, NSDefaultRunLoopMode, NSRunLoop};
use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::runloop::{CFRunLoop, kCFRunLoopDefaultMode, CFRunLoopRun};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::dictionary::CFDictionary;
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use serde::Serialize;

/// CLI flags
#[derive(Debug, Parser)]
#[command(name = "focus-track", version, about = "Track active app and focus changes on macOS")]
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
    pid: i32,
    window_title: Option<String>,
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

static mut TRACKER: Option<Arc<Mutex<FocusTracker>>> = None;

struct FocusTracker {
    current_state: Option<AppState>,
    json_output: bool,
    ax_observer: Option<AXObserverRef>,
}

impl FocusTracker {
    fn new(json_output: bool) -> Self {
        Self {
            current_state: None,
            json_output,
            ax_observer: None,
        }
    }

    fn handle_app_change(&mut self, new_app_name: String, new_bundle_id: String, new_pid: i32) {
        let window_title = Self::get_window_title(new_pid);
        
        let new_state = AppState {
            app_name: new_app_name,
            bundle_id: new_bundle_id,
            pid: new_pid,
            window_title,
            started_at: Instant::now(),
        };

        if let Some(ref old_state) = self.current_state {
            if old_state.app_name != new_state.app_name {
                let event = FocusEvent {
                    event_type: "app_switch".to_string(),
                    from: old_state.clone(),
                    to: new_state.clone(),
                    duration_ms: old_state.started_at.elapsed().as_millis(),
                };

                if self.json_output {
                    println!("{}", serde_json::to_string(&event).unwrap());
                } else {
                    println!("[app_switch] {} → {} ({}ms)", 
                        event.from.app_name,
                        event.to.app_name,
                        event.duration_ms
                    );
                }
            }
        } else if !self.json_output {
            println!("Started tracking...");
            println!("Current: {} ({})", new_state.app_name, new_state.bundle_id);
        }

        // Set up AX observer for the new app
        self.setup_ax_observer(new_pid);
        
        self.current_state = Some(new_state);
    }

    fn handle_window_change(&mut self) {
        if let Some(ref mut state) = self.current_state {
            let new_window_title = Self::get_window_title(state.pid);
            
            if new_window_title != state.window_title {
                let old_state = state.clone();
                state.window_title = new_window_title.clone();
                state.started_at = Instant::now();
                
                let event = FocusEvent {
                    event_type: "window_change".to_string(),
                    from: old_state.clone(),
                    to: state.clone(),
                    duration_ms: old_state.started_at.elapsed().as_millis(),
                };

                if self.json_output {
                    println!("{}", serde_json::to_string(&event).unwrap());
                } else {
                    println!("[window_change] {} → {} ({}ms)", 
                        old_state.window_title.as_deref().unwrap_or("none"),
                        state.window_title.as_deref().unwrap_or("none"),
                        event.duration_ms
                    );
                }
            }
        }
    }

    fn get_window_title(pid: i32) -> Option<String> {
        unsafe {
            let app_element = AXUIElementCreateApplication(pid);
            let mut window_ref: CFTypeRef = null_mut();
            let window_attr = CFString::new("AXFocusedWindow");
            
            if AXUIElementCopyAttributeValue(
                app_element,
                window_attr.as_concrete_TypeRef() as CFStringRef,
                &mut window_ref,
            ) != kAXErrorSuccess || window_ref.is_null()
            {
                CFRelease(app_element as CFTypeRef);
                return None;
            }

            let window = window_ref as AXUIElementRef;
            let mut title_ref: CFTypeRef = null_mut();
            let title_attr = CFString::new("AXTitle");

            let result = if AXUIElementCopyAttributeValue(
                window,
                title_attr.as_concrete_TypeRef() as CFStringRef,
                &mut title_ref,
            ) == kAXErrorSuccess && !title_ref.is_null()
            {
                let title_cfstr = CFString::wrap_under_create_rule(title_ref as CFStringRef);
                Some(title_cfstr.to_string())
            } else {
                None
            };

            CFRelease(window_ref);
            CFRelease(app_element as CFTypeRef);
            result
        }
    }

    fn setup_ax_observer(&mut self, pid: i32) {
        unsafe {
            // Remove old observer if any
            if let Some(old_obs) = self.ax_observer {
                CFRelease(old_obs as CFTypeRef);
                self.ax_observer = None;
            }

            // Create new observer for window changes
            let mut observer: AXObserverRef = null_mut();
            if AXObserverCreate(pid, Some(ax_callback), &mut observer) == kAXErrorSuccess {
                let app_element = AXUIElementCreateApplication(pid);
                
                // Add notification for focused window changes
                let notif = CFString::new("AXFocusedWindowChanged");
                AXObserverAddNotification(
                    observer,
                    app_element,
                    notif.as_concrete_TypeRef() as CFStringRef,
                    null_mut()
                );
                
                // Add observer to run loop
                let source = AXObserverGetRunLoopSource(observer);
                let run_loop = CFRunLoop::get_current();
                run_loop.add_source(&source.wrap_under_get_rule(), kCFRunLoopDefaultMode);
                
                self.ax_observer = Some(observer);
                CFRelease(app_element as CFTypeRef);
            }
        }
    }
}

// AX Observer callback
extern "C" fn ax_callback(
    _observer: AXObserverRef,
    _element: AXUIElementRef,
    notification: CFStringRef,
    _user_data: *mut c_void,
) {
    unsafe {
        if let Some(ref tracker_arc) = TRACKER {
            let mut tracker = tracker_arc.lock().unwrap();
            
            let notif_str = CFString::wrap_under_get_rule(notification).to_string();
            if notif_str == "AXFocusedWindowChanged" {
                tracker.handle_window_change();
            }
        }
    }
}

// NSWorkspace observer
extern "C" fn workspace_did_activate_app(this: &Object, _cmd: Sel, notification: id) {
    unsafe {
        let user_info: id = msg_send![notification, userInfo];
        if user_info == nil {
            return;
        }

        let app_key = NSString::from_str("NSWorkspaceApplicationKey");
        let app: id = msg_send![user_info, objectForKey:app_key];
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

        if let Some(ref tracker_arc) = TRACKER {
            let mut tracker = tracker_arc.lock().unwrap();
            tracker.handle_app_change(name_str, bundle_str, pid);
        }
    }
}

fn create_observer_class() -> *const Class {
    let superclass = class!(NSObject);
    let mut decl = ClassDecl::new("WorkspaceObserver", superclass).unwrap();
    
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

// Helper to create NSString
struct NSString;
impl NSString {
    fn from_str(s: &str) -> id {
        unsafe {
            let ns_str: id = msg_send![class!(NSString), alloc];
            msg_send![ns_str, initWithBytes:s.as_ptr() length:s.len() encoding:4] // UTF8
        }
    }
}

fn main() {
    let cli = Cli::parse();
    
    if !ensure_ax_trust(!cli.no_prompt) {
        eprintln!("⚠️  Accessibility access required!");
        eprintln!("Please enable in: System Settings → Privacy & Security → Accessibility");
        std::process::exit(1);
    }

    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        
        // Initialize app
        let app = NSApp();
        app.setActivationPolicy_(NSApplicationActivationPolicyProhibited);
        
        // Create tracker
        TRACKER = Some(Arc::new(Mutex::new(FocusTracker::new(cli.format == "json"))));
        
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
            
            if let Some(ref tracker_arc) = TRACKER {
                let mut tracker = tracker_arc.lock().unwrap();
                tracker.handle_app_change(name_str, bundle_str, pid);
            }
        }
        
        // Create observer for NSWorkspace notifications
        let observer_class = create_observer_class();
        let observer: id = msg_send![observer_class, new];
        
        // Register for app activation notifications
        let notification_center: id = msg_send![workspace, notificationCenter];
        let notif_name = NSString::from_str("NSWorkspaceDidActivateApplicationNotification");
        
        let _: () = msg_send![notification_center,
            addObserver:observer
            selector:sel!(workspaceDidActivateApp:)
            name:notif_name
            object:nil
        ];
        
        // Run the event loop
        CFRunLoopRun();
        
        let _: () = msg_send![pool, drain];
    }
}