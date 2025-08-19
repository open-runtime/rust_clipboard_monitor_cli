//! Research Assistant Tracker Library
//!
//! This library provides a modular, extensible system for tracking
//! application focus and context on macOS.

#![cfg(target_os = "macos")]
#![deny(unsafe_op_in_unsafe_fn)]

mod frb_generated; /* AUTO INJECTED BY flutter_rust_bridge. This line may not be accurate, and you can change it according to your needs. */

pub mod core;
pub mod extractors;
// pub mod ffi_api;  // Temporarily disabled to avoid conflicts with new API
pub mod api;

pub use core::app_switcher_types::{AppInfo, AppSwitchEvent, AppSwitchListener, AppSwitcher};

// Re-export enhanced block variant
#[cfg(feature = "enhanced_block")]
pub use core::app_switcher_enhanced_block;

/// Re-export commonly used types
pub mod prelude {
    pub use crate::core::app_switcher_types::{
        AppInfo, AppSwitchEvent, AppSwitchListener, AppSwitchType, AppSwitcher,
    };
    pub use crate::api::*;
}

// Export Flutter Rust Bridge API
pub use api::*;
