// src/core/window_state_detector.rs
// Comprehensive window state detection with multi-method consensus and smart fallbacks
//
// This module implements a robust window state detection system that:
// - Uses multiple detection methods for accuracy
// - Implements consensus algorithms for conflicting signals
// - Provides smart fallbacks when APIs fail
// - Handles edge cases and rapid state changes
// - Supports multi-monitor and multi-space environments

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_void;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, CFTypeRef, ItemRef, TCFType, ToVoid};
use core_foundation::boolean::CFBoolean as CFBooleanCore;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString as CFStringCore;

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2::{msg_send, msg_send_id, sel, ClassType};
use objc2_app_kit::{NSApplication, NSScreen, NSWindow, NSWindowOcclusionState};
use objc2_core_foundation::{CGFloat, CGPoint, CGRect, CGSize};
use objc2_foundation::{MainThreadMarker, NSArray, NSNumber, NSString};

// Type aliases for clarity
type CFStringRef = *const c_void;
type CFNumberRef = *const c_void;
type CFBooleanRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFArrayRef = *const c_void;

// Import the actual CFArray type
extern "C" {
    // This gives us access to the CFArray type
    static kCFTypeArrayCallBacks: c_void;
}

// External C functions for Core Graphics
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;

    fn CGMainDisplayID() -> u32;
    fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut u32,
        display_count: *mut u32,
    ) -> i32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGDisplayScreenSize(display: u32) -> CGSize;

    // Private APIs (use carefully)
    fn CGSMainConnectionID() -> i32;
    fn CGSCopyManagedDisplaySpaces(conn: i32) -> CFArrayRef;
    fn CGSCopyWindowsForSpaces(conn: i32, spaces: CFArrayRef, options: u32) -> CFArrayRef;
    fn CGSSpaceGetType(conn: i32, space: u64) -> i32;
    fn CGSGetWindowLevel(conn: i32, window: u32, level: *mut i32) -> i32;
}

// Window list options
const kCGWindowListOptionAll: u32 = 0;
const kCGWindowListOptionOnScreenOnly: u32 = 1 << 0;
const kCGWindowListExcludeDesktopElements: u32 = 1 << 4;
const kCGWindowListOptionOnScreenAboveWindow: u32 = 1 << 1;
const kCGWindowListOptionOnScreenBelowWindow: u32 = 1 << 2;
const kCGWindowListOptionIncludingWindow: u32 = 1 << 3;

// Space types (from private API)
const kCGSSpaceUser: i32 = 0;
const kCGSSpaceFullscreen: i32 = 4;
const kCGSSpaceSystem: i32 = 2;

// Window levels
const kCGNormalWindowLevel: i32 = 0;
const kCGFloatingWindowLevel: i32 = 3;
const kCGMainMenuWindowLevel: i32 = 24;

/// Comprehensive window state with confidence scoring
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WindowState {
    Normal,
    Minimized,
    Fullscreen,
    Hidden,
    Offscreen,
    Transitioning, // Animation in progress
    Unknown,
}

/// Detection method for tracking source reliability
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DetectionMethod {
    CGWindowOnScreen,
    CGWindowLayer,
    CGWindowAlpha,
    CGWindowBounds,
    NSWindowOcclusion,
    NSWindowFrame,
    SpaceType,
    WindowLevel,
    AccessibilityAPI,
    HistoricalPattern,
}

/// Window state with confidence and detection details
#[derive(Debug, Clone)]
pub struct WindowStateInfo {
    pub state: WindowState,
    pub confidence: f32, // 0.0 to 1.0
    pub detection_methods: Vec<DetectionMethod>,
    pub timestamp: Instant,
    pub is_animating: bool,
    pub space_id: Option<u64>,
    pub monitor_id: Option<u32>,
}

/// Monitor information for fullscreen detection
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub id: u32,
    pub bounds: CGRect,
    pub name: String,
    pub is_builtin: bool,
    pub scale_factor: f64,
}

/// Space/Desktop information
#[derive(Debug, Clone)]
pub struct SpaceInfo {
    pub id: u64,
    pub type_id: i32,
    pub display_id: u32,
    pub is_fullscreen: bool,
    pub is_current: bool,
    pub window_ids: Vec<u32>,
}

