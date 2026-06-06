//! XML implementation of [`ContentParser`], behind the optional `xml` cargo
//! feature.
//!
//! Parses an XML body into the SAME canonical [`ParsedContent`] the Markdown and
//! HTML parsers produce, so an identical document expressed in XML yields
//! identical sections, headings, and items.
//!
//! # Element convention
//!
//! Because XML has no intrinsic "heading" or "list item" elements, this parser
//! defines a small, explicit convention:
//!
//! ```xml
//! <section>
//!   <heading level="2">Success Criteria</heading>
//!   <item>[hard] REQ-01: first</item>
//!   <item>[aspirational] REQ-02: second</item>
//! </section>
//! ```
//!
//! - `<heading level="N">` opens a section at level `N` (1..6). A missing or
//!   unparseable `level` attribute defaults to level 1.
//! - `<item>` elements become the current section's [`items`], one item per
//!   element, using the flattened, whitespace-normalized text content. Nested
//!   `<item>` text is folded into the enclosing item (mirroring the
//!   top-level-only semantics of the Markdown and HTML parsers).
//! - The wrapping `<section>` element is optional structurally — a `<heading>`
//!   begins a section regardless of nesting — but it documents intent and is
//!   recommended. A single XML document needs one root element, so wrap the
//!   whole body in a `<document>` (or `<sections>`) root when it has more than
//!   one top-level section.
//! - Content before the first `<heading>` has no section to attach to and is
//!   ignored, exactly like the Markdown and HTML parsers.
//!
//! Headings whose slugs collide are MERGED (items concatenated in document
//! order) via the shared [`merge_section`] helper, matching the other parsers.
//!
//! Detection: XML is recognized by an `<?xml` prolog or a leading `<section`/
//! `<document` element. Malformed XML never errors: parsing stops at the first
//! error and returns whatever was accumulated so far, matching the Markdown
//! parser which never panics.
//!
//! [`items`]: Section::items

use super::{merge_section, normalize_ws, ContentParser, ParsedContent, Section};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use quick_xml::XmlVersion;

/// XML content parser.
///
/// Pure: [`ContentParser::parse`] is a deterministic function of the input.
/// Available only when the `xml` feature is enabled. See the [module
/// docs](self) for the element convention it expects.
///
/// # Examples
///
/// ```
/// # #[cfg(feature = "xml")] {
/// use jit::document::{ContentParser, XmlContentParser};
///
/// let xml = "<section><heading level=\"2\">Notes</heading>\
///     <item>first</item><item>second</item></section>";
/// let parsed = XmlContentParser.parse(xml);
/// let notes = parsed.sections.get("notes").unwrap();
/// assert_eq!(notes.heading, "Notes");
/// assert_eq!(notes.level, 2);
/// assert_eq!(notes.items, vec!["first".to_string(), "second".to_string()]);
/// # }
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct XmlContentParser;

