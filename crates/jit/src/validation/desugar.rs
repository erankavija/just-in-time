//! Desugar shorthand rule kinds into equivalent JSON Schema (Draft 2020-12).
//!
//! The validation engine has exactly one evaluation primitive: a JSON Schema run
//! against the canonical [`Projection`](crate::domain::Projection) shape (DR §5,
//! §5.2). The ergonomic shorthand assertion kinds (`require-label`,
//! `label-value-pattern`, `require-section`, `require-doc-type`) are therefore
//! NOT a parallel mechanism — they are sugar that lowers to JSON Schema here, so
//! a single cached validator underlies every local rule.
//!
//! This module is **pure**: [`desugar`] is a deterministic function of its
//! input [`Assertion`] with no I/O. It does not touch the filesystem, compile
//! schemas, or evaluate anything; it only produces the schema
//! [`serde_json::Value`] that a downstream task (`local-eval`) will feed to the
//! [`SchemaEngine`](crate::validation::engine::SchemaEngine).
//!
//! # Target shape
//!
//! Generated schemas validate against the [`Projection`] contract
//! (`type`/`state`/`priority`/`labels.<ns>`/`doc_types`/`sections.<slug>.items`).
//! Each desugaring constrains exactly the slice of that shape its shorthand
//! addresses and leaves the rest unconstrained (`type: object`), so unrelated
//! projection fields never trigger spurious findings.
//!
//! # Coverage
//!
//! Only the four LOCAL shorthand kinds desugar. [`desugar`] returns `None` for
//! [`Assertion::JsonSchema`] (already a schema), [`Assertion::CheckerCommand`]
//! (an escape hatch, not schema-expressible), and the graph kinds
//! ([`Assertion::LabelCoverage`], [`Assertion::LabelReference`],
//! [`Assertion::DependencyShape`]) which need the whole store and have no schema.

use serde_json::{json, Value};

use crate::document::slugify_heading;
use crate::validation::rules::Assertion;

/// Lower a shorthand [`Assertion`] to its equivalent JSON Schema (Draft 2020-12).
///
/// Returns `Some(schema)` for the four local shorthand kinds and `None` for the
/// raw [`Assertion::JsonSchema`], the [`Assertion::CheckerCommand`] escape hatch,
/// and the graph kinds (which carry no schema). The returned schema validates
/// against the canonical [`Projection`](crate::domain::Projection) shape and is
/// safe to feed directly to the
/// [`SchemaEngine`](crate::validation::engine::SchemaEngine).
///
/// This function is pure: no I/O, deterministic in its input.
///
/// # Examples
///
/// ```
/// use jit::validation::desugar::desugar;
/// use jit::validation::rules::Assertion;
///
/// // A shorthand kind lowers to a JSON Schema object.
/// let schema = desugar(&Assertion::RequireDocType {
///     doc_type: "design".to_string(),
/// })
/// .expect("require-doc-type desugars");
/// assert_eq!(schema["type"], "object");
///
/// // A raw JSON Schema kind carries its own schema and is not desugared here.
/// let checker = Assertion::CheckerCommand("./check.sh".to_string());
/// assert!(desugar(&checker).is_none());
/// ```
pub fn desugar(assertion: &Assertion) -> Option<Value> {
    match assertion {
        Assertion::RequireLabel { label, min, max } => {
            Some(desugar_require_label(label, *min, *max))
        }
        Assertion::LabelValuePattern { namespace, regex } => {
            Some(desugar_label_value_pattern(namespace, regex))
        }
        Assertion::RequireSection { heading } => Some(desugar_require_section(heading)),
        Assertion::RequireDocType { doc_type } => Some(desugar_require_doc_type(doc_type)),
        Assertion::JsonSchema(_)
        | Assertion::CheckerCommand(_)
        | Assertion::LabelCoverage { .. }
        | Assertion::LabelReference { .. }
        | Assertion::DependencyShape { .. } => None,
    }
}

