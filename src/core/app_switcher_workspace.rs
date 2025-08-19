// src/core/app_switcher_workspace.rs
//! Advanced workspace-based application monitoring system with deep CGWindow integration
//!
//! This module complements app_switcher_enhanced.rs by providing:
//! - Deep CGWindow tracking for all windows
//! - Browser tab detection and URL extraction  
//! - IDE/Editor file context extraction
//! - Terminal command tracking
//! - Real-time window content analysis
//! - Performance-optimized polling with smart caching

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use objc2::rc::Retained;
use objc2::{define_class, msg_send, sel, AnyThread, MainThreadMarker, Message};
use objc2_app_kit::{NSRunningApplication, NSWorkspace};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSString,
};

// Import core-foundation with proper traits
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, FromVoid, TCFType, ToVoid};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;

// Core Graphics display functions
extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGGetActiveDisplayList(
        maxDisplays: u32,
        activeDisplays: *mut u32,
        displayCount: *mut u32,
    ) -> i32;
}

// Import shared types
use crate::core::app_switcher_types::{AppInfo, AppSwitchEvent, AppSwitchListener, AppSwitchType};

// CGWindow functions
use core_foundation::array::CFArrayRef;
use core_foundation::data::CFDataRef;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
    fn CGWindowListCreateImage(
        screenBounds: CGRect,
        listOption: u32,
        windowID: u32,
        imageOption: u32,
    ) -> CFDataRef;
}

// Window list options
#[allow(non_upper_case_globals)]
const kCGWindowListOptionAll: u32 = 0;
#[allow(non_upper_case_globals)]
const kCGWindowListOptionOnScreenOnly: u32 = 1 << 0;
#[allow(non_upper_case_globals)]
const kCGWindowListExcludeDesktopElements: u32 = 1 << 4;
#[allow(non_upper_case_globals)]
const kCGWindowListOptionOnScreenAboveWindow: u32 = 1 << 1;
#[allow(non_upper_case_globals)]
const kCGWindowListOptionOnScreenBelowWindow: u32 = 1 << 2;
#[allow(non_upper_case_globals)]
const kCGWindowListOptionIncludingWindow: u32 = 1 << 3;

// Image options
#[allow(non_upper_case_globals)]
const kCGWindowImageDefault: u32 = 0;
#[allow(non_upper_case_globals)]
const kCGWindowImageBoundsIgnoreFraming: u32 = 1 << 0;
#[allow(non_upper_case_globals)]
const kCGWindowImageShouldBeOpaque: u32 = 1 << 1;
#[allow(non_upper_case_globals)]
const kCGWindowImageOnlyShadows: u32 = 1 << 2;
#[allow(non_upper_case_globals)]
const kCGWindowImageBestResolution: u32 = 1 << 3;
#[allow(non_upper_case_globals)]
const kCGWindowImageNominalResolution: u32 = 1 << 4;

/// Core Foundation CGRect structure
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

/// Detailed window information with content analysis
#[derive(Debug, Clone)]
pub struct DetailedWindowInfo {
    pub window_id: u32,
    pub title: Option<String>,
    pub owner_name: String,
    pub owner_pid: i32,
    pub layer: i32,
    pub alpha: f64,
    pub bounds: CGRect,
    pub is_onscreen: bool,
    pub is_minimized: bool,
    pub sharing_state: Option<u32>,
    pub store_type: Option<u32>,

    // Content analysis
    pub detected_url: Option<String>,
    pub detected_file_path: Option<String>,
    pub detected_tab_title: Option<String>,
    pub detected_command: Option<String>,
    pub content_hash: Option<u64>,
    pub last_content_change: Option<Instant>,
}

/// Browser tab information
#[derive(Debug, Clone)]
pub struct TabInfo {
    pub title: String,
    pub url: Option<String>,
    pub favicon_url: Option<String>,
    pub is_active: bool,
    pub tab_index: usize,
}

/// Enhanced application info with deep window analysis
#[derive(Debug, Clone)]
pub struct WorkspaceAppInfo {
    pub basic_info: AppInfo,
    pub windows: Vec<DetailedWindowInfo>,
    pub focused_window: Option<DetailedWindowInfo>,
    pub browser_tabs: Vec<TabInfo>,
    pub active_file_paths: Vec<String>,
    pub terminal_sessions: Vec<String>,
    pub window_hierarchy: Vec<u32>, // Window IDs in z-order
    pub total_screen_coverage: f64,
    pub is_fullscreen: bool,
    pub is_minimized: bool,
    pub last_interaction: Option<Instant>,
}

/// Workspace-specific app switch event
#[derive(Debug, Clone)]
pub struct WorkspaceAppSwitchEvent {
    pub timestamp: Instant,
    pub system_time: SystemTime,
    pub event_type: AppSwitchType,
    pub app_info: WorkspaceAppInfo,
    pub previous_app: Option<WorkspaceAppInfo>,
    pub window_changes: WindowChangeInfo,
    pub confidence_score: f32,
}

/// Information about window changes
#[derive(Debug, Clone)]
pub struct WindowChangeInfo {
    pub windows_created: Vec<u32>,
    pub windows_destroyed: Vec<u32>,
    pub windows_moved: Vec<u32>,
    pub windows_resized: Vec<u32>,
    pub focus_changed: bool,
    pub z_order_changed: bool,
}

