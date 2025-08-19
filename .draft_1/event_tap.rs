use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType};
use core_graphics::event::{CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use std::os::raw::c_void;
use std::ptr;

pub type EventCallback = Box<dyn Fn(CGEventType, CGEvent) + Send + 'static>;

pub struct EventTap {
    tap: Option<*mut c_void>,
    callback: EventCallback,
}

impl EventTap {
    pub fn new(callback: EventCallback) -> Self {
        EventTap {
            tap: None,
            callback,
        }
    }

    pub fn start_monitoring(&mut self, track_mouse: bool, track_keyboard: bool, track_scroll: bool) -> Result<(), String> {
        unsafe {
            let mut event_mask: u64 = 0;
            
            if track_mouse {
                // Mouse events
                event_mask |= 1 << CGEventType::LeftMouseDown as u64;
                event_mask |= 1 << CGEventType::LeftMouseUp as u64;
                event_mask |= 1 << CGEventType::RightMouseDown as u64;
                event_mask |= 1 << CGEventType::RightMouseUp as u64;
                event_mask |= 1 << CGEventType::MouseMoved as u64;
                event_mask |= 1 << CGEventType::LeftMouseDragged as u64;
                event_mask |= 1 << CGEventType::RightMouseDragged as u64;
            }
            
            if track_keyboard {
                // Keyboard events
                event_mask |= 1 << CGEventType::KeyDown as u64;
                event_mask |= 1 << CGEventType::KeyUp as u64;
                event_mask |= 1 << CGEventType::FlagsChanged as u64;
            }
            
            if track_scroll {
                // Scroll events
                event_mask |= 1 << CGEventType::ScrollWheel as u64;
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
        let tap = &mut *(user_info as *mut EventTap);
        (tap.callback)(event_type, event.clone());
    }
    event
}

// FFI declarations for Core Graphics that might be missing
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

// Additional event types that might be missing
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