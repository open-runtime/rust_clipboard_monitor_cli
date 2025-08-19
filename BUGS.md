# Bug Report: Clipboard Monitor CLI Issues

## Date: 2024-12-20
## Component: rust_clipboard_monitor_cli

---

## 🔴 Critical Issues Identified

### 1. Safari URL Extraction Completely Disabled
**Severity**: High  
**Impact**: No URL tracking for Safari browser  
**Status**: Broken

#### Problem Description
Safari URL extraction is entirely disabled in the codebase, resulting in `null` values for both `url` and `actual_url` fields when Safari is the active application.

#### Root Cause
In `src/main.rs` line 468:
```rust
if bundle_id.contains("Safari") && false {
    // Safari-specific URL extraction - TEMPORARILY DISABLED due to crashes
    // TODO: Fix Safari helper functions to prevent segfaults
```

The condition `&& false` effectively disables all Safari URL extraction logic. According to the comment, this was done to prevent crashes/segfaults.

#### Evidence from Output
```json
{
    "app_name": "Safari",
    "bundle_id": "com.apple.Safari",
    "window_title": "Start Page",
    "url": null,
    "actual_url": null,
    "page_title": null
}
```

#### Impact
- Cannot track browsing activity in Safari
- Cannot capture URLs for research tracking
- Cannot monitor web-based workflows in Safari
- Context is incomplete for Safari-based copy/paste operations

---

### 2. Clipboard Content Not Captured on Copy/Paste Events
**Severity**: High  
**Impact**: Missing clipboard content in copy/paste event tracking  
**Status**: Race Condition

#### Problem Description
When copy/paste keyboard shortcuts are detected, the clipboard content is consistently `null` even though metadata shows content exists.

#### Root Cause
**Race Condition in Keyboard Event Handler** (`src/main.rs` lines 1452-1476):
```rust
fn handle_clipboard_shortcut(&mut self, action: &str) {
    match action {
        "copy" | "cut" => {
            let event = ClipboardEvent {
                content: self.get_clipboard_text(), // ⚠️ Called immediately
                // ...
            };
```

The keyboard event handler captures Cmd+C/V/X keystrokes but immediately attempts to read clipboard content. At this point, the OS may not have completed updating the clipboard, resulting in either:
- Old clipboard content being read
- Null content if the clipboard update is still in progress

#### Evidence from Output
```json
{
    "event_type": "copy",
    "content": null,  // ⚠️ Content is null
    "content_type": "unknown",
    "metadata": {
        "available_types": "public.utf8-plain-text, NSStringPboardType, ...",
        "type_count": "4",
        "change_count": "1271"  // ⚠️ But clipboard has content!
    }
}
```

#### Technical Analysis
The issue occurs because:
1. Keyboard event is captured at the input level (before clipboard update)
2. `get_clipboard_text()` is called synchronously without waiting
3. The clipboard update happens asynchronously after the key event
4. The separate clipboard monitoring thread (polling every 100ms) might catch the content later, but it's disconnected from the keyboard event

#### Impact
- Cannot see what content was copied
- Cannot track data flow between applications
- Audit trail is incomplete
- Security monitoring capabilities are limited

---

### 3. QuickTime Player File Paths Not Extracted
**Severity**: Medium  
**Impact**: Missing document paths for media files  
**Status**: Missing Implementation

#### Problem Description
QuickTime Player shows the filename in the window title but `document_path` remains `null`, preventing tracking of which media files are being accessed.

#### Root Cause
**No QuickTime-Specific Implementation** (`src/main.rs` lines 585-715):

The code has specific handlers for various applications:
- IDEs (VS Code, Cursor, IntelliJ)
- Finder
- Browsers (Chrome, Firefox)

But **no handler exists for QuickTime Player**. The generic document extraction methods don't work for QuickTime because it doesn't expose the file path through standard accessibility attributes like `AXDocument`, `AXURL`, or `AXPath`.

#### Evidence from Output
```json
{
    "app_name": "QuickTime Player",
    "bundle_id": "com.apple.QuickTimePlayerX",
    "window_title": "Richard Crist \"Next Level\".mov",  // ⚠️ Filename is visible
    "document_path": null,  // ⚠️ But path is not extracted
    "active_file": null
}
```

#### Missing Implementation Details
QuickTime Player requires special handling to:
1. Parse the window title to extract the filename
2. Use QuickTime-specific accessibility attributes
3. Potentially use AppleScript to query the actual file path
4. Check for recently opened files in the system

#### Impact
- Cannot track which media files are being viewed
- Cannot correlate media consumption with other activities
- File access audit is incomplete
- Workflow analysis misses media file interactions

---

## 🟡 Additional Issues

### 4. Clipboard Thread Timing Issue
**Severity**: Low  
**Impact**: Delayed clipboard content capture

The clipboard monitoring thread polls every 100ms (`src/main.rs` line 1540), which means:
- Best case: 0ms delay if perfectly timed
- Worst case: 100ms delay
- Average case: 50ms delay

This contributes to the race condition in issue #2.

### 5. Incomplete Error Handling
The disabled Safari code mentions "segfaults" but there's no error recovery or fallback mechanism. When Safari extraction was disabled, no alternative method was implemented.

---

## 📊 Summary Statistics

