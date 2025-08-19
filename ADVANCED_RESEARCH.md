Based on my comprehensive research into macOS APIs for building artificial memory systems with maximum context ingestion, here's a complete report on all available APIs categorized by their capabilities:

# macOS Context Ingestion APIs Report

## **1. Core Event Monitoring APIs**

### **CGEvent / Quartz Event Services**[1][2][3]
- **Primary Use**: Global system-wide keyboard, mouse, and scroll event monitoring
- **Key Capabilities**:
  - Monitor all keyboard input system-wide with `CGEventTapCreate`
  - Track mouse movements, clicks, and scroll events globally
  - Detect modifier key combinations and states
  - Intercept events before they reach target applications
  - Works across all applications and system contexts

```swift
// Global keyboard and mouse monitoring
let eventMask = (1  Unmanaged? in
        // Process all system events here
        return Unmanaged.passRetained(event)
    },
    userInfo: nil
)
```

### **NSEvent Global Monitoring**[4]
- **Primary Use**: Application-level global event monitoring
- **Key Capabilities**:
  - Monitor global keyboard/mouse events when app is in background
  - Detect hot key combinations and global shortcuts
  - Track mouse movements and clicks system-wide
  - Monitor scroll events and trackpad gestures

```swift
// Global mouse and keyboard monitoring
NSEvent.addGlobalMonitorForEvents(matching: [.keyDown, .mouseMoved, .scrollWheel]) { event in
    // Process global events
    print("Event type: \(event.type), Location: \(event.locationInWindow)")
}
```

### **IOKit HID Manager**[5][6]
- **Primary Use**: Low-level hardware input device monitoring
- **Key Capabilities**:
  - Direct access to keyboard input at hardware level
  - Monitor USB device connections/disconnections
  - Track specific device vendor/product IDs
  - Bypass some accessibility permission requirements
  - Works even when other event monitoring fails

```swift
// Hardware-level keyboard monitoring
let manager = IOHIDManagerCreate(kCFAllocatorDefault, 0)
let matchingDict = IOServiceMatching(kIOHIDDeviceKey)
IOHIDManagerSetDeviceMatching(manager, matchingDict as CFDictionary)
IOHIDManagerRegisterInputValueCallback(manager, inputValueCallback, nil)
```

## **2. Screen and Visual Context APIs**

### **ScreenCaptureKit**[7][8][9][10]
- **Primary Use**: High-performance screen content capture and monitoring
- **Key Capabilities**:
  - Capture full displays or specific windows at native resolution
  - Real-time video frame streaming with hardware acceleration
  - Monitor screen content changes and window focus changes
  - Built-in system picker for content selection
  - Screenshot API for high-definition captures
  - Presenter overlay integration

```swift
// Screen content monitoring with change detection
let filter = SCContentFilter(display: display, excludingWindows: [])
let configuration = SCStreamConfiguration()
let stream = SCStream(filter: filter, configuration: configuration, delegate: self)

func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
    // Process each frame change for context analysis
}
```

### **CGDisplayStream**
- **Primary Use**: Lower-level display capture for frame analysis
- **Key Capabilities**:
  - Capture display frames with custom handlers
  - Monitor pixel-level changes on screen
  - Real-time frame processing for visual context analysis

### **Metal/MetalKit Screen Capture**[11][12][13]
- **Primary Use**: GPU-accelerated visual processing and capture
- **Key Capabilities**:
  - Direct GPU access to screen textures
  - Real-time visual processing and analysis
  - Efficient frame buffer manipulation

## **3. Window and Application Context APIs**

### **NSWorkspace**[14][15][16]
- **Primary Use**: Application lifecycle and focus monitoring
- **Key Capabilities**:
  - Monitor frontmost application changes
  - Track application launches and terminations
  - Get running applications with user interfaces
  - Detect workspace changes and user switches

```swift
// Monitor application focus changes
NSWorkspace.shared.notificationCenter.addObserver(
    self,
    selector: #selector(appDidActivate),
    name: NSWorkspace.didActivateApplicationNotification,
    object: nil
)
```

