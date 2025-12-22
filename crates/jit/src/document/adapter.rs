//! Document format adapter trait and registry
//!
//! Provides a pluggable system for handling different document formats.
//! Each format (Markdown, AsciiDoc, etc.) implements the `DocFormatAdapter` trait
//! to provide format-specific operations.

use std::collections::HashSet;
use std::path::Path;

/// Trait for document format adapters
///
/// Adapters handle format-specific operations like asset scanning and link rewriting.
/// Each format (Markdown, AsciiDoc, reStructuredText, MDX) implements this trait.
///
/// # Example
///
/// ```
/// use jit::document::{DocFormatAdapter, MarkdownAdapter};
///
/// let adapter = MarkdownAdapter;
/// assert_eq!(adapter.id(), "markdown");
/// assert!(adapter.supports_path("readme.md"));
/// ```
pub trait DocFormatAdapter {
    /// Returns the adapter identifier (e.g., "markdown", "asciidoc")
    fn id(&self) -> &str;

    /// Check if this adapter supports the given file path based on extension
    fn supports_path(&self, path: &str) -> bool;

    /// Detect format from content (content-based detection)
    ///
    /// Returns true if the content appears to be in this format.
    /// Used as fallback when extension-based detection fails.
    fn detect(&self, content: &str) -> bool;

    /// Extract asset references from document content
    ///
    /// Returns a set of unique asset paths referenced in the document.
    /// Includes images and other embedded assets.
    /// Excludes external URLs and anchor-only links.
    fn scan_assets(&self, content: &str) -> HashSet<String>;

    /// Rewrite links when documents or assets move
    ///
    /// Updates links in content when files are moved.
    /// Currently a stub for Phase 2 implementation.
    fn rewrite_links(&self, _content: &str, _old_path: &str, _new_path: &str) -> String {
        unimplemented!("Link rewriting not implemented in Phase 1")
    }
}

/// Registry for managing document format adapters
///
/// Maintains a collection of format adapters and resolves the appropriate
/// adapter for a given file path and content.
///
/// # Example
///
/// ```
/// use jit::document::{AdapterRegistry, MarkdownAdapter};
///
/// let mut registry = AdapterRegistry::new();
/// registry.register(Box::new(MarkdownAdapter));
///
/// let adapter = registry.resolve("readme.md", "# Hello").unwrap();
/// assert_eq!(adapter.id(), "markdown");
/// ```
pub struct AdapterRegistry {
    adapters: Vec<Box<dyn DocFormatAdapter>>,
}

impl AdapterRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
        }
    }

    /// Create a registry with built-in adapters registered
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(MarkdownAdapter));
        registry
    }

    /// Register a format adapter
    pub fn register(&mut self, adapter: Box<dyn DocFormatAdapter>) {
        self.adapters.push(adapter);
    }

    /// Resolve adapter for a file path and content
    ///
    /// Tries extension-based matching first, then falls back to content detection.
    /// Returns None if no adapter matches.
    pub fn resolve(&self, path: &str, content: &str) -> Option<&dyn DocFormatAdapter> {
        // Try extension match first
        for adapter in &self.adapters {
            if adapter.supports_path(path) {
                return Some(adapter.as_ref());
            }
        }

        // Fallback to content detection
        for adapter in &self.adapters {
            if adapter.detect(content) {
                return Some(adapter.as_ref());
            }
        }

        None
    }
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

/// Markdown format adapter
///
/// Supports standard Markdown files with `.md` and `.markdown` extensions.
/// Extracts image and link references from Markdown syntax.
///
/// # Supported Syntax
///
/// - Images: `![alt](path)`
/// - Links: `[text](path)`
/// - Relative paths: `./image.png`, `../doc.md`
/// - Root-relative paths: `/assets/image.png`
///
/// # Excluded
///
/// - External URLs: `https://example.com/image.png`
/// - Anchor-only links: `#section`
/// - Mailto links: `mailto:user@example.com`
pub struct MarkdownAdapter;

impl DocFormatAdapter for MarkdownAdapter {
    fn id(&self) -> &str {
        "markdown"
    }

    fn supports_path(&self, path: &str) -> bool {
        let path_obj = Path::new(path);
        if let Some(ext) = path_obj.extension() {
            let ext_str = ext.to_string_lossy().to_lowercase();
            ext_str == "md" || ext_str == "markdown"
        } else {
            false
        }
    }

    fn detect(&self, content: &str) -> bool {
        // Simple heuristic: contains markdown heading or link syntax
        content.contains("# ") || content.contains("](") || content.contains("![")
    }