/// Desugar a `require-label` shorthand into a cardinality constraint on the
/// `labels.<ns>` array of the projection.
///
/// Two label forms are handled per the rules model:
///
/// - **Wildcard `ns:*`** — constrains the *size* of the namespace's array via
///   `minItems`/`maxItems`. Since the kind is "require", an absent `min`
///   defaults to `1` (at least one label in the namespace), which is expressed
///   by making `<ns>` a `required` property of `labels`.
/// - **Exact `ns:value`** — constrains how many array entries equal `value` via
///   `contains` (with `const: value`) plus `minContains`/`maxContains`. An
///   absent `min` again defaults to `1` (the value must appear at least once).
///
/// A label with no `:` is treated as a bare namespace with no value (wildcard
/// semantics on the whole namespace).
///
/// # Examples
///
/// ```
/// use jit::validation::desugar::desugar;
/// use jit::validation::rules::Assertion;
///
/// // Wildcard with a minimum: the `req` namespace must hold at least 2 labels.
/// let schema = desugar(&Assertion::RequireLabel {
///     label: "req:*".to_string(),
///     min: Some(2),
///     max: None,
/// })
/// .unwrap();
/// assert_eq!(schema["properties"]["labels"]["properties"]["req"]["minItems"], 2);
/// ```
fn desugar_require_label(label: &str, min: Option<u32>, max: Option<u32>) -> Value {
    // Default `min` to 1: a "require-label" with no explicit minimum still
    // requires the label to be present at least once.
    let effective_min = min.unwrap_or(1);

    match label.split_once(':') {
        // Wildcard: bound the SIZE of the namespace's array.
        Some((namespace, "*")) | Some((namespace, "")) => {
            label_size_schema(namespace, effective_min, max)
        }
        // Exact `ns:value`: bound the COUNT of array entries equal to `value`.
        Some((namespace, value)) => label_count_schema(namespace, value, effective_min, max),
        // Bare namespace with no `:` — treat as wildcard over the whole namespace.
        None => label_size_schema(label, effective_min, max),
    }
}

/// Build a schema bounding the size of `labels.<namespace>` (the `ns:*` form).
fn label_size_schema(namespace: &str, min: u32, max: Option<u32>) -> Value {
    // Build the array sub-schema directly as a JSON object so no fallible
    // downcast (and therefore no panic path) is needed in library code.
    let mut obj = serde_json::Map::new();
    obj.insert("type".to_string(), json!("array"));
    if min > 0 {
        obj.insert("minItems".to_string(), json!(min));
    }
    if let Some(max) = max {
        obj.insert("maxItems".to_string(), json!(max));
    }
    let array_schema = Value::Object(obj);

    let mut schema = json!({
        "type": "object",
        "properties": {
            "labels": {
                "type": "object",
                "properties": { namespace: array_schema },
            }
        }
    });
    // `min >= 1` means the namespace must exist at all, so require it on `labels`.
    if min >= 1 {
        schema["properties"]["labels"]["required"] = json!([namespace]);
    }
    schema
}

/// Build a schema bounding how many `labels.<namespace>` entries equal `value`
/// (the exact `ns:value` form), using Draft 2020-12 `contains`/`minContains`/
/// `maxContains`.
fn label_count_schema(namespace: &str, value: &str, min: u32, max: Option<u32>) -> Value {
    // Build the array sub-schema directly as a JSON object so no fallible
    // downcast (and therefore no panic path) is needed in library code.
    let mut obj = serde_json::Map::new();
    obj.insert("type".to_string(), json!("array"));
    obj.insert("contains".to_string(), json!({ "const": value }));
    // `minContains: 0` would make `contains` vacuously satisfied, so only a
    // positive minimum actually constrains presence.
    if min > 0 {
        obj.insert("minContains".to_string(), json!(min));
    } else {
        obj.insert("minContains".to_string(), json!(0));
    }
    if let Some(max) = max {
        obj.insert("maxContains".to_string(), json!(max));
    }
    let array_schema = Value::Object(obj);

    let mut schema = json!({
        "type": "object",
        "properties": {
            "labels": {
                "type": "object",
                "properties": { namespace: array_schema },
            }
        }
    });
    // The value must appear at least once unless `min` was explicitly 0, which
    // requires the namespace array to exist.
    if min >= 1 {
        schema["properties"]["labels"]["required"] = json!([namespace]);
    }
    schema
}

