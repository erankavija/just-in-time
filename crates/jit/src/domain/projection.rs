//! Pure projection of an [`Issue`] into the canonical validation shape.
//!
//! Every user-authored schema in `.jit/schemas/` validates against this shape, so
//! the [`Projection`] struct is a **documented, stable contract** (DR §6.1). Its
//! schema is generated from this type via `schemars` and asserted by a test as
//! the contract user schemas depend on; do not change it casually.
//!
//! # Shape
//!
//! ```jsonc
//! {
//!   "type": "epic",                 // from the `type:*` label, if any
//!   "state": "ready",               // Issue.state
//!   "priority": "high",             // Issue.priority
//!   "labels": {                     // all labels grouped by namespace
//!     "type": ["epic"],
//!     "req":  ["REQ-01", "REQ-02"],
//!     "epic": ["validation-engine"]
//!   },
//!   "doc_types": ["design", "implementation"],  // from documents[].doc_type
//!   "sections": {                   // parsed from the description, LAZY
//!     "success_criteria": {
//!       "heading": "Success Criteria",
//!       "level": 2,
//!       "items": ["[hard] REQ-01 ...", "[aspirational] ..."]
//!     }
//!   }
//! }
//! ```
//!
//! # Laziness (DR §6.1, perf)
//!
//! [`project`] reads only the cheap selector fields (`type`/`state`/`priority`/
//! `labels`/`doc_types`) directly off the [`Issue`] — it NEVER parses the
//! Markdown description. The `sections` view is computed on demand by
//! [`Projection::ensure_sections`] / [`Projection::with_sections`], which take a
//! [`ContentParser`]. A write whose matching rules need no body assertion thus
//! never parses Markdown. `sections` serializes as `null` until populated.
//!
//! # Purity
//!
//! This module is pure: no filesystem, no I/O. `project` is a deterministic
//! function of the input [`Issue`]; section computation is a deterministic
//! function of the issue description and the supplied parser.

use crate::document::{ContentParser, ParsedContent};
use crate::domain::types::Issue;
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::BTreeMap;

/// The canonical JSON shape an [`Issue`] normalizes to before validation.
///
/// See the module docs for the documented contract and laziness rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Projection {
    /// The issue's primary type, taken from its `type:*` label (e.g. `"epic"`).
    /// `None` when the issue carries no `type:` label.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,

    /// The issue's lifecycle state, serialized like [`crate::domain::State`]
    /// (snake_case, e.g. `"in_progress"`).
    pub state: String,

    /// The issue's priority (snake_case, e.g. `"high"`).
    pub priority: String,

    /// All labels grouped by namespace. The key is the namespace
    /// (`"req"`), the value is the list of values in that namespace in label
    /// order. Labels without a `namespace:value` shape are skipped.
    pub labels: BTreeMap<String, Vec<String>>,

    /// Distinct `doc_type` values across the issue's document references.
    pub doc_types: Vec<String>,

    /// The parsed body, keyed by normalized heading slug. `None` until computed
    /// lazily via [`Projection::ensure_sections`] — `project` leaves it unset so
    /// a write with no body-targeting rule never parses Markdown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sections: Option<BTreeMap<String, ProjectedSection>>,
}

/// One section of the parsed body in the projection shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ProjectedSection {
    /// The original heading text (e.g. `"Success Criteria"`).
    pub heading: String,
    /// The heading level (1 for `#`, 2 for `##`, ...).
    pub level: u8,
    /// The raw text of the top-level list items under this heading, in order.
    pub items: Vec<String>,
}

impl From<crate::document::Section> for ProjectedSection {
    fn from(s: crate::document::Section) -> Self {
        ProjectedSection {
            heading: s.heading,
            level: s.level,
            items: s.items,
        }
    }
}

/// The label namespace conventionally carrying an issue's primary type.
const TYPE_NAMESPACE: &str = "type";

/// Group labels by namespace, preserving per-namespace order.
///
/// A label is `namespace:value`; the first `:` splits the two. Labels with no
/// `:` are skipped (they are not part of the namespaced contract).
fn group_labels(labels: &[String]) -> BTreeMap<String, Vec<String>> {
    labels.iter().fold(BTreeMap::new(), |mut acc, label| {
        if let Some((ns, value)) = label.split_once(':') {
            if !ns.is_empty() {
                acc.entry(ns.to_string())
                    .or_default()
                    .push(value.to_string());
            }
        }
        acc
    })
}

