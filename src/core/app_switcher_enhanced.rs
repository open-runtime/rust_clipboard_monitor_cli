// src/core/app_switcher_enhanced.rs
//! Enhanced multi-layer application monitoring system with maximum information extraction
//!
//! This implements a robust, low-latency app switching detector with:
//! - Layer 1: NSWorkspace notifications (primary, zero polling)
//! - Layer 2: CGWindow cross-checking for validation and window info
//! - Layer 3: Event coalescing to handle rapid switches
//! - Layer 4: Space/sleep/wake transition handling
//! - Layer 5: Process and system information extraction
//! - Layer 6: Window metadata and desktop state

use std::collections::HashMap;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use objc2::rc::Retained;
use objc2::{define_class, msg_send, sel, AnyThread, MainThreadMarker, Message};
use objc2_app_kit::{NSImage, NSRunningApplication, NSWorkspace};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSString,
};

// Import core-foundation traits
use crate::core::spaces::{query_spaces, SpacesSnapshot};
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, FromVoid, TCFType, ToVoid};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;

// Process information
use sysinfo::{Pid as SysPid, ProcessesToUpdate, System};

// CGWindow and system functions
use core_foundation::array::CFArrayRef;
use core_foundation::dictionary::CFDictionaryRef;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
    fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
    // Displays
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGDisplayRect;
    fn CGGetActiveDisplayList(
        maxDisplays: u32,
        activeDisplays: *mut u32,
        displayCount: *mut u32,
    ) -> i32;
}

// Window list options
#[allow(non_upper_case_globals)]
const kCGWindowListOptionOnScreenOnly: u32 = 1 << 0;
#[allow(non_upper_case_globals)]
const kCGWindowListExcludeDesktopElements: u32 = 1 << 4;
#[allow(non_upper_case_globals)]
const kCGWindowListOptionIncludingWindow: u32 = 1 << 3;
#[allow(non_upper_case_globals)]
const kCGWindowListOptionAll: u32 = 0;

/// Minimal CoreGraphics CGRect equivalent for display bounds in this module
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CGDisplayRect {
    pub origin: CGDisplayPoint,
    pub size: CGDisplaySize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CGDisplayPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CGDisplaySize {
    pub width: f64,
    pub height: f64,
}

// Notification names
const WORKSPACE_DID_ACTIVATE_APP: &str = "NSWorkspaceDidActivateApplicationNotification";
const WORKSPACE_DID_DEACTIVATE_APP: &str = "NSWorkspaceDidDeactivateApplicationNotification";
const WORKSPACE_DID_LAUNCH_APP: &str = "NSWorkspaceDidLaunchApplicationNotification";
const WORKSPACE_DID_TERMINATE_APP: &str = "NSWorkspaceDidTerminateApplicationNotification";
const WORKSPACE_DID_HIDE_APP: &str = "NSWorkspaceDidHideApplicationNotification";
const WORKSPACE_DID_UNHIDE_APP: &str = "NSWorkspaceDidUnhideApplicationNotification";
const WORKSPACE_ACTIVE_SPACE_CHANGED: &str = "NSWorkspaceActiveSpaceDidChangeNotification";
const WORKSPACE_SESSION_DID_BECOME_ACTIVE: &str = "NSWorkspaceSessionDidBecomeActiveNotification";
const WORKSPACE_SESSION_DID_RESIGN_ACTIVE: &str = "NSWorkspaceSessionDidResignActiveNotification";
const WORKSPACE_DID_WAKE: &str = "NSWorkspaceDidWakeNotification";
const WORKSPACE_SCREEN_CHANGED: &str = "NSApplicationDidChangeScreenParametersNotification";

/// Window information extracted from CGWindow
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub window_id: u32,
    pub title: Option<String>,
    pub bounds: WindowBounds,
    pub layer: i32,
    pub alpha: f64,
    pub memory_usage: Option<u64>,
    pub sharing_state: Option<u32>,
    pub backing_store_type: Option<String>,
    pub is_onscreen: bool,
}

#[derive(Debug, Clone)]
pub struct WindowBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Process information from system
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub cpu_usage: f32,
    pub memory_bytes: u64,
    pub virtual_memory_bytes: u64,
    pub num_threads: usize,
    pub start_time: Option<SystemTime>,
    pub parent_pid: Option<i32>,
    pub environment: HashMap<String, String>,
    pub command_line: Vec<String>,
    pub executable_path: Option<PathBuf>,
    pub working_directory: Option<PathBuf>,
}

