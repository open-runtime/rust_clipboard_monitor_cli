// Quick Reference: Migrating Your Specific setup_observer Function

// ORIGINAL CODE (from main.rs):
/*
fn setup_observer(&mut self, pid: i32) {
    unsafe {
        let mut observer: AXObserverRef = null_mut();

        if AXObserverCreate(pid, ax_callback, &mut observer) == kAXErrorSuccess {
            let app = AXUIElementCreateApplication(pid);

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
*/

// MIGRATED CODE WITH objc2:

use accessibility_sys::*;
use core_foundation::base::{CFRelease, CFTypeRef};
use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoop};
use core_foundation::string::{CFString, CFStringRef};
use objc2_foundation::NSAutoreleasePool;
use std::collections::HashMap;
use std::ptr::null_mut;

impl Tracker {
    fn setup_observer(&mut self, pid: i32) {
        unsafe {
            let mut observer: AXObserverRef = null_mut();

            // Create the observer - this remains the same as it's C API
            if AXObserverCreate(pid, ax_callback_with_pool, &mut observer) == kAXErrorSuccess {
                let app = AXUIElementCreateApplication(pid);

                // Group notifications by category for better organization
                let window_notifications = [
                    "AXFocusedWindowChanged",
                    "AXMainWindowChanged",
                    "AXWindowCreated",
                    "AXWindowMiniaturized",
                    "AXWindowDeminiaturized",
                    "AXWindowMoved",
                    "AXWindowResized",
                ];

                let app_state_notifications = [
                    "AXApplicationActivated",   // App comes to foreground
                    "AXApplicationDeactivated", // App goes to background
                    "AXApplicationShown",       // App becomes visible
                    "AXApplicationHidden",      // App becomes hidden
                ];

                let ui_element_notifications = [
                    "AXTitleChanged",
                    "AXFocusedUIElementChanged",
                    "AXValueChanged",
                    "AXSelectedChildrenChanged",
                    "AXSelectedTextChanged",
                    "AXMenuItemSelected",
                    "AXSelectedRowsChanged",
                    "AXRowCountChanged",
                ];

                // Register all notifications
                let all_notifications: Vec<&str> = window_notifications
                    .iter()
                    .chain(app_state_notifications.iter())
                    .chain(ui_element_notifications.iter())
                    .copied()
                    .collect();

                for notif in &all_notifications {
                    let cfstr = CFString::new(notif);
                    AXObserverAddNotification(
                        observer,
                        app,
                        cfstr.as_concrete_TypeRef() as CFStringRef,
                        null_mut(),
                    );
                }

                // Add to run loop
                let source = AXObserverGetRunLoopSource(observer);
                CFRunLoopAddSource(
                    CFRunLoop::get_current().as_concrete_TypeRef(),
                    source,
                    kCFRunLoopDefaultMode as CFStringRef,
                );

                // Store observer reference
                self.current_observer = Some(observer as usize);
                CFRelease(app as CFTypeRef);
            }
        }
    }

    // Enhanced cleanup in Drop
    fn cleanup_observer(&mut self) {
        if let Some(observer_ptr) = self.current_observer.take() {
            unsafe {
                let observer = observer_ptr as AXObserverRef;
                let source = AXObserverGetRunLoopSource(observer);
                CFRunLoopRemoveSource(
                    CFRunLoop::get_current().as_concrete_TypeRef(),
                    source,
                    kCFRunLoopDefaultMode as CFStringRef,
                );
                CFRelease(observer as CFTypeRef);
            }
        }
    }
}

