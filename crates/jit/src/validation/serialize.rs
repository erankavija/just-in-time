//! Serialize a [`RuleSet`] back to `.jit/rules.toml` text plus its referenced
//! `.jit/schemas/*.json` files (DR §8.2, decision D4).
//!
//! This is the inverse of the [`rules`](crate::validation::rules) loader: it
//! renders an arbitrary in-memory [`RuleSet`] (typically the built-in
//! [`default_ruleset`](crate::validation::defaults::default_ruleset)) into a
//! complete, reloadable `rules.toml`. `jit init` uses it (via
//! [`scaffold_default_rules`]) to materialize the fixed default ruleset so the
//! file becomes the single operative source of truth.
//!
//! # Why a custom renderer (no `Serialize` derive)
//!
//! [`Severity`] and [`Selector`] derive only `Deserialize`, and [`Assertion`]
//! mixes shorthand scalars, raw schema values, and graph config tables. Rather
//! than retrofit `Serialize` onto the model, each field is rendered by hand into
//! inline TOML. Regex-bearing shorthands use [`toml_literal_string`] so
//! backslashes survive verbatim (DR §8.1).
//!
//! # JSON Schema rules materialize to files
//!
//! A [`Assertion::JsonSchema`] rule carries its schema inline (the built-in
//! defaults synthesize it; a loaded rule read it from a file). TOML cannot carry
//! raw JSON Schema (DR §8.1), so the serializer writes each schema to
//! `schemas/<sanitized-rule-name>.json` and emits a `json-schema = "schemas/…"`
//! reference. The returned [`SerializedRuleSet`] lists those files so the caller
//! writes them alongside `rules.toml`.
//!
//! # Round-trip contract
//!
//! Re-loading the emitted file with [`RuleSet::load`](crate::validation::rules::RuleSet::load)
//! reproduces every rule field — name, selector, severity, enforce, assertion
//! kind, and (for JSON Schema) the parsed schema VALUE — EXCEPT the
//! [`SchemaSource`](crate::validation::rules::SchemaSource) `reference`/`path`,
//! which necessarily change from the in-code placeholder to the on-disk file
//! reference. The round-trip test compares field-wise, excluding those two.

use crate::validation::rules::{Assertion, Rule, RuleSet, Selector, TypeHierarchyKind};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::Path;

/// A `rules.toml` body together with the schema files it references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializedRuleSet {
    /// The full `.jit/rules.toml` text.
    pub rules_toml: String,
    /// Schema files to write under `.jit/schemas/` (name + pretty JSON content).
    pub schema_files: Vec<SchemaFile>,
}

/// One `.jit/schemas/<name>.json` file the serialized ruleset references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaFile {
    /// File name (no directory), e.g. `"default-label-format.json"`.
    pub name: String,
    /// Pretty-printed JSON Schema content (trailing newline).
    pub content: String,
}

/// Serialize a [`RuleSet`] into a reloadable `rules.toml` + schema files.
///
/// Pure: performs no I/O. The header documents that the file is the operative
/// source. JSON Schema rules are materialized into [`SchemaFile`]s referenced by
/// `schemas/<sanitized-name>.json`.
///
/// # Examples
///
/// ```
/// use jit::validation::rules::RuleSet;
/// use jit::validation::serialize::serialize_ruleset;
/// use std::path::Path;
///
/// let toml = r#"
/// [[rules]]
/// name = "epic-needs-req"
/// when = { type = "epic" }
/// severity = "error"
/// enforce = true
/// assert = { require-label = { label = "req:*", min = 1 } }
/// "#;
/// let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
/// let out = serialize_ruleset(&set);
/// assert!(out.rules_toml.contains("name = \"epic-needs-req\""));
/// assert!(out.schema_files.is_empty());
/// ```
pub fn serialize_ruleset(set: &RuleSet) -> SerializedRuleSet {
    let mut rules_toml = String::new();
    rules_toml.push_str(FILE_HEADER);

    let mut schema_files: Vec<SchemaFile> = Vec::new();
    // Track the schema file STEMS already used so two rule names that sanitize to
    // the same stem (e.g. `default:label` and `default/label` both -> `default-label`)
    // do not silently overwrite each other's `schemas/<stem>.json` (a rule would
    // then validate against the wrong schema). Collisions get a numeric suffix.
    let mut used_stems: HashSet<String> = HashSet::new();

    for rule in &set.rules {
        render_rule(rule, &mut rules_toml, &mut schema_files, &mut used_stems);
        rules_toml.push('\n');
    }

    SerializedRuleSet {
        rules_toml,
        schema_files,
    }
}

