//! Simple test for the enhanced block variant
//!
//! Run this to test the enhanced monitoring capabilities

use objc2::MainThreadMarker;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Testing Enhanced Block App Switcher");
    println!("Switch between applications to see detailed monitoring\n");
    
    // Get main thread marker
    let mtm = MainThreadMarker::new()
        .ok_or("Must run on main thread")?;
    
    // Use the enhanced switcher directly from the module
    use research_assistant_tracker::core::app_switcher_enhanced_block::{
        EnhancedAppSwitcher, DebugListener
    };
    
    let mut switcher = EnhancedAppSwitcher::new();
    switcher.add_listener(DebugListener);
    
    // Start monitoring
    switcher.start_monitoring(mtm)?;
    
    println!("Enhanced monitoring started!");
    println!("Switch apps to see events. Press Enter to stop...");
    
    // Wait for user input
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    
    switcher.stop_monitoring();
    println!("Test completed!");
    
    Ok(())
}