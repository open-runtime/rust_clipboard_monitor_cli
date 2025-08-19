// src/tab_monitor.rs
//! Tab monitoring implementation using Accessibility API and AppleScript
//!
//! This module provides tab detection within applications using multiple approaches:
//! 1. Accessibility API for generic tab detection
//! 2. AppleScript/JXA for browser-specific information
//! 3. Keyboard event monitoring for tab shortcuts

use accessibility_sys::*;
use core_foundation::array::CFArray;
use core_foundation::base::{CFRelease, CFTypeRef};
use core_foundation::string::{CFString, CFStringRef};
use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use objc2_foundation::NSAutoreleasePool;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::c_void;
use std::process::Command;
use std::ptr::null_mut;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub title: String,
    pub url: Option<String>,
    pub index: usize,
    pub is_active: bool,
    pub window_id: Option<i32>,
    pub app_name: String,
    pub detected_via: TabDetectionMethod,
    pub timestamp: Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TabDetectionMethod {
    Accessibility,
    AppleScript,
    KeyboardShortcut,
    Extension,
}

#[derive(Debug)]
pub struct TabMonitor {
    current_app: String,
    current_pid: i32,
    tab_groups: Vec<usize>, // Store AXUIElementRef as usize
    current_tabs: Vec<TabInfo>,
    observers: HashMap<i32, usize>, // pid -> AXObserverRef
    last_update: Instant,
    cache_duration: Duration,
}

impl TabMonitor {
    pub fn new() -> Self {
        Self {
            current_app: String::new(),
            current_pid: 0,
            tab_groups: Vec::new(),
            current_tabs: Vec::new(),
            observers: HashMap::new(),
            last_update: Instant::now(),
            cache_duration: Duration::from_millis(500),
        }
    }

    /// Update the current application being monitored
    pub fn set_current_app(&mut self, app_name: String, pid: i32) {
        if self.current_app != app_name || self.current_pid != pid {
            self.current_app = app_name;
            self.current_pid = pid;

            // Clear old observers
            self.cleanup_observers();

            // Set up monitoring for new app
            self.setup_tab_monitoring(pid);
        }
    }

    /// Set up tab monitoring for a specific application
    fn setup_tab_monitoring(&mut self, pid: i32) {
        unsafe {
            // Create application element
            let app = AXUIElementCreateApplication(pid);

            // Find windows
            if let Some(windows) = self.get_windows(app) {
                for window in windows {
                    // Find tab groups in each window
                    self.find_and_monitor_tab_groups(window as AXUIElementRef, pid);
                }
            }

            CFRelease(app as CFTypeRef);
        }

        // For browsers, also set up AppleScript monitoring
        if self.is_browser(&self.current_app) {
            self.start_browser_monitoring();
        }
    }

    /// Find tab groups in a window and set up monitoring
    fn find_and_monitor_tab_groups(&mut self, window: AXUIElementRef, pid: i32) {
        unsafe {
            // Look for tab groups or radio groups (Safari uses radio groups)
            let roles_to_check = ["AXTabGroup", "AXRadioGroup", "AXToolbar"];

            for role_name in &roles_to_check {
                if let Some(elements) = self.find_elements_by_role(window, role_name) {
                    for element in elements {
                        // Check if this is actually a tab container
                        if self.is_tab_container(element as AXUIElementRef) {
                            self.tab_groups.push(element as usize);
                            self.setup_tab_observer(element as AXUIElementRef, pid);
                        }
                    }
                }
            }
        }
    }