/// Desktop state information
#[derive(Debug, Clone)]
pub struct DesktopState {
    pub active_space_id: Option<u32>,
    pub display_count: u32,
    pub session_active: bool,
    pub screen_locked: bool,
    pub console_user: Option<String>,
    pub login_time: Option<SystemTime>,
    pub idle_time_seconds: Option<f64>,
    // New fields from Spaces snapshot
    pub active_space_uuid: Option<String>,
    pub active_space_index: Option<u32>,
    pub active_space_type: Option<String>,
    pub active_space_name: Option<String>,
    pub active_space_label: Option<String>,
}

/// Extended application information with maximum detail
#[derive(Debug, Clone)]
pub struct ExtendedAppInfo {
    // Basic info
    pub name: String,
    pub bundle_id: String,
    pub pid: i32,
    pub path: Option<String>,
    pub executable_path: Option<String>,
    pub launch_date: Option<Instant>,

    // Visual info
    pub icon_base64: Option<String>,
    pub icon_path: Option<String>,

    // State info
    pub is_active: bool,
    pub is_hidden: bool,
    pub is_terminated: bool,
    pub activation_policy: String,
    pub activation_count: u32,

    // Window info
    pub windows: Vec<WindowInfo>,
    pub frontmost_window: Option<WindowInfo>,
    pub window_count: usize,

    // Display info for front window
    pub front_window_display_id: Option<u32>,

    // Process info
    pub process_info: Option<ProcessInfo>,

    // Additional metadata
    pub bundle_version: Option<String>,
    pub bundle_short_version: Option<String>,
    pub minimum_system_version: Option<String>,
    pub category: Option<String>,
    pub developer: Option<String>,
}

/// Enhanced app switch event with comprehensive information
#[derive(Debug, Clone)]
pub struct EnhancedAppSwitchEvent {
    pub timestamp: Instant,
    pub system_time: SystemTime,
    pub event_type: AppSwitchType,
    pub app_info: ExtendedAppInfo,
    pub previous_app: Option<ExtendedAppInfo>,
    pub desktop_state: DesktopState,
    pub trigger_source: TriggerSource,
    pub confidence_score: f32,
}

#[derive(Debug, Clone)]
pub enum AppSwitchType {
    Foreground,
    Background,
    Launch,
    Terminate,
    Hide,
    Unhide,
    SpaceChange,
    SessionChange,
    WakeFromSleep,
}

#[derive(Debug, Clone)]
pub enum TriggerSource {
    NSWorkspaceNotification,
    CGWindowVerification,
    SpaceTransition,
    SessionTransition,
    WakeEvent,
    ManualResample,
    EventCoalescing,
}

/// Trait for listening to enhanced app switch events
pub trait EnhancedAppSwitchListener: Send + Sync {
    fn on_app_switch(&mut self, event: &EnhancedAppSwitchEvent);
    fn on_monitoring_started(&mut self) {}
    fn on_monitoring_stopped(&mut self) {}
    fn on_desktop_state_change(&mut self, state: &DesktopState) {}
}

// Define the NSWorkspace observer class with bridged Objective-C methods
define_class!(
    #[unsafe(super(NSObject))]
    #[derive(Debug)]
    pub struct EnhancedWorkspaceObserver;

    unsafe impl NSObjectProtocol for EnhancedWorkspaceObserver {}

    impl EnhancedWorkspaceObserver {
        // These methods will be called via selectors registered with NSNotificationCenter
        #[unsafe(method(appDidActivate:))]
        fn app_did_activate(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "activate");
        }

        #[unsafe(method(appDidDeactivate:))]
        fn app_did_deactivate(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "deactivate");
        }

        #[unsafe(method(appDidLaunch:))]
        fn app_did_launch(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "launch");
        }

        #[unsafe(method(appDidTerminate:))]
        fn app_did_terminate(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "terminate");
        }

        #[unsafe(method(appDidHide:))]
        fn app_did_hide(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "hide");
        }

        #[unsafe(method(appDidUnhide:))]
        fn app_did_unhide(&self, notification: &NSNotification) {
            Self::handle_notification(notification, "unhide");
        }

        #[unsafe(method(spaceDidChange:))]
        fn space_did_change(&self, _notification: &NSNotification) {
            Self::handle_space_change();
        }

        #[unsafe(method(sessionDidBecomeActive:))]
        fn session_did_become_active(&self, _notification: &NSNotification) {
            Self::handle_session_change(true);
        }

        #[unsafe(method(sessionDidResignActive:))]
        fn session_did_resign_active(&self, _notification: &NSNotification) {
            Self::handle_session_change(false);
        }

        #[unsafe(method(didWake:))]
        fn did_wake(&self, _notification: &NSNotification) {
            Self::handle_wake();
        }

        #[unsafe(method(screenParametersChanged:))]
        fn screen_parameters_changed(&self, _notification: &NSNotification) {
            Self::handle_screen_change();
        }
    }
);

