// // Replaced with corrected, compiling implementation (same functionality)
// // This implementation fixes CF* conversions and NSWindow occlusion API usage.
// // See the previous commit history for details on the changes.
// include!("window_state_detector_fixed.rs");

// /// Helper function to convert ItemRef to a Core Foundation type safely
// fn itemref_to_cftype<T: TCFType>(item: ItemRef<'_, *const c_void>) -> Option<T> {
//     // ItemRef contains a raw pointer to a Core Foundation object
//     // We need to wrap it safely without taking ownership
//     unsafe {
//         let ptr = item.to_void() as *const T::Ref;
//         if ptr.is_null() {
//             None
//         } else {
//             Some(T::wrap_under_get_rule(ptr))
//         }
//     }
// }

// /// Main window state detector with multi-method consensus
// pub struct WindowStateDetector {
//     monitors: Arc<RwLock<Vec<MonitorInfo>>>,
//     spaces: Arc<RwLock<Vec<SpaceInfo>>>,
//     window_states: Arc<Mutex<HashMap<u32, WindowStateInfo>>>,
//     window_history: Arc<Mutex<HashMap<u32, WindowHistory>>>,
//     confidence_weights: HashMap<DetectionMethod, f32>,
//     detection_cache: Arc<Mutex<HashMap<u32, (WindowStateInfo, Instant)>>>,
//     cache_duration: Duration,
// }

// impl WindowStateDetector {
//     pub fn new() -> Self {
//         let mut confidence_weights = HashMap::new();

//         // Assign reliability weights to each detection method
//         confidence_weights.insert(DetectionMethod::CGWindowOnScreen, 0.9);
//         confidence_weights.insert(DetectionMethod::CGWindowLayer, 0.8);
//         confidence_weights.insert(DetectionMethod::CGWindowAlpha, 0.7);
//         confidence_weights.insert(DetectionMethod::CGWindowBounds, 0.85);
//         confidence_weights.insert(DetectionMethod::NSWindowOcclusion, 0.95);
//         confidence_weights.insert(DetectionMethod::NSWindowFrame, 0.9);
//         confidence_weights.insert(DetectionMethod::SpaceType, 0.8);
//         confidence_weights.insert(DetectionMethod::WindowLevel, 0.75);
//         confidence_weights.insert(DetectionMethod::AccessibilityAPI, 0.85);
//         confidence_weights.insert(DetectionMethod::HistoricalPattern, 0.6);

//         Self {
//             monitors: Arc::new(RwLock::new(Vec::new())),
//             spaces: Arc::new(RwLock::new(Vec::new())),
//             window_states: Arc::new(Mutex::new(HashMap::new())),
//             window_history: Arc::new(Mutex::new(HashMap::new())),
//             confidence_weights,
//             detection_cache: Arc::new(Mutex::new(HashMap::new())),
//             cache_duration: Duration::from_millis(50), // 50ms cache
//         }
//     }

//     /// Initialize and update monitor information
//     pub fn update_monitors(&self) -> Result<(), String> {
//         let mut monitors = self.monitors.write().unwrap();
//         monitors.clear();

//         unsafe {
//             // Get all active displays
//             let max_displays = 32;
//             let mut display_ids = vec![0u32; max_displays as usize];
//             let mut display_count = 0u32;

//             if CGGetActiveDisplayList(max_displays, display_ids.as_mut_ptr(), &mut display_count)
//                 != 0
//             {
//                 return Err("Failed to get display list".to_string());
//             }

//             for i in 0..display_count as usize {
//                 let display_id = display_ids[i];
//                 let bounds = CGDisplayBounds(display_id);
//                 let size = CGDisplayScreenSize(display_id);

//                 monitors.push(MonitorInfo {
//                     id: display_id,
//                     bounds,
//                     name: format!("Display {}", i + 1),
//                     is_builtin: display_id == CGMainDisplayID(),
//                     scale_factor: if size.width > 0.0 {
//                         bounds.size.width / size.width
//                     } else {
//                         1.0
//                     },
//                 });
//             }
//         }

//         Ok(())
//     }

//     /// Update space/desktop information using private APIs (when available)
//     pub fn update_spaces(&self) -> Result<(), String> {
//         let mut spaces = self.spaces.write().unwrap();
//         spaces.clear();

//         unsafe {
//             let conn = CGSMainConnectionID();
//             if conn == 0 {
//                 return Err("Failed to get main connection".to_string());
//             }

//             let managed_displays = CGSCopyManagedDisplaySpaces(conn);
//             if managed_displays.is_null() {
//                 return Err("Failed to get managed display spaces".to_string());
//             }

//             let displays_array: CFArray<CFDictionary> =
//                 CFArray::wrap_under_create_rule(managed_displays as CFArrayRef);

