// src/extractors/time_tracker.rs
//! Time tracking module for application usage analytics
//!
//! This module implements comprehensive time tracking for application usage,
//! providing insights into how time is spent across different applications.
//! Think of this as your personal productivity analytics engine.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::core::app_switcher_types::{AppInfo, AppSwitchEvent, AppSwitchListener, AppSwitchType};

/// Represents a single session of app usage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSession {
    pub app_name: String,
    pub bundle_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub duration: Duration,
    pub pid: i32,
}

/// Statistics for a specific application
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStatistics {
    pub app_name: String,
    pub app_path: String,
    pub bundle_id: String,
    pub total_time: Duration,
    pub session_count: usize,
    pub average_session_duration: Duration,
    pub longest_session: Duration,
    pub shortest_session: Duration,
    pub last_used: DateTime<Utc>,
    pub first_used: DateTime<Utc>,
}

/// Time tracker that maintains a complete history of app usage
pub struct TimeTracker {
    /// Current active app and when it became active
    current_session: Option<(AppInfo, Instant, DateTime<Utc>)>,

    /// Complete history of all app sessions
    session_history: Vec<AppSession>,

    /// Aggregated statistics per application
    app_statistics: HashMap<String, AppStatistics>,

    /// Total tracking start time
    tracking_started: Option<Instant>,

    /// Configuration
    config: TimeTrackerConfig,
}

/// Configuration for the time tracker
#[derive(Debug, Clone)]
pub struct TimeTrackerConfig {
    /// Minimum session duration to record (filters out brief switches)
    pub min_session_duration: Duration,

    /// Whether to print real-time updates
    pub print_updates: bool,

    /// Whether to track background time as well
    pub track_background: bool,

    /// Maximum history size (0 = unlimited)
    pub max_history_size: usize,
}

impl Default for TimeTrackerConfig {
    fn default() -> Self {
        Self {
            min_session_duration: Duration::from_secs(1),
            print_updates: true,
            track_background: false,
            max_history_size: 10000,
        }
    }
}

impl TimeTracker {
    /// Create a new time tracker with default configuration
    pub fn new() -> Self {
        Self::with_config(TimeTrackerConfig::default())
    }

    /// Create a new time tracker with custom configuration
    pub fn with_config(config: TimeTrackerConfig) -> Self {
        Self {
            current_session: None,
            session_history: Vec::new(),
            app_statistics: HashMap::new(),
            tracking_started: None,
            config,
        }
    }

    /// End the current session and record it
    fn end_current_session(&mut self, end_instant: Instant) {
        if let Some((app_info, start_instant, start_time)) = self.current_session.take() {
            let duration = end_instant.duration_since(start_instant);

            // Only record if duration meets minimum threshold
            if duration >= self.config.min_session_duration {
                let session = AppSession {
                    app_name: app_info.name.clone(),
                    bundle_id: app_info.bundle_id.clone(),
                    start_time,
                    end_time: Some(Utc::now()),
                    duration,
                    pid: app_info.pid,
                };

                // Update statistics (passing the app_info for path)
                self.update_statistics(&app_info, &session);

                // Add to history
                self.session_history.push(session.clone());

                // Trim history if needed
                if self.config.max_history_size > 0
                    && self.session_history.len() > self.config.max_history_size
                {
                    self.session_history.remove(0);
                }

                // Print update if configured
                if self.config.print_updates {
                    self.print_session_end(&app_info, duration);
                }
            }
        }
    }

    /// Update statistics for an application
    fn update_statistics(&mut self, app_info: &AppInfo, session: &AppSession) {
        let stats = self
            .app_statistics
            .entry(app_info.bundle_id.clone())
            .or_insert_with(|| AppStatistics {
                app_name: session.app_name.clone(),
                app_path: app_info.path.clone().unwrap_or_default(),
                bundle_id: app_info.bundle_id.clone(),
                total_time: Duration::from_secs(0),
                session_count: 0,
                average_session_duration: Duration::from_secs(0),
                longest_session: Duration::from_secs(0),
                shortest_session: Duration::from_secs(u64::MAX),
                last_used: session.start_time,
                first_used: session.start_time,
            });

        // Update statistics
        stats.total_time += session.duration;
        stats.session_count += 1;
        stats.average_session_duration = stats.total_time / stats.session_count as u32;
        stats.longest_session = stats.longest_session.max(session.duration);
        stats.shortest_session = stats.shortest_session.min(session.duration);
        stats.last_used = session.start_time;

        if session.start_time < stats.first_used {
            stats.first_used = session.start_time;
        }
    }

    /// Print session end information
    fn print_session_end(&self, app_info: &AppInfo, duration: Duration) {
        let minutes = duration.as_secs() / 60;
        let seconds = duration.as_secs() % 60;

        if minutes > 0 {
            println!("⏱️  {} - {}m {}s", app_info.name, minutes, seconds);
        } else {
            println!("⏱️  {} - {}s", app_info.name, seconds);
        }
    }

    /// Get statistics for all tracked applications
    pub fn get_all_statistics(&self) -> Vec<AppStatistics> {
        let mut stats: Vec<AppStatistics> = self.app_statistics.values().cloned().collect();
        stats.sort_by(|a, b| b.total_time.cmp(&a.total_time));
        stats
    }

    /// Get statistics for a specific application
    pub fn get_app_statistics(&self, bundle_id: &str) -> Option<&AppStatistics> {
        self.app_statistics.get(bundle_id)
    }

    /// Get the complete session history
    pub fn get_session_history(&self) -> &[AppSession] {
        &self.session_history
    }

