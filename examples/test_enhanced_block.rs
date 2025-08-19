//! Test the enhanced block variant app switcher
//!
//! This example demonstrates how to use the enhanced block variant
//! with its comprehensive monitoring capabilities.

use std::time::Duration;
use objc2::MainThreadMarker;
use research_assistant_tracker::core::app_switcher_enhanced_block::{
    EnhancedAppSwitcher, EnhancedAppSwitchListener, EnhancedAppSwitchEvent, DebugListener
};

struct TestListener {
    event_count: usize,
}

impl TestListener {
    fn new() -> Self {
        Self { event_count: 0 }
    }
}

impl EnhancedAppSwitchListener for TestListener {
    fn on_app_switch(&mut self, event: &EnhancedAppSwitchEvent) {
        self.event_count += 1;
        
        println!("🔄 Enhanced Event #{}", self.event_count);
        println!("  App: {} ({})", event.app_info.name, event.app_info.bundle_id);
        println!("  PID: {}", event.app_info.pid);
        println!("  Type: {:?}", event.event_type);
        println!("  Trigger: {:?}", event.trigger_source);
        println!("  Confidence: {:.0}%", event.confidence_score * 100.0);
        
        // Enhanced data
        if let Some(process) = &event.app_info.process_info {
            println!("  CPU: {:.1}%", process.cpu_usage);
            println!("  Memory: {:.1} MB", process.memory_bytes as f64 / 1_048_576.0);
            println!("  Threads: {}", process.num_threads);
        }
        
        if let Some(window) = &event.app_info.frontmost_window {
            if let Some(title) = &window.title {
                println!("  Window: \"{}\"", title);
            }
            println!("  Window Bounds: {:.0}x{:.0} at ({:.0}, {:.0})", 
                window.bounds.width, window.bounds.height, 
                window.bounds.x, window.bounds.y);
            println!("  Window Layer: {}", window.layer);
            println!("  Window Alpha: {:.2}", window.alpha);
        }
        
        // Desktop state
        println!("  Desktop State:");
        println!("    Session Active: {}", event.desktop_state.session_active);
        println!("    Screen Locked: {}", event.desktop_state.screen_locked);
        println!("    Display Count: {}", event.desktop_state.display_count);
        if let Some(user) = &event.desktop_state.console_user {
            println!("    Console User: {}", user);
        }
        if let Some(idle) = event.desktop_state.idle_time_seconds {
            println!("    Idle Time: {:.1}s", idle);
        }
        
        if let Some(icon) = &event.app_info.icon_base64_png {
            println!("  Icon: {} bytes base64", icon.len());
        }
        
        println!();
    }
    
    fn on_monitoring_started(&mut self) {
        println!("✅ Enhanced block monitoring started!");
    }
    
    fn on_monitoring_stopped(&mut self) {
        println!("🛑 Enhanced block monitoring stopped. Total events: {}", self.event_count);
    }
    
    fn on_desktop_state_change(&mut self, state: &research_assistant_tracker::core::app_switcher_enhanced_block::DesktopState) {
        println!("🖥️  Desktop changed: {} displays, idle: {:.1}s", 
            state.display_count,
            state.idle_time_seconds.unwrap_or(0.0)
        );
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Testing Enhanced Block App Switcher");
    println!("Switch between applications to see detailed monitoring data\n");
    
    // Must be on main thread for NSWorkspace
    let mtm = MainThreadMarker::new()
        .ok_or("Must run on main thread")?;
    
    // Create the enhanced switcher
    let mut switcher = EnhancedAppSwitcher::new();
    
    // Add our test listener
    switcher.add_listener(TestListener::new());
    
    // Also add the debug listener for comparison
    switcher.add_listener(DebugListener);
    
    // Start monitoring
    switcher.start_monitoring(mtm)?;
    
    println!("Monitoring active. Press Ctrl+C to stop...\n");
    
    // Wait for interrupt
    tokio::signal::ctrl_c().await?;
    
    // Stop monitoring
    switcher.stop_monitoring();
    
    println!("\n✅ Test completed!");
    Ok(())
}