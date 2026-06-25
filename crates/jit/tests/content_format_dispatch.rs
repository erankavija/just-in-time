//! End-to-end PRODUCTION-DISPATCH tests for per-issue `content_format`.
//!
//! The sibling `content_parser_cross_format.rs` proves the three parsers produce
//! identical canonical output. THIS file proves something different and stronger:
//! that the PRODUCTION validation entry points (`evaluate_local` and
//! `evaluate_graph`) actually DISPATCH to the parser selected by
//! [`content_parser_for`] — issue `content_format` -> repo default -> Markdown —
//! rather than always using Markdown.
//!
//! The proof technique: feed an HTML (or XML) body to a rule that depends on the
//! parsed `sections`. If the production path genuinely selected the HTML/XML
//! parser, the section is found and the rule behaves accordingly. If it had
//! silently used Markdown, the `<h2>`/`<heading>` markup is opaque text, no
//! section is found, and the rule behaves the OPPOSITE way. The two outcomes are
//! distinguishable, so a passing test can only mean the selected parser ran.
//!
//! Feature matrix:
//! - Default build (no features): Markdown dispatch + the feature-not-compiled
//!   error path are always exercised.
//! - `--features html`: HTML per-issue override, repo-default, and
//!   absent->markdown cases.
//! - `--features xml`: the XML analogue.

use jit::domain::{ContentFormat, Issue};
use jit::validation::local::evaluate_local;
#[cfg(any(not(feature = "html"), not(feature = "xml")))]
use jit::validation::local::LocalEvalError;
use jit::validation::rules::RuleSet;
use std::path::Path;

/// A `require-section` rule (enforce/error) keyed on epics: the issue MUST have a
/// parsed `Success Criteria` section. Whether the section is found depends ENTIRELY
/// on which parser ran over the body, which is exactly the dispatch we want to
/// observe.
fn require_criteria_rule() -> RuleSet {
    RuleSet::from_toml_str(
        r#"
[[rules]]
name = "epic-needs-criteria"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { require-section = { heading = "Success Criteria" } }
"#,
        Path::new("/nonexistent"),
    )
    .unwrap()
}

fn epic(body: &str, format: Option<ContentFormat>) -> Issue {
    let mut issue = Issue::new("An epic".to_string(), body.to_string());
    issue.labels = vec!["type:epic".to_string()];
    issue.content_format = format;
    issue
}

const MARKDOWN_BODY: &str = "## Success Criteria\n\n- [hard] REQ-01: ship it\n";
const HTML_BODY: &str =
    "<h2>Success Criteria</h2>\n<ul>\n  <li>[hard] REQ-01: ship it</li>\n</ul>\n";
const XML_BODY: &str = "<document>\n  <section>\n    <heading level=\"2\">Success Criteria</heading>\n    <item>[hard] REQ-01: ship it</item>\n  </section>\n</document>\n";

// ---------------------------------------------------------------------------
// Default-build cases (always compiled): Markdown dispatch + feature-off error.
// ---------------------------------------------------------------------------

/// Absent per-issue format + Markdown repo default -> Markdown parser is used:
/// the Markdown `## Success Criteria` heading is found and the rule passes.
#[test]
fn test_absent_format_falls_back_to_markdown_repo_default() {
    let rules = require_criteria_rule();
    let issue = epic(MARKDOWN_BODY, None);
    let eval = evaluate_local(&issue, &rules, ContentFormat::Markdown).unwrap();
    assert!(
        !eval.is_blocking(),
        "Markdown section must be found via the Markdown parser: {:?}",
        eval.findings()
    );
}

/// Absent per-issue format + Markdown repo default, fed an HTML body: the
/// Markdown parser does NOT recognize `<h2>` as a section heading, so the
/// required section is missing and the rule BLOCKS. This is the negative control
/// that the later html-feature test contrasts against — it pins that Markdown is
/// genuinely the parser in play here.
#[test]
fn test_html_body_under_markdown_default_is_not_parsed_as_sections() {
    let rules = require_criteria_rule();
    let issue = epic(HTML_BODY, None);
    let eval = evaluate_local(&issue, &rules, ContentFormat::Markdown).unwrap();
    assert!(
        eval.is_blocking(),
        "HTML markup must be opaque to the Markdown parser (no section found)"
    );
}

/// Per-issue `content_format = html` with the `html` feature NOT compiled must
/// return a CLEAR error from the production path — never a silent Markdown
/// fallback (which would mis-parse the body).
#[cfg(not(feature = "html"))]
#[test]
fn test_html_selected_without_feature_errors_not_silent_fallback() {
    let rules = require_criteria_rule();
    let issue = epic(HTML_BODY, Some(ContentFormat::Html));
    let err = evaluate_local(&issue, &rules, ContentFormat::Markdown).unwrap_err();
    assert!(
        matches!(err, LocalEvalError::ContentParser(_)),
        "expected a ContentParser feature error, got {err:?}"
    );
    assert!(
        err.to_string().contains("html") && err.to_string().contains("feature"),
        "error must name the missing html feature: {err}"
    );
}