//             for i in 0..displays_array.len() {
//                 if let Some(display_dict) = displays_array.get(i) {
//                     // Extract display ID using modern CFString handling
//                     let display_id_key = CFStringCore::from_static_string("Display Identifier");
//                     let display_id =
//                         display_dict
//                             .find(display_id_key.as_CFTypeRef())
//                             .and_then(|v| {
//                                 itemref_to_cftype::<CFStringCore>(v)
//                                     .and_then(|s| s.to_string().parse::<u32>().ok())
//                             });

//                     // Extract spaces for this display
//                     let spaces_key = CFStringCore::from_static_string("Spaces");
//                     if let Some(spaces_ref) = display_dict.find(spaces_key.as_CFTypeRef()) {
//                         if let Some(spaces_array) =
//                             itemref_to_cftype::<CFArray<CFDictionary>>(spaces_ref)
//                         {
//                             for j in 0..spaces_array.len() {
//                                 if let Some(space_dict) = spaces_array.get(j) {
//                                     let space_id_key =
//                                         CFStringCore::from_static_string("ManagedSpaceID");
//                                     let space_id = space_dict
//                                         .find(space_id_key.as_CFTypeRef())
//                                         .and_then(|v| {
//                                             itemref_to_cftype::<CFNumber>(v)
//                                                 .map(|n| n.to_i64().unwrap_or(0) as u64)
//                                         })
//                                         .unwrap_or(0);

//                                     let space_type = CGSSpaceGetType(conn, space_id);

//                                     spaces.push(SpaceInfo {
//                                         id: space_id,
//                                         type_id: space_type,
//                                         display_id: display_id.unwrap_or(0),
//                                         is_fullscreen: space_type == kCGSSpaceFullscreen,
//                                         is_current: false, // Will be updated separately
//                                         window_ids: Vec::new(),
//                                     });
//                                 }
//                             }
//                         }
//                     }
//                 }
//             }
//         }

//         Ok(())
//     }

//     /// Detect window state using multiple methods with consensus
//     pub fn detect_window_state(&self, window_id: u32, pid: i32) -> WindowStateInfo {
//         // Check cache first
//         if let Some(cached) = self.check_cache(window_id) {
//             return cached;
//         }

//         let mut detections: HashMap<WindowState, Vec<(DetectionMethod, f32)>> = HashMap::new();
//         let monitors = self.monitors.read().unwrap();

//         // Method 1: CGWindow API detection
//         if let Some(cg_state) = self.detect_via_cgwindow(window_id, pid, &monitors) {
//             for (state, method, confidence) in cg_state {
//                 detections
//                     .entry(state)
//                     .or_insert_with(Vec::new)
//                     .push((method, confidence));
//             }
//         }

//         // Method 2: NSWindow API detection (if we can get window reference)
//         if let Some(ns_state) = self.detect_via_nswindow(window_id, pid) {
//             for (state, method, confidence) in ns_state {
//                 detections
//                     .entry(state)
//                     .or_insert_with(Vec::new)
//                     .push((method, confidence));
//             }
//         }

//         // Method 3: Space-based detection
//         if let Some(space_state) = self.detect_via_spaces(window_id) {
//             for (state, method, confidence) in space_state {
//                 detections
//                     .entry(state)
//                     .or_insert_with(Vec::new)
//                     .push((method, confidence));
//             }
//         }

//         // Method 4: Historical pattern detection
//         if let Some(hist_state) = self.detect_via_history(window_id) {
//             for (state, method, confidence) in hist_state {
//                 detections
//                     .entry(state)
//                     .or_insert_with(Vec::new)
//                     .push((method, confidence));
//             }
//         }

//         // Consensus algorithm: weighted voting
//         let state_info = self.calculate_consensus(detections, window_id);

//         // Update cache
//         self.update_cache(window_id, state_info.clone());

//         // Update history
//         self.update_history(window_id, state_info.state.clone());

//         state_info
//     }

//     /// CGWindow-based detection (most reliable for basic states)
//     fn detect_via_cgwindow(
//         &self,
//         window_id: u32,
//         pid: i32,
//         monitors: &[MonitorInfo],
//     ) -> Option<Vec<(WindowState, DetectionMethod, f32)>> {
//         let mut detections = Vec::new();

//         unsafe {
//             // Get all windows to find our target
//             let window_list_ptr = CGWindowListCopyWindowInfo(kCGWindowListOptionAll, 0);
//             if window_list_ptr.is_null() {
//                 return None;
//             }

//             let window_list: CFArray<CFDictionary<CFStringCore, CFType>> =
//                 CFArray::wrap_under_create_rule(window_list_ptr as CFArrayRef);