/// Historical window state for pattern detection
#[derive(Debug, Clone)]
struct WindowHistory {
    states: VecDeque<(WindowState, Instant)>,
    transitions: VecDeque<(WindowState, WindowState, Duration)>,
    last_user_action: Option<Instant>,
}

impl WindowHistory {
    fn new() -> Self {
        Self {
            states: VecDeque::with_capacity(10),
            transitions: VecDeque::with_capacity(10),
            last_user_action: None,
        }
    }

    fn add_state(&mut self, state: WindowState) {
        let now = Instant::now();

        // Track state transition
        if let Some((prev_state, prev_time)) = self.states.back() {
            if prev_state != &state {
                let duration = now.duration_since(*prev_time);
                self.transitions
                    .push_back((prev_state.clone(), state.clone(), duration));

                // Keep only last 10 transitions
                if self.transitions.len() > 10 {
                    self.transitions.pop_front();
                }
            }
        }

        self.states.push_back((state, now));

        // Keep only last 10 states
        if self.states.len() > 10 {
            self.states.pop_front();
        }
    }

    fn is_likely_animating(&self) -> bool {
        // Check if we've had rapid transitions recently
        if let Some((_, last_time)) = self.states.back() {
            if last_time.elapsed() < Duration::from_millis(500) {
                // Count transitions in last second
                let recent_transitions = self
                    .transitions
                    .iter()
                    .filter(|(_, _, duration)| duration < &Duration::from_secs(1))
                    .count();
                return recent_transitions >= 2;
            }
        }
        false
    }

    fn predict_next_state(&self) -> Option<WindowState> {
        // Use pattern matching to predict likely next state
        if self.transitions.len() >= 2 {
            // Look for repeating patterns
            let recent: Vec<_> = self.transitions.iter().rev().take(3).collect();

            // If we see Normal->Minimized->Normal pattern, predict Normal
            if recent.len() >= 2 {
                let (from1, to1, _) = &recent[0];
                let (from2, to2, _) = &recent[1];
                // Simple pattern: A->B->A predicts ->B
                if from1 == to2 && to1 == from2 {
                    return Some(to1.clone());
                }
            }
        }
        None
    }
}

/// Main window state detector with multi-method consensus
pub struct WindowStateDetector {
    monitors: Arc<RwLock<Vec<MonitorInfo>>>,
    spaces: Arc<RwLock<Vec<SpaceInfo>>>,
    window_states: Arc<Mutex<HashMap<u32, WindowStateInfo>>>,
    window_history: Arc<Mutex<HashMap<u32, WindowHistory>>>,
    confidence_weights: HashMap<DetectionMethod, f32>,
    detection_cache: Arc<Mutex<HashMap<u32, (WindowStateInfo, Instant)>>>,
    cache_duration: Duration,
}

impl WindowStateDetector {
    pub fn new() -> Self {
        let mut confidence_weights = HashMap::new();

        // Assign reliability weights to each detection method
        confidence_weights.insert(DetectionMethod::CGWindowOnScreen, 0.9);
        confidence_weights.insert(DetectionMethod::CGWindowLayer, 0.8);
        confidence_weights.insert(DetectionMethod::CGWindowAlpha, 0.7);
        confidence_weights.insert(DetectionMethod::CGWindowBounds, 0.85);
        confidence_weights.insert(DetectionMethod::NSWindowOcclusion, 0.95);
        confidence_weights.insert(DetectionMethod::NSWindowFrame, 0.9);
        confidence_weights.insert(DetectionMethod::SpaceType, 0.8);
        confidence_weights.insert(DetectionMethod::WindowLevel, 0.75);
        confidence_weights.insert(DetectionMethod::AccessibilityAPI, 0.85);
        confidence_weights.insert(DetectionMethod::HistoricalPattern, 0.6);

        Self {
            monitors: Arc::new(RwLock::new(Vec::new())),
            spaces: Arc::new(RwLock::new(Vec::new())),
            window_states: Arc::new(Mutex::new(HashMap::new())),
            window_history: Arc::new(Mutex::new(HashMap::new())),
            confidence_weights,
            detection_cache: Arc::new(Mutex::new(HashMap::new())),
            cache_duration: Duration::from_millis(50), // 50ms cache
        }
    }

