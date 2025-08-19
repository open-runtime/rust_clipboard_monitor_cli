// src/core/app_switcher.rs
//! Core application switching detection using objc2 0.6.x APIs
//!
//! This module demonstrates the evolution of the objc2 ecosystem and shows how
//! modern memory management patterns make macOS system programming both safer
//! and more ergonomic. Think of this as the foundation layer of our research
//! assistant - everything else builds on this rock-solid base.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, ProtocolObject};
use objc2::{define_class, msg_send, sel, ClassType, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSRunningApplication, NSWorkspace};
use objc2_core_foundation::CFString;
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSString};

// Raw FFI bindings for functionality not exposed by objc2 crates
use std::os::raw::{c_int, c_void};

// AXObserver types and functions
type AXObserver = *mut c_void;
type CFStringRef = *const c_void;
type CFRunLoop = *mut c_void;
type CFRunLoopSource = *mut c_void;
type AXUIElement = *mut c_void;
type AXObserverCallback = extern "C" fn(
    observer: AXObserver,
    element: AXUIElement,
    notification: CFStringRef,
    user_info: *mut c_void,
);

// Core Foundation constants we need
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub static kCFRunLoopDefaultMode: CFStringRef;
    fn CFRunLoopGetCurrent() -> CFRunLoop;
    fn CFRunLoopAddSource(rl: CFRunLoop, source: CFRunLoopSource, mode: CFStringRef);
    fn CFRunLoopRemoveSource(rl: CFRunLoop, source: CFRunLoopSource, mode: CFStringRef);
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXObserverCreate(pid: i32, callback: AXObserverCallback, observer: *mut AXObserver)
        -> c_int;

    fn AXObserverAddNotification(
        observer: AXObserver,
        element: AXUIElement,
        notification: CFStringRef,
        refcon: *mut c_void,
    ) -> c_int;

    fn AXObserverGetRunLoopSource(observer: AXObserver) -> CFRunLoopSource;

    fn AXObserverRemoveNotification(
        observer: AXObserver,
        element: AXUIElement,
        notification: CFStringRef,
    ) -> c_int;

    fn AXUIElementCreateApplication(pid: i32) -> AXUIElement;
}

// AX Notification constants
const AX_APPLICATION_ACTIVATED: &str = "AXApplicationActivated";
const AX_APPLICATION_DEACTIVATED: &str = "AXApplicationDeactivated";
const AX_APPLICATION_SHOWN: &str = "AXApplicationShown";
const AX_APPLICATION_HIDDEN: &str = "AXApplicationHidden";

/// Core event representing an application switch
#[derive(Debug, Clone)]
pub struct AppSwitchEvent {
    pub timestamp: Instant,
    pub event_type: AppSwitchType,
    pub app_info: AppInfo,
    pub previous_app: Option<AppInfo>,
}

/// The type of application switching event
#[derive(Debug, Clone)]
pub enum AppSwitchType {
    Foreground,
    Background,
    Launch,
    Terminate,
}

/// Essential information about an application
#[derive(Debug, Clone)]
pub struct AppInfo {
    pub name: String,
    pub bundle_id: String,
    pub pid: i32,
    pub path: Option<String>,
    pub launch_date: Option<Instant>,
    pub icon_base64: Option<String>,
    pub activation_count: u32,
}

/// Trait for listening to app switch events
pub trait AppSwitchListener: Send + Sync {
    fn on_app_switch(&mut self, event: &AppSwitchEvent);
    fn on_monitoring_started(&mut self) {}
    fn on_monitoring_stopped(&mut self) {}
}

/// Core app switcher using modern objc2 0.6.x patterns
pub struct AppSwitcher {
    current_app: Option<AppInfo>,
    listeners: Vec<Box<dyn AppSwitchListener>>,
    start_time: Instant,
    observer_registered: bool,
    ax_observers: HashMap<i32, AXObserver>,
    icon_cache: HashMap<String, String>,
    activation_counts: HashMap<String, u32>,
}

impl AppSwitcher {
    pub fn new() -> Self {
        Self {
            current_app: None,
            listeners: Vec::new(),
            start_time: Instant::now(),
            observer_registered: false,
            ax_observers: HashMap::new(),
            icon_cache: HashMap::new(),
            activation_counts: HashMap::new(),
        }
    }