/// Trait for workspace event listeners
pub trait WorkspaceAppSwitchListener: Send + Sync {
    fn on_workspace_app_switch(&mut self, event: &WorkspaceAppSwitchEvent);
    fn on_window_change(&mut self, change: &WindowChangeInfo) {}
    fn on_tab_change(&mut self, app: &str, tabs: &[TabInfo]) {}
    fn on_file_change(&mut self, app: &str, files: &[String]) {}
}

// NSWorkspace observer class with Objective-C bridged handlers
define_class!(
    #[unsafe(super(NSObject))]
    #[derive(Debug)]
    pub struct WorkspaceObserver;

    unsafe impl NSObjectProtocol for WorkspaceObserver {}

    impl WorkspaceObserver {
        #[unsafe(method(workspaceDidActivateApplication:))]
        fn workspace_did_activate_application(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "activate");
        }

        #[unsafe(method(workspaceDidDeactivateApplication:))]
        fn workspace_did_deactivate_application(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "deactivate");
        }

        #[unsafe(method(workspaceDidLaunchApplication:))]
        fn workspace_did_launch_application(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "launch");
        }

        #[unsafe(method(workspaceDidTerminateApplication:))]
        fn workspace_did_terminate_application(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "terminate");
        }

        #[unsafe(method(workspaceDidHideApplication:))]
        fn workspace_did_hide_application(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "hide");
        }

        #[unsafe(method(workspaceDidUnhideApplication:))]
        fn workspace_did_unhide_application(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "unhide");
        }

        #[unsafe(method(workspaceDidChangeFileLabels:))]
        fn workspace_did_change_file_labels(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "file_labels");
        }

        #[unsafe(method(workspaceDidMount:))]
        fn workspace_did_mount(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "mount");
        }

        #[unsafe(method(workspaceDidUnmount:))]
        fn workspace_did_unmount(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "unmount");
        }

        #[unsafe(method(workspaceDidWake:))]
        fn workspace_did_wake(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "wake");
        }

        #[unsafe(method(workspaceWillSleep:))]
        fn workspace_will_sleep(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "sleep");
        }

        #[unsafe(method(workspaceScreensDidSleep:))]
        fn workspace_screens_did_sleep(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "screens_sleep");
        }

        #[unsafe(method(workspaceScreensDidWake:))]
        fn workspace_screens_did_wake(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "screens_wake");
        }
    }
);

// Implementation details and helpers

/// Global state for the workspace monitor
static mut WORKSPACE_GLOBAL_STATE: Option<Arc<Mutex<WorkspaceState>>> = None;

struct WorkspaceState {
    current_app: Option<WorkspaceAppInfo>,
    window_cache: HashMap<u32, DetailedWindowInfo>,
    app_window_map: HashMap<i32, Vec<u32>>, // PID -> Window IDs
    listeners: Vec<Box<dyn WorkspaceAppSwitchListener>>,
    basic_listeners: Vec<Box<dyn AppSwitchListener>>,
    last_window_poll: Instant,
    observer: Option<Retained<WorkspaceObserver>>,
    notification_center: Option<Retained<NSNotificationCenter>>,
    monitoring_active: bool,
}

/// Advanced workspace application monitor
pub struct WorkspaceAppMonitor {
    state: Arc<Mutex<WorkspaceState>>,
    poll_interval: Duration,
}

impl WorkspaceAppMonitor {
    pub fn new() -> Self {
        let state = Arc::new(Mutex::new(WorkspaceState {
            current_app: None,
            window_cache: HashMap::new(),
            app_window_map: HashMap::new(),
            listeners: Vec::new(),
            basic_listeners: Vec::new(),
            last_window_poll: Instant::now(),
            observer: None,
            notification_center: None,
            monitoring_active: false,
        }));

        unsafe {
            WORKSPACE_GLOBAL_STATE = Some(state.clone());
        }

        Self {
            state,
            poll_interval: Duration::from_millis(100), // Smart polling interval
        }
    }

