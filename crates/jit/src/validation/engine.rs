//! JSON Schema evaluation core: compile a rule's schema once, validate a
//! projection against it, and report a [`Finding`] per failure.
//!
//! This is the engine's evaluation primitive (DR §5). It does exactly one thing:
//! given a [`Rule`] carrying a JSON Schema and a [`Projection`] serialized to a
//! [`serde_json::Value`], it returns one [`Finding`] for every schema violation,
//! carrying the rule's name and severity into each finding.
//!
//! # Draft pinning and format handling
//!
//! Validators are built deterministically with
//! `jsonschema::options().with_draft(Draft::Draft202012).build(&schema)` (DR
//! §5.1). Format validation is left at the 2020-12 default (annotation-only), so
//! `format` keywords never cause failures here.
//!
//! # Caching (DR §5.2)
//!
//! Compiling a schema is the documented perf pitfall; validating is cheap. The
//! compiled [`jsonschema::Validator`] (which is `Clone + Send + Sync`) is
//! therefore compiled at most ONCE per distinct schema and cached behind an
//! [`Arc`]. The cache is keyed by the **schema's identity** (a stable hash of the
//! schema's canonical serialized form), never by rule name. This makes validator
//! aliasing impossible regardless of how a [`Rule`] is constructed: two rules
//! that share the same schema correctly reuse one compiled validator (so the same
//! rule never recompiles), while two rules carrying different schemas NEVER share
//! a validator — even if they happen to have identical names. See
//! [`SchemaEngine::validator_for`] and the `test_validator_is_cached_not_recompiled`
//! and `test_same_name_different_schema_does_not_alias` unit tests.
//!
//! # Custom keywords are out of scope
//!
//! This core does NOT implement `x-jit-*` custom keywords (a separate task,
//! 33f23ec7). Unknown keywords are treated as annotations — the `jsonschema`
//! 0.46 default — so schemas that use them validate without error here. No
//! strict-keyword rejection is added.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use jsonschema::{Draft, Validator};
use thiserror::Error;

use crate::validation::rules::{Rule, Severity};

/// A single validation failure, ready to surface to a user or machine.
///
/// One [`Finding`] is produced per schema violation. The `rule` name and
/// `severity` are copied from the originating [`Rule`] so downstream consumers
/// (local-eval, graph, `jit validate`) can group, sort, and gate on findings
/// without re-consulting the rule set.
///
/// # Examples
///
/// ```
/// use jit::validation::engine::Finding;
/// use jit::validation::rules::Severity;
///
/// let finding = Finding {
///     rule: "epic-has-success-criteria".to_string(),
///     severity: Severity::Error,
///     message: "missing required property 'sections'".to_string(),
/// };
/// assert_eq!(finding.rule, "epic-has-success-criteria");
/// assert_eq!(finding.severity, Severity::Error);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Name of the rule that produced this finding.
    pub rule: String,
    /// Severity carried over from the rule that produced this finding.
    pub severity: Severity,
    /// Human-readable description of the violation (from the validator).
    pub message: String,
}

/// Error raised when a rule's JSON Schema fails to compile.
///
/// Compilation errors are never swallowed: a malformed schema surfaces here as a
/// `Result::Err` rather than silently producing zero findings.
///
/// # Examples
///
/// ```
/// use jit::validation::engine::{SchemaEngine, SchemaCompileError};
/// use jit::validation::rules::RuleSet;
/// use std::path::Path;
///
/// // A schema with an invalid `type` value cannot be compiled.
/// let dir = tempfile::tempdir().unwrap();
/// let schemas = dir.path().join("schemas");
/// std::fs::create_dir_all(&schemas).unwrap();
/// std::fs::write(schemas.join("bad.json"), r#"{ "type": "not-a-type" }"#).unwrap();
/// let toml = r#"
/// [[rules]]
/// name = "bad-schema"
/// assert = { json-schema = "schemas/bad.json" }
/// "#;
/// let set = RuleSet::from_toml_str(toml, dir.path()).unwrap();
/// let engine = SchemaEngine::new();
/// let projection = serde_json::json!({});
/// let result = engine.validate(&set.rules[0], &projection);
/// assert!(matches!(result, Err(SchemaCompileError { .. })));
/// ```
#[derive(Debug, Error)]
#[error("rule '{rule}': failed to compile JSON Schema: {message}")]
pub struct SchemaCompileError {
    /// Name of the rule whose schema failed to compile.
    pub rule: String,
    /// Human-readable description of the compilation failure.
    pub message: String,
}

