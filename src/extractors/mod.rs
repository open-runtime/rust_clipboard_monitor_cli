// src/extractors/mod.rs
pub mod time_tracker;

use crate::core::app_switcher_types::{AppSwitchEvent, AppSwitchListener};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use time_tracker::{TimeTracker, TimeTrackerConfig, AppSession, AppStatistics};

/// Enhanced context information extracted from applications
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppContext {
    pub basic_info: BasicAppInfo,
    pub enhanced_context: HashMap<String, ContextValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicAppInfo {
    pub name: String,
    pub bundle_id: String,
    pub pid: i32,
    pub path: Option<String>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContextValue {
    Text(String),
    Number(f64),
    Boolean(bool),
    List(Vec<String>),
}

/// Trait for extracting specific context from an application
pub trait ContextExtractor: Send + Sync {
    /// Extract context for a specific app type
    fn extract_context(
        &self,
        app_info: &crate::core::app_switcher_types::AppInfo,
    ) -> HashMap<String, ContextValue>;

    /// Check if this extractor applies to the given app
    fn applies_to(&self, bundle_id: &str) -> bool;

    /// Get a human-readable name for this extractor
    fn name(&self) -> &str;
}

/// Simple logging listener that just prints app switches
pub struct SimpleLogger {
    pub format: LogFormat,
}

#[derive(Debug, Clone)]
pub enum LogFormat {
    Json,
    Human,
}

impl AppSwitchListener for SimpleLogger {
    fn on_app_switch(&mut self, event: &AppSwitchEvent) {
        match self.format {
            LogFormat::Json => {
                let json_event = serde_json::json!({
                    "timestamp": event.timestamp.elapsed().as_millis(),
                    "event_type": format!("{:?}", event.event_type),
                    "app": {
                        "name": event.app_info.name,
                        "bundle_id": event.app_info.bundle_id,
                        "pid": event.app_info.pid,
                        "path": event.app_info.path
                    },
                    "previous_app": event.previous_app.as_ref().map(|app| {
                        serde_json::json!({
                            "name": app.name,
                            "bundle_id": app.bundle_id,
                            "pid": app.pid
                        })
                    })
                });
                println!("{}", serde_json::to_string_pretty(&json_event).unwrap());
            }
            LogFormat::Human => match event.event_type {
                crate::core::app_switcher_types::AppSwitchType::Foreground => {
                    println!(
                        "\n🔥 SWITCHED TO: {} ({})",
                        event.app_info.name, event.app_info.bundle_id
                    );
                    if let Some(prev) = &event.previous_app {
                        println!("   From: {}", prev.name);
                    }
                }
                crate::core::app_switcher_types::AppSwitchType::Background => {
                    println!("📱 {} went to background", event.app_info.name);
                }
                _ => {
                    println!("📋 {:?}: {}", event.event_type, event.app_info.name);
                }
            },
        }
    }
}

/// Enhanced listener that uses context extractors to get detailed information
pub struct ContextAwareListener {
    extractors: Vec<Box<dyn ContextExtractor>>,
    format: LogFormat,
}

impl ContextAwareListener {
    pub fn new(format: LogFormat) -> Self {
        Self {
            extractors: Vec::new(),
            format,
        }
    }

    /// Add a context extractor to enhance app switch events
    pub fn add_extractor<T: ContextExtractor + 'static>(&mut self, extractor: T) {
        self.extractors.push(Box::new(extractor));
    }

    /// Extract all available context for an app
    fn extract_all_context(
        &self,
        app_info: &crate::core::app_switcher_types::AppInfo,
    ) -> HashMap<String, ContextValue> {
        let mut context = HashMap::new();

        for extractor in &self.extractors {
            if extractor.applies_to(&app_info.bundle_id) {
                let extracted = extractor.extract_context(app_info);
                context.extend(extracted);
            }
        }

        context
    }
}

impl AppSwitchListener for ContextAwareListener {
    fn on_app_switch(&mut self, event: &AppSwitchEvent) {
        // Extract enhanced context
        let enhanced_context = self.extract_all_context(&event.app_info);

        let app_context = AppContext {
            basic_info: BasicAppInfo {
                name: event.app_info.name.clone(),
                bundle_id: event.app_info.bundle_id.clone(),
                pid: event.app_info.pid,
                path: event.app_info.path.clone(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            },
            enhanced_context,
        };

        match self.format {
            LogFormat::Json => {
                println!("{}", serde_json::to_string_pretty(&app_context).unwrap());
            }
            LogFormat::Human => {
                println!(
                    "\n🔥 ENHANCED SWITCH TO: {} ({})",
                    app_context.basic_info.name, app_context.basic_info.bundle_id
                );

                if !app_context.enhanced_context.is_empty() {
                    println!("   Enhanced Context:");
                    for (key, value) in &app_context.enhanced_context {
                        match value {
                            ContextValue::Text(text) => {
                                let preview = if text.len() > 100 {
                                    format!("{}...", &text[..100])
                                } else {
                                    text.clone()
                                };
                                println!("     {}: {}", key, preview);
                            }
                            ContextValue::List(items) => {
                                if items.len() <= 3 {
                                    println!("     {}: {}", key, items.join(", "));
                                } else {
                                    println!("     {}: {} items", key, items.len());
                                }
                            }
                            ContextValue::Boolean(b) => println!("     {}: {}", key, b),
                            ContextValue::Number(n) => println!("     {}: {}", key, n),
                        }
                    }
                }
            }
        }
    }
}

/// Example: Browser context extractor
pub struct BrowserContextExtractor;

impl ContextExtractor for BrowserContextExtractor {
    fn extract_context(
        &self,
        app_info: &crate::core::app_switcher_types::AppInfo,
    ) -> HashMap<String, ContextValue> {
        let mut context = HashMap::new();

        // In a real implementation, you'd use accessibility APIs here
        // For now, just demonstrate the concept
        if app_info.bundle_id.contains("chrome") || app_info.bundle_id.contains("Chrome") {
            context.insert(
                "browser_type".to_string(),
                ContextValue::Text("Chrome".to_string()),
            );
            // You would extract actual URL here using the accessibility code from original
            context.insert(
                "placeholder_url".to_string(),
                ContextValue::Text("https://example.com".to_string()),
            );
        } else if app_info.bundle_id.contains("Safari") {
            context.insert(
                "browser_type".to_string(),
                ContextValue::Text("Safari".to_string()),
            );
            // You would extract actual URL here
            context.insert(
                "placeholder_url".to_string(),
                ContextValue::Text("https://apple.com".to_string()),
            );
        }

        context
    }

    fn applies_to(&self, bundle_id: &str) -> bool {
        bundle_id.contains("chrome")
            || bundle_id.contains("Chrome")
            || bundle_id.contains("safari")
            || bundle_id.contains("Safari")
            || bundle_id.contains("firefox")
            || bundle_id.contains("edge")
    }

    fn name(&self) -> &str {
        "Browser Context"
    }
}

/// Example: IDE context extractor
pub struct IDEContextExtractor;

impl ContextExtractor for IDEContextExtractor {
    fn extract_context(
        &self,
        app_info: &crate::core::app_switcher_types::AppInfo,
    ) -> HashMap<String, ContextValue> {
        let mut context = HashMap::new();

        if app_info.name.contains("Code") || app_info.name.contains("Cursor") {
            context.insert(
                "ide_type".to_string(),
                ContextValue::Text("VS Code Family".to_string()),
            );
            // You would extract actual file paths here using accessibility APIs
            context.insert(
                "placeholder_project".to_string(),
                ContextValue::Text("my-awesome-project".to_string()),
            );
            context.insert(
                "placeholder_files".to_string(),
                ContextValue::List(vec!["src/main.rs".to_string(), "Cargo.toml".to_string()]),
            );
        }

        context
    }

    fn applies_to(&self, bundle_id: &str) -> bool {
        bundle_id.contains("code")
            || bundle_id.contains("cursor")
            || bundle_id.contains("jetbrains")
    }

    fn name(&self) -> &str {
        "IDE Context"
    }
}
