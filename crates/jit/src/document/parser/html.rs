//! HTML implementation of [`ContentParser`], behind the optional `html` cargo
//! feature.
//!
//! Parses an HTML body into the SAME canonical [`ParsedContent`] the Markdown
//! parser produces, so an identical document expressed in HTML yields identical
//! sections, headings, and items. This is what proves the projection layer is
//! genuinely format-agnostic.
//!
//! # Mapping
//!
//! - `<h1>`..`<h6>` open a section at level 1..6. The heading text is the tag's
//!   flattened, whitespace-normalized inner text.
//! - `<li>` elements that appear under a heading become that section's [`items`],
//!   one item per `<li>`, using the flattened, whitespace-normalized inner text.
//!   Nested-list text is folded into its parent item (mirroring the Markdown
//!   parser's "only top-level list items" semantics), because `inner_text`
//!   recursively concatenates descendant text.
//! - Anything before the first heading has no section to attach to and is
//!   ignored, exactly like the Markdown parser.
//!
//! Headings whose slugs collide are MERGED (items concatenated in document
//! order) via the shared [`merge_section`] helper, matching Markdown.
//!
//! Format detection delegates to [`HtmlAdapter`] so the heuristic lives in one
//! place (DR §6.2). Malformed HTML never errors: `tl` is lenient, and a parse
//! failure degrades to an empty [`ParsedContent`] rather than panicking.
//!
//! [`items`]: Section::items

use super::{merge_section, normalize_ws, ContentParser, ParsedContent, Section};
use crate::document::{DocFormatAdapter, HtmlAdapter};
use tl::{Node, Parser as TlParser};

/// HTML content parser.
///
/// Pure: [`ContentParser::parse`] is a deterministic function of the input.
/// Available only when the `html` feature is enabled.
///
/// # Examples
///
/// ```
/// # #[cfg(feature = "html")] {
/// use jit::document::{ContentParser, HtmlContentParser};
///
/// let parsed = HtmlContentParser.parse("<h2>Notes</h2><ul><li>first</li><li>second</li></ul>");
/// let notes = parsed.sections.get("notes").unwrap();
/// assert_eq!(notes.heading, "Notes");
/// assert_eq!(notes.level, 2);
/// assert_eq!(notes.items, vec!["first".to_string(), "second".to_string()]);
/// # }
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct HtmlContentParser;