### **CGWindowListCopyWindowInfo**[17][18]
- **Primary Use**: Comprehensive window metadata extraction
- **Key Capabilities**:
  - Get all window information without accessibility requirements
  - Extract window bounds, ownership, and layering information
  - Monitor window creation/destruction system-wide
  - Track window positioning and sizing changes

```swift
// Get comprehensive window information
let windowList = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID)
for windowDict in windowList as! [[String: Any]] {
    let ownerName = windowDict[kCGWindowOwnerName] as? String
    let bounds = windowDict[kCGWindowBounds] as? [String: Any]
    // Extract complete window context
}
```

## **4. Accessibility and UI Context APIs**

### **AXUIElement/Accessibility API**[19][20][9][21][22]
- **Primary Use**: Deep UI hierarchy traversal and content extraction
- **Key Capabilities**:
  - Complete accessibility tree traversal for any application
  - Extract text content, UI labels, and form data
  - Monitor focused elements and user interactions
  - Get browser URLs and web content
  - Extract document paths and file information
  - Monitor UI changes with AXObserver notifications

```swift
// Deep accessibility tree traversal
func traverseAccessibilityHierarchy(element: AXUIElement) -> [String: Any] {
    var context: [String: Any] = [:]
    
    // Extract all UI element properties
    var role: CFTypeRef?
    AXUIElementCopyAttributeValue(element, kAXRoleAttribute as CFString, &role)
    
    var value: CFTypeRef?
    AXUIElementCopyAttributeValue(element, kAXValueAttribute as CFString, &value)
    
    var children: CFTypeRef?
    AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children)
    
    // Recursively traverse all child elements
    if let childrenArray = children as? [AXUIElement] {
        for child in childrenArray {
            let childContext = traverseAccessibilityHierarchy(element: child)
            // Merge child context
        }
    }
    
    return context
}
```

### **AXObserver Notifications**[23][19]
- **Primary Use**: Real-time UI change monitoring
- **Key Capabilities**:
  - Monitor focused window changes across all apps
  - Track text field focus and content changes
  - Detect UI element creation and destruction
  - Monitor scroll events and content changes

```swift
// Monitor UI changes system-wide
var observer: AXObserver?
AXObserverCreate(pid, { (observer, element, notification, userData) in
    switch notification {
    case kAXFocusedWindowChangedNotification:
        // Handle window focus change
    case kAXValueChangedNotification:
        // Handle text/value changes
    case kAXScrolledNotification:
        // Handle scroll events for re-analysis
    }
}, &observer)
```

## **5. System-Wide Notification APIs**

### **NSDistributedNotificationCenter**[24][25]
- **Primary Use**: Inter-process notification monitoring
- **Key Capabilities**:
  - Monitor system-wide distributed notifications
  - Track preference changes and system events
  - Detect application-specific notifications
  - Cross-process communication monitoring

```swift
// Monitor system-wide notifications
DistributedNotificationCenter.default().addObserver(
    self,
    selector: #selector(handleDistributedNotification(_:)),
    name: nil, // Monitor all distributed notifications
    object: nil
)
```

### **Darwin Notifications**[26]
- **Primary Use**: Low-level system notifications
- **Key Capabilities**:
  - True global notifications across all execution contexts
  - Works between GUI and daemon processes
  - System-level event monitoring

## **6. File System and Document Context APIs**

### **Core Data with Persistent History Tracking**[27][28]
- **Primary Use**: Data change monitoring and context persistence
- **Key Capabilities**:
  - Monitor all database changes across processes
  - Track document modifications and access patterns
  - Maintain historical context of data changes
  - Cross-process data synchronization

### **Document Path Extraction**[29][30]
- **Primary Use**: Active document context extraction
- **Key Capabilities**:
  - Extract file paths of currently open documents
  - Monitor document switches and saves
  - Track file access patterns across applications

## **7. Advanced Context Analysis APIs**