    pub fn add_workspace_listener<T: WorkspaceAppSwitchListener + 'static>(&mut self, listener: T) {
        let mut state = self.state.lock().unwrap();
        state.listeners.push(Box::new(listener));
    }

    pub fn add_basic_listener<T: AppSwitchListener + 'static>(&mut self, listener: T) {
        let mut state = self.state.lock().unwrap();
        state.basic_listeners.push(Box::new(listener));
    }

    pub fn start_monitoring(&mut self, _mtm: MainThreadMarker) -> Result<(), String> {
        // Fast pre-check without holding the lock long
        {
            let state = self.state.lock().unwrap();
            if state.monitoring_active {
                return Err("Already monitoring".to_string());
            }
        }

        // Create observer and register notifications without holding the mutex to avoid re-entrancy deadlocks
        let observer: Retained<WorkspaceObserver> =
            unsafe { msg_send![WorkspaceObserver::alloc(), init] };
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };
        let notification_center = unsafe { workspace.notificationCenter() };

        // Register for workspace notifications (broad coverage)
        unsafe {
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(workspaceDidActivateApplication:),
                "NSWorkspaceDidActivateApplicationNotification",
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(workspaceDidDeactivateApplication:),
                "NSWorkspaceDidDeactivateApplicationNotification",
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(workspaceDidLaunchApplication:),
                "NSWorkspaceDidLaunchApplicationNotification",
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(workspaceDidTerminateApplication:),
                "NSWorkspaceDidTerminateApplicationNotification",
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(workspaceDidHideApplication:),
                "NSWorkspaceDidHideApplicationNotification",
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(workspaceDidUnhideApplication:),
                "NSWorkspaceDidUnhideApplicationNotification",
            );
            // Additional NSWorkspace events
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(workspaceDidChangeFileLabels:),
                "NSWorkspaceDidChangeFileLabelsNotification",
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(workspaceDidMount:),
                "NSWorkspaceDidMountNotification",
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(workspaceDidUnmount:),
                "NSWorkspaceDidUnmountNotification",
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(workspaceDidWake:),
                "NSWorkspaceDidWakeNotification",
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(workspaceWillSleep:),
                "NSWorkspaceWillSleepNotification",
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(workspaceScreensDidSleep:),
                "NSWorkspaceScreensDidSleepNotification",
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(workspaceScreensDidWake:),
                "NSWorkspaceScreensDidWakeNotification",
            );
        }

        // Emit initial state BEFORE flipping the monitoring flag to ensure listeners get a first event
        self.update_current_app();

        // Now record observer and mark active under the lock
        {
            let mut state = self.state.lock().unwrap();
            state.observer = Some(observer);
            state.notification_center = Some(notification_center);
            state.monitoring_active = true;
        }

        // Start worker threads
        self.start_window_polling_thread();
        self.start_content_analysis_thread();
        self.start_frontmost_resampler();

        Ok(())
    }

    pub fn stop_monitoring(&mut self) {
        let mut state = self.state.lock().unwrap();

        if let (Some(observer), Some(nc)) = (&state.observer, &state.notification_center) {
            unsafe {
                let _: () = msg_send![&**nc, removeObserver: &**observer];
            }
        }

        state.monitoring_active = false;
        state.observer = None;
        state.notification_center = None;
    }

    fn update_current_app(&self) {
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };

        if let Some(frontmost) = unsafe { workspace.frontmostApplication() } {
            let mut state = self.state.lock().unwrap();
            let app_info = Self::extract_workspace_app_info(&frontmost, &mut state.window_cache);

            // Detect changes and notify listeners
            let changed = state
                .current_app
                .as_ref()
                .map(|c| c.basic_info.pid != app_info.basic_info.pid)
                .unwrap_or(true);

            if changed {
                let window_changes =
                    Self::detect_window_changes(&state.window_cache, &app_info.windows);
                let primary_url = app_info
                    .windows
                    .iter()
                    .filter_map(|w| w.detected_url.clone())
                    .next();
                let basic_workspace = crate::core::app_switcher_types::WorkspaceSummary {
                    window_count: app_info.windows.len(),
                    focused_title: app_info
                        .focused_window
                        .as_ref()
                        .and_then(|w| w.title.clone()),
                    total_screen_coverage: Some(app_info.total_screen_coverage),
                    is_fullscreen: Some(app_info.is_fullscreen),
                    is_minimized: Some(app_info.is_minimized),
                    tab_titles: app_info
                        .browser_tabs
                        .iter()
                        .map(|t| t.title.clone())
                        .collect(),
                    active_file_paths: app_info.active_file_paths.clone(),
                    primary_url,
                };

                let event = WorkspaceAppSwitchEvent {
                    timestamp: Instant::now(),
                    system_time: SystemTime::now(),
                    event_type: AppSwitchType::Foreground,
                    app_info: app_info.clone(),
                    previous_app: state.current_app.clone(),
                    window_changes,
                    confidence_score: 1.0,
                };

                for listener in &mut state.listeners {
                    listener.on_workspace_app_switch(&event);
                }

                // Also notify basic listeners
                let basic_event = AppSwitchEvent {
                    timestamp: Instant::now(),
                    event_type: AppSwitchType::Foreground,
                    app_info: app_info.basic_info.clone(),
                    previous_app: state.current_app.as_ref().map(|a| a.basic_info.clone()),
                    workspace: Some(basic_workspace),
                    enhanced: None,
                    confidence: Some(1.0),
                };

                for listener in &mut state.basic_listeners {
                    listener.on_app_switch(&basic_event);
                }

                state.current_app = Some(app_info);
            }
        }
    }

    fn extract_workspace_app_info(
        app: &NSRunningApplication,
        cache: &mut HashMap<u32, DetailedWindowInfo>,
    ) -> WorkspaceAppInfo {
        unsafe {
            let bundle_id = app
                .bundleIdentifier()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "unknown".to_string());

            let name = app
                .localizedName()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Unknown".to_string());

            let pid = app.processIdentifier();

            let path = app
                .bundleURL()
                .and_then(|url| url.path())
                .map(|p| p.to_string());

            let basic_info = AppInfo {
                name: name.clone(),
                bundle_id: bundle_id.clone(),
                pid,
                path,
                launch_date: app.launchDate().map(|_| Instant::now()),
                icon_base64: None,
                icon_path: None,
                activation_count: 0,
            };

            // Get all windows for this app (front-to-back order on screen)
            let (windows, primary_front_id) = Self::get_detailed_windows_for_pid(pid);
            // Choose focused window as the first encountered in global z-order
            let focused_window = primary_front_id
                .and_then(|wid| windows.iter().find(|w| w.window_id == wid).cloned())
                .or_else(|| windows.first().cloned());

            // Update cache
            for window in &windows {
                cache.insert(window.window_id, window.clone());
            }

            // Extract browser tabs if it's a browser
            let browser_tabs = if Self::is_browser(&bundle_id) {
                Self::extract_browser_tabs(&windows)
            } else {
                Vec::new()
            };

            // Extract file paths if it's an editor/IDE
            let active_file_paths = if Self::is_editor(&bundle_id) {
                Self::extract_editor_files(&windows)
            } else {
                Vec::new()
            };

            // Extract terminal sessions if it's a terminal
            let terminal_sessions = if Self::is_terminal(&bundle_id) {
                Self::extract_terminal_sessions(&windows)
            } else {
                Vec::new()
            };

            // Calculate window hierarchy and coverage
            let window_hierarchy: Vec<u32> = windows.iter().map(|w| w.window_id).collect();
            let total_screen_coverage = Self::calculate_screen_coverage(&windows);
            let is_fullscreen = windows.iter().any(|w| Self::is_window_fullscreen(w));
            let is_minimized = windows.is_empty() || windows.iter().all(|w| w.is_minimized);

            WorkspaceAppInfo {
                basic_info,
                windows,
                focused_window,
                browser_tabs,
                active_file_paths,
                terminal_sessions,
                window_hierarchy,
                total_screen_coverage,
                is_fullscreen,
                is_minimized,
                last_interaction: Some(Instant::now()),
            }
        }
    }

    fn get_detailed_windows_for_pid(pid: i32) -> (Vec<DetailedWindowInfo>, Option<u32>) {
        let mut windows = Vec::new();
        let mut first_for_pid: Option<u32> = None;

        unsafe {
            // Use on-screen only and exclude desktop elements to get front-to-back visible stack
            let options = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
            let window_list_ptr = CGWindowListCopyWindowInfo(options, 0);

            if window_list_ptr.is_null() {
                return (windows, None);
            }

            let window_list: CFArray<CFDictionary> =
                CFArray::wrap_under_create_rule(window_list_ptr as *const _);

            for i in 0..window_list.len() {
                if let Some(window_dict) = window_list.get(i) {
                    let key = CFString::from("kCGWindowOwnerPID");
                    if let Some(owner_pid_ref) = window_dict.find(key.to_void()) {
                        let owner_pid = CFNumber::from_void(*owner_pid_ref).to_i32().unwrap_or(0);

                        if owner_pid == pid {
                            if first_for_pid.is_none() {
                                // Record the first encountered window for this PID (frontmost for this owner)
                                let key_num = CFString::from("kCGWindowNumber");
                                if let Some(id_ref) = window_dict.find(key_num.to_void()) {
                                    let id_val =
                                        CFNumber::from_void(*id_ref).to_i32().unwrap_or(0) as u32;
                                    first_for_pid = Some(id_val);
                                }
                            }
                            let window = Self::extract_detailed_window_info(&window_dict);
                            windows.push(window);
                        }
                    }
                }
            }
        }

        (windows, first_for_pid)
    }

    fn extract_detailed_window_info(dict: &CFDictionary) -> DetailedWindowInfo {
        let window_id = {
            let key = CFString::from("kCGWindowNumber");
            if let Some(value_ptr) = dict.find(key.to_void()) {
                unsafe { CFNumber::from_void(*value_ptr) }
                    .to_i32()
                    .unwrap_or(0) as u32
            } else {
                0
            }
        };

        let title = {
            let key = CFString::from("kCGWindowName");
            if let Some(value_ptr) = dict.find(key.to_void()) {
                Some(unsafe { CFString::from_void(*value_ptr) }.to_string())
            } else {
                None
            }
        };

        let owner_name = {
            let key = CFString::from("kCGWindowOwnerName");
            if let Some(value_ptr) = dict.find(key.to_void()) {
                unsafe { CFString::from_void(*value_ptr) }.to_string()
            } else {
                "Unknown".to_string()
            }
        };

        let owner_pid = {
            let key = CFString::from("kCGWindowOwnerPID");
            if let Some(value_ptr) = dict.find(key.to_void()) {
                unsafe { CFNumber::from_void(*value_ptr) }
                    .to_i32()
                    .unwrap_or(0)
            } else {
                0
            }
        };

        let layer = {
            let key = CFString::from("kCGWindowLayer");
            if let Some(value_ptr) = dict.find(key.to_void()) {
                unsafe { CFNumber::from_void(*value_ptr) }
                    .to_i32()
                    .unwrap_or(0)
            } else {
                0
            }
        };

        let alpha = {
            let key = CFString::from("kCGWindowAlpha");
            if let Some(value_ptr) = dict.find(key.to_void()) {
                unsafe { CFNumber::from_void(*value_ptr) }
                    .to_f64()
                    .unwrap_or(1.0)
            } else {
                1.0
            }
        };

        let is_onscreen = {
            let key = CFString::from("kCGWindowIsOnscreen");
            if let Some(value_ptr) = dict.find(key.to_void()) {
                let item = unsafe { CFBoolean::from_void(*value_ptr) };
                let cf_bool = (*item).clone();
                bool::from(cf_bool)
            } else {
                false
            }
        };

        let sharing_state = {
            let key = CFString::from("kCGWindowSharingState");
            if let Some(value_ptr) = dict.find(key.to_void()) {
                Some(
                    unsafe { CFNumber::from_void(*value_ptr) }
                        .to_i32()
                        .unwrap_or(0) as u32,
                )
            } else {
                None
            }
        };

        let store_type = {
            let key = CFString::from("kCGWindowStoreType");
            if let Some(value_ptr) = dict.find(key.to_void()) {
                Some(
                    unsafe { CFNumber::from_void(*value_ptr) }
                        .to_i32()
                        .unwrap_or(0) as u32,
                )
            } else {
                None
            }
        };

        // Extract bounds
        let bounds = {
            let key = CFString::from("kCGWindowBounds");
            if let Some(bounds_dict_ptr) = dict.find(key.to_void()) {
                let bounds_dict =
                    unsafe { CFDictionary::<CFString, CFType>::from_void(*bounds_dict_ptr) };

                let x = (*bounds_dict)
                    .find(&CFString::from("X"))
                    .and_then(|n| n.downcast::<CFNumber>())
                    .map(|num| num.to_f64().unwrap_or(0.0))
                    .unwrap_or(0.0);

                let y = (*bounds_dict)
                    .find(&CFString::from("Y"))
                    .and_then(|n| n.downcast::<CFNumber>())
                    .map(|num| num.to_f64().unwrap_or(0.0))
                    .unwrap_or(0.0);

                let width = (*bounds_dict)
                    .find(&CFString::from("Width"))
                    .and_then(|n| n.downcast::<CFNumber>())
                    .map(|num| num.to_f64().unwrap_or(0.0))
                    .unwrap_or(0.0);

                let height = (*bounds_dict)
                    .find(&CFString::from("Height"))
                    .and_then(|n| n.downcast::<CFNumber>())
                    .map(|num| num.to_f64().unwrap_or(0.0))
                    .unwrap_or(0.0);

                CGRect {
                    origin: CGPoint { x, y },
                    size: CGSize { width, height },
                }
            } else {
                CGRect {
                    origin: CGPoint { x: 0.0, y: 0.0 },
                    size: CGSize {
                        width: 0.0,
                        height: 0.0,
                    },
                }
            }
        };

        // Try to detect content based on window title
        let (detected_url, detected_file_path, detected_tab_title, detected_command) =
            Self::analyze_window_title(&title, &owner_name);

        DetailedWindowInfo {
            window_id,
            title,
            owner_name,
            owner_pid,
            layer,
            alpha,
            bounds,
            is_onscreen,
            is_minimized: !is_onscreen && alpha < 0.1,
            sharing_state,
            store_type,
            detected_url,
            detected_file_path,
            detected_tab_title,
            detected_command,
            content_hash: None,
            last_content_change: None,
        }
    }

    fn analyze_window_title(
        title: &Option<String>,
        owner: &str,
    ) -> (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        let mut url = None;
        let mut file_path = None;
        let mut tab_title = None;
        let mut command = None;

        if let Some(title_str) = title {
            // Detect URLs in browser windows
            if Self::is_browser(owner) {
                if let Some(dash_pos) = title_str.rfind(" — ") {
                    tab_title = Some(title_str[..dash_pos].to_string());
                    // Try to extract URL from browser title patterns
                    if title_str.contains("http://") || title_str.contains("https://") {
                        url = Some(title_str.to_string());
                    }
                } else if let Some(dash_pos) = title_str.rfind(" - ") {
                    tab_title = Some(title_str[..dash_pos].to_string());
                }
            }

            // Detect file paths in editor windows
            if Self::is_editor(owner) {
                if title_str.contains("/") || title_str.contains("\\") {
                    file_path = Some(title_str.to_string());
                } else if let Some(dash_pos) = title_str.find(" — ") {
                    file_path = Some(title_str[..dash_pos].to_string());
                }
            }

            // Detect terminal commands
            if Self::is_terminal(owner) {
                if title_str.contains("—") {
                    if let Some(cmd_part) = title_str.split("—").last() {
                        command = Some(cmd_part.trim().to_string());
                    }
                }
            }
        }

        (url, file_path, tab_title, command)
    }

    fn is_browser(app: &str) -> bool {
        let browsers = [
            "Safari",
            "Chrome",
            "Firefox",
            "Edge",
            "Opera",
            "Brave",
            "Arc",
            "Vivaldi",
            "com.apple.Safari",
            "com.google.Chrome",
            "org.mozilla.firefox",
        ];
        browsers.iter().any(|b| app.contains(b))
    }

    fn is_editor(app: &str) -> bool {
        let editors = [
            "Code", "VSCode", "Sublime", "Atom", "TextEdit", "Xcode", "IntelliJ", "WebStorm",
            "PyCharm", "RubyMine", "GoLand", "Cursor", "Zed", "Nova", "BBEdit", "TextMate", "Vim",
            "Neovim", "Emacs",
        ];
        editors.iter().any(|e| app.contains(e))
    }

    fn is_terminal(app: &str) -> bool {
        let terminals = [
            "Terminal",
            "iTerm",
            "Hyper",
            "Alacritty",
            "kitty",
            "WezTerm",
            "com.apple.Terminal",
        ];
        terminals.iter().any(|t| app.contains(t))
    }

    fn extract_browser_tabs(windows: &[DetailedWindowInfo]) -> Vec<TabInfo> {
        windows
            .iter()
            .filter_map(|w| {
                if let Some(tab_title) = &w.detected_tab_title {
                    Some(TabInfo {
                        title: tab_title.clone(),
                        url: w.detected_url.clone(),
                        favicon_url: None,
                        is_active: w.is_onscreen,
                        tab_index: 0,
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    fn extract_editor_files(windows: &[DetailedWindowInfo]) -> Vec<String> {
        windows
            .iter()
            .filter_map(|w| w.detected_file_path.clone())
            .collect()
    }

    fn extract_terminal_sessions(windows: &[DetailedWindowInfo]) -> Vec<String> {
        windows
            .iter()
            .filter_map(|w| w.detected_command.clone())
            .collect()
    }

    fn calculate_screen_coverage(windows: &[DetailedWindowInfo]) -> f64 {
        // Get actual screen dimensions from all displays
        let screen_area = Self::get_total_screen_area();

        if screen_area == 0.0 {
            return 0.0;
        }

        // Calculate visible window area (handling overlaps)
        let mut visible_area = 0.0;
        let mut sorted_windows: Vec<_> = windows
            .iter()
            .filter(|w| w.is_onscreen && w.alpha > 0.1)
            .collect();

        // Sort by layer (lower layer = behind)
        sorted_windows.sort_by_key(|w| w.layer);

        // For simplicity, just sum areas (proper implementation would handle overlaps)
        // In practice, you'd use a more sophisticated algorithm like a sweep line
        for window in sorted_windows {
            let window_area = window.bounds.size.width * window.bounds.size.height;
            // Apply alpha as coverage factor
            visible_area += window_area * window.alpha;
        }

        (visible_area / screen_area).min(1.0)
    }

    fn get_total_screen_area() -> f64 {
        unsafe {
            // Get main display bounds
            let main_display_id = CGMainDisplayID();
            let main_bounds = CGDisplayBounds(main_display_id);

            // Get all active displays
            let max_displays = 32;
            let mut display_count: u32 = 0;
            let mut display_ids = vec![0u32; max_displays];

            let result = CGGetActiveDisplayList(
                max_displays as u32,
                display_ids.as_mut_ptr(),
                &mut display_count,
            );

            if result != 0 {
                // Fallback to main display only
                return main_bounds.size.width * main_bounds.size.height;
            }

            // Calculate total area from all displays
            let mut total_area = 0.0;
            for i in 0..display_count as usize {
                let display_id = display_ids[i];
                let bounds = CGDisplayBounds(display_id);
                total_area += bounds.size.width * bounds.size.height;
            }

            // Return total area or main display area as fallback
            if total_area > 0.0 {
                total_area
            } else {
                main_bounds.size.width * main_bounds.size.height
            }
        }
    }

    fn is_window_fullscreen(window: &DetailedWindowInfo) -> bool {
        unsafe {
            // Get the display that contains most of the window
            let window_center_x = window.bounds.origin.x + window.bounds.size.width / 2.0;
            let window_center_y = window.bounds.origin.y + window.bounds.size.height / 2.0;

            // Get all active displays and find which one contains this window
            let max_displays = 32;
            let mut display_count: u32 = 0;
            let mut display_ids = vec![0u32; max_displays];

            let result = CGGetActiveDisplayList(
                max_displays as u32,
                display_ids.as_mut_ptr(),
                &mut display_count,
            );

            if result != 0 {
                // Fallback to simple check
                return window.bounds.size.width >= 1920.0 * 0.95
                    && window.bounds.size.height >= 1080.0 * 0.95;
            }

            // Find the display containing the window center
            for i in 0..display_count as usize {
                let display_id = display_ids[i];
                let bounds = CGDisplayBounds(display_id);

                // Check if this display contains the window center
                if window_center_x >= bounds.origin.x
                    && window_center_x <= bounds.origin.x + bounds.size.width
                    && window_center_y >= bounds.origin.y
                    && window_center_y <= bounds.origin.y + bounds.size.height
                {
                    // Check if window covers most of this display
                    let coverage_threshold = 0.90; // 90% coverage
                    return window.bounds.size.width >= bounds.size.width * coverage_threshold
                        && window.bounds.size.height >= bounds.size.height * coverage_threshold;
                }
            }

            // If we couldn't find the display, use main display
            let main_bounds = CGDisplayBounds(CGMainDisplayID());
            window.bounds.size.width >= main_bounds.size.width * 0.90
                && window.bounds.size.height >= main_bounds.size.height * 0.90
        }
    }

    fn detect_window_changes(
        cache: &HashMap<u32, DetailedWindowInfo>,
        current: &[DetailedWindowInfo],
    ) -> WindowChangeInfo {
        let old_ids: HashSet<u32> = cache.keys().cloned().collect();
        let new_ids: HashSet<u32> = current.iter().map(|w| w.window_id).collect();

        let windows_created: Vec<u32> = new_ids.difference(&old_ids).cloned().collect();
        let windows_destroyed: Vec<u32> = old_ids.difference(&new_ids).cloned().collect();

        // Detect moved/resized windows
        let mut windows_moved = Vec::new();
        let mut windows_resized = Vec::new();

        for window in current {
            if let Some(old_window) = cache.get(&window.window_id) {
                if (window.bounds.origin.x - old_window.bounds.origin.x).abs() > 1.0
                    || (window.bounds.origin.y - old_window.bounds.origin.y).abs() > 1.0
                {
                    windows_moved.push(window.window_id);
                }
                if (window.bounds.size.width - old_window.bounds.size.width).abs() > 1.0
                    || (window.bounds.size.height - old_window.bounds.size.height).abs() > 1.0
                {
                    windows_resized.push(window.window_id);
                }
            }
        }

        let focus_changed = !windows_created.is_empty() || !windows_destroyed.is_empty();
        WindowChangeInfo {
            windows_created,
            windows_destroyed,
            windows_moved,
            windows_resized,
            focus_changed,
            z_order_changed: false, // Would need to track z-order
        }
    }

    fn start_window_polling_thread(&self) {
        let state = self.state.clone();
        let poll_interval = self.poll_interval;

        thread::spawn(move || {
            loop {
                thread::sleep(poll_interval);
                // Exit if monitoring stopped
                {
                    let st = state.lock().unwrap();
                    if !st.monitoring_active {
                        break;
                    }
                }

                let do_poll = {
                    let st = state.lock().unwrap();
                    st.last_window_poll.elapsed() >= poll_interval
                };
                if !do_poll {
                    continue;
                }

                // Poll windows and detect changes
                let (pid_opt, cache_snapshot) = {
                    let st = state.lock().unwrap();
                    (
                        st.current_app.as_ref().map(|a| a.basic_info.pid),
                        st.window_cache.clone(),
                    )
                };
                if let Some(pid) = pid_opt {
                    let (new_windows, _front_id) = Self::get_detailed_windows_for_pid(pid);
                    let changes = Self::detect_window_changes(&cache_snapshot, &new_windows);

                    // Notify listeners and update cache under one mutable lock
                    let mut st = state.lock().unwrap();
                    if changes.focus_changed
                        || !changes.windows_moved.is_empty()
                        || !changes.windows_resized.is_empty()
                    {
                        for listener in &mut st.listeners {
                            listener.on_window_change(&changes);
                        }
                    }
                    st.app_window_map
                        .insert(pid, new_windows.iter().map(|w| w.window_id).collect());
                    for window in &new_windows {
                        st.window_cache.insert(window.window_id, window.clone());
                    }
                    st.last_window_poll = Instant::now();
                } else {
                    let mut st = state.lock().unwrap();
                    st.last_window_poll = Instant::now();
                }
            }
        });
    }

    fn start_frontmost_resampler(&self) {
        let state = self.state.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(150));
            // Exit if monitoring stopped
            {
                let st = state.lock().unwrap();
                if !st.monitoring_active {
                    break;
                }
            }
            WorkspaceAppMonitor::resample_frontmost(&state);
        });
    }

    fn resample_frontmost(state: &Arc<Mutex<WorkspaceState>>) {
        unsafe {
            let workspace = NSWorkspace::sharedWorkspace();
            if let Some(frontmost) = workspace.frontmostApplication() {
                // Snapshot cache to avoid overlapping borrows
                let cache_snapshot = { state.lock().unwrap().window_cache.clone() };
                let mut tmp_cache = cache_snapshot.clone();
                let app_info =
                    WorkspaceAppMonitor::extract_workspace_app_info(&frontmost, &mut tmp_cache);

                let (changed, previous_app) = {
                    let st = state.lock().unwrap();
                    (
                        st.current_app
                            .as_ref()
                            .map(|c| c.basic_info.pid != app_info.basic_info.pid)
                            .unwrap_or(true),
                        st.current_app.clone(),
                    )
                };

                if changed {
                    let window_changes = WorkspaceAppMonitor::detect_window_changes(
                        &cache_snapshot,
                        &app_info.windows,
                    );
                    // Update PID -> window map
                    let mut st = state.lock().unwrap();
                    st.app_window_map.insert(
                        app_info.basic_info.pid,
                        app_info.windows.iter().map(|w| w.window_id).collect(),
                    );

                    let event = WorkspaceAppSwitchEvent {
                        timestamp: Instant::now(),
                        system_time: SystemTime::now(),
                        event_type: AppSwitchType::Foreground,
                        app_info: app_info.clone(),
                        previous_app,
                        window_changes,
                        confidence_score: 0.9,
                    };

                    for listener in &mut st.listeners {
                        listener.on_workspace_app_switch(&event);
                    }

                    st.current_app = Some(app_info);
                }
            }
        }
    }

    fn start_content_analysis_thread(&self) {
        let state = self.state.clone();

        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(1));

                let should_analyze = {
                    let state = state.lock().unwrap();
                    state.monitoring_active
                };

                if should_analyze {
                    // Analyze current app's content
                    let mut state = state.lock().unwrap();
                    let mut pending_tab_change: Option<(String, Vec<TabInfo>)> = None;
                    let mut pending_file_change: Option<(String, Vec<String>)> = None;
                    if let Some(current_app) = &state.current_app {
                        if Self::is_browser(&current_app.basic_info.bundle_id) {
                            let new_tabs = Self::extract_browser_tabs(&current_app.windows);
                            if new_tabs.len() != current_app.browser_tabs.len() {
                                pending_tab_change =
                                    Some((current_app.basic_info.name.clone(), new_tabs));
                            }
                        }
                        if Self::is_editor(&current_app.basic_info.bundle_id) {
                            let new_files = Self::extract_editor_files(&current_app.windows);
                            if new_files.len() != current_app.active_file_paths.len() {
                                pending_file_change =
                                    Some((current_app.basic_info.name.clone(), new_files));
                            }
                        }
                    }
                    if let Some((app_name, tabs)) = pending_tab_change {
                        for listener in &mut state.listeners {
                            listener.on_tab_change(&app_name, &tabs);
                        }
                    }
                    if let Some((app_name, files)) = pending_file_change {
                        for listener in &mut state.listeners {
                            listener.on_file_change(&app_name, &files);
                        }
                    }
                }

                if !should_analyze {
                    break;
                }
            }
        });
    }

    unsafe fn register_notification(
        nc: &NSNotificationCenter,
        observer: &WorkspaceObserver,
        selector: objc2::runtime::Sel,
        name: &str,
    ) {
        let notification_name = NSString::from_str(name);
        let _: () = msg_send![
            nc,
            addObserver: observer,
            selector: selector,
            name: &*notification_name,
            object: std::ptr::null::<NSObject>()
        ];
    }

    pub fn current_app(&self) -> Option<WorkspaceAppInfo> {
        let state = self.state.lock().unwrap();
        state.current_app.clone()
    }

    pub fn get_all_windows(&self) -> Vec<DetailedWindowInfo> {
        let state = self.state.lock().unwrap();
        state.window_cache.values().cloned().collect()
    }

    pub fn get_windows_for_app(&self, pid: i32) -> Vec<DetailedWindowInfo> {
        let (wins, _front) = Self::get_detailed_windows_for_pid(pid);
        wins
    }

    /// Public trigger to resample the current frontmost and emit if changed
    pub fn resample_now(&self) {
        Self::resample_frontmost(&self.state);
    }

    pub fn take_window_screenshot(&self, window_id: u32) -> Option<Vec<u8>> {
        // Would implement window screenshot capture
        None
    }
}

