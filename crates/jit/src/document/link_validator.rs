//! Internal document link validation
//!
//! Validates that Markdown links between documents resolve correctly
//! and won't break during archival operations.

use super::link_target::{classify_link_target, LinkTarget};
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
    /// Uses pulldown_cmark parser to properly handle Markdown structure:
    /// inline, angle-bracket and reference links alike. Skips links inside
    /// code blocks (fenced and indented); inline code cannot contain links.
    ///
    /// Each link is positioned where its destination is written: an inline
    /// link at the link itself, a reference link at its definition.
    ///
    /// Taking content rather than a path lets a caller scan the version a
    /// document reference names, such as a commit-pinned document that the
    /// working tree no longer carries.
    pub fn scan_links(content: &str) -> Vec<InternalLink> {
        use pulldown_cmark::{Event, LinkType as MarkdownLink, Parser, Tag, TagEnd};

        let mut parsed = Parser::new(content).into_offset_iter();
        let events = parsed.by_ref().collect::<Vec<_>>();
        let definitions = parsed.reference_definitions();
        let lines = LineStarts::new(content);
        let mut in_code_block = false;
        let mut links = Vec::new();
        for (event, range) in events {
            match event {
                Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
                Event::End(TagEnd::CodeBlock) => in_code_block = false,
                Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    id,
                    ..
                }) if !in_code_block => {
                    let Some(LinkTarget::Local(target)) = classify_link_target(dest_url.as_ref())
                    else {
                        continue;
                    };
                    let span = match link_type {
                        MarkdownLink::Reference
                        | MarkdownLink::Collapsed
                        | MarkdownLink::Shortcut => definitions
                            .get(id.as_ref())
                            .map_or(range, |definition| definition.span.clone()),
                        _ => range,
                    };
                    let (line_number, column) = lines.position(content, span.start);
                    links.push(InternalLink {
                        target: target.to_string(),
                        line_number,
                        column,
                        span,
                        link_type: if target.starts_with('/') {
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
            if self.all_document_paths.contains(target) {
                crate::document::DocumentTargetKind::File
            } else {
                crate::document::classify_filesystem_target(&self.repo_root.join(target))
            }
        })
    }

    /// Validate a single link from a document, asking `holds_target` whether the
    /// resolved repository-relative path's semantic kind.
    ///
    /// The predicate names the version the link is read at, so a commit-pinned
    /// document's links resolve at its commit rather than in the working tree.
    pub fn validate_link_at(
        &self,
        from_doc: &Path,
        link: &InternalLink,
        holds_target: impl Fn(&Path) -> crate::document::DocumentTargetKind,
    ) -> LinkValidationResult {
        // A same-document anchor names no path, and is always valid
        let Some(normalized) = self.resolve_target(from_doc, link) else {
            return LinkValidationResult::Valid;
        };

        let target_kind = holds_target(&normalized);
        if target_kind == crate::document::DocumentTargetKind::Directory {
            return LinkValidationResult::Valid;
        }

        // A link must hold a file in the version being checked. In particular,
        // a target currently registered by another document may not have
        // existed at a pinned document's commit.
        if target_kind == crate::document::DocumentTargetKind::Missing {
            LinkValidationResult::Broken {
                reason: format!(
                    "Document '{}' not found (resolved to {})",
                    link.target,
                    normalized.display()
                ),
            }
        } else if target_kind == crate::document::DocumentTargetKind::Unsupported {
            LinkValidationResult::Broken {
                reason: format!(
                    "Document '{}' resolves to an unsupported artifact type ({})",
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
    /// 1-based line where the link's destination is written.
    pub line_number: usize,
    /// 1-based character column on that line.
    pub column: usize,
    /// Byte range of the link, or of its reference definition.
    pub span: std::ops::Range<usize>,
    pub link_type: LinkType,
}

/// Byte offsets where each line of a text starts.
struct LineStarts(Vec<usize>);

impl LineStarts {
    fn new(text: &str) -> Self {
        Self(
            std::iter::once(0)
                .chain(text.match_indices('\n').map(|(offset, _)| offset + 1))
                .collect(),
        )
    }

    /// The 1-based line and character column of byte `offset` in `text`.
    fn position(&self, text: &str, offset: usize) -> (usize, usize) {
        let line = self.0.partition_point(|start| *start <= offset);
        let start = self.0[line - 1];
        (line, text[start..offset].chars().count() + 1)
    }
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
            column: 1,
            span: 0..0,
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
            column: 1,
            span: 0..0,
            link_type: LinkType::Relative,
        };

        let result = validator.validate_link(&doc1, &link);
        match result {
            LinkValidationResult::Broken { .. } => {}
            _ => panic!("Expected broken link"),
        }
    }

    #[test]
    fn test_validate_link_to_existing_directory_is_navigation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let doc = PathBuf::from("docs/guide.md");
        std::fs::create_dir_all(temp_dir.path().join("docs/studies")).unwrap();
        let validator = LinkValidator::new(temp_dir.path().to_path_buf(), vec![doc.clone()]);
        let link = InternalLink {
            target: "studies".to_string(),
            line_number: 1,
            column: 1,
            span: 0..0,
            link_type: LinkType::Relative,
        };

        assert!(matches!(
            validator.validate_link(&doc, &link),
            LinkValidationResult::Valid
        ));
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
            column: 1,
            span: 0..0,
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
            validator.validate_link_at(&from, &link, |path| {
                if path == target {
                    crate::document::DocumentTargetKind::File
                } else {
                    crate::document::DocumentTargetKind::Missing
                }
            }),
            LinkValidationResult::Valid
        ));
        assert!(matches!(
            validator.validate_link_at(&from, &link, |_| {
                crate::document::DocumentTargetKind::Missing
            }),
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
                    column: 1,
                    span: 0..0,
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
                    column: 1,
                    span: 0..0,
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
                    column: 1,
                    span: 0..0,
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
    fn test_scan_links_positions_each_link_where_its_destination_is_written() {
        let content = "# Title\n\nSee [ä](inline.md) and [b](<angle.md>).\n\nUse [the ref][r].\n\n[r]: ref.md\n";

        let positions = LinkValidator::scan_links(content)
            .into_iter()
            .map(|link| {
                let line = content.lines().nth(link.line_number - 1).unwrap();
                let at = line.chars().skip(link.column - 1).collect::<String>();
                (link.target, at, content[link.span].to_string())
            })
            .collect::<Vec<_>>();

        // An inline link sits at its own `[`; a reference link at its
        // definition, the text naming its destination.
        assert_eq!(positions.len(), 3);
        for (target, at, spanned) in &positions {
            assert!(
                at.starts_with('['),
                "{target} is positioned at a link: {at}"
            );
            assert!(
                at.starts_with(spanned.as_str()),
                "{target}: {at} / {spanned}"
            );
            assert!(
                spanned.contains(target.as_str()),
                "{target} lies in {spanned}"
            );
        }
        assert_eq!(
            positions
                .iter()
                .map(|(target, ..)| target.as_str())
                .collect::<Vec<_>>(),
            ["inline.md", "angle.md", "ref.md"]
        );
        assert!(positions[2].1.starts_with("[r]:"));
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
