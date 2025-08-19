// src/main.rs
//! Modern Research Assistant Focus Tracker
//!
//! This application demonstrates the power of the modern objc2 ecosystem
//! for building sophisticated macOS system monitoring tools. The architecture
//! is designed to be educational, showing how to properly layer functionality
//! while maintaining type safety and memory correctness.

#![deny(unsafe_op_in_unsafe_fn)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_foundation::NSAutoreleasePool;
// use tokio::signal;  // no longer used; CFRunLoop drives the runloop
use core_foundation::runloop::CFRunLoopRun;
use tracing::{error, info, warn};

use research_assistant_tracker::core::accessibility::AccessibilityContextExtractor;
use research_assistant_tracker::core::app_switcher::{
    initialize_app_switcher, AppSwitchEvent, AppSwitchListener, AppSwitchType, AppSwitcher,
};
// Optional non-AX scroll trigger (use local module wrapper to avoid crate path issues)
mod detectors;
use crate::detectors::scroll_tap::{ScrollEvent, ScrollListener, ScrollTap};
use research_assistant_tracker::extractors::time_tracker::{TimeTracker, TimeTrackerConfig};

/// Command line interface for the research assistant tracker
///
/// This CLI demonstrates modern Rust patterns for configuration management
/// while providing a clean interface for different use cases.
#[derive(Debug, Parser)]
#[command(
    name = "research-tracker",
    about = "Modern macOS focus tracking system for research assistance",
    long_about = "A sophisticated, modular system for tracking application focus and context on macOS. Built with modern Rust patterns and the objc2 ecosystem for maximum safety and performance."
)]
struct Args {
    /// Output format for events
    #[arg(long, default_value = "human", value_enum)]
    format: OutputFormat,

    /// Enable enhanced context extraction using accessibility APIs
    #[arg(
        long,
        default_value_t = true,
        help = "Extract detailed context (URLs, file paths, etc.) - requires accessibility permissions"
    )]
    enhanced: bool,

    /// Verbosity level for logging
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Run in background mode (no interactive prompts)
    #[arg(long, help = "Run without prompting for permissions")]
    background: bool,

    /// Filter to specific application types
    #[arg(
        long,
        help = "Only track specific app types: browser, ide, productivity"
    )]
    filter: Option<String>,

    /// Output file for structured data
    #[arg(long, help = "Write structured events to file")]
    output_file: Option<std::path::PathBuf>,

    /// Check permissions and exit
    #[arg(long, help = "Check required permissions and exit")]
    check_permissions: bool,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum OutputFormat {
    /// Human-readable output with colors and formatting
    Human,
    /// JSON output for programmatic processing
    Json,
    /// Structured output optimized for research analysis
    Research,
}

/// The main application state
///
/// This structure encapsulates all the moving parts of our system
/// and demonstrates how to organize complex state in a thread-safe way.
struct TrackerApp {
    app_switcher: Arc<Mutex<AppSwitcher>>,
    config: Args,
    start_time: std::time::Instant,
}

impl TrackerApp {
    /// Create a new tracker application
    ///
    /// This constructor sets up all the necessary components and validates
    /// that we have the required permissions to operate.
    async fn new(config: Args) -> Result<Self> {
        let start_time = std::time::Instant::now();

        // Initialize logging based on verbosity
        Self::setup_logging(&config)?;

        info!(
            "🚀 Starting Research Assistant Tracker v{}",
            env!("CARGO_PKG_VERSION")
        );
        info!("Configuration: {:#?}", config);

        // Check permissions first if requested
        if config.check_permissions {
            Self::check_and_report_permissions().await?;
            std::process::exit(0);
        }

        // Validate that we're running on macOS
        #[cfg(not(target_os = "macos"))]
        {
            return Err(anyhow::anyhow!("This application only runs on macOS"));
        }

        // Set up the core app switcher
        let app_switcher = Arc::new(Mutex::new(AppSwitcher::new()));

        Ok(Self {
            app_switcher,
            config,
            start_time,
        })
    }

