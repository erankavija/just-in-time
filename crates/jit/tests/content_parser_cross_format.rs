//! Cross-format equality test for the content parsers.
//!
//! Proves the projection is genuinely format-agnostic: semantically equivalent
//! Markdown, HTML, and XML documents must parse to byte-identical
//! [`ParsedContent`], so any content rule (a section/item check) yields IDENTICAL
//! findings across all three formats.
//!
//! Compiled only when BOTH the `html` and `xml` features are enabled:
//!   cargo test -p jit --features html,xml

#![cfg(all(feature = "html", feature = "xml"))]

use jit::document::{
    ContentParser, HtmlContentParser, MarkdownContentParser, ParsedContent, XmlContentParser,
};

/// The same document expressed in Markdown.
const MARKDOWN: &str = "\
# Plan

some prose

## Success Criteria

- [hard] REQ-01: parser is format-agnostic
- [aspirational] REQ-02: nice docs

## Notes

- first note
- second note
";

/// The same document expressed in HTML.
const HTML: &str = "\
<h1>Plan</h1>
<p>some prose</p>
<h2>Success Criteria</h2>
<ul>
  <li>[hard] REQ-01: parser is format-agnostic</li>
  <li>[aspirational] REQ-02: nice docs</li>
</ul>
<h2>Notes</h2>
<ul>
  <li>first note</li>
  <li>second note</li>
</ul>
";

/// The same document expressed in XML, using the documented element convention.
const XML: &str = "\
<document>
  <section>
    <heading level=\"1\">Plan</heading>
  </section>
  <section>
    <heading level=\"2\">Success Criteria</heading>
    <item>[hard] REQ-01: parser is format-agnostic</item>
    <item>[aspirational] REQ-02: nice docs</item>
  </section>
  <section>
    <heading level=\"2\">Notes</heading>
    <item>first note</item>
    <item>second note</item>
  </section>
</document>
";

#[test]
fn test_parsers_produce_identical_parsed_content_across_formats() {
    let md: ParsedContent = MarkdownContentParser.parse(MARKDOWN);
    let html: ParsedContent = HtmlContentParser.parse(HTML);
    let xml: ParsedContent = XmlContentParser.parse(XML);

    // All three canonical structures are equal.
    assert_eq!(md, html, "Markdown and HTML projections differ");
    assert_eq!(md, xml, "Markdown and XML projections differ");
    assert_eq!(html, xml, "HTML and XML projections differ");

    // Spot-check the shape so a future regression that makes all three equally
    // wrong is still caught.
    let criteria = md
        .sections
        .get("success_criteria")
        .expect("success_criteria section present");
    assert_eq!(criteria.heading, "Success Criteria");
    assert_eq!(criteria.level, 2);
    assert_eq!(
        criteria.items,
        vec![
            "[hard] REQ-01: parser is format-agnostic".to_string(),
            "[aspirational] REQ-02: nice docs".to_string(),
        ]
    );
    assert_eq!(md.sections.len(), 3);
}

/// A content rule (here: "the success_criteria section has at least one `[hard]`
/// item") produces the SAME finding regardless of source format.
#[test]
fn test_content_rule_findings_identical_across_formats() {
    fn has_hard_criterion(parser: &dyn ContentParser, body: &str) -> bool {
        parser
            .parse(body)
            .sections
            .get("success_criteria")
            .map(|s| s.items.iter().any(|i| i.starts_with("[hard]")))
            .unwrap_or(false)
    }

    assert!(has_hard_criterion(&MarkdownContentParser, MARKDOWN));
    assert!(has_hard_criterion(&HtmlContentParser, HTML));
    assert!(has_hard_criterion(&XmlContentParser, XML));
}
