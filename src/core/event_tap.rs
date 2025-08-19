// src/core/event_tap.rs
//! Enhanced CGEventTap implementation for comprehensive event monitoring
//!
//! This module provides low-level event monitoring capabilities that complement
//! our two-layer app switching system. It captures keyboard shortcuts, mouse
//! events, and scroll actions for complete context awareness.

use std::collections::HashMap;
use std::os::raw::{c_void, c_double};
use std::ptr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

// Define our own CGEvent types since core-graphics crate is incomplete
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CGEventType(pub u32);

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CGEventFlags(pub u64);

#[repr(C)]
#[derive(Debug, Clone)]
pub struct CGEvent(*mut c_void);

#[repr(C)]
pub struct CGEventTapProxy(*mut c_void);

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CGPoint {
    pub x: c_double,
    pub y: c_double,
}

// Keyboard event data
#[derive(Debug, Clone)]
pub struct KeyboardEvent {
    pub keycode: u16,
    pub flags: CGEventFlags,
    pub timestamp: Instant,
    pub is_shortcut: bool,
    pub shortcut_type: Option<ShortcutType>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShortcutType {
    Copy,     // Cmd+C
    Paste,    // Cmd+V
    Cut,      // Cmd+X
    SelectAll,// Cmd+A
    Undo,     // Cmd+Z
    Redo,     // Cmd+Shift+Z
    Save,     // Cmd+S
    Open,     // Cmd+O
    Find,     // Cmd+F
    Custom(String),
}

pub type EventCallback = Arc<Mutex<dyn Fn(EventInfo) + Send + 'static>>;

#[derive(Debug, Clone)]
pub enum EventInfo {
    Keyboard(KeyboardEvent),
    Mouse { position: CGPoint, button: MouseButton, action: MouseAction },
    Scroll { position: CGPoint, delta_x: f64, delta_y: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u8),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MouseAction {
    Down,
    Up,
    Moved,
    Dragged,
}

/// Enhanced event tap for comprehensive input monitoring
pub struct EventTap {
    tap: Option<*mut c_void>,
    callback: EventCallback,
    /// Track modifier key states
    modifier_states: Arc<Mutex<ModifierState>>,
    /// Debounce rapid events
    last_event_time: Arc<Mutex<HashMap<String, Instant>>>,
}

#[derive(Debug, Clone, Default)]
struct ModifierState {
    cmd: bool,
    shift: bool,
    option: bool,
    control: bool,
    caps_lock: bool,
}

impl EventTap {
    pub fn new(callback: EventCallback) -> Self {
        EventTap {
            tap: None,
            callback,
            modifier_states: Arc::new(Mutex::new(ModifierState::default())),
            last_event_time: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Detect keyboard shortcuts from keycode and modifiers
    fn detect_shortcut(keycode: u16, flags: CGEventFlags) -> Option<ShortcutType> {
        let cmd = (flags.0 & CGEventFlags::CMD_KEY) != 0;
        let shift = (flags.0 & CGEventFlags::SHIFT_KEY) != 0;
        let option = (flags.0 & CGEventFlags::OPTION_KEY) != 0;
        let control = (flags.0 & CGEventFlags::CONTROL_KEY) != 0;
        
        if cmd && !option && !control {
            match keycode {
                8 => Some(ShortcutType::Copy),     // C
                9 => Some(ShortcutType::Paste),    // V
                7 => Some(ShortcutType::Cut),      // X
                0 => Some(ShortcutType::SelectAll), // A
                6 if shift => Some(ShortcutType::Redo), // Z with Shift
                6 => Some(ShortcutType::Undo),     // Z
                1 => Some(ShortcutType::Save),     // S
                31 => Some(ShortcutType::Open),    // O
                3 => Some(ShortcutType::Find),     // F
                _ => None,
            }
        } else {
            None
        }
    }

    pub fn start_monitoring(&mut self, track_mouse: bool, track_keyboard: bool, track_scroll: bool) -> Result<(), String> {
        unsafe {
            let mut event_mask: u64 = 0;
            
            if track_mouse {
                // Mouse events
                event_mask |= 1 << CGEventType::LeftMouseDown.0 as u64;
                event_mask |= 1 << CGEventType::LeftMouseUp.0 as u64;
                event_mask |= 1 << CGEventType::RightMouseDown.0 as u64;
                event_mask |= 1 << CGEventType::RightMouseUp.0 as u64;
                event_mask |= 1 << CGEventType::MouseMoved.0 as u64;
                event_mask |= 1 << CGEventType::LeftMouseDragged.0 as u64;
                event_mask |= 1 << CGEventType::RightMouseDragged.0 as u64;
            }
            
            if track_keyboard {
                // Keyboard events
                event_mask |= 1 << CGEventType::KeyDown.0 as u64;
                event_mask |= 1 << CGEventType::KeyUp.0 as u64;
                event_mask |= 1 << CGEventType::FlagsChanged.0 as u64;
            }
            
            if track_scroll {
                // Scroll events
                event_mask |= 1 << CGEventType::ScrollWheel.0 as u64;
            }
            
            if event_mask == 0 {
                return Ok(()); // Nothing to monitor
            }
            
            // Create event tap
            let tap = CGEventTapCreate(
                CGEventTapLocation::HIDEventTap,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                event_mask,
                event_tap_callback,
                self as *mut _ as *mut c_void,
            );
            
            if tap.is_null() {
                return Err("Failed to create event tap".to_string());
            }
            
            self.tap = Some(tap);
            
            // Add to run loop
            let run_loop_source = CFMachPortCreateRunLoopSource(ptr::null_mut(), tap, 0);
            if run_loop_source.is_null() {
                CFRelease(tap as _);
                self.tap = None;
                return Err("Failed to create run loop source".to_string());
            }
            
            let run_loop = CFRunLoopGetCurrent();
            CFRunLoopAddSource(run_loop, run_loop_source, kCFRunLoopDefaultMode);
            CFRelease(run_loop_source as _);
            
            // Enable the event tap
            CGEventTapEnable(tap, true);
            
            Ok(())
        }
    }
    
    pub fn stop_monitoring(&mut self) {
        if let Some(tap) = self.tap.take() {
            unsafe {
                CGEventTapEnable(tap, false);
                CFRelease(tap as _);
            }
        }
    }
}

extern "C" fn event_tap_callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: CGEvent,
    user_info: *mut c_void,
) -> CGEvent {
    unsafe {
        let tap = &*(user_info as *const EventTap);
        
        // Get mouse position for all events
        let position = CGEventGetLocation(event.0);
        
        let event_info = match event_type.0 {
            // Keyboard events
            10 => { // KeyDown
                let keycode = CGEventGetIntegerValueField(event.0, CGEventField::KeyboardEventKeycode) as u16;
                let flags = CGEventFlags(CGEventGetFlags(event.0));
                let shortcut_type = EventTap::detect_shortcut(keycode, flags);
                
                EventInfo::Keyboard(KeyboardEvent {
                    keycode,
                    flags,
                    timestamp: Instant::now(),
                    is_shortcut: shortcut_type.is_some(),
                    shortcut_type,
                })
            }
            // Mouse events
            1 | 2 => { // LeftMouseDown | LeftMouseUp
                EventInfo::Mouse {
                    position,
                    button: MouseButton::Left,
                    action: if event_type.0 == 1 { MouseAction::Down } else { MouseAction::Up },
                }
            }
            3 | 4 => { // RightMouseDown | RightMouseUp
                EventInfo::Mouse {
                    position,
                    button: MouseButton::Right,
                    action: if event_type.0 == 3 { MouseAction::Down } else { MouseAction::Up },
                }
            }
            5 => { // MouseMoved
                EventInfo::Mouse {
                    position,
                    button: MouseButton::Left,
                    action: MouseAction::Moved,
                }
            }
            // Scroll events
            22 => { // ScrollWheel
                let delta_x = CGEventGetDoubleValueField(event.0, CGEventField::ScrollWheelEventDeltaAxis2);
                let delta_y = CGEventGetDoubleValueField(event.0, CGEventField::ScrollWheelEventDeltaAxis1);
                
                EventInfo::Scroll {
                    position,
                    delta_x,
                    delta_y,
                }
            }
            _ => return event, // Ignore other events
        };
        
        // Call the callback with the event info
        if let Ok(callback) = tap.callback.lock() {
            callback(event_info);
        }
    }
    event
}

// FFI declarations for Core Graphics
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: CGEventTapLocation,
        place: CGEventTapPlacement,
        options: CGEventTapOptions,
        events_of_interest: u64,
        callback: extern "C" fn(CGEventTapProxy, CGEventType, CGEvent, *mut c_void) -> CGEvent,
        user_info: *mut c_void,
    ) -> *mut c_void;
    
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
    fn CGEventGetIntegerValueField(event: *mut c_void, field: CGEventField) -> i64;
    fn CGEventGetDoubleValueField(event: *mut c_void, field: CGEventField) -> c_double;
    fn CGEventGetFlags(event: *mut c_void) -> u64;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFMachPortCreateRunLoopSource(
        allocator: *mut c_void,
        port: *mut c_void,
        order: isize,
    ) -> *mut c_void;
    
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRelease(cf: *const c_void);
    