/// Scaffold the FIXED default `.jit/rules.toml` (+ referenced `.jit/schemas/
/// *.json`) for `jit init`, derived from `namespaces` via
/// [`default_ruleset`](crate::validation::defaults::default_ruleset).
///
/// This is the ONLY place that materializes the default ruleset to disk (MF4):
/// it runs from `jit init` under the existing write lock. The read path
/// ([`effective_rules`](crate::commands::CommandExecutor::effective_rules)) builds
/// the same defaults in memory and never writes.
///
/// Idempotent by design at the call site: `jit init` invokes this only when
/// `rules.toml` is absent (a present file is the sole source and is left intact),
/// so re-init is a no-op. Writes are atomic (temp + rename).
pub fn scaffold_default_rules(
    jit_root: &Path,
    namespaces: &crate::domain::LabelNamespaces,
) -> Result<()> {
    let set = crate::validation::defaults::default_ruleset(namespaces);
    let serialized = serialize_ruleset(&set);
    write_schema_files(jit_root, &serialized.schema_files)?;
    write_atomic(&jit_root.join("rules.toml"), &serialized.rules_toml)
}

/// Write schema files under `<jit_root>/schemas/`, creating the directory.
fn write_schema_files(jit_root: &Path, files: &[SchemaFile]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let schemas_dir = jit_root.join("schemas");
    std::fs::create_dir_all(&schemas_dir)
        .with_context(|| format!("creating {}", schemas_dir.display()))?;
    for file in files {
        write_atomic(&schemas_dir.join(&file.name), &file.content)?;
    }
    Ok(())
}

/// Write `content` to `path` atomically (temp file + rename).
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Pick a schema-file stem for `rule_name` that is unique within `used_stems`,
/// inserting the chosen stem. Starts from the sanitized name; on collision
/// appends `-2`, `-3`, … until unused. Guarantees no two schema files collide.
fn unique_schema_stem(rule_name: &str, used_stems: &mut HashSet<String>) -> String {
    let base = sanitize_rule_name(rule_name);
    if used_stems.insert(base.clone()) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| used_stems.insert(candidate.clone()))
        .expect("integer counter always yields an unused stem")
}

const FILE_HEADER: &str = "\
# .jit/rules.toml — the operative source of truth for issue/label validation.
#
# Generated by `jit init`. Every rule below (including the `default:*` rules) is
# fully editable; this file is the SOLE source when present. Raw JSON Schema
# rules reference files under `.jit/schemas/`.

";

/// Render one rule's `[[rules]]` block, appending any schema file it needs.
fn render_rule(
    rule: &Rule,
    out: &mut String,
    schema_files: &mut Vec<SchemaFile>,
    used_stems: &mut HashSet<String>,
) {
    out.push_str("[[rules]]\n");
    out.push_str(&format!("name = {}\n", toml_basic_string(&rule.name)));

    if let Some(selector) = render_selector(&rule.when) {
        out.push_str(&format!("when = {selector}\n"));
    }

    out.push_str(&format!(
        "severity = {}\n",
        toml_basic_string(rule.severity.token())
    ));
    out.push_str(&format!("enforce = {}\n", rule.enforce));
    out.push_str(&format!(
        "assert = {}\n",
        render_assertion(&rule.name, &rule.assert, schema_files, used_stems)
    ));
}

