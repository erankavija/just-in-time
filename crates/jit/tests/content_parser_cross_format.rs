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
use jit::domain::{project, Issue};
use serde_json::Value;

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

/// The same document in Markdown, but with NO `[hard]` success criterion. Used to
/// prove the content rule produces an identical NON-empty finding set across
/// formats when the hard criterion is absent.
const MARKDOWN_NO_HARD: &str = "\
# Plan

some prose

## Success Criteria

- [aspirational] REQ-02: nice docs

## Notes

- first note
- second note
";

/// The HTML counterpart of [`MARKDOWN_NO_HARD`].
const HTML_NO_HARD: &str = "\
<h1>Plan</h1>
<p>some prose</p>
<h2>Success Criteria</h2>
<ul>
  <li>[aspirational] REQ-02: nice docs</li>
</ul>
<h2>Notes</h2>
<ul>
  <li>first note</li>
  <li>second note</li>
</ul>
";

/// The XML counterpart of [`MARKDOWN_NO_HARD`].
const XML_NO_HARD: &str = "\
<document>
  <section>
    <heading level=\"1\">Plan</heading>
  </section>
  <section>
    <heading level=\"2\">Success Criteria</heading>
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

/// A single content "finding": the path that failed and a human message. Mirrors
/// the shape a JSON-Schema validation engine would emit per failed assertion, so
/// the cross-format comparison is over real findings rather than a bare boolean.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Finding {
    path: String,
    message: String,
}

/// Build a real [`Projection`] for a document by running its format-specific
/// [`ContentParser`] through the SAME public projection pipeline the production
/// validation path uses: `project(&issue).with_sections(&body, &parser)`.
///
/// This is the seam that makes content rules format-agnostic — every format is
/// normalized into the one canonical [`Projection`] shape before any rule runs.
fn projection_json(parser: &dyn ContentParser, body: &str) -> Value {
    // The body is what the projection parses; the issue carries it as its
    // description, exactly as in production.
    let issue = Issue::new("Plan".to_string(), body.to_string());
    let projection = project(&issue).with_sections(&issue.description, parser);
    serde_json::to_value(&projection).expect("projection serializes")
}

/// Evaluate the canonical `[hard]`-criterion content rule against a serialized
/// [`Projection`] and return its findings.
///
/// This is the JSON-Schema `contains`/`minContains` rule shape applied to
/// `sections.success_criteria.items`: at least one item must match `^\[hard\]`.
/// Operating on the projection JSON (not on `ParsedContent` directly) means the
/// rule sees only the format-agnostic shape, so identical projections must yield
/// identical findings.
fn hard_criterion_findings(projection: &Value) -> Vec<Finding> {
    const PATH: &str = "sections.success_criteria.items";
    let items = projection
        .get("sections")
        .and_then(|s| s.get("success_criteria"))
        .and_then(|sc| sc.get("items"))
        .and_then(Value::as_array);

    let has_hard = items
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .any(|i| i.starts_with("[hard]"))
        })
        .unwrap_or(false);

    if has_hard {
        Vec::new()
    } else {
        vec![Finding {
            path: PATH.to_string(),
            message: "must contain at least one item matching ^\\[hard\\]".to_string(),
        }]
    }
}

/// Criterion 2 (end-to-end): a content rule produces IDENTICAL findings across
/// Markdown, HTML, and XML for semantically equivalent inputs.
///
/// Unlike a shallow `ParsedContent.sections` peek, this drives the REAL pipeline
/// for each format: format-specific parse -> `Projection::with_sections` ->
/// evaluate the `[hard]`-criterion `contains` rule over the projection JSON.
#[test]
fn test_content_rule_findings_identical_across_formats() {
    // --- Passing document: the hard criterion is present in all three formats. ---
    let md = hard_criterion_findings(&projection_json(&MarkdownContentParser, MARKDOWN));
    let html = hard_criterion_findings(&projection_json(&HtmlContentParser, HTML));
    let xml = hard_criterion_findings(&projection_json(&XmlContentParser, XML));

    assert!(
        md.is_empty(),
        "Markdown should satisfy the rule, got {md:?}"
    );
    assert_eq!(md, html, "Markdown and HTML findings differ");
    assert_eq!(md, xml, "Markdown and XML findings differ");
    assert_eq!(html, xml, "HTML and XML findings differ");

    // --- Failing document: the hard criterion is missing in all three formats. ---
    // The rule must produce an IDENTICAL, NON-empty finding set for each format.
    let md_miss =
        hard_criterion_findings(&projection_json(&MarkdownContentParser, MARKDOWN_NO_HARD));
    let html_miss = hard_criterion_findings(&projection_json(&HtmlContentParser, HTML_NO_HARD));
    let xml_miss = hard_criterion_findings(&projection_json(&XmlContentParser, XML_NO_HARD));

    assert!(
        !md_miss.is_empty(),
        "a document missing the hard criterion must produce a finding"
    );
    assert_eq!(
        md_miss, html_miss,
        "missing-criterion findings differ (HTML)"
    );
    assert_eq!(md_miss, xml_miss, "missing-criterion findings differ (XML)");
    assert_eq!(
        html_miss, xml_miss,
        "missing-criterion findings differ (HTML vs XML)"
    );
}
