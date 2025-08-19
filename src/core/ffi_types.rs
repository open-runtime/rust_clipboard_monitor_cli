// src/core/ffi_types.rs
//! Common FFI type definitions for Core Foundation and Accessibility APIs
//!
//! This module provides type aliases and conversions for working with
//! Core Foundation and Accessibility framework types that aren't fully
//! exposed by the objc2 crates.

use std::os::raw::c_void;

// Core Foundation raw pointer types
pub type CFStringRef = *const c_void;
pub type CFTypeRef = *mut c_void;
pub type CFDictionaryRef = *mut c_void;
pub type CFArrayRef = *mut c_void;
pub type CFBooleanRef = *const c_void;
pub type CFRunLoopSourceRef = *mut c_void;

// Accessibility raw pointer types
pub type AXUIElementRef = *mut c_void;
pub type AXObserverRef = *mut c_void;

// Constants for common CF values
pub const kCFBooleanTrue: CFBooleanRef = 0x1 as CFBooleanRef;
pub const kCFBooleanFalse: CFBooleanRef = 0x0 as CFBooleanRef;