/// Distinct doc types across an issue's document references, in first-seen order.
fn collect_doc_types(issue: &Issue) -> Vec<String> {
    issue
        .documents
        .iter()
        .filter_map(|d| d.doc_type.clone())
        .fold(Vec::new(), |mut acc, dt| {
            if !acc.contains(&dt) {
                acc.push(dt);
            }
            acc
        })
}

/// Project an [`Issue`] into its canonical shape, WITHOUT parsing the body.
///
/// Selector fields (`type`/`state`/`priority`/`labels`/`doc_types`) are read
/// directly off the issue. `sections` is left `None`; call
/// [`Projection::ensure_sections`] or [`Projection::with_sections`] to populate
/// it lazily only when a body assertion needs it (DR §6.1).
pub fn project(issue: &Issue) -> Projection {
    let labels = group_labels(&issue.labels);
    let type_ = labels
        .get(TYPE_NAMESPACE)
        .and_then(|vs| vs.first().cloned());

    Projection {
        type_,
        state: serde_plain_state(&issue.state),
        priority: serde_plain_priority(&issue.priority),
        labels,
        doc_types: collect_doc_types(issue),
        sections: None,
    }
}

/// Render a [`State`](crate::domain::State) as its canonical snake_case string,
/// reusing its serde representation so the projection never drifts from the type.
fn serde_plain_state(state: &crate::domain::State) -> String {
    // serde_json renders the enum as a quoted string; strip the quotes.
    serde_json::to_value(state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Render a [`Priority`](crate::domain::Priority) as its canonical snake_case
/// string, reusing its serde representation.
fn serde_plain_priority(priority: &crate::domain::Priority) -> String {
    serde_json::to_value(priority)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

impl Projection {
    /// Compute the `sections` view from `description` if not already present.
    ///
    /// Pure and idempotent: parses the description with the supplied
    /// [`ContentParser`] only on the first call and stores the result. A no-op if
    /// `sections` is already populated.
    pub fn ensure_sections(&mut self, description: &str, parser: &dyn ContentParser) {
        if self.sections.is_none() {
            self.sections = Some(sections_from_parsed(parser.parse(description)));
        }
    }

    /// Consume `self`, returning a projection with `sections` populated.
    ///
    /// Convenience builder over [`Projection::ensure_sections`] for callers that
    /// want the section view eagerly (e.g. tests, `jit validate --explain`).
    pub fn with_sections(mut self, description: &str, parser: &dyn ContentParser) -> Self {
        self.ensure_sections(description, parser);
        self
    }
}

/// Convert a parsed body into the projection's section map.
fn sections_from_parsed(parsed: ParsedContent) -> BTreeMap<String, ProjectedSection> {
    parsed
        .sections
        .into_iter()
        .map(|(k, v)| (k, v.into()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::MarkdownContentParser;
    use crate::domain::types::{DocumentReference, Issue};

    fn issue_with(labels: Vec<&str>, description: &str) -> Issue {
        let mut issue = Issue::new("Title".to_string(), description.to_string());
        issue.labels = labels.into_iter().map(str::to_string).collect();
        issue
    }

    #[test]
    fn test_group_labels_by_namespace() {
        let p = project(&issue_with(
            vec!["type:epic", "req:REQ-01", "req:REQ-02", "epic:ve"],
            "",
        ));
        assert_eq!(p.labels.get("type"), Some(&vec!["epic".to_string()]));
        assert_eq!(
            p.labels.get("req"),
            Some(&vec!["REQ-01".to_string(), "REQ-02".to_string()])
        );
        assert_eq!(p.labels.get("epic"), Some(&vec!["ve".to_string()]));
    }

    #[test]
    fn test_group_labels_skips_unnamespaced() {
        let p = project(&issue_with(vec!["nocolon", "type:task"], ""));
        assert!(!p.labels.contains_key("nocolon"));
        assert_eq!(p.labels.get("type"), Some(&vec!["task".to_string()]));
    }

    #[test]
    fn test_type_from_type_label() {
        let p = project(&issue_with(vec!["type:story"], ""));
        assert_eq!(p.type_, Some("story".to_string()));
    }

    #[test]
    fn test_type_none_without_label() {
        let p = project(&issue_with(vec!["req:REQ-01"], ""));
        assert_eq!(p.type_, None);
    }

    #[test]
    fn test_state_and_priority_are_snake_case() {
        let mut issue = issue_with(vec![], "");
        issue.state = crate::domain::State::InProgress;
        issue.priority = crate::domain::Priority::High;
        let p = project(&issue);
        assert_eq!(p.state, "in_progress");
        assert_eq!(p.priority, "high");
    }

    #[test]
    fn test_doc_types_distinct_in_order() {
        let mut issue = issue_with(vec![], "");
        issue.documents = vec![
            DocumentReference::new("a.md".to_string()).with_type("design".to_string()),
            DocumentReference::new("b.md".to_string()).with_type("design".to_string()),
            DocumentReference::new("c.md".to_string()).with_type("impl".to_string()),
        ];
        let p = project(&issue);
        assert_eq!(p.doc_types, vec!["design".to_string(), "impl".to_string()]);
    }

    #[test]
    fn test_project_does_not_populate_sections() {
        // Laziness: project() must not parse the body.
        let p = project(&issue_with(
            vec![],
            "## Success Criteria\n- [hard] REQ-01\n",
        ));
        assert!(p.sections.is_none());
    }

    #[test]
    fn test_ensure_sections_lazily_parses() {
        let issue = issue_with(vec![], "## Success Criteria\n\n- [hard] REQ-01: x\n");
        let mut p = project(&issue);
        assert!(p.sections.is_none());
        p.ensure_sections(&issue.description, &MarkdownContentParser);
        let sections = p.sections.as_ref().expect("sections populated");
        let sc = sections.get("success_criteria").expect("section present");
        assert_eq!(sc.heading, "Success Criteria");
        assert_eq!(sc.level, 2);
        assert_eq!(sc.items, vec!["[hard] REQ-01: x".to_string()]);
    }

    #[test]
    fn test_ensure_sections_is_idempotent() {
        let issue = issue_with(vec![], "## A\n- one\n");
        let mut p = project(&issue);
        p.ensure_sections(&issue.description, &MarkdownContentParser);
        let first = p.sections.clone();
        // Second call with different content must NOT overwrite (idempotent).
        p.ensure_sections("## B\n- two\n", &MarkdownContentParser);
        assert_eq!(p.sections, first);
    }

    #[test]
    fn test_with_sections_builder() {
        let issue = issue_with(vec![], "## Notes\n- a\n- b\n");
        let p = project(&issue).with_sections(&issue.description, &MarkdownContentParser);
        let sections = p.sections.unwrap();
        let items = &sections.get("notes").unwrap().items;
        assert_eq!(items, &vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn test_projection_is_deterministic() {
        let issue = issue_with(
            vec!["type:epic", "req:REQ-02", "req:REQ-01"],
            "## Success Criteria\n- [hard] REQ-01\n",
        );
        let a = project(&issue).with_sections(&issue.description, &MarkdownContentParser);
        let b = project(&issue).with_sections(&issue.description, &MarkdownContentParser);
        assert_eq!(a, b);
        // And serialization is stable too.
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    #[test]
    fn test_serialized_shape_matches_contract() {
        let issue = issue_with(
            vec!["type:epic", "req:REQ-01"],
            "## Success Criteria\n\n- [hard] REQ-01: do it\n",
        );
        let p = project(&issue).with_sections(&issue.description, &MarkdownContentParser);
        let value = serde_json::to_value(&p).unwrap();
        assert_eq!(value["type"], "epic");
        assert_eq!(value["state"], "backlog");
        assert_eq!(value["priority"], "normal");
        assert_eq!(value["labels"]["type"][0], "epic");
        assert_eq!(value["labels"]["req"][0], "REQ-01");
        assert_eq!(
            value["sections"]["success_criteria"]["items"][0],
            "[hard] REQ-01: do it"
        );
    }

    /// The schemars-generated schema for [`Projection`] is the stable contract
    /// user schemas in `.jit/schemas/` depend on. Pin its key structure so any
    /// accidental shape change fails loudly (DR §6.1, success criterion 4).
    #[test]
    fn test_projection_schema_is_stable_contract() {
        let schema = serde_json::to_value(schemars::schema_for!(Projection)).unwrap();
        let props = &schema["properties"];
        // Top-level contract fields must be present.
        for field in [
            "type",
            "state",
            "priority",
            "labels",
            "doc_types",
            "sections",
        ] {
            assert!(
                props.get(field).is_some(),
                "projection contract missing field `{field}`"
            );
        }
        // labels is an object whose values are arrays of strings.
        assert_eq!(props["labels"]["type"], "object");
        assert_eq!(
            props["labels"]["additionalProperties"]["type"], "array",
            "labels namespace values must be arrays"
        );
        // doc_types is an array of strings.
        assert_eq!(props["doc_types"]["type"], "array");
        // state and priority are strings.
        assert_eq!(props["state"]["type"], "string");
        assert_eq!(props["priority"]["type"], "string");
    }
}
