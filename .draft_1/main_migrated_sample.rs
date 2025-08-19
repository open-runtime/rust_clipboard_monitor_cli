#![cfg(target_os = "macos")]

use std::collections::{HashMap, HashSet};
use std::ffi::{c_void, CStr};
use std::process::Command;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use arboard::Clipboard;

// Import the clipboard file reader module
mod clipboard_file_reader;
use clipboard_file_reader::{is_text_file, read_file_contents_safe};

use accessibility_sys::*;

// MIGRATION: Replace cocoa and objc imports with objc2
// OLD:
// use cocoa::appkit::{NSApp, NSApplication, NSApplicationActivationPolicyProhibited, NSPasteboard};
// use cocoa::base::{id, nil};
// use cocoa::foundation::NSAutoreleasePool;
// use objc::declare::ClassDecl;
// use objc::runtime::{Class, Object, Sel};
// use objc::{class, msg_send, sel, sel_impl};

// NEW:
use block2::{Block, ConcreteBlock};
use objc2::declare::ClassBuilder;
use objc2::rc::{Id, Retained};
use objc2::runtime::{AnyClass, AnyObject, ProtocolObject, Sel};
use objc2::{msg_send, msg_send_id, sel, ClassType};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSPasteboard, NSPasteboardType,
    NSRunningApplication, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSAutoreleasePool, NSDictionary, NSNotificationCenter,
    NSNotificationName, NSNumber, NSObject, NSString, NSThread,
};

// Keep existing imports
use clap::Parser;
use core_foundation::array::CFArray;
use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopRun};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation_sys::base::{CFGetRetainCount, CFGetTypeID, CFIndex, CFTypeID};
use core_foundation_sys::runloop::{CFRunLoopAddSource, CFRunLoopRemoveSource};
use core_foundation_sys::string::CFStringGetTypeID;
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType};
use core_graphics::event_source::CGEventSource;
use serde::Serialize;

// ... [Keep existing external functions, structs, and other definitions] ...

// Example of migrated functions:

impl Tracker {
    // Example: Migrated get_clipboard_change_count function
    fn get_clipboard_change_count(&self) -> i64 {
        unsafe {
            // OLD:
            // let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            // msg_send![pasteboard, changeCount]

            // NEW:
            let pasteboard = NSPasteboard::generalPasteboard();
            pasteboard.changeCount()
        }
    }

    // Example: Migrated get_clipboard_text_nspasteboard function
    fn get_clipboard_text_nspasteboard(&self) -> Option<String> {
        unsafe {
            // OLD:
            // let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            // let string: id = msg_send![pasteboard, stringForType: nil];
            // if string != nil {
            //     let c_str: *const i8 = msg_send![string, UTF8String];
            //     if !c_str.is_null() {
            //         return Some(CStr::from_ptr(c_str).to_string_lossy().to_string());
            //     }
            // }

            // NEW:
            let pasteboard = NSPasteboard::generalPasteboard();

            // Try to get string for default type
            if let Some(types) = pasteboard.types() {
                // Check for string type
                let string_type = NSPasteboardType::string();
                if types.iter().any(|t| t == &string_type) {
                    if let Some(string) = pasteboard.stringForType(&string_type) {
                        return Some(string.to_string());
                    }
                }

                // Try other text types
                let plain_text_type = NSPasteboardType::from_str("public.utf8-plain-text");
                if let Some(string) = pasteboard.stringForType(&plain_text_type) {
                    return Some(string.to_string());
                }
            }

            None
        }
    }

    // Example: Migrated get_clipboard_file_paths function
    fn get_clipboard_file_paths(&self) -> Vec<String> {
        unsafe {
            // OLD:
            // let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            // let mut paths = Vec::new();
            // let filenames_type: id = msg_send![class!(NSString), stringWithUTF8String: "NSFilenamesPboardType".as_ptr()];
            // let filenames: id = msg_send![pasteboard, propertyListForType: filenames_type];

            // NEW:
            let pasteboard = NSPasteboard::generalPasteboard();
            let mut paths = Vec::new();

            // Check for file URLs
            let file_url_type = NSPasteboardType::fileURL();
            if let Some(items) = pasteboard
                .readObjectsForClasses_options(&NSArray::from_slice(&[NSURL::class()]), None)
            {
                for item in items.iter() {
                    if let Some(url) = item.as_ref().downcast::<NSURL>() {
                        if let Some(path) = url.path() {
                            paths.push(path.to_string());
                        }
                    }
                }
            }

            paths
        }
    }