### **Idle Time Detection**[31]
- **Primary Use**: User activity state monitoring
- **Key Capabilities**:
  - Detect when user is away from computer
  - Monitor interaction patterns and activity levels
  - Determine context relevance based on user presence

### **Browser URL Extraction**[21][22][32]
- **Primary Use**: Web context monitoring
- **Key Capabilities**:
  - Extract URLs from Safari, Chrome, and other browsers
  - Monitor web navigation and content changes
  - Track web application states and interactions

## **Implementation Strategy for Maximum Context Ingestion**

### **Multi-API Integration Approach**
```swift
class ContextIngestionSystem {
    // Event monitoring
    private var eventTap: CFMachPort?
    private var globalEventMonitor: Any?
    private var hidManager: IOHIDManagerRef?
    
    // Screen monitoring
    private var screenCaptureStream: SCStream?
    private var displayStream: CGDisplayStream?
    
    // Application monitoring
    private var workspaceObserver: NSObjectProtocol?
    private var accessibilityObservers: [pid_t: AXObserver] = [:]
    
    // System monitoring
    private var distributedNotificationObserver: NSObjectProtocol?
    
    func startContextIngestion() {
        setupEventMonitoring()      // All keyboard/mouse input
        setupScreenMonitoring()     // Visual changes and content
        setupApplicationMonitoring() // App focus and window changes
        setupAccessibilityMonitoring() // UI content and interactions
        setupSystemMonitoring()     // System-wide notifications
        setupScrollDetection()      // Scroll events for re-analysis
    }
    
    func processContextChange(type: ContextChangeType, data: Any) {
        // Trigger re-analysis based on context change
        switch type {
        case .appFocusChange, .windowFocusChange, .scrollEvent:
            // Re-analyze current context
            performFullContextAnalysis()
        case .textContentChange, .documentChange:
            // Incremental context update
            updateIncrementalContext(data)
        }
    }
}
```

### **Key Requirements for Maximum Context Extraction**

1. **Permissions Required**:
   - Accessibility permissions (for AXUIElement access)
   - Screen Recording permissions (for ScreenCaptureKit)
   - Input Monitoring permissions (for global event monitoring)
   - Full Disk Access (for comprehensive file monitoring)

2. **Architecture Considerations**:
   - Disable sandboxing for full API access
   - Use background queues for intensive processing
   - Implement efficient caching and incremental updates
   - Handle permission failures gracefully with fallbacks

3. **Performance Optimization**:
   - Use hardware-accelerated screen capture when possible
   - Implement intelligent throttling for high-frequency events
   - Cache accessibility tree traversals
   - Process context changes asynchronously

This comprehensive API suite provides maximum possible context ingestion capability on macOS, allowing your artificial memory system to understand exactly what the user is doing, viewing, and interacting with across all applications and system contexts.

