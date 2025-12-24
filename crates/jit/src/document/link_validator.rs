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
    pub fn scan_document_links(&self, doc_path: &Path) -> Result<Vec<InternalLink>> {
        let content = std::fs::read_to_string(self.repo_root.join(doc_path))?;
        let mut links = Vec::new();

        for (line_num, line) in content.lines().enumerate() {
            // Match Markdown links: [text](url) but NOT images: ![alt](url)
            let mut chars = line.char_indices().peekable();
            while let Some((idx, ch)) = chars.next() {
                if ch == '[' {
                    // Check if this is an image link by looking back for !
                    let is_image = idx > 0 && line.chars().nth(idx - 1) == Some('!');

                    if is_image {
                        continue; // Skip image links
                    }

                    // Find closing ]
                    let remaining: String = chars.clone().map(|(_, c)| c).collect();
                    if let Some(close_bracket) = remaining.find(']') {
                        // Skip ahead
                        for _ in 0..close_bracket {
                            chars.next();
                        }
                        chars.next(); // Skip ]

                        // Check for (url)
                        if let Some((_, '(')) = chars.peek() {
                            chars.next(); // Skip (
                            let url_start: String = chars.clone().map(|(_, c)| c).collect();
                            if let Some(close_paren) = url_start.find(')') {
                                let url = &url_start[..close_paren];

                                // Skip external URLs and anchors
                                if !url.starts_with("http://")
                                    && !url.starts_with("https://")
                                    && !url.starts_with('#')
                                {
                                    links.push(InternalLink {
                                        target: url.to_string(),
                                        line_number: line_num + 1,
                                        link_type: if url.starts_with('/') {
                                            LinkType::RootRelative
                                        } else {
                                            LinkType::Relative
                                        },
                                    });
                                }
                            }
                        }
                    }
                }
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
}
