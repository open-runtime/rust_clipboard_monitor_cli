// src/core/app_switcher.rs
//! Unified app switching facade that re-exports common types and
//! provides a simple, high-level switcher used by `main.rs`.

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use objc2::MainThreadMarker;

use crate::core::accessibility::ax_focused_window_title_quick;
use crate::core::app_switcher_enhanced::{
    EnhancedAppSwitchEvent, EnhancedAppSwitchListener, EnhancedAppSwitcher,
};
use crate::core::app_switcher_workspace::{
    WorkspaceAppMonitor, WorkspaceAppSwitchEvent, WorkspaceAppSwitchListener,
};

pub use crate::core::app_switcher_types::{
    AppInfo, AppSwitchEvent, AppSwitchListener, AppSwitchType, EnhancedSummary, WorkspaceSummary,
};

/// Initialize any global state needed before creating a switcher.
/// Currently a no-op, reserved for future expansion.
pub fn initialize_app_switcher(_mtm: MainThreadMarker) -> Result<(), String> {
    Ok(())
}

struct FusionHub {
    listeners: Arc<Mutex<Vec<Box<dyn AppSwitchListener>>>>,
    pending: Arc<Mutex<HashMap<(i32, AppSwitchType), (AppSwitchEvent, Instant)>>>,
    fuse_window: Duration,
}

impl FusionHub {
    fn new(listeners: Arc<Mutex<Vec<Box<dyn AppSwitchListener>>>>) -> Arc<Self> {
        Arc::new(Self {
            listeners,
            pending: Arc::new(Mutex::new(HashMap::new())),
            fuse_window: Duration::from_millis(300),
        })
    }

    fn emit_or_merge(self: &Arc<Self>, mut incoming: AppSwitchEvent) {
        let key_pid = incoming.app_info.pid;
        let key_kind = incoming.event_type.clone();
        let now = Instant::now();
        let mut pending = self.pending.lock().unwrap();
        if let Some((existing, _ts)) = pending.remove(&(key_pid, key_kind.clone())) {
            // Merge summaries and confidence
            incoming.workspace = incoming.workspace.or(existing.workspace);
            incoming.enhanced = incoming.enhanced.or(existing.enhanced);
            incoming.confidence = match (incoming.confidence, existing.confidence) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
            drop(pending);
            self.dispatch(incoming);
        } else {
            // Insert and schedule flush
            pending.insert((key_pid, key_kind.clone()), (incoming, now));
            drop(pending);
            let hub = Arc::clone(self);
            std::thread::spawn(move || {
                std::thread::sleep(hub.fuse_window);
                let mut pending = hub.pending.lock().unwrap();
                if let Some((evt, ts)) = pending.remove(&(key_pid, key_kind.clone())) {
                    if ts.elapsed() >= hub.fuse_window {
                        drop(pending);
                        hub.dispatch(evt);
                    } else {
                        // put back if not elapsed (unlikely due to sleep)
                        pending.insert((key_pid, key_kind), (evt, ts));
                    }
                }
            });
        }
    }

    fn dispatch(&self, event: AppSwitchEvent) {
        // Build a richer title for Human/Research by fusing from multiple sources
        let mut fused = event;
        if fused
            .workspace
            .as_ref()
            .and_then(|w| w.focused_title.clone())
            .is_none()
        {
            if let Some(enh) = &fused.enhanced {
                let merged = enh
                    .front_window_title
                    .clone()
                    .or_else(|| enh.tab_title.clone());
                if let Some(title) = merged {
                    // Patch into workspace summary so downstream loggers see it
                    if let Some(ws) = &mut fused.workspace {
                        if ws.focused_title.is_none() {
                            ws.focused_title = Some(title);
                        }
                    }
                }
                // Final fallback: AX focused window title (covers Electron apps like Cursor)
                if fused
                    .workspace
                    .as_ref()
                    .and_then(|w| w.focused_title.clone())
                    .is_none()
                {
                    if let Some(ax_title) = ax_focused_window_title_quick(fused.app_info.pid) {
                        if !ax_title.is_empty() {
                            if let Some(ws) = &mut fused.workspace {
                                ws.focused_title = Some(ax_title);
                            } else {
                                fused.workspace = Some(WorkspaceSummary {
                                    window_count: 0,
                                    focused_title: Some(ax_title),
                                    total_screen_coverage: None,
                                    is_fullscreen: None,
                                    is_minimized: None,
                                    tab_titles: Vec::new(),
                                    active_file_paths: Vec::new(),
                                    primary_url: None,
                                });
                            }
                        }
                    }
                }
            }
        }
        for l in &mut *self.listeners.lock().unwrap() {
            l.on_app_switch(&fused);
        }
    }
}