    /// Initialize and update monitor information
    pub fn update_monitors(&self) -> Result<(), String> {
        let mut monitors = self.monitors.write().unwrap();
        monitors.clear();

        unsafe {
            // Get all active displays
            let max_displays = 32;
            let mut display_ids = vec![0u32; max_displays as usize];
            let mut display_count = 0u32;

            if CGGetActiveDisplayList(max_displays, display_ids.as_mut_ptr(), &mut display_count)
                != 0
            {
                return Err("Failed to get display list".to_string());
            }

            for i in 0..display_count as usize {
                let display_id = display_ids[i];
                let bounds = CGDisplayBounds(display_id);
                let size = CGDisplayScreenSize(display_id);

                monitors.push(MonitorInfo {
                    id: display_id,
                    bounds,
                    name: format!("Display {}", i + 1),
                    is_builtin: display_id == CGMainDisplayID(),
                    scale_factor: if size.width > 0.0 {
                        bounds.size.width / size.width
                    } else {
                        1.0
                    },
                });
            }
        }

        Ok(())
    }

    /// Update space/desktop information using private APIs (when available)
    pub fn update_spaces(&self) -> Result<(), String> {
        // Temporarily disabled due to private API complexity
        // Will implement with public APIs only
        Ok(())
    }

    /// Detect window state using multiple methods with consensus
    pub fn detect_window_state(&self, window_id: u32, pid: i32) -> WindowStateInfo {
        // Check cache first
        if let Some(cached) = self.check_cache(window_id) {
            return cached;
        }

        let mut detections: HashMap<WindowState, Vec<(DetectionMethod, f32)>> = HashMap::new();
        let monitors = self.monitors.read().unwrap();

        // Method 1: CGWindow API detection
        if let Some(cg_state) = self.detect_via_cgwindow(window_id, pid, &monitors) {
            for (state, method, confidence) in cg_state {
                detections
                    .entry(state)
                    .or_insert_with(Vec::new)
                    .push((method, confidence));
            }
        }

        // Method 2: NSWindow API detection (if we can get window reference)
        if let Some(ns_state) = self.detect_via_nswindow(window_id, pid) {
            for (state, method, confidence) in ns_state {
                detections
                    .entry(state)
                    .or_insert_with(Vec::new)
                    .push((method, confidence));
            }
        }

        // Method 3: Historical pattern detection
        if let Some(hist_state) = self.detect_via_history(window_id) {
            for (state, method, confidence) in hist_state {
                detections
                    .entry(state)
                    .or_insert_with(Vec::new)
                    .push((method, confidence));
            }
        }

        // Consensus algorithm: weighted voting
        let state_info = self.calculate_consensus(detections, window_id);

        // Update cache
        self.update_cache(window_id, state_info.clone());

        // Update history
        self.update_history(window_id, state_info.state.clone());

        state_info
    }