[1] https://github.com/usagimaru/EventTapper
[2] https://leopard-adc.pepas.com/documentation/Carbon/Reference/QuartzEventServicesRef/QuartzEventServicesRef.pdf
[3] https://stackoverflow.com/questions/60079741/macos-quartz-event-tap-listening-to-wrong-events
[4] https://www.reddit.com/r/macprogramming/comments/foy6vd/how_to_make_a_macos_swift_app_run_even_when_not/
[5] https://stackoverflow.com/questions/78905127/not-able-to-detect-trackpad-events-on-macos-using-iokit-hidmanager-iohidmanage
[6] http://theevilbit.blogspot.com/2019/02/macos-keylogging-through-hid-device.html
[7] https://developer.apple.com/videos/play/wwdc2022/10155/
[8] https://developer.apple.com/videos/play/wwdc2022/10156/
[9] https://stackoverflow.com/questions/77896841/screencapturekit-delivers-incorrect-frames-on-macos-after-move-focus-to-next-wi
[10] https://developer.apple.com/videos/play/wwdc2023/10136/
[11] https://stackoverflow.com/questions/33844130/take-a-snapshot-of-current-screen-with-metal-in-swift
[12] https://img.ly/blog/build-a-simple-real-time-video-editor-with-metal-for-ios/
[13] https://metalbyexample.com/first-look-at-metalkit/
[14] https://www.reddit.com/r/macosprogramming/comments/1b6myqh/how_can_i_get_notified_about_systemwide_window/
[15] https://gertrude.app/blog/querying-running-applications-in-macos
[16] https://stackoverflow.com/questions/20054662/how-to-get-list-of-all-applications-currently-running-and-visible-in-dock-for-ma
[17] https://stackoverflow.com/questions/30336740/how-to-get-window-list-from-core-grapics-api-with-swift
[18] https://gist.github.com/dedeexe/3cd8ccf760125d692e2eec574269e46d
[19] https://stackoverflow.com/questions/66158075/how-to-use-axobserver-in-swift
[20] https://www.youtube.com/watch?v=-xFJJdi07Ng
[21] https://stackoverflow.com/questions/71461990/how-to-access-a-browsers-url-or-an-apps-file-using-axuielementcopyattributeval
[22] https://stackoverflow.com/questions/53229924/how-to-retrieve-active-window-url-using-mac-os-x-accessibility-api
[23] https://www.reddit.com/r/swift/comments/18k909w/i_hit_a_dead_end_with_accessibility_apis/
[24] https://stackoverflow.com/questions/45593529/observe-for-new-system-notifications-osx
[25] https://objective-see.org/blog/blog_0x39.html
[26] https://developer.apple.com/forums/thread/750543
[27] https://fatbobman.com/en/posts/mastering-data-tracking-and-notifications-in-core-data-and-swiftdata/
[28] https://www.avanderlee.com/swift/persistent-history-tracking-core-data/
[29] https://stackoverflow.com/questions/49076103/swift-get-file-path-of-currently-opened-document-in-another-application
[30] https://stackoverflow.com/questions/49076103/swift-get-file-path-of-currently-opened-document-in-another-application/49076860
[31] https://xs-labs.com/en/archives/articles/iokit-idle-time/
[32] https://www.reddit.com/r/swift/comments/1637mok/getting_the_active_google_chrome_url_from_swift/
[33] https://stackoverflow.com/questions/49413972/cgdisplaystream-only-capturing-a-single-frame
[34] https://www.youtube.com/watch?v=mIztoF9CzP8
[35] https://gist.github.com/stephancasas/fd27ebcd2a0e36f3e3f00109d70abcdc
[36] https://www.youtube.com/watch?v=PZc8ZFRDdrE
[37] https://forum.latenightsw.com/t/parsing-notifications-in-macos-sequoia/5001
[38] https://stackoverflow.com/questions/38512281/swift-on-os-x-how-to-handle-global-mouse-events
[39] https://img.ly/blog/record-screen-with-swift-replaykit/
[40] https://talk.remobjects.com/t/monitor-mouse-and-key-board-events/27277
[41] https://stackoverflow.com/questions/tagged/nsevent?tab=Frequent
[42] https://shadowfacts.net/2021/auto-switch-scroll-direction/
[43] https://eternalstorms.wordpress.com/2015/11/16/how-to-detect-force-touch-capable-devices-on-the-mac/
[44] https://developer.apple.com/documentation/coregraphics/quartz-event-services
[45] https://www.reddit.com/r/swift/comments/158n4c9/cmsamplebuffer_hell_color_and_timing_issues_with/
[46] https://nonstrict.eu/blog/2023/recording-to-disk-with-screencapturekit
[47] https://cocoadev.github.io/NSDistributedNotificationCenter/
[48] https://www.davydovconsulting.com/ios-app-development/using-core-data-for-persistent-storage
[49] https://dev.to/javiersalcedopuyo/tutorial-metal-hellotriangle-using-swift-5-and-no-xcode-i72
[50] https://appleinsider.com/inside/xcode/tips/understanding-metalkit-getting-started-with-apples-graphics-framework
[51] https://link.springer.com/chapter/10.1007/978-1-4842-7045-5_6
[52] https://developer.apple.com/documentation/foundation/distributednotificationcenter