/// State for event coalescing and tracking
struct CoalescingState {
    last_deactivate: Option<(ExtendedAppInfo, Instant)>,
    last_activate: Option<(ExtendedAppInfo, Instant)>,
    pending_events: Vec<(EnhancedAppSwitchEvent, Instant)>,
}

/// Global state (using Arc<Mutex> for thread safety)
static mut GLOBAL_STATE: Option<Arc<Mutex<EnhancedState>>> = None;

struct EnhancedState {
    current_app: Option<ExtendedAppInfo>,
    listeners: Vec<Box<dyn EnhancedAppSwitchListener>>,
    coalescing: CoalescingState,
    observer: Option<Retained<EnhancedWorkspaceObserver>>,
    notification_center: Option<Retained<NSNotificationCenter>>,
    activation_counts: HashMap<String, u32>,
    last_event_time: Instant,
    system_info: System,
    desktop_state: DesktopState,
}

/// Enhanced app switcher with multi-layer monitoring
pub struct EnhancedAppSwitcher {
    state: Arc<Mutex<EnhancedState>>,
}

impl EnhancedAppSwitcher {
    pub fn new() -> Self {
        let state = Arc::new(Mutex::new(EnhancedState {
            current_app: None,
            listeners: Vec::new(),
            coalescing: CoalescingState {
                last_deactivate: None,
                last_activate: None,
                pending_events: Vec::new(),
            },
            observer: None,
            notification_center: None,
            activation_counts: HashMap::new(),
            last_event_time: Instant::now(),
            system_info: System::new_all(),
            desktop_state: DesktopState {
                active_space_id: None,
                display_count: 0,
                session_active: true,
                screen_locked: false,
                console_user: None,
                login_time: None,
                idle_time_seconds: None,
                active_space_uuid: None,
                active_space_index: None,
                active_space_type: None,
                active_space_name: None,
                active_space_label: None,
            },
        }));

        unsafe {
            GLOBAL_STATE = Some(state.clone());
        }

        Self { state }
    }

