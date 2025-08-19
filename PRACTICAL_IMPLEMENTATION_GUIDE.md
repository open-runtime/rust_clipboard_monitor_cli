# Practical Implementation Guide: Building an Artificial Memory System on macOS

## Table of Contents
1. [System Architecture](#system-architecture)
2. [Permission Management](#permission-management)
3. [Context Extraction Pipeline](#context-extraction-pipeline)
4. [Real-World Implementation](#real-world-implementation)
5. [Production Considerations](#production-considerations)

---

## System Architecture

### High-Level Design

```
┌─────────────────────────────────────────────────────────┐
│                   User Interface Layer                   │
│            (Status, Settings, Memory Viewer)             │
└─────────────────────────┬───────────────────────────────┘
                          │
┌─────────────────────────▼───────────────────────────────┐
│                 Memory Processing Layer                  │
│     (Context Analysis, Memory Formation, Indexing)       │
└─────────────────────────┬───────────────────────────────┘
                          │
┌─────────────────────────▼───────────────────────────────┐
│                Context Aggregation Layer                 │
│      (Data Fusion, Deduplication, Prioritization)        │
└─────────────────────────┬───────────────────────────────┘
                          │
        ┌─────────────────┴─────────────────┐
        │                                   │
┌───────▼────────┐  ┌──────────────┐  ┌───▼──────────────┐
│ Event Monitor  │  │Screen Monitor│  │ Content Extractor│
│ (CGEventTap,   │  │(ScreenCapture│  │ (AXUIElement,    │
│  IOHIDManager) │  │     Kit)     │  │  AppleScript)    │
└────────────────┘  └──────────────┘  └──────────────────┘
```

### Core Components Implementation

```swift
// Main system coordinator
class ArtificialMemoryCoordinator {
    private let permissionManager = PermissionManager()
    private let eventMonitor = EventMonitor()
    private let screenMonitor = ScreenMonitor()
    private let contentExtractor = ContentExtractor()
    private let memoryProcessor = MemoryProcessor()
    private let storageManager = StorageManager()
    
    // Context buffer for aggregation
    private var contextBuffer = ContextBuffer(maxSize: 100)
    private var processingQueue = DispatchQueue(label: "memory.processing", qos: .userInitiated)
    
    func initialize() async throws {
        // Step 1: Check and request permissions
        try await permissionManager.ensureRequiredPermissions()
        
        // Step 2: Initialize monitors based on available permissions
        let permissions = permissionManager.currentPermissions
        
        if permissions.accessibility {
            contentExtractor.initialize()
        }
        
        if permissions.inputMonitoring {
            eventMonitor.initialize()
        }
        
        if permissions.screenRecording {
            screenMonitor.initialize()
        }
        
        // Step 3: Set up data flow
        setupDataPipeline()
        
        // Step 4: Start monitoring
        startAllMonitors()
    }
    
    private func setupDataPipeline() {
        // Event monitor → Context buffer
        eventMonitor.onEvent = { [weak self] event in
            self?.contextBuffer.add(.event(event))
        }
        
        // Screen monitor → Context buffer
        screenMonitor.onFrameCapture = { [weak self] frame in
            self?.contextBuffer.add(.visual(frame))
        }
        
        // Content extractor → Context buffer
        contentExtractor.onContentExtracted = { [weak self] content in
            self?.contextBuffer.add(.textContent(content))
        }
        
        // Context buffer → Memory processor
        contextBuffer.onBufferFull = { [weak self] contexts in
            self?.processingQueue.async {
                self?.processContextBatch(contexts)
            }
        }
    }
    
    private func processContextBatch(_ contexts: [ContextItem]) {
        // Aggregate related contexts
        let aggregated = ContextAggregator.aggregate(contexts)
        
        // Form memory from aggregated context
        let memory = memoryProcessor.formMemory(from: aggregated)
        
        // Store memory
        storageManager.store(memory)
        
        // Trigger any real-time features
        notifyMemoryUpdate(memory)
    }
}
```

---

## Permission Management

### Progressive Permission Strategy

```swift
class PermissionManager {
    enum PermissionLevel {
        case minimal      // NSWorkspace only
        case basic        // + CGWindowList
        case enhanced     // + Accessibility
        case full         // + Screen Recording + Input Monitoring
    }
    
    struct Permissions {
        var accessibility: Bool = false
        var screenRecording: Bool = false
        var inputMonitoring: Bool = false
        var automation: [String: Bool] = [:] // Per-app automation
        
        var level: PermissionLevel {
            if screenRecording && inputMonitoring && accessibility {
                return .full
            } else if accessibility {
                return .enhanced
            } else if !automation.isEmpty {
                return .basic
            } else {
                return .minimal
            }
        }
    }
    
    func requestPermissions(for level: PermissionLevel) async throws {
        switch level {
        case .minimal:
            // No permissions needed
            break
            
        case .basic:
            // Request automation for key apps
            try await requestAutomation(for: ["com.apple.Safari", "com.google.Chrome"])
            
        case .enhanced:
            // Request accessibility
            try await requestAccessibility()
            
        case .full:
            // Request all permissions
            try await requestAccessibility()
            try await requestScreenRecording()
            try await requestInputMonitoring()
        }
    }
    
    private func requestAccessibility() async throws {
        let options: NSDictionary = [
            kAXTrustedCheckOptionPrompt.takeRetainedValue(): true
        ]
        
        if !AXIsProcessTrustedWithOptions(options) {
            // Show custom UI explaining why we need accessibility
            await showPermissionExplanation(.accessibility)
            
            // Open System Settings
            NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")!)
            
            // Wait for permission
            try await waitForPermission(timeout: 60) {
                AXIsProcessTrusted()
            }
        }
    }
    
    private func requestScreenRecording() async throws {
        // Attempt to create screen capture to trigger permission
        do {
            _ = try await SCShareableContent.current
        } catch {
            // Permission denied
            await showPermissionExplanation(.screenRecording)
            
            // Open System Settings
            NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")!)
            
            // Wait for permission
            try await waitForPermission(timeout: 60) {
                Task {
                    do {
                        _ = try await SCShareableContent.current
                        return true
                    } catch {
                        return false
                    }
                }.result.get()
            }
        }
    }
}
```

### Permission Fallback System

```swift
class AdaptiveMonitor {
    private var availableAPIs: Set<AvailableAPI> = []
    
    func determineAvailableAPIs() {
        // Check each API availability
        if AXIsProcessTrusted() {
            availableAPIs.insert(.accessibility)
        }
        
        if canCreateEventTap() {
            availableAPIs.insert(.eventTap)
        }
        
        if canUseScreenCapture() {
            availableAPIs.insert(.screenCapture)
        }
        
        // Always available
        availableAPIs.insert(.nsWorkspace)
        availableAPIs.insert(.cgWindowList)
    }
    
    func getBestMethodFor(_ task: MonitoringTask) -> MonitoringMethod {
        switch task {
        case .detectAppSwitch:
            // Prefer NSWorkspace (no permissions)
            return .nsWorkspace
            
        case .extractText:
            if availableAPIs.contains(.accessibility) {
                return .axUIElement
            } else if availableAPIs.contains(.screenCapture) {
                return .screenCaptureWithOCR
            } else {
                return .none
            }
            
        case .monitorKeyboard:
            if availableAPIs.contains(.eventTap) {
                return .cgEventTap
            } else {
                return .ioHIDManager // Usually works without permissions
            }
            
        case .captureScreen:
            if availableAPIs.contains(.screenCapture) {
                return .screenCaptureKit
            } else {
                return .cgWindowListImage // Limited but works
            }
        }
    }
}
```

---

## Context Extraction Pipeline

### Multi-Source Context Fusion

```swift
class ContextExtractor {
    struct ExtractedContext {
        var timestamp: Date
        var applicationName: String
        var applicationBundleID: String
        var windowTitle: String?
        var documentPath: String?
        var webURL: String?
        var selectedText: String?
        var visibleText: String?
        var uiHierarchy: [String: Any]?
        var screenContent: CGImage?
        var keyboardActivity: KeyboardMetrics?
        var mouseActivity: MouseMetrics?
        var scrollPosition: CGPoint?
        var confidence: Float
    }
    
    func extractFullContext() async -> ExtractedContext {
        var context = ExtractedContext(
            timestamp: Date(),
            applicationName: "",
            applicationBundleID: "",
            confidence: 0.0
        )
        
        // Layer 1: Basic app info (always works)
        if let appInfo = extractBasicAppInfo() {
            context.applicationName = appInfo.name
            context.applicationBundleID = appInfo.bundleID
            context.confidence += 0.2
        }
        
        // Layer 2: Window info (usually works)
        if let windowInfo = extractWindowInfo() {
            context.windowTitle = windowInfo.title
            context.confidence += 0.2
        }
        
        // Layer 3: Accessibility content (needs permission)
        if let accessibilityContent = await extractAccessibilityContent() {
            context.selectedText = accessibilityContent.selectedText
            context.visibleText = accessibilityContent.visibleText
            context.documentPath = accessibilityContent.documentPath
            context.webURL = accessibilityContent.webURL
            context.uiHierarchy = accessibilityContent.hierarchy
            context.confidence += 0.4
        }
        
        // Layer 4: Visual content (needs screen recording)
        if let screenContent = await captureScreenContent() {
            context.screenContent = screenContent
            
            // Run OCR if no text from accessibility
            if context.visibleText == nil {
                context.visibleText = await performOCR(on: screenContent)
            }
            context.confidence += 0.2
        }
        
        return context
    }
    
    private func extractAccessibilityContent() async -> AccessibilityContent? {
        guard AXIsProcessTrusted() else { return nil }
        
        return await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                var content = AccessibilityContent()
                
                // Get focused element
                let systemWide = AXUIElementCreateSystemWide()
                var focusedElement: CFTypeRef?
                
                let result = AXUIElementCopyAttributeValue(
                    systemWide,
                    kAXFocusedUIElementAttribute as CFString,
                    &focusedElement
                )
                
                if result == .success, let element = focusedElement {
                    // Extract text content
                    content.selectedText = self.extractText(from: element as! AXUIElement, attribute: kAXSelectedTextAttribute)
                    content.visibleText = self.extractText(from: element as! AXUIElement, attribute: kAXValueAttribute)
                    
                    // Get document path if available
                    content.documentPath = self.extractDocumentPath(from: element as! AXUIElement)
                    
                    // Get web URL if in browser
                    content.webURL = self.extractWebURL(from: element as! AXUIElement)
                    
                    // Build UI hierarchy
                    content.hierarchy = self.buildUIHierarchy(from: element as! AXUIElement)
                }
                
                continuation.resume(returning: content)
            }
        }
    }
    
    private func extractWebURL(from element: AXUIElement) -> String? {
        // Try direct URL attribute
        if let url = getAttributeValue(element, kAXURLAttribute) as? String {
            return url
        }
        
        // Try to find browser address bar
        if let window = findParentWindow(of: element) {
            // Safari
            if let addressField = findDescendant(
                in: window,
                withRole: kAXTextFieldRole,
                identifier: "Address and Search"
            ) {
                return getAttributeValue(addressField, kAXValueAttribute) as? String
            }
            
            // Chrome
            if let omnibox = findDescendant(
                in: window,
                withRole: kAXTextFieldRole,
                subrole: "AXURLField"
            ) {
                return getAttributeValue(omnibox, kAXValueAttribute) as? String
            }
        }
        
        return nil
    }
}
```

### Browser-Specific Extraction

```swift
class BrowserContextExtractor {
    enum Browser {
        case safari
        case chrome
        case firefox
        case edge
        
        static func detect(from bundleID: String) -> Browser? {
            switch bundleID {
            case "com.apple.Safari": return .safari
            case "com.google.Chrome": return .chrome
            case "org.mozilla.firefox": return .firefox
            case "com.microsoft.Edge": return .edge
            default: return nil
            }
        }
    }
    
    func extractBrowserContext(for browser: Browser, pid: pid_t) -> BrowserContext? {
        switch browser {
        case .safari:
            return extractSafariContext(pid: pid)
        case .chrome:
            return extractChromeContext(pid: pid)
        case .firefox:
            return extractFirefoxContext(pid: pid)
        case .edge:
            return extractEdgeContext(pid: pid)
        }
    }
    
    private func extractSafariContext(pid: pid_t) -> BrowserContext? {
        // Method 1: AppleScript (most reliable for Safari)
        if let scriptResult = executeAppleScript("""
            tell application "Safari"
                set currentTab to current tab of front window
                return {URL of currentTab, name of currentTab}
            end tell
        """) {
            return BrowserContext(
                url: scriptResult[0],
                title: scriptResult[1],
                method: .appleScript
            )
        }
        
        // Method 2: Accessibility API fallback
        return extractViaAccessibility(pid: pid)
    }
    
    private func extractChromeContext(pid: pid_t) -> BrowserContext? {
        // Chrome requires different approach
        // Try AppleScript first
        if let scriptResult = executeAppleScript("""
            tell application "Google Chrome"
                set currentTab to active tab of front window
                return {URL of currentTab, title of currentTab}
            end tell
        """) {
            return BrowserContext(
                url: scriptResult[0],
                title: scriptResult[1],
                method: .appleScript
            )
        }
        
        // Fallback to accessibility
        return extractViaAccessibility(pid: pid)
    }
    
    private func extractViaAccessibility(pid: pid_t) -> BrowserContext? {
        let app = AXUIElementCreateApplication(pid)
        
        // Find the web area
        if let webArea = findUIElement(in: app, withRole: "AXWebArea") {
            let url = getAttributeValue(webArea, kAXURLAttribute) as? String
            let title = getAttributeValue(webArea, kAXTitleAttribute) as? String
            
            return BrowserContext(
                url: url,
                title: title,
                method: .accessibility
            )
        }
        
        return nil
    }
}
```

---

## Real-World Implementation

### Complete Working Example

```swift
// Complete implementation of a basic memory system
class MemorySystem {
    private var isRunning = false
    private let updateInterval: TimeInterval = 1.0 // Update every second
    private var updateTimer: Timer?
    
    // Monitors
    private var appMonitor: AppMonitor?
    private var contentMonitor: ContentMonitor?
    private var eventMonitor: EventMonitor?
    
    // Storage
    private let memoryStore = MemoryStore()
    
    // Current context
    private var currentContext = CurrentContext()
    
    func start() {
        guard !isRunning else { return }
        isRunning = true
        
        // Initialize monitors based on available permissions
        setupMonitors()
        
        // Start periodic context capture
        updateTimer = Timer.scheduledTimer(withTimeInterval: updateInterval, repeats: true) { _ in
            self.captureContext()
        }
        
        print("Memory system started")
    }
    
    func stop() {
        isRunning = false
        updateTimer?.invalidate()
        updateTimer = nil
        
        // Clean up monitors
        appMonitor?.stop()
        contentMonitor?.stop()
        eventMonitor?.stop()
        
        print("Memory system stopped")
    }
    
    private func setupMonitors() {
        // Always available - app switching
        appMonitor = AppMonitor()
        appMonitor?.onAppSwitch = { [weak self] app in
            self?.handleAppSwitch(app)
        }
        appMonitor?.start()
        
        // If we have accessibility permission
        if AXIsProcessTrusted() {
            contentMonitor = ContentMonitor()
            contentMonitor?.onContentChange = { [weak self] content in
                self?.handleContentChange(content)
            }
            contentMonitor?.start()
        }
        
        // If we have input monitoring permission
        if canMonitorInput() {
            eventMonitor = EventMonitor()
            eventMonitor?.onKeyPress = { [weak self] key in
                self?.handleKeyPress(key)
            }
            eventMonitor?.start()
        }
    }
    
    private func captureContext() {
        // Build comprehensive context
        var context = MemoryContext()
        context.timestamp = Date()
        
        // App context
        if let frontmostApp = NSWorkspace.shared.frontmostApplication {
            context.applicationName = frontmostApp.localizedName
            context.applicationBundleID = frontmostApp.bundleIdentifier
        }
        
        // Window context
        if let windowInfo = getActiveWindowInfo() {
            context.windowTitle = windowInfo.title
            context.windowBounds = windowInfo.bounds
        }
        
        // Content context (if available)
        if let content = contentMonitor?.getCurrentContent() {
            context.visibleText = content.text
            context.selectedText = content.selectedText
            context.documentPath = content.documentPath
            context.webURL = content.webURL
        }
        
        // Activity metrics
        context.keyboardActivity = eventMonitor?.getActivityMetrics()
        context.idleTime = CGEventSource.secondsSinceLastEventType(
            .combinedSessionState,
            eventType: .any
        )
        
        // Determine if context is significant enough to store
        if isSignificantContext(context) {
            formAndStoreMemory(from: context)
        }
    }
    
    private func isSignificantContext(_ context: MemoryContext) -> Bool {
        // Don't store if user is idle
        if context.idleTime > 60 {
            return false
        }
        
        // Don't store if no meaningful content
        if context.visibleText?.isEmpty ?? true &&
           context.webURL?.isEmpty ?? true &&
           context.documentPath?.isEmpty ?? true {
            return false
        }
        
        // Don't store if too similar to last context
        if let lastContext = memoryStore.getLastContext() {
            let similarity = calculateSimilarity(context, lastContext)
            if similarity > 0.95 {
                return false
            }
        }
        
        return true
    }
    
    private func formAndStoreMemory(from context: MemoryContext) {
        // Create memory entry
        let memory = Memory(
            id: UUID(),
            timestamp: context.timestamp,
            context: context,
            tags: extractTags(from: context),
            importance: calculateImportance(context)
        )
        
        // Store memory
        memoryStore.store(memory)
        
        // Trigger any listeners
        NotificationCenter.default.post(
            name: .memoryCreated,
            object: memory
        )
    }
}

// Usage
let memorySystem = MemorySystem()
memorySystem.start()

// Query memories
let memories = memorySystem.memoryStore.query(
    containing: "Swift",
    inApp: "Xcode",
    from: Date().addingTimeInterval(-3600)
)
```

---

## Production Considerations

### Performance Optimization

```swift
class PerformanceOptimizedMonitor {
    // Adaptive sampling based on system load
    private var samplingRate: TimeInterval = 0.5
    private let cpuMonitor = CPUMonitor()
    
    func adaptiveSampling() {
        let cpuUsage = cpuMonitor.getCurrentUsage()
        
        if cpuUsage > 80 {
            // High CPU - reduce sampling
            samplingRate = 2.0
        } else if cpuUsage > 50 {
            // Moderate CPU - normal sampling
            samplingRate = 1.0
        } else {
            // Low CPU - increase sampling
            samplingRate = 0.5
        }
    }
    
    // Intelligent caching
    private let cache = ContextCache(maxSize: 1000)
    
    func getCachedOrExtract(for app: NSRunningApplication) -> AppContext? {
        let cacheKey = "\(app.processIdentifier)_\(Date().timeIntervalSince1970.rounded())"
        
        if let cached = cache.get(cacheKey) {
            return cached
        }
        
        let context = extractContext(for: app)
        cache.set(cacheKey, context)
        return context
    }
    
    // Batch processing
    private var eventQueue = [Event]()
    private let batchSize = 100
    
    func queueEvent(_ event: Event) {
        eventQueue.append(event)
        
        if eventQueue.count >= batchSize {
            processBatch()
        }
    }
    
    private func processBatch() {
        let batch = eventQueue
        eventQueue.removeAll()
        
        DispatchQueue.global(qos: .background).async {
            // Process events in batch for efficiency
            let aggregated = self.aggregateEvents(batch)
            self.storeAggregated(aggregated)
        }
    }
}
```

### Error Handling and Recovery

```swift
class RobustMemorySystem {
    enum SystemError: Error {
        case permissionDenied(Permission)
        case apiUnavailable(API)
        case resourceExhausted
        case storageFailure
    }
    
    private var failureCount: [String: Int] = [:]
    private let maxFailures = 3
    
    func monitorWithRecovery() {
        do {
            try performMonitoring()
            // Reset failure count on success
            failureCount.removeAll()
        } catch {
            handleError(error)
        }
    }
    
    private func handleError(_ error: Error) {
        let errorKey = String(describing: error)
        failureCount[errorKey, default: 0] += 1
        
        if failureCount[errorKey]! >= maxFailures {
            // Too many failures - switch to degraded mode
            enterDegradedMode(for: error)
        } else {
            // Attempt recovery
            attemptRecovery(from: error)
        }
    }
    
    private func enterDegradedMode(for error: Error) {
        switch error {
        case SystemError.permissionDenied(.accessibility):
            // Disable features requiring accessibility
            disableAccessibilityFeatures()
            // Use alternative methods
            enableOCRFallback()
            
        case SystemError.apiUnavailable(.screenCaptureKit):
            // Fall back to CGWindowListCreateImage
            useAlternativeScreenCapture()
            
        case SystemError.resourceExhausted:
            // Reduce monitoring frequency
            reduceMonitoringIntensity()
            
        default:
            // Generic degraded mode
            switchToMinimalMonitoring()
        }
    }
    
    private func attemptRecovery(from error: Error) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) {
            self.monitorWithRecovery()
        }
    }
}
```

### Privacy and Security

```swift
class PrivacyAwareMonitor {
    // Exclude sensitive applications
    private let excludedApps = Set([
        "com.apple.keychainaccess",
        "com.1password.1password",
        "com.lastpass.LastPass",
        "com.agilebits.onepassword7"
    ])
    
    // Exclude sensitive URLs
    private let sensitiveURLPatterns = [
        #/.*\.bank\..*/,
        #/.*paypal\..*/,
        #/.*\.gov\..*/,
        #/.*login.*/,
        #/.*signin.*/,
        #/.*password.*/
    ]
    
    func shouldMonitor(app: NSRunningApplication) -> Bool {
        guard let bundleID = app.bundleIdentifier else { return false }
        return !excludedApps.contains(bundleID)
    }
    
    func sanitizeURL(_ url: String) -> String? {
        // Check if URL contains sensitive patterns
        for pattern in sensitiveURLPatterns {
            if url.contains(pattern) {
                return nil // Don't store sensitive URLs
            }
        }
        
        // Remove query parameters that might contain sensitive data
        if let urlComponents = URLComponents(string: url) {
            var sanitized = urlComponents
            sanitized.queryItems = nil
            sanitized.fragment = nil
            return sanitized.string
        }
        
        return url
    }
    
    func sanitizeText(_ text: String) -> String {
        // Remove potential passwords (basic heuristic)
        let passwordPattern = #/password[:\s]*\S+/i
        var sanitized = text.replacing(passwordPattern, with: "password: [REDACTED]")
        
        // Remove credit card numbers
        let ccPattern = #/\b(?:\d{4}[\s-]?){3}\d{4}\b/
        sanitized = sanitized.replacing(ccPattern, with: "[CREDIT CARD REDACTED]")
        
        // Remove SSNs
        let ssnPattern = #/\b\d{3}-\d{2}-\d{4}\b/
        sanitized = sanitized.replacing(ssnPattern, with: "[SSN REDACTED]")
        
        return sanitized
    }
}
```

### Distribution and Updates

```swift
class DistributionManager {
    enum DistributionMethod {
        case appStore        // Maximum restrictions
        case developerID     // Notarized, fewer restrictions
        case enterprise      // Internal distribution
        case development     // Testing only
    }
    
    static func configureForDistribution(_ method: DistributionMethod) -> AppConfiguration {
        switch method {
        case .appStore:
            return AppConfiguration(
                sandboxed: true,
                entitlements: [
                    // Very limited - no accessibility
                    "com.apple.security.app-sandbox": true,
                    "com.apple.security.files.user-selected.read-only": true
                ],
                features: [.basic]
            )
            
        case .developerID:
            return AppConfiguration(
                sandboxed: false,
                entitlements: [
                    // Full access with notarization
                    "com.apple.security.automation.apple-events": true,
                    "com.apple.security.temporary-exception.apple-events": true
                ],
                features: [.basic, .enhanced, .full]
            )
            
        case .enterprise:
            return AppConfiguration(
                sandboxed: false,
                entitlements: [
                    // Full access for internal use
                ],
                features: [.basic, .enhanced, .full, .experimental]
            )
            
        case .development:
            return AppConfiguration(
                sandboxed: false,
                entitlements: [
                    // All permissions for testing
                ],
                features: Feature.allCases
            )
        }
    }
}
```

---

## Summary

This practical implementation guide provides:

1. **Working code examples** that can be directly used
2. **Permission management strategies** with fallbacks
3. **Multi-layered context extraction** for maximum information
4. **Production-ready patterns** for error handling and performance
5. **Privacy considerations** for responsible development

The key to success is:
- Start with minimal permissions and progressively request more
- Implement graceful degradation when APIs fail
- Cache aggressively to reduce system load
- Respect user privacy and exclude sensitive content
- Test thoroughly across different macOS versions and configurations