    /// Run the tracker application
    ///
    /// This is the main event loop that coordinates all the different
    /// components and handles graceful shutdown.
    async fn run(mut self) -> Result<()> {
        // Get main thread marker for objc2 safety
        let mtm = MainThreadMarker::new()
            .context("Must run on main thread for NSApplication integration")?;

        // Initialize the macOS application context
        self.setup_macos_context(mtm)?;

        // Set up listeners based on configuration
        self.setup_listeners().await?;

        // Start monitoring
        {
            let mut switcher = self.app_switcher.lock().unwrap();
            switcher
                .start_monitoring(mtm)
                .map_err(|e| anyhow::anyhow!("Failed to start monitoring app switches: {}", e))?;
        }

        info!("👀 Monitoring started. Press Ctrl+C to stop gracefully.");

        // Run until interrupted
        self.run_until_interrupted().await?;

        // Graceful shutdown
        self.shutdown().await?;

        let elapsed = self.start_time.elapsed();
        info!(
            "📊 Session completed. Runtime: {:.2}s",
            elapsed.as_secs_f64()
        );

        Ok(())
    }

    /// Set up the macOS application context
    ///
    /// This method demonstrates the modern way to initialize NSApplication
    /// for a background monitoring app using objc2.
    fn setup_macos_context(&self, mtm: MainThreadMarker) -> Result<()> {
        // Initialize the app switcher system
        initialize_app_switcher(mtm).map_err(|e| anyhow::anyhow!("{}", e))?;

        // Configure NSApplication for background operation
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);

        info!("✅ macOS application context initialized");

