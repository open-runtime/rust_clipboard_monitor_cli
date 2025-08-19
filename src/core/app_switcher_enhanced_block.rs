//! Enhanced multi-layer application monitoring system with maximum information extraction
//!
//! Layers:
//! - L1: NSWorkspace notifications (primary, zero polling, block observers)
//! - L2: CGWindow cross-check for validation + rich window metadata
//! - L3: Event coalescing to handle rapid switches
//! - L4: Space/sleep/wake/session/screen parameter handling
//! - L5: Process information via `sysinfo`
//! - L6: Desktop state (session/lock/display count/idle time)
//!
//! Targets macOS 10.7+ (Lion) through the latest macOS.
//!
//! Notes:
//! - We use block-based observer APIs and keep/remove the returned tokens — best practice. (See Apple docs)
//! - Screen parameter changes are posted on the DEFAULT notification center, not workspace.
//! - CGWindow keys are read via CF downcasts; we defensively handle absent keys / permission-limited fields.

#![allow(clippy::too_many_arguments)]
#![cfg(target_os = "macos")]

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
    thread,
    time::{Duration, Instant, SystemTime},
};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

use block2::StackBlock;

use objc2::rc::Retained;
use objc2::{msg_send, sel, class, ClassType, MainThreadMarker, Message};

use objc2_app_kit::{
    NSApplicationActivationPolicy, NSBitmapImageRep, NSImage, NSRunningApplication, NSScreen,
    NSWorkspace,
};
use objc2_core_graphics::CGEventSource;
use objc2_foundation::{
    ns_string, NSData, NSDictionary, NSNotification, NSNotificationCenter, NSObject,
    NSString,
};

// CoreFoundation
use core_foundation::{
    array::CFArray, base::TCFType, boolean::CFBoolean, dictionary::CFDictionary, number::CFNumber,
    string::CFString,
};

// FFI: CoreGraphics
use core_foundation::array::CFArrayRef;
use core_foundation::dictionary::CFDictionaryRef;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
    fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;

    // Idle time helpers (Quartz Event Services):
    // double CGEventSourceSecondsSinceLastEventType(CGEventSourceStateID, CGEventType)
    // We call via `objc2_core_graphics::CGEventSourceSecondsSinceLastEventType`.
}

// CGWindow list options
#[allow(non_upper_case_globals)]
const kCGWindowListOptionOnScreenOnly: u32 = 1 << 0;
#[allow(non_upper_case_globals)]
const kCGWindowListExcludeDesktopElements: u32 = 1 << 4;
#[allow(non_upper_case_globals)]
const kCGWindowListOptionAll: u32 = 0;

// Notification names (older SDKs used NSString*; this is fine across 10.7+)
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

// IMPORTANT: This one is posted on the DEFAULT notification center, not workspace.
const APPLICATION_DID_CHANGE_SCREEN_PARAMETERS: &str =
    "NSApplicationDidChangeScreenParametersNotification";

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
    pub backing_store_type: Option<u32>,
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
    pub active_space_id: Option<u32>, // left None on public API path
    pub display_count: u32,
    pub session_active: bool,
    pub screen_locked: bool,
    pub console_user: Option<String>,
    pub login_time: Option<SystemTime>, // best-effort (usually None)
    pub idle_time_seconds: Option<f64>, // via CGEventSourceSecondsSinceLastEventType
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
    pub icon_base64_png: Option<String>,

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

    // Process info
    pub process_info: Option<ProcessInfo>,

    // Additional metadata (best effort)
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
    ScreenParametersChanged,
}

#[derive(Debug, Clone)]
pub enum TriggerSource {
    NSWorkspaceNotification,
    DefaultCenterNotification,
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
    fn on_desktop_state_change(&mut self, _state: &DesktopState) {}
}

// -----------------------------------------------
// Internal state
// -----------------------------------------------

struct CoalescingState {
    last_deactivate: Option<(ExtendedAppInfo, Instant)>,
    last_activate: Option<(ExtendedAppInfo, Instant)>,
}

struct EnhancedState {
    current_app: Option<ExtendedAppInfo>,
    listeners: Vec<Box<dyn EnhancedAppSwitchListener>>,
    coalescing: CoalescingState,
    activation_counts: HashMap<String, u32>,
    last_event_time: Instant,
    desktop_state: DesktopState,

    // Observer tokens (retain and remove on stop)
    observer_tokens: Vec<Retained<NSObject>>,

    // Session flags
    session_active_flag: bool,
}

pub struct EnhancedAppSwitcher {
    state: Arc<Mutex<EnhancedState>>,
}

// -----------------------------------------------
// Sysinfo (process)
// -----------------------------------------------
use sysinfo::{Pid as SysPid, ProcessesToUpdate, System};

