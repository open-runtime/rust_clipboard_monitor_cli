use accessibility_sys::*;
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::base::{TCFType, CFRelease, CFTypeRef};
use core_foundation_sys::base::{CFGetTypeID};
use core_foundation_sys::string::CFStringGetTypeID;
use std::ptr::null_mut;

fn main() {
    println!("Testing Safari URL extraction...");
    println!("Please switch to Safari within 3 seconds...");
    std::thread::sleep(std::time::Duration::from_secs(3));
    
    unsafe {
        let system = AXUIElementCreateSystemWide();
        
        // Get focused app
        let mut app_ref: CFTypeRef = null_mut();
        let attr = CFString::new("AXFocusedApplication");
        
        if AXUIElementCopyAttributeValue(system, attr.as_concrete_TypeRef(), &mut app_ref) == kAXErrorSuccess {
            let app = app_ref as AXUIElementRef;
            
            // Get app name
            let mut name_ref: CFTypeRef = null_mut();
            let name_attr = CFString::new("AXTitle");
            
            if AXUIElementCopyAttributeValue(app, name_attr.as_concrete_TypeRef(), &mut name_ref) == kAXErrorSuccess {
                let name_type = CFGetTypeID(name_ref);
                if name_type == CFStringGetTypeID() {
                    let name_str = name_ref as CFStringRef;
                    let name = CFString::wrap_under_get_rule(name_str).to_string();
                    println!("\n✅ Current app: {}", name);
                    
                    if name == "Safari" {
                        println!("🔍 Safari detected! Analyzing structure...\n");
                        
                        // Get focused window
                        let mut window_ref: CFTypeRef = null_mut();
                        let window_attr = CFString::new("AXFocusedWindow");
                        
                        if AXUIElementCopyAttributeValue(app, window_attr.as_concrete_TypeRef(), &mut window_ref) == kAXErrorSuccess {
                            let window = window_ref as AXUIElementRef;
                            
                            // Get window title
                            let title_attr = CFString::new("AXTitle");
                            let mut title_ref: CFTypeRef = null_mut();
                            if AXUIElementCopyAttributeValue(window, title_attr.as_concrete_TypeRef(), &mut title_ref) == kAXErrorSuccess {
                                if CFGetTypeID(title_ref) == CFStringGetTypeID() {
                                    let title = CFString::wrap_under_get_rule(title_ref as CFStringRef).to_string();
                                    println!("Window title: {}", title);
                                }
                                CFRelease(title_ref);
                            }
                            
                            // Try AXURL on window
                            let url_attr = CFString::new("AXURL");
                            let mut url_ref: CFTypeRef = null_mut();
                            if AXUIElementCopyAttributeValue(window, url_attr.as_concrete_TypeRef(), &mut url_ref) == kAXErrorSuccess {
                                if CFGetTypeID(url_ref) == CFStringGetTypeID() {
                                    let url = CFString::wrap_under_get_rule(url_ref as CFStringRef).to_string();
                                    println!("Window AXURL: {}", url);
                                }
                                CFRelease(url_ref);
                            } else {
                                println!("Window AXURL: not found");
                            }
                            
                            // Get focused element
                            let focused_attr = CFString::new("AXFocusedUIElement");
                            let mut focused_ref: CFTypeRef = null_mut();
                            
                            if AXUIElementCopyAttributeValue(app, focused_attr.as_concrete_TypeRef(), &mut focused_ref) == kAXErrorSuccess {
                                let focused = focused_ref as AXUIElementRef;
                                println!("\n📍 Focused element:");
                                
                                // Get role
                                let role_attr = CFString::new("AXRole");
                                let mut role_ref: CFTypeRef = null_mut();
                                if AXUIElementCopyAttributeValue(focused, role_attr.as_concrete_TypeRef(), &mut role_ref) == kAXErrorSuccess {
                                    if CFGetTypeID(role_ref) == CFStringGetTypeID() {
                                        let role = CFString::wrap_under_get_rule(role_ref as CFStringRef).to_string();
                                        println!("  Role: {}", role);
                                    }
                                    CFRelease(role_ref);
                                }
                                
                                // Get value
                                let value_attr = CFString::new("AXValue");
                                let mut value_ref: CFTypeRef = null_mut();
                                if AXUIElementCopyAttributeValue(focused, value_attr.as_concrete_TypeRef(), &mut value_ref) == kAXErrorSuccess {
                                    if CFGetTypeID(value_ref) == CFStringGetTypeID() {
                                        let value = CFString::wrap_under_get_rule(value_ref as CFStringRef).to_string();
                                        println!("  Value: {}", value);
                                        if value.starts_with("http") || value.contains("://") {
                                            println!("\n🎯 FOUND URL: {}", value);
                                        }
                                    }
                                    CFRelease(value_ref);
                                }
                                
                                // Get description
                                let desc_attr = CFString::new("AXDescription");
                                let mut desc_ref: CFTypeRef = null_mut();
                                if AXUIElementCopyAttributeValue(focused, desc_attr.as_concrete_TypeRef(), &mut desc_ref) == kAXErrorSuccess {
                                    if CFGetTypeID(desc_ref) == CFStringGetTypeID() {
                                        let desc = CFString::wrap_under_get_rule(desc_ref as CFStringRef).to_string();
                                        println!("  Description: {}", desc);
                                    }
                                    CFRelease(desc_ref);
                                }
                                
                                CFRelease(focused_ref);
                            }
                            
                            CFRelease(window_ref);
                        }
                    } else {
                        println!("❌ Not Safari - current app is: {}", name);
                        println!("Please switch to Safari and run again.");
                    }
                }
                CFRelease(name_ref);
            }
            CFRelease(app_ref);
        }
    }
}