/// Compiles and caches JSON Schema validators, then evaluates projections.
///
/// The engine owns a cache of compiled [`jsonschema::Validator`]s keyed by
/// **schema identity** — a stable hash of each schema's canonical serialized
/// form — rather than by rule name. Compilation happens lazily on first use of a
/// given schema and the resulting validator is reused on every subsequent call
/// for that same schema, so the documented per-write recompilation pitfall (DR
/// §5.2) is avoided. Because the key is the schema itself, two distinct schemas
/// can never share a validator even if their rules have identical names, so no
/// validator aliasing is possible. Reuse the same engine instance across all
/// evaluations within a command for the cache to be effective.
///
/// # Examples
///
/// ```
/// use jit::validation::engine::SchemaEngine;
/// use jit::validation::rules::RuleSet;
///
/// // A schema requiring a `state` property; a projection missing it fails.
/// let dir = tempfile::tempdir().unwrap();
/// let schemas = dir.path().join("schemas");
/// std::fs::create_dir_all(&schemas).unwrap();
/// std::fs::write(
///     schemas.join("needs-state.json"),
///     r#"{ "type": "object", "required": ["state"] }"#,
/// )
/// .unwrap();
/// let toml = r#"
/// [[rules]]
/// name = "needs-state"
/// severity = "error"
/// assert = { json-schema = "schemas/needs-state.json" }
/// "#;
/// let set = RuleSet::from_toml_str(toml, dir.path()).unwrap();
///
/// let engine = SchemaEngine::new();
/// let bad = serde_json::json!({ "priority": "high" });
/// let findings = engine.validate(&set.rules[0], &bad).unwrap();
/// assert_eq!(findings.len(), 1);
/// assert_eq!(findings[0].rule, "needs-state");
///
/// let ok = serde_json::json!({ "state": "ready" });
/// assert!(engine.validate(&set.rules[0], &ok).unwrap().is_empty());
/// ```
#[derive(Debug, Default)]
pub struct SchemaEngine {
    /// Compiled validators keyed by schema identity (a stable hash of the
    /// schema's canonical serialized form). Keying by the schema rather than the
    /// rule name makes validator aliasing impossible. Interior mutability lets
    /// evaluation populate the cache behind a shared `&self`.
    cache: RefCell<HashMap<u64, Arc<Validator>>>,
}

