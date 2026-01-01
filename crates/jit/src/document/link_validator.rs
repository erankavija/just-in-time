//! Internal document link validation
//!
//! Validates that Markdown links between documents resolve correctly
//! and won't break during archival operations.

use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Validates internal document links
pub struct LinkValidator {
    repo_root: PathBuf,
    all_document_paths: HashSet<PathBuf>,
}

impl LinkValidator {
    /// Create a new link validator
    pub fn new(repo_root: PathBuf, document_paths: Vec<PathBuf>) -> Self {
        let all_document_paths = document_paths.into_iter().collect();
        Self {
            repo_root,
            all_document_paths,
        }
    }

    /// Scan document for internal links to other documents
    ///
    /// Uses pulldown_cmark parser to properly handle Markdown structure.
    /// Skips links inside code blocks (fenced and indented).
    /// Note: Inline code cannot contain links in Markdown, so we don't need to track it.
    pub fn scan_document_links(&self, doc_path: &Path) -> Result<Vec<InternalLink>> {
        use pulldown_cmark::{Event, Parser, Tag};

        let content = std::fs::read_to_string(self.repo_root.join(doc_path))?;
        let mut links = Vec::new();

        let parser = Parser::new(&content);

        // Track whether we're inside a code block
        let mut in_code_block = false;

        for event in parser {
            match event {
                // Track code block boundaries
                Event::Start(Tag::CodeBlock(_)) => {
                    in_code_block = true;
                }
                Event::End(Tag::CodeBlock(_)) => {
                    in_code_block = false;
                }
                // Process links only if not in code block
                // Note: Inline code (`...`) cannot contain Markdown links, so no need to track it
                Event::Start(Tag::Link(_, dest, _)) if !in_code_block => {
                    let url = dest.as_ref().trim();

                    // Skip external URLs
                    if url.starts_with("http://") || url.starts_with("https://") {
                        continue;
                    }

                    // Skip mailto links
                    if url.starts_with("mailto:") {
                        continue;
                    }

                    // Skip anchor-only links
                    if url.starts_with('#') {
                        continue;
                    }

                    // Remove anchor fragments from URLs
                    let url_without_anchor = if let Some(pos) = url.find('#') {
                        &url[..pos]
                    } else {
                        url
                    };

                    // Skip empty paths
                    if url_without_anchor.is_empty() {
                        continue;
                    }

                    links.push(InternalLink {
                        target: url_without_anchor.to_string(),
                        line_number: 0, // Line number tracking requires more complex parsing
                        link_type: if url_without_anchor.starts_with('/') {
                            LinkType::RootRelative
                        } else {
                            LinkType::Relative
                        },
                    });
                }
                _ => {}
            }
        }

        Ok(links)
    }

    /// Validate a single link from a document
    pub fn validate_link(&self, from_doc: &Path, link: &InternalLink) -> LinkValidationResult {
        let from_dir = from_doc.parent().unwrap_or(Path::new(""));

        // Resolve the target path
        let target_path = match link.link_type {
            LinkType::RootRelative => {
                // Root-relative: starts with /
                PathBuf::from(link.target.trim_start_matches('/'))
            }
            LinkType::Relative => {
                // Relative to document location
                from_dir.join(&link.target)
            }
            LinkType::Anchor => {
                // Same-document anchor, always valid
                return LinkValidationResult::Valid;
            }
        };

        // Normalize path (resolve .. and .)
        let normalized = self.normalize_path(&target_path);

        // Check if target exists in our document set
        if self.all_document_paths.contains(&normalized) {
            // Check if it's risky
            if self.is_risky_path(&link.target) {
                LinkValidationResult::Risky {
                    warning: format!(
                        "Deep relative path '{}' may break if document is moved",
                        link.target
                    ),
                }
            } else {
                LinkValidationResult::Valid
            }
        } else {
            // Check if it exists in the filesystem
            let full_path = self.repo_root.join(&normalized);
            if full_path.exists() && full_path.is_file() {
                // File exists but not tracked as a document
                LinkValidationResult::Risky {
                    warning: format!(
                        "Link to '{}' exists but is not tracked as a document",
                        link.target
                    ),
                }
            } else {
                LinkValidationResult::Broken {
                    reason: format!(
                        "Document '{}' not found (resolved to {})",
                        link.target,
                        normalized.display()
                    ),
                }
            }
        }
    }

    /// Normalize a path by resolving . and ..
    fn normalize_path(&self, path: &Path) -> PathBuf {
        let mut components = Vec::new();
        for component in path.components() {
            match component {
                std::path::Component::Normal(c) => components.push(c),
                std::path::Component::ParentDir => {
                    components.pop();
                }
                std::path::Component::CurDir => {}
                _ => {}
            }
        }
        components.iter().collect()
    }

    /// Check if a path is risky (deep relative traversal)
    fn is_risky_path(&self, path: &str) -> bool {
        // Count ../ occurrences
        let parent_count = path.matches("../").count();
        parent_count >= 2
    }
}