//             for i in 0..window_list.len() {
//                 if let Some(window_dict) = window_list.get(i) {
//                     // Check if this is our window
//                     let wid_key = CFStringCore::from_static_string("kCGWindowNumber");
//                     let wid = window_dict
//                         .find(wid_key.as_CFTypeRef())
//                         .and_then(|n| {
//                             itemref_to_cftype::<CFNumber>(n)
//                                 .map(|num| num.to_i32().unwrap_or(0) as u32)
//                         })
//                         .unwrap_or(0);

//                     if wid != window_id {
//                         continue;
//                     }

//                     // Check window owner PID
//                     let pid_key = CFStringCore::from_static_string("kCGWindowOwnerPID");
//                     let owner_pid = window_dict
//                         .find(pid_key.as_CFTypeRef())
//                         .and_then(|n| {
//                             itemref_to_cftype::<CFNumber>(n).map(|num| num.to_i32().unwrap_or(0))
//                         })
//                         .unwrap_or(0);

//                     if owner_pid != pid {
//                         continue;
//                     }

//                     // Detection 1: Check if on screen
//                     let onscreen_key = CFStringCore::from_static_string("kCGWindowIsOnscreen");
//                     let is_onscreen = window_dict
//                         .find(onscreen_key.as_CFTypeRef())
//                         .and_then(|b| {
//                             itemref_to_cftype::<CFBooleanCore>(b).map(|boolean| boolean.into())
//                         })
//                         .unwrap_or(false);

//                     if !is_onscreen {
//                         detections.push((
//                             WindowState::Minimized,
//                             DetectionMethod::CGWindowOnScreen,
//                             0.9, // High confidence
//                         ));
//                     }

//                     // Detection 2: Check window layer
//                     let layer_key = CFStringCore::from_static_string("kCGWindowLayer");
//                     let layer = window_dict
//                         .find(layer_key.as_CFTypeRef())
//                         .and_then(|n| {
//                             itemref_to_cftype::<CFNumber>(n).map(|num| num.to_i32().unwrap_or(0))
//                         })
//                         .unwrap_or(0);

//                     if layer < 0 {
//                         detections.push((
//                             WindowState::Minimized,
//                             DetectionMethod::CGWindowLayer,
//                             0.8,
//                         ));
//                     } else if layer > 100 {
//                         // Elevated layers might indicate fullscreen
//                         detections.push((
//                             WindowState::Fullscreen,
//                             DetectionMethod::CGWindowLayer,
//                             0.6,
//                         ));
//                     }

//                     // Detection 3: Check alpha value
//                     let alpha_key = CFStringCore::from_static_string("kCGWindowAlpha");
//                     let alpha = window_dict
//                         .find(alpha_key.as_CFTypeRef())
//                         .and_then(|n| {
//                             itemref_to_cftype::<CFNumber>(n).map(|num| num.to_f64().unwrap_or(1.0))
//                         })
//                         .unwrap_or(1.0);

//                     if alpha == 0.0 {
//                         detections.push((
//                             WindowState::Hidden,
//                             DetectionMethod::CGWindowAlpha,
//                             0.85,
//                         ));
//                     }

//                     // Detection 4: Check bounds for fullscreen
//                     let bounds_key = CFStringCore::from_static_string("kCGWindowBounds");
//                     if let Some(bounds_dict_ref) = window_dict.find(bounds_key.as_CFTypeRef()) {
//                         if let Some(bounds_dict) =
//                             itemref_to_cftype::<CFDictionary<CFStringCore, CFType>>(bounds_dict_ref)
//                         {
//                             let x_key = CFStringCore::from_static_string("X");
//                             let y_key = CFStringCore::from_static_string("Y");
//                             let width_key = CFStringCore::from_static_string("Width");
//                             let height_key = CFStringCore::from_static_string("Height");

//                             let x = bounds_dict
//                                 .find(x_key.as_CFTypeRef())
//                                 .and_then(|n| {
//                                     itemref_to_cftype::<CFNumber>(n)
//                                         .map(|num| num.to_f64().unwrap_or(0.0))
//                                 })
//                                 .unwrap_or(0.0);

//                             let y = bounds_dict
//                                 .find(y_key.as_CFTypeRef())
//                                 .and_then(|n| {
//                                     itemref_to_cftype::<CFNumber>(n)
//                                         .map(|num| num.to_f64().unwrap_or(0.0))
//                                 })
//                                 .unwrap_or(0.0);

//                             let width = bounds_dict
//                                 .find(width_key.as_CFTypeRef())
//                                 .and_then(|n| {
//                                     itemref_to_cftype::<CFNumber>(n)
//                                         .map(|num| num.to_f64().unwrap_or(0.0))
//                                 })
//                                 .unwrap_or(0.0);