/// The XML analogue of the feature-off error path.
#[cfg(not(feature = "xml"))]
#[test]
fn test_xml_selected_without_feature_errors_not_silent_fallback() {
    let rules = require_criteria_rule();
    let issue = epic(XML_BODY, Some(ContentFormat::Xml));
    let err = evaluate_local(&issue, &rules, ContentFormat::Markdown).unwrap_err();
    assert!(
        matches!(err, LocalEvalError::ContentParser(_)),
        "expected a ContentParser feature error, got {err:?}"
    );
    assert!(
        err.to_string().contains("xml") && err.to_string().contains("feature"),
        "error must name the missing xml feature: {err}"
    );
}

// ---------------------------------------------------------------------------
// HTML feature: prove production validation USES the HTML parser.
// ---------------------------------------------------------------------------

/// Per-issue `content_format = html` overrides a Markdown repo default: the HTML
/// `<h2>Success Criteria</h2>` section is parsed by the HTML parser, so the
/// require-section rule PASSES. Combined with
/// `test_html_body_under_markdown_default_is_not_parsed_as_sections` (same body,
/// Markdown parser -> BLOCKS), this proves the per-issue override actually
/// switched the parser in production.
#[cfg(feature = "html")]
#[test]
fn test_html_per_issue_override_uses_html_parser_in_production() {
    let rules = require_criteria_rule();
    let issue = epic(HTML_BODY, Some(ContentFormat::Html));
    // Repo default is Markdown; only the per-issue override selects HTML.
    let eval = evaluate_local(&issue, &rules, ContentFormat::Markdown).unwrap();
    assert!(
        !eval.is_blocking(),
        "HTML section must be parsed via the HTML parser: {:?}",
        eval.findings()
    );
}

/// Repo-default `content_format = html` with NO per-issue override also selects
/// the HTML parser, proving the repo-default fallback wiring works in production.
#[cfg(feature = "html")]
#[test]
fn test_html_repo_default_uses_html_parser_in_production() {
    let rules = require_criteria_rule();
    let issue = epic(HTML_BODY, None);
    let eval = evaluate_local(&issue, &rules, ContentFormat::Html).unwrap();
    assert!(
        !eval.is_blocking(),
        "HTML section must be parsed via the repo-default HTML parser: {:?}",
        eval.findings()
    );
}

/// The graph path dispatches per-issue too: an epic with HTML success criteria
/// and an UNCOVERED criterion produces a label-coverage finding ONLY because the
/// HTML parser extracted the criterion id from the `<h2>` section. Under Markdown
/// the section is opaque, no criteria are found, and the rule is vacuously
/// satisfied — so a finding here can only come from the HTML parser running.
#[cfg(feature = "html")]
#[test]
fn test_html_graph_label_coverage_uses_html_parser_in_production() {
    use jit::type_hierarchy::HierarchyConfig;
    use jit::validation::graph::{evaluate_graph, DriftInputs};

    let rule = RuleSet::from_toml_str(
        "[[rules]]\nname = \"coverage\"\nwhen = { type = \"epic\" }\n\
         severity = \"error\"\nassert = { label-coverage = { child-state = \"done\" } }\n",
        Path::new("/nonexistent"),
    )
    .unwrap()
    .rules
    .into_iter()
    .next()
    .unwrap();

    // Epic with an HTML success-criteria section declaring REQ-01, no covering child.
    let mut html_epic = Issue::new("epic".to_string(), HTML_BODY.to_string());
    html_epic.labels = vec!["type:epic".to_string()];
    html_epic.content_format = Some(ContentFormat::Html);

    let rules = vec![&rule];
    let findings = evaluate_graph(
        &rules,
        &[html_epic],
        &HierarchyConfig::default(),
        ContentFormat::Markdown,
        chrono::Utc::now(),
        &std::collections::HashMap::new(),
        &DriftInputs::none(),
    );
    assert_eq!(
        findings.len(),
        1,
        "HTML criterion must be extracted by the HTML parser and reported uncovered: {findings:?}"
    );
    assert!(findings[0].finding.message.contains("REQ-01"));
}

// ---------------------------------------------------------------------------
// XML feature: prove production validation USES the XML parser.
// ---------------------------------------------------------------------------

/// Per-issue `content_format = xml` overrides a Markdown repo default and the XML
/// `<heading level="2">Success Criteria</heading>` section is parsed by the XML
/// parser, so the require-section rule PASSES.
#[cfg(feature = "xml")]
#[test]
fn test_xml_per_issue_override_uses_xml_parser_in_production() {
    let rules = require_criteria_rule();
    let issue = epic(XML_BODY, Some(ContentFormat::Xml));
    let eval = evaluate_local(&issue, &rules, ContentFormat::Markdown).unwrap();
    assert!(
        !eval.is_blocking(),
        "XML section must be parsed via the XML parser: {:?}",
        eval.findings()
    );
}

/// Repo-default `content_format = xml` with no per-issue override selects the XML
/// parser in production.
#[cfg(feature = "xml")]
#[test]
fn test_xml_repo_default_uses_xml_parser_in_production() {
    let rules = require_criteria_rule();
    let issue = epic(XML_BODY, None);
    let eval = evaluate_local(&issue, &rules, ContentFormat::Xml).unwrap();
    assert!(
        !eval.is_blocking(),
        "XML section must be parsed via the repo-default XML parser: {:?}",
        eval.findings()
    );
}
