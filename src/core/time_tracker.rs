// src/core/time_tracker.rs
//! Time tracking and analytics for application usage
//!
//! This module provides:
//! - Precise time tracking for app focus duration
//! - Session management and idle detection
//! - Usage analytics and reporting
//! - Historical data management

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// Time tracking configuration
#[derive(Debug, Clone)]
pub struct TimeTrackerConfig {
    pub idle_threshold: Duration,
    pub session_timeout: Duration,
    pub history_limit: usize,
    pub aggregate_interval: Duration,
}

impl Default for TimeTrackerConfig {
    fn default() -> Self {
        Self {
            idle_threshold: Duration::from_secs(300),   // 5 minutes
            session_timeout: Duration::from_secs(1800), // 30 minutes
            history_limit: 10000,
            aggregate_interval: Duration::from_secs(60), // 1 minute
        }
    }
}

/// A time session for an application
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSession {
    pub app_name: String,
    pub bundle_id: String,
    pub start_time: SystemTime,
    pub end_time: Option<SystemTime>,
    pub duration: Duration,
    pub active_duration: Duration,
    pub idle_duration: Duration,
    pub focus_count: u32,
    pub window_count: usize,
}

/// Aggregated usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    pub total_time: Duration,
    pub active_time: Duration,
    pub idle_time: Duration,
    pub session_count: u32,
    pub average_session_duration: Duration,
    pub longest_session: Duration,
    pub last_used: SystemTime,
}

/// Time entry for tracking
#[derive(Debug, Clone)]
struct TimeEntry {
    start: Instant,
    end: Option<Instant>,
    active: bool,
}

/// Time tracker for application usage
pub struct TimeTracker {
    config: TimeTrackerConfig,
    current_app: Option<String>,
    current_entry: Option<TimeEntry>,
    app_sessions: Arc<Mutex<HashMap<String, Vec<AppSession>>>>,
    app_stats: Arc<Mutex<HashMap<String, UsageStats>>>,
    history: Arc<Mutex<VecDeque<AppSession>>>,
    last_activity: Instant,
    session_start: Option<Instant>,
}

impl TimeTracker {
    pub fn new(config: TimeTrackerConfig) -> Self {
        let history_limit = config.history_limit;
        Self {
            config,
            current_app: None,
            current_entry: None,
            app_sessions: Arc::new(Mutex::new(HashMap::new())),
            app_stats: Arc::new(Mutex::new(HashMap::new())),
            history: Arc::new(Mutex::new(VecDeque::with_capacity(history_limit))),
            last_activity: Instant::now(),
            session_start: None,
        }
    }

    /// Start tracking time for an application
    pub fn start_tracking(&mut self, _app_name: String, bundle_id: String) {
        // End current tracking if exists
        if let Some(_current) = &self.current_app {
            self.end_tracking();
        }

        self.current_app = Some(bundle_id.clone());
        self.current_entry = Some(TimeEntry {
            start: Instant::now(),
            end: None,
            active: true,
        });

        if self.session_start.is_none() {
            self.session_start = Some(Instant::now());
        }

        self.last_activity = Instant::now();
    }

    /// End current tracking
    pub fn end_tracking(&mut self) {
        if let (Some(app_id), Some(entry)) = (&self.current_app, &mut self.current_entry) {
            entry.end = Some(Instant::now());

            let duration = entry.end.unwrap() - entry.start;
            let active_duration = if entry.active {
                duration
            } else {
                Duration::ZERO
            };
            let idle_duration = if !entry.active {
                duration
            } else {
                Duration::ZERO
            };

            // Create session
            let session = AppSession {
                app_name: self.get_app_name(app_id),
                bundle_id: app_id.clone(),
                start_time: SystemTime::now() - duration,
                end_time: Some(SystemTime::now()),
                duration,
                active_duration,
                idle_duration,
                focus_count: 1,
                window_count: 0,
            };

            // Update sessions
            let mut sessions = self.app_sessions.lock().unwrap();
            sessions
                .entry(app_id.clone())
                .or_insert_with(Vec::new)
                .push(session.clone());

            // Update stats
            let mut stats = self.app_stats.lock().unwrap();
            let stat = stats.entry(app_id.clone()).or_insert_with(|| UsageStats {
                total_time: Duration::ZERO,
                active_time: Duration::ZERO,
                idle_time: Duration::ZERO,
                session_count: 0,
                average_session_duration: Duration::ZERO,
                longest_session: Duration::ZERO,
                last_used: SystemTime::now(),
            });

            stat.total_time += duration;
            stat.active_time += active_duration;
            stat.idle_time += idle_duration;
            stat.session_count += 1;
            stat.average_session_duration = stat.total_time / stat.session_count;
            stat.longest_session = stat.longest_session.max(duration);
            stat.last_used = SystemTime::now();

            // Add to history
            let mut history = self.history.lock().unwrap();
            if history.len() >= self.config.history_limit {
                history.pop_front();
            }
            history.push_back(session);
        }

        self.current_app = None;
        self.current_entry = None;
    }

