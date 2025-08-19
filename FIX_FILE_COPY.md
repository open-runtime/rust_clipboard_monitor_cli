# Fix: Reading File Contents When Files Are Copied

## The Issue
When you copy a file from the tree view (Finder, VS Code, etc.), the clipboard contains a **file reference** (the path), not the file's contents. This is standard OS behavior:

- **Copy file** = Copy file reference (for file operations like move/paste)
- **Copy text** = Copy actual content (for text operations)

## Current Behavior
```json
{
  "event_type": "copy",
  "content": null,              // ❌ No text content
  "file_paths": ["/Users/.../test.sh"],  // ✅ File path is captured
  "content_type": "files"
}
```

## Quick Fix: Add to your main.rs

Add this enhancement to automatically read file contents when files are copied:

```rust
// In your handle_clipboard_shortcut or clipboard monitoring function:

fn handle_clipboard_shortcut(&mut self, action: &str) {
    match action {
        "copy" | "cut" => {
            // ... existing code ...
            
            // Get file paths if any
            let file_paths = self.get_clipboard_file_paths();
            
            // NEW: If files were copied, try to read their contents
            let mut file_contents = None;
            if !file_paths.is_empty() && file_paths.len() == 1 {
                // Single file copied - try to read it
                let path = &file_paths[0];
                
                // Check if it's a text file (by extension)
                if path.ends_with(".sh") || path.ends_with(".txt") || 
                   path.ends_with(".md") || path.ends_with(".rs") || 
                   path.ends_with(".json") || path.ends_with(".yml") ||
                   path.ends_with(".toml") || path.ends_with(".py") ||
                   path.ends_with(".js") || path.ends_with(".ts") {
                    
                    // Try to read the file
                    if let Ok(contents) = std::fs::read_to_string(path) {
                        // Limit to first 10KB to avoid huge files
                        let preview = if contents.len() > 10000 {
                            format!("{}...\n[Truncated]", &contents[..10000])
                        } else {
                            contents
                        };
                        file_contents = Some(preview.clone());
                        
                        // IMPORTANT: Also set the content field
                        content = Some(preview);
                    }
                }
            }
            
            let event = ClipboardEvent {
                timestamp: self.start_time.elapsed().as_millis(),
                event_type: action.to_string(),
                content: content,  // Now includes file contents if available
                content_type: self.get_clipboard_type(),
                // ... rest of fields ...
            };
        }
    }
}
```

## Complete Solution: Use the clipboard_file_reader module

1. **Add the module** (already created as `src/clipboard_file_reader.rs`)

2. **Import in main.rs:**
```rust
mod clipboard_file_reader;
use clipboard_file_reader::{read_file_contents_safe, EnhancedClipboardEvent};
```

3. **Modify your clipboard event creation:**
```rust
// When clipboard changes are detected:
let mut event = ClipboardEvent {
    // ... your existing fields ...
};

// Enhance the event with file contents
if !event.file_paths.is_empty() {
    // Try to read contents of text files
    for path in &event.file_paths {
        if let Some(contents) = read_file_contents_safe(path, 10_000) {
            // Add to content field if it's the only file
            if event.file_paths.len() == 1 && event.content.is_none() {
                event.content = Some(contents);
            }
        }
    }
}
```

## Testing

1. **Copy a text file from Finder/VS Code tree**
   - Should now show file contents in `content` field

2. **Copy a binary file**
   - Should show `[Binary file: filename]` message

3. **Copy multiple files**
   - Should show file count, individual files can be read

4. **Copy text normally (select text → Cmd+C)**
   - Should work as before

## Alternative Approaches

### Option 1: Separate Field for File Contents
Add a new field to track file contents separately from clipboard text:
```rust
struct ClipboardEvent {
    // ... existing fields ...
    file_contents: Option<HashMap<String, String>>, // NEW
}
```

### Option 2: User Preference
Add a CLI flag to control this behavior:
```rust
#[arg(long, help = "Read contents of copied text files")]
read_file_contents: bool,
```

### Option 3: Different Event Type
Create separate event types:
- `copy_text` - Text was copied
- `copy_files` - Files were copied (with optional content reading)
- `copy_mixed` - Both text and files

## Important Notes

1. **Performance**: Reading large files can slow down clipboard monitoring
2. **Security**: Be careful with sensitive files
3. **Binary files**: The module checks extensions to avoid reading binary files
4. **File size limit**: Default is 10KB per file to avoid memory issues

## Summary

The issue you found is a common confusion between:
- **File operations** (copy file for move/paste)
- **Content operations** (copy text for editing)

The clipboard_file_reader module provides a clean solution to bridge this gap by automatically reading text file contents when files are copied, making the clipboard monitor more useful for tracking actual content flow.
