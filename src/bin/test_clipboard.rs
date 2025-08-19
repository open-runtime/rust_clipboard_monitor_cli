// src/bin/test_clipboard.rs
//! Test binary to demonstrate comprehensive clipboard monitoring capabilities

use research_assistant_tracker::test_comprehensive_clipboard_monitoring;
use anyhow::Result;

fn main() -> Result<()> {
    println!("🚀 CLIPBOARD MONITORING PROOF-OF-CONCEPT");
    println!("========================================");
    
    // Test comprehensive clipboard monitoring
    test_comprehensive_clipboard_monitoring()?;
    
    Ok(())
}