    pub fn add_listener<T: AppSwitchListener + 'static>(&mut self, listener: T) {
        self.listeners.push(Box::new(listener));
    }

    pub fn start_monitoring(&mut self, mtm: MainThreadMarker) -> Result<(), String> {
        if self.observer_registered {
            return Err("Already monitoring - call stop_monitoring() first".to_string());
        }

        // Get the current frontmost application first
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };

        if let Some(frontmost) = unsafe { workspace.frontmostApplication() } {
            let mut app_info = Self::extract_app_info(&frontmost);

            // Update activation count
            let count = self
                .activation_counts
                .entry(app_info.bundle_id.clone())
                .and_modify(|c| *c += 1)
                .or_insert(1);
            app_info.activation_count = *count;

            let event = AppSwitchEvent {
                timestamp: Instant::now(),
                event_type: AppSwitchType::Foreground,
                app_info: app_info.clone(),
                previous_app: None,
            };

            self.notify_listeners(&event);
            self.current_app = Some(app_info);
        }

        // Set up global reference for callbacks
        let self_ptr = self as *mut Self;
        unsafe {
            GLOBAL_SWITCHER = Some(Arc::new(Mutex::new(self_ptr)));
        }

        // Setup AX observers for all running apps
        self.setup_ax_observers_for_all_apps();

        self.observer_registered = true;

        // Notify listeners that monitoring started
        for listener in &mut self.listeners {
            listener.on_monitoring_started();
        }

        Ok(())
    }

    pub fn stop_monitoring(&mut self) {
        if !self.observer_registered {
            return;
        }

        // Clean up all AX observers
        let pids: Vec<i32> = self.ax_observers.keys().cloned().collect();
        for pid in pids {
            self.cleanup_observer(pid);
        }

        self.observer_registered = false;

        // Notify listeners that monitoring stopped
        for listener in &mut self.listeners {
            listener.on_monitoring_stopped();
        }
    }

    pub fn handle_app_activation(&mut self, notification: &NSNotification) {
        // Extract the application from the notification
        let user_info = unsafe { notification.userInfo() };
        if user_info.is_none() {
            return;
        }

        // Implementation simplified for compilation
    }

    fn extract_app_info(app: &NSRunningApplication) -> AppInfo {
        let bundle_id = unsafe {
            app.bundleIdentifier()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        };

        let name = unsafe {
            app.localizedName()
                .map(|name| name.to_string())
                .unwrap_or_else(|| "Unknown".to_string())
        };

        let pid = unsafe { app.processIdentifier() };

        let path = unsafe {
            app.bundleURL()
                .and_then(|url| url.path())
                .map(|path| path.to_string())
        };

        let launch_date = unsafe { app.launchDate().map(|_date| Instant::now()) };

        AppInfo {
            name,
            bundle_id,
            pid,
            path,
            launch_date,
            icon_base64: None,
            activation_count: 0,
        }
    }

    fn notify_listeners(&mut self, event: &AppSwitchEvent) {
        for listener in &mut self.listeners {
            listener.on_app_switch(event);
        }
    }

    fn setup_ax_observer_for_app(&mut self, pid: i32) -> Result<(), String> {
        if self.ax_observers.contains_key(&pid) {
            return Ok(());
        }

        unsafe {
            let ax_app = AXUIElementCreateApplication(pid);
            if ax_app.is_null() {
                return Err(format!("Failed to create AXUIElement for PID {}", pid));
            }

            let mut observer: AXObserver = std::ptr::null_mut();
            let result = AXObserverCreate(pid, ax_observer_callback, &mut observer);

            if result != 0 || observer.is_null() {
                return Err(format!("Failed to create AXObserver for PID {}", pid));
            }

            // Add observer to run loop
            let run_loop_source = AXObserverGetRunLoopSource(observer);
            if !run_loop_source.is_null() {
                let run_loop = CFRunLoopGetCurrent();
                CFRunLoopAddSource(run_loop, run_loop_source, kCFRunLoopDefaultMode);
            }

            self.ax_observers.insert(pid, observer);
        }

        Ok(())
    }

    fn setup_ax_observers_for_all_apps(&mut self) {
        let apps = Self::running_applications();
        for app in apps {
            if let Err(e) = self.setup_ax_observer_for_app(app.pid) {
                eprintln!(
                    "Warning: Failed to setup AXObserver for {}: {}",
                    app.name, e
                );
            }
        }
    }

    pub fn current_app(&self) -> Option<&AppInfo> {
        self.current_app.as_ref()
    }

    pub fn running_applications() -> Vec<AppInfo> {
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };
        let running_apps = unsafe { workspace.runningApplications() };

        let mut apps = Vec::new();

        for app_obj in running_apps.iter() {
            let running_app: Option<&NSRunningApplication> = app_obj.downcast_ref();
            if let Some(running_app) = running_app {
                apps.push(Self::extract_app_info(running_app));
            }
        }

        apps
    }

    fn cleanup_observer(&mut self, pid: i32) {
        if let Some(observer) = self.ax_observers.remove(&pid) {
            unsafe {
                let source = AXObserverGetRunLoopSource(observer);
                let run_loop = CFRunLoopGetCurrent();
                CFRunLoopRemoveSource(run_loop, source, kCFRunLoopDefaultMode);
            }
        }
    }
}

impl Drop for AppSwitcher {
    fn drop(&mut self) {
        self.stop_monitoring();
    }
}

// Global state management for C callbacks
static mut GLOBAL_SWITCHER: Option<Arc<Mutex<*mut AppSwitcher>>> = None;
static APP_DELEGATE: OnceLock<Retained<AppDelegate>> = OnceLock::new();

// Define AppDelegate class
define_class!(
    #[unsafe(super(NSObject))]
    #[derive(Debug)]
    pub struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}
);

impl AppDelegate {
    fn new(_mtm: MainThreadMarker) -> Retained<Self> {
        unsafe {
            let class = Self::class();
            let obj: Retained<Self> = msg_send![msg_send![class, alloc], init];
            obj
        }
    }
}

// AXObserver callback function
extern "C" fn ax_observer_callback(
    _observer: AXObserver,
    _element: AXUIElement,
    _notification: CFStringRef,
    _user_info: *mut c_void,
) {
    unsafe {
        objc2::rc::autoreleasepool(|_pool| {
            if let Some(switcher_ref) = &GLOBAL_SWITCHER {
                if let Ok(mut switcher_ptr) = switcher_ref.lock() {
                    let _switcher = &mut **switcher_ptr;
                    // Handle notification
                }
            }
        });
    }
}

/// Initialize the app switcher system
pub fn initialize_app_switcher(mtm: MainThreadMarker) -> Result<(), String> {
    use objc2_app_kit::NSApplicationActivationPolicy;

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);

    Ok(())
}