    /// Get sessions for a specific application
    pub fn get_app_sessions(&self, bundle_id: &str) -> Vec<&AppSession> {
        self.session_history
            .iter()
            .filter(|s| s.bundle_id == bundle_id)
            .collect()
    }

    /// Get current session information
    pub fn get_current_session(&self) -> Option<(AppInfo, Duration)> {
        self.current_session
            .as_ref()
            .map(|(app, start, _)| (app.clone(), Instant::now().duration_since(*start)))
    }

    /// Generate a summary report
    pub fn generate_report(&self) -> TimeTrackingReport {
        let total_tracked_time = if let Some(start) = self.tracking_started {
            Instant::now().duration_since(start)
        } else {
            Duration::from_secs(0)
        };

        let total_active_time: Duration = self.app_statistics.values().map(|s| s.total_time).sum();

        TimeTrackingReport {
            tracking_duration: total_tracked_time,
            total_active_time,
            total_sessions: self.session_history.len(),
            unique_apps: self.app_statistics.len(),
            top_apps: self.get_top_apps(5),
            current_session: self.get_current_session(),
        }
    }

    /// Get top N applications by usage time
    pub fn get_top_apps(&self, n: usize) -> Vec<(String, Duration, f64)> {
        let mut apps: Vec<_> = self
            .app_statistics
            .values()
            .map(|s| {
                let total_time = self
                    .tracking_started
                    .map(|start| Instant::now().duration_since(start))
                    .unwrap_or(Duration::from_secs(1));

                let percentage = (s.total_time.as_secs_f64() / total_time.as_secs_f64()) * 100.0;
                (s.app_name.clone(), s.total_time, percentage)
            })
            .collect();

        apps.sort_by(|a, b| b.1.cmp(&a.1));
        apps.truncate(n);
        apps
    }

    /// Export session history to JSON
    pub fn export_to_json(&self) -> Result<String, serde_json::Error> {
        let export = TimeTrackingExport {
            metadata: ExportMetadata {
                export_time: Utc::now(),
                tracking_started: self.tracking_started.map(|_| {
                    Utc::now()
                        - chrono::Duration::from_std(
                            self.tracking_started
                                .map(|s| Instant::now().duration_since(s))
                                .unwrap_or_default(),
                        )
                        .unwrap_or_default()
                }),
                total_sessions: self.session_history.len(),
                unique_apps: self.app_statistics.len(),
            },
            sessions: self.session_history.clone(),
            statistics: self.get_all_statistics(),
        };

        serde_json::to_string_pretty(&export)
    }
}

impl AppSwitchListener for TimeTracker {
    fn on_app_switch(&mut self, event: &AppSwitchEvent) {
        let now = Instant::now();

        match event.event_type {
            AppSwitchType::Foreground => {
                // End previous session if exists
                self.end_current_session(now);

                // Start new session
                self.current_session = Some((event.app_info.clone(), now, Utc::now()));

                if self.config.print_updates {
                    println!("⏰ Started tracking: {}", event.app_info.name);
                }
            }
            AppSwitchType::Background => {
                // Only end session if it's the current app going to background
                if let Some((ref current_app, _, _)) = self.current_session {
                    if current_app.pid == event.app_info.pid {
                        self.end_current_session(now);
                    }
                }
            }
            AppSwitchType::Terminate => {
                // End session if this app was active
                if let Some((ref current_app, _, _)) = self.current_session {
                    if current_app.pid == event.app_info.pid {
                        self.end_current_session(now);
                    }
                }
            }
            _ => {}
        }
    }

    fn on_monitoring_started(&mut self) {
        self.tracking_started = Some(Instant::now());
        println!("⏰ Time tracking started");
    }

    fn on_monitoring_stopped(&mut self) {
        // End current session
        self.end_current_session(Instant::now());

        // Print summary
        println!("\n📊 Time Tracking Summary");
        println!("═══════════════════════════════════════");

        let report = self.generate_report();

        println!(
            "Total tracked time: {}m",
            report.tracking_duration.as_secs() / 60
        );
        println!(
            "Total active time: {}m",
            report.total_active_time.as_secs() / 60
        );
        println!("Total sessions: {}", report.total_sessions);
        println!("Unique applications: {}", report.unique_apps);

        if !report.top_apps.is_empty() {
            println!("\n🏆 Top Applications:");
            for (i, (name, duration, percentage)) in report.top_apps.iter().enumerate() {
                let minutes = duration.as_secs() / 60;
                let seconds = duration.as_secs() % 60;
                println!(
                    "   {}. {} - {}m {}s ({:.1}%)",
                    i + 1,
                    name,
                    minutes,
                    seconds,
                    percentage
                );
            }
        }

        println!("═══════════════════════════════════════");
    }
}

/// Report structure for time tracking summary
#[derive(Debug, Clone)]
pub struct TimeTrackingReport {
    pub tracking_duration: Duration,
    pub total_active_time: Duration,
    pub total_sessions: usize,
    pub unique_apps: usize,
    pub top_apps: Vec<(String, Duration, f64)>,
    pub current_session: Option<(AppInfo, Duration)>,
}

/// Export structure for JSON serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeTrackingExport {
    pub metadata: ExportMetadata,
    pub sessions: Vec<AppSession>,
    pub statistics: Vec<AppStatistics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportMetadata {
    pub export_time: DateTime<Utc>,
    pub tracking_started: Option<DateTime<Utc>>,
    pub total_sessions: usize,
    pub unique_apps: usize,
}