    /// Mark current tracking as idle
    pub fn mark_idle(&mut self) {
        if let Some(entry) = &mut self.current_entry {
            entry.active = false;
        }
    }

    /// Mark current tracking as active
    pub fn mark_active(&mut self) {
        if let Some(entry) = &mut self.current_entry {
            entry.active = true;
        }
        self.last_activity = Instant::now();
    }

    /// Check if currently idle
    pub fn is_idle(&self) -> bool {
        self.last_activity.elapsed() > self.config.idle_threshold
    }

    /// Get total usage stats for an app
    pub fn get_app_stats(&self, bundle_id: &str) -> Option<UsageStats> {
        let stats = self.app_stats.lock().unwrap();
        stats.get(bundle_id).cloned()
    }

    /// Get all app stats
    pub fn get_all_stats(&self) -> HashMap<String, UsageStats> {
        let stats = self.app_stats.lock().unwrap();
        stats.clone()
    }

    /// Get recent sessions
    pub fn get_recent_sessions(&self, limit: usize) -> Vec<AppSession> {
        let history = self.history.lock().unwrap();
        history.iter().rev().take(limit).cloned().collect()
    }

    /// Get sessions for a specific app
    pub fn get_app_sessions(&self, bundle_id: &str) -> Vec<AppSession> {
        let sessions = self.app_sessions.lock().unwrap();
        sessions
            .get(bundle_id)
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// Get today's usage
    pub fn get_today_usage(&self) -> HashMap<String, Duration> {
        let mut usage = HashMap::new();
        let today = Local::now().date_naive();

        let history = self.history.lock().unwrap();
        for session in history.iter() {
            let session_date = DateTime::<Local>::from(session.start_time).date_naive();
            if session_date == today {
                *usage
                    .entry(session.bundle_id.clone())
                    .or_insert(Duration::ZERO) += session.duration;
            }
        }

        usage
    }

    /// Get usage for date range
    pub fn get_usage_range(&self, start: SystemTime, end: SystemTime) -> HashMap<String, Duration> {
        let mut usage = HashMap::new();

        let history = self.history.lock().unwrap();
        for session in history.iter() {
            if session.start_time >= start && session.start_time <= end {
                *usage
                    .entry(session.bundle_id.clone())
                    .or_insert(Duration::ZERO) += session.duration;
            }
        }

        usage
    }

    /// Export data as JSON
    pub fn export_json(&self) -> Result<String, serde_json::Error> {
        let history = self.history.lock().unwrap();
        let sessions: Vec<AppSession> = history.iter().cloned().collect();
        serde_json::to_string_pretty(&sessions)
    }

    /// Import data from JSON
    pub fn import_json(&mut self, json: &str) -> Result<(), serde_json::Error> {
        let sessions: Vec<AppSession> = serde_json::from_str(json)?;

        let mut history = self.history.lock().unwrap();
        let mut app_sessions = self.app_sessions.lock().unwrap();
        let mut stats = self.app_stats.lock().unwrap();

        for session in sessions {
            // Add to history
            if history.len() >= self.config.history_limit {
                history.pop_front();
            }
            history.push_back(session.clone());

            // Update app sessions
            app_sessions
                .entry(session.bundle_id.clone())
                .or_insert_with(Vec::new)
                .push(session.clone());

            // Update stats
            let stat = stats
                .entry(session.bundle_id.clone())
                .or_insert_with(|| UsageStats {
                    total_time: Duration::ZERO,
                    active_time: Duration::ZERO,
                    idle_time: Duration::ZERO,
                    session_count: 0,
                    average_session_duration: Duration::ZERO,
                    longest_session: Duration::ZERO,
                    last_used: SystemTime::now(),
                });

            stat.total_time += session.duration;
            stat.active_time += session.active_duration;
            stat.idle_time += session.idle_duration;
            stat.session_count += 1;
        }

        // Recalculate averages
        for (_, stat) in stats.iter_mut() {
            if stat.session_count > 0 {
                stat.average_session_duration = stat.total_time / stat.session_count;
            }
        }

        Ok(())
    }

    /// Clear all data
    pub fn clear_data(&mut self) {
        self.app_sessions.lock().unwrap().clear();
        self.app_stats.lock().unwrap().clear();
        self.history.lock().unwrap().clear();
        self.current_app = None;
        self.current_entry = None;
        self.session_start = None;
    }

    /// Get summary report
    pub fn get_summary_report(&self) -> SummaryReport {
        let stats = self.app_stats.lock().unwrap();
        let _history = self.history.lock().unwrap();

        let total_time: Duration = stats.values().map(|s| s.total_time).sum();
        let total_active: Duration = stats.values().map(|s| s.active_time).sum();
        let total_idle: Duration = stats.values().map(|s| s.idle_time).sum();
        let total_sessions: u32 = stats.values().map(|s| s.session_count).sum();

        let top_apps: Vec<(String, Duration)> = {
            let mut apps: Vec<_> = stats
                .iter()
                .map(|(id, s)| (id.clone(), s.total_time))
                .collect();
            apps.sort_by(|a, b| b.1.cmp(&a.1));
            apps.into_iter().take(10).collect()
        };

        SummaryReport {
            total_tracked_time: total_time,
            total_active_time: total_active,
            total_idle_time: total_idle,
            total_sessions,
            unique_apps: stats.len(),
            top_apps_by_usage: top_apps,
            current_session_duration: self.session_start.map(|s| s.elapsed()),
            last_activity: Some(self.last_activity),
        }
    }

    fn get_app_name(&self, bundle_id: &str) -> String {
        // Simple extraction - could be enhanced
        bundle_id.split('.').last().unwrap_or(bundle_id).to_string()
    }
}

/// Summary report of time tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryReport {
    pub total_tracked_time: Duration,
    pub total_active_time: Duration,
    pub total_idle_time: Duration,
    pub total_sessions: u32,
    pub unique_apps: usize,
    pub top_apps_by_usage: Vec<(String, Duration)>,
    pub current_session_duration: Option<Duration>,
    #[serde(skip)]
    pub last_activity: Option<Instant>,
}