/// Render a [`Selector`] as an inline table, or `None` when it is empty (so an
/// empty selector is omitted rather than emitting `when = {}`).
fn render_selector(selector: &Selector) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(ty) = &selector.type_ {
        parts.push(format!("type = {}", toml_basic_string(ty)));
    }
    if let Some(label) = &selector.label {
        parts.push(format!("label = {}", toml_basic_string(label)));
    }
    if let Some(state) = &selector.state {
        let tokens = state.tokens();
        let rendered = if tokens.len() == 1 {
            toml_basic_string(&tokens[0])
        } else {
            let items: Vec<String> = tokens.iter().map(|t| toml_basic_string(t)).collect();
            format!("[{}]", items.join(", "))
        };
        parts.push(format!("state = {rendered}"));
    }
    if let Some(doc_type) = &selector.has_doc_type {
        parts.push(format!("has_doc_type = {}", toml_basic_string(doc_type)));
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("{{ {} }}", parts.join(", ")))
    }
}

/// Render an [`Assertion`] as an inline `{ kind = … }` table. JSON Schema rules
/// push a [`SchemaFile`] and reference it.
fn render_assertion(
    rule_name: &str,
    assertion: &Assertion,
    schema_files: &mut Vec<SchemaFile>,
    used_stems: &mut HashSet<String>,
) -> String {
    match assertion {
        Assertion::RequireLabel { label, min, max } => {
            let mut parts = vec![format!("label = {}", toml_basic_string(label))];
            if let Some(min) = min {
                parts.push(format!("min = {min}"));
            }
            if let Some(max) = max {
                parts.push(format!("max = {max}"));
            }
            format!("{{ require-label = {{ {} }} }}", parts.join(", "))
        }
        Assertion::RequireSection { heading } => format!(
            "{{ require-section = {{ heading = {} }} }}",
            toml_basic_string(heading)
        ),
        Assertion::RequireDocType { doc_type } => format!(
            "{{ require-doc-type = {{ doc-type = {} }} }}",
            toml_basic_string(doc_type)
        ),
        Assertion::LabelValuePattern { namespace, regex } => format!(
            "{{ label-value-pattern = {{ namespace = {}, regex = {} }} }}",
            toml_basic_string(namespace),
            toml_literal_string(regex)
        ),
        Assertion::JsonSchema(source) => {
            let file_name = format!("{}.json", unique_schema_stem(rule_name, used_stems));
            schema_files.push(SchemaFile {
                name: file_name.clone(),
                content: pretty_schema(&source.schema),
            });
            format!(
                "{{ json-schema = {} }}",
                toml_basic_string(&format!("schemas/{file_name}"))
            )
        }
        Assertion::CheckerCommand(cmd) => {
            format!("{{ checker-command = {} }}", toml_basic_string(cmd))
        }
        Assertion::LabelCoverage { config } => {
            format!("{{ label-coverage = {} }}", render_config_table(config))
        }
        Assertion::LabelReference { config } => {
            format!("{{ label-reference = {} }}", render_config_table(config))
        }
        Assertion::DependencyShape { config } => {
            format!("{{ dependency-shape = {} }}", render_config_table(config))
        }
        Assertion::GateRecency {
            max_age_hours,
            gates,
        } => {
            // Round-trip the normalized hours; emit the `gates` filter only when
            // present (an empty list means "all of the issue's gates_required").
            let mut parts = vec![format!("max-age-hours = {max_age_hours}")];
            if !gates.is_empty() {
                let rendered: Vec<String> = gates.iter().map(|g| toml_basic_string(g)).collect();
                parts.push(format!("gates = [{}]", rendered.join(", ")));
            }
            format!("{{ gate-recency = {{ {} }} }}", parts.join(", "))
        }
        Assertion::TypeHierarchy { kind } => {
            let kind_token = match kind {
                TypeHierarchyKind::OrphanLeaf => "orphan-leaf",
                TypeHierarchyKind::StrategicConsistency => "strategic-consistency",
            };
            format!(
                "{{ type-hierarchy = {{ kind = {} }} }}",
                toml_basic_string(kind_token)
            )
        }
    }
}

/// Render a graph rule's `toml::value::Table` as an inline TOML table, reusing
/// `toml_edit` so nested tables/arrays/escaping are handled correctly.
fn render_config_table(table: &toml::value::Table) -> String {
    let value = toml::Value::Table(table.clone());
    let edit = toml_value_to_edit(&value);
    edit.to_string()
}