Foreground Context Ingestion on macOS (10.7+) – Key APIs and Strategies
To build an “artificial memory” system that captures as much context as possible from the user’s current activity, you will need to leverage multiple macOS APIs in tandem. Below is a comprehensive overview of the best APIs and techniques to extract foreground application context, UI content (including web pages), and detect dynamic changes (like focus shifts or scrolling).

1. Detecting Foreground Application & Window Changes
NSWorkspace Notifications – Use NSWorkspace to monitor when the user switches apps or when apps terminate. This has been available since macOS 10.6/10.7 and provides a high-level hook for app changes:

App Activation: Subscribe to NSWorkspaceDidActivateApplicationNotification to know when a new app comes to the front. The userInfo includes NSWorkspaceApplicationKey pointing to the NSRunningApplication that was activated[1].
App Deactivation: Similarly, NSWorkspaceDidDeactivateApplicationNotification tells you when an app goes to background (which can help identify the previously frontmost app)[2].
// Example: Listen for app activation
NSWorkspace.shared.notificationCenter.addObserver(
   self,
   selector: #selector(activeAppDidChange(_:)),
   name: NSWorkspace.didActivateApplicationNotification,
   object: nil
)

App Termination: NSWorkspaceDidTerminateApplicationNotification signals when an app quits[3].
CGWindowList API – Use CoreGraphics to get metadata about windows at the OS level, even across apps. CGWindowListCopyWindowInfo returns a list of dictionaries with info on all windows:

Window metadata: Each window dictionary includes keys like the owning app’s name (kCGWindowOwnerName), window title (kCGWindowName), window layer, bounds, and more[4].
Usage: Typically call with .optionOnScreenOnly to get visible windows. For example:
let infoList = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID)?
                .takeRetainedValue() as? [[String: Any]]
for window in infoList ?? [] {
   let owner = window["kCGWindowOwnerName"] as? String
   let title = window["kCGWindowName"] as? String
   // ... use owner/title as needed
}

(Note: On macOS 10.15+, retrieving window titles of other apps requires the Screen Recording permission. Without it, titles may be blank[5][6].)

Why NSWorkspace + CGWindowList: NSWorkspace tells you which app is frontmost, while CGWindowList can enumerate all windows (and their titles/positions) for richer context. For example, when the user switches to Safari, NSWorkspace gives you Safari’s bundle ID and NSRunningApplication, and CGWindowList can find Safari’s front window title.

2. Accessibility API for In-Depth UI Context
The Accessibility framework (AX API) is essential for digging into the foreground app’s UI hierarchy and content. Make sure your app has Accessibility/Assistive Device access enabled, which you mentioned is already set up.

AXUIElement and the Accessibility Tree – Accessibility provides a tree of UI elements for any app’s UI. Key functions and patterns:

Get Frontmost App Element: Create an AXUIElement for the frontmost app’s process ID. For example:
let frontApp = NSWorkspace.shared.frontmostApplication!
let appElement = AXUIElementCreateApplication(frontApp.processIdentifier)

Focused UI Element: The system-wide AX element can yield the currently focused control or view. Use AXUIElementCopyAttributeValue with kAXFocusedUIElementAttribute on the system-wide element[7]:
let systemElem = AXUIElementCreateSystemWide()
var focusedElem: AXUIElement?
AXUIElementCopyAttributeValue(systemElem, kAXFocusedUIElementAttribute as CFString, &focusedElem)

This gives you the element the user is interacting with (e.g. a text field, web view, list item, etc).

Extracting Text Content: If the focused element or one of its children is a text field or text view, you can retrieve its text. Use:
kAXValueAttribute to get the full text value of a text field or document[8] (for non-editable content, AXValue may hold the label or value).
kAXSelectedTextAttribute to get only the currently selected text (if any)[9].
For example, after getting focusedElem above:

var textValue: CFTypeRef?
if AXUIElementCopyAttributeValue(focusedElem!, kAXValueAttribute as CFString, &textValue) == .success {
   if let text = textValue as? String {
       print("Text content: \(text)")
   }
}