/// An internal link found in a document
#[derive(Debug, Clone)]
pub struct InternalLink {
    pub target: String,
    pub line_number: usize,
    pub link_type: LinkType,
}

/// Type of link
#[derive(Debug, Clone, PartialEq)]
pub enum LinkType {
    Relative,
    RootRelative,
    Anchor,
}

/// Result of link validation
#[derive(Debug, Clone)]
pub enum LinkValidationResult {
    Valid,
    Broken { reason: String },
    Risky { warning: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_simple_markdown_link() {
        let temp_dir = tempfile::tempdir().unwrap();
        let doc_path = temp_dir.path().join("test.md");
        std::fs::write(&doc_path, "See [other doc](other.md) for details.").unwrap();

        let validator = LinkValidator::new(temp_dir.path().to_path_buf(), vec![]);
        let links = validator.scan_document_links(Path::new("test.md")).unwrap();

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "other.md");
        assert_eq!(links[0].link_type, LinkType::Relative);
    }

    #[test]
    fn test_scan_ignores_external_urls() {
        let temp_dir = tempfile::tempdir().unwrap();
        let doc_path = temp_dir.path().join("test.md");
        std::fs::write(
            &doc_path,
            "See [example](https://example.com) and [local](other.md).",
        )
        .unwrap();

        let validator = LinkValidator::new(temp_dir.path().to_path_buf(), vec![]);
        let links = validator.scan_document_links(Path::new("test.md")).unwrap();

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "other.md");
    }

    #[test]
    fn test_validate_link_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let doc1 = PathBuf::from("docs/doc1.md");
        let doc2 = PathBuf::from("docs/doc2.md");

        let validator = LinkValidator::new(
            temp_dir.path().to_path_buf(),
            vec![doc1.clone(), doc2.clone()],
        );

        let link = InternalLink {
            target: "doc2.md".to_string(),
            line_number: 1,
            link_type: LinkType::Relative,
        };

        let result = validator.validate_link(&doc1, &link);
        match result {
            LinkValidationResult::Valid => {}
            _ => panic!("Expected valid link"),
        }
    }

    #[test]
    fn test_validate_link_broken() {
        let temp_dir = tempfile::tempdir().unwrap();
        let doc1 = PathBuf::from("docs/doc1.md");

        let validator = LinkValidator::new(temp_dir.path().to_path_buf(), vec![doc1.clone()]);

        let link = InternalLink {
            target: "nonexistent.md".to_string(),
            line_number: 1,
            link_type: LinkType::Relative,
        };

        let result = validator.validate_link(&doc1, &link);
        match result {
            LinkValidationResult::Broken { .. } => {}
            _ => panic!("Expected broken link"),
        }
    }

    #[test]
    fn test_risky_path_detection() {
        let validator = LinkValidator::new(PathBuf::from("/tmp"), vec![]);

        assert!(validator.is_risky_path("../../other/doc.md"));
        assert!(!validator.is_risky_path("../doc.md"));
        assert!(!validator.is_risky_path("doc.md"));
    }

    #[test]
    fn test_scan_skips_code_blocks() {
        let temp_dir = tempfile::tempdir().unwrap();
        let doc_path = temp_dir.path().join("test.md");
        std::fs::write(
            &doc_path,
            r#"# Document

This is a real link: [real](real.md)

```markdown
This is a fake link in a code block: [fake](fake.md)
```

Another real link: [another](another.md)
"#,
        )
        .unwrap();

        let validator = LinkValidator::new(temp_dir.path().to_path_buf(), vec![]);
        let links = validator.scan_document_links(Path::new("test.md")).unwrap();

        // Should only find the two real links, not the one in the code block
        assert_eq!(links.len(), 2);
        let targets: Vec<_> = links.iter().map(|l| l.target.as_str()).collect();
        assert!(targets.contains(&"real.md"));
        assert!(targets.contains(&"another.md"));
        assert!(!targets.contains(&"fake.md"));
    }

    #[test]
    fn test_scan_handles_inline_code_correctly() {
        let temp_dir = tempfile::tempdir().unwrap();
        let doc_path = temp_dir.path().join("test.md");
        // Note: In Markdown, inline code cannot contain actual links
        // The backticks prevent link parsing, so `[text](url)` is literal text
        std::fs::write(
            &doc_path,
            "Real link: [doc](doc.md). Inline code with literal brackets: `[not parsed as link](fake.md)`. Another: [other](other.md)",
        )
        .unwrap();

        let validator = LinkValidator::new(temp_dir.path().to_path_buf(), vec![]);
        let links = validator.scan_document_links(Path::new("test.md")).unwrap();

        // Should find both real links; inline code content is not parsed as Markdown
        assert_eq!(links.len(), 2);
        let targets: Vec<_> = links.iter().map(|l| l.target.as_str()).collect();
        assert!(targets.contains(&"doc.md"));
        assert!(targets.contains(&"other.md"));
    }
}