//                             let height = bounds_dict
//                                 .find(height_key.as_CFTypeRef())
//                                 .and_then(|n| {
//                                     itemref_to_cftype::<CFNumber>(n)
//                                         .map(|num| num.to_f64().unwrap_or(0.0))
//                                 })
//                                 .unwrap_or(0.0);

//                             let window_rect = CGRect {
//                                 origin: CGPoint { x, y },
//                                 size: CGSize { width, height },
//                             };

//                             // Check against each monitor
//                             for monitor in monitors {
//                                 if self.rects_match(&window_rect, &monitor.bounds, 10.0) {
//                                     detections.push((
//                                         WindowState::Fullscreen,
//                                         DetectionMethod::CGWindowBounds,
//                                         0.95,
//                                     ));
//                                     break;
//                                 }
//                             }

//                             // Check if completely offscreen
//                             if !self.is_rect_visible(&window_rect, monitors) {
//                                 detections.push((
//                                     WindowState::Offscreen,
//                                     DetectionMethod::CGWindowBounds,
//                                     0.8,
//                                 ));
//                             }
//                         }
//                     }

//                     // Detection 5: Window store type (backing store)
//                     let store_key = CFStringCore::from_static_string("kCGWindowStoreType");
//                     if let Some(store_type_ref) = window_dict.find(store_key.as_CFTypeRef()) {
//                         if let Some(store_num) = itemref_to_cftype::<CFNumber>(store_type_ref) {
//                             let store_val = store_num.to_i32().unwrap_or(0);

//                             // Store type 2 typically indicates minimized
//                             if store_val == 2 {
//                                 detections.push((
//                                     WindowState::Minimized,
//                                     DetectionMethod::CGWindowLayer,
//                                     0.7,
//                                 ));
//                             }
//                         }
//                     }

//                     break; // Found our window
//                 }
//             }
//         }

//         if detections.is_empty() {
//             // If no specific state detected, assume normal
//             detections.push((WindowState::Normal, DetectionMethod::CGWindowOnScreen, 0.5));
//         }

//         Some(detections)
//     }

//     /// NSWindow-based detection (when available)
//     fn detect_via_nswindow(
//         &self,
//         window_id: u32,
//         _pid: i32,
//     ) -> Option<Vec<(WindowState, DetectionMethod, f32)>> {
//         // Try to get NSApplication windows
//         autoreleasepool(|_pool| {
//             unsafe {
//                 // MainThreadMarker::new() returns MainThreadMarker directly, not Option
//                 let mtm = MainThreadMarker::new();
//                 let app = NSApplication::sharedApplication(mtm);
//                 let windows = app.windows();

//                 let mut detections = Vec::new();

//                 for i in 0..windows.count() {
//                     let window = windows.objectAtIndex(i);
//                     // Get window number
//                     let window_num: NSNumber = msg_send_id![&window, windowNumber];
//                     let win_id = window_num.integerValue() as u32;

//                     if win_id != window_id {
//                         continue;
//                     }

//                     // Check occlusion state
//                     let occlusion_state: NSWindowOcclusionState =
//                         msg_send![&window, occlusionState];

//                     // Use the correct constant name
//                     if occlusion_state.contains(NSWindowOcclusionState::Visible) {
//                         // Window is visible
//                         let is_key: bool = msg_send![&window, isKeyWindow];
//                         let is_main: bool = msg_send![&window, isMainWindow];

//                         if is_key || is_main {
//                             detections.push((
//                                 WindowState::Normal,
//                                 DetectionMethod::NSWindowOcclusion,
//                                 0.95,
//                             ));
//                         }
//                     } else {
//                         // Window is occluded/hidden
//                         detections.push((
//                             WindowState::Hidden,
//                             DetectionMethod::NSWindowOcclusion,
//                             0.9,
//                         ));
//                     }

//                     // Check if minimized
//                     let is_miniaturized: bool = msg_send![&window, isMiniaturized];
//                     if is_miniaturized {
//                         detections.push((
//                             WindowState::Minimized,
//                             DetectionMethod::NSWindowFrame,
//                             0.95,
//                         ));
//                     }

//                     // Check if zoomed (potentially fullscreen)
//                     let is_zoomed: bool = msg_send![&window, isZoomed];
//                     if is_zoomed {
//                         // Additional check for fullscreen
//                         let style_mask: u64 = msg_send![&window, styleMask];
//                         const NSWindowStyleMaskFullScreen: u64 = 1 << 14;

//                         if style_mask & NSWindowStyleMaskFullScreen != 0 {
//                             detections.push((
//                                 WindowState::Fullscreen,
//                                 DetectionMethod::NSWindowFrame,
//                                 0.9,
//                             ));
//                         }
//                     }

