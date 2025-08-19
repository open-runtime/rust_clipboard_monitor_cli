#![allow(non_camel_case_types, non_upper_case_globals)]

use std::ffi::CStr;
use std::os::raw::{c_char, c_void};

type CFIndex = isize;
type CFStringRef = *const c_void;
type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFTypeRef = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(theArray: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(theArray: CFArrayRef, idx: CFIndex) -> *const c_void;
    fn CFDictionaryGetValue(theDict: CFDictionaryRef, key: CFTypeRef) -> *const c_void;
    fn CFStringGetCStringPtr(theString: CFStringRef, encoding: u32) -> *const c_char;
    fn CFStringGetCString(
        theString: CFStringRef,
        buffer: *mut c_char,
        bufferSize: CFIndex,
        encoding: u32,
    ) -> bool;
    fn CFNumberGetValue(cf: CFTypeRef, theType: i32, valuePtr: *mut i32) -> bool;
    fn CFRelease(cf: CFTypeRef);
}

// kCFStringEncodingUTF8
const kCFStringEncodingUTF8: u32 = 0x08000100;
// kCFNumberSInt32Type
const kCFNumberSInt32Type: i32 = 3;

type CGWindowID = u32;
type CGWindowListOption = u32;

const kCGWindowListOptionOnScreenOnly: CGWindowListOption = 1;
const kCGWindowListExcludeDesktopElements: CGWindowListOption = 1 << 4;
const kCGNullWindowID: CGWindowID = 0;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(
        options: CGWindowListOption,
        relativeToWindow: CGWindowID,
    ) -> CFArrayRef;

    // Exported CFStringRef keys from CGWindow.h
    static kCGWindowOwnerPID: CFStringRef;
    static kCGWindowOwnerName: CFStringRef;
    static kCGWindowLayer: CFStringRef;
    static kCGWindowName: CFStringRef;
}

#[derive(Debug, Clone)]
pub struct FrontWindowInfo {
    pub owner_pid: i32,
    pub owner_name: String,
    pub window_title: Option<String>,
    pub layer: i32,
}

/// Returns the topmost on-screen, layer-0 window's owner (PID/name) and title (if available).
pub fn front_window_info() -> Option<FrontWindowInfo> {
    unsafe {
        let options = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
        let arr = CGWindowListCopyWindowInfo(options, kCGNullWindowID);
        if arr.is_null() {
            return None;
        }
        let count = CFArrayGetCount(arr);
        let mut result: Option<FrontWindowInfo> = None;

        for i in 0..count {
            let dict_ptr = CFArrayGetValueAtIndex(arr, i) as CFDictionaryRef;
            if dict_ptr.is_null() {
                continue;
            }

            // layer
            let layer_val = CFDictionaryGetValue(dict_ptr, kCGWindowLayer as CFTypeRef);
            let mut layer_i32 = 0;
            if !layer_val.is_null() {
                let _ =
                    CFNumberGetValue(layer_val as CFTypeRef, kCFNumberSInt32Type, &mut layer_i32);
            }
            if layer_i32 != 0 {
                continue;
            }

            // owner pid
            let pid_val = CFDictionaryGetValue(dict_ptr, kCGWindowOwnerPID as CFTypeRef);
            let mut pid_i32 = 0;
            if pid_val.is_null()
                || !CFNumberGetValue(pid_val as CFTypeRef, kCFNumberSInt32Type, &mut pid_i32)
            {
                continue;
            }

            // owner name
            let name_val =
                CFDictionaryGetValue(dict_ptr, kCGWindowOwnerName as CFTypeRef) as CFStringRef;
            let owner_name = cfstring_to_string(name_val).unwrap_or_else(|| "".to_string());

            // window title (may require screen recording)
            let title_val =
                CFDictionaryGetValue(dict_ptr, kCGWindowName as CFTypeRef) as CFStringRef;
            let window_title = cfstring_to_string(title_val);

            result = Some(FrontWindowInfo {
                owner_pid: pid_i32,
                owner_name,
                window_title,
                layer: layer_i32,
            });
            break;
        }

        CFRelease(arr as CFTypeRef);
        result
    }
}

unsafe fn cfstring_to_string(s: CFStringRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let cptr = CFStringGetCStringPtr(s, kCFStringEncodingUTF8);
    if !cptr.is_null() {
        let cstr = CStr::from_ptr(cptr);
        return cstr.to_str().ok().map(|s| s.to_string());
    }
    let mut buf = [0i8; 4096];
    if CFStringGetCString(
        s,
        buf.as_mut_ptr(),
        buf.len() as CFIndex,
        kCFStringEncodingUTF8,
    ) {
        let cstr = CStr::from_ptr(buf.as_ptr());
        return cstr.to_str().ok().map(|s| s.to_string());
    }
    None
}