impl SchemaEngine {
    /// Create an engine with an empty validator cache.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::engine::SchemaEngine;
    ///
    /// let engine = SchemaEngine::new();
    /// // A fresh engine has compiled nothing yet.
    /// assert!(engine.is_empty());
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns whether the validator cache is currently empty.
    ///
    /// Primarily useful for tests asserting lazy compilation behavior.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::engine::SchemaEngine;
    ///
    /// assert!(SchemaEngine::new().is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.cache.borrow().is_empty()
    }

    /// Validate a serialized [`Projection`](crate::domain::Projection) against a
    /// rule's JSON Schema, returning one [`Finding`] per violation.
    ///
    /// The rule's compiled validator is fetched from (or inserted into) the cache
    /// via [`SchemaEngine::validator_for`]. Each schema error becomes a
    /// [`Finding`] carrying the rule's name and severity. A schema that fails to
    /// compile surfaces as [`SchemaCompileError`]; a valid projection yields an
    /// empty `Vec`.
    ///
    /// Only the schema embedded in a [`Rule::assert`](crate::validation::rules::Rule)
    /// of kind [`Assertion::JsonSchema`](crate::validation::rules::Assertion::JsonSchema)
    /// is evaluated. Shorthand and graph assertion kinds carry no schema here and
    /// yield no findings; they are evaluated by their own downstream tasks.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::engine::SchemaEngine;
    /// use jit::validation::rules::RuleSet;
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let schemas = dir.path().join("schemas");
    /// std::fs::create_dir_all(&schemas).unwrap();
    /// std::fs::write(
    ///     schemas.join("string-state.json"),
    ///     r#"{ "type": "object", "properties": { "state": { "type": "string" } } }"#,
    /// )
    /// .unwrap();
    /// let toml = r#"
    /// [[rules]]
    /// name = "state-is-string"
    /// assert = { json-schema = "schemas/string-state.json" }
    /// "#;
    /// let set = RuleSet::from_toml_str(toml, dir.path()).unwrap();
    /// let engine = SchemaEngine::new();
    ///
    /// // `state` as a number violates the schema.
    /// let bad = serde_json::json!({ "state": 7 });
    /// let findings = engine.validate(&set.rules[0], &bad).unwrap();
    /// assert_eq!(findings.len(), 1);
    /// ```
    pub fn validate(
        &self,
        rule: &Rule,
        projection: &serde_json::Value,
    ) -> Result<Vec<Finding>, SchemaCompileError> {
        let schema = match rule_schema(rule) {
            Some(schema) => schema,
            // No JSON Schema to evaluate (shorthand/graph kind): no findings.
            None => return Ok(Vec::new()),
        };

        // Key the cache by the schema's identity, NOT the rule name, so two rules
        // sharing a name but carrying different schemas can never alias onto one
        // compiled validator.
        let key = schema_key(schema);
        let validator = self.validator_for(key, &rule.name, schema)?;

        let findings = validator
            .iter_errors(projection)
            .map(|error| Finding {
                rule: rule.name.clone(),
                severity: rule.severity,
                message: error.to_string(),
            })
            .collect();

        Ok(findings)
    }

    /// Return the compiled validator for the schema identified by `schema_key`,
    /// compiling `schema` and caching it on first request and reusing the cached
    /// [`Arc`] thereafter.
    ///
    /// This is the caching primitive. The cache is keyed by `schema_key` — a
    /// stable hash of the schema's canonical form, obtained from
    /// [`schema_key`](crate::validation::engine::schema_key) — so the returned
    /// `Arc<Validator>` is pointer-identical across calls for the **same schema**,
    /// and the schema is compiled at most once (DR §5.2). The validator is built
    /// with the 2020-12 draft pinned explicitly. `rule_name` is used only to
    /// attribute a [`SchemaCompileError`] to a rule; it does NOT affect caching,
    /// so two rules sharing a name but different schemas never alias.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::engine::{schema_key, SchemaEngine};
    ///
    /// let engine = SchemaEngine::new();
    /// let schema = serde_json::json!({ "type": "object" });
    /// let key = schema_key(&schema);
    /// let first = engine.validator_for(key, "r", &schema).unwrap();
    /// let second = engine.validator_for(key, "r", &schema).unwrap();
    /// // Same schema => same cached validator (no recompilation).
    /// assert!(std::sync::Arc::ptr_eq(&first, &second));
    /// ```
    pub fn validator_for(
        &self,
        schema_key: u64,
        rule_name: &str,
        schema: &serde_json::Value,
    ) -> Result<Arc<Validator>, SchemaCompileError> {
        if let Some(cached) = self.cache.borrow().get(&schema_key) {
            return Ok(Arc::clone(cached));
        }

        let validator = jsonschema::options()
            .with_draft(Draft::Draft202012)
            .build(schema)
            .map_err(|error| SchemaCompileError {
                rule: rule_name.to_string(),
                message: error.to_string(),
            })?;
        let validator = Arc::new(validator);

        self.cache
            .borrow_mut()
            .insert(schema_key, Arc::clone(&validator));
        Ok(validator)
    }
}