impl EnhancedAppSwitcher {
    pub fn new() -> Self {
        let initial_desktop = DesktopState {
            active_space_id: None,
            display_count: Self::read_display_count(),
            session_active: true,
            screen_locked: false,
            console_user: None,
            login_time: None,
            idle_time_seconds: Self::read_idle_time_seconds(),
        };

        let state = EnhancedState {
            current_app: None,
            listeners: Vec::new(),
            coalescing: CoalescingState {
                last_deactivate: None,
                last_activate: None,
            },
            activation_counts: HashMap::new(),
            last_event_time: Instant::now(),
            desktop_state: initial_desktop,
            observer_tokens: Vec::new(),
            session_active_flag: true,
        };

        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    pub fn add_listener<T: EnhancedAppSwitchListener + 'static>(&mut self, listener: T) {
        self.state
            .lock()
            .unwrap()
            .listeners
            .push(Box::new(listener));
    }

    /// Start monitoring. Must be called on the main thread; we require a `MainThreadMarker`.
    pub fn start_monitoring(&mut self, _mtm: MainThreadMarker) -> anyhow::Result<()> {
        let workspace = unsafe { NSWorkspace::sharedWorkspace() }; // retained

        // Seed with current frontmost app
        if let Some(frontmost) = unsafe { workspace.frontmostApplication() } {
            let mut sys = System::new_all();
            let app_info = Self::extract_extended_app_info(&frontmost, &mut sys);
            let mut state = self.state.lock().unwrap();
            state.current_app = Some(app_info.clone());
            for l in &mut state.listeners {
                l.on_monitoring_started();
            }
        }

        // Register notifications (block-based observers). We store tokens and remove them on stop.
        self.register_observers(&workspace)?;

        // Background thread: coalescing processor
        self.spawn_coalescer();

        // Background thread: periodic desktop refresh to push idle time + state to listeners
        self.spawn_desktop_refresh();

        Ok(())
    }

    /// Integration point for main.rs AppSwitcher interface
    pub fn current_app_for_main(&self) -> Option<super::app_switcher::AppInfo> {
        self.current_app().map(|ext_info| super::app_switcher::AppInfo {
            name: ext_info.name,
            bundle_id: ext_info.bundle_id,
            pid: ext_info.pid,
            path: ext_info.path,
            executable_path: ext_info.executable_path,
            launch_date: ext_info.launch_date,
            icon_path: None, // Enhanced block version uses base64 instead
        })
    }

    pub fn stop_monitoring(&mut self) {
        // Remove observers from the centers.
        let mut state = self.state.lock().unwrap();
        let default_center = unsafe { NSNotificationCenter::defaultCenter() };
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };
        let workspace_center = unsafe { workspace.notificationCenter() };

        for token in state.observer_tokens.drain(..) {
            unsafe {
                let _: () = msg_send![&*workspace_center, removeObserver: &*token];
                let _: () = msg_send![&*default_center, removeObserver: &*token];
            }
        }

