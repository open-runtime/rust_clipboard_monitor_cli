#![allow(non_camel_case_types, non_upper_case_globals)]

use std::os::raw::c_void;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::{ptr, thread};

use objc2_app_kit::NSWorkspace;

type CFRunLoopRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFAllocatorRef = *const c_void;
type CFMachPortRef = *mut c_void;
type CFIndex = isize;
type CFStringRef = *const c_void;

type CGEventRef = *mut c_void;
type CGEventMask = u64;
type CGEventTapProxy = *mut c_void;
type CGEventType = u32;

#[repr(u32)]
#[derive(Copy, Clone)]
enum CGEventTapLocation {
    kCGHIDEventTap = 0,
    kCGSessionEventTap = 1,
    kCGAnnotatedSessionEventTap = 2,
}
#[repr(u32)]
#[derive(Copy, Clone)]
enum CGEventTapPlacement {
    kCGHeadInsertEventTap = 0,
    kCGTailAppendEventTap = 1,
}

bitflags::bitflags! {
    struct CGEventTapOptions: u32 {
        const kCGEventTapOptionDefault    = 0x00000000;
        const kCGEventTapOptionListenOnly = 0x00000001;
    }
}

const kCGEventScrollWheel: CGEventType = 22;
const fn event_mask(t: CGEventType) -> CGEventMask {
    1u64 << t
}

// CGEventField values
const kCGScrollWheelEventDeltaAxis1: u32 = 93;
const kCGScrollWheelEventDeltaAxis2: u32 = 94;
const kCGScrollWheelEventPointDeltaAxis1: u32 = 96;
const kCGScrollWheelEventPointDeltaAxis2: u32 = 97;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: CGEventTapLocation,
        place: CGEventTapPlacement,
        options: u32,
        eventsOfInterest: CGEventMask,
        callback: extern "C" fn(
            CGEventTapProxy,
            CGEventType,
            CGEventRef,
            *mut c_void,
        ) -> CGEventRef,
        userInfo: *mut c_void,
    ) -> CFMachPortRef;

    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopDefaultMode: CFStringRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFMachPortCreateRunLoopSource(
        allocator: CFAllocatorRef,
        port: CFMachPortRef,
        order: CFIndex,
    ) -> CFRunLoopSourceRef;
    fn CFRelease(cf: *const c_void);
}

#[derive(Debug, Clone)]
pub struct ScrollEvent {
    pub timestamp: Instant,
    pub vertical_px: i64,
    pub horizontal_px: i64,
    pub frontmost_pid: Option<i32>,
    pub frontmost_bundle_id: Option<String>,
    pub frontmost_name: Option<String>,
}

pub trait ScrollListener: Send + Sync {
    fn on_scroll(&mut self, event: &ScrollEvent);
}

struct ScrollState {
    listeners: Vec<Box<dyn ScrollListener>>,
    min_interval: Duration,
    last_emit: Instant,
}

static mut GLOBAL_STATE: Option<Arc<Mutex<ScrollState>>> = None;

pub struct ScrollTap;

impl ScrollTap {
    pub fn start(min_interval: Duration) -> Result<(), String> {
        unsafe {
            if GLOBAL_STATE.is_none() {
                GLOBAL_STATE = Some(Arc::new(Mutex::new(ScrollState {
                    listeners: Vec::new(),
                    min_interval,
                    last_emit: Instant::now(),
                })));
            }
        }

        thread::Builder::new()
            .name("scroll_tap".into())
            .spawn(|| unsafe {
                let mask = event_mask(kCGEventScrollWheel);
                let tap = CGEventTapCreate(
                    CGEventTapLocation::kCGSessionEventTap,
                    CGEventTapPlacement::kCGHeadInsertEventTap,
                    CGEventTapOptions::kCGEventTapOptionListenOnly.bits(),
                    mask,
                    tap_callback,
                    ptr::null_mut(),
                );
                if tap.is_null() {
                    eprintln!("[scroll_tap] CGEventTapCreate failed (permission?)");
                    return;
                }
                CGEventTapEnable(tap, true);
                let src = CFMachPortCreateRunLoopSource(ptr::null(), tap, 0);
                if src.is_null() {
                    eprintln!("[scroll_tap] CFMachPortCreateRunLoopSource failed");
                    CFRelease(tap as *const c_void);
                    return;
                }
                let rl = CFRunLoopGetCurrent();
                CFRunLoopAddSource(rl, src, kCFRunLoopDefaultMode);
                loop {
                    thread::park_timeout(Duration::from_secs(3600));
                }
            })
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    pub fn add_listener<T: ScrollListener + 'static>(&self, listener: T) {
        unsafe {
            if let Some(st) = &GLOBAL_STATE {
                if let Ok(mut s) = st.lock() {
                    s.listeners.push(Box::new(listener));
                }
            }
        }
    }
}

extern "C" fn tap_callback(
    _proxy: CGEventTapProxy,
    ty: CGEventType,
    event: CGEventRef,
    _user: *mut c_void,
) -> CGEventRef {
    if ty != kCGEventScrollWheel || event.is_null() {
        return event;
    }
    unsafe {
        let mut v = CGEventGetIntegerValueField(event, kCGScrollWheelEventPointDeltaAxis1) as i64;
        let mut h = CGEventGetIntegerValueField(event, kCGScrollWheelEventPointDeltaAxis2) as i64;
        if v == 0 {
            v = CGEventGetIntegerValueField(event, kCGScrollWheelEventDeltaAxis1) as i64;
        }
        if h == 0 {
            h = CGEventGetIntegerValueField(event, kCGScrollWheelEventDeltaAxis2) as i64;
        }

        if let Some(st) = &GLOBAL_STATE {
            if let Ok(mut s) = st.lock() {
                if s.last_emit.elapsed() >= s.min_interval {
                    s.last_emit = Instant::now();
                    let (pid, bundle_id, name) = frontmost_triplet();
                    let ev = ScrollEvent {
                        timestamp: Instant::now(),
                        vertical_px: v,
                        horizontal_px: h,
                        frontmost_pid: pid,
                        frontmost_bundle_id: bundle_id,
                        frontmost_name: name,
                    };
                    for l in s.listeners.iter_mut() {
                        l.on_scroll(&ev);
                    }
                }
            }
        }
    }
    event
}

fn frontmost_triplet() -> (Option<i32>, Option<String>, Option<String>) {
    unsafe {
        let ws = NSWorkspace::sharedWorkspace();
        if let Some(front) = ws.frontmostApplication() {
            let pid = Some(front.processIdentifier());
            let bid = front.bundleIdentifier().map(|s| s.to_string());
            let name = front.localizedName().map(|s| s.to_string());
            (pid, bid, name)
        } else {
            (None, None, None)
        }
    }
}