    pub fn add_listener<T: EnhancedAppSwitchListener + 'static>(&mut self, listener: T) {
        let mut state = self.state.lock().unwrap();
        state.listeners.push(Box::new(listener));
    }

    pub fn start_monitoring(&mut self, _mtm: MainThreadMarker) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();

        // Create observer
        let observer: Retained<EnhancedWorkspaceObserver> =
            unsafe { msg_send![EnhancedWorkspaceObserver::alloc(), init] };

        // Get NSWorkspace notification center
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };
        let notification_center = unsafe { workspace.notificationCenter() };

        // Register for all workspace notifications
        unsafe {
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(appDidActivate:),
                WORKSPACE_DID_ACTIVATE_APP,
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(appDidDeactivate:),
                WORKSPACE_DID_DEACTIVATE_APP,
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(appDidLaunch:),
                WORKSPACE_DID_LAUNCH_APP,
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(appDidTerminate:),
                WORKSPACE_DID_TERMINATE_APP,
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(appDidHide:),
                WORKSPACE_DID_HIDE_APP,
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(appDidUnhide:),
                WORKSPACE_DID_UNHIDE_APP,
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(spaceDidChange:),
                WORKSPACE_ACTIVE_SPACE_CHANGED,
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(sessionDidBecomeActive:),
                WORKSPACE_SESSION_DID_BECOME_ACTIVE,
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(sessionDidResignActive:),
                WORKSPACE_SESSION_DID_RESIGN_ACTIVE,
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(didWake:),
                WORKSPACE_DID_WAKE,
            );
            Self::register_notification(
                &notification_center,
                &observer,
                sel!(screenParametersChanged:),
                WORKSPACE_SCREEN_CHANGED,
            );
        }

        // Update desktop state
        state.desktop_state = Self::capture_desktop_state();

        // Seed with current frontmost app
        if let Some(frontmost) = unsafe { workspace.frontmostApplication() } {
            let app_info = Self::extract_extended_app_info(&frontmost, &mut state.system_info);
            state.current_app = Some(app_info.clone());

            // Notify listeners of initial state
            for listener in &mut state.listeners {
                listener.on_monitoring_started();
            }
        }

        state.observer = Some(observer);
        state.notification_center = Some(notification_center);

        // Start coalescing timer
        Self::start_coalescing_timer();

        // Start system info refresh timer
        Self::start_system_refresh_timer();

        Ok(())
    }

    pub fn stop_monitoring(&mut self) {
        let mut state = self.state.lock().unwrap();

        if let (Some(observer), Some(nc)) = (&state.observer, &state.notification_center) {
            unsafe {
                let _: () = msg_send![&**nc, removeObserver: &**observer];
            }
        }

        for listener in &mut state.listeners {
            listener.on_monitoring_stopped();
        }

        state.observer = None;
        state.notification_center = None;
    }

    pub fn current_app(&self) -> Option<ExtendedAppInfo> {
        let state = self.state.lock().unwrap();
        state.current_app.clone()
    }

    pub fn desktop_state(&self) -> DesktopState {
        let state = self.state.lock().unwrap();
        state.desktop_state.clone()
    }

    unsafe fn register_notification(
        nc: &NSNotificationCenter,
        observer: &EnhancedWorkspaceObserver,
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

    fn extract_extended_app_info(app: &NSRunningApplication, sys: &mut System) -> ExtendedAppInfo {
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

            let bundle_url = app.bundleURL();
            let path = bundle_url
                .as_ref()
                .and_then(|url| url.path())
                .map(|p| p.to_string());

            let executable_url = app.executableURL();
            let executable_path = executable_url
                .as_ref()
                .and_then(|url| url.path())
                .map(|p| p.to_string());

            let launch_date = app.launchDate().map(|_| Instant::now());

            // Get icon
            let icon_data = app.icon().and_then(|icon| {
                // Convert NSImage to base64
                Self::nsimage_to_base64(&icon)
            });

            // Get windows for this app
            let windows = Self::get_windows_for_pid(pid);
            let frontmost_window = windows.first().cloned();
            let window_count = windows.len();
            let front_window_display_id = frontmost_window
                .as_ref()
                .and_then(|w| Self::display_id_for_window(&w.bounds).map(|id| id));

            // Get process info - using proper sysinfo API
            sys.refresh_processes(ProcessesToUpdate::Some(&[SysPid::from(pid as usize)]), true);
            let process_info = sys.process(SysPid::from(pid as usize)).map(|proc| {
                ProcessInfo {
                    cpu_usage: proc.cpu_usage(),
                    memory_bytes: proc.memory(),
                    virtual_memory_bytes: proc.virtual_memory(),
                    num_threads: 0, // Not available in sysinfo
                    start_time: Some(
                        SystemTime::UNIX_EPOCH + Duration::from_secs(proc.start_time()),
                    ),
                    parent_pid: proc.parent().map(|p| p.as_u32() as i32),
                    environment: HashMap::new(), // Convert environment properly
                    command_line: proc
                        .cmd()
                        .iter()
                        .map(|s| s.to_string_lossy().to_string())
                        .collect(),
                    executable_path: proc.exe().map(|p| p.to_path_buf()),
                    working_directory: proc.cwd().map(|p| p.to_path_buf()),
                }
            });

            // Get activation policy
            let activation_policy = if app.activationPolicy()
                == objc2_app_kit::NSApplicationActivationPolicy::Regular
            {
                "Regular".to_string()
            } else if app.activationPolicy()
                == objc2_app_kit::NSApplicationActivationPolicy::Accessory
            {
                "Accessory".to_string()
            } else {
                "Prohibited".to_string()
            };

            ExtendedAppInfo {
                name,
                bundle_id,
                pid,
                path,
                executable_path,
                launch_date,
                icon_base64: icon_data,
                icon_path: None,
                is_active: app.isActive(),
                is_hidden: app.isHidden(),
                is_terminated: app.isTerminated(),
                activation_policy,
                activation_count: 0,
                windows,
                frontmost_window,
                window_count,
                front_window_display_id,
                process_info,
                bundle_version: None,
                bundle_short_version: None,
                minimum_system_version: None,
                category: None,
                developer: None,
            }
        }
    }

    fn nsimage_to_base64(_image: &NSImage) -> Option<String> {
        // Simplified - would need proper implementation
        None
    }

    fn get_windows_for_pid(pid: i32) -> Vec<WindowInfo> {
        let mut windows = Vec::new();

        unsafe {
            // Prefer on-screen windows in front-to-back global order
            let options = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
            let window_list_ptr = CGWindowListCopyWindowInfo(options, 0);

            if window_list_ptr.is_null() {
                return windows;
            }

            let window_list: CFArray<CFDictionary> =
                CFArray::wrap_under_create_rule(window_list_ptr as *const _);

            for i in 0..window_list.len() {
                if let Some(window_dict) = window_list.get(i) {
                    // Check if window belongs to our PID
                    let key = CFString::from("kCGWindowOwnerPID");
                    let owner_pid = if let Some(value_ptr) = window_dict.find(key.to_void()) {
                        unsafe { CFNumber::from_void(*value_ptr) }
                            .to_i32()
                            .unwrap_or(0)
                    } else {
                        0
                    };

                    if owner_pid == pid {
                        // Extract window info (the first encountered for this PID will be the frontmost for this app)
                        let window_id = {
                            let key = CFString::from("kCGWindowNumber");
                            window_dict
                                .find(key.to_void())
                                .map(|n| {
                                    unsafe { CFNumber::from_void(*n) }.to_i32().unwrap_or(0) as u32
                                })
                                .unwrap_or(0)
                        };

                        let title = {
                            let key = CFString::from("kCGWindowName");
                            window_dict
                                .find(key.to_void())
                                .map(|s| unsafe { CFString::from_void(*s) }.to_string())
                        };

                        let layer = {
                            let key = CFString::from("kCGWindowLayer");
                            window_dict
                                .find(key.to_void())
                                .map(|n| unsafe { CFNumber::from_void(*n) }.to_i32().unwrap_or(0))
                                .unwrap_or(0)
                        };

                        let alpha = {
                            let key = CFString::from("kCGWindowAlpha");
                            window_dict
                                .find(key.to_void())
                                .map(|n| unsafe { CFNumber::from_void(*n) }.to_f64().unwrap_or(1.0))
                                .unwrap_or(1.0)
                        };

                        let is_onscreen = {
                            let key = CFString::from("kCGWindowIsOnscreen");
                            window_dict
                                .find(key.to_void())
                                .map(|b| {
                                    let item = unsafe { CFBoolean::from_void(*b) };
                                    let cf_bool = (*item).clone();
                                    bool::from(cf_bool)
                                })
                                .unwrap_or(false)
                        };

                        // Extract bounds
                        let bounds = {
                            let key = CFString::from("kCGWindowBounds");
                            if let Some(bounds_dict_ref) = window_dict.find(key.to_void()) {
                                let bounds_dict = unsafe {
                                    CFDictionary::<CFString, CFType>::from_void(*bounds_dict_ref)
                                };
                                let x = {
                                    let key = CFString::from("X");
                                    (*bounds_dict)
                                        .find(&key)
                                        .and_then(|n| n.downcast::<CFNumber>())
                                        .map(|num| num.to_f64().unwrap_or(0.0))
                                        .unwrap_or(0.0)
                                };

                                let y = {
                                    let key = CFString::from("Y");
                                    (*bounds_dict)
                                        .find(&key)
                                        .and_then(|n| n.downcast::<CFNumber>())
                                        .map(|num| num.to_f64().unwrap_or(0.0))
                                        .unwrap_or(0.0)
                                };

                                let width = {
                                    let key = CFString::from("Width");
                                    (*bounds_dict)
                                        .find(&key)
                                        .and_then(|n| n.downcast::<CFNumber>())
                                        .map(|num| num.to_f64().unwrap_or(0.0))
                                        .unwrap_or(0.0)
                                };

                                let height = {
                                    let key = CFString::from("Height");
                                    (*bounds_dict)
                                        .find(&key)
                                        .and_then(|n| n.downcast::<CFNumber>())
                                        .map(|num| num.to_f64().unwrap_or(0.0))
                                        .unwrap_or(0.0)
                                };

                                WindowBounds {
                                    x,
                                    y,
                                    width,
                                    height,
                                }
                            } else {
                                WindowBounds {
                                    x: 0.0,
                                    y: 0.0,
                                    width: 0.0,
                                    height: 0.0,
                                }
                            }
                        };

                        windows.push(WindowInfo {
                            window_id,
                            title,
                            bounds,
                            layer,
                            alpha,
                            memory_usage: None,
                            sharing_state: None,
                            backing_store_type: None,
                            is_onscreen,
                        });
                    }
                }
            }
        }
        windows
    }

    /// Determine which display contains the center of the given window bounds
    fn display_id_for_window(bounds: &WindowBounds) -> Option<u32> {
        unsafe {
            let max = 16u32;
            let mut out_count: u32 = 0;
            let mut ids = [0u32; 16];
            let rc = CGGetActiveDisplayList(max, ids.as_mut_ptr(), &mut out_count);
            if rc != 0 || out_count == 0 {
                return Some(CGMainDisplayID());
            }
            let center_x = bounds.x + bounds.width / 2.0;
            let center_y = bounds.y + bounds.height / 2.0;
            for i in 0..(out_count as usize) {
                let did = ids[i];
                let rect = CGDisplayBounds(did);
                if center_x >= rect.origin.x
                    && center_x <= rect.origin.x + rect.size.width
                    && center_y >= rect.origin.y
                    && center_y <= rect.origin.y + rect.size.height
                {
                    return Some(did);
                }
            }
            Some(CGMainDisplayID())
        }
    }

    fn capture_desktop_state() -> DesktopState {
        unsafe {
            let session_dict_ptr = CGSessionCopyCurrentDictionary();

            let mut state = DesktopState {
                active_space_id: None,
                display_count: 0,
                session_active: true,
                screen_locked: false,
                console_user: None,
                login_time: None,
                idle_time_seconds: None,
                active_space_uuid: None,
                active_space_index: None,
                active_space_type: None,
                active_space_name: None,
                active_space_label: None,
            };

            if !session_dict_ptr.is_null() {
                let session_dict: CFDictionary =
                    CFDictionary::wrap_under_create_rule(session_dict_ptr as *const _);

                // Extract session information
                if let Some(user_ref) =
                    session_dict.find(CFString::from("kCGSSessionUserNameKey").to_void())
                {
                    let username_str_item = unsafe { CFString::from_void(*user_ref) };
                    state.console_user = Some((*username_str_item).to_string());
                }

                if let Some(locked_ref) =
                    session_dict.find(CFString::from("CGSSessionScreenIsLocked").to_void())
                {
                    let item = unsafe { CFBoolean::from_void(*locked_ref) };
                    state.screen_locked = (*item) == CFBoolean::true_value();
                }
            }

            // Count active displays
            let max = 16u32;
            let mut out_count: u32 = 0;
            let mut ids = [0u32; 16];
            let rc = CGGetActiveDisplayList(max, ids.as_mut_ptr(), &mut out_count);
            if rc == 0 {
                state.display_count = out_count;
            } else {
                state.display_count = 1; // assume at least main display
            }

            // Spaces snapshot via SkyLight (best-effort)
            if let Some(snapshot) = query_spaces() {
                state.active_space_uuid = snapshot.active_space_uuid.clone();
                if let Some(first) = snapshot.displays.first() {
                    state.active_space_index = first.current_space_index;
                    state.active_space_type = first.current_space_type.clone();
                    state.active_space_name = first.current_space_name.clone();
                    state.active_space_label = snapshot.label_for_display(0);
                }
            }

            state
        }
    }

    fn start_coalescing_timer() {
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_millis(50));
            Self::process_coalesced_events();
        });
    }

    fn start_system_refresh_timer() {
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(5));
                unsafe {
                    if let Some(global) = &GLOBAL_STATE {
                        let mut state = global.lock().unwrap();
                        state.system_info.refresh_all();
                        state.desktop_state = Self::capture_desktop_state();

                        // Clone listeners to avoid borrow issues
                        let desktop_state = state.desktop_state.clone();
                        for listener in &mut state.listeners {
                            listener.on_desktop_state_change(&desktop_state);
                        }
                    }
                }
            }
        });
    }

    fn process_coalesced_events() {
        unsafe {
            if let Some(global) = &GLOBAL_STATE {
                let mut state = global.lock().unwrap();
                let now = Instant::now();

                // Process deactivate/activate pairs
                if let (Some((deact_app, deact_time)), Some((act_app, act_time))) = (
                    &state.coalescing.last_deactivate,
                    &state.coalescing.last_activate,
                ) {
                    // If they happened within 250ms, coalesce into single event
                    if act_time.duration_since(*deact_time) < Duration::from_millis(250) {
                        let event = EnhancedAppSwitchEvent {
                            timestamp: now,
                            system_time: SystemTime::now(),
                            event_type: AppSwitchType::Foreground,
                            app_info: act_app.clone(),
                            previous_app: Some(deact_app.clone()),
                            desktop_state: state.desktop_state.clone(),
                            trigger_source: TriggerSource::EventCoalescing,
                            confidence_score: 0.95,
                        };

                        state.current_app = Some(act_app.clone());

                        for listener in &mut state.listeners {
                            listener.on_app_switch(&event);
                        }

                        state.coalescing.last_deactivate = None;
                        state.coalescing.last_activate = None;
                    }
                }

                // Process any standalone deactivate after timeout
                if let Some((app, time)) = &state.coalescing.last_deactivate {
                    if now.duration_since(*time) > Duration::from_millis(300) {
                        let event = EnhancedAppSwitchEvent {
                            timestamp: now,
                            system_time: SystemTime::now(),
                            event_type: AppSwitchType::Background,
                            app_info: app.clone(),
                            previous_app: state.current_app.clone(),
                            desktop_state: state.desktop_state.clone(),
                            trigger_source: TriggerSource::NSWorkspaceNotification,
                            confidence_score: 0.9,
                        };

                        for listener in &mut state.listeners {
                            listener.on_app_switch(&event);
                        }

                        state.coalescing.last_deactivate = None;
                    }
                }
            }
        }
    }

    pub fn verify_frontmost_via_cgwindow(pid: i32) -> bool {
        unsafe {
            let options = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
            let window_list_ptr = CGWindowListCopyWindowInfo(options, 0);

            if window_list_ptr.is_null() {
                return false;
            }

            let window_list: CFArray<CFDictionary> =
                CFArray::wrap_under_create_rule(window_list_ptr as *const _);

            // First window in list is frontmost
            if let Some(first_window) = window_list.get(0) {
                if let Some(owner_pid_ref) =
                    first_window.find(CFString::from("kCGWindowOwnerPID").to_void())
                {
                    let owner_pid = unsafe { CFNumber::from_void(*owner_pid_ref) }
                        .to_i32()
                        .unwrap_or(0);
                    return owner_pid == pid;
                }
            }
        }
        false
    }

    pub fn get_all_windows() -> Vec<WindowInfo> {
        let mut windows = Vec::new();

        unsafe {
            let options = kCGWindowListOptionOnScreenOnly;
            let window_list_ptr = CGWindowListCopyWindowInfo(options, 0);

            if !window_list_ptr.is_null() {
                let window_list: CFArray<CFDictionary> =
                    CFArray::wrap_under_create_rule(window_list_ptr as *const _);

                for i in 0..window_list.len() {
                    if let Some(_window_dict) = window_list.get(i) {
                        // Extract window info (same as get_windows_for_pid but for all)
                        // Implementation would go here
                    }
                }
            }
        }

        windows
    }

    /// Public trigger to resample the current frontmost application and emit if changed
    pub fn resample_now(&self) {
        EnhancedWorkspaceObserver::resample_frontmost();
    }
}