/// Convert a `toml::Value` into a `toml_edit::Value`, rendering tables as INLINE
/// tables (so the result fits inside an `assert = { … }` line).
fn toml_value_to_edit(value: &toml::Value) -> toml_edit::Value {
    match value {
        toml::Value::String(s) => toml_edit::Value::from(s.clone()),
        toml::Value::Integer(i) => toml_edit::Value::from(*i),
        toml::Value::Float(f) => toml_edit::Value::from(*f),
        toml::Value::Boolean(b) => toml_edit::Value::from(*b),
        toml::Value::Datetime(dt) => toml_edit::Value::from(dt.to_string()),
        toml::Value::Array(items) => {
            let mut arr = toml_edit::Array::new();
            for item in items {
                arr.push(toml_value_to_edit(item));
            }
            toml_edit::Value::Array(arr)
        }
        toml::Value::Table(map) => {
            let mut inline = toml_edit::InlineTable::new();
            for (k, v) in map {
                inline.insert(k, toml_value_to_edit(v));
            }
            toml_edit::Value::InlineTable(inline)
        }
    }
}

/// Sanitize a rule name into a safe schema file stem: lowercase identifier-ish
/// characters preserved, everything else (`:`, `/`, etc.) replaced with `-`.
/// Keeps `default:namespace-values:type` -> `default-namespace-values-type`.
fn sanitize_rule_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Pretty-print a JSON Schema value with a trailing newline (stable output).
fn pretty_schema(schema: &serde_json::Value) -> String {
    let mut s = serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
    s.push('\n');
    s
}