        // Start passive scroll tap to trigger re-ingestion (best-effort)
        #[allow(unused_mut)]
        {
            struct ReIngestOnScroll {
                switcher: Arc<Mutex<AppSwitcher>>,
            }
            impl ScrollListener for ReIngestOnScroll {
                fn on_scroll(&mut self, _event: &ScrollEvent) {
                    if let Ok(sw) = self.switcher.lock() {
                        sw.resample_now();
                    }
                }
            }
            let _ = ScrollTap::start(Duration::from_millis(250));
            let listener = ReIngestOnScroll {
                switcher: self.app_switcher.clone(),
            };
            let tap = ScrollTap;
            tap.add_listener(listener);
        }
        Ok(())
    }

    /// Set up event listeners based on configuration
    ///
    /// This method shows how the modular architecture allows us to
    /// conditionally enable different types of monitoring based on
    /// user preferences and available permissions.
    async fn setup_listeners(&mut self) -> Result<()> {
        let mut switcher = self.app_switcher.lock().unwrap();

        // Always add basic logging
        let basic_logger = BasicEventLogger::new(self.config.format.clone());
        switcher.add_listener(basic_logger);

        // Always add time tracking - this is core functionality
        let time_tracker_config = TimeTrackerConfig {
            print_updates: self.config.verbose > 0,
            min_session_duration: Duration::from_secs(2),
            track_background: false,
            max_history_size: 10000,
        };
        let time_tracker = TimeTracker::with_config(time_tracker_config);
        switcher.add_listener(time_tracker);
        info!("⏰ Time tracking enabled");

        // Add enhanced context extraction if requested
        if self.config.enhanced {
            match AccessibilityContextExtractor::new() {
                Ok(extractor) => {
                    info!("🔍 Enhanced context extraction enabled");
                    switcher.add_listener(extractor);
                }
                Err(e) => {
                    if self.config.background {
                        error!(
                            "❌ Enhanced context requires accessibility permissions: {}",
                            e
                        );
                        return Err(anyhow::anyhow!("Accessibility permissions required"));
                    } else {
                        warn!("⚠️  Enhanced context unavailable: {}", e);
                        warn!("💡 Enable in: System Settings → Privacy & Security → Accessibility");
                    }
                }
            }
        }

        // Add file output if specified
        if let Some(output_path) = &self.config.output_file {
            let file_logger = FileEventLogger::new(output_path.clone())?;
            switcher.add_listener(file_logger);
            info!("📁 File output enabled: {}", output_path.display());
        }

        Ok(())
    }

    /// Run the main event loop until interrupted
    ///
    /// This method shows how to properly integrate tokio async runtime
    /// with the NSRunLoop-based objc2 event system.
    async fn run_until_interrupted(&self) -> Result<()> {
        // Pump the CoreFoundation run loop on the main thread so AppKit/NSWorkspace notifications fire.
        // Our helper scripts send SIGTERM to exit; CFRunLoopRun will be interrupted by process kill.
        let _pool = unsafe { NSAutoreleasePool::new() };
        unsafe { CFRunLoopRun() };
        Ok(())
    }

    /// Periodic health check to ensure the system is working correctly
    ///
    /// This demonstrates how to add robustness to long-running monitoring applications.
    async fn periodic_health_check(&self) {
        let mut interval = tokio::time::interval(Duration::from_secs(60));

        loop {
            interval.tick().await;

            // Check if the switcher is still responsive
            if let Ok(switcher) = self.app_switcher.try_lock() {
                if let Some(current_app) = switcher.current_app() {
                    info!("💓 Health check: Currently tracking {}", current_app.name);
                } else {
                    warn!("⚠️  Health check: No current application tracked");
                }
            } else {
                error!("❌ Health check: Switcher lock unavailable");
                break;
            }
        }
    }

    /// Graceful shutdown
    async fn shutdown(&mut self) -> Result<()> {
        info!("🛑 Initiating graceful shutdown...");

        // Stop monitoring
        {
            let mut switcher = self.app_switcher.lock().unwrap();
            switcher.stop_monitoring();
        }

        // Give async tasks time to complete
        tokio::time::sleep(Duration::from_millis(100)).await;

        info!("✅ Shutdown complete");
        Ok(())
    }

    /// Set up logging based on verbosity level
    ///
    /// This shows modern Rust logging practices with the tracing ecosystem.
    fn setup_logging(config: &Args) -> Result<()> {
        use tracing_subscriber::{fmt, EnvFilter};

        let level = match config.verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        };

        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

        fmt()
            .with_env_filter(filter)
            .with_target(config.verbose > 1)
            .with_thread_ids(config.verbose > 2)
            .init();

        Ok(())
    }

    /// Check and report on required permissions
    async fn check_and_report_permissions() -> Result<()> {
        use accessibility_sys::AXIsProcessTrusted;

        println!("🔐 Checking required permissions...\n");

        // Check accessibility permissions
        let accessibility_trusted = unsafe { AXIsProcessTrusted() };

        if accessibility_trusted {
            println!("✅ Accessibility: Granted");
        } else {
            println!("❌ Accessibility: Not granted");
            println!("   Enable in: System Settings → Privacy & Security → Accessibility");
            println!("   Add this application and enable the checkbox");
        }

        // Check if we can create NSApplication (basic app functionality)
        let basic_app_access = {
            if let Some(mtm) = MainThreadMarker::new() {
                let _ = NSApplication::sharedApplication(mtm);
                true
            } else {
                false
            }
        };

        if basic_app_access {
            println!("✅ Application Framework: Available");
        } else {
            println!("❌ Application Framework: Unavailable");
        }

        println!("\n📋 Summary:");
        println!(
            "   Basic app switching: {}",
            if basic_app_access {
                "✅ Available"
            } else {
                "❌ Unavailable"
            }
        );
        println!(
            "   Enhanced context: {}",
            if accessibility_trusted {
                "✅ Available"
            } else {
                "❌ Requires accessibility"
            }
        );

        if !accessibility_trusted {
            println!("\n💡 To enable enhanced context extraction:");
            println!("   1. Open System Settings");
            println!("   2. Go to Privacy & Security → Accessibility");
            println!("   3. Add this application");
            println!("   4. Enable the checkbox");
        }

        Ok(())
    }
}

/// Basic event logger that prints to stdout
///
/// This demonstrates how to implement the AppSwitchListener trait
/// for different output formats.
struct BasicEventLogger {
    format: OutputFormat,
    event_count: usize,
    last_switch_at: Option<Instant>,
    last_app: Option<research_assistant_tracker::core::app_switcher::AppInfo>,
}

impl BasicEventLogger {
    fn new(format: OutputFormat) -> Self {
        Self {
            format,
            event_count: 0,
            last_switch_at: None,
            last_app: None,
        }
    }
}