    /// Check if an element is a tab container
    fn is_tab_container(&self, element: AXUIElementRef) -> bool {
        unsafe {
            // Check children for tab-like elements
            if let Some(children) = self.get_children(element) {
                for child in children {
                    let role = self.get_element_role(child as AXUIElementRef);
                    if role.contains("Tab") || role.contains("RadioButton") {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Set up observer for tab changes
    fn setup_tab_observer(&mut self, tab_group: AXUIElementRef, pid: i32) {
        unsafe {
            let mut observer: AXObserverRef = null_mut();

            if AXObserverCreate(pid, tab_observer_callback, &mut observer) == kAXErrorSuccess {
                // Monitor various tab-related notifications
                let notifications = [
                    "AXSelectedChildrenChanged", // Tab selection changed
                    "AXValueChanged",            // Tab state changed
                    "AXFocusedUIElementChanged", // Focus changed
                    "AXTitleChanged",            // Tab title changed
                    "AXUIElementDestroyed",      // Tab closed
                    "AXCreated",                 // New tab
                ];

                for notif in &notifications {
                    let cfstr = CFString::new(notif);
                    AXObserverAddNotification(
                        observer,
                        tab_group,
                        cfstr.as_concrete_TypeRef() as CFStringRef,
                        null_mut(),
                    );
                }

                // Add to run loop
                let source = AXObserverGetRunLoopSource(observer);
                CFRunLoopAddSource(
                    CFRunLoop::get_current().as_concrete_TypeRef(),
                    source,
                    kCFRunLoopDefaultMode as CFStringRef,
                );

                self.observers.insert(pid, observer as usize);
            }
        }
    }

    /// Get current tabs for the active application
    pub fn get_current_tabs(&mut self) -> Vec<TabInfo> {
        // Use cache if recent
        if self.last_update.elapsed() < self.cache_duration {
            return self.current_tabs.clone();
        }

        // Update based on app type
        let tabs = if self.is_browser(&self.current_app) {
            self.get_browser_tabs()
        } else if self.is_ide(&self.current_app) {
            self.get_ide_tabs()
        } else {
            self.get_generic_tabs()
        };

        self.current_tabs = tabs.clone();
        self.last_update = Instant::now();
        tabs
    }

    /// Get tabs from browsers using AppleScript
    fn get_browser_tabs(&self) -> Vec<TabInfo> {
        match self.current_app.as_str() {
            app if app.contains("Safari") => self.get_safari_tabs(),
            app if app.contains("Chrome") => self.get_chrome_tabs(),
            app if app.contains("Firefox") => self.get_firefox_tabs(),
            _ => self.get_generic_tabs(),
        }
    }

    /// Get Safari tabs using AppleScript
    fn get_safari_tabs(&self) -> Vec<TabInfo> {
        let script = r#"
            const Safari = Application('Safari');
            const tabs = [];
            
            if (Safari.windows.length > 0) {
                Safari.windows().forEach((window, wi) => {
                    try {
                        const currentTab = window.currentTab();
                        window.tabs().forEach((tab, ti) => {
                            tabs.push({
                                title: tab.name(),
                                url: tab.url(),
                                index: ti,
                                is_active: currentTab && tab.url() === currentTab.url(),
                                window_id: wi
                            });
                        });
                    } catch (e) {
                        // Window might be minimized or inaccessible
                    }
                });
            }
            
            JSON.stringify(tabs);
        "#;

        self.execute_jxa_script(script)
    }

    /// Get Chrome tabs using AppleScript
    fn get_chrome_tabs(&self) -> Vec<TabInfo> {
        let script = r#"
            const Chrome = Application('Google Chrome');
            const tabs = [];
            
            if (Chrome.windows.length > 0) {
                Chrome.windows().forEach((window, wi) => {
                    try {
                        const activeTab = window.activeTab();
                        window.tabs().forEach((tab, ti) => {
                            tabs.push({
                                title: tab.title(),
                                url: tab.url(),
                                index: ti,
                                is_active: activeTab && tab.id() === activeTab.id(),
                                window_id: wi
                            });
                        });
                    } catch (e) {
                        // Window might be minimized
                    }
                });
            }
            
            JSON.stringify(tabs);
        "#;

        self.execute_jxa_script(script)
    }

    /// Get Firefox tabs (limited support)
    fn get_firefox_tabs(&self) -> Vec<TabInfo> {
        // Firefox has limited AppleScript support
        // Fall back to accessibility API
        self.get_generic_tabs()
    }

    /// Execute JXA script and parse results
    fn execute_jxa_script(&self, script: &str) -> Vec<TabInfo> {
        match Command::new("osascript")
            .args(&["-l", "JavaScript", "-e", script])
            .output()
        {
            Ok(output) => {
                if output.status.success() {
                    let json_str = String::from_utf8_lossy(&output.stdout);
                    if let Ok(tabs) = serde_json::from_str::<Vec<serde_json::Value>>(&json_str) {
                        return tabs
                            .iter()
                            .map(|tab| TabInfo {
                                title: tab["title"].as_str().unwrap_or("").to_string(),
                                url: tab["url"].as_str().map(|s| s.to_string()),
                                index: tab["index"].as_u64().unwrap_or(0) as usize,
                                is_active: tab["is_active"].as_bool().unwrap_or(false),
                                window_id: tab["window_id"].as_i64().map(|i| i as i32),
                                app_name: self.current_app.clone(),
                                detected_via: TabDetectionMethod::AppleScript,
                                timestamp: Instant::now(),
                            })
                            .collect();
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to execute JXA script: {}", e);
            }
        }

        Vec::new()
    }

    /// Get IDE tabs using Accessibility API
    fn get_ide_tabs(&self) -> Vec<TabInfo> {
        let mut tabs = Vec::new();

        for tab_group_ptr in &self.tab_groups {
            let tab_group = *tab_group_ptr as AXUIElementRef;
            tabs.extend(self.extract_tabs_from_group(tab_group));
        }

        tabs
    }

    /// Get generic tabs using Accessibility API
    fn get_generic_tabs(&self) -> Vec<TabInfo> {
        self.get_ide_tabs() // Same approach works for most apps
    }

    /// Extract tab information from a tab group
    fn extract_tabs_from_group(&self, tab_group: AXUIElementRef) -> Vec<TabInfo> {
        let mut tabs = Vec::new();

        unsafe {
            if let Some(children) = self.get_children(tab_group) {
                // Get selected children
                let selected = self.get_selected_children(tab_group);

                for (index, child) in children.iter().enumerate() {
                    let element = *child as AXUIElementRef;
                    let title = self
                        .get_element_title(element)
                        .unwrap_or_else(|| format!("Tab {}", index + 1));

                    let is_active = selected.contains(child);

                    tabs.push(TabInfo {
                        title,
                        url: None, // Not available via accessibility
                        index,
                        is_active,
                        window_id: None,
                        app_name: self.current_app.clone(),
                        detected_via: TabDetectionMethod::Accessibility,
                        timestamp: Instant::now(),
                    });
                }
            }
        }

        tabs
    }

    // Helper methods for Accessibility API

    fn get_windows(&self, app: AXUIElementRef) -> Option<Vec<usize>> {
        self.get_attribute_array(app, "AXWindows")
    }

    fn get_children(&self, element: AXUIElementRef) -> Option<Vec<usize>> {
        self.get_attribute_array(element, "AXChildren")
    }

    fn get_selected_children(&self, element: AXUIElementRef) -> Vec<usize> {
        self.get_attribute_array(element, "AXSelectedChildren")
            .unwrap_or_default()
    }

    fn get_attribute_array(&self, element: AXUIElementRef, attribute: &str) -> Option<Vec<usize>> {
        unsafe {
            let attr_name = CFString::new(attribute);
            let mut value_ref: CFTypeRef = null_mut();

            if AXUIElementCopyAttributeValue(
                element,
                attr_name.as_concrete_TypeRef() as CFStringRef,
                &mut value_ref,
            ) == kAXErrorSuccess
            {
                let array = CFArray::wrap_under_get_rule(value_ref as CFArrayRef);
                let count = CFArrayGetCount(value_ref as CFArrayRef);

                let mut elements = Vec::new();
                for i in 0..count {
                    let element = CFArrayGetValueAtIndex(value_ref as CFArrayRef, i);
                    elements.push(element as usize);
                }

                Some(elements)
            } else {
                None
            }
        }
    }

    fn get_element_role(&self, element: AXUIElementRef) -> String {
        self.get_string_attribute(element, "AXRole")
            .unwrap_or_default()
    }

    fn get_element_title(&self, element: AXUIElementRef) -> Option<String> {
        self.get_string_attribute(element, "AXTitle")
            .or_else(|| self.get_string_attribute(element, "AXDescription"))
            .or_else(|| self.get_string_attribute(element, "AXValue"))
    }

    fn get_string_attribute(&self, element: AXUIElementRef, attribute: &str) -> Option<String> {
        unsafe {
            let attr_name = CFString::new(attribute);
            let mut value_ref: CFTypeRef = null_mut();

            if AXUIElementCopyAttributeValue(
                element,
                attr_name.as_concrete_TypeRef() as CFStringRef,
                &mut value_ref,
            ) == kAXErrorSuccess
            {
                let cf_str = CFString::wrap_under_get_rule(value_ref as CFStringRef);
                Some(cf_str.to_string())
            } else {
                None
            }
        }
    }

    fn find_elements_by_role(&self, parent: AXUIElementRef, role: &str) -> Option<Vec<usize>> {
        let mut found = Vec::new();

        if let Some(children) = self.get_children(parent) {
            for child in children {
                let element = child as AXUIElementRef;
                if self.get_element_role(element) == role {
                    found.push(child);
                }
                // Recurse into children
                if let Some(nested) = self.find_elements_by_role(element, role) {
                    found.extend(nested);
                }
            }
        }

        if found.is_empty() {
            None
        } else {
            Some(found)
        }
    }

    // Utility methods

    fn is_browser(&self, app_name: &str) -> bool {
        app_name.contains("Safari")
            || app_name.contains("Chrome")
            || app_name.contains("Firefox")
            || app_name.contains("Edge")
            || app_name.contains("Brave")
            || app_name.contains("Opera")
    }

    fn is_ide(&self, app_name: &str) -> bool {
        app_name.contains("Code")
            || app_name.contains("Xcode")
            || app_name.contains("IntelliJ")
            || app_name.contains("Sublime")
            || app_name.contains("Atom")
            || app_name.contains("TextMate")
    }

    fn start_browser_monitoring(&self) {
        // Additional browser-specific setup if needed
    }

    fn cleanup_observers(&mut self) {
        for (_, observer_ptr) in self.observers.drain() {
            unsafe {
                let observer = observer_ptr as AXObserverRef;
                let source = AXObserverGetRunLoopSource(observer);
                CFRunLoopRemoveSource(
                    CFRunLoop::get_current().as_concrete_TypeRef(),
                    source,
                    kCFRunLoopDefaultMode as CFStringRef,
                );
                CFRelease(observer as CFTypeRef);
            }
        }
        self.tab_groups.clear();
    }
}

// Callback for tab changes
extern "C" fn tab_observer_callback(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    _user_data: *mut c_void,
) {
    NSAutoreleasePool::with(|_pool| {
        unsafe {
            let notif = CFString::wrap_under_get_rule(notification).to_string();

            match notif.as_str() {
                "AXSelectedChildrenChanged" => {
                    eprintln!("Tab selection changed");
                    // Update tab state
                }
                "AXTitleChanged" => {
                    eprintln!("Tab title changed");
                }
                "AXCreated" => {
                    eprintln!("New tab created");
                }
                "AXUIElementDestroyed" => {
                    eprintln!("Tab closed");
                }
                _ => {}
            }
        }
    });
}

impl Drop for TabMonitor {
    fn drop(&mut self) {
        self.cleanup_observers();
    }
}

// Integration with your existing tracker
impl crate::Tracker {
    pub fn setup_tab_monitoring(&mut self) {
        self.tab_monitor = Some(TabMonitor::new());
    }

    pub fn update_tab_context(&mut self, app_name: String, pid: i32) {
        if let Some(tab_monitor) = &mut self.tab_monitor {
            tab_monitor.set_current_app(app_name, pid);

            // Get current tabs
            let tabs = tab_monitor.get_current_tabs();

            // Find active tab
            if let Some(active_tab) = tabs.iter().find(|t| t.is_active) {
                eprintln!(
                    "Active tab: {} {}",
                    active_tab.title,
                    active_tab.url.as_deref().unwrap_or("")
                );

                // Update context
                if let Some(ctx) = &mut self.current_context {
                    ctx.active_tab = Some(active_tab.title.clone());
                    ctx.tab_url = active_tab.url.clone();
                    ctx.total_tabs = tabs.len();
                }
            }
        }
    }
}
