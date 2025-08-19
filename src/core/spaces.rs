// src/core/spaces.rs
//! Best-effort Mission Control Spaces snapshot using the private SkyLight framework.
//! We dlopen at runtime to avoid link-time failures on systems where symbols differ.
//! Returns lightweight identifiers and labels for the current Space per display.

use std::ffi::{c_void, CString};
use std::mem::transmute;

use core_foundation::array::CFArray;
use core_foundation::base::{FromVoid, TCFType, ToVoid};
// use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;

type CFArrayRef = *const c_void;
// type CFDictionaryRef = *const c_void;
type CFTypeRef = *const c_void;

/// Information about the current space on a display
#[derive(Debug, Clone)]
pub struct DisplaySpaceInfo {
    pub display_uuid: String,
    pub current_space_uuid: Option<String>,
    pub current_space_index: Option<u32>,
    pub current_space_type: Option<String>,
    pub current_space_name: Option<String>,
}

/// Snapshot of all displays/spaces
#[derive(Debug, Clone)]
pub struct SpacesSnapshot {
    pub displays: Vec<DisplaySpaceInfo>,
    pub visible_space_uuids: Vec<String>,
    pub active_space_uuid: Option<String>,
}

impl SpacesSnapshot {
    /// Compute a human-friendly label for a display's current space.
    /// If `name` is present, use it. Otherwise, derive from type/index.
    pub fn label_for_display(&self, display_idx: usize) -> Option<String> {
        let info = self.displays.get(display_idx)?;
        if let Some(name) = &info.current_space_name {
            if !name.is_empty() {
                return Some(name.clone());
            }
        }
        let ty = info
            .current_space_type
            .clone()
            .unwrap_or_else(|| "user".into());
        let idx = info.current_space_index.unwrap_or(0);
        match ty.as_str() {
            "fullscreen" => Some("Fullscreen".to_string()),
            "system" => Some("System".to_string()),
            _ => {
                if idx > 0 {
                    Some(format!("Desktop {}", idx))
                } else {
                    Some("Desktop".to_string())
                }
            }
        }
    }
}

// Runtime-loaded SkyLight functions
type CopyManagedDisplaySpacesFn = unsafe extern "C" fn(i32) -> CFArrayRef;
type MainConnectionIDFn = unsafe extern "C" fn() -> i32;

struct SkyLightFns {
    copy_managed_display_spaces: CopyManagedDisplaySpacesFn,
    main_connection_id: MainConnectionIDFn,
}

fn load_skylight() -> Option<SkyLightFns> {
    // SAFETY: We dlopen a private framework. On failure, return None.
    unsafe {
        let path =
            CString::new("/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight").unwrap();
        let handle = libc::dlopen(path.as_ptr(), libc::RTLD_LAZY);
        if handle.is_null() {
            return None;
        }

        // Try both CGS and SLS symbol prefixes for robustness across macOS versions
        let sym_copy = [
            CString::new("CGSCopyManagedDisplaySpaces").unwrap(),
            CString::new("SLSCopyManagedDisplaySpaces").unwrap(),
        ]
        .into_iter()
        .find_map(|name| {
            let p = libc::dlsym(handle, name.as_ptr());
            if p.is_null() {
                None
            } else {
                Some(p)
            }
        })?;

        let sym_conn = [
            CString::new("CGSMainConnectionID").unwrap(),
            CString::new("SLSMainConnectionID").unwrap(),
        ]
        .into_iter()
        .find_map(|name| {
            let p = libc::dlsym(handle, name.as_ptr());
            if p.is_null() {
                None
            } else {
                Some(p)
            }
        })?;

        let copy_managed_display_spaces: CopyManagedDisplaySpacesFn = transmute(sym_copy);
        let main_connection_id: MainConnectionIDFn = transmute(sym_conn);

        Some(SkyLightFns {
            copy_managed_display_spaces,
            main_connection_id,
        })
    }
}

/// Query Mission Control spaces. Returns None if SkyLight is unavailable.
pub fn query_spaces() -> Option<SpacesSnapshot> {
    let fns = load_skylight()?;
    unsafe {
        let conn = (fns.main_connection_id)();
        let arr = (fns.copy_managed_display_spaces)(conn);
        if arr.is_null() {
            return None;
        }

        let displays_cf: CFArray<CFDictionary> = CFArray::wrap_under_create_rule(arr as *const _);
        let mut displays_out: Vec<DisplaySpaceInfo> = Vec::new();
        let mut visible: Vec<String> = Vec::new();
        let mut active: Option<String> = None;

        for i in 0..displays_cf.len() {
            if let Some(display_dict) = displays_cf.get(i) {
                // Display UUID (string)
                let display_uuid = display_dict
                    .find(CFString::from("Display Identifier").to_void())
                    .and_then(|s| Some(unsafe { CFString::from_void(*s) }.to_string()))
                    .unwrap_or_else(|| "unknown-display".to_string());

                // Current Space dictionary
                let current_space_dict = display_dict
                    .find(CFString::from("Current Space").to_void())
                    .map(|d| unsafe { CFDictionary::<CFString, CFTypeRef>::from_void(*d) });

                // All spaces for this display
                let spaces_array = display_dict
                    .find(CFString::from("Spaces").to_void())
                    .map(|a| unsafe { CFArray::<CFDictionary>::from_void(*a) });

                let (mut current_space_uuid, mut current_space_type, mut current_space_name) =
                    (None, None, None);
                let mut current_space_index: Option<u32> = None;

                if let Some(cs) = &current_space_dict {
                    // uuid
                    current_space_uuid = cs
                        .find(&CFString::from("uuid"))
                        .map(|s| unsafe { CFString::from_void(*s) }.to_string());

                    // name (if any)
                    current_space_name = cs
                        .find(&CFString::from("name"))
                        .or_else(|| cs.find(&CFString::from("Name")))
                        .map(|s| unsafe { CFString::from_void(*s) }.to_string());

                    // type (number or string, normalize)
                    if let Some(tref) = cs.find(&CFString::from("type")) {
                        let ty_num = unsafe { CFNumber::from_void(*tref) }.to_i64();
                        current_space_type = ty_num.map(|n| match n {
                            4 => "fullscreen".to_string(),
                            5 => "system".to_string(),
                            _ => "user".to_string(),
                        });
                        if current_space_type.is_none() {
                            current_space_type =
                                Some(unsafe { CFString::from_void(*tref) }.to_string());
                        }
                    }
                }

                // Derive index by matching uuid within Spaces array
                if let (Some(uuid), Some(spaces)) = (&current_space_uuid, spaces_array) {
                    for j in 0..spaces.len() {
                        if let Some(sp_dict) = spaces.get(j) {
                            if let Some(u) = sp_dict
                                .find(CFString::from("uuid").to_void())
                                .map(|s| unsafe { CFString::from_void(*s) }.to_string())
                            {
                                if &u == uuid {
                                    current_space_index = Some((j as u32) + 1);
                                }
                                // Collect all visible space uuids (heuristic: treat all as visible for this display)
                                visible.push(u);
                            }
                        }
                    }
                }

                if active.is_none() {
                    active = current_space_uuid.clone();
                }

                displays_out.push(DisplaySpaceInfo {
                    display_uuid,
                    current_space_uuid,
                    current_space_index,
                    current_space_type,
                    current_space_name,
                });
            }
        }

        Some(SpacesSnapshot {
            displays: displays_out,
            visible_space_uuids: visible,
            active_space_uuid: active,
        })
    }
}