impl WorkspaceObserver {
    fn handle_notification(notification: &NSNotification, event_type: &str) {
        unsafe {
            if let Some(global) = &WORKSPACE_GLOBAL_STATE {
                let app = Self::get_app_from_notification(notification);
                if let Some(app) = app {
                    let mut state = global.lock().unwrap();
                    let app_info = WorkspaceAppMonitor::extract_workspace_app_info(
                        &app,
                        &mut state.window_cache,
                    );

                    let event = WorkspaceAppSwitchEvent {
                        timestamp: Instant::now(),
                        system_time: SystemTime::now(),
                        event_type: match event_type {
                            "activate" => AppSwitchType::Foreground,
                            "deactivate" => AppSwitchType::Background,
                            "launch" => AppSwitchType::Launch,
                            "terminate" => AppSwitchType::Terminate,
                            "hide" => AppSwitchType::Hide,
                            "unhide" => AppSwitchType::Unhide,
                            _ => AppSwitchType::Foreground,
                        },
                        app_info: app_info.clone(),
                        previous_app: state.current_app.clone(),
                        window_changes: WindowChangeInfo {
                            windows_created: Vec::new(),
                            windows_destroyed: Vec::new(),
                            windows_moved: Vec::new(),
                            windows_resized: Vec::new(),
                            focus_changed: true,
                            z_order_changed: false,
                        },
                        confidence_score: 1.0,
                    };

                    for listener in &mut state.listeners {
                        listener.on_workspace_app_switch(&event);
                    }

                    state.current_app = Some(app_info);
                }
            }
        }
    }