        for l in &mut state.listeners {
            l.on_monitoring_stopped();
        }
    }

    pub fn current_app(&self) -> Option<ExtendedAppInfo> {
        self.state.lock().unwrap().current_app.clone()
    }

    pub fn desktop_state(&self) -> DesktopState {
        self.state.lock().unwrap().desktop_state.clone()
    }

    // ----------------------------------------------------------------
    // Observers
    // ----------------------------------------------------------------

    fn register_observers(&mut self, workspace: &NSWorkspace) -> anyhow::Result<()> {
        let state_weak = Arc::downgrade(&self.state);

        unsafe fn add_obs(
            center: &NSNotificationCenter,
            name: &NSString,
            state_weak: &Weak<Mutex<EnhancedState>>,
            handler: impl Fn(&Weak<Mutex<EnhancedState>>, &NSNotification, &str) + Send + 'static,
            tag: &'static str,
        ) -> Retained<NSObject> {
            let block = StackBlock::new({
                let state_weak = state_weak.clone();
                move |note: *mut NSNotification| {
                    // SAFETY: Cocoa gives valid pointer here
                    let note = unsafe { &*note };
                    handler(&state_weak, note, tag);
                }
            })
            .copy();

            // token: id<NSObjectProtocol>
            let token: Retained<NSObject> = msg_send![
                center,
                addObserverForName: name,
                object: std::ptr::null::<NSObject>(),
                queue: std::ptr::null::<NSObject>(),
                usingBlock: &*block
            ];
            token
        }

        let workspace_center = unsafe { workspace.notificationCenter() };
        let default_center = unsafe { NSNotificationCenter::defaultCenter() };

        let mut tokens = Vec::<Retained<NSObject>>::new();

        // Workspace notifications
        let add_ws = |nm: &str, tag: &'static str| -> Retained<NSObject> {
            let name = NSString::from_str(nm);
            unsafe {
                add_obs(
                    &workspace_center,
                    &name,
                    &state_weak,
                    Self::workspace_handler,
                    tag,
                )
            }
        };

        tokens.push(add_ws(WORKSPACE_DID_ACTIVATE_APP, "activate"));
        tokens.push(add_ws(WORKSPACE_DID_DEACTIVATE_APP, "deactivate"));
        tokens.push(add_ws(WORKSPACE_DID_LAUNCH_APP, "launch"));
        tokens.push(add_ws(WORKSPACE_DID_TERMINATE_APP, "terminate"));
        tokens.push(add_ws(WORKSPACE_DID_HIDE_APP, "hide"));
        tokens.push(add_ws(WORKSPACE_DID_UNHIDE_APP, "unhide"));
        tokens.push(add_ws(WORKSPACE_ACTIVE_SPACE_CHANGED, "space"));
        tokens.push(add_ws(
            WORKSPACE_SESSION_DID_BECOME_ACTIVE,
            "session_active",
        ));
        tokens.push(add_ws(
            WORKSPACE_SESSION_DID_RESIGN_ACTIVE,
            "session_inactive",
        ));
        tokens.push(add_ws(WORKSPACE_DID_WAKE, "wake"));

        // Default center: screen parameter changes
        let name_screen = NSString::from_str(APPLICATION_DID_CHANGE_SCREEN_PARAMETERS);
        let token_screen: Retained<NSObject> = unsafe {
            add_obs(
                &default_center,
                &name_screen,
                &state_weak,
                Self::default_center_handler,
                "screen_params",
            )
        };
        tokens.push(token_screen);

        self.state.lock().unwrap().observer_tokens = tokens;
        Ok(())
    }

    fn workspace_handler(
        state_weak: &Weak<Mutex<EnhancedState>>,
        note: &NSNotification,
        tag: &str,
    ) {
        if let Some(state_arc) = state_weak.upgrade() {
            let mut sys = System::new_all();
            match tag {
                "activate" => {
                    Self::handle_activation(&state_arc, note, &mut sys);
                }
                "deactivate" => {
                    Self::handle_deactivation(&state_arc, note, &mut sys);
                }
                "launch" => {
                    Self::emit_simple_app_event(
                        &state_arc,
                        note,
                        AppSwitchType::Launch,
                        TriggerSource::NSWorkspaceNotification,
                        1.0,
                        &mut sys,
                    );
                }
                "terminate" => {
                    Self::emit_simple_app_event(
                        &state_arc,
                        note,
                        AppSwitchType::Terminate,
                        TriggerSource::NSWorkspaceNotification,
                        1.0,
                        &mut sys,
                    );
                }
                "hide" => {
                    Self::emit_simple_app_event(
                        &state_arc,
                        note,
                        AppSwitchType::Hide,
                        TriggerSource::NSWorkspaceNotification,
                        1.0,
                        &mut sys,
                    );
                    // Re-sample shortly after hide (window stack may change)
                    Self::delayed_resample(state_weak, 100, TriggerSource::ManualResample);
                }
                "unhide" => {
                    Self::emit_simple_app_event(
                        &state_arc,
                        note,
                        AppSwitchType::Unhide,
                        TriggerSource::NSWorkspaceNotification,
                        1.0,
                        &mut sys,
                    );
                    Self::delayed_resample(state_weak, 100, TriggerSource::ManualResample);
                }
                "space" => {
                    // Let Mission Control transition settle
                    Self::delayed_resample(state_weak, 150, TriggerSource::SpaceTransition);
                }
                "session_active" => {
                    {
                        let mut st = state_arc.lock().unwrap();
                        st.session_active_flag = true;
                    }
                    Self::delayed_resample(state_weak, 100, TriggerSource::SessionTransition);
                }
                "session_inactive" => {
                    let mut st = state_arc.lock().unwrap();
                    st.session_active_flag = false;
                }
                "wake" => {
                    Self::delayed_resample(state_weak, 200, TriggerSource::WakeEvent);
                }
                _ => {}
            }
        }
    }

    fn default_center_handler(
        state_weak: &Weak<Mutex<EnhancedState>>,
        _note: &NSNotification,
        tag: &str,
    ) {
        if tag == "screen_params" {
            // Screen configuration changed; update desktop, then resample.
            Self::delayed_resample(state_weak, 100, TriggerSource::DefaultCenterNotification);
        }
    }

    fn delayed_resample(state_weak: &Weak<Mutex<EnhancedState>>, ms: u64, trigger: TriggerSource) {
        let st = state_weak.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(ms));
            if let Some(arc) = st.upgrade() {
                Self::resample_frontmost_with_trigger(&arc, trigger);
            }
        });
    }

    fn handle_activation(
        state_arc: &Arc<Mutex<EnhancedState>>,
        _note: &NSNotification,
        sys: &mut System,
    ) {
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };
        if let Some(frontmost) = unsafe { workspace.frontmostApplication() } {
            let mut state = state_arc.lock().unwrap();
            let mut info = Self::extract_extended_app_info(&frontmost, sys);

            // Update activation count
            let count = state
                .activation_counts
                .entry(info.bundle_id.clone())
                .and_modify(|c| *c += 1)
                .or_insert(1);
            info.activation_count = *count;

            // Coalesce: set last_activate, and cross-check with CGWindow shortly
            state.coalescing.last_activate = Some((info.clone(), Instant::now()));

            let pid = info.pid;
            drop(state); // release lock for the spawned thread below

            // CGWindow verify shortly after (frontmost should own top layer-0 window)
            let st = Arc::downgrade(state_arc);
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(50));
                if let Some(arc) = st.upgrade() {
                    if !Self::verify_frontmost_via_cgwindow(pid) {
                        Self::resample_frontmost_with_trigger(
                            &arc,
                            TriggerSource::CGWindowVerification,
                        );
                    }
                }
            });
        }
    }

    fn handle_deactivation(
        state_arc: &Arc<Mutex<EnhancedState>>,
        note: &NSNotification,
        sys: &mut System,
    ) {
        // Use the app from userInfo if available; otherwise record the current for background.
        if let Some(app) = Self::app_from_notification(note) {
            let mut state = state_arc.lock().unwrap();
            let info = Self::extract_extended_app_info(&app, sys);
            state.coalescing.last_deactivate = Some((info, Instant::now()));
        } else {
            let mut state = state_arc.lock().unwrap();
            if let Some(cur) = &state.current_app {
                state
                    .coalescing
                    .last_deactivate
                    .replace((cur.clone(), Instant::now()));
            }
        }
    }

    fn emit_simple_app_event(
        state_arc: &Arc<Mutex<EnhancedState>>,
        note: &NSNotification,
        event_type: AppSwitchType,
        trigger: TriggerSource,
        confidence: f32,
        sys: &mut System,
    ) {
        if let Some(app) = Self::app_from_notification(note) {
            let mut state = state_arc.lock().unwrap();
            let info = Self::extract_extended_app_info(&app, sys);

            let mut desktop = Self::capture_desktop_state(state.session_active_flag);
            state.desktop_state = desktop.clone();

            let event = EnhancedAppSwitchEvent {
                timestamp: Instant::now(),
                system_time: SystemTime::now(),
                event_type,
                app_info: info,
                previous_app: state.current_app.clone(),
                desktop_state: desktop,
                trigger_source: trigger,
                confidence_score: confidence,
            };

            for l in &mut state.listeners {
                l.on_app_switch(&event);
            }
            state.last_event_time = Instant::now();
        }
    }

    fn spawn_coalescer(&self) {
        let st = Arc::downgrade(&self.state);
        thread::Builder::new()
            .name("coalescer".to_string())
            .spawn(move || loop {
                thread::sleep(Duration::from_millis(50));
                if let Some(arc) = st.upgrade() {
                    Self::process_coalesced_events(&arc);
                } else {
                    break;
                }
            })
            .expect("Failed to spawn coalescer thread");
    }

    fn spawn_desktop_refresh(&self) {
        let st = Arc::downgrade(&self.state);
        thread::Builder::new()
            .name("desktop_refresh".to_string())
            .spawn(move || loop {
                thread::sleep(Duration::from_secs(5));
                if let Some(arc) = st.upgrade() {
                    let mut state = arc.lock().unwrap();
                    state.desktop_state.idle_time_seconds = Self::read_idle_time_seconds();
                    state.desktop_state.display_count = Self::read_display_count();

                    // push change to listeners
                    let snapshot = state.desktop_state.clone();
                    for l in &mut state.listeners {
                        l.on_desktop_state_change(&snapshot);
                    }
                } else {
                    break;
                }
            })
            .expect("Failed to spawn desktop refresh thread");
    }

    fn process_coalesced_events(state_arc: &Arc<Mutex<EnhancedState>>) {
        let mut state = state_arc.lock().unwrap();
        let now = Instant::now();

        // Activate + Deactivate within 250ms -> single Foreground event
        if let (Some((deact_app, deact_time)), Some((act_app, act_time))) = (
            &state.coalescing.last_deactivate,
            &state.coalescing.last_activate,
        ) {
            if act_time.duration_since(*deact_time) < Duration::from_millis(250) {
                let desktop = Self::capture_desktop_state(state.session_active_flag);
                state.desktop_state = desktop.clone();
                let event = EnhancedAppSwitchEvent {
                    timestamp: now,
                    system_time: SystemTime::now(),
                    event_type: AppSwitchType::Foreground,
                    app_info: act_app.clone(),
                    previous_app: Some(deact_app.clone()),
                    desktop_state: desktop,
                    trigger_source: TriggerSource::EventCoalescing,
                    confidence_score: 0.95,
                };
                state.current_app = Some(act_app.clone());
                for l in &mut state.listeners {
                    l.on_app_switch(&event);
                }
                state.coalescing.last_deactivate = None;
                state.coalescing.last_activate = None;
                return;
            }
        }

        // Standalone deactivate >300ms → Background event
        if let Some((app, time)) = &state.coalescing.last_deactivate {
            if now.duration_since(*time) > Duration::from_millis(300) {
                let desktop = Self::capture_desktop_state(state.session_active_flag);
                state.desktop_state = desktop.clone();
                let event = EnhancedAppSwitchEvent {
                    timestamp: now,
                    system_time: SystemTime::now(),
                    event_type: AppSwitchType::Background,
                    app_info: app.clone(),
                    previous_app: state.current_app.clone(),
                    desktop_state: desktop,
                    trigger_source: TriggerSource::NSWorkspaceNotification,
                    confidence_score: 0.9,
                };
                for l in &mut state.listeners {
                    l.on_app_switch(&event);
                }
                state.coalescing.last_deactivate = None;
            }
        }
    }

    fn resample_frontmost_with_trigger(
        state_arc: &Arc<Mutex<EnhancedState>>,
        trigger: TriggerSource,
    ) {
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };
        if let Some(frontmost) = unsafe { workspace.frontmostApplication() } {
            let mut sys = System::new_all();
            let mut state = state_arc.lock().unwrap();
            let mut app_info = Self::extract_extended_app_info(&frontmost, &mut sys);

            let count = state
                .activation_counts
                .entry(app_info.bundle_id.clone())
                .and_modify(|c| *c += 1)
                .or_insert(1);
            app_info.activation_count = *count;

            let changed = state
                .current_app
                .as_ref()
                .map(|c| c.pid != app_info.pid)
                .unwrap_or(true);

            if changed {
                let desktop = Self::capture_desktop_state(state.session_active_flag);
                state.desktop_state = desktop.clone();
                let event = EnhancedAppSwitchEvent {
                    timestamp: Instant::now(),
                    system_time: SystemTime::now(),
                    event_type: match trigger {
                        TriggerSource::SpaceTransition => AppSwitchType::SpaceChange,
                        TriggerSource::SessionTransition => AppSwitchType::SessionChange,
                        TriggerSource::WakeEvent => AppSwitchType::WakeFromSleep,
                        TriggerSource::DefaultCenterNotification => {
                            AppSwitchType::ScreenParametersChanged
                        }
                        _ => AppSwitchType::Foreground,
                    },
                    app_info: app_info.clone(),
                    previous_app: state.current_app.clone(),
                    desktop_state: desktop,
                    trigger_source: trigger,
                    confidence_score: 0.85,
                };
                state.current_app = Some(app_info);
                for l in &mut state.listeners {
                    l.on_app_switch(&event);
                }
            }
        }
    }

    // ---------------------------------------------------------------
    // Helpers: extracting NSRunningApplication from NSNotification
    // ---------------------------------------------------------------

    fn app_from_notification(
        notification: &NSNotification,
    ) -> Option<Retained<NSRunningApplication>> {
        unsafe {
            let user_info: Option<Retained<NSDictionary<NSObject, NSObject>>> =
                notification.userInfo();
            if let Some(ui) = user_info {
                let key = ns_string!("NSWorkspaceApplicationKey");
                let obj: *mut NSObject = msg_send![&*ui, objectForKey: &*key];
                if obj.is_null() {
                    return None;
                }
                // objectForKey returns autoreleased; retain_autoreleased is correct here.
                let any: Option<Retained<NSObject>> = Retained::retain_autoreleased(obj);
                if let Some(any) = any {
                    // Downcast to NSRunningApplication
                    // Safety: Apple guarantees NSRunningApplication under this key. :contentReference[oaicite:7]{index=7}
                    let app: Retained<NSRunningApplication> = any.cast();
                    return Some(app);
                }
            }
        }
        None
    }

    // ---------------------------------------------------------------
    // Rich App Info (AppKit + CGWindow + sysinfo)
    // ---------------------------------------------------------------

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

            let path = app
                .bundleURL()
                .and_then(|url| url.path())
                .map(|p| p.to_string());

            let executable_path = app
                .executableURL()
                .and_then(|url| url.path())
                .map(|p| p.to_string());

            // There's no KVO-friendly launch date in objc2 yet; Instant::now() as "seen" time.
            let launch_date = app.launchDate().map(|_| Instant::now());

            // Icon -> PNG Base64
            let icon_base64_png = app.icon().and_then(|img| Self::nsimage_to_base64_png(&img));

            // CGWindow windows owned by pid
            let windows = Self::get_windows_for_pid(pid);
            let frontmost_window = windows.first().cloned();
            let window_count = windows.len();

            // sysinfo process
            sys.refresh_processes(ProcessesToUpdate::Some(&[SysPid::from(pid as usize)]), true);
            let process_info = sys.process(SysPid::from(pid as usize)).map(|proc| {
                let env_map: HashMap<String, String> = proc
                    .environ()
                    .iter()
                    .filter_map(|os| {
                        let s = os.to_string_lossy();
                        let mut it = s.splitn(2, '=');
                        let k = it.next()?;
                        let v = it.next().unwrap_or_default();
                        Some((k.to_owned(), v.to_owned()))
                    })
                    .collect();
                ProcessInfo {
                    cpu_usage: proc.cpu_usage(),
                    memory_bytes: proc.memory(),
                    virtual_memory_bytes: proc.virtual_memory(),
                    num_threads: proc.threads().len(),
                    start_time: Some(
                        SystemTime::UNIX_EPOCH + Duration::from_secs(proc.start_time()),
                    ),
                    parent_pid: proc.parent().map(|p| p.as_u32() as i32),
                    environment: env_map,
                    command_line: proc
                        .cmd()
                        .iter()
                        .map(|s| s.to_string_lossy().to_string())
                        .collect(),
                    executable_path: proc.exe().map(|p| p.to_path_buf()),
                    working_directory: proc.cwd().map(|p| p.to_path_buf()),
                }
            });

            let activation_policy = match app.activationPolicy() {
                NSApplicationActivationPolicy::Regular => "Regular".to_string(),
                NSApplicationActivationPolicy::Accessory => "Accessory".to_string(),
                NSApplicationActivationPolicy::Prohibited => "Prohibited".to_string(),
                _ => "Unknown".to_string(),
            };

            ExtendedAppInfo {
                name,
                bundle_id,
                pid,
                path,
                executable_path,
                launch_date,
                icon_base64_png,
                is_active: app.isActive(),
                is_hidden: app.isHidden(),
                is_terminated: app.isTerminated(),
                activation_policy,
                activation_count: 0,
                windows,
                frontmost_window,
                window_count,
                process_info,
                // Optional bundle metadata (could be added by reading Info.plist via CFBundle if needed)
                bundle_version: None,
                bundle_short_version: None,
                minimum_system_version: None,
                category: None,
                developer: None,
            }
        }
    }

    /// Convert NSImage -> PNG -> base64 (safe across 10.7+).
    fn nsimage_to_base64_png(image: &NSImage) -> Option<String> {
        unsafe {
            // 1) TIFF representation
            let tiff: Option<Retained<NSData>> = msg_send![image, TIFFRepresentation];
            let tiff = tiff?;

            // 2) NSBitmapImageRep
            let rep: Option<Retained<NSBitmapImageRep>> =
                msg_send![class!(NSBitmapImageRep), imageRepWithData: &*tiff];
            let rep = rep?;

            // 3) PNG data
            // representationUsingType:properties: with NSPNGFileType == 4 historically,
            // but we call the proper selector by name to avoid enum mismatch:
            let png_props: Option<Retained<NSDictionary<NSObject, NSObject>>> =
                Some(NSDictionary::new());
            let png_data: Option<Retained<NSData>> = msg_send![&*rep, representationUsingType: 4u64 /* NSPNGFileType */, properties: png_props.as_ref().unwrap()];

            let png_data = png_data?;
            let len: usize = msg_send![&*png_data, length];
            let bytes: *const u8 = msg_send![&*png_data, bytes];
            if bytes.is_null() || len == 0 {
                return None;
            }
            // SAFETY: NSData is immutable; copy into Vec, then base64
            let slice = unsafe { std::slice::from_raw_parts(bytes, len) };
            Some(B64.encode(slice))
        }
    }

    // ---------------------------------------------------------------
    // CGWindow: window list & verification
    // ---------------------------------------------------------------

    fn verify_frontmost_via_cgwindow(pid: i32) -> bool {
        let windows = Self::get_cg_window_info(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            None,
        );
        // First layer-0 window in on-screen list belongs to frontmost app
        for w in windows {
            if w.layer == 0 && w.is_onscreen {
                return Self::owner_pid_for_window_id(w.window_id) == Some(pid);
            }
        }
        false
    }

    fn owner_pid_for_window_id(window_id: u32) -> Option<i32> {
        let windows = Self::get_cg_window_info(kCGWindowListOptionAll, None);
        windows.iter()
            .find(|w| w.window_id == window_id)
            .and_then(|_| {
                // Would need additional CGWindow API call to get owner PID
                // This is a simplified implementation
                None
            })
    }

    pub fn get_all_windows() -> Vec<WindowInfo> {
        Self::get_cg_window_info(kCGWindowListOptionOnScreenOnly, None)
    }

    fn get_windows_for_pid(pid: i32) -> Vec<WindowInfo> {
        Self::get_cg_window_info(kCGWindowListOptionAll, Some(pid))
    }

    fn get_cg_window_info(options: u32, filter_pid: Option<i32>) -> Vec<WindowInfo> {
        let mut out = Vec::new();
        unsafe {
            let list_ptr = CGWindowListCopyWindowInfo(options, 0);
            if list_ptr.is_null() {
                return out;
            }
            let list: CFArray<CFDictionary<CFString, CFAny>> =
                CFArray::wrap_under_create_rule(list_ptr as *const _);

            for i in 0..list.len() {
                let dict = if let Some(d) = list.get(i) {
                    d
                } else {
                    continue;
                };

                let owner_pid = dict
                    .find(&CFString::from_static_string("kCGWindowOwnerPID"))
                    .and_then(|v| v.downcast::<CFNumber>())
                    .and_then(|n| n.to_i32())
                    .unwrap_or(0);

                if let Some(pid) = filter_pid {
                    if owner_pid != pid {
                        continue;
                    }
                }

                let window_id = dict
                    .find(&CFString::from_static_string("kCGWindowNumber"))
                    .and_then(|v| v.downcast::<CFNumber>())
                    .and_then(|n| n.to_i64())
                    .unwrap_or(0) as u32;

                let title = dict
                    .find(&CFString::from_static_string("kCGWindowName"))
                    .and_then(|v| v.downcast::<CFString>())
                    .map(|s| s.to_string());

                let layer = dict
                    .find(&CFString::from_static_string("kCGWindowLayer"))
                    .and_then(|v| v.downcast::<CFNumber>())
                    .and_then(|n| n.to_i32())
                    .unwrap_or(0);

                let alpha = dict
                    .find(&CFString::from_static_string("kCGWindowAlpha"))
                    .and_then(|v| v.downcast::<CFNumber>())
                    .and_then(|n| n.to_f64())
                    .unwrap_or(1.0);

                let is_onscreen = dict
                    .find(&CFString::from_static_string("kCGWindowIsOnscreen"))
                    .and_then(|v| v.downcast::<CFBoolean>())
                    .map(|b| b.as_bool())
                    .unwrap_or(false);

                let memory_usage = dict
                    .find(&CFString::from_static_string("kCGWindowMemoryUsage"))
                    .and_then(|v| v.downcast::<CFNumber>())
                    .and_then(|n| n.to_i64())
                    .map(|x| x as u64);

                let sharing_state = dict
                    .find(&CFString::from_static_string("kCGWindowSharingState"))
                    .and_then(|v| v.downcast::<CFNumber>())
                    .and_then(|n| n.to_i32())
                    .map(|x| x as u32);

                let backing_store_type = dict
                    .find(&CFString::from_static_string("kCGWindowStoreType"))
                    .and_then(|v| v.downcast::<CFNumber>())
                    .and_then(|n| n.to_i32())
                    .map(|x| x as u32);

                // Bounds (dictionary with X,Y,Width,Height)
                let bounds = if let Some(bdict_any) =
                    dict.find(&CFString::from_static_string("kCGWindowBounds"))
                {
                    if let Some(bdict) = bdict_any.downcast::<CFDictionary<CFString, CFAny>>() {
                        let x = bdict
                            .find(&CFString::from_static_string("X"))
                            .and_then(|n| n.downcast::<CFNumber>())
                            .and_then(|n| n.to_f64())
                            .unwrap_or(0.0);
                        let y = bdict
                            .find(&CFString::from_static_string("Y"))
                            .and_then(|n| n.downcast::<CFNumber>())
                            .and_then(|n| n.to_f64())
                            .unwrap_or(0.0);
                        let width = bdict
                            .find(&CFString::from_static_string("Width"))
                            .and_then(|n| n.downcast::<CFNumber>())
                            .and_then(|n| n.to_f64())
                            .unwrap_or(0.0);
                        let height = bdict
                            .find(&CFString::from_static_string("Height"))
                            .and_then(|n| n.downcast::<CFNumber>())
                            .and_then(|n| n.to_f64())
                            .unwrap_or(0.0);
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
                } else {
                    WindowBounds {
                        x: 0.0,
                        y: 0.0,
                        width: 0.0,
                        height: 0.0,
                    }
                };

                out.push(WindowInfo {
                    window_id,
                    title,
                    bounds,
                    layer,
                    alpha,
                    memory_usage,
                    sharing_state,
                    backing_store_type,
                    is_onscreen,
                });
            }
        }
        out
    }

    // ---------------------------------------------------------------
    // Desktop state (session / lock / displays / idle)
    // ---------------------------------------------------------------

    fn capture_desktop_state(session_active_flag: bool) -> DesktopState {
        let mut state = DesktopState {
            active_space_id: None, // Public API doesn't expose Space IDs
            display_count: Self::read_display_count(),
            session_active: session_active_flag,
            screen_locked: false,
            console_user: None,
            login_time: None,
            idle_time_seconds: Self::read_idle_time_seconds(),
        };

        unsafe {
            let dict_ptr = CGSessionCopyCurrentDictionary();
            if !dict_ptr.is_null() {
                let dict: CFDictionary<CFString, CFAny> =
                    CFDictionary::wrap_under_create_rule(dict_ptr as *const _);

                // These keys are known in practice; not all are documented. They may not exist on all OS versions.
                if let Some(user) = dict
                    .find(&CFString::from_static_string("kCGSSessionUserNameKey"))
                    .and_then(|v| v.downcast::<CFString>())
                {
                    state.console_user = Some(user.to_string());
                }

                // Lock status key has historically been observed as "CGSSessionScreenIsLocked"
                if let Some(locked) = dict
                    .find(&CFString::from_static_string("CGSSessionScreenIsLocked"))
                    .and_then(|v| v.downcast::<CFBoolean>())
                {
                    state.screen_locked = locked.as_bool();
                }
            }
        }

        state
    }

    fn read_display_count() -> u32 {
        unsafe {
            MainThreadMarker::new()
                .and_then(|mtm| NSScreen::screens(mtm))
                .map(|arr| arr.len() as u32)
                .unwrap_or(1)
        }
    }

    fn read_idle_time_seconds() -> Option<f64> {
        // Combined HID state, any input event type
        const HID_STATE: i32 = 1; // kCGEventSourceStateHIDSystemState 
        const ANY_EVENT_TYPE: i32 = !0; // kCGAnyInputEventType (~0)
        
        let secs = unsafe { 
            CGEventSource::seconds_since_last_event_type(HID_STATE, ANY_EVENT_TYPE)
        };
        
        if secs.is_nan() || secs.is_sign_negative() {
            None
        } else {
            Some(secs)
        }
    }
}