    static kCFRunLoopDefaultMode: *const c_void;
}

// CGEventField for accessing event properties
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CGEventField(i32);

impl CGEventField {
    pub const KeyboardEventKeycode: CGEventField = CGEventField(9);
    pub const ScrollWheelEventDeltaAxis1: CGEventField = CGEventField(11);
    pub const ScrollWheelEventDeltaAxis2: CGEventField = CGEventField(12);
}

// Event type constants
impl CGEventType {
    pub const LeftMouseDown: CGEventType = CGEventType(1);
    pub const LeftMouseUp: CGEventType = CGEventType(2);
    pub const RightMouseDown: CGEventType = CGEventType(3);
    pub const RightMouseUp: CGEventType = CGEventType(4);
    pub const MouseMoved: CGEventType = CGEventType(5);
    pub const LeftMouseDragged: CGEventType = CGEventType(6);
    pub const RightMouseDragged: CGEventType = CGEventType(7);
    pub const KeyDown: CGEventType = CGEventType(10);
    pub const KeyUp: CGEventType = CGEventType(11);
    pub const FlagsChanged: CGEventType = CGEventType(12);
    pub const ScrollWheel: CGEventType = CGEventType(22);
}

// Modifier key flags
impl CGEventFlags {
    pub const CMD_KEY: u64 = 0x100000;      // Command key
    pub const SHIFT_KEY: u64 = 0x20000;     // Shift key
    pub const OPTION_KEY: u64 = 0x80000;    // Option/Alt key
    pub const CONTROL_KEY: u64 = 0x40000;   // Control key
    pub const CAPS_LOCK: u64 = 0x10000;     // Caps Lock
}

#[repr(C)]
pub struct CGEventTapLocation(i32);
impl CGEventTapLocation {
    pub const HIDEventTap: CGEventTapLocation = CGEventTapLocation(0);
    pub const SessionEventTap: CGEventTapLocation = CGEventTapLocation(1);
    pub const CGAnnotatedSessionEventTap: CGEventTapLocation = CGEventTapLocation(2);
}

#[repr(C)]
pub struct CGEventTapPlacement(i32);
impl CGEventTapPlacement {
    pub const HeadInsertEventTap: CGEventTapPlacement = CGEventTapPlacement(0);
    pub const TailAppendEventTap: CGEventTapPlacement = CGEventTapPlacement(1);
}

#[repr(C)]
pub struct CGEventTapOptions(i32);
impl CGEventTapOptions {
    pub const DefaultTap: CGEventTapOptions = CGEventTapOptions(0);
    pub const ListenOnly: CGEventTapOptions = CGEventTapOptions(1);
}