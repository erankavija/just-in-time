//! Markdown implementation of [`ContentParser`].
//!
//! Parses a Markdown body into the canonical [`ParsedContent`]: a map of sections
//! keyed by normalized heading slug, each carrying the heading text, its level,
//! and the raw text of the top-level list items beneath it.
//!
//! Format detection delegates to [`MarkdownAdapter`] so the heuristic lives in a
//! single place (DR §6.2). Built on `pulldown-cmark` 0.13.

use super::{slugify_heading, ContentParser, ParsedContent, Section};
use crate::document::{DocFormatAdapter, MarkdownAdapter};
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

/// Markdown content parser.
///
/// Pure: [`ContentParser::parse`] is a deterministic function of the input.
///
/// # Examples
///
/// ```
/// use jit::document::{ContentParser, MarkdownContentParser};
///
/// let parsed = MarkdownContentParser.parse("## Notes\n\n- first\n- second\n");
/// let notes = parsed.sections.get("notes").unwrap();
/// assert_eq!(notes.heading, "Notes");
/// assert_eq!(notes.items, vec!["first".to_string(), "second".to_string()]);
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct MarkdownContentParser;

/// Insert a parsed section, MERGING it with any existing section that normalizes
/// to the same slug.
///
/// Two headings that slugify identically (e.g. two `## Success Criteria`, or
/// `Success Criteria` vs `Success-Criteria`) address the same canonical section,
/// so their list items are concatenated in document order rather than the later
/// section silently overwriting (or being dropped by) the earlier one. The first
/// section's `heading` text and `level` are kept; only `items` are appended.
fn merge_section(result: &mut ParsedContent, mut section: Section) {
    let slug = slugify_heading(&section.heading);
    match result.sections.get_mut(&slug) {
        Some(existing) => existing.items.append(&mut section.items),
        None => {
            result.sections.insert(slug, section);
        }
    }
}

fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

impl ContentParser for MarkdownContentParser {
    fn id(&self) -> &str {
        "markdown"
    }

    fn detect(&self, content: &str) -> bool {
        // Reuse the adapter heuristic rather than duplicating it (DR §6.2).
        MarkdownAdapter.detect(content)
    }