From the test data analyzed:
- **Safari Events**: 4 occurrences, 0% with URLs
- **Copy Events**: 2 occurrences, 0% with content
- **QuickTime Events**: 5 occurrences, 0% with file paths
- **Overall Data Completeness**: ~60% (missing critical fields)

---

## 🔧 Recommended Fixes

### Fix 1: Safari URL Extraction
1. **Investigate the segfault issue** in Safari helper functions
2. **Implement safer extraction** with proper error handling:
   ```rust
   if bundle_id.contains("Safari") {
       if let Ok(url) = self.safe_extract_safari_url(window) {
           ctx.url = Some(url);
       }
   }
   ```
3. **Add fallback methods**:
   - AppleScript as backup
   - JavaScript injection via accessibility API
   - Parse from window/tab titles

### Fix 2: Clipboard Content Capture
1. **Add delay after keyboard event**:
   ```rust
   "copy" | "cut" => {
       // Wait for clipboard to update
       thread::sleep(Duration::from_millis(50));
       let content = self.get_clipboard_text();
   }
   ```
2. **Better approach - Deferred reading**:
   ```rust
   "copy" | "cut" => {
       // Schedule clipboard read for next iteration
       self.pending_clipboard_read = Some((action.to_string(), Instant::now()));
   }
   ```
3. **Use clipboard change count** to verify update:
   ```rust
   let initial_count = self.get_clipboard_change_count();
   // ... wait for count to change ...
   let content = self.get_clipboard_text();
   ```

### Fix 3: QuickTime File Path Extraction
1. **Add QuickTime-specific handler**:
   ```rust
   if bundle_id == "com.apple.QuickTimePlayerX" {
       // Extract from window title
       if let Some(title) = &ctx.window_title {
           ctx.document_path = self.extract_quicktime_path(title);
       }
       // Try QuickTime-specific attributes
       ctx.document_path = ctx.document_path.or_else(|| {
           self.get_quicktime_document_path(window)
       });
   }
   ```
2. **Implement AppleScript fallback**:
   ```applescript
   tell application "QuickTime Player"
       if (count documents) > 0 then
           return file of front document as string
       end if
   end tell
   ```

---

## 🧪 Testing Recommendations

1. **Safari URL Tests**:
   - Test with multiple tabs
   - Test with different URL types (http, https, file://)
   - Test during navigation events

2. **Clipboard Tests**:
   - Test rapid copy/paste sequences
   - Test different content types (text, images, files)
   - Test cross-application copy/paste

3. **QuickTime Tests**:
   - Test with local files
   - Test with streamed content
   - Test with different media formats

---

## 📈 Priority Matrix

| Issue | Severity | Effort | Priority | User Impact |
|-------|----------|--------|----------|-------------|
| Safari URLs | High | Medium | P1 | High - Browser tracking broken |
| Clipboard Content | High | Low | P1 | High - Core functionality broken |
| QuickTime Paths | Medium | Low | P2 | Medium - Media tracking incomplete |
| Thread Timing | Low | Low | P3 | Low - Minor delays |

---

## 🎯 Next Steps

1. **Immediate** (Today):
   - Re-enable Safari extraction with proper error handling
   - Add delay/retry logic for clipboard content capture

2. **Short-term** (This Week):
   - Implement QuickTime file path extraction
   - Add comprehensive error logging

3. **Long-term** (Next Sprint):
   - Refactor clipboard monitoring to event-driven model
   - Add unit tests for all extraction methods
   - Implement fallback chains for all data extraction

---

## 📝 Notes

- The codebase shows evidence of iterative development with multiple `main_*.rs` files
- The Safari crash issue suggests possible memory management problems with Core Foundation objects
- Consider using a more robust clipboard monitoring library like `clipboard` or `arboard`
- The race condition in clipboard capture is a common issue in system-level monitoring tools

---

### 4. File Copy vs Content Copy Confusion
**Severity**: Medium  
**Impact**: User expectation mismatch  
**Status**: Working as designed, but confusing

#### Problem Description
When copying a file from tree view (Finder, IDE file explorer), the clipboard contains file references/paths, not the file contents.

#### Current Behavior
```json
{
    "event_type": "copy",
    "content": null,  // No text content
    "file_paths": ["/Users/tsavoknott/.../test_enhanced.sh"],  // File reference
    "content_type": "files"
}
```

#### User Expectation
Users might expect the file contents to be captured when copying a file.

#### Solution Options

**Option 1: Auto-read file contents when file is copied**
```rust
fn handle_file_copy(&mut self, file_paths: Vec<String>) {
    // If single text file is copied, read its contents
    if file_paths.len() == 1 {
        let path = &file_paths[0];
        if is_text_file(path) {
            if let Ok(contents) = std::fs::read_to_string(path) {
                // Add file contents to event
                self.file_contents = Some(contents);
            }
        }
    }
}
```

**Option 2: Add separate field for file contents**
```rust
struct ClipboardEvent {
    // ... existing fields ...
    file_contents: Option<HashMap<String, String>>, // path -> contents
}
```

**Option 3: Document the difference clearly**
- File copy = file reference (for file operations)
- Text copy = actual content (for text operations)
- To copy file contents: Open file → Select All → Copy

---

*Report generated from analysis of test.json output and src/main.rs implementation*