/// Render a string as a TOML BASIC string (`"…"`) with the minimal escaping TOML
/// requires (backslash, double-quote, control chars). Used for plain scalars
/// (names, labels, severities) that contain no regex backslashes.
fn toml_basic_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Render a string as a TOML LITERAL string (`'…'`) so regex backslashes are
/// preserved verbatim (DR §8.1). A literal string cannot contain a single quote;
/// in that (vanishingly rare for a regex) case fall back to a basic string.
pub(crate) fn toml_literal_string(s: &str) -> String {
    if s.contains('\'') {
        toml_basic_string(s)
    } else {
        format!("'{s}'")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{LabelNamespace, LabelNamespaces};
    use crate::validation::defaults::default_ruleset;
    use crate::validation::rules::{RuleSet, Severity};
    use std::collections::HashMap;
    use std::path::Path;

    fn registry(entries: Vec<(&str, LabelNamespace)>) -> LabelNamespaces {
        let mut namespaces = HashMap::new();
        for (name, ns) in entries {
            namespaces.insert(name.to_string(), ns);
        }
        LabelNamespaces {
            schema_version: 2,
            namespaces,
            type_hierarchy: None,
            label_associations: None,
            strategic_types: None,
        }
    }

    /// Write the serialized ruleset to a temp `.jit` and reload it.
    fn round_trip(set: &RuleSet) -> (tempfile::TempDir, RuleSet) {
        let out = serialize_ruleset(set);
        let dir = tempfile::tempdir().unwrap();
        let schemas = dir.path().join("schemas");
        std::fs::create_dir_all(&schemas).unwrap();
        for f in &out.schema_files {
            std::fs::write(schemas.join(&f.name), &f.content).unwrap();
        }
        std::fs::write(dir.path().join("rules.toml"), &out.rules_toml).unwrap();
        let reloaded = RuleSet::load(dir.path()).expect("serialized rules.toml must reload");
        (dir, reloaded)
    }

    /// Field-wise equality EXCLUDING `SchemaSource.reference`/`path` (which
    /// necessarily differ between an in-code placeholder and an on-disk file).
    fn assert_rules_equivalent(original: &Rule, reloaded: &Rule) {
        assert_eq!(original.name, reloaded.name, "name");
        assert_eq!(original.when, reloaded.when, "selector ({})", original.name);
        assert_eq!(
            original.severity, reloaded.severity,
            "severity ({})",
            original.name
        );
        assert_eq!(
            original.enforce, reloaded.enforce,
            "enforce ({})",
            original.name
        );
        assert_eq!(original.scope, reloaded.scope, "scope ({})", original.name);
        match (&original.assert, &reloaded.assert) {
            (Assertion::JsonSchema(a), Assertion::JsonSchema(b)) => {
                // Compare the parsed schema VALUE only; reference/path differ.
                assert_eq!(a.schema, b.schema, "schema value ({})", original.name);
            }
            (a, b) => assert_eq!(a, b, "assertion ({})", original.name),
        }
    }

    fn assert_round_trips(set: &RuleSet) {
        let (_dir, reloaded) = round_trip(set);
        assert_eq!(
            set.rules.len(),
            reloaded.rules.len(),
            "rule count must be preserved"
        );
        for (orig, back) in set.rules.iter().zip(reloaded.rules.iter()) {
            assert_rules_equivalent(orig, back);
        }
        // No duplicate names survived serialization.
        let mut names: Vec<&str> = reloaded.rules.iter().map(|r| r.name.as_str()).collect();
        let count = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), count, "no duplicate names after reload");
    }

    #[test]
    fn test_round_trip_full_default_ruleset() {
        // A registry exercising every default rule kind (json-schema, shorthand,
        // graph) the fixed default emits.
        let reg = registry(vec![
            ("type", LabelNamespace::new("Type", true)),
            ("milestone", LabelNamespace::new("Release", false)),
        ]);
        let set = default_ruleset(&reg);
        // Sanity: the set covers json-schema, shorthand, and graph kinds.
        assert!(set
            .rules
            .iter()
            .any(|r| matches!(r.assert, Assertion::JsonSchema(_))));
        assert!(set
            .rules
            .iter()
            .any(|r| matches!(r.assert, Assertion::RequireLabel { .. })));
        assert!(set
            .rules
            .iter()
            .any(|r| matches!(r.assert, Assertion::TypeHierarchy { .. })));
        assert_round_trips(&set);
    }

    #[test]
    fn test_round_trip_canonical_label_format_specifically() {
        // The always-on canonical label-format rule is the highest-traffic
        // JsonSchema rule and most exposed to the reference/path exclusion.
        let set = default_ruleset(&registry(vec![]));
        let canonical: Vec<&Rule> = set
            .rules
            .iter()
            .filter(|r| r.name == "default:label-format")
            .collect();
        assert_eq!(canonical.len(), 1);
        assert_round_trips(&set);
        // The schema file is materialized.
        let out = serialize_ruleset(&set);
        assert!(out
            .schema_files
            .iter()
            .any(|f| f.name == "default-label-format.json"));
    }

    #[test]
    fn test_round_trip_severity_off_rule() {
        // An `off` rule must serialize and reload with severity preserved.
        let toml = r#"
[[rules]]
name = "muted"
severity = "off"
assert = { require-section = { heading = "Goals" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        assert_eq!(set.rules[0].severity, Severity::Off);
        assert_round_trips(&set);
    }

    #[test]
    fn test_round_trip_graph_config_kinds() {
        // The three graph config-table kinds round-trip their config tables.
        let toml = r#"
[[rules]]
name = "coverage"
when = { type = "epic" }
severity = "error"
assert = { label-coverage = { criteria-section = "success_criteria", marker = "[hard]", child-state = "done" } }

[[rules]]
name = "reference"
severity = "warn"
assert = { label-reference = { from = "satisfies", to = "req", scope = "linked" } }

[[rules]]
name = "shape"
when = { type = "task" }
severity = "error"
assert = { dependency-shape = { target = { type = "design" }, mode = "must", transitive = true } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        assert_round_trips(&set);
    }

    #[test]
    fn test_round_trip_regex_backslashes_preserved() {
        // A label-value-pattern with backslashes must survive via a literal string.
        let toml = r#"
[[rules]]
name = "marker"
assert = { label-value-pattern = { namespace = "sc", regex = '^\[hard\]\s+\w+' } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        let out = serialize_ruleset(&set);
        // Rendered as a literal string (single quotes), backslashes intact.
        assert!(out.rules_toml.contains(r"regex = '^\[hard\]\s+\w+'"));
        assert_round_trips(&set);
    }

    #[test]
    fn test_round_trip_empty_ruleset() {
        let set = RuleSet::empty();
        let (_dir, reloaded) = round_trip(&set);
        assert!(reloaded.rules.is_empty());
    }

    #[test]
    fn test_round_trip_state_predicate_single_and_list() {
        // A single-state selector serializes as a string and a multi-state
        // selector as a TOML array; both must reload to the same predicate.
        let toml = r#"
[[rules]]
name = "single-state"
when = { type = "epic", state = "in_progress" }
assert = { require-section = { heading = "Plan" } }

[[rules]]
name = "list-state"
when = { state = ["ready", "in_progress", "gated"] }
assert = { require-section = { heading = "Plan" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        let out = serialize_ruleset(&set);
        assert!(out.rules_toml.contains(r#"state = "in_progress""#));
        assert!(out
            .rules_toml
            .contains(r#"state = ["ready", "in_progress", "gated"]"#));
        assert_round_trips(&set);
    }

    #[test]
    fn test_empty_selector_is_omitted() {
        // A rule with no selector must NOT emit `when = {}` (which the loader
        // accepts, but the canonical form omits it).
        let set = default_ruleset(&registry(vec![]));
        let out = serialize_ruleset(&set);
        assert!(!out.rules_toml.contains("when = {}"));
        assert!(!out.rules_toml.contains("when = { }"));
    }

    #[test]
    fn test_colliding_schema_rule_names_get_unique_files() {
        // Two DISTINCT rule names that sanitize to the SAME stem
        // (`a:b` and `a/b` both -> `a-b`) must NOT produce the same schema
        // file name, or one schema would silently overwrite the other and a
        // rule would validate against the wrong schema (finding #1).
        let toml = r#"
[[rules]]
name = "a:b"
severity = "error"
assert = { json-schema = "schemas/first.json" }

[[rules]]
name = "a/b"
severity = "error"
assert = { json-schema = "schemas/second.json" }
"#;
        let dir = tempfile::tempdir().unwrap();
        let schemas = dir.path().join("schemas");
        std::fs::create_dir_all(&schemas).unwrap();
        // Two DIFFERENT schema contents so an overwrite would be observable.
        std::fs::write(schemas.join("first.json"), "{\"const\": 1}\n").unwrap();
        std::fs::write(schemas.join("second.json"), "{\"const\": 2}\n").unwrap();
        std::fs::write(dir.path().join("rules.toml"), toml).unwrap();
        let set = RuleSet::load(dir.path()).unwrap();

        let out = serialize_ruleset(&set);
        // Both rules carry a JsonSchema, so two schema files are emitted.
        assert_eq!(out.schema_files.len(), 2, "two json-schema rules");
        // The file names must be UNIQUE despite the sanitized-name collision.
        let names: HashSet<&str> = out.schema_files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names.len(),
            2,
            "colliding sanitized names must yield distinct schema files, got {:?}",
            out.schema_files.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        // And the two distinct schema VALUES are both preserved (no overwrite).
        let contents: HashSet<&str> = out
            .schema_files
            .iter()
            .map(|f| f.content.as_str())
            .collect();
        assert_eq!(contents.len(), 2, "both schema values must survive");

        // The whole thing must round-trip: reload and re-serialize stable.
        assert_round_trips(&set);
    }

    #[test]
    fn test_serialized_file_is_canonical_idempotent() {
        // Serializing, reloading, and re-serializing yields identical text once
        // schema references are stable (file-name driven). The second pass uses
        // the reloaded set (whose JsonSchema references now point at files).
        let set = default_ruleset(&registry(vec![]));
        let (_dir, reloaded) = round_trip(&set);
        let first = serialize_ruleset(&set).rules_toml;
        let second = serialize_ruleset(&reloaded).rules_toml;
        assert_eq!(first, second, "serialization must be canonical/stable");
    }
}