    fn scan_assets(&self, content: &str) -> HashSet<String> {
        let mut assets = HashSet::new();

        // Regex patterns for Markdown links and images
        // Pattern: ![alt](path) or [text](path)
        let link_pattern = regex::Regex::new(r"!?\[(?:[^\]]+)\]\(([^)]+)\)").unwrap();

        for cap in link_pattern.captures_iter(content) {
            if let Some(path_match) = cap.get(1) {
                let path = path_match.as_str().trim();

                // Skip anchor-only links
                if path.starts_with('#') {
                    continue;
                }

                // Skip mailto links
                if path.starts_with("mailto:") {
                    continue;
                }

                // Remove anchor fragments
                let path_without_anchor = if let Some(pos) = path.find('#') {
                    &path[..pos]
                } else {
                    path
                };

                // Add non-empty paths (including external URLs)
                if !path_without_anchor.is_empty() {
                    assets.insert(path_without_anchor.to_string());
                }
            }
        }

        assets
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_markdown_adapter_id() {
        let adapter = MarkdownAdapter;
        assert_eq!(adapter.id(), "markdown");
    }

    #[test]
    fn test_markdown_supports_path() {
        let adapter = MarkdownAdapter;

        assert!(adapter.supports_path("README.md"));
        assert!(adapter.supports_path("docs/guide.md"));
        assert!(adapter.supports_path("file.markdown"));
        assert!(adapter.supports_path("FILE.MD"));

        assert!(!adapter.supports_path("file.txt"));
        assert!(!adapter.supports_path("file.adoc"));
        assert!(!adapter.supports_path("file"));
    }

    #[test]
    fn test_markdown_detect() {
        let adapter = MarkdownAdapter;

        assert!(adapter.detect("# Heading\n\nSome text"));
        assert!(adapter.detect("Check [this link](path)"));
        assert!(adapter.detect("Image: ![alt](image.png)"));

        assert!(!adapter.detect("Plain text without markdown"));
        assert!(!adapter.detect(""));
    }

    #[test]
    fn test_markdown_scan_assets_basic() {
        let adapter = MarkdownAdapter;
        let content = r#"
# Document

Here's an image: ![Logo](./assets/logo.png)
And a link: [Guide](../docs/guide.md)
        "#;

        let assets = adapter.scan_assets(content);
        assert_eq!(assets.len(), 2);
        assert!(assets.contains("./assets/logo.png"));
        assert!(assets.contains("../docs/guide.md"));
    }

    #[test]
    fn test_markdown_scan_assets_includes_external_urls() {
        let adapter = MarkdownAdapter;
        let content = r#"
External image: ![External](https://example.com/image.png)
External link: [Site](http://example.com)
Local image: ![Local](local.png)
        "#;

        let assets = adapter.scan_assets(content);
        assert_eq!(assets.len(), 3);
        assert!(assets.contains("https://example.com/image.png"));
        assert!(assets.contains("http://example.com"));
        assert!(assets.contains("local.png"));
    }

    #[test]
    fn test_markdown_scan_assets_excludes_anchors() {
        let adapter = MarkdownAdapter;
        let content = r#"
Anchor only: [Section](#section)
File with anchor: [Doc](doc.md#section)
Plain file: [Other](other.md)
        "#;

        let assets = adapter.scan_assets(content);
        assert_eq!(assets.len(), 2);
        assert!(assets.contains("doc.md"));
        assert!(assets.contains("other.md"));
    }

    #[test]
    fn test_markdown_scan_assets_excludes_mailto() {
        let adapter = MarkdownAdapter;
        let content = r#"
Email: [Contact](mailto:user@example.com)
File: [Doc](file.md)
        "#;

        let assets = adapter.scan_assets(content);
        assert_eq!(assets.len(), 1);
        assert!(assets.contains("file.md"));
    }

    #[test]
    fn test_markdown_scan_assets_deduplicates() {
        let adapter = MarkdownAdapter;
        let content = r#"
First: [One](file.md)
Second: [Two](file.md)
Third: [Three](other.md)
        "#;

        let assets = adapter.scan_assets(content);
        assert_eq!(assets.len(), 2);
        assert!(assets.contains("file.md"));
        assert!(assets.contains("other.md"));
    }

    #[test]
    fn test_markdown_scan_assets_root_relative() {
        let adapter = MarkdownAdapter;
        let content = r#"
Root relative: ![Image](/assets/image.png)
Regular relative: ![Other](./local.png)
        "#;

        let assets = adapter.scan_assets(content);
        assert_eq!(assets.len(), 2);
        assert!(assets.contains("/assets/image.png"));
        assert!(assets.contains("./local.png"));
    }

    #[test]
    fn test_registry_new_is_empty() {
        let registry = AdapterRegistry::new();
        assert_eq!(registry.adapters.len(), 0);
    }

    #[test]
    fn test_registry_with_builtins() {
        let registry = AdapterRegistry::with_builtins();
        assert_eq!(registry.adapters.len(), 1);
    }

    #[test]
    fn test_registry_register() {
        let mut registry = AdapterRegistry::new();
        registry.register(Box::new(MarkdownAdapter));
        assert_eq!(registry.adapters.len(), 1);
    }

    #[test]
    fn test_registry_resolve_by_extension() {
        let registry = AdapterRegistry::with_builtins();
        let adapter = registry.resolve("readme.md", "Plain text").unwrap();
        assert_eq!(adapter.id(), "markdown");
    }

    #[test]
    fn test_registry_resolve_by_content() {
        let registry = AdapterRegistry::with_builtins();
        // File without .md extension but with markdown content
        let adapter = registry.resolve("README", "# Heading\n\nText").unwrap();
        assert_eq!(adapter.id(), "markdown");
    }

    #[test]
    fn test_registry_resolve_returns_none_for_unknown() {
        let registry = AdapterRegistry::with_builtins();
        let result = registry.resolve("file.xyz", "Plain text content");
        assert!(result.is_none());
    }

    #[test]
    fn test_registry_default() {
        let registry = AdapterRegistry::default();
        assert_eq!(registry.adapters.len(), 1);
        let adapter = registry.resolve("test.md", "").unwrap();
        assert_eq!(adapter.id(), "markdown");
    }
}
