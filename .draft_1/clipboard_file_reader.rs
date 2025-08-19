// clipboard_file_reader.rs - Enhancement to read file contents when files are copied

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Determines if a file is likely a text file based on extension
pub fn is_text_file(path: &str) -> bool {
    let text_extensions = [
        // Source code
        "rs",
        "c",
        "cpp",
        "cc",
        "h",
        "hpp",
        "java",
        "kt",
        "swift",
        "m",
        "mm",
        "py",
        "rb",
        "go",
        "js",
        "ts",
        "jsx",
        "tsx",
        "vue",
        "svelte",
        "php",
        "cs",
        "vb",
        "fs",
        "ml",
        "clj",
        "scala",
        "hs",
        "elm",
        "erl",
        "ex",
        "exs",
        "lua",
        "r",
        "jl",
        "nim",
        "zig",
        "v",
        // Web
        "html",
        "htm",
        "css",
        "scss",
        "sass",
        "less",
        "xml",
        "svg",
        // Data/Config
        "json",
        "yaml",
        "yml",
        "toml",
        "ini",
        "conf",
        "config",
        "env",
        "properties",
        "plist",
        "lock",
        // Scripts
        "sh",
        "bash",
        "zsh",
        "fish",
        "ps1",
        "bat",
        "cmd",
        // Documentation
        "md",
        "markdown",
        "rst",
        "txt",
        "text",
        "log",
        "csv",
        "adoc",
        "org",
        "tex",
        "rtf",
        // Build/Project files
        "Makefile",
        "makefile",
        "cmake",
        "gradle",
        "sbt",
        "Dockerfile",
        "dockerfile",
        "dockerignore",
        "gitignore",
        "gitattributes",
        "editorconfig",
    ];

    // Check if path has an extension
    if let Some(extension) = Path::new(path).extension() {
        if let Some(ext_str) = extension.to_str() {
            return text_extensions.contains(&ext_str.to_lowercase().as_str());
        }
    }

    // Check for known filenames without extensions
    let filename = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    matches!(
        filename,
        "Makefile"
            | "makefile"
            | "Dockerfile"
            | "dockerfile"
            | "README"
            | "LICENSE"
            | "CHANGELOG"
            | "AUTHORS"
            | "CONTRIBUTORS"
            | ".gitignore"
            | ".dockerignore"
            | ".env"
            | ".editorconfig"
    )
}

/// Reads file contents if it's a readable text file
pub fn read_file_contents_safe(path: &str, max_size: usize) -> Option<String> {
    // Check if file exists and is a regular file
    let path_obj = Path::new(path);
    if !path_obj.exists() || !path_obj.is_file() {
        return None;
    }

    // Check file size to avoid reading huge files
    if let Ok(metadata) = fs::metadata(path) {
        if metadata.len() > max_size as u64 {
            return Some(format!("[File too large: {} bytes]", metadata.len()));
        }
    }

    // Only read text files
    if !is_text_file(path) {
        return Some(format!(
            "[Binary file: {}]",
            path_obj.file_name()?.to_str()?
        ));
    }

    // Try to read the file
    match fs::read_to_string(path) {
        Ok(contents) => {
            // Truncate if still too long
            if contents.len() > max_size {
                Some(format!(
                    "{}...\n[Truncated at {} bytes]",
                    &contents[..max_size],
                    max_size
                ))
            } else {
                Some(contents)
            }
        }
        Err(e) => Some(format!("[Error reading file: {}]", e)),
    }
}

/// Reads contents of multiple files
pub fn read_multiple_file_contents(
    paths: &[String],
    max_size_per_file: usize,
) -> HashMap<String, String> {
    let mut contents = HashMap::new();

    for path in paths {
        if let Some(content) = read_file_contents_safe(path, max_size_per_file) {
            contents.insert(path.clone(), content);
        }
    }

    contents
}

/// Enhanced clipboard event with file contents
#[derive(Debug, Clone, serde::Serialize)]
pub struct EnhancedClipboardEvent {
    #[serde(flatten)]
    pub base_event: super::ClipboardEvent,

    /// Actual contents of copied files (if they're text files)
    pub file_contents: Option<HashMap<String, String>>,

    /// Summary of what was copied
    pub summary: String,
}

impl EnhancedClipboardEvent {
    pub fn from_base(mut base: super::ClipboardEvent) -> Self {
        let mut file_contents = None;
        let mut summary = String::new();

        // If files were copied, try to read their contents
        if !base.file_paths.is_empty() {
            let contents = read_multiple_file_contents(&base.file_paths, 10_000); // 10KB max per file

            if !contents.is_empty() {
                file_contents = Some(contents.clone());

                // Create a helpful summary
                if base.file_paths.len() == 1 {
                    let path = &base.file_paths[0];
                    if let Some(content) = contents.get(path) {
                        if content.starts_with("[") {
                            // It's a status message, not actual content
                            summary = format!(
                                "File copied: {} {}",
                                Path::new(path)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or(path),
                                content
                            );
                        } else {
                            // We have actual content
                            let preview = if content.len() > 100 {
                                format!("{}...", &content[..100])
                            } else {
                                content.clone()
                            };
                            summary = format!(
                                "File copied with content: {}\nPreview: {}",
                                Path::new(path)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or(path),
                                preview
                            );

                            // Also put a preview in the main content field
                            if base.content.is_none() {
                                base.content = Some(preview);
                            }
                        }
                    }
                } else {
                    summary = format!("{} files copied", base.file_paths.len());
                }
            } else {
                summary = format!("{} file(s) copied (paths only)", base.file_paths.len());
            }
        } else if let Some(ref content) = base.content {
            // Regular text copy
            let preview = if content.len() > 100 {
                format!("{}...", &content[..100])
            } else {
                content.clone()
            };
            summary = format!("Text copied: {}", preview);
        } else {
            summary = "Unknown clipboard content".to_string();
        }

        Self {
            base_event: base,
            file_contents,
            summary,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_text_file() {
        assert!(is_text_file("test.rs"));
        assert!(is_text_file("script.sh"));
        assert!(is_text_file("data.json"));
        assert!(is_text_file("Makefile"));
        assert!(is_text_file(".gitignore"));

        assert!(!is_text_file("image.png"));
        assert!(!is_text_file("binary.exe"));
        assert!(!is_text_file("video.mp4"));
    }

    #[test]
    fn test_read_file_contents_safe() {
        // This would need actual test files to work
        // Just testing the logic here

        // Non-existent file
        assert!(read_file_contents_safe("/nonexistent/file.txt", 1000).is_none());

        // Binary file (simulated by extension)
        let result = read_file_contents_safe("test.exe", 1000);
        assert!(result.is_none() || result.unwrap().contains("Binary file"));
    }
}