// CRITICAL CHANGE: Wrap callback in NSAutoreleasePool
extern "C" fn ax_callback_with_pool(
    observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    user_data: *mut c_void,
) {
    // This is the KEY CHANGE - wrap everything in autorelease pool
    NSAutoreleasePool::with(|_pool| {
        unsafe {
            let notif = CFString::wrap_under_get_rule(notification).to_string();

            // Categorize the notification
            let event_category = match notif.as_str() {
                "AXApplicationActivated" => "app_foreground",
                "AXApplicationDeactivated" => "app_background",
                "AXApplicationShown" => "app_shown",
                "AXApplicationHidden" => "app_hidden",
                "AXFocusedWindowChanged" | "AXMainWindowChanged" => "window_focus",
                "AXWindowCreated" => "window_created",
                "AXTitleChanged" => "title_change",
                _ => "other",
            };

            // Log important events
            match event_category {
                "app_foreground" => {
                    eprintln!("🟢 App moved to FOREGROUND");
                    with_state(|tracker| {
                        tracker.handle_app_foreground();
                    });
                }
                "app_background" => {
                    eprintln!("🔴 App moved to BACKGROUND");
                    with_state(|tracker| {
                        tracker.handle_app_background();
                    });
                }
                "app_shown" | "app_hidden" => {
                    eprintln!("👁️ App visibility changed: {}", notif);
                }
                _ => {
                    // Handle other notifications
                }
            }

            // Call the original handler
            with_state(|tracker| {
                tracker.handle_ui_change(&notif);
            });
        }
    });
}

// Additional helper methods for foreground/background handling
impl Tracker {
    fn handle_app_foreground(&mut self) {
        // App came to foreground
        if let Some(ctx) = &mut self.current_context {
            ctx.app_state = AppState::Foreground;
            ctx.foreground_time = Some(Instant::now());

            // Resume any paused operations
            self.resume_monitoring();
        }
    }

    fn handle_app_background(&mut self) {
        // App went to background
        if let Some(ctx) = &mut self.current_context {
            ctx.app_state = AppState::Background;

            // Calculate time spent in foreground
            if let Some(fg_time) = ctx.foreground_time {
                let duration = fg_time.elapsed();
                ctx.total_foreground_time += duration;
            }

            // Potentially pause intensive operations
            self.pause_monitoring();
        }
    }

    fn resume_monitoring(&mut self) {
        // Resume clipboard monitoring, network checks, etc.
        self.monitoring_paused = false;
    }

    fn pause_monitoring(&mut self) {
        // Pause non-essential monitoring to save resources
        self.monitoring_paused = true;
    }
}

#[derive(Debug, Clone, PartialEq)]
enum AppState {
    Foreground,
    Background,
    Hidden,
    Minimized,
}

// Example of how to use with NSWorkspace for double verification
use block2::ConcreteBlock;
use objc2_app_kit::{NSRunningApplication, NSWorkspace};
use objc2_foundation::{NSNotificationCenter, NSNotificationName};

impl Tracker {
    fn setup_workspace_notifications(&mut self) {
        let workspace = NSWorkspace::sharedWorkspace();
        let nc = workspace.notificationCenter();

        // Monitor both activate and deactivate
        let notifications = [
            ("NSWorkspaceDidActivateApplicationNotification", true), // foreground
            ("NSWorkspaceDidDeactivateApplicationNotification", false), // background
        ];

        for (notif_name, is_foreground) in notifications {
            let state = STATE.get().unwrap().clone();

            let block = ConcreteBlock::new(move |notification: &NSNotification| {
                NSAutoreleasePool::with(|_pool| {
                    if let Ok(mut tracker) = state.lock() {
                        if is_foreground {
                            tracker.handle_app_foreground();
                        } else {
                            tracker.handle_app_background();
                        }
                    }
                });
            });

            let name = NSNotificationName::from_str(notif_name);
            let observer = unsafe {
                nc.addObserverForName_object_queue_usingBlock(Some(&name), None, None, &block)
            };

            self.workspace_observers.push(observer);
        }
    }
}

// Complete Drop implementation
impl Drop for Tracker {
    fn drop(&mut self) {
        // Clean up AX observers
        self.cleanup_observer();

        // Clean up event taps
        unsafe {
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
        }

        // Workspace observers are automatically cleaned when dropped
        self.workspace_observers.clear();
    }
}