/// Map an HTML heading tag name (`h1`..`h6`) to its numeric level.
///
/// Returns `None` for any other tag name.
fn heading_level(tag_name: &str) -> Option<u8> {
    match tag_name {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

/// Depth-first, document-order walk of one node, updating the section in
/// progress.
///
/// `current` is the section being filled (if a heading has been seen). When a
/// heading tag is encountered the previous section is flushed via
/// [`merge_section`] and a fresh one begins. `<li>` text is appended to the
/// current section's items; `<li>` children are NOT descended into, so a nested
/// list is folded into its parent item by `inner_text` rather than producing
/// extra items.
fn walk(node: &Node, parser: &TlParser, result: &mut ParsedContent, current: &mut Option<Section>) {
    let Some(tag) = node.as_tag() else {
        return;
    };
    let name = tag.name().as_utf8_str();

    if let Some(level) = heading_level(&name) {
        if let Some(section) = current.take() {
            merge_section(result, section);
        }
        *current = Some(Section {
            heading: normalize_ws(&tag.inner_text(parser)),
            level,
            items: Vec::new(),
        });
        return;
    }

    if name == "li" {
        if let Some(section) = current.as_mut() {
            section.items.push(normalize_ws(&tag.inner_text(parser)));
        }
        // Do not descend: nested-list text is already part of inner_text above,
        // mirroring the Markdown parser's top-level-only item capture.
        return;
    }

    for child in tag.children().top().iter() {
        if let Some(child_node) = child.get(parser) {
            walk(child_node, parser, result, current);
        }
    }
}

impl ContentParser for HtmlContentParser {
    fn id(&self) -> &str {
        "html"
    }

    fn detect(&self, content: &str) -> bool {
        // Reuse the adapter heuristic rather than duplicating it (DR §6.2).
        HtmlAdapter.detect(content)
    }

    fn parse(&self, content: &str) -> ParsedContent {
        let mut result = ParsedContent::default();
        // `tl` is lenient and only errors on pathological input; degrade to an
        // empty result rather than panicking, matching the Markdown parser which
        // never errors.
        let Ok(dom) = tl::parse(content, tl::ParserOptions::default()) else {
            return result;
        };
        let parser = dom.parser();
        let mut current: Option<Section> = None;

        for handle in dom.children() {
            if let Some(node) = handle.get(parser) {
                walk(node, parser, &mut result, &mut current);
            }
        }

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
    fn test_id_is_html() {
        assert_eq!(HtmlContentParser.id(), "html");
    }

    #[test]
    fn test_detect_delegates_to_adapter() {
        let p = HtmlContentParser;
        assert!(p.detect("<!DOCTYPE html><html><body></body></html>"));
        assert!(!p.detect("plain text no markup"));
    }

    #[test]
    fn test_parse_extracts_section_and_items() {
        let html = "<h2>Success Criteria</h2><ul>\
            <li>[hard] REQ-01: first</li>\
            <li>[aspirational] REQ-02: second</li></ul>";
        let parsed = HtmlContentParser.parse(html);
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
        let html = "<h1>Plan</h1><p>some prose</p><h2>Notes</h2><ul><li>one</li><li>two</li></ul>";
        let parsed = HtmlContentParser.parse(html);
        assert_eq!(parsed.sections.len(), 2);
        assert!(parsed.sections.contains_key("plan"));
        let notes = parsed.sections.get("notes").unwrap();
        assert_eq!(notes.items, vec!["one".to_string(), "two".to_string()]);
    }

    #[test]
    fn test_parse_only_top_level_items() {
        // A nested list inside an <li> is folded into the parent item's text.
        let html = "<h2>S</h2><ul>\
            <li>parent<ul><li>child-nested</li></ul></li>\
            <li>sibling</li></ul>";
        let parsed = HtmlContentParser.parse(html);
        let section = parsed.sections.get("s").unwrap();
        assert_eq!(section.items.len(), 2);
        assert!(section.items[0].starts_with("parent"));
        assert_eq!(section.items[1], "sibling");
    }

    #[test]
    fn test_parse_empty_is_empty() {
        let parsed = HtmlContentParser.parse("");
        assert!(parsed.sections.is_empty());
    }

    #[test]
    fn test_parse_is_deterministic() {
        let html = "<h2>A</h2><ul><li>x</li><li>y</li></ul><h2>B</h2><ul><li>z</li></ul>";
        let first = HtmlContentParser.parse(html);
        let second = HtmlContentParser.parse(html);
        assert_eq!(first, second);
    }

    #[test]
    fn test_parse_multiline_item_keeps_word_boundary() {
        // Whitespace (including newlines) inside an item collapses to one space.
        let html = "<h2>S</h2><ul><li>foo\n  bar</li></ul>";
        let parsed = HtmlContentParser.parse(html);
        let section = parsed.sections.get("s").unwrap();
        assert_eq!(section.items, vec!["foo bar".to_string()]);
    }

    #[test]
    fn test_parse_colliding_slugs_merge_items_in_order() {
        let html = "<h2>Success Criteria</h2><ul><li>one</li><li>two</li></ul>\
            <h2>Success-Criteria</h2><ul><li>three</li><li>four</li></ul>";
        let parsed = HtmlContentParser.parse(html);
        assert_eq!(parsed.sections.len(), 1);
        let section = parsed
            .sections
            .get("success_criteria")
            .expect("merged section present");
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
        let html = "<ul><li>orphan item</li></ul><h2>Real</h2><ul><li>kept</li></ul>";
        let parsed = HtmlContentParser.parse(html);
        assert_eq!(parsed.sections.len(), 1);
        assert_eq!(parsed.sections.get("real").unwrap().items, vec!["kept"]);
    }
}