/// Desugar a `label-value-pattern` shorthand into a `pattern` constraint on the
/// items of `labels.<namespace>`.
///
/// Every value in the namespace's array must match `regex`. The constraint is
/// applied to the array's `items`, so the schema does not require the namespace
/// to be present — it only constrains the values that ARE present (an absent
/// namespace vacuously satisfies the rule, matching "value format" semantics).
///
/// # Examples
///
/// ```
/// use jit::validation::desugar::desugar;
/// use jit::validation::rules::Assertion;
///
/// let schema = desugar(&Assertion::LabelValuePattern {
///     namespace: "req".to_string(),
///     regex: "^REQ-[0-9]+$".to_string(),
/// })
/// .unwrap();
/// assert_eq!(
///     schema["properties"]["labels"]["properties"]["req"]["items"]["pattern"],
///     "^REQ-[0-9]+$"
/// );
/// ```
fn desugar_label_value_pattern(namespace: &str, regex: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "labels": {
                "type": "object",
                "properties": {
                    namespace: {
                        "type": "array",
                        "items": { "type": "string", "pattern": regex }
                    }
                }
            }
        }
    })
}

/// Desugar a `require-section` shorthand into a schema asserting that
/// `sections.<slug>` exists and is non-empty.
///
/// The heading is normalized to its slug with
/// [`slugify_heading`](crate::document::slugify_heading) so it matches the slug
/// key the [`Projection`](crate::domain::Projection) uses for the section. The
/// section is required to exist and to carry at least one `items` entry
/// (`minItems: 1`), which is what "the section is present" means for a parsed
/// body.
///
/// # Examples
///
/// ```
/// use jit::validation::desugar::desugar;
/// use jit::validation::rules::Assertion;
///
/// let schema = desugar(&Assertion::RequireSection {
///     heading: "Success Criteria".to_string(),
/// })
/// .unwrap();
/// // The heading slugifies to the projection's section key.
/// assert_eq!(schema["properties"]["sections"]["required"][0], "success_criteria");
/// ```
fn desugar_require_section(heading: &str) -> Value {
    let slug = slugify_heading(heading);
    json!({
        "type": "object",
        "required": ["sections"],
        "properties": {
            "sections": {
                "type": "object",
                "required": [slug],
                "properties": {
                    slug: {
                        "type": "object",
                        "required": ["items"],
                        "properties": {
                            "items": { "type": "array", "minItems": 1 }
                        }
                    }
                }
            }
        }
    })
}

