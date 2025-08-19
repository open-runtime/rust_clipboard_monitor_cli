// src/core/app_switcher.rs
//! Common types and traits for app switching detection

use std::fmt;
use std::hash::Hash;
use std::time::Instant;

/// Information about an application
#[derive(Debug, Clone)]
pub struct AppInfo {
    pub name: String,
    pub bundle_id: String,
    pub pid: i32,
    pub path: Option<String>,
    pub launch_date: Option<Instant>,
    pub icon_base64: Option<String>,
    pub icon_path: Option<String>,
    pub activation_count: u32,
}

impl AppInfo {
    pub fn new(name: String, bundle_id: String, pid: i32) -> Self {
        Self {
            name,
            bundle_id,
            pid,
            path: None,
            launch_date: None,
            icon_base64: None,
            icon_path: None,
            activation_count: 0,
        }
    }
}

impl fmt::Display for AppInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}, pid: {})", self.name, self.bundle_id, self.pid)
    }
}

/// Type of app switch event
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AppSwitchType {
    /// App came to foreground
    Foreground,
    /// App went to background
    Background,
    /// App was launched
    Launch,
    /// App was terminated
    Terminate,
    /// App was hidden
    Hide,
    /// App was unhidden
    Unhide,
}

/// Workspace (CGWindow) summary data for convenience
#[derive(Debug, Clone)]
pub struct WorkspaceSummary {
    pub window_count: usize,
    pub focused_title: Option<String>,
    pub total_screen_coverage: Option<f64>,
    pub is_fullscreen: Option<bool>,
    pub is_minimized: Option<bool>,
    pub tab_titles: Vec<String>,
    pub active_file_paths: Vec<String>,
    pub primary_url: Option<String>,
}

/// Enhanced (NSWorkspace/process/desktop) summary data
#[derive(Debug, Clone)]
pub struct EnhancedSummary {
    pub activation_count: u32,
    pub front_window_title: Option<String>,
    pub cpu_usage: Option<f32>,
    pub memory_bytes: Option<u64>,
    pub session_active: Option<bool>,
    pub screen_locked: Option<bool>,
    // Display/space info
    pub display_count: Option<u32>,
    pub display_id: Option<u32>,
    pub space_id: Option<u32>,
    pub space_uuid: Option<String>,
    pub space_index: Option<u32>,
    pub space_type: Option<String>,
    pub space_name: Option<String>,
    pub space_label: Option<String>,
    // Browser/IDE context
    pub url: Option<String>,
    pub tab_title: Option<String>,
}

/// An app switch event
#[derive(Debug, Clone)]
pub struct AppSwitchEvent {
    pub timestamp: Instant,
    pub event_type: AppSwitchType,
    pub app_info: AppInfo,
    pub previous_app: Option<AppInfo>,
    /// Optional workspace (CGWindow) summary when available
    pub workspace: Option<WorkspaceSummary>,
    /// Optional enhanced (NSWorkspace/process/desktop) summary when available
    pub enhanced: Option<EnhancedSummary>,
    /// Optional confidence score when derived from multiple sources
    pub confidence: Option<f32>,
}

impl AppSwitchEvent {
    pub fn new(event_type: AppSwitchType, app_info: AppInfo) -> Self {
        Self {
            timestamp: Instant::now(),
            event_type,
            app_info,
            previous_app: None,
            workspace: None,
            enhanced: None,
            confidence: None,
        }
    }

    pub fn with_previous(event_type: AppSwitchType, app_info: AppInfo, previous: AppInfo) -> Self {
        Self {
            timestamp: Instant::now(),
            event_type,
            app_info,
            previous_app: Some(previous),
            workspace: None,
            enhanced: None,
            confidence: None,
        }
    }
}

/// Trait for app switch event listeners
pub trait AppSwitchListener: Send + Sync {
    /// Called when an app switch occurs
    fn on_app_switch(&mut self, event: &AppSwitchEvent);

    /// Called when monitoring starts
    fn on_monitoring_started(&mut self) {}

    /// Called when monitoring stops
    fn on_monitoring_stopped(&mut self) {}
}

/// Main app switcher trait that all implementations should follow
pub trait AppSwitcher {
    /// Add a listener for app switch events
    fn add_listener<T: AppSwitchListener + 'static>(&mut self, listener: T);

    /// Start monitoring for app switches
    fn start_monitoring(&mut self) -> Result<(), String>;

    /// Stop monitoring
    fn stop_monitoring(&mut self);

    /// Get current app if available
    fn current_app(&self) -> Option<AppInfo>;
}