/// Compute a stable cache key from a JSON Schema's identity.
///
/// The key is a `u64` hash of the schema's canonical serialized form
/// (`serde_json::to_string`, which emits object keys in insertion order — stable
/// for a given parsed [`serde_json::Value`]). Two schemas that serialize to the
/// same string share a key (and therefore a compiled validator); any difference
/// in content yields a different key, so distinct schemas never collide on the
/// cache regardless of the rule names that carry them.
///
/// # Examples
///
/// ```
/// use jit::validation::engine::schema_key;
///
/// let a = serde_json::json!({ "type": "object" });
/// let b = serde_json::json!({ "type": "object" });
/// let c = serde_json::json!({ "type": "array" });
/// // Equal schemas hash equal; different schemas hash differently.
/// assert_eq!(schema_key(&a), schema_key(&b));
/// assert_ne!(schema_key(&a), schema_key(&c));
/// ```
pub fn schema_key(schema: &serde_json::Value) -> u64 {
    let canonical = serde_json::to_string(schema).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canonical.hash(&mut hasher);
    hasher.finish()
}

/// Extract the JSON Schema a rule validates against, if it carries one.
///
/// Only [`Assertion::JsonSchema`](crate::validation::rules::Assertion::JsonSchema)
/// carries a raw schema at this layer; shorthand kinds desugar to schemas in a
/// downstream task, and graph kinds have no schema at all.
fn rule_schema(rule: &Rule) -> Option<&serde_json::Value> {
    use crate::validation::rules::Assertion;
    match &rule.assert {
        Assertion::JsonSchema(source) => Some(&source.schema),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::rules::RuleSet;
    use std::path::Path;
    use tempfile::TempDir;

    /// Build a `RuleSet` from a single `json-schema` rule whose schema file holds
    /// `schema_json`. Returns the temp dir (kept alive) and the set.
    fn rule_set_with_schema(name: &str, severity: &str, schema_json: &str) -> (TempDir, RuleSet) {
        let dir = tempfile::tempdir().unwrap();
        let schemas = dir.path().join("schemas");
        std::fs::create_dir_all(&schemas).unwrap();
        std::fs::write(schemas.join("s.json"), schema_json).unwrap();
        let toml = format!(
            "[[rules]]\nname = \"{name}\"\nseverity = \"{severity}\"\nassert = {{ json-schema = \"schemas/s.json\" }}\n"
        );
        let set = RuleSet::from_toml_str(&toml, dir.path()).unwrap();
        (dir, set)
    }

    #[test]
    fn test_validate_passing_projection_yields_no_findings() {
        let (_dir, set) = rule_set_with_schema(
            "needs-state",
            "error",
            r#"{ "type": "object", "required": ["state"] }"#,
        );
        let engine = SchemaEngine::new();
        let projection = serde_json::json!({ "state": "ready" });
        let findings = engine.validate(&set.rules[0], &projection).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn test_validate_failing_projection_carries_rule_and_severity() {
        let (_dir, set) = rule_set_with_schema(
            "needs-state",
            "error",
            r#"{ "type": "object", "required": ["state"] }"#,
        );
        let engine = SchemaEngine::new();
        let projection = serde_json::json!({ "priority": "high" });
        let findings = engine.validate(&set.rules[0], &projection).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "needs-state");
        assert_eq!(findings[0].severity, Severity::Error);
        assert!(!findings[0].message.is_empty());
    }

    #[test]
    fn test_severity_warn_is_carried_into_finding() {
        let (_dir, set) = rule_set_with_schema(
            "needs-state",
            "warn",
            r#"{ "type": "object", "required": ["state"] }"#,
        );
        let engine = SchemaEngine::new();
        let projection = serde_json::json!({});
        let findings = engine.validate(&set.rules[0], &projection).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warn);
    }

    #[test]
    fn test_multiple_violations_yield_multiple_findings() {
        // Two required properties both absent => two findings.
        let (_dir, set) = rule_set_with_schema(
            "needs-both",
            "error",
            r#"{ "type": "object", "required": ["state", "priority"] }"#,
        );
        let engine = SchemaEngine::new();
        let projection = serde_json::json!({});
        let findings = engine.validate(&set.rules[0], &projection).unwrap();
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().all(|f| f.rule == "needs-both"));
    }

    #[test]
    fn test_non_schema_assertion_yields_no_findings() {
        // A shorthand rule carries no schema; the engine returns no findings
        // rather than erroring.
        let toml = r#"
[[rules]]
name = "shorthand"
assert = { require-label = { label = "type:*" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        let engine = SchemaEngine::new();
        let findings = engine
            .validate(&set.rules[0], &serde_json::json!({}))
            .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn test_invalid_schema_surfaces_compile_error() {
        let (_dir, set) = rule_set_with_schema("bad", "error", r#"{ "type": "not-a-real-type" }"#);
        let engine = SchemaEngine::new();
        let result = engine.validate(&set.rules[0], &serde_json::json!({}));
        match result {
            Err(SchemaCompileError { rule, .. }) => assert_eq!(rule, "bad"),
            Ok(_) => panic!("expected a compile error for an invalid schema"),
        }
    }

    #[test]
    fn test_validator_is_cached_not_recompiled() {
        // The compiled validator for a rule must be compiled at most once: the
        // cached Arc is pointer-identical across repeated lookups, and the cache
        // holds exactly one entry no matter how many times we validate.
        let (_dir, set) = rule_set_with_schema(
            "cached",
            "error",
            r#"{ "type": "object", "required": ["state"] }"#,
        );
        let engine = SchemaEngine::new();
        let schema = match &set.rules[0].assert {
            crate::validation::rules::Assertion::JsonSchema(s) => &s.schema,
            other => panic!("expected JsonSchema, got {other:?}"),
        };

        assert!(engine.is_empty(), "engine starts with an empty cache");

        let key = schema_key(schema);
        let first = engine.validator_for(key, "cached", schema).unwrap();
        let second = engine.validator_for(key, "cached", schema).unwrap();
        // Pointer identity proves the same compiled validator was reused.
        assert!(
            Arc::ptr_eq(&first, &second),
            "validator must be cached, not recompiled"
        );

        // Repeated full validations do not add cache entries or recompile.
        let projection = serde_json::json!({ "state": "ready" });
        for _ in 0..5 {
            engine.validate(&set.rules[0], &projection).unwrap();
        }
        assert_eq!(
            engine.cache.borrow().len(),
            1,
            "exactly one validator should ever be compiled for one rule"
        );
        let after = engine.validator_for(key, "cached", schema).unwrap();
        assert!(
            Arc::ptr_eq(&first, &after),
            "validator must remain the same instance after many validations"
        );
    }

    #[test]
    fn test_distinct_rules_get_distinct_validators() {
        // Two rules carrying *different* schemas must each compile their own
        // validator, so the cache ends with two entries.
        let dir = tempfile::tempdir().unwrap();
        let schemas = dir.path().join("schemas");
        std::fs::create_dir_all(&schemas).unwrap();
        std::fs::write(schemas.join("a.json"), r#"{ "type": "object" }"#).unwrap();
        std::fs::write(schemas.join("b.json"), r#"{ "type": "array" }"#).unwrap();
        let toml = r#"
[[rules]]
name = "rule-a"
assert = { json-schema = "schemas/a.json" }

[[rules]]
name = "rule-b"
assert = { json-schema = "schemas/b.json" }
"#;
        let set = RuleSet::from_toml_str(toml, dir.path()).unwrap();
        let engine = SchemaEngine::new();
        let projection = serde_json::json!({});
        engine.validate(&set.rules[0], &projection).unwrap();
        engine.validate(&set.rules[1], &projection).unwrap();
        assert_eq!(engine.cache.borrow().len(), 2);
    }

    #[test]
    fn test_distinct_rules_with_same_schema_share_one_validator() {
        // Two distinct rules carrying the *same* schema must share a single
        // compiled validator: keying by schema identity collapses them to one
        // cache entry (still satisfying "no per-call recompile").
        let dir = tempfile::tempdir().unwrap();
        let schemas = dir.path().join("schemas");
        std::fs::create_dir_all(&schemas).unwrap();
        std::fs::write(schemas.join("a.json"), r#"{ "type": "object" }"#).unwrap();
        std::fs::write(schemas.join("b.json"), r#"{ "type": "object" }"#).unwrap();
        let toml = r#"
[[rules]]
name = "rule-a"
assert = { json-schema = "schemas/a.json" }

[[rules]]
name = "rule-b"
assert = { json-schema = "schemas/b.json" }
"#;
        let set = RuleSet::from_toml_str(toml, dir.path()).unwrap();
        let engine = SchemaEngine::new();
        let projection = serde_json::json!({});
        engine.validate(&set.rules[0], &projection).unwrap();
        engine.validate(&set.rules[1], &projection).unwrap();
        assert_eq!(
            engine.cache.borrow().len(),
            1,
            "identical schemas must compile exactly one validator"
        );
    }

    #[test]
    fn test_same_name_different_schema_does_not_alias() {
        // Regression for the validator-aliasing bug reachable via the public API:
        // construct TWO Rules with the SAME `name` but DIFFERENT JSON schemas
        // DIRECTLY through the public Rule/RuleSet/Assertion::JsonSchema API,
        // bypassing the TOML loader (which would reject the duplicate name). Each
        // rule must validate against ITS OWN schema — no validator reuse across
        // the two schemas.
        use crate::validation::rules::{Assertion, Rule, SchemaSource, Selector};
        use std::path::PathBuf;

        // Rule one requires property "alpha"; rule two requires property "beta".
        // Both rules deliberately share the name "dup".
        let rule_alpha = Rule {
            name: "dup".to_string(),
            when: Selector::default(),
            severity: Severity::Error,
            enforce: false,
            assert: Assertion::JsonSchema(SchemaSource {
                reference: "inline".to_string(),
                path: PathBuf::from("inline"),
                schema: serde_json::json!({ "type": "object", "required": ["alpha"] }),
            }),
            scope: crate::validation::rules::Scope::Local,
        };
        let rule_beta = Rule {
            name: "dup".to_string(),
            when: Selector::default(),
            severity: Severity::Error,
            enforce: false,
            assert: Assertion::JsonSchema(SchemaSource {
                reference: "inline".to_string(),
                path: PathBuf::from("inline"),
                schema: serde_json::json!({ "type": "object", "required": ["beta"] }),
            }),
            scope: crate::validation::rules::Scope::Local,
        };
        let set = RuleSet {
            rules: vec![rule_alpha, rule_beta],
        };

        let engine = SchemaEngine::new();

        // A projection that has "alpha" but not "beta": rule one passes, rule two
        // fails. If the cache aliased on name, rule two would reuse rule one's
        // validator and (wrongly) pass.
        let has_alpha = serde_json::json!({ "alpha": 1 });
        let alpha_findings = engine.validate(&set.rules[0], &has_alpha).unwrap();
        assert!(
            alpha_findings.is_empty(),
            "rule with schema requiring 'alpha' must pass when 'alpha' is present, got {alpha_findings:?}"
        );
        let beta_findings = engine.validate(&set.rules[1], &has_alpha).unwrap();
        assert_eq!(
            beta_findings.len(),
            1,
            "rule with schema requiring 'beta' must FAIL when only 'alpha' is present (no aliasing), got {beta_findings:?}"
        );

        // Symmetric check: a projection with "beta" but not "alpha".
        let has_beta = serde_json::json!({ "beta": 1 });
        let alpha_findings2 = engine.validate(&set.rules[0], &has_beta).unwrap();
        assert_eq!(
            alpha_findings2.len(),
            1,
            "rule requiring 'alpha' must FAIL when only 'beta' is present (no aliasing), got {alpha_findings2:?}"
        );
        let beta_findings2 = engine.validate(&set.rules[1], &has_beta).unwrap();
        assert!(
            beta_findings2.is_empty(),
            "rule requiring 'beta' must pass when 'beta' is present, got {beta_findings2:?}"
        );

        // Two distinct schemas => two distinct cache entries despite the shared name.
        assert_eq!(
            engine.cache.borrow().len(),
            2,
            "two different schemas must compile two validators even with identical rule names"
        );
    }

    // --- contains / minContains / maxContains under Draft 2020-12 ----------

    /// The `[hard]` criterion pattern: at least one `sections.success_criteria.items`
    /// entry must begin with `[hard]`. This exercises `contains` + `minContains`,
    /// which are Draft 2020-12 array keywords.
    fn hard_criterion_schema() -> &'static str {
        r#"{
  "type": "object",
  "properties": {
    "sections": {
      "type": "object",
      "properties": {
        "success_criteria": {
          "type": "object",
          "properties": {
            "items": {
              "type": "array",
              "contains": { "type": "string", "pattern": "^\\[hard\\]" },
              "minContains": 1
            }
          },
          "required": ["items"]
        }
      },
      "required": ["success_criteria"]
    }
  },
  "required": ["sections"]
}"#
    }

    #[test]
    fn test_contains_min_contains_passes_with_hard_item() {
        let (_dir, set) = rule_set_with_schema("hard-criteria", "error", hard_criterion_schema());
        let engine = SchemaEngine::new();
        let projection = serde_json::json!({
            "sections": {
                "success_criteria": {
                    "items": ["[aspirational] nice to have", "[hard] REQ-01 must hold"]
                }
            }
        });
        let findings = engine.validate(&set.rules[0], &projection).unwrap();
        assert!(
            findings.is_empty(),
            "a [hard] item satisfies minContains: 1, got {findings:?}"
        );
    }

    #[test]
    fn test_contains_min_contains_fails_without_hard_item() {
        let (_dir, set) = rule_set_with_schema("hard-criteria", "error", hard_criterion_schema());
        let engine = SchemaEngine::new();
        let projection = serde_json::json!({
            "sections": {
                "success_criteria": {
                    "items": ["[aspirational] nice to have", "[soft] maybe"]
                }
            }
        });
        let findings = engine.validate(&set.rules[0], &projection).unwrap();
        assert_eq!(
            findings.len(),
            1,
            "no [hard] item must violate contains/minContains, got {findings:?}"
        );
        assert_eq!(findings[0].rule, "hard-criteria");
    }

    #[test]
    fn test_max_contains_enforces_upper_bound() {
        // maxContains: 1 means at most one matching item; two [hard] items fail.
        let schema = r#"{
  "type": "object",
  "properties": {
    "items": {
      "type": "array",
      "contains": { "type": "string", "pattern": "^\\[hard\\]" },
      "minContains": 1,
      "maxContains": 1
    }
  },
  "required": ["items"]
}"#;
        let (_dir, set) = rule_set_with_schema("at-most-one-hard", "error", schema);
        let engine = SchemaEngine::new();

        let ok = serde_json::json!({ "items": ["[hard] one", "[soft] two"] });
        assert!(engine.validate(&set.rules[0], &ok).unwrap().is_empty());

        let too_many = serde_json::json!({ "items": ["[hard] one", "[hard] two"] });
        let findings = engine.validate(&set.rules[0], &too_many).unwrap();
        assert_eq!(findings.len(), 1, "two [hard] items exceed maxContains: 1");
    }

    #[test]
    fn test_unknown_keyword_is_treated_as_annotation() {
        // An `x-jit-*` style unknown keyword must NOT cause a compile error or a
        // finding: jsonschema 0.46 treats unknown keywords as annotations and the
        // core adds no strict-keyword rejection (custom keywords are task 33f23ec7).
        let schema = r#"{ "type": "object", "x-jit-custom": { "anything": true } }"#;
        let (_dir, set) = rule_set_with_schema("annotation", "warn", schema);
        let engine = SchemaEngine::new();
        let findings = engine
            .validate(&set.rules[0], &serde_json::json!({ "state": "ready" }))
            .unwrap();
        assert!(
            findings.is_empty(),
            "unknown keyword must not fail, got {findings:?}"
        );
    }
}