And similarly for kAXSelectedTextAttribute to get highlighted text[9].

Hierarchical Traversal: To dump the entire UI hierarchy of the frontmost window (or any AX element), use the AXChildren attribute recursively. For example:
func dumpAXHierarchy(element: AXUIElement, indent: Int = 0) {
   // Get this element’s role and title (if any)
   var role: CFTypeRef?
   AXUIElementCopyAttributeValue(element, kAXRoleAttribute as CFString, &role)
   var title: CFTypeRef?
   AXUIElementCopyAttributeValue(element, kAXTitleAttribute as CFString, &title)
   print(String(repeating: " ", count: indent) + "\(role ?? "" as CFType): \(title ?? "" as CFType)")
   // Recurse into children
   var children: CFTypeRef?
   if AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children) == .success,
      let childList = children as? [AXUIElement] {
      for child in childList {
          dumpAXHierarchy(element: child, indent: indent + 2)
      }
   }
}
This will walk through groups, split panes, buttons, text areas, etc. Note: For large web pages or complex views, this can be intensive. Consider using AXVisibleChildren for large scrollable containers to get only what’s visible[10].
Document Context: Many document-based apps expose the current document path or name via the AXDocument attribute. For instance, a TextEdit window’s AXDocument might give the file path[11][12]. You can query:
var docURL: CFTypeRef?
if AXUIElementCopyAttributeValue(windowElement, kAXDocumentAttribute as CFString, &docURL) == .success {
   if let docPath = docURL as? String {
       print("Current document: \(docPath)")
   }
}
Web Browser Specifics – If the foreground app is a browser (Safari, Chrome, etc.), you likely want the active tab’s URL and title:

Safari: The Safari UI is accessible. The active tab’s web content is an element with role AXWebArea inside the front window. This AXWebArea has an attribute AXURL whose value is the page’s URL[12]. You can find it by traversing children or using AXUIElementCopyAttributeValue if you obtain a reference to the web area element. Safari’s address bar field (role AXSafariAddressAndSearchField) also holds the URL as its value[13], but if the user is currently editing it, that value might be the in-progress text. It’s safer to use the AXWebArea.AXURL.
Chrome (and others): Chrome and many Chromium-based browsers also expose an AXWebArea with an AXURL. However, Chrome’s address bar may not be a standard NSTextField, so Accessibility might require deeper traversal. A general approach is to enumerate the AX hierarchy looking for an element with role "AXWebArea", then query its AXURL[12]. (In cases where a browser does not expose the URL via AX – e.g., older Firefox – you may need to fall back to AppleScript or browser-specific APIs. But Safari/Chrome/Edge/Brave do work with the AX method.)
Tab Changes: When the user switches tabs within a browser window, the window title often changes (to the new tab’s title). Also, the focused UI element might change to the web content. By listening to AX notifications (see next section), you can catch tab switches. Alternatively, poll the AXURL or AXTitle of the web area periodically or on focus changes.
3. Monitoring Focus, Tabs, and UI Changes via Notifications
To capture context every time the user switches to a new app, window, tab, or even pane, you should subscribe to Accessibility notifications. These let your app react to UI changes in real-time:

AXObserver: Create an AXObserver for the frontmost app’s process to receive events. For example:
var observer: AXObserver?
AXObserverCreate(frontAppPID, observerCallback, &observer)
AXObserverAddNotification(observer!, appElement, kAXFocusedWindowChangedNotification as CFString, nil)
CFRunLoopAddSource(CFRunLoopGetCurrent(), AXObserverGetRunLoopSource(observer!), .defaultMode)
(Make sure to keep observer alive, e.g., as a property, so callbacks fire[14][15].)
Key Notifications to Use:
Focus Change – kAXFocusedUIElementChangedNotification: Fires when the keyboard focus moves to a different UI element (e.g., user tabs between fields or switches panes)[16]. Use this to know when the user’s attention within the app shifts, so you can update context (like which pane or control is active).
Focused Window Change – kAXFocusedWindowChangedNotification: Fires when the user switches windows within the same app[17]. E.g., switching from one open document window to another in Word.
Window Created/Closed – kAXWindowCreatedNotification and kAXUIElementDestroyedNotification (for when windows are closed/destroyed) let you know when new windows (or tabs, in some apps) appear[18][19]. This is useful to trigger context ingestion for a newly opened window or tab.
Title or Value Changed – kAXTitleChangedNotification on a window or view can indicate, for example, a tab title change or document name change. kAXValueChangedNotification on certain elements indicates content changes (e.g., typing in a text field, or a slider or scroll value changed)[20].
Selection Changed – kAXSelectedTextChangedNotification triggers when the user changes the text selection (e.g., highlighting text in a document) – allowing you to grab new selected text[21][22]. (Be aware of macOS quirks: sometimes this notification doesn’t fire until an AX API “wakes up” the app’s AX system[23][24]. A reliable alternative is to poll AXSelectedText on focus changes or keypresses if needed.)
Handling Tab Changes: Different apps implement tabs differently. For browsers, a new tab might either be a new window (for apps like Terminal, each tab might actually be an AX “window” in hierarchy) or an AXTabGroup within a window. Many apps will emit a FocusedWindowChanged or a TitleChanged on the window when the active tab changes (since the window’s title updates to the new tab). By listening for those on the app’s main window element, you can detect tab switches. In Safari’s case, switching tabs will change the AXFocusedUIElement to the web area of the new tab (triggering a focus change event).
Scrolling Detection: Scrolling is a bit tricky, as not all scroll events generate AX notifications. Some possibilities:
Listen for AXValueChangedNotification on scroll bar elements or scroll areas. For example, a scrollable text view’s scroll bar may fire value changes (though on some apps, this might only fire at the end of scroll, or not reliably at all[25]). You can obtain the scroll bars via the parent scroll area’s AXHorizontalScrollBar/AXVerticalScrollBar attributes[26][27].
Monitor the AXVisibleChildren or AXVisibleCharacterRange attributes if available. For instance, a large text view might have AXVisibleCharacterRange indicating which part of the text is on-screen[28][29]. Changes in this range imply a scroll occurred. There’s no direct notification for it, but you could poll it or compare after other events.
Event Taps (lower-level): As a fallback, you can use a CGEventTap to intercept scroll-wheel events (type kCGEventScrollWheel). This requires enabling the Input Monitoring permission for your app (since 10.15). An event tap will tell you whenever the user scrolls via mouse or trackpad, and you could then trigger a re-scan of the focused element’s content. Keep in mind: you’d need to map that event to the current focused element (usually the one under the cursor or with focus is scrolling). In many cases, simply knowing “user scrolled” while your app knows which element is focused can be enough to decide to refresh context.
Tip: Many of these notifications should be added to the application AX element (the root AXUIElement for the app) so that they propagate to you[22]. Once you know the frontmost app (from NSWorkspace), create an AXObserver for it and register for notifications on that app’s AXUIElement.

4. Putting It Together – Ingestion Loop
Every time the user foregrounds an app, switches window/tab, or scrolls significantly, you should capture context. A possible flow:

App Switch (NSWorkspace) – When a new app becomes active, create an AXUIElement for it and attach an AXObserver for key events (focus changes, window/tab changes). Immediately fetch initial context:
App name, bundle ID (from NSRunningApplication).
Frontmost window title (AXTitle of kAXFocusedWindow).
If document-based, document path (AXDocument).
If text-focused, selected text or entire text (AXSelectedText or AXValue).
If web content, the URL (AXWebArea.AXURL or address field).
Any other identifiable context (e.g., currently playing song info if iTunes, etc., via AX or Scripting if needed).
Focus/Pane Change – When you get a AXFocusedUIElementChangedNotification, update context for the new focused element. For example, user moved from document text area to the search bar in an app – you’d capture that the focus is now a different UI element (and maybe grab its text if any).
Window/Tab Change – On AXFocusedWindowChangedNotification or window title changes, treat it similar to an app switch: gather info about the new window or tab (title, document, URL, etc.).
Scroll/Content Change – On scroll events or value changes, you might do a more lightweight update: e.g., update the visible text snippet or just note that a scroll happened. You could throttle re-analysis (for example, don’t re-scan on every tiny scroll event, but perhaps every few seconds during a continuous scroll, or after scrolling stops).
By combining these APIs – NSWorkspace for high-level app changes, CGWindow for window metadata, and Accessibility for deep content – you can continuously maintain a rich representation of the user’s current context. Apple’s own sample UIElementInspector (though old) is a great resource to see how to traverse and inspect the accessibility hierarchy[30]. The core concepts haven’t changed since macOS 10.7, ensuring broad compatibility.