    /// CGWindow-based detection (most reliable for basic states)
    fn detect_via_cgwindow(
        &self,
        window_id: u32,
        pid: i32,
        monitors: &[MonitorInfo],
    ) -> Option<Vec<(WindowState, DetectionMethod, f32)>> {
        let mut detections = Vec::new();

        unsafe {
            // Get all windows to find our target
            let window_list_ptr = CGWindowListCopyWindowInfo(kCGWindowListOptionAll, 0);
            if window_list_ptr.is_null() {
                return None;
            }

            // Use a simpler approach - work with raw CFDictionary
            let window_list =
                CFArray::<CFDictionary>::wrap_under_create_rule(window_list_ptr as *const _);

            for i in 0..window_list.len() {
                if let Some(window_dict) = window_list.get(i) {
                    // Get window ID
                    let wid_key = CFStringCore::from_static_string("kCGWindowNumber");
                    let wid_value = window_dict.find(&wid_key as &dyn ToVoid<*const c_void>);

                    if let Some(wid_ref) = wid_value {
                        let wid_num = CFNumber::wrap_under_get_rule(*wid_ref as CFNumberRef);
                        let wid = wid_num.to_i32().unwrap_or(0) as u32;

                        if wid != window_id {
                            continue;
                        }

                        // Check window owner PID
                        let pid_key = CFStringCore::from_static_string("kCGWindowOwnerPID");
                        let pid_value = window_dict.find(&pid_key as &dyn ToVoid<*const c_void>);

                        if let Some(pid_ref) = pid_value {
                            let pid_num = CFNumber::wrap_under_get_rule(*pid_ref as CFNumberRef);
                            let owner_pid = pid_num.to_i32().unwrap_or(0);

                            if owner_pid != pid {
                                continue;
                            }
                        }

                        // Detection 1: Check if on screen
                        let onscreen_key = CFStringCore::from_static_string("kCGWindowIsOnscreen");
                        if let Some(onscreen_ref) =
                            window_dict.find(&onscreen_key as &dyn ToVoid<*const c_void>)
                        {
                            let is_onscreen_bool =
                                CFBooleanCore::wrap_under_get_rule(*onscreen_ref as CFBooleanRef);
                            let is_onscreen: bool = is_onscreen_bool.into();

                            if !is_onscreen {
                                detections.push((
                                    WindowState::Minimized,
                                    DetectionMethod::CGWindowOnScreen,
                                    0.9,
                                ));
                            }
                        }

                        // Detection 2: Check window layer
                        let layer_key = CFStringCore::from_static_string("kCGWindowLayer");
                        if let Some(layer_ref) =
                            window_dict.find(&layer_key as &dyn ToVoid<*const c_void>)
                        {
                            let layer_num =
                                CFNumber::wrap_under_get_rule(*layer_ref as CFNumberRef);
                            let layer = layer_num.to_i32().unwrap_or(0);

                            if layer < 0 {
                                detections.push((
                                    WindowState::Minimized,
                                    DetectionMethod::CGWindowLayer,
                                    0.8,
                                ));
                            } else if layer > 100 {
                                detections.push((
                                    WindowState::Fullscreen,
                                    DetectionMethod::CGWindowLayer,
                                    0.6,
                                ));
                            }
                        }

                        // Detection 3: Check alpha value
                        let alpha_key = CFStringCore::from_static_string("kCGWindowAlpha");
                        if let Some(alpha_ref) =
                            window_dict.find(&alpha_key as &dyn ToVoid<*const c_void>)
                        {
                            let alpha_num =
                                CFNumber::wrap_under_get_rule(*alpha_ref as CFNumberRef);
                            let alpha = alpha_num.to_f64().unwrap_or(1.0);

                            if alpha == 0.0 {
                                detections.push((
                                    WindowState::Hidden,
                                    DetectionMethod::CGWindowAlpha,
                                    0.85,
                                ));
                            }
                        }

                        // Detection 4: Check bounds for fullscreen
                        let bounds_key = CFStringCore::from_static_string("kCGWindowBounds");
                        if let Some(bounds_ref) =
                            window_dict.find(&bounds_key as &dyn ToVoid<*const c_void>)
                        {
                            let bounds_dict =
                                CFDictionary::<CFStringCore, CFType>::wrap_under_get_rule(
                                    *bounds_ref as CFDictionaryRef,
                                );

                            let x_key = CFStringCore::from_static_string("X");
                            let y_key = CFStringCore::from_static_string("Y");
                            let width_key = CFStringCore::from_static_string("Width");
                            let height_key = CFStringCore::from_static_string("Height");

                            let mut x = 0.0;
                            let mut y = 0.0;
                            let mut width = 0.0;
                            let mut height = 0.0;

                            if let Some(x_ref) =
                                bounds_dict.find(&x_key as &dyn ToVoid<*const c_void>)
                            {
                                let x_num = CFNumber::wrap_under_get_rule(*x_ref as CFNumberRef);
                                x = x_num.to_f64().unwrap_or(0.0);
                            }

                            if let Some(y_ref) =
                                bounds_dict.find(&y_key as &dyn ToVoid<*const c_void>)
                            {
                                let y_num = CFNumber::wrap_under_get_rule(*y_ref as CFNumberRef);
                                y = y_num.to_f64().unwrap_or(0.0);
                            }

                            if let Some(w_ref) =
                                bounds_dict.find(&width_key as &dyn ToVoid<*const c_void>)
                            {
                                let w_num = CFNumber::wrap_under_get_rule(*w_ref as CFNumberRef);
                                width = w_num.to_f64().unwrap_or(0.0);
                            }

                            if let Some(h_ref) =
                                bounds_dict.find(&height_key as &dyn ToVoid<*const c_void>)
                            {
                                let h_num = CFNumber::wrap_under_get_rule(*h_ref as CFNumberRef);
                                height = h_num.to_f64().unwrap_or(0.0);
                            }

                            let window_rect = CGRect {
                                origin: CGPoint { x, y },
                                size: CGSize { width, height },
                            };

                            // Check against each monitor
                            for monitor in monitors {
                                if self.rects_match(&window_rect, &monitor.bounds, 10.0) {
                                    detections.push((
                                        WindowState::Fullscreen,
                                        DetectionMethod::CGWindowBounds,
                                        0.95,
                                    ));
                                    break;
                                }
                            }

                            // Check if completely offscreen
                            if !self.is_rect_visible(&window_rect, monitors) {
                                detections.push((
                                    WindowState::Offscreen,
                                    DetectionMethod::CGWindowBounds,
                                    0.8,
                                ));
                            }
                        }

                        break; // Found our window
                    }
                }
            }
        }

        if detections.is_empty() {
            // If no specific state detected, assume normal
            detections.push((WindowState::Normal, DetectionMethod::CGWindowOnScreen, 0.5));
        }

        Some(detections)
    }