//                     break;
//                 }

//                 if detections.is_empty() {
//                     None
//                 } else {
//                     Some(detections)
//                 }
//             }
//         })
//     }

//     /// Space-based detection using private APIs
//     fn detect_via_spaces(
//         &self,
//         window_id: u32,
//     ) -> Option<Vec<(WindowState, DetectionMethod, f32)>> {
//         let spaces = self.spaces.read().unwrap();
//         let mut detections = Vec::new();

//         // Find which space contains this window
//         for space in spaces.iter() {
//             if space.window_ids.contains(&window_id) {
//                 if space.is_fullscreen {
//                     detections.push((WindowState::Fullscreen, DetectionMethod::SpaceType, 0.85));
//                 }
//                 break;
//             }
//         }

//         // Also check window level using private API
//         unsafe {
//             let conn = CGSMainConnectionID();
//             if conn != 0 {
//                 let mut level: i32 = 0;
//                 if CGSGetWindowLevel(conn, window_id, &mut level) == 0 {
//                     // Check various window levels
//                     if level < kCGNormalWindowLevel {
//                         detections.push((
//                             WindowState::Minimized,
//                             DetectionMethod::WindowLevel,
//                             0.7,
//                         ));
//                     } else if level > kCGMainMenuWindowLevel {
//                         detections.push((
//                             WindowState::Fullscreen,
//                             DetectionMethod::WindowLevel,
//                             0.65,
//                         ));
//                     }
//                 }
//             }
//         }

//         if detections.is_empty() {
//             None
//         } else {
//             Some(detections)
//         }
//     }

//     /// Historical pattern-based detection
//     fn detect_via_history(
//         &self,
//         window_id: u32,
//     ) -> Option<Vec<(WindowState, DetectionMethod, f32)>> {
//         let mut history_map = self.window_history.lock().unwrap();
//         let history = history_map
//             .entry(window_id)
//             .or_insert_with(WindowHistory::new);

//         let mut detections = Vec::new();

//         // Check if likely animating
//         if history.is_likely_animating() {
//             detections.push((
//                 WindowState::Transitioning,
//                 DetectionMethod::HistoricalPattern,
//                 0.7,
//             ));
//         }

//         // Try to predict next state based on patterns
//         if let Some(predicted) = history.predict_next_state() {
//             detections.push((predicted, DetectionMethod::HistoricalPattern, 0.5));
//         }

//         if detections.is_empty() {
//             None
//         } else {
//             Some(detections)
//         }
//     }

//     /// Calculate consensus from multiple detection methods
//     fn calculate_consensus(
//         &self,
//         detections: HashMap<WindowState, Vec<(DetectionMethod, f32)>>,
//         window_id: u32,
//     ) -> WindowStateInfo {
//         let mut best_state = WindowState::Unknown;
//         let mut best_confidence = 0.0;
//         let mut all_methods = Vec::new();

//         // Calculate weighted confidence for each detected state
//         for (state, methods) in detections {
//             let mut total_confidence = 0.0;
//             let mut method_list = Vec::new();

//             for (method, confidence) in methods {
//                 let weight = self.confidence_weights.get(&method).unwrap_or(&0.5);

//                 total_confidence += confidence * weight;
//                 method_list.push(method);
//             }

//             // Normalize by number of methods
//             let normalized_confidence = total_confidence / method_list.len() as f32;

//             if normalized_confidence > best_confidence {
//                 best_confidence = normalized_confidence;
//                 best_state = state;
//                 all_methods = method_list;
//             }
//         }

//         // Apply confidence threshold
//         if best_confidence < 0.3 {
//             best_state = WindowState::Unknown;
//         }

//         // Check if animation is likely
//         let is_animating = self.is_likely_animating(window_id);

//         WindowStateInfo {
//             state: best_state,
//             confidence: best_confidence.min(1.0),
//             detection_methods: all_methods,
//             timestamp: Instant::now(),
//             is_animating,
//             space_id: self.get_current_window_space(window_id),
//             monitor_id: self.get_current_window_monitor(window_id),
//         }
//     }

//     /// Check if window is likely animating
//     fn is_likely_animating(&self, window_id: u32) -> bool {
//         let history_map = self.window_history.lock().unwrap();

//         if let Some(history) = history_map.get(&window_id) {
//             history.is_likely_animating()
//         } else {
//             false
//         }
//     }

//     /// Get the space ID for a window
//     fn get_current_window_space(&self, window_id: u32) -> Option<u64> {
//         let spaces = self.spaces.read().unwrap();