impl EnhancedWorkspaceObserver {
    fn handle_notification(notification: &NSNotification, event_type: &str) {
        unsafe {
            if let Some(global) = &GLOBAL_STATE {
                let app = Self::get_app_from_notification(notification);
                if let Some(app) = app {
                    let mut state = global.lock().unwrap();
                    let app_info = EnhancedAppSwitcher::extract_extended_app_info(
                        &app,
                        &mut state.system_info,
                    );
                    let now = Instant::now();

                    match event_type {
                        "activate" => {
                            // Update activation count
                            let count = state
                                .activation_counts
                                .entry(app_info.bundle_id.clone())
                                .and_modify(|c| *c += 1)
                                .or_insert(1);

                            let mut app_info = app_info;
                            app_info.activation_count = *count;

                            state.coalescing.last_activate = Some((app_info.clone(), now));

                            // Cross-check with CGWindow after small delay
                            let pid = app_info.pid;
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_millis(50));
                                if !EnhancedAppSwitcher::verify_frontmost_via_cgwindow(pid) {
                                    Self::resample_frontmost();
                                }
                            });
                        }
                        "deactivate" => {
                            state.coalescing.last_deactivate = Some((app_info, now));
                        }
                        "launch" => {
                            let event = EnhancedAppSwitchEvent {
                                timestamp: now,
                                system_time: SystemTime::now(),
                                event_type: AppSwitchType::Launch,
                                app_info,
                                previous_app: state.current_app.clone(),
                                desktop_state: state.desktop_state.clone(),
                                trigger_source: TriggerSource::NSWorkspaceNotification,
                                confidence_score: 1.0,
                            };
                            for listener in &mut state.listeners {
                                listener.on_app_switch(&event);
                            }
                        }
                        "terminate" => {
                            let event = EnhancedAppSwitchEvent {
                                timestamp: now,
                                system_time: SystemTime::now(),
                                event_type: AppSwitchType::Terminate,
                                app_info,
                                previous_app: state.current_app.clone(),
                                desktop_state: state.desktop_state.clone(),
                                trigger_source: TriggerSource::NSWorkspaceNotification,
                                confidence_score: 1.0,
                            };
                            for listener in &mut state.listeners {
                                listener.on_app_switch(&event);
                            }
                        }
                        "hide" => {
                            let event = EnhancedAppSwitchEvent {
                                timestamp: now,
                                system_time: SystemTime::now(),
                                event_type: AppSwitchType::Hide,
                                app_info,
                                previous_app: state.current_app.clone(),
                                desktop_state: state.desktop_state.clone(),
                                trigger_source: TriggerSource::NSWorkspaceNotification,
                                confidence_score: 1.0,
                            };
                            for listener in &mut state.listeners {
                                listener.on_app_switch(&event);
                            }
                            // Schedule resample
                            std::thread::spawn(|| {
                                std::thread::sleep(Duration::from_millis(100));
                                Self::resample_frontmost();
                            });
                        }
                        "unhide" => {
                            let event = EnhancedAppSwitchEvent {
                                timestamp: now,
                                system_time: SystemTime::now(),
                                event_type: AppSwitchType::Unhide,
                                app_info,
                                previous_app: state.current_app.clone(),
                                desktop_state: state.desktop_state.clone(),
                                trigger_source: TriggerSource::NSWorkspaceNotification,
                                confidence_score: 1.0,
                            };
                            for listener in &mut state.listeners {
                                listener.on_app_switch(&event);
                            }
                            // Schedule resample
                            std::thread::spawn(|| {
                                std::thread::sleep(Duration::from_millis(100));
                                Self::resample_frontmost();
                            });
                        }
                        _ => {}
                    }

                    state.last_event_time = now;
                }
            }
        }
    }

    fn handle_space_change() {
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(150));
            Self::resample_frontmost_with_trigger(TriggerSource::SpaceTransition);
        });
    }

    fn handle_session_change(active: bool) {
        if active {
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                Self::resample_frontmost_with_trigger(TriggerSource::SessionTransition);
            });
        }
    }

    fn handle_wake() {
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(200));
            Self::resample_frontmost_with_trigger(TriggerSource::WakeEvent);
        });
    }

    fn handle_screen_change() {
        // Resample after screen configuration changes
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            Self::resample_frontmost();
        });
    }

    fn resample_frontmost() {
        Self::resample_frontmost_with_trigger(TriggerSource::ManualResample);
    }

    fn resample_frontmost_with_trigger(trigger: TriggerSource) {
        unsafe {
            let workspace = NSWorkspace::sharedWorkspace();
            if let Some(frontmost) = workspace.frontmostApplication() {
                if let Some(global) = &GLOBAL_STATE {
                    let mut state = global.lock().unwrap();

                    let app_info = EnhancedAppSwitcher::extract_extended_app_info(
                        &frontmost,
                        &mut state.system_info,
                    );

                    // Update activation count
                    let count = state
                        .activation_counts
                        .entry(app_info.bundle_id.clone())
                        .and_modify(|c| *c += 1)
                        .or_insert(1);

                    let mut app_info = app_info;
                    app_info.activation_count = *count;

                    // Only emit event if actually changed
                    let changed = state
                        .current_app
                        .as_ref()
                        .map(|c| c.pid != app_info.pid)
                        .unwrap_or(true);

                    if changed {
                        // Update desktop state
                        state.desktop_state = EnhancedAppSwitcher::capture_desktop_state();

                        let event = EnhancedAppSwitchEvent {
                            timestamp: Instant::now(),
                            system_time: SystemTime::now(),
                            event_type: match trigger {
                                TriggerSource::SpaceTransition => AppSwitchType::SpaceChange,
                                TriggerSource::SessionTransition => AppSwitchType::SessionChange,
                                TriggerSource::WakeEvent => AppSwitchType::WakeFromSleep,
                                _ => AppSwitchType::Foreground,
                            },
                            app_info: app_info.clone(),
                            previous_app: state.current_app.clone(),
                            desktop_state: state.desktop_state.clone(),
                            trigger_source: trigger,
                            confidence_score: 0.85,
                        };

                        state.current_app = Some(app_info);

                        for listener in &mut state.listeners {
                            listener.on_app_switch(&event);
                        }
                    }
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
                // Convert AnyObject to NSRunningApplication properly
                let app: Retained<NSRunningApplication> =
                    unsafe { Retained::cast_unchecked(app_obj.retain()) };
                return Some(app);
            }
        }
        None
    }
}