/// Desugar a `require-doc-type` shorthand into a schema asserting that
/// `doc_types` contains the value.
///
/// `doc_types` is an array of distinct document-type strings on the projection;
/// requiring a value means the array must `contain` that constant.
///
/// # Examples
///
/// ```
/// use jit::validation::desugar::desugar;
/// use jit::validation::rules::Assertion;
///
/// let schema = desugar(&Assertion::RequireDocType {
///     doc_type: "design".to_string(),
/// })
/// .unwrap();
/// assert_eq!(
///     schema["properties"]["doc_types"]["contains"]["const"],
///     "design"
/// );
/// ```
fn desugar_require_doc_type(doc_type: &str) -> Value {
    json!({
        "type": "object",
        "required": ["doc_types"],
        "properties": {
            "doc_types": {
                "type": "array",
                "contains": { "const": doc_type }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::engine::SchemaEngine;
    use crate::validation::rules::{Assertion, Rule, SchemaSource, Scope, Selector, Severity};
    use serde_json::Value;
    use std::path::PathBuf;

    /// Wrap a JSON Schema value in a `Rule` carrying it as a `json-schema`
    /// assertion, so it can be driven through the real [`SchemaEngine`].
    fn rule_for_schema(name: &str, schema: Value) -> Rule {
        Rule {
            name: name.to_string(),
            when: Selector::default(),
            severity: Severity::Error,
            enforce: false,
            assert: Assertion::JsonSchema(SchemaSource {
                reference: "inline".to_string(),
                path: PathBuf::from("inline"),
                schema,
            }),
            scope: Scope::Local,
        }
    }

    /// Returns whether the schema ACCEPTS the projection (no findings).
    fn accepts(engine: &SchemaEngine, schema: &Value, projection: &Value) -> bool {
        let rule = rule_for_schema("under-test", schema.clone());
        engine
            .validate(&rule, projection)
            .expect("schema compiles")
            .is_empty()
    }

    /// Assert that the desugared schema and a hand-written schema agree on
    /// accept/reject for every projection in `cases`, driving BOTH through the
    /// `SchemaEngine`. This is the core equivalence harness for every kind.
    fn assert_equivalent(desugared: &Value, handwritten: &Value, cases: &[Value]) {
        let engine = SchemaEngine::new();
        for projection in cases {
            let d = accepts(&engine, desugared, projection);
            let h = accepts(&engine, handwritten, projection);
            assert_eq!(
                d, h,
                "desugared vs hand-written disagree on {projection}: desugared accepts={d}, hand-written accepts={h}"
            );
        }
    }

    // --- require-label: wildcard form -------------------------------------

    #[test]
    fn test_require_label_wildcard_matches_handwritten() {
        // `req:*` with min=1, max=2: the `req` namespace must hold 1..=2 labels.
        let desugared = desugar(&Assertion::RequireLabel {
            label: "req:*".to_string(),
            min: Some(1),
            max: Some(2),
        })
        .unwrap();
        let handwritten = json!({
            "type": "object",
            "properties": {
                "labels": {
                    "type": "object",
                    "required": ["req"],
                    "properties": {
                        "req": { "type": "array", "minItems": 1, "maxItems": 2 }
                    }
                }
            }
        });
        let cases = vec![
            json!({ "labels": {} }),                              // missing ns -> reject
            json!({ "labels": { "req": [] } }),                   // empty -> reject
            json!({ "labels": { "req": ["REQ-01"] } }),           // 1 -> accept
            json!({ "labels": { "req": ["REQ-01", "REQ-02"] } }), // 2 -> accept
            json!({ "labels": { "req": ["a", "b", "c"] } }),      // 3 -> reject (> max)
            json!({ "labels": { "type": ["epic"] } }),            // other ns -> reject
        ];
        assert_equivalent(&desugared, &handwritten, &cases);
    }

    #[test]
    fn test_require_label_wildcard_default_min_requires_presence() {
        // No explicit min: "require-label" still demands at least one label.
        let desugared = desugar(&Assertion::RequireLabel {
            label: "req:*".to_string(),
            min: None,
            max: None,
        })
        .unwrap();
        let handwritten = json!({
            "type": "object",
            "properties": {
                "labels": {
                    "type": "object",
                    "required": ["req"],
                    "properties": { "req": { "type": "array", "minItems": 1 } }
                }
            }
        });
        let cases = vec![
            json!({ "labels": {} }),
            json!({ "labels": { "req": [] } }),
            json!({ "labels": { "req": ["REQ-01"] } }),
            json!({ "labels": { "req": ["a", "b", "c"] } }),
        ];
        assert_equivalent(&desugared, &handwritten, &cases);
    }

    // --- require-label: exact form ----------------------------------------

    #[test]
    fn test_require_label_exact_matches_handwritten() {
        // `profile:sdd` exact: the `profile` namespace must contain "sdd".
        let desugared = desugar(&Assertion::RequireLabel {
            label: "profile:sdd".to_string(),
            min: None,
            max: None,
        })
        .unwrap();
        let handwritten = json!({
            "type": "object",
            "properties": {
                "labels": {
                    "type": "object",
                    "required": ["profile"],
                    "properties": {
                        "profile": {
                            "type": "array",
                            "contains": { "const": "sdd" },
                            "minContains": 1
                        }
                    }
                }
            }
        });
        let cases = vec![
            json!({ "labels": {} }),
            json!({ "labels": { "profile": [] } }),
            json!({ "labels": { "profile": ["sdd"] } }),
            json!({ "labels": { "profile": ["other"] } }),
            json!({ "labels": { "profile": ["other", "sdd"] } }),
        ];
        assert_equivalent(&desugared, &handwritten, &cases);
    }

    #[test]
    fn test_require_label_exact_with_max_count() {
        // `profile:sdd` with max=1: "sdd" must appear exactly once (min default 1).
        let desugared = desugar(&Assertion::RequireLabel {
            label: "profile:sdd".to_string(),
            min: Some(1),
            max: Some(1),
        })
        .unwrap();
        let handwritten = json!({
            "type": "object",
            "properties": {
                "labels": {
                    "type": "object",
                    "required": ["profile"],
                    "properties": {
                        "profile": {
                            "type": "array",
                            "contains": { "const": "sdd" },
                            "minContains": 1,
                            "maxContains": 1
                        }
                    }
                }
            }
        });
        let cases = vec![
            json!({ "labels": { "profile": ["sdd"] } }),
            json!({ "labels": { "profile": ["sdd", "sdd"] } }),
            json!({ "labels": { "profile": ["sdd", "other"] } }),
            json!({ "labels": {} }),
        ];
        assert_equivalent(&desugared, &handwritten, &cases);
    }

    // --- label-value-pattern ----------------------------------------------

    #[test]
    fn test_label_value_pattern_matches_handwritten() {
        let desugared = desugar(&Assertion::LabelValuePattern {
            namespace: "req".to_string(),
            regex: "^REQ-[0-9]+$".to_string(),
        })
        .unwrap();
        let handwritten = json!({
            "type": "object",
            "properties": {
                "labels": {
                    "type": "object",
                    "properties": {
                        "req": {
                            "type": "array",
                            "items": { "type": "string", "pattern": "^REQ-[0-9]+$" }
                        }
                    }
                }
            }
        });
        let cases = vec![
            json!({ "labels": {} }),                    // absent ns -> accept (vacuous)
            json!({ "labels": { "req": [] } }),         // empty -> accept
            json!({ "labels": { "req": ["REQ-01"] } }), // matches -> accept
            json!({ "labels": { "req": ["REQ-01", "REQ-99"] } }), // all match -> accept
            json!({ "labels": { "req": ["bad"] } }),    // no match -> reject
            json!({ "labels": { "req": ["REQ-01", "nope"] } }), // one bad -> reject
        ];
        assert_equivalent(&desugared, &handwritten, &cases);
    }

    // --- require-section ---------------------------------------------------

    #[test]
    fn test_require_section_matches_handwritten() {
        // Heading slugifies to "success_criteria" (the projection's section key).
        let desugared = desugar(&Assertion::RequireSection {
            heading: "Success Criteria".to_string(),
        })
        .unwrap();
        // Confirm the slug matched the projection convention.
        assert_eq!(
            desugared["properties"]["sections"]["required"][0],
            "success_criteria"
        );
        let handwritten = json!({
            "type": "object",
            "required": ["sections"],
            "properties": {
                "sections": {
                    "type": "object",
                    "required": ["success_criteria"],
                    "properties": {
                        "success_criteria": {
                            "type": "object",
                            "required": ["items"],
                            "properties": { "items": { "type": "array", "minItems": 1 } }
                        }
                    }
                }
            }
        });
        let cases = vec![
            json!({}),                                                      // no sections -> reject
            json!({ "sections": {} }), // section absent -> reject
            json!({ "sections": { "success_criteria": { "items": [] } } }), // empty -> reject
            json!({ "sections": { "success_criteria": { "items": ["[hard] x"] } } }), // present -> accept
            json!({ "sections": { "other": { "items": ["y"] } } }), // wrong section -> reject
        ];
        assert_equivalent(&desugared, &handwritten, &cases);
    }

    #[test]
    fn test_require_section_slug_matches_real_projection() {
        // End-to-end: desugar a heading, then validate a REAL projection built
        // from an issue body whose heading text differs in punctuation/case but
        // slugifies identically. The desugared schema must accept it.
        use crate::document::MarkdownContentParser;
        use crate::domain::{project, Issue};

        let desugared = desugar(&Assertion::RequireSection {
            heading: "Success Criteria".to_string(),
        })
        .unwrap();
        let issue = Issue::new(
            "t".to_string(),
            "## Success / Criteria!\n\n- [hard] REQ-01\n".to_string(),
        );
        let projection = project(&issue).with_sections(&issue.description, &MarkdownContentParser);
        let value = serde_json::to_value(&projection).unwrap();

        let engine = SchemaEngine::new();
        assert!(
            accepts(&engine, &desugared, &value),
            "slug from heading must match the projection's section key, projection={value}"
        );
    }

    // --- require-doc-type --------------------------------------------------

    #[test]
    fn test_require_doc_type_matches_handwritten() {
        let desugared = desugar(&Assertion::RequireDocType {
            doc_type: "design".to_string(),
        })
        .unwrap();
        let handwritten = json!({
            "type": "object",
            "required": ["doc_types"],
            "properties": {
                "doc_types": { "type": "array", "contains": { "const": "design" } }
            }
        });
        let cases = vec![
            json!({}),                                  // no doc_types -> reject
            json!({ "doc_types": [] }),                 // empty -> reject
            json!({ "doc_types": ["design"] }),         // present -> accept
            json!({ "doc_types": ["impl", "design"] }), // present among others -> accept
            json!({ "doc_types": ["impl"] }),           // absent -> reject
        ];
        assert_equivalent(&desugared, &handwritten, &cases);
    }

    // --- non-shorthand kinds return None ----------------------------------

    #[test]
    fn test_non_shorthand_kinds_return_none() {
        assert!(desugar(&Assertion::JsonSchema(SchemaSource {
            reference: "schemas/x.json".to_string(),
            path: PathBuf::from("x"),
            schema: json!({ "type": "object" }),
        }))
        .is_none());
        assert!(desugar(&Assertion::CheckerCommand("./c.sh".to_string())).is_none());
        assert!(desugar(&Assertion::LabelCoverage {
            config: toml::value::Table::new()
        })
        .is_none());
        assert!(desugar(&Assertion::LabelReference {
            config: toml::value::Table::new()
        })
        .is_none());
        assert!(desugar(&Assertion::DependencyShape {
            config: toml::value::Table::new()
        })
        .is_none());
    }

    // --- generated schemas compile under Draft 2020-12 --------------------

    #[test]
    fn test_all_desugared_schemas_compile() {
        // Every generated schema must be a valid Draft 2020-12 schema the engine
        // can compile (no findings on an empty object means it compiled and ran).
        let schemas = [
            desugar(&Assertion::RequireLabel {
                label: "req:*".to_string(),
                min: Some(1),
                max: Some(3),
            })
            .unwrap(),
            desugar(&Assertion::RequireLabel {
                label: "profile:sdd".to_string(),
                min: None,
                max: Some(1),
            })
            .unwrap(),
            desugar(&Assertion::LabelValuePattern {
                namespace: "req".to_string(),
                regex: r"^\[hard\]".to_string(),
            })
            .unwrap(),
            desugar(&Assertion::RequireSection {
                heading: "Plan (v2)".to_string(),
            })
            .unwrap(),
            desugar(&Assertion::RequireDocType {
                doc_type: "design".to_string(),
            })
            .unwrap(),
        ];
        let engine = SchemaEngine::new();
        for schema in &schemas {
            let rule = rule_for_schema("compiles", schema.clone());
            // A schema that fails to compile surfaces as Err; just-an-empty-object
            // projection is enough to force compilation.
            engine
                .validate(&rule, &json!({}))
                .unwrap_or_else(|e| panic!("schema failed to compile: {e}; schema={schema}"));
        }
    }
}