/// High-level App Switcher used by the application
pub struct AppSwitcher {
    workspace: WorkspaceAppMonitor,
    enhanced: Option<EnhancedAppSwitcher>,
    listeners: Arc<Mutex<Vec<Box<dyn AppSwitchListener>>>>,
    hub: Arc<FusionHub>,
}

impl AppSwitcher {
    pub fn new() -> Self {
        let listeners = Arc::new(Mutex::new(Vec::new()));
        let hub = FusionHub::new(listeners.clone());
        Self {
            workspace: WorkspaceAppMonitor::new(),
            enhanced: Some(EnhancedAppSwitcher::new()),
            listeners,
            hub,
        }
    }

    pub fn add_listener<T: AppSwitchListener + 'static>(&mut self, listener: T) {
        self.listeners.lock().unwrap().push(Box::new(listener));
    }

    pub fn start_monitoring(&mut self, mtm: MainThreadMarker) -> Result<(), String> {
        // Register workspace adapter
        let adapter = WorkspaceAdapter {
            hub: Arc::clone(&self.hub),
        };
        self.workspace.add_workspace_listener(adapter);
        self.workspace.start_monitoring(mtm)?;

        // Register enhanced adapter (best-effort)
        if let Some(enh) = &mut self.enhanced {
            let adapter = EnhancedAdapter {
                hub: Arc::clone(&self.hub),
            };
            enh.add_listener(adapter);
            let _ = enh.start_monitoring(mtm);
        }

        Ok(())
    }

    /// Trigger a best-effort resample of the current foreground app and window context
    pub fn resample_now(&self) {
        self.workspace.resample_now();
        if let Some(enh) = &self.enhanced {
            enh.resample_now();
        }
    }

    pub fn stop_monitoring(&mut self) {
        self.workspace.stop_monitoring();
        if let Some(enh) = &mut self.enhanced {
            enh.stop_monitoring();
        }
    }

    pub fn current_app(&self) -> Option<AppInfo> {
        if let Some(info) = self.workspace.current_app() {
            return Some(info.basic_info);
        }
        if let Some(enhanced) = &self.enhanced {
            return enhanced.current_app().map(|ext| AppInfo {
                name: ext.name,
                bundle_id: ext.bundle_id,
                pid: ext.pid,
                path: ext.path,
                launch_date: ext.launch_date,
                icon_base64: ext.icon_base64,
                icon_path: ext.icon_path,
                activation_count: ext.activation_count,
            });
        }
        None
    }
}

struct WorkspaceAdapter {
    hub: Arc<FusionHub>,
}

impl WorkspaceAdapter {
    fn to_basic_event(evt: &WorkspaceAppSwitchEvent) -> AppSwitchEvent {
        let app = evt.app_info.basic_info.clone();
        let prev = evt.previous_app.as_ref().map(|p| p.basic_info.clone());
        let workspace = WorkspaceSummary {
            window_count: evt.app_info.windows.len(),
            focused_title: evt
                .app_info
                .focused_window
                .as_ref()
                .and_then(|w| w.title.clone()),
            total_screen_coverage: Some(evt.app_info.total_screen_coverage),
            is_fullscreen: Some(evt.app_info.is_fullscreen),
            is_minimized: Some(evt.app_info.is_minimized),
            tab_titles: evt
                .app_info
                .browser_tabs
                .iter()
                .map(|t| t.title.clone())
                .collect(),
            active_file_paths: evt.app_info.active_file_paths.clone(),
            primary_url: evt
                .app_info
                .windows
                .iter()
                .filter_map(|w| w.detected_url.clone())
                .next(),
        };
        AppSwitchEvent {
            timestamp: evt.timestamp,
            event_type: evt.event_type.clone(),
            app_info: app,
            previous_app: prev,
            workspace: Some(workspace),
            enhanced: None,
            confidence: Some(evt.confidence_score),
        }
    }
}

impl WorkspaceAppSwitchListener for WorkspaceAdapter {
    fn on_workspace_app_switch(&mut self, event: &WorkspaceAppSwitchEvent) {
        let basic = Self::to_basic_event(event);
        self.hub.emit_or_merge(basic);
    }
}

struct EnhancedAdapter {
    hub: Arc<FusionHub>,
}

