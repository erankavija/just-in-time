//! Content parsers for document bodies.
//!
//! This module defines the [`ContentParser`] trait and the canonical structure
//! it produces. Unlike [`DocFormatAdapter`](crate::document::DocFormatAdapter) —
//! which is an asset/link tool — a `ContentParser` parses a document body into a
//! single canonical structure of sections, headings, and list items that the
//! validation engine projects over.
//!
//! Each parser yields ONE canonical [`ParsedContent`] regardless of source
//! format, so the projection layer and validation rules are genuinely
//! format-agnostic. The **Markdown** parser ([`markdown::MarkdownContentParser`])
//! is in the default build; HTML/XML parsers live behind optional cargo features
//! (added by a later task).
//!
//! Format detection reuses [`DocFormatAdapter::detect`](crate::document::DocFormatAdapter::detect)
//! rather than duplicating heuristics: a `ContentParser` is expected to wrap (or
//! delegate to) the matching adapter for content-based detection.

pub mod markdown;

#[cfg(feature = "html")]
pub mod html;
#[cfg(feature = "xml")]
pub mod xml;

use serde::Serialize;
use std::collections::BTreeMap;

pub use markdown::MarkdownContentParser;

#[cfg(feature = "html")]
pub use html::HtmlContentParser;
#[cfg(feature = "xml")]
pub use xml::XmlContentParser;

/// A canonical parsed document body.
///
/// Sections are keyed by a normalized heading slug (lowercased, spaces ->
/// underscores) so that callers can address them stably regardless of the source
/// heading text casing. A [`BTreeMap`] keeps the serialized order deterministic.
///
/// # Examples
///
/// ```
/// use jit::document::{ContentParser, MarkdownContentParser, ParsedContent};
///
/// let parsed: ParsedContent = MarkdownContentParser.parse("## Plan\n\n- step\n");
/// assert!(parsed.sections.contains_key("plan"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ParsedContent {
    /// Sections keyed by normalized heading slug (e.g. `"success_criteria"`).
    pub sections: BTreeMap<String, Section>,
}

/// One section of a document, identified by a heading.
///
/// # Examples
///
/// ```
/// use jit::document::{ContentParser, MarkdownContentParser, Section};
///
/// let parsed = MarkdownContentParser.parse("## Notes\n\n- a\n- b\n");
/// let section: &Section = parsed.sections.get("notes").unwrap();
/// assert_eq!(section.heading, "Notes");
/// assert_eq!(section.level, 2);
/// assert_eq!(section.items, vec!["a".to_string(), "b".to_string()]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct Section {
    /// The original heading text as written (e.g. `"Success Criteria"`).
    pub heading: String,
    /// The heading level (1 for `#`, 2 for `##`, ...).
    pub level: u8,
    /// The raw text runs of list entries directly under this heading, in order.
    /// Each item is the flattened text of one top-level list entry.
    pub items: Vec<String>,
}

/// Normalize a heading into a stable slug used as the section key.
///
/// Lowercases, trims, and replaces runs of non-alphanumeric characters with a
/// single underscore. `"## Success Criteria"` text `"Success Criteria"` becomes
/// `"success_criteria"`.
///
/// # Examples
///
/// ```
/// use jit::document::slugify_heading;
///
/// assert_eq!(slugify_heading("Success Criteria"), "success_criteria");
/// assert_eq!(slugify_heading("  Plan (v2)!  "), "plan_v2");
/// ```
pub fn slugify_heading(heading: &str) -> String {
    let mut slug = String::with_capacity(heading.len());
    let mut prev_underscore = false;
    for ch in heading.trim().chars() {
        if ch.is_alphanumeric() {
            slug.extend(ch.to_lowercase());
            prev_underscore = false;
        } else if !prev_underscore && !slug.is_empty() {
            slug.push('_');
            prev_underscore = true;
        }
    }
    // Trim a possible trailing underscore.
    while slug.ends_with('_') {
        slug.pop();
    }
    slug
}

/// Collapse all runs of whitespace into single spaces and trim the ends.
///
/// This is the canonicalization every parser applies to heading and item text
/// so that the same logical content yields byte-identical strings regardless of
/// how the source format wrapped or indented it. It mirrors the Markdown
/// parser's break handling, where a `SoftBreak`/`HardBreak` between two words
/// becomes a single space (`foo bar`, never `foobar` or `foo  bar`).
///
/// # Examples
///
/// ```
/// # use jit::document::ContentParser;
/// # use jit::document::MarkdownContentParser;
/// // `normalize_ws` is exercised indirectly through every parser: a wrapped
/// // list item collapses to a single-spaced string.
/// let parsed = MarkdownContentParser.parse("## S\n\n- foo\n  bar\n");
/// assert_eq!(parsed.sections["s"].items, vec!["foo bar".to_string()]);
/// ```
#[cfg(any(feature = "html", feature = "xml"))]
pub(crate) fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Insert a parsed section, MERGING it with any existing section that normalizes
/// to the same slug.
///
/// Two headings that slugify identically (e.g. two `Success Criteria`, or
/// `Success Criteria` vs `Success-Criteria`) address the same canonical section,
/// so their list items are concatenated in document order rather than the later
/// section silently overwriting (or being dropped by) the earlier one. The first
/// section's `heading` text and `level` are kept; only `items` are appended.
///
/// Shared by every [`ContentParser`] implementation so cross-format equality
/// holds for colliding-slug documents.
pub(crate) fn merge_section(result: &mut ParsedContent, mut section: Section) {
    let slug = slugify_heading(&section.heading);
    match result.sections.get_mut(&slug) {
        Some(existing) => existing.items.append(&mut section.items),
        None => {
            result.sections.insert(slug, section);
        }
    }
}

/// A parser that turns a document body into a canonical [`ParsedContent`].
///
/// Implementations are pure: `parse` is a deterministic function of `content`
/// alone, with no I/O. `detect` mirrors
/// [`DocFormatAdapter::detect`](crate::document::DocFormatAdapter::detect) and is
/// expected to delegate to the matching adapter.
///
/// # Examples
///
/// ```
/// use jit::document::{ContentParser, MarkdownContentParser};
///
/// fn first_heading(parser: &dyn ContentParser, body: &str) -> Option<String> {
///     parser
///         .parse(body)
///         .sections
///         .into_values()
///         .next()
///         .map(|s| s.heading)
/// }
///
/// assert_eq!(
///     first_heading(&MarkdownContentParser, "## Plan\n\n- step\n"),
///     Some("Plan".to_string())
/// );
/// ```
pub trait ContentParser {
    /// The parser identifier (e.g. `"markdown"`); matches the adapter id.
    fn id(&self) -> &str;

    /// Content-based format detection, reusing the format adapter heuristics.
    fn detect(&self, content: &str) -> bool;

    /// Parse a document body into the canonical structure.
    fn parse(&self, content: &str) -> ParsedContent;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_heading_basic() {
        assert_eq!(slugify_heading("Success Criteria"), "success_criteria");
    }

    #[test]
    fn test_slugify_heading_collapses_punctuation() {
        assert_eq!(
            slugify_heading("  Success / Criteria!  "),
            "success_criteria"
        );
        assert_eq!(slugify_heading("Plan (v2)"), "plan_v2");
    }

    #[test]
    fn test_slugify_heading_empty() {
        assert_eq!(slugify_heading(""), "");
        assert_eq!(slugify_heading("   "), "");
        assert_eq!(slugify_heading("---"), "");
    }
}