    // Example: Migrated get_app_path function
    fn get_app_path(&self, bundle_id: &str) -> Option<String> {
        unsafe {
            // OLD:
            // let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
            // let ns_str: id = msg_send![class!(NSString), alloc];
            // let ns_str: id = msg_send![ns_str, initWithBytes:bundle_id.as_ptr()
            //                            length:bundle_id.len()
            //                            encoding:4];
            // let url: id = msg_send![workspace, URLForApplicationWithBundleIdentifier:ns_str];

            // NEW:
            let workspace = NSWorkspace::sharedWorkspace();
            let bundle_id_str = NSString::from_str(bundle_id);

            if let Some(url) = workspace.URLForApplicationWithBundleIdentifier(&bundle_id_str) {
                if let Some(path) = url.path() {
                    return Some(path.to_string());
                }
            }

            None
        }
    }
}

// Example: Migrated workspace_callback function
unsafe extern "C" fn workspace_callback(this: &AnyObject, _cmd: Sel, notification: &AnyObject) {
    // OLD:
    // let user_info: id = msg_send![notification, userInfo];
    // if user_info == nil { return; }
    // let key = "NSWorkspaceApplicationKey";
    // let ns_key: id = msg_send![class!(NSString), alloc];
    // let ns_key: id = msg_send![ns_key, initWithBytes:key.as_ptr()
    //                                   length:key.len()
    //                                   encoding:4];
    // let app: id = msg_send![user_info, objectForKey:ns_key];

    // NEW:
    let notification = notification as *const AnyObject as *const NSNotification;
    let notification = &*notification;

    if let Some(user_info) = notification.userInfo() {
        let key = NSString::from_str("NSWorkspaceApplicationKey");

        if let Some(app) = user_info.get(&key) {
            // Cast to NSRunningApplication
            if let Some(running_app) = app.downcast::<NSRunningApplication>() {
                let pid = running_app.processIdentifier();

                let bundle_str = running_app
                    .bundleIdentifier()
                    .map(|s| s.to_string())
                    .unwrap_or_default();

                let name_str = running_app
                    .localizedName()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| String::from("Unknown"));

                with_state(|tracker| {
                    tracker.handle_app_change(name_str, bundle_str, pid);
                });
            }
        }
    }
}

// Example: Migrated create_observer_class function
fn create_observer_class() -> &'static AnyClass {
    // OLD:
    // let superclass = class!(NSObject);
    // let mut decl = ClassDecl::new("FocusObserver", superclass).unwrap();
    // unsafe {
    //     decl.add_method(
    //         sel!(workspaceDidActivateApp:),
    //         workspace_callback as extern "C" fn(&Object, Sel, id),
    //     );
    // }
    // decl.register()

    // NEW:
    let mut builder = ClassBuilder::new("FocusObserver", NSObject::class())
        .expect("Failed to create class builder");

    unsafe {
        builder.add_method(
            sel!(workspaceDidActivateApp:),
            workspace_callback as unsafe extern "C" fn(&AnyObject, Sel, &AnyObject),
        );
    }

    builder.register()
}

// Example: Migrated main function (partial)
fn main() {
    let cli = Cli::parse();

    if !ensure_ax_trust(!cli.no_prompt) {
        eprintln!("Accessibility access required.");
        eprintln!("Enable in: System Settings → Privacy & Security → Accessibility");
        std::process::exit(1);
    }

    unsafe {
        // OLD:
        // let pool = NSAutoreleasePool::new(nil);
        // let app = NSApp();
        // app.setActivationPolicy_(NSApplicationActivationPolicyProhibited);

        // NEW:
        NSAutoreleasePool::with(|_pool| {
            let app = NSApplication::sharedApplication();
            app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);

            STATE
                .set(Arc::new(Mutex::new(Tracker::new(&cli))))
                .expect("Failed to initialize");

            // Start clipboard monitoring thread
            {
                let mut tracker = STATE.get().unwrap().lock().unwrap();
                tracker.start_clipboard_monitor();
                tracker.setup_keyboard_tap();
                tracker.setup_scroll_tap();
            }

            // Get initial app
            let workspace = NSWorkspace::sharedWorkspace();
            if let Some(frontmost) = workspace.frontmostApplication() {
                let bundle_str = frontmost
                    .bundleIdentifier()
                    .map(|s| s.to_string())
                    .unwrap_or_default();

                let name_str = frontmost
                    .localizedName()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| String::from("Unknown"));

                let pid = frontmost.processIdentifier();

                with_state(|tracker| {
                    tracker.handle_app_change(name_str, bundle_str, pid);
                });
            }

            // Register workspace observer
            let observer_class = create_observer_class();
            let observer: Retained<AnyObject> = msg_send_id![observer_class, new];

            let nc = workspace.notificationCenter();
            let notif_name =
                NSNotificationName::from_str("NSWorkspaceDidActivateApplicationNotification");

            nc.addObserver_selector_name_object(
                &observer,
                sel!(workspaceDidActivateApp:),
                Some(&notif_name),
                None,
            );

            if cli.format != "json" {
                println!("Tracking app context, URLs, files, and UI state...");
            }

            CFRunLoopRun();
        });
    }
}