impl AppSwitchListener for BasicEventLogger {
    fn on_app_switch(&mut self, event: &AppSwitchEvent) {
        self.event_count += 1;

        let now = Instant::now();
        let prev_app = event.previous_app.clone().or_else(|| self.last_app.clone());
        let prev_duration = self
            .last_switch_at
            .map(|t| now.saturating_duration_since(t))
            .unwrap_or(Duration::from_secs(0));

        match self.format {
            OutputFormat::Human => match event.event_type {
                AppSwitchType::Foreground => {
                    println!(
                        "\n🔥 #{} SWITCHED TO: {} ({})",
                        self.event_count, event.app_info.name, event.app_info.bundle_id
                    );
                    if let Some(prev) = &prev_app {
                        let secs = prev_duration.as_secs_f32();
                        println!("   From: {} (pid: {}, {:.1}s)", prev.name, prev.pid, secs);
                    }
                    if let Some(path) = &event.app_info.path {
                        println!("   Path: {}", path);
                    }
                    if let Some(icon_path) = &event.app_info.icon_path {
                        println!("   Icon path: {}", icon_path);
                    }
                    let window_title = event
                        .workspace
                        .as_ref()
                        .and_then(|w| w.focused_title.clone())
                        .or_else(|| {
                            event
                                .enhanced
                                .as_ref()
                                .and_then(|e| e.front_window_title.clone())
                        });
                    if let Some(title) = window_title {
                        println!("   Window: {}", title);
                    }
                    // Prefer workspace URL; fall back to enhanced URL if available
                    if let Some(url) = event
                        .workspace
                        .as_ref()
                        .and_then(|w| w.primary_url.clone())
                        .or_else(|| event.enhanced.as_ref().and_then(|e| e.url.clone()))
                    {
                        println!("   URL: {}", url);
                    }
                    // Display / Space info
                    if let Some(enh) = &event.enhanced {
                        if let Some(dc) = enh.display_count {
                            println!("   Displays: {}", dc);
                        }
                        if let Some(did) = enh.display_id {
                            println!("   Display ID: {}", did);
                        }
                        if let Some(space) = enh.space_id {
                            println!("   Space (ID): {}", space);
                        }
                        if enh.space_index.is_some()
                            || enh.space_type.is_some()
                            || enh.space_name.is_some()
                            || enh.space_uuid.is_some()
                            || enh.space_label.is_some()
                        {
                            println!(
                                "   Space info: index={:?} type={:?} name={:?} label={:?} uuid={:?}",
                                enh.space_index, enh.space_type, enh.space_name, enh.space_label, enh.space_uuid
                            );
                        }
                    }
                }
                AppSwitchType::Background => {
                    println!("📱 {} went to background", event.app_info.name);
                }
                _ => {
                    println!(
                        "📋 #{} {:?}: {}",
                        self.event_count, event.event_type, event.app_info.name
                    );
                }
            },
            OutputFormat::Json => {
                let json_event = serde_json::json!({
                    "event_number": self.event_count,
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "event_type": format!("{:?}", event.event_type),
                    "app": {
                        "name": event.app_info.name,
                        "bundle_id": event.app_info.bundle_id,
                        "pid": event.app_info.pid,
                        "path": event.app_info.path,
                        "icon_path": event.app_info.icon_path,
                    },
                    "previous_app": prev_app.as_ref().map(|app| {
                        serde_json::json!({
                            "name": app.name,
                            "bundle_id": app.bundle_id,
                            "pid": app.pid,
                            "duration_seconds": prev_duration.as_secs_f64()
                        })
                    }),
                    "workspace": event.workspace.as_ref().map(|w| serde_json::json!({
                        "window_count": w.window_count,
                        "focused_title": w.focused_title,
                        "primary_url": w.primary_url,
                    })),
                    "enhanced": event.enhanced.as_ref().map(|e| serde_json::json!({
                        "activation_count": e.activation_count,
                        "front_window_title": e.front_window_title,
                        "cpu_usage": e.cpu_usage,
                        "memory_bytes": e.memory_bytes,
                        "session_active": e.session_active,
                        "screen_locked": e.screen_locked,
                        "display_count": e.display_count,
                        "display_id": e.display_id,
                        "space_id": e.space_id,
                        "url": e.url,
                        "tab_title": e.tab_title,
                    })),
                    "confidence": event.confidence
                });
                println!("{}", serde_json::to_string(&json_event).unwrap());
            }
            OutputFormat::Research => {
                // Optimized format for research analysis
                let timestamp = chrono::Utc::now().to_rfc3339();
                println!(
                    "RESEARCH|{}|{:?}|{}|{}|{}|prev_pid={}|prev_secs={:.1}|title={}|url={}|display_count={}|space={}",
                    timestamp,
                    event.event_type,
                    event.app_info.name,
                    event.app_info.bundle_id,
                    event.app_info.pid,
                    prev_app.as_ref().map(|p| p.pid).unwrap_or_default(),
                    prev_duration.as_secs_f32(),
                    event
                        .workspace
                        .as_ref()
                        .and_then(|w| w.focused_title.clone())
                        .or_else(|| event.enhanced.as_ref().and_then(|e| e.front_window_title.clone()))
                        .or_else(|| event.enhanced.as_ref().and_then(|e| e.tab_title.clone()))
                        .unwrap_or_default(),
                    event
                        .workspace
                        .as_ref()
                        .and_then(|w| w.primary_url.clone())
                        .or_else(|| event.enhanced.as_ref().and_then(|e| e.url.clone()))
                        .unwrap_or_default(),
                    event
                        .enhanced
                        .as_ref()
                        .and_then(|e| e.display_count)
                        .unwrap_or(0),
                    event
                        .enhanced
                        .as_ref()
                        .and_then(|e| e.space_id)
                        .map(|s| s.to_string())
                        .unwrap_or_default()
                );
            }
        }

        // Update dwell tracking
        self.last_switch_at = Some(now);
        self.last_app = Some(event.app_info.clone());
    }