    unsafe fn get_app_from_notification(
        notification: &NSNotification,
    ) -> Option<Retained<NSRunningApplication>> {
        if let Some(user_info) = unsafe { notification.userInfo() } {
            let key = NSString::from_str("NSWorkspaceApplicationKey");
            if let Some(app_obj) = user_info.objectForKey(&key) {
                let app: Retained<NSRunningApplication> =
                    unsafe { Retained::cast_unchecked(app_obj.retain()) };
                return Some(app);
            }
        }
        None
    }
}

impl Drop for WorkspaceAppMonitor {
    fn drop(&mut self) {
        self.stop_monitoring();
    }
}

/// Debug listener for workspace events
pub struct WorkspaceDebugListener;

impl WorkspaceAppSwitchListener for WorkspaceDebugListener {
    fn on_workspace_app_switch(&mut self, event: &WorkspaceAppSwitchEvent) {
        println!("🔄 Workspace App Switch:");
        println!(
            "  App: {} ({})",
            event.app_info.basic_info.name, event.app_info.basic_info.bundle_id
        );
        println!("  Windows: {}", event.app_info.windows.len());
        println!(
            "  Coverage: {:.1}%",
            event.app_info.total_screen_coverage * 100.0
        );

        if !event.app_info.browser_tabs.is_empty() {
            println!("  Browser Tabs:");
            for tab in &event.app_info.browser_tabs {
                println!("    - {}", tab.title);
                if let Some(url) = &tab.url {
                    println!("      URL: {}", url);
                }
            }
        }

        if !event.app_info.active_file_paths.is_empty() {
            println!("  Open Files:");
            for file in &event.app_info.active_file_paths {
                println!("    - {}", file);
            }
        }

        if !event.app_info.terminal_sessions.is_empty() {
            println!("  Terminal Commands:");
            for cmd in &event.app_info.terminal_sessions {
                println!("    $ {}", cmd);
            }
        }
    }

    fn on_window_change(&mut self, _change: &WindowChangeInfo) {}

    fn on_tab_change(&mut self, _app: &str, _tabs: &[TabInfo]) {}

    fn on_file_change(&mut self, _app: &str, _files: &[String]) {}
}