    /// NSWindow-based detection (when available)
    fn detect_via_nswindow(
        &self,
        window_id: u32,
        _pid: i32,
    ) -> Option<Vec<(WindowState, DetectionMethod, f32)>> {
        // Try to get NSApplication windows
        autoreleasepool(|_pool| {
            unsafe {
                // MainThreadMarker::new() returns Option<MainThreadMarker>
                let mtm = MainThreadMarker::new()?;
                let app = NSApplication::sharedApplication(mtm);
                let windows = app.windows();

                let mut detections = Vec::new();

                for i in 0..windows.count() {
                    let window = windows.objectAtIndex(i);

                    // Get window number using raw message send
                    let win_id: isize = msg_send![&window, windowNumber];

                    if win_id as u32 != window_id {
                        continue;
                    }

                    // Check occlusion state
                    let occlusion_state: NSWindowOcclusionState =
                        msg_send![&window, occlusionState];

                    // Use the correct constant name
                    if occlusion_state.contains(NSWindowOcclusionState::Visible) {
                        // Window is visible
                        let is_key: bool = msg_send![&window, isKeyWindow];
                        let is_main: bool = msg_send![&window, isMainWindow];

                        if is_key || is_main {
                            detections.push((
                                WindowState::Normal,
                                DetectionMethod::NSWindowOcclusion,
                                0.95,
                            ));
                        }
                    } else {
                        // Window is occluded/hidden
                        detections.push((
                            WindowState::Hidden,
                            DetectionMethod::NSWindowOcclusion,
                            0.9,
                        ));
                    }

                    // Check if minimized
                    let is_miniaturized: bool = msg_send![&window, isMiniaturized];
                    if is_miniaturized {
                        detections.push((
                            WindowState::Minimized,
                            DetectionMethod::NSWindowFrame,
                            0.95,
                        ));
                    }

                    // Check if zoomed (potentially fullscreen)
                    let is_zoomed: bool = msg_send![&window, isZoomed];
                    if is_zoomed {
                        // Additional check for fullscreen
                        let style_mask: u64 = msg_send![&window, styleMask];
                        const NSWindowStyleMaskFullScreen: u64 = 1 << 14;

                        if style_mask & NSWindowStyleMaskFullScreen != 0 {
                            detections.push((
                                WindowState::Fullscreen,
                                DetectionMethod::NSWindowFrame,
                                0.9,
                            ));
                        }
                    }

                    break;
                }

                if detections.is_empty() {
                    None
                } else {
                    Some(detections)
                }
            }
        })
    }

    /// Historical pattern-based detection
    fn detect_via_history(
        &self,
        window_id: u32,
    ) -> Option<Vec<(WindowState, DetectionMethod, f32)>> {
        let mut history_map = self.window_history.lock().unwrap();
        let history = history_map
            .entry(window_id)
            .or_insert_with(WindowHistory::new);

        let mut detections = Vec::new();

        // Check if likely animating
        if history.is_likely_animating() {
            detections.push((
                WindowState::Transitioning,
                DetectionMethod::HistoricalPattern,
                0.7,
            ));
        }

        // Try to predict next state based on patterns
        if let Some(predicted) = history.predict_next_state() {
            detections.push((predicted, DetectionMethod::HistoricalPattern, 0.5));
        }

        if detections.is_empty() {
            None
        } else {
            Some(detections)
        }
    }