    fn parse(&self, content: &str) -> ParsedContent {
        let mut result = ParsedContent::default();

        // The section currently being filled, if any.
        let mut current: Option<Section> = None;
        // Accumulated text for the heading or list item being read.
        let mut text_buf = String::new();
        // Nesting depth of lists; we only capture top-level (depth 1) items.
        let mut list_depth: usize = 0;
        // Whether we're inside the heading currently being read.
        let mut in_heading = false;
        // Whether we're capturing text for a top-level list item.
        let mut capturing_item = false;

        for event in Parser::new(content) {
            match event {
                Event::Start(Tag::Heading { level, .. }) => {
                    // Flush any section in progress before starting a new one.
                    if let Some(section) = current.take() {
                        merge_section(&mut result, section);
                    }
                    in_heading = true;
                    text_buf.clear();
                    current = Some(Section {
                        heading: String::new(),
                        level: heading_level_to_u8(level),
                        items: Vec::new(),
                    });
                }
                Event::End(TagEnd::Heading(_)) => {
                    if let Some(section) = current.as_mut() {
                        section.heading = text_buf.trim().to_string();
                    }
                    in_heading = false;
                    text_buf.clear();
                }
                Event::Start(Tag::List(_)) => {
                    list_depth += 1;
                }
                Event::End(TagEnd::List(_)) => {
                    list_depth = list_depth.saturating_sub(1);
                }
                // Only capture items directly under a section heading at the top
                // list level; nested-list items are part of their parent's text.
                Event::Start(Tag::Item) if list_depth == 1 => {
                    capturing_item = true;
                    text_buf.clear();
                }
                Event::End(TagEnd::Item) if capturing_item && list_depth == 1 => {
                    let item = text_buf.trim().to_string();
                    if let Some(section) = current.as_mut() {
                        section.items.push(item);
                    }
                    capturing_item = false;
                    text_buf.clear();
                }
                Event::Text(t) | Event::Code(t)
                    if in_heading || (capturing_item && list_depth == 1) =>
                {
                    text_buf.push_str(&t);
                }
                // A wrapped line in a heading or item (e.g. `- foo\n  bar`)
                // arrives as a soft/hard break. Map it to a single space so word
                // boundaries survive (`foo bar`, not `foobar`). Trimming on flush
                // removes any leading/trailing space this introduces.
                Event::SoftBreak | Event::HardBreak
                    if in_heading || (capturing_item && list_depth == 1) =>
                {
                    text_buf.push(' ');
                }
                _ => {}
            }
        }

        // Flush the trailing section.
        if let Some(section) = current.take() {
            merge_section(&mut result, section);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_is_markdown() {
        assert_eq!(MarkdownContentParser.id(), "markdown");
    }

    #[test]
    fn test_detect_delegates_to_adapter() {
        let p = MarkdownContentParser;
        assert!(p.detect("# Heading\n\ntext"));
        assert!(!p.detect("plain text no markup"));
    }

    #[test]
    fn test_parse_extracts_section_and_items() {
        let md = "## Success Criteria\n\n\
            - [hard] REQ-01: first\n\
            - [aspirational] REQ-02: second\n";
        let parsed = MarkdownContentParser.parse(md);
        let section = parsed
            .sections
            .get("success_criteria")
            .expect("section present");
        assert_eq!(section.heading, "Success Criteria");
        assert_eq!(section.level, 2);
        assert_eq!(
            section.items,
            vec![
                "[hard] REQ-01: first".to_string(),
                "[aspirational] REQ-02: second".to_string(),
            ]
        );
    }

    #[test]
    fn test_parse_multiple_sections() {
        let md = "# Plan\n\nsome prose\n\n## Notes\n\n- one\n- two\n";
        let parsed = MarkdownContentParser.parse(md);
        assert_eq!(parsed.sections.len(), 2);
        assert!(parsed.sections.contains_key("plan"));
        let notes = parsed.sections.get("notes").unwrap();
        assert_eq!(notes.items, vec!["one".to_string(), "two".to_string()]);
    }

    #[test]
    fn test_parse_only_top_level_items() {
        let md = "## S\n\n\
            - parent\n  \
            - child-nested\n\
            - sibling\n";
        let parsed = MarkdownContentParser.parse(md);
        let section = parsed.sections.get("s").unwrap();
        // Nested items are flattened into their parent's text run, not separate.
        assert_eq!(section.items.len(), 2);
        assert!(section.items[0].starts_with("parent"));
        assert_eq!(section.items[1], "sibling");
    }

    #[test]
    fn test_parse_inline_code_in_item_preserved() {
        let md = "## S\n\n- use `jit validate` here\n";
        let parsed = MarkdownContentParser.parse(md);
        let section = parsed.sections.get("s").unwrap();
        assert_eq!(section.items, vec!["use jit validate here".to_string()]);
    }

    #[test]
    fn test_parse_empty_is_empty() {
        let parsed = MarkdownContentParser.parse("");
        assert!(parsed.sections.is_empty());
    }

    #[test]
    fn test_parse_is_deterministic() {
        let md = "## A\n- x\n- y\n## B\n- z\n";
        let first = MarkdownContentParser.parse(md);
        let second = MarkdownContentParser.parse(md);
        assert_eq!(first, second);
    }

    #[test]
    fn test_parse_multiline_list_item_keeps_word_boundary() {
        // A wrapped list item must not collapse `foo` and `bar` into `foobar`.
        let md = "## S\n\n- foo\n  bar\n";
        let parsed = MarkdownContentParser.parse(md);
        let section = parsed.sections.get("s").unwrap();
        assert_eq!(section.items, vec!["foo bar".to_string()]);
    }

    #[test]
    fn test_parse_multiline_heading_keeps_word_boundary() {
        // A wrapped Setext heading emits a SoftBreak between its lines; that
        // break must become a space so the heading reads `Success Criteria`,
        // not `SuccessCriteria` (which would also slugify differently).
        let md = "Success\nCriteria\n===============\n\n- x\n";
        let parsed = MarkdownContentParser.parse(md);
        let section = parsed
            .sections
            .get("success_criteria")
            .expect("section present");
        assert_eq!(section.heading, "Success Criteria");
        assert_eq!(section.level, 1);
    }

    #[test]
    fn test_parse_colliding_slugs_merge_items_in_order() {
        // Two headings normalizing to the same slug must merge their items in
        // document order rather than the later section being dropped.
        let md = "## Success Criteria\n\n- one\n- two\n\n\
            ## Success-Criteria\n\n- three\n- four\n";
        let parsed = MarkdownContentParser.parse(md);
        assert_eq!(parsed.sections.len(), 1);
        let section = parsed
            .sections
            .get("success_criteria")
            .expect("merged section present");
        // First section's heading text and level are kept.
        assert_eq!(section.heading, "Success Criteria");
        assert_eq!(section.level, 2);
        assert_eq!(
            section.items,
            vec![
                "one".to_string(),
                "two".to_string(),
                "three".to_string(),
                "four".to_string(),
            ]
        );
    }

    #[test]
    fn test_parse_items_without_heading_ignored() {
        // Items before any heading have no section to attach to.
        let md = "- orphan item\n\n## Real\n- kept\n";
        let parsed = MarkdownContentParser.parse(md);
        assert_eq!(parsed.sections.len(), 1);
        assert_eq!(parsed.sections.get("real").unwrap().items, vec!["kept"]);
    }
}