    fn on_monitoring_started(&mut self) {
        match self.format {
            OutputFormat::Human => {
                println!("🚀 Basic event logging started");
            }
            OutputFormat::Json => {
                let start_event = serde_json::json!({
                    "event_type": "monitoring_started",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                });
                println!("{}", serde_json::to_string(&start_event).unwrap());
            }
            OutputFormat::Research => {
                println!(
                    "RESEARCH|{}|monitoring_started",
                    chrono::Utc::now().to_rfc3339()
                );
            }
        }
    }
}

/// File-based event logger for persistent storage
///
/// This shows how to implement file output for long-term research data collection.
struct FileEventLogger {
    file: std::fs::File,
}

impl FileEventLogger {
    fn new(path: std::path::PathBuf) -> Result<Self> {
        use std::fs::OpenOptions;

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .context("Failed to open output file")?;

        Ok(Self { file })
    }
}

impl AppSwitchListener for FileEventLogger {
    fn on_app_switch(&mut self, event: &AppSwitchEvent) {
        use std::io::Write;

        let json_event = serde_json::json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "event_type": format!("{:?}", event.event_type),
            "app": {
                "name": event.app_info.name,
                "bundle_id": event.app_info.bundle_id,
                "pid": event.app_info.pid,
                "path": event.app_info.path,
                "icon_path": event.app_info.icon_path,
                "launch_date": event.app_info.launch_date.map(|_| chrono::Utc::now().to_rfc3339())
            },
            "previous_app": event.previous_app.as_ref().map(|app| {
                serde_json::json!({
                    "name": app.name,
                    "bundle_id": app.bundle_id,
                    "pid": app.pid
                })
            }),
            "workspace": event.workspace.as_ref().map(|w| serde_json::json!({
                "window_count": w.window_count,
                "focused_title": w.focused_title,
                "primary_url": w.primary_url,
            })),
            "enhanced": event.enhanced.as_ref().map(|e| serde_json::json!({
                "activation_count": e.activation_count,
                "front_window_title": e.front_window_title,
                "cpu_usage": e.cpu_usage,
                "memory_bytes": e.memory_bytes,
                "session_active": e.session_active,
                "screen_locked": e.screen_locked,
            })),
            "confidence": event.confidence
        });

        if let Err(e) = writeln!(self.file, "{}", serde_json::to_string(&json_event).unwrap()) {
            error!("Failed to write to output file: {}", e);
        }
    }
}

/// Application entry point
///
/// This demonstrates the modern async main pattern with proper error handling.
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Create and run the tracker application
    let app = TrackerApp::new(args)
        .await
        .context("Failed to initialize tracker application")?;

    app.run().await.context("Application runtime error")?;

    Ok(())
}