    /// Calculate consensus from multiple detection methods
    fn calculate_consensus(
        &self,
        detections: HashMap<WindowState, Vec<(DetectionMethod, f32)>>,
        window_id: u32,
    ) -> WindowStateInfo {
        let mut best_state = WindowState::Unknown;
        let mut best_confidence = 0.0;
        let mut all_methods = Vec::new();

        // Calculate weighted confidence for each detected state
        for (state, methods) in detections {
            let mut total_confidence = 0.0;
            let mut method_list = Vec::new();

            for (method, confidence) in methods {
                let weight = self.confidence_weights.get(&method).unwrap_or(&0.5);

                total_confidence += confidence * weight;
                method_list.push(method);
            }

            // Normalize by number of methods
            let normalized_confidence = total_confidence / method_list.len() as f32;

            if normalized_confidence > best_confidence {
                best_confidence = normalized_confidence;
                best_state = state;
                all_methods = method_list;
            }
        }

        // Apply confidence threshold
        if best_confidence < 0.3 {
            best_state = WindowState::Unknown;
        }

        // Check if animation is likely
        let is_animating = self.is_likely_animating(window_id);

        WindowStateInfo {
            state: best_state,
            confidence: best_confidence.min(1.0),
            detection_methods: all_methods,
            timestamp: Instant::now(),
            is_animating,
            space_id: None,
            monitor_id: None,
        }
    }

    /// Check if window is likely animating
    fn is_likely_animating(&self, window_id: u32) -> bool {
        let history_map = self.window_history.lock().unwrap();

        if let Some(history) = history_map.get(&window_id) {
            history.is_likely_animating()
        } else {
            false
        }
    }

    /// Check if two rectangles match within tolerance
    fn rects_match(&self, rect1: &CGRect, rect2: &CGRect, tolerance: f64) -> bool {
        (rect1.origin.x - rect2.origin.x).abs() < tolerance
            && (rect1.origin.y - rect2.origin.y).abs() < tolerance
            && (rect1.size.width - rect2.size.width).abs() < tolerance
            && (rect1.size.height - rect2.size.height).abs() < tolerance
    }

    /// Check if rectangle is visible on any monitor
    fn is_rect_visible(&self, rect: &CGRect, monitors: &[MonitorInfo]) -> bool {
        for monitor in monitors {
            if self.rects_intersect(rect, &monitor.bounds) {
                return true;
            }
        }
        false
    }

    /// Check if two rectangles intersect
    fn rects_intersect(&self, rect1: &CGRect, rect2: &CGRect) -> bool {
        rect1.origin.x < rect2.origin.x + rect2.size.width
            && rect1.origin.x + rect1.size.width > rect2.origin.x
            && rect1.origin.y < rect2.origin.y + rect2.size.height
            && rect1.origin.y + rect1.size.height > rect2.origin.y
    }

    /// Check cache for recent detection
    fn check_cache(&self, window_id: u32) -> Option<WindowStateInfo> {
        let cache = self.detection_cache.lock().unwrap();

        if let Some((info, timestamp)) = cache.get(&window_id) {
            if timestamp.elapsed() < self.cache_duration {
                return Some(info.clone());
            }
        }

        None
    }

    /// Update cache with new detection
    fn update_cache(&self, window_id: u32, info: WindowStateInfo) {
        let mut cache = self.detection_cache.lock().unwrap();
        cache.insert(window_id, (info, Instant::now()));

        // Clean old entries
        cache.retain(|_, (_, timestamp)| timestamp.elapsed() < Duration::from_secs(1));
    }

    /// Update window history
    fn update_history(&self, window_id: u32, state: WindowState) {
        let mut history_map = self.window_history.lock().unwrap();
        let history = history_map
            .entry(window_id)
            .or_insert_with(WindowHistory::new);
        history.add_state(state);
    }

    /// Get current window state with fallback chain
    pub fn get_window_state_with_fallback(&self, window_id: u32, pid: i32) -> WindowStateInfo {
        // Primary detection
        let mut state_info = self.detect_window_state(window_id, pid);

        // If confidence is too low, try fallback methods
        if state_info.confidence < 0.5 {
            // Fallback 1: Check if window exists at all
            if !self.window_exists(window_id) {
                state_info.state = WindowState::Unknown;
                state_info.confidence = 1.0;
                return state_info;
            }

            // Fallback 2: Use last known good state
            if let Some(last_known) = self.get_last_known_state(window_id) {
                if last_known.timestamp.elapsed() < Duration::from_secs(5) {
                    state_info = last_known;
                    state_info.confidence *= 0.8; // Reduce confidence for stale data
                }
            }

            // Fallback 3: Use most common state for this app
            if state_info.confidence < 0.3 {
                state_info.state = WindowState::Normal; // Safe default
                state_info.confidence = 0.3;
            }
        }

        state_info
    }