Remember that some private or sandboxed contexts might be unavailable (e.g., password fields won’t yield text, some system apps may have limited AX info), but generally this approach maximizes what you can get. With all permissions granted and using the above strategies, your artificial memory system will capture the who/what/where of the user’s current focus to a very high degree.

Sources:

NSWorkspace app activation observer example[1]
Using Accessibility API to get focused UI element and selected text[7][9]
Accessibility API to get text values from focused elements[8]
Safari AX hierarchy for URL (AXWebArea and address field)[13][12]
List of useful AX notifications (focus, window, value changed, etc.)[16][20]
CGWindowList usage for window info[4]
AXVisibleChildren and visible range for scrollable content[10][28]
Apple DTS advice to prefer AX API over CGWindow for tracking UI state[30]
 
[1] [3] objective c - NSWorkspaceDidActivateApplicationNotification fails when app is closed by clicking red cross and reopened - Stack Overflow

https://stackoverflow.com/questions/46604111/nsworkspacedidactivateapplicationnotification-fails-when-app-is-closed-by-clicki
[2] objective c - Getting Previous Frontmost Application - Stack Overflow

https://stackoverflow.com/questions/24720530/getting-previous-frontmost-application
[4] macos - How to get window list from core-grapics API with swift - Stack Overflow

https://stackoverflow.com/questions/30336740/how-to-get-window-list-from-core-grapics-api-with-swift
[5] [6] GitHub - sindresorhus/get-windows: Get metadata about the active window and open windows (title, id, bounds, owner, etc)

https://github.com/sindresorhus/get-windows
[7] [9] How to get selected text and its coordinates from any system wide application using Accessibility API? | mac developers

https://macdevelopers.wordpress.com/2014/02/05/how-to-get-selected-text-and-its-coordinates-from-any-system-wide-application-using-accessibility-api/
[8] Accessing text value from any System wide Application via Accessibility API | mac developers

https://macdevelopers.wordpress.com/2014/01/31/accessing-text-value-from-any-system-wide-application-via-accessibility-api/
[10] [16] [17] [18] [19] [20] [26] [27] [28] [29] Carbon Accessibility Reference

https://leopard-adc.pepas.com/documentation/Accessibility/Reference/AccessibilityCarbonRef/AccessibilityCarbonRef.pdf
[11] [12] [13] macos - How to retrieve active window URL using Mac OS X accessibility API - Stack Overflow

https://stackoverflow.com/questions/53229924/how-to-retrieve-active-window-url-using-mac-os-x-accessibility-api
[14] [15] swift - How use AXObserverAddNotification? - Stack Overflow

https://stackoverflow.com/questions/68793532/how-use-axobserveraddnotification
[21] [22] [23] [24] macos - kAXSelectedTextChangedNotification not received after restart, until launching Accessibility Inspector - Stack Overflow

https://stackoverflow.com/questions/79618732/kaxselectedtextchangednotification-not-received-after-restart-until-launching-a
[25] Scroll bars don't post accessibility value changed events on macOS

https://youtrack.jetbrains.com/issue/JBR-8408
[30] Swift macOS, listen to open, close events of any application - Using Swift - Swift Forums

https://forums.swift.org/t/swift-macos-listen-to-open-close-events-of-any-application/29021