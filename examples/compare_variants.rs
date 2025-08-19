//! Compare standard vs enhanced block variants
//!
//! This example runs both variants side by side to show the differences
//! in monitoring capabilities and data richness.

use std::sync::{Arc, Mutex};
use objc2::MainThreadMarker;

use research_assistant_tracker::core::app_switcher::{AppSwitcher, AppSwitchListener, AppSwitchEvent};
use research_assistant_tracker::core::app_switcher_enhanced_block::{
    EnhancedAppSwitcher, EnhancedAppSwitchListener, EnhancedAppSwitchEvent
};

struct StandardListener {
    label: String,
    count: usize,
}

impl StandardListener {
    fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            count: 0,
        }
    }
}

impl AppSwitchListener for StandardListener {
    fn on_app_switch(&mut self, event: &AppSwitchEvent) {
        self.count += 1;
        println!("📱 [{}] #{}: {} -> {}", 
            self.label, self.count, 
            event.previous_app.as_ref().map(|a| a.name.as_str()).unwrap_or("None"),
            event.app_info.name
        );
    }
}

struct EnhancedListener {
    label: String,
    count: usize,
}

impl EnhancedListener {
    fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            count: 0,
        }
    }
}

impl EnhancedAppSwitchListener for EnhancedListener {
    fn on_app_switch(&mut self, event: &EnhancedAppSwitchEvent) {
        self.count += 1;
        
        let prev_name = event.previous_app.as_ref()
            .map(|a| a.name.as_str())
            .unwrap_or("None");
            
        let window_info = event.app_info.frontmost_window.as_ref()
            .and_then(|w| w.title.as_ref())
            .map(|t| format!(" [{}]", t))
            .unwrap_or_default();
            
        let cpu_info = event.app_info.process_info.as_ref()
            .map(|p| format!(" CPU:{:.1}%", p.cpu_usage))
            .unwrap_or_default();
            
        println!("🔥 [{}] #{}: {} -> {}{}{} (conf:{:.0}%)", 
            self.label, self.count, prev_name, event.app_info.name,
            window_info, cpu_info, event.confidence_score * 100.0
        );
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔬 Comparing Standard vs Enhanced Block Variants");
    println!("Both will monitor simultaneously - notice the difference in detail\n");
    
    let mtm = MainThreadMarker::new()
        .ok_or("Must run on main thread")?;
    
    // Set up standard switcher
    let mut standard_switcher = AppSwitcher::new();
    standard_switcher.add_listener(StandardListener::new("Standard"));
    
    // Set up enhanced switcher  
    let mut enhanced_switcher = EnhancedAppSwitcher::new();
    enhanced_switcher.add_listener(EnhancedListener::new("Enhanced"));
    
    // Start both
    standard_switcher.start_monitoring(mtm)?;
    enhanced_switcher.start_monitoring(mtm)?;
    
    println!("Both monitors active. Switch apps to see the differences!\n");
    
    // Run for comparison
    tokio::signal::ctrl_c().await?;
    
    // Stop both
    standard_switcher.stop_monitoring();
    enhanced_switcher.stop_monitoring();
    
    println!("\n📊 Comparison completed!");
    Ok(())
}