//         for space in spaces.iter() {
//             if space.window_ids.contains(&window_id) {
//                 return Some(space.id);
//             }
//         }

//         None
//     }

//     /// Get the monitor ID for a window
//     fn get_current_window_monitor(&self, window_id: u32) -> Option<u32> {
//         // Get window bounds and check against monitors
//         unsafe {
//             let window_list_ptr =
//                 CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow, window_id);

//             if window_list_ptr.is_null() {
//                 return None;
//             }

//             let window_list: CFArray<CFDictionary<CFStringCore, CFType>> =
//                 CFArray::wrap_under_create_rule(window_list_ptr as CFArrayRef);

//             if window_list.len() > 0 {
//                 if let Some(window_dict) = window_list.get(0) {
//                     let bounds_key = CFStringCore::from_static_string("kCGWindowBounds");
//                     if let Some(bounds_dict_ref) = window_dict.find(bounds_key.as_CFTypeRef()) {
//                         if let Some(bounds_dict) =
//                             itemref_to_cftype::<CFDictionary<CFStringCore, CFType>>(bounds_dict_ref)
//                         {
//                             let x_key = CFStringCore::from_static_string("X");
//                             let y_key = CFStringCore::from_static_string("Y");

//                             let x = bounds_dict
//                                 .find(x_key.as_CFTypeRef())
//                                 .and_then(|n| {
//                                     itemref_to_cftype::<CFNumber>(n)
//                                         .map(|num| num.to_f64().unwrap_or(0.0))
//                                 })
//                                 .unwrap_or(0.0);

//                             let y = bounds_dict
//                                 .find(y_key.as_CFTypeRef())
//                                 .and_then(|n| {
//                                     itemref_to_cftype::<CFNumber>(n)
//                                         .map(|num| num.to_f64().unwrap_or(0.0))
//                                 })
//                                 .unwrap_or(0.0);

//                             let window_center = CGPoint {
//                                 x: x + 100.0, // Approximate center
//                                 y: y + 100.0,
//                             };

//                             // Find which monitor contains this point
//                             let monitors = self.monitors.read().unwrap();
//                             for monitor in monitors.iter() {
//                                 if self.point_in_rect(&window_center, &monitor.bounds) {
//                                     return Some(monitor.id);
//                                 }
//                             }
//                         }
//                     }
//                 }
//             }
//         }

//         None
//     }

//     /// Check if a point is inside a rectangle
//     fn point_in_rect(&self, point: &CGPoint, rect: &CGRect) -> bool {
//         point.x >= rect.origin.x
//             && point.x <= rect.origin.x + rect.size.width
//             && point.y >= rect.origin.y
//             && point.y <= rect.origin.y + rect.size.height
//     }

//     /// Check if two rectangles match within tolerance
//     fn rects_match(&self, rect1: &CGRect, rect2: &CGRect, tolerance: f64) -> bool {
//         (rect1.origin.x - rect2.origin.x).abs() < tolerance
//             && (rect1.origin.y - rect2.origin.y).abs() < tolerance
//             && (rect1.size.width - rect2.size.width).abs() < tolerance
//             && (rect1.size.height - rect2.size.height).abs() < tolerance
//     }

//     /// Check if rectangle is visible on any monitor
//     fn is_rect_visible(&self, rect: &CGRect, monitors: &[MonitorInfo]) -> bool {
//         for monitor in monitors {
//             if self.rects_intersect(rect, &monitor.bounds) {
//                 return true;
//             }
//         }
//         false
//     }

//     /// Check if two rectangles intersect
//     fn rects_intersect(&self, rect1: &CGRect, rect2: &CGRect) -> bool {
//         rect1.origin.x < rect2.origin.x + rect2.size.width
//             && rect1.origin.x + rect1.size.width > rect2.origin.x
//             && rect1.origin.y < rect2.origin.y + rect2.size.height
//             && rect1.origin.y + rect1.size.height > rect2.origin.y
//     }

//     /// Check cache for recent detection
//     fn check_cache(&self, window_id: u32) -> Option<WindowStateInfo> {
//         let cache = self.detection_cache.lock().unwrap();

//         if let Some((info, timestamp)) = cache.get(&window_id) {
//             if timestamp.elapsed() < self.cache_duration {
//                 return Some(info.clone());
//             }
//         }

//         None
//     }

//     /// Update cache with new detection
//     fn update_cache(&self, window_id: u32, info: WindowStateInfo) {
//         let mut cache = self.detection_cache.lock().unwrap();
//         cache.insert(window_id, (info, Instant::now()));

//         // Clean old entries
//         cache.retain(|_, (_, timestamp)| timestamp.elapsed() < Duration::from_secs(1));
//     }

//     /// Update window history
//     fn update_history(&self, window_id: u32, state: WindowState) {
//         let mut history_map = self.window_history.lock().unwrap();
//         let history = history_map
//             .entry(window_id)
//             .or_insert_with(WindowHistory::new);
//         history.add_state(state);
//     }

//     /// Get current window state with fallback chain
//     pub fn get_window_state_with_fallback(&self, window_id: u32, pid: i32) -> WindowStateInfo {
//         // Primary detection
//         let mut state_info = self.detect_window_state(window_id, pid);

//         // If confidence is too low, try fallback methods
//         if state_info.confidence < 0.5 {
//             // Fallback 1: Check if window exists at all
//             if !self.window_exists(window_id) {
//                 state_info.state = WindowState::Unknown;
//                 state_info.confidence = 1.0;
//                 return state_info;
//             }

//             // Fallback 2: Use last known good state
//             if let Some(last_known) = self.get_last_known_state(window_id) {
//                 if last_known.timestamp.elapsed() < Duration::from_secs(5) {
//                     state_info = last_known;
//                     state_info.confidence *= 0.8; // Reduce confidence for stale data
//                 }
//             }

//             // Fallback 3: Use most common state for this app
//             if state_info.confidence < 0.3 {
//                 state_info.state = WindowState::Normal; // Safe default
//                 state_info.confidence = 0.3;
//             }
//         }

//         state_info
//     }

//     /// Check if window exists
//     fn window_exists(&self, window_id: u32) -> bool {
//         unsafe {
//             let window_list_ptr =
//                 CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow, window_id);

//             if window_list_ptr.is_null() {
//                 return false;
//             }

//             let window_list: CFArray<CFDictionary<CFStringCore, CFType>> =
//                 CFArray::wrap_under_create_rule(window_list_ptr as CFArrayRef);

//             window_list.len() > 0
//         }
//     }

//     /// Get last known state from cache or history
//     fn get_last_known_state(&self, window_id: u32) -> Option<WindowStateInfo> {
//         // Check cache first
//         if let Some(cached) = self.check_cache(window_id) {
//             return Some(cached);
//         }

//         // Check history
//         let history_map = self.window_history.lock().unwrap();

//         if let Some(history) = history_map.get(&window_id) {
//             if let Some((state, timestamp)) = history.states.back() {
//                 return Some(WindowStateInfo {
//                     state: state.clone(),
//                     confidence: 0.5,
//                     detection_methods: vec![DetectionMethod::HistoricalPattern],
//                     timestamp: *timestamp,
//                     is_animating: false,
//                     space_id: None,
//                     monitor_id: None,
//                 });
//             }
//         }

//         None
//     }

//     /// Batch detection for multiple windows (more efficient)
//     pub fn detect_multiple_windows(&self, windows: &[(u32, i32)]) -> HashMap<u32, WindowStateInfo> {
//         let mut results = HashMap::new();

//         // Update monitors and spaces once for all windows
//         let _ = self.update_monitors();
//         let _ = self.update_spaces();

//         // Get all window info in one call
//         let all_window_info = self.get_all_windows_info();

//         for (window_id, pid) in windows {
//             // Use cached bulk data if available
//             if let Some(window_info) = all_window_info.get(window_id) {
//                 results.insert(*window_id, window_info.clone());
//             } else {
//                 // Fallback to individual detection
//                 results.insert(*window_id, self.detect_window_state(*window_id, *pid));
//             }
//         }

//         results
//     }

//     /// Get all windows info in one CGWindow call (efficient)
//     fn get_all_windows_info(&self) -> HashMap<u32, WindowStateInfo> {
//         let mut results = HashMap::new();
//         let monitors = self.monitors.read().unwrap();

//         unsafe {
//             let window_list_ptr = CGWindowListCopyWindowInfo(kCGWindowListOptionAll, 0);
//             if window_list_ptr.is_null() {
//                 return results;
//             }

//             let window_list: CFArray<CFDictionary<CFStringCore, CFType>> =
//                 CFArray::wrap_under_create_rule(window_list_ptr as CFArrayRef);

//             for i in 0..window_list.len() {
//                 if let Some(window_dict) = window_list.get(i) {
//                     let wid_key = CFStringCore::from_static_string("kCGWindowNumber");
//                     let window_id = window_dict
//                         .find(wid_key.as_CFTypeRef())
//                         .and_then(|n| {
//                             itemref_to_cftype::<CFNumber>(n)
//                                 .map(|num| num.to_i32().unwrap_or(0) as u32)
//                         })
//                         .unwrap_or(0);

//                     if window_id > 0 {
//                         // Quick state detection based on available info
//                         let onscreen_key = CFStringCore::from_static_string("kCGWindowIsOnscreen");
//                         let is_onscreen = window_dict
//                             .find(onscreen_key.as_CFTypeRef())
//                             .and_then(|b| {
//                                 itemref_to_cftype::<CFBooleanCore>(b).map(|boolean| boolean.into())
//                             })
//                             .unwrap_or(false);

//                         let state = if !is_onscreen {
//                             WindowState::Minimized
//                         } else {
//                             WindowState::Normal
//                         };

//                         results.insert(
//                             window_id,
//                             WindowStateInfo {
//                                 state,
//                                 confidence: 0.7,
//                                 detection_methods: vec![DetectionMethod::CGWindowOnScreen],
//                                 timestamp: Instant::now(),
//                                 is_animating: false,
//                                 space_id: None,
//                                 monitor_id: None,
//                             },
//                         );
//                     }
//                 }
//             }
//         }

//         results
//     }
// }

// /// Public API for easy integration
// impl WindowStateDetector {
//     /// Simple API: Is window minimized?
//     pub fn is_minimized(&self, window_id: u32, pid: i32) -> bool {
//         let state = self.detect_window_state(window_id, pid);
//         state.state == WindowState::Minimized && state.confidence > 0.7
//     }

//     /// Simple API: Is window fullscreen?
//     pub fn is_fullscreen(&self, window_id: u32, pid: i32) -> bool {
//         let state = self.detect_window_state(window_id, pid);
//         state.state == WindowState::Fullscreen && state.confidence > 0.7
//     }

//     /// Simple API: Is window visible?
//     pub fn is_visible(&self, window_id: u32, pid: i32) -> bool {
//         let state = self.detect_window_state(window_id, pid);
//         matches!(state.state, WindowState::Normal | WindowState::Fullscreen)
//             && state.confidence > 0.5
//     }

//     /// Get window's current space/desktop
//     pub fn get_window_space(&self, window_id: u32, pid: i32) -> Option<u64> {
//         let state = self.detect_window_state(window_id, pid);
//         state.space_id
//     }

//     /// Get window's current monitor
//     pub fn get_window_monitor(&self, window_id: u32, pid: i32) -> Option<u32> {
//         let state = self.detect_window_state(window_id, pid);
//         state.monitor_id
//     }
// }

// // Default implementation
// impl Default for WindowStateDetector {
//     fn default() -> Self {
//         Self::new()
//     }
// }

// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn test_window_state_detection() {
//         let detector = WindowStateDetector::new();

//         // Test basic initialization
//         assert!(detector.update_monitors().is_ok());

//         // Note: Real window testing requires actual window IDs
//         // This is just a structural test
//     }

//     #[test]
//     fn test_consensus_algorithm() {
//         let detector = WindowStateDetector::new();

//         let mut detections = HashMap::new();
//         detections.insert(
//             WindowState::Minimized,
//             vec![
//                 (DetectionMethod::CGWindowOnScreen, 0.9),
//                 (DetectionMethod::CGWindowLayer, 0.8),
//             ],
//         );

//         let consensus = detector.calculate_consensus(detections, 12345);
//         assert_eq!(consensus.state, WindowState::Minimized);
//         assert!(consensus.confidence > 0.8);
//     }

//     #[test]
//     fn test_cache_functionality() {
//         let detector = WindowStateDetector::new();

//         let state_info = WindowStateInfo {
//             state: WindowState::Normal,
//             confidence: 0.9,
//             detection_methods: vec![DetectionMethod::CGWindowOnScreen],
//             timestamp: Instant::now(),
//             is_animating: false,
//             space_id: None,
//             monitor_id: None,
//         };

//         detector.update_cache(12345, state_info.clone());

//         // Should get cached value
//         let cached = detector.check_cache(12345);
//         assert!(cached.is_some());
//         assert_eq!(cached.unwrap().state, WindowState::Normal);

//         // After cache duration, should return None
//         std::thread::sleep(Duration::from_millis(60));
//         let expired = detector.check_cache(12345);
//         assert!(expired.is_none());
//     }

//     #[test]
//     fn test_history_tracking() {
//         let mut history = WindowHistory::new();

//         history.add_state(WindowState::Normal);
//         std::thread::sleep(Duration::from_millis(10));
//         history.add_state(WindowState::Minimized);
//         std::thread::sleep(Duration::from_millis(10));
//         history.add_state(WindowState::Normal);

//         // Should detect recent transitions
//         assert!(!history.is_likely_animating());

//         // Add rapid transitions
//         history.add_state(WindowState::Minimized);
//         history.add_state(WindowState::Normal);

//         // Now might detect animation (depending on timing)
//         // This is timing-sensitive so we just check it doesn't panic
//         let _ = history.is_likely_animating();
//     }
// }