/// Listener trait for time tracking events
pub trait TimeTrackingListener: Send + Sync {
    fn on_session_start(&mut self, app: &str, bundle_id: &str);
    fn on_session_end(&mut self, session: &AppSession);
    fn on_idle_detected(&mut self, app: &str);
    fn on_active_detected(&mut self, app: &str);
    fn on_milestone_reached(&mut self, app: &str, total_time: Duration);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_tracking() {
        let mut tracker = TimeTracker::new(TimeTrackerConfig::default());

        // Start tracking
        tracker.start_tracking("Safari".to_string(), "com.apple.Safari".to_string());
        std::thread::sleep(Duration::from_millis(100));
        tracker.end_tracking();

        // Check stats
        let stats = tracker.get_app_stats("com.apple.Safari").unwrap();
        assert!(stats.total_time >= Duration::from_millis(100));
        assert_eq!(stats.session_count, 1);
    }

    #[test]
    fn test_idle_detection() {
        let mut config = TimeTrackerConfig::default();
        config.idle_threshold = Duration::from_millis(50);
        let mut tracker = TimeTracker::new(config);

        tracker.start_tracking("Test".to_string(), "com.test".to_string());
        assert!(!tracker.is_idle());

        std::thread::sleep(Duration::from_millis(60));
        assert!(tracker.is_idle());

        tracker.mark_active();
        assert!(!tracker.is_idle());
    }
}