impl EnhancedAdapter {
    fn to_basic_event(evt: &EnhancedAppSwitchEvent) -> AppSwitchEvent {
        let app = AppInfo {
            name: evt.app_info.name.clone(),
            bundle_id: evt.app_info.bundle_id.clone(),
            pid: evt.app_info.pid,
            path: evt.app_info.path.clone(),
            launch_date: evt.app_info.launch_date,
            icon_base64: evt.app_info.icon_base64.clone(),
            icon_path: evt.app_info.icon_path.clone(),
            activation_count: evt.app_info.activation_count,
        };
        let prev = evt.previous_app.as_ref().map(|p| AppInfo {
            name: p.name.clone(),
            bundle_id: p.bundle_id.clone(),
            pid: p.pid,
            path: p.path.clone(),
            launch_date: p.launch_date,
            icon_base64: p.icon_base64.clone(),
            icon_path: p.icon_path.clone(),
            activation_count: p.activation_count,
        });
        let kind = match evt.event_type {
            crate::core::app_switcher_enhanced::AppSwitchType::Foreground => {
                AppSwitchType::Foreground
            }
            crate::core::app_switcher_enhanced::AppSwitchType::Background => {
                AppSwitchType::Background
            }
            crate::core::app_switcher_enhanced::AppSwitchType::Launch => AppSwitchType::Launch,
            crate::core::app_switcher_enhanced::AppSwitchType::Terminate => {
                AppSwitchType::Terminate
            }
            crate::core::app_switcher_enhanced::AppSwitchType::Hide => AppSwitchType::Hide,
            crate::core::app_switcher_enhanced::AppSwitchType::Unhide => AppSwitchType::Unhide,
            _ => AppSwitchType::Foreground,
        };
        // Best-effort enrichment for browsers via AppleScript (non-AX)
        let browser_url = best_effort_browser_url(&evt.app_info.bundle_id);
        let browser_title = if browser_url.is_some() {
            best_effort_browser_title(&evt.app_info.bundle_id)
        } else {
            None
        };

        let enhanced = EnhancedSummary {
            activation_count: evt.app_info.activation_count,
            front_window_title: browser_title.clone().or_else(|| {
                evt.app_info
                    .frontmost_window
                    .as_ref()
                    .and_then(|w| w.title.clone())
            }),
            cpu_usage: evt.app_info.process_info.as_ref().map(|p| p.cpu_usage),
            memory_bytes: evt.app_info.process_info.as_ref().map(|p| p.memory_bytes),
            session_active: Some(evt.desktop_state.session_active),
            screen_locked: Some(evt.desktop_state.screen_locked),
            display_count: Some(evt.desktop_state.display_count),
            display_id: evt
                .app_info
                .frontmost_window
                .as_ref()
                .and_then(|_| evt.app_info.front_window_display_id),
            space_id: evt.desktop_state.active_space_id,
            space_uuid: evt.desktop_state.active_space_uuid.clone(),
            space_index: evt.desktop_state.active_space_index,
            space_type: evt.desktop_state.active_space_type.clone(),
            space_name: evt.desktop_state.active_space_name.clone(),
            space_label: evt.desktop_state.active_space_label.clone(),
            url: browser_url,
            tab_title: browser_title.or_else(|| {
                evt.app_info
                    .frontmost_window
                    .as_ref()
                    .and_then(|w| w.title.clone())
            }),
        };
        AppSwitchEvent {
            timestamp: evt.timestamp,
            event_type: kind,
            app_info: app,
            previous_app: prev,
            workspace: None,
            enhanced: Some(enhanced),
            confidence: Some(evt.confidence_score),
        }
    }
}

impl EnhancedAppSwitchListener for EnhancedAdapter {
    fn on_app_switch(&mut self, event: &EnhancedAppSwitchEvent) {
        let basic = Self::to_basic_event(event);
        self.hub.emit_or_merge(basic);
    }
}

// --- Local helpers ----------------------------------------------------------

fn best_effort_browser_url(bundle_id: &str) -> Option<String> {
    let script = if bundle_id.contains("com.google.Chrome") {
        Some(r#"tell application "Google Chrome" to get URL of active tab of front window"#)
    } else if bundle_id.contains("com.apple.SafariTechnologyPreview") {
        Some(r#"tell application "Safari Technology Preview" to get URL of front document"#)
    } else if bundle_id.contains("com.apple.Safari") {
        Some(r#"tell application "Safari" to get URL of front document"#)
    } else {
        None
    }?;
    if let Ok(out) = Command::new("osascript").arg("-e").arg(script).output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

fn best_effort_browser_title(bundle_id: &str) -> Option<String> {
    let script = if bundle_id.contains("com.google.Chrome") {
        Some(r#"tell application "Google Chrome" to get title of active tab of front window"#)
    } else if bundle_id.contains("com.apple.SafariTechnologyPreview") {
        Some(r#"tell application "Safari Technology Preview" to get name of front document"#)
    } else if bundle_id.contains("com.apple.Safari") {
        Some(r#"tell application "Safari" to get name of front document"#)
    } else {
        None
    }?;
    if let Ok(out) = Command::new("osascript").arg("-e").arg(script).output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}