impl Drop for EnhancedAppSwitcher {
    fn drop(&mut self) {
        self.stop_monitoring();
    }
}

/// Simple debug listener for testing
pub struct DebugListener;

impl EnhancedAppSwitchListener for DebugListener {
    fn on_app_switch(&mut self, event: &EnhancedAppSwitchEvent) {
        println!("🔄 App Switch Event:");
        println!("  Type: {:?}", event.event_type);
        println!(
            "  App: {} ({})",
            event.app_info.name, event.app_info.bundle_id
        );
        println!("  PID: {}", event.app_info.pid);
        println!("  Windows: {}", event.app_info.window_count);
        if let Some(front_window) = &event.app_info.frontmost_window {
            if let Some(title) = &front_window.title {
                println!("  Front Window: {}", title);
            }
        }
        if let Some(proc) = &event.app_info.process_info {
            println!("  CPU: {:.1}%", proc.cpu_usage);
            println!("  Memory: {:.1} MB", proc.memory_bytes as f64 / 1_048_576.0);
        }
        println!("  Activation #: {}", event.app_info.activation_count);
        println!("  Trigger: {:?}", event.trigger_source);
        println!("  Confidence: {:.0}%", event.confidence_score * 100.0);
        println!("  Desktop State:");
        println!("    Session Active: {}", event.desktop_state.session_active);
        println!("    Screen Locked: {}", event.desktop_state.screen_locked);
        if let Some(user) = &event.desktop_state.console_user {
            println!("    User: {}", user);
        }
        println!();
    }

    fn on_monitoring_started(&mut self) {
        println!("✅ Enhanced monitoring started");
    }

    fn on_monitoring_stopped(&mut self) {
        println!("🛑 Enhanced monitoring stopped");
    }

    fn on_desktop_state_change(&mut self, state: &DesktopState) {
        println!("🖥️  Desktop state changed:");
        println!(
            "  Session: {}",
            if state.session_active {
                "Active"
            } else {
                "Inactive"
            }
        );
        println!("  Locked: {}", state.screen_locked);
    }
}
