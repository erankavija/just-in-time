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

    /// Scan a document in the working tree for internal links to other documents
    pub fn scan_document_links(&self, doc_path: &Path) -> Result<Vec<InternalLink>> {
        let content = std::fs::read_to_string(self.repo_root.join(doc_path))?;
        Ok(Self::scan_links(&content))
    }

    /// Scan document content for internal links to other documents
    ///
    /// Uses pulldown_cmark parser to properly handle Markdown structure.
    /// Skips links inside code blocks (fenced and indented).
    /// Note: Inline code cannot contain links in Markdown, so we don't need to track it.
    ///
    /// Taking content rather than a path lets a caller scan the version a
    /// document reference names, such as a commit-pinned document that the
    /// working tree no longer carries.
    pub fn scan_links(content: &str) -> Vec<InternalLink> {
        use pulldown_cmark::{Event, Parser, Tag, TagEnd};

        let mut links = Vec::new();

        let parser = Parser::new(content);

        // Track whether we're inside a code block
        let mut in_code_block = false;

        for event in parser {
            match event {
                // Track code block boundaries
                Event::Start(Tag::CodeBlock(_)) => {
                    in_code_block = true;
                }
                Event::End(TagEnd::CodeBlock) => {
                    in_code_block = false;
                }
                // Process links only if not in code block
                // Note: Inline code (`...`) cannot contain Markdown links, so no need to track it
                Event::Start(Tag::Link { dest_url, .. }) if !in_code_block => {
                    let url = dest_url.as_ref().trim();

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

        links
    }

    /// Validate a single link from a document against the working tree
    pub fn validate_link(&self, from_doc: &Path, link: &InternalLink) -> LinkValidationResult {
        self.validate_link_at(from_doc, link, |target| {
            self.all_document_paths.contains(target) || {
                let full_path = self.repo_root.join(target);
                full_path.exists() && full_path.is_file()
            }
        })
    }

    /// Validate a single link from a document, asking `holds_target` whether the
    /// resolved repository-relative path holds a file.
    ///
    /// The predicate names the version the link is read at, so a commit-pinned
    /// document's links resolve at its commit rather than in the working tree.
    pub fn validate_link_at(
        &self,
        from_doc: &Path,
        link: &InternalLink,
        holds_target: impl Fn(&Path) -> bool,
    ) -> LinkValidationResult {
        // A same-document anchor names no path, and is always valid
        let Some(normalized) = self.resolve_target(from_doc, link) else {
            return LinkValidationResult::Valid;
        };

        // A link must hold a file in the version being checked. In particular,
        // a target currently registered by another document may not have
        // existed at a pinned document's commit.
        if !holds_target(&normalized) {
            LinkValidationResult::Broken {
                reason: format!(
                    "Document '{}' not found (resolved to {})",
                    link.target,
                    normalized.display()
                ),
            }
        } else if self.all_document_paths.contains(&normalized) {
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
        } else if link.link_type == LinkType::RootRelative && self.is_permanent_path(&normalized) {
            // Root-relative links to permanent paths (docs/, README.md) are safe.
            LinkValidationResult::Valid
        } else {
            // The file exists at the named version but is not tracked as a document.
            LinkValidationResult::Risky {
                warning: format!(
                    "Link to '{}' exists but is not tracked as a document",
                    link.target
                ),
            }
        }
    }

    /// The normalized repository-relative path a link from `from_doc` names.
    ///
    /// Returns `None` for a same-document anchor, which names no path. Callers
    /// that must know a link's target before reading it — capturing the target's
    /// evidence at a pinned commit, for instance — resolve it here, so the path
    /// they read is the one [`validate_link_at`](Self::validate_link_at) asks
    /// about.
    pub fn resolve_target(&self, from_doc: &Path, link: &InternalLink) -> Option<PathBuf> {
        let from_dir = from_doc.parent().unwrap_or(Path::new(""));
        let target_path = match link.link_type {
            // Root-relative: starts with /
            LinkType::RootRelative => PathBuf::from(link.target.trim_start_matches('/')),
            // Relative to document location
            LinkType::Relative => from_dir.join(&link.target),
            LinkType::Anchor => return None,
        };
        Some(self.normalize_path(&target_path))
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

    /// Check if a path is to permanent documentation
    ///
    /// Permanent paths are expected to be stable and not move during archival.
    fn is_permanent_path(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        // Permanent documentation paths
        path_str.starts_with("docs/")
            || path_str.starts_with("README.md")
            || path_str.starts_with("CONTRIBUTING.md")
            || path_str.starts_with("LICENSE")
            || path_str.starts_with("dev/architecture/") // Architecture docs are permanent
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
    fn test_validate_link_at_requires_registered_target_at_named_version() {
        let temp_dir = tempfile::tempdir().unwrap();
        let from = PathBuf::from("docs/report.md");
        let target = PathBuf::from("docs/appendix.md");
        let validator = LinkValidator::new(
            temp_dir.path().to_path_buf(),
            vec![from.clone(), target.clone()],
        );
        let link = InternalLink {
            target: "appendix.md".to_string(),
            line_number: 1,
            link_type: LinkType::Relative,
        };

        // A target registered in the working tree may have been introduced
        // after the pinned document's commit, so registration alone cannot
        // satisfy a versioned link check.
        assert!(matches!(
            validator.validate_link(&from, &link),
            LinkValidationResult::Valid
        ));
        assert!(matches!(
            validator.validate_link_at(&from, &link, |path| path == target),
            LinkValidationResult::Valid
        ));
        assert!(matches!(
            validator.validate_link_at(&from, &link, |_| false),
            LinkValidationResult::Broken { .. }
        ));
    }

    #[test]
    fn test_resolve_target_normalizes_relative_targets_and_skips_anchors() {
        let validator = LinkValidator::new(PathBuf::from("/tmp"), vec![]);
        let from = PathBuf::from("docs/guide/report.md");

        assert_eq!(
            validator.resolve_target(
                &from,
                &InternalLink {
                    target: "../assets/diagram.png".to_string(),
                    line_number: 1,
                    link_type: LinkType::Relative,
                }
            ),
            Some(PathBuf::from("docs/assets/diagram.png"))
        );
        assert_eq!(
            validator.resolve_target(
                &from,
                &InternalLink {
                    target: "/docs/index.md".to_string(),
                    line_number: 1,
                    link_type: LinkType::RootRelative,
                }
            ),
            Some(PathBuf::from("docs/index.md"))
        );
        assert_eq!(
            validator.resolve_target(
                &from,
                &InternalLink {
                    target: "section".to_string(),
                    line_number: 1,
                    link_type: LinkType::Anchor,
                }
            ),
            None
        );
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