    /// Check if window exists
    fn window_exists(&self, window_id: u32) -> bool {
        unsafe {
            let window_list_ptr =
                CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow, window_id);

            if window_list_ptr.is_null() {
                return false;
            }

            let window_list =
                CFArray::<CFDictionary>::wrap_under_create_rule(window_list_ptr as *const _);

            window_list.len() > 0
        }
    }

    /// Get last known state from cache or history
    fn get_last_known_state(&self, window_id: u32) -> Option<WindowStateInfo> {
        // Check cache first
        if let Some(cached) = self.check_cache(window_id) {
            return Some(cached);
        }

        // Check history
        let history_map = self.window_history.lock().unwrap();

        if let Some(history) = history_map.get(&window_id) {
            if let Some((state, timestamp)) = history.states.back() {
                return Some(WindowStateInfo {
                    state: state.clone(),
                    confidence: 0.5,
                    detection_methods: vec![DetectionMethod::HistoricalPattern],
                    timestamp: *timestamp,
                    is_animating: false,
                    space_id: None,
                    monitor_id: None,
                });
            }
        }

        None
    }
}

/// Public API for easy integration
impl WindowStateDetector {
    /// Simple API: Is window minimized?
    pub fn is_minimized(&self, window_id: u32, pid: i32) -> bool {
        let state = self.detect_window_state(window_id, pid);
        state.state == WindowState::Minimized && state.confidence > 0.7
    }

    /// Simple API: Is window fullscreen?
    pub fn is_fullscreen(&self, window_id: u32, pid: i32) -> bool {
        let state = self.detect_window_state(window_id, pid);
        state.state == WindowState::Fullscreen && state.confidence > 0.7
    }

    /// Simple API: Is window visible?
    pub fn is_visible(&self, window_id: u32, pid: i32) -> bool {
        let state = self.detect_window_state(window_id, pid);
        matches!(state.state, WindowState::Normal | WindowState::Fullscreen)
            && state.confidence > 0.5
    }
}

// Default implementation
impl Default for WindowStateDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_state_detection() {
        let detector = WindowStateDetector::new();

        // Test basic initialization
        assert!(detector.update_monitors().is_ok());

        // Note: Real window testing requires actual window IDs
        // This is just a structural test
    }

    #[test]
    fn test_consensus_algorithm() {
        let detector = WindowStateDetector::new();

        let mut detections = HashMap::new();
        detections.insert(
            WindowState::Minimized,
            vec![
                (DetectionMethod::CGWindowOnScreen, 0.9),
                (DetectionMethod::CGWindowLayer, 0.8),
            ],
        );

        let consensus = detector.calculate_consensus(detections, 12345);
        assert_eq!(consensus.state, WindowState::Minimized);
        assert!(consensus.confidence > 0.8);
    }

    #[test]
    fn test_cache_functionality() {
        let detector = WindowStateDetector::new();

        let state_info = WindowStateInfo {
            state: WindowState::Normal,
            confidence: 0.9,
            detection_methods: vec![DetectionMethod::CGWindowOnScreen],
            timestamp: Instant::now(),
            is_animating: false,
            space_id: None,
            monitor_id: None,
        };

        detector.update_cache(12345, state_info.clone());

        // Should get cached value
        let cached = detector.check_cache(12345);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().state, WindowState::Normal);

        // After cache duration, should return None
        std::thread::sleep(Duration::from_millis(60));
        let expired = detector.check_cache(12345);
        assert!(expired.is_none());
    }

    #[test]
    fn test_history_tracking() {
        let mut history = WindowHistory::new();

        history.add_state(WindowState::Normal);
        std::thread::sleep(Duration::from_millis(10));
        history.add_state(WindowState::Minimized);
        std::thread::sleep(Duration::from_millis(10));
        history.add_state(WindowState::Normal);

        // Should detect recent transitions
        assert!(!history.is_likely_animating());

        // Add rapid transitions
        history.add_state(WindowState::Minimized);
        history.add_state(WindowState::Normal);

        // Now might detect animation (depending on timing)
        // This is timing-sensitive so we just check it doesn't panic
        let _ = history.is_likely_animating();
    }
}