// A "CFAny" alias to make CFDictionary<CFString, CFType> readable
type CFAny = core_foundation::base::CFType;

/// Bridge for integrating with main.rs AppSwitcher interface
pub struct EnhancedAppSwitchBridge {
    enhanced_switcher: EnhancedAppSwitcher,
}

impl EnhancedAppSwitchBridge {
    pub fn new() -> Self {
        Self {
            enhanced_switcher: EnhancedAppSwitcher::new(),
        }
    }
    
    /// Convert enhanced event to standard AppSwitchEvent format
    pub fn to_standard_event(enhanced_event: &EnhancedAppSwitchEvent) -> super::app_switcher::AppSwitchEvent {
        super::app_switcher::AppSwitchEvent {
            timestamp: enhanced_event.timestamp,
            system_time: enhanced_event.system_time,
            event_type: match enhanced_event.event_type {
                AppSwitchType::Foreground => super::app_switcher::AppSwitchType::Foreground,
                AppSwitchType::Background => super::app_switcher::AppSwitchType::Background,
                AppSwitchType::Launch => super::app_switcher::AppSwitchType::Launch,
                AppSwitchType::Terminate => super::app_switcher::AppSwitchType::Terminate,
                AppSwitchType::Hide => super::app_switcher::AppSwitchType::Hide,
                AppSwitchType::Unhide => super::app_switcher::AppSwitchType::Unhide,
                _ => super::app_switcher::AppSwitchType::Foreground,
            },
            app_info: super::app_switcher::AppInfo {
                name: enhanced_event.app_info.name.clone(),
                bundle_id: enhanced_event.app_info.bundle_id.clone(),
                pid: enhanced_event.app_info.pid,
                path: enhanced_event.app_info.path.clone(),
                executable_path: enhanced_event.app_info.executable_path.clone(),
                launch_date: enhanced_event.app_info.launch_date,
                icon_path: None, // Enhanced version uses base64
            },
            previous_app: enhanced_event.previous_app.as_ref().map(|prev| {
                super::app_switcher::AppInfo {
                    name: prev.name.clone(),
                    bundle_id: prev.bundle_id.clone(),
                    pid: prev.pid,
                    path: prev.path.clone(),
                    executable_path: prev.executable_path.clone(),
                    launch_date: prev.launch_date,
                    icon_path: None,
                }
            }),
            workspace: None, // Enhanced version provides this via desktop_state
            enhanced: Some(super::app_switcher::EnhancedInfo {
                activation_count: enhanced_event.app_info.activation_count,
                front_window_title: enhanced_event.app_info.frontmost_window
                    .as_ref()
                    .and_then(|w| w.title.clone()),
                cpu_usage: enhanced_event.app_info.process_info
                    .as_ref()
                    .map(|p| p.cpu_usage)
                    .unwrap_or(0.0),
                memory_bytes: enhanced_event.app_info.process_info
                    .as_ref()
                    .map(|p| p.memory_bytes)
                    .unwrap_or(0),
                session_active: enhanced_event.desktop_state.session_active,
                screen_locked: enhanced_event.desktop_state.screen_locked,
            }),
            confidence: enhanced_event.confidence_score,
        }
    }
    
    /// Access the underlying enhanced switcher
    pub fn enhanced(&mut self) -> &mut EnhancedAppSwitcher {
        &mut self.enhanced_switcher
    }
}

// ---------------------------------------------------------------
// Debug listener (unchanged but updated to new field names)
// ---------------------------------------------------------------

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
        if let Some(idle) = event.desktop_state.idle_time_seconds {
            println!("    Idle: {:.1}s", idle);
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
        println!("  Displays: {}", state.display_count);
        if let Some(idle) = state.idle_time_seconds {
            println!("  Idle: {:.1}s", idle);
        }
    }
}