/// Read the `level` attribute off a `<heading>` start tag, defaulting to 1.
///
/// A missing, non-numeric, or out-of-range value falls back to level 1 rather
/// than erroring, consistent with the never-panic parse contract.
fn heading_level(start: &quick_xml::events::BytesStart) -> u8 {
    start
        .try_get_attribute(b"level".as_slice())
        .ok()
        .flatten()
        .and_then(|attr| {
            attr.normalized_value(XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
        .and_then(|v| v.trim().parse::<u8>().ok())
        .filter(|n| (1..=6).contains(n))
        .unwrap_or(1)
}

impl ContentParser for XmlContentParser {
    fn id(&self) -> &str {
        "xml"
    }

    fn detect(&self, content: &str) -> bool {
        let trimmed = content.trim_start();
        trimmed.starts_with("<?xml")
            || trimmed.starts_with("<section")
            || trimmed.starts_with("<document")
            || trimmed.starts_with("<sections")
    }

    fn parse(&self, content: &str) -> ParsedContent {
        let mut result = ParsedContent::default();
        let mut reader = Reader::from_str(content);

        // The section currently being filled, if any.
        let mut current: Option<Section> = None;
        // Accumulated text for the heading or item currently open.
        let mut text_buf = String::new();
        // Whether we are inside a <heading> element.
        let mut in_heading = false;
        // Nesting depth of <item> elements; we capture text at any depth into the
        // top-level item so nested <item>s fold into their parent's text.
        let mut item_depth: usize = 0;

        loop {
            match reader.read_event() {
                Ok(Event::Start(start)) => {
                    let name = start.name();
                    match name.as_ref() {
                        b"heading" => {
                            if let Some(section) = current.take() {
                                merge_section(&mut result, section);
                            }
                            current = Some(Section {
                                heading: String::new(),
                                level: heading_level(&start),
                                items: Vec::new(),
                            });
                            in_heading = true;
                            text_buf.clear();
                        }
                        b"item" => {
                            if item_depth == 0 {
                                text_buf.clear();
                            }
                            item_depth += 1;
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(end)) => match end.name().as_ref() {
                    b"heading" => {
                        if let Some(section) = current.as_mut() {
                            section.heading = normalize_ws(&text_buf);
                        }
                        in_heading = false;
                        text_buf.clear();
                    }
                    b"item" => {
                        item_depth = item_depth.saturating_sub(1);
                        if item_depth == 0 {
                            let item = normalize_ws(&text_buf);
                            if let Some(section) = current.as_mut() {
                                section.items.push(item);
                            }
                            text_buf.clear();
                        }
                    }
                    _ => {}
                },
                Ok(Event::Text(t)) if in_heading || item_depth > 0 => {
                    if let Ok(decoded) = t.decode() {
                        text_buf.push_str(&decoded);
                    }
                }
                Ok(Event::CData(c)) if in_heading || item_depth > 0 => {
                    if let Ok(decoded) = c.decode() {
                        text_buf.push_str(&decoded);
                    }
                }
                Ok(Event::Eof) => break,
                // Stop at the first malformed event, keeping what we accumulated.
                Err(_) => break,
                _ => {}
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
    fn test_id_is_xml() {
        assert_eq!(XmlContentParser.id(), "xml");
    }

    #[test]
    fn test_detect_recognizes_xml() {
        let p = XmlContentParser;
        assert!(p.detect("<?xml version=\"1.0\"?><document/>"));
        assert!(p.detect("<section><heading>x</heading></section>"));
        assert!(!p.detect("plain text no markup"));
    }

    #[test]
    fn test_parse_extracts_section_and_items() {
        let xml = "<section><heading level=\"2\">Success Criteria</heading>\
            <item>[hard] REQ-01: first</item>\
            <item>[aspirational] REQ-02: second</item></section>";
        let parsed = XmlContentParser.parse(xml);
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
        let xml = "<document>\
            <section><heading level=\"1\">Plan</heading></section>\
            <section><heading level=\"2\">Notes</heading>\
            <item>one</item><item>two</item></section></document>";
        let parsed = XmlContentParser.parse(xml);
        assert_eq!(parsed.sections.len(), 2);
        assert!(parsed.sections.contains_key("plan"));
        let notes = parsed.sections.get("notes").unwrap();
        assert_eq!(notes.items, vec!["one".to_string(), "two".to_string()]);
    }

    #[test]
    fn test_parse_nested_item_folds_into_parent() {
        let xml = "<section><heading level=\"2\">S</heading>\
            <item>parent<item>child-nested</item></item>\
            <item>sibling</item></section>";
        let parsed = XmlContentParser.parse(xml);
        let section = parsed.sections.get("s").unwrap();
        assert_eq!(section.items.len(), 2);
        assert!(section.items[0].starts_with("parent"));
        assert_eq!(section.items[1], "sibling");
    }

    #[test]
    fn test_parse_missing_level_defaults_to_one() {
        let xml = "<section><heading>Top</heading><item>x</item></section>";
        let parsed = XmlContentParser.parse(xml);
        let section = parsed.sections.get("top").unwrap();
        assert_eq!(section.level, 1);
    }

    #[test]
    fn test_parse_empty_is_empty() {
        let parsed = XmlContentParser.parse("");
        assert!(parsed.sections.is_empty());
    }

    #[test]
    fn test_parse_is_deterministic() {
        let xml = "<document>\
            <section><heading level=\"2\">A</heading><item>x</item><item>y</item></section>\
            <section><heading level=\"2\">B</heading><item>z</item></section></document>";
        let first = XmlContentParser.parse(xml);
        let second = XmlContentParser.parse(xml);
        assert_eq!(first, second);
    }

    #[test]
    fn test_parse_multiline_item_keeps_word_boundary() {
        let xml = "<section><heading level=\"2\">S</heading><item>foo\n  bar</item></section>";
        let parsed = XmlContentParser.parse(xml);
        let section = parsed.sections.get("s").unwrap();
        assert_eq!(section.items, vec!["foo bar".to_string()]);
    }

    #[test]
    fn test_parse_colliding_slugs_merge_items_in_order() {
        let xml = "<document>\
            <section><heading level=\"2\">Success Criteria</heading>\
            <item>one</item><item>two</item></section>\
            <section><heading level=\"2\">Success-Criteria</heading>\
            <item>three</item><item>four</item></section></document>";
        let parsed = XmlContentParser.parse(xml);
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
        let xml = "<document><item>orphan item</item>\
            <section><heading level=\"2\">Real</heading><item>kept</item></section></document>";
        let parsed = XmlContentParser.parse(xml);
        assert_eq!(parsed.sections.len(), 1);
        assert_eq!(parsed.sections.get("real").unwrap().items, vec!["kept"]);
    }

    #[test]
    fn test_parse_malformed_degrades_gracefully() {
        // Unclosed tag: parser stops at the error and returns what it has.
        let xml = "<section><heading level=\"2\">Kept</heading><item>one</item>";
        let parsed = XmlContentParser.parse(xml);
        // No panic; the completed item before EOF/error is retained.
        let section = parsed.sections.get("kept").unwrap();
        assert_eq!(section.items, vec!["one".to_string()]);
    }
}
