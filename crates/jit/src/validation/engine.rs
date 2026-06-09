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
//! [`Arc`]. The cache is keyed by the **schema's identity** (the schema's
//! canonical serialized form itself, not a lossy hash), never by rule name. This makes validator
//! aliasing impossible regardless of how a [`Rule`] is constructed: two rules
//! that share the same schema correctly reuse one compiled validator (so the same
//! rule never recompiles), while two rules carrying different schemas NEVER share
//! a validator — even if they happen to have identical names. See
//! [`SchemaEngine::validator_for`] and the `test_validator_is_cached_not_recompiled`
//! and `test_same_name_different_schema_does_not_alias` unit tests.
//!
//! # Custom keywords (`x-jit-*`) extension point
//!
//! By default the engine registers NO custom keywords, so unknown `x-jit-*`
//! keywords are treated as annotations — the `jsonschema` 0.46 default — and
//! schemas using them validate without error (graceful degradation). To attach
//! domain-specific behavior, register a keyword via
//! [`SchemaEngine::with_keyword`] (task 33f23ec7); the supplied [`Keyword`]
//! factory is threaded into the `options()` builder on every compile. Keywords
//! are fixed for the engine instance, so the schema-identity cache key stays
//! correct. See the `test_degradation_x_jit_keyword_validates_under_standard_validator`
//! regression guard.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use jsonschema::error::ValidationErrorKind;
use jsonschema::paths::Location;
use jsonschema::{Draft, Keyword, ValidationError, Validator};
use thiserror::Error;

use crate::validation::rules::{Rule, Severity};

/// Factory closure that constructs a custom-keyword [`Keyword`] validator.
///
/// This mirrors the signature accepted by
/// [`jsonschema::ValidationOptions::with_keyword`]: given the parent schema
/// object, the keyword's own schema value, and its [`Location`], it returns a
/// boxed [`Keyword`] implementation (or a [`ValidationError`] if the keyword's
/// schema is itself malformed). Stored behind an [`Arc`] so the same factory can
/// be re-applied every time a schema is (re)compiled for a fresh validator.
///
/// Used to register `x-jit-*` custom keywords on a [`SchemaEngine`] via
/// [`SchemaEngine::with_keyword`].
///
/// # Examples
///
/// ```
/// use jit::validation::engine::KeywordFactory;
/// use jsonschema::{Keyword, ValidationError};
/// use std::sync::Arc;
///
/// struct AlwaysOk;
/// impl Keyword for AlwaysOk {
///     fn validate<'i>(&self, _i: &'i serde_json::Value) -> Result<(), ValidationError<'i>> {
///         Ok(())
///     }
///     fn is_valid(&self, _i: &serde_json::Value) -> bool {
///         true
///     }
/// }
///
/// // A factory matching the `KeywordFactory` alias.
/// let factory: Arc<KeywordFactory> =
///     Arc::new(|_parent, _schema, _location| Ok(Box::new(AlwaysOk) as Box<dyn Keyword>));
/// // It can be invoked like the closure it wraps.
/// let parent = serde_json::Map::new();
/// let schema = serde_json::json!(true);
/// let result = factory(&parent, &schema, jsonschema::paths::Location::new());
/// assert!(result.is_ok());
/// ```
pub type KeywordFactory = dyn for<'a> Fn(
        &'a serde_json::Map<String, serde_json::Value>,
        &'a serde_json::Value,
        Location,
    ) -> Result<Box<dyn Keyword>, ValidationError<'a>>
    + Send
    + Sync;

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
/// **schema identity** — each schema's canonical serialized form itself (not a
/// lossy hash) — rather than by rule name. Compilation happens lazily on first use of a
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
#[derive(Default)]
pub struct SchemaEngine {
    /// Compiled validators keyed by schema identity — the schema's canonical
    /// serialized form itself (not a lossy hash), so two distinct schemas can
    /// never collide onto one cached validator. Keying by the schema rather than
    /// the rule name makes validator aliasing impossible. Interior mutability
    /// lets evaluation populate the cache behind a shared `&self`.
    cache: RefCell<HashMap<String, Arc<Validator>>>,
    /// Custom `x-jit-*` keyword factories registered via
    /// [`SchemaEngine::with_keyword`], in registration order. These are FIXED for
    /// the lifetime of the engine instance and applied to every validator the
    /// engine compiles, so the schema-identity cache key remains correct: a given
    /// schema always compiles to the same validator under one engine. An empty
    /// vec (the default) means the engine behaves as a standard validator.
    keywords: Vec<(String, Arc<KeywordFactory>)>,
}

impl std::fmt::Debug for SchemaEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Keyword factories are closures and cannot be `Debug`; show their names.
        f.debug_struct("SchemaEngine")
            .field("cached_validators", &self.cache.borrow().len())
            .field(
                "keywords",
                &self.keywords.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            )
            .finish()
    }
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

    /// Register an `x-jit-*` custom keyword, returning the engine for chaining.
    ///
    /// This is the engine's extension point (DR §5.3, task 33f23ec7). The
    /// `factory` is threaded into the `jsonschema` `options()` builder via
    /// [`jsonschema::ValidationOptions::with_keyword`] every time the engine
    /// compiles a schema, so any schema referencing `name` is validated by the
    /// supplied [`Keyword`] implementation. Keywords are FIXED once the engine is
    /// built (this is a consuming builder), which keeps the schema-identity cache
    /// correct: under a given engine a schema always compiles to the same
    /// validator, so caching by schema alone remains valid.
    ///
    /// The default [`SchemaEngine::new`] registers no keywords and behaves as a
    /// standard validator; unknown `x-jit-*` keywords then degrade to annotations
    /// (see the degradation regression test).
    ///
    /// By convention `name` should start with `x-jit-`, but no naming check is
    /// imposed here — `jsonschema` accepts any keyword name.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::engine::SchemaEngine;
    /// use jit::validation::rules::RuleSet;
    /// use jsonschema::{Keyword, ValidationError};
    ///
    /// // A custom keyword: the annotated string must not be empty.
    /// struct NonEmpty;
    /// impl Keyword for NonEmpty {
    ///     fn validate<'i>(&self, i: &'i serde_json::Value) -> Result<(), ValidationError<'i>> {
    ///         if self.is_valid(i) {
    ///             Ok(())
    ///         } else {
    ///             Err(ValidationError::custom("string must not be empty"))
    ///         }
    ///     }
    ///     fn is_valid(&self, i: &serde_json::Value) -> bool {
    ///         i.as_str().is_none_or(|s| !s.is_empty())
    ///     }
    /// }
    ///
    /// let engine = SchemaEngine::new()
    ///     .with_keyword("x-jit-non-empty", |_p, _s, _l| Ok(Box::new(NonEmpty)));
    /// assert_eq!(engine.registered_keywords(), vec!["x-jit-non-empty".to_string()]);
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let schemas = dir.path().join("schemas");
    /// std::fs::create_dir_all(&schemas).unwrap();
    /// std::fs::write(
    ///     schemas.join("t.json"),
    ///     r#"{ "type": "object",
    ///          "properties": { "title": { "type": "string", "x-jit-non-empty": true } } }"#,
    /// )
    /// .unwrap();
    /// let toml = r#"
    /// [[rules]]
    /// name = "title-non-empty"
    /// assert = { json-schema = "schemas/t.json" }
    /// "#;
    /// let set = RuleSet::from_toml_str(toml, dir.path()).unwrap();
    ///
    /// // An empty title now violates the registered keyword.
    /// let bad = serde_json::json!({ "title": "" });
    /// assert_eq!(engine.validate(&set.rules[0], &bad).unwrap().len(), 1);
    /// // A non-empty title passes.
    /// let ok = serde_json::json!({ "title": "hello" });
    /// assert!(engine.validate(&set.rules[0], &ok).unwrap().is_empty());
    /// ```
    #[must_use]
    pub fn with_keyword<N, F>(mut self, name: N, factory: F) -> Self
    where
        N: Into<String>,
        F: for<'a> Fn(
                &'a serde_json::Map<String, serde_json::Value>,
                &'a serde_json::Value,
                Location,
            ) -> Result<Box<dyn Keyword>, ValidationError<'a>>
            + Send
            + Sync
            + 'static,
    {
        self.keywords.push((name.into(), Arc::new(factory)));
        // Registering a keyword changes how schemas compile, so any validators
        // already cached under the previous keyword set are now stale. Clear the
        // cache so subsequent validations recompile with the full keyword set —
        // this keeps a warmed engine correct, not just a freshly-built one.
        self.cache.borrow_mut().clear();
        self
    }

    /// Names of the custom keywords registered on this engine, in registration
    /// order.
    ///
    /// A standard engine ([`SchemaEngine::new`]) returns an empty `Vec`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::engine::SchemaEngine;
    ///
    /// assert!(SchemaEngine::new().registered_keywords().is_empty());
    /// ```
    pub fn registered_keywords(&self) -> Vec<String> {
        self.keywords.iter().map(|(name, _)| name.clone()).collect()
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
        let validator = self.validator_for(&key, &rule.name, schema)?;

        let findings = validator
            .iter_errors(projection)
            .map(|error| Finding {
                rule: rule.name.clone(),
                severity: rule.severity,
                message: render_finding_message(&error, schema, projection),
            })
            .collect();

        Ok(findings)
    }

    /// Return the compiled validator for the schema identified by `schema_key`,
    /// compiling `schema` and caching it on first request and reusing the cached
    /// [`Arc`] thereafter.
    ///
    /// This is the caching primitive. The cache is keyed by `schema_key` — the
    /// schema's canonical serialized form, obtained from
    /// [`schema_key`](crate::validation::engine::schema_key) — so the returned
    /// `Arc<Validator>` is pointer-identical across calls for the **same schema**,
    /// and the schema is compiled at most once (DR §5.2). Because the key is the
    /// full serialized schema (not a lossy hash), two distinct schemas can never
    /// collide onto one validator. The validator is built with the 2020-12 draft
    /// pinned explicitly. `rule_name` is used only to attribute a
    /// [`SchemaCompileError`] to a rule; it does NOT affect caching, so two rules
    /// sharing a name but different schemas never alias.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::engine::{schema_key, SchemaEngine};
    ///
    /// let engine = SchemaEngine::new();
    /// let schema = serde_json::json!({ "type": "object" });
    /// let key = schema_key(&schema);
    /// let first = engine.validator_for(&key, "r", &schema).unwrap();
    /// let second = engine.validator_for(&key, "r", &schema).unwrap();
    /// // Same schema => same cached validator (no recompilation).
    /// assert!(std::sync::Arc::ptr_eq(&first, &second));
    /// ```
    pub fn validator_for(
        &self,
        schema_key: &str,
        rule_name: &str,
        schema: &serde_json::Value,
    ) -> Result<Arc<Validator>, SchemaCompileError> {
        if let Some(cached) = self.cache.borrow().get(schema_key) {
            return Ok(Arc::clone(cached));
        }

        // Apply any registered `x-jit-*` custom keywords. Each factory is cloned
        // (cheap `Arc` clone) into an owned `'static` closure so every compiled
        // validator carries its own handle; the keyword set is fixed for the
        // engine, so this does not affect schema-identity caching. The builder
        // consumes and returns `self`, so it is folded over the keyword list.
        let options = self.keywords.iter().fold(
            jsonschema::options().with_draft(Draft::Draft202012),
            |options, (name, factory)| {
                let factory = Arc::clone(factory);
                options.with_keyword(name.clone(), move |parent, schema, location| {
                    factory(parent, schema, location)
                })
            },
        );
        let validator = options.build(schema).map_err(|error| SchemaCompileError {
            rule: rule_name.to_string(),
            message: error.to_string(),
        })?;
        let validator = Arc::new(validator);

        self.cache
            .borrow_mut()
            .insert(schema_key.to_string(), Arc::clone(&validator));
        Ok(validator)
    }
}

/// Compute a stable cache key from a JSON Schema's identity.
///
/// The key is the schema's canonical serialized form itself
/// (`serde_json::to_string`, which emits object keys in insertion order — stable
/// for a given parsed [`serde_json::Value`]). Returning the full serialized
/// schema rather than a hash means the key is a true identity: two schemas that
/// serialize identically share a key (and therefore a compiled validator), and
/// any difference in content yields a different key. There is no hash, so
/// distinct schemas can never collide on the cache regardless of the rule names
/// that carry them.
///
/// # Examples
///
/// ```
/// use jit::validation::engine::schema_key;
///
/// let a = serde_json::json!({ "type": "object" });
/// let b = serde_json::json!({ "type": "object" });
/// let c = serde_json::json!({ "type": "array" });
/// // Equal schemas share a key; different schemas have different keys.
/// assert_eq!(schema_key(&a), schema_key(&b));
/// assert_ne!(schema_key(&a), schema_key(&c));
/// ```
pub fn schema_key(schema: &serde_json::Value) -> String {
    serde_json::to_string(schema).unwrap_or_default()
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

/// Render a single [`ValidationError`] into an actionable [`Finding`] message
/// (CC-4).
///
/// This is the one renderer shared by [`SchemaEngine::validate`],
/// [`evaluate_local`](crate::validation::local)'s write-path `iter_errors` loop,
/// and the `jit validate` command path, so message quality is fixed in exactly
/// one place and cannot drift. It is pure: a deterministic function of the error,
/// the rule's `schema`, and the validated `projection`, with no I/O.
///
/// Five behaviors, in priority order:
///
/// 1. **Empty-projection special case.** A `minItems`/`contains` failure on an
///    empty `sections.<slug>.items` array (the prose-without-bullets case) emits
///    `section '<Heading>' has no list items; items must be Markdown bullets
///    (lines starting with '- ')`. The heading is recovered from the section
///    schema's `x-jit-section-heading` annotation, falling back to the slug.
/// 2. **Non-empty contains failure.** A `contains` failure on a non-empty
///    `sections.<slug>.items` array emits `section '<Heading>' items must include
///    at least one entry matching <humanized pattern>` when the `contains`
///    subschema carries a `pattern`. When no pattern is present, emits a generic
///    path-prefixed message naming the path without dumping the full array.
/// 3. **did-you-mean for a missing section.** A `required` failure naming an
///    absent section slug appends `did you mean '<Present Heading>'?` when a
///    present section heading is a near-miss (Levenshtein on slugified forms,
///    threshold `max(2, 20% of heading length)`).
/// 4. **Readable character classes.** Any `pattern`-keyword message is passed
///    through [`humanize_regex`] so control/zero-width characters render as
///    escapes (`\t`, `\u{200b}`) and `\s` survives intact.
/// 5. **Instance-path prefix.** Every other finding is prefixed with the
///    readable instance path, e.g. `at sections.success_criteria.items: <msg>`;
///    an empty path renders as the projection root (no prefix change).
pub fn render_finding_message(
    error: &ValidationError,
    schema: &serde_json::Value,
    projection: &serde_json::Value,
) -> String {
    let path = error.instance_path();

    // (1) Empty-section special case: an array keyword failed on an empty
    // `sections.<slug>.items` instance because the body had prose, not bullets.
    if let Some(slug) = empty_items_section_slug(error, path) {
        let heading = section_heading(schema, &slug).unwrap_or(slug);
        return format!(
            "section '{heading}' has no list items; items must be Markdown bullets \
             (lines starting with '- ')"
        );
    }

    // (2) Non-empty contains failure: the items array has entries but none satisfies
    // the `contains` subschema. The default rendering dumps the full array; instead
    // name the section heading (or path) and the required pattern.
    if matches!(error.kind(), ValidationErrorKind::Contains) {
        return render_contains_message(error, schema, path);
    }

    let readable = readable_instance_path(path);

    // (3) Missing required section: name the field and, when a present heading is
    // a near-miss of the required one, append a did-you-mean hint.
    if let ValidationErrorKind::Required { property } = error.kind() {
        if let Some(required) = property.as_str() {
            let base = with_path_prefix(&readable, &error.to_string());
            if let Some(suggestion) = did_you_mean_section(required, projection) {
                return format!("{base}; did you mean '{suggestion}'?");
            }
            return base;
        }
    }

    // (4) Pattern failures: render the regex readably before prefixing.
    if let ValidationErrorKind::Pattern { pattern } = error.kind() {
        let humanized = humanize_regex(pattern);
        let message = format!(
            "{} is not valid under the given pattern (expected to match {humanized})",
            error.instance()
        );
        return with_path_prefix(&readable, &message);
    }

    // (5) Default: prefix the readable instance path.
    with_path_prefix(&readable, &error.to_string())
}

/// If `error` is an array-shape failure (`minItems`/`contains`) on an *empty*
/// `sections.<slug>.items` instance, return the section slug. This is the
/// prose-without-bullets signature: the projection parsed the section but found
/// no list items, so its `items` array is `[]`.
fn empty_items_section_slug(error: &ValidationError, path: &Location) -> Option<String> {
    // Only the array-presence keywords produce the content-free messages we want
    // to replace; a `type` or `pattern` failure on items is a different problem.
    if !matches!(
        error.kind(),
        ValidationErrorKind::MinItems { .. } | ValidationErrorKind::Contains
    ) {
        return None;
    }
    // The instance that failed must be the empty array itself.
    match error.instance().as_ref() {
        serde_json::Value::Array(items) if items.is_empty() => {}
        _ => return None,
    }
    // Path shape: /sections/<slug>/items
    let segments: Vec<&str> = path.as_str().split('/').skip(1).collect();
    match segments.as_slice() {
        ["sections", slug, "items"] => Some(unescape_pointer_token(slug)),
        _ => None,
    }
}

/// Render an actionable message for a `contains`/`minContains` failure on a
/// *non-empty* array (the empty-array case is already handled upstream by
/// `empty_items_section_slug`).
///
/// When the failing `contains` subschema carries a `pattern`, the message reads:
/// `section '<Heading>' items must include at least one entry matching <pattern>`
/// (heading via the `x-jit-section-heading` annotation, falling back to the
/// slug or to the readable instance path). When no pattern is present, emits a
/// generic path-prefixed message that does not dump the full array.
///
/// The `contains` subschema is located by walking `schema` using the error's
/// `schema_path` (which ends with `/contains`). If the walk fails for any reason
/// (hand-written schema with an unexpected shape), the function falls back
/// gracefully to a generic path-prefixed message — it never panics.
fn render_contains_message(
    error: &ValidationError,
    schema: &serde_json::Value,
    path: &jsonschema::paths::Location,
) -> String {
    let readable = readable_instance_path(path);

    // Locate the `contains` subschema by following the error's schema_path.
    let contains_sub = contains_subschema_via_schema_path(error.schema_path(), schema);

    // Extract the pattern from the contains subschema, if present.
    let maybe_pattern = contains_sub
        .as_ref()
        .and_then(|sub| sub.get("pattern"))
        .and_then(serde_json::Value::as_str);

    // Try to name the section by slug (instance path: /sections/<slug>/items).
    let maybe_slug = {
        let segments: Vec<&str> = path.as_str().split('/').skip(1).collect();
        match segments.as_slice() {
            ["sections", slug, "items"] => Some(unescape_pointer_token(slug)),
            _ => None,
        }
    };

    match (maybe_slug, maybe_pattern) {
        (Some(slug), Some(pattern)) => {
            let heading = section_heading(schema, &slug).unwrap_or(slug);
            let humanized = humanize_regex(pattern);
            format!(
                "section '{heading}' items must include at least one entry matching {humanized}"
            )
        }
        (Some(slug), None) => {
            let heading = section_heading(schema, &slug).unwrap_or(slug);
            format!(
                "section '{heading}' items must include at least one entry \
                 satisfying the required shape"
            )
        }
        (None, Some(pattern)) => {
            let humanized = humanize_regex(pattern);
            with_path_prefix(
                &readable,
                &format!("items must include at least one entry matching {humanized}"),
            )
        }
        (None, None) => {
            // No slug, no pattern: generic message without the array dump.
            with_path_prefix(&readable, "items must include at least one matching entry")
        }
    }
}

/// Walk `schema` using the segments of `schema_path` to locate the `contains`
/// subschema and return a reference to it, or `None` if the walk fails.
///
/// # Schema-path variants
///
/// A `ValidationErrorKind::Contains` error can originate from two different
/// internal validators, each of which sets a distinct schema_path:
///
/// - **`ContainsValidator`** (plain `contains`, no `minContains`/`maxContains`):
///   schema_path ends with `contains`, e.g. `.../properties/items/contains`.
///   Walking all segments reaches the `contains` subschema directly.
///
/// - **`MinContainsValidator`** / **`MinMaxContainsValidator`** (`minContains: 1`
///   or both bounds): schema_path ends with `minContains` (or `maxContains`),
///   e.g. `.../properties/items/minContains`. The `contains` subschema lives one
///   level up as a sibling key `contains`. We strip the trailing `minContains` /
///   `maxContains`, walk to the parent, then fetch `contains` from it.
///
/// If any segment is not found, the schema node is not an object, or the
/// trailing segment is neither `contains` nor `minContains`/`maxContains`, we
/// return `None` so the caller falls back gracefully — no panics.
fn contains_subschema_via_schema_path<'s>(
    schema_path: &jsonschema::paths::Location,
    schema: &'s serde_json::Value,
) -> Option<&'s serde_json::Value> {
    let path_str = schema_path.as_str();
    // Segments are `/`-separated; skip the leading empty string from the leading `/`.
    let segments: Vec<&str> = path_str
        .split('/')
        .skip(1)
        .filter(|s| !s.is_empty())
        .collect();

    let last = segments.last().copied()?;

    if last == "contains" {
        // Walk all segments: the final node IS the contains subschema.
        walk_schema_path(schema, &segments)
    } else if last == "minContains" || last == "maxContains" {
        // Walk all-but-last to reach the parent (the array keyword object), then
        // get its `contains` sibling.
        let parent_segments = &segments[..segments.len() - 1];
        let parent = walk_schema_path(schema, parent_segments)?;
        parent.get("contains")
    } else {
        None
    }
}

/// Walk an object-only JSON schema node following `segments` in order, returning
/// a reference to the final node, or `None` if any segment is missing or the
/// current node is not an object.
fn walk_schema_path<'s>(
    schema: &'s serde_json::Value,
    segments: &[&str],
) -> Option<&'s serde_json::Value> {
    let mut node = schema;
    for segment in segments {
        match node {
            serde_json::Value::Object(map) => {
                node = map.get(*segment)?;
            }
            // A numeric segment (array index in a schema path) is not expected for
            // `contains` errors; treat as unrecognized shape and bail.
            _ => return None,
        }
    }
    Some(node)
}

/// Look up the original heading for a section `slug` via the
/// `x-jit-section-heading` annotation desugar emits on the section subschema.
///
/// Returns `None` when the schema carries no annotation for the slug (e.g. a
/// hand-written `json-schema` rule), in which case the caller falls back to the
/// slug itself — never a lossy de-slugify.
fn section_heading(schema: &serde_json::Value, slug: &str) -> Option<String> {
    schema
        .get("properties")?
        .get("sections")?
        .get("properties")?
        .get(slug)?
        .get("x-jit-section-heading")?
        .as_str()
        .map(str::to_string)
}

/// For a missing required section `slug`, find the present section heading whose
/// slug is the nearest near-miss and return its original heading text.
///
/// Compares the required slug against the slugs of the sections actually present
/// in `projection` (`sections.<slug>`) by Levenshtein distance, accepting the
/// nearest within `max(2, 20% of the required slug length)` edits. Returns the
/// present section's *heading* (recovered from the projection's stored heading,
/// falling back to the present slug) so the hint reads naturally.
fn did_you_mean_section(required_slug: &str, projection: &serde_json::Value) -> Option<String> {
    let sections = projection.get("sections")?.as_object()?;
    let threshold = (required_slug.chars().count() / 5).max(2);
    sections
        .iter()
        .filter(|(present, _)| present.as_str() != required_slug)
        .map(|(present, value)| {
            let distance = levenshtein(required_slug, present);
            (distance, present, value)
        })
        .filter(|(distance, _, _)| *distance <= threshold)
        .min_by_key(|(distance, _, _)| *distance)
        .map(|(_, present, value)| section_present_heading(value, present))
}

/// Recover a present section's display heading from its projected value
/// (`{"heading": "...", "items": [...]}`), falling back to the slug.
fn section_present_heading(value: &serde_json::Value, slug: &str) -> String {
    value
        .get("heading")
        .and_then(serde_json::Value::as_str)
        .map_or_else(|| slug.to_string(), str::to_string)
}

/// Prefix a message with its readable instance path, e.g.
/// `at sections.success_criteria.items: <message>`. An empty path (the
/// projection root) yields the message unchanged.
fn with_path_prefix(readable_path: &str, message: &str) -> String {
    if readable_path.is_empty() {
        message.to_string()
    } else {
        format!("at {readable_path}: {message}")
    }
}

/// Convert a JSON Pointer [`Location`] (`/sections/success_criteria/items`) into
/// the readable dotted path the projection uses (`sections.success_criteria.items`).
/// The empty (root) location renders as the empty string.
fn readable_instance_path(path: &Location) -> String {
    path.as_str()
        .split('/')
        .skip(1)
        .map(unescape_pointer_token)
        .collect::<Vec<_>>()
        .join(".")
}

/// Decode the two JSON Pointer escapes (`~1` -> `/`, `~0` -> `~`) in a single
/// pointer token so a section slug containing them renders literally.
fn unescape_pointer_token(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

/// Render a regex so control and zero-width characters become readable escapes,
/// while ordinary regex syntax (including `\s`, `\d`, character classes) is left
/// untouched (CC-4 point 3).
///
/// The fix targets the failure mode where a `pattern` keyword's source text
/// contains a literal tab or zero-width space, which `error.to_string()` would
/// emit raw into a message — invisible or confusing to an agent. Each `char` is
/// mapped: a literal tab/newline/carriage-return becomes `\t`/`\n`/`\r`, any
/// other control or zero-width character becomes a `\u{XXXX}` escape, and every
/// other character (letters, digits, backslash-escapes already in the source,
/// brackets) is passed through verbatim so `\s` stays `\s`.
///
/// # Examples
///
/// ```
/// use jit::validation::engine::humanize_regex;
///
/// // A backslash-s class is preserved verbatim.
/// assert_eq!(humanize_regex(r"\s+"), r"\s+");
/// // A literal tab in the source becomes a visible escape, never raw.
/// assert_eq!(humanize_regex("a\tb"), r"a\tb");
/// // A zero-width space becomes a unicode escape.
/// assert_eq!(humanize_regex("a\u{200b}b"), r"a\u{200b}b");
/// ```
pub fn humanize_regex(pattern: &str) -> String {
    pattern
        .chars()
        .map(|ch| match ch {
            '\t' => "\\t".to_string(),
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            // Other control characters and zero-width / formatting characters
            // are invisible when emitted raw; render them as a unicode escape.
            c if c.is_control() || is_zero_width(c) => format!("\\u{{{:04x}}}", c as u32),
            c => c.to_string(),
        })
        .collect()
}

/// Whether a character is zero-width or an invisible formatting character that
/// would render as nothing in a message (so it must be escaped to be readable).
fn is_zero_width(ch: char) -> bool {
    matches!(
        ch,
        '\u{200b}' // zero-width space
            | '\u{200c}' // zero-width non-joiner
            | '\u{200d}' // zero-width joiner
            | '\u{2060}' // word joiner
            | '\u{feff}' // zero-width no-break space / BOM
    )
}

/// Levenshtein edit distance over `char`s, used for the section did-you-mean
/// hint. Kept local to the renderer so the message layer carries no dependency
/// on other modules' internals.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    // Single-row dynamic-programming table: `row[j]` holds the distance from the
    // first `i` chars of `a` to the first `j` chars of `b`.
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, &ca) in a.iter().enumerate() {
        let mut prev_diagonal = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            let candidate = (row[j] + 1).min(row[j + 1] + 1).min(prev_diagonal + cost);
            prev_diagonal = row[j + 1];
            row[j + 1] = candidate;
        }
    }
    row[b.len()]
}

/// The `x-jit-section-heading` annotation key desugar attaches to section
/// subschemas so the renderer can name the original heading in a finding.
pub(crate) const SECTION_HEADING_ANNOTATION: &str = "x-jit-section-heading";

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
        let first = engine.validator_for(&key, "cached", schema).unwrap();
        let second = engine.validator_for(&key, "cached", schema).unwrap();
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
        let after = engine.validator_for(&key, "cached", schema).unwrap();
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

    // --- x-jit-* custom keyword extension point (task 33f23ec7) -------------

    /// Sample custom keyword: the annotated string value must be non-empty when
    /// present. The keyword's own schema value is ignored; it exists only to opt a
    /// location into the check. Exercised end-to-end (success criterion 3).
    struct NonEmpty;
    impl jsonschema::Keyword for NonEmpty {
        fn validate<'i>(
            &self,
            instance: &'i serde_json::Value,
        ) -> Result<(), jsonschema::ValidationError<'i>> {
            if self.is_valid(instance) {
                Ok(())
            } else {
                Err(jsonschema::ValidationError::custom(
                    "x-jit-non-empty: string must not be empty",
                ))
            }
        }
        fn is_valid(&self, instance: &serde_json::Value) -> bool {
            instance.as_str().is_none_or(|s| !s.is_empty())
        }
    }

    /// Factory matching the [`with_keyword`](SchemaEngine::with_keyword) HRTB
    /// signature exactly (a free `fn`, so the `for<'a>` bound is satisfied
    /// without lifetime pinning).
    fn non_empty_string_factory<'a>(
        _parent: &'a serde_json::Map<String, serde_json::Value>,
        _schema: &'a serde_json::Value,
        _location: jsonschema::paths::Location,
    ) -> Result<Box<dyn jsonschema::Keyword>, jsonschema::ValidationError<'a>> {
        Ok(Box::new(NonEmpty))
    }

    #[test]
    fn test_degradation_x_jit_keyword_validates_under_standard_validator() {
        // GRACEFUL-DEGRADATION REGRESSION GUARD (success criterion 2):
        //
        // A schema that uses an `x-jit-*` custom keyword must still compile and
        // validate STRUCTURALLY under a STANDARD validator that has NOT registered
        // that keyword. jsonschema 0.46 treats unknown keywords as annotations
        // (never errors), so the custom keyword degrades to a no-op rather than a
        // hard failure. If a future crate bump flipped to strict-keyword rejection
        // this test would fail, alerting us before it reaches users.
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "x-jit-non-empty": true }
            }
        });

        // A STANDARD engine: no custom keywords registered at all.
        let standard = SchemaEngine::new();
        assert!(
            standard.registered_keywords().is_empty(),
            "the default engine must register no custom keywords"
        );

        // Compiling the schema under the standard engine must succeed: the unknown
        // `x-jit-non-empty` keyword is an annotation, not a compile error.
        let key = schema_key(&schema);
        let validator = standard
            .validator_for(&key, "degradation", &schema)
            .expect("schema using x-jit-* must compile under a standard validator");

        // And it must still enforce the STANDARD keywords (`type: string`). The
        // custom keyword silently does nothing, but `title: 7` violates `string`.
        let structurally_bad = serde_json::json!({ "title": 7 });
        assert!(
            validator.iter_errors(&structurally_bad).next().is_some(),
            "standard keywords must still be enforced even with an x-jit-* annotation present"
        );

        // An EMPTY string would violate the custom keyword, but under the standard
        // validator (no registration) it must PASS — proving the keyword degraded
        // to a no-op rather than being enforced or erroring.
        let empty_title = serde_json::json!({ "title": "" });
        assert!(
            validator.iter_errors(&empty_title).next().is_none(),
            "unregistered x-jit-* keyword must degrade to a no-op, not reject"
        );
    }

    #[test]
    fn test_with_keyword_registers_named_keyword() {
        // The registration point records the keyword name on the engine instance.
        let engine = SchemaEngine::new().with_keyword("x-jit-non-empty", non_empty_string_factory);
        assert_eq!(
            engine.registered_keywords(),
            vec!["x-jit-non-empty".to_string()]
        );
    }

    #[test]
    fn test_custom_keyword_enforced_end_to_end() {
        // SAMPLE CUSTOM KEYWORD END-TO-END (success criterion 3):
        //
        // With the `x-jit-non-empty` keyword REGISTERED, a schema using it must now
        // actually enforce the rule: an empty string fails, a non-empty string
        // passes, and a non-string is left to the standard `type` keyword.
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "x-jit-non-empty": true }
            },
            "required": ["title"]
        });
        let rule = Rule {
            name: "title-non-empty".to_string(),
            when: crate::validation::rules::Selector::default(),
            severity: Severity::Error,
            enforce: false,
            assert: crate::validation::rules::Assertion::JsonSchema(
                crate::validation::rules::SchemaSource {
                    reference: "inline".to_string(),
                    path: std::path::PathBuf::from("inline"),
                    schema,
                },
            ),
            scope: crate::validation::rules::Scope::Local,
        };
        let set = RuleSet { rules: vec![rule] };

        let engine = SchemaEngine::new().with_keyword("x-jit-non-empty", non_empty_string_factory);

        // Non-empty title: passes.
        let ok = serde_json::json!({ "title": "Ship it" });
        assert!(
            engine.validate(&set.rules[0], &ok).unwrap().is_empty(),
            "a non-empty title must satisfy the custom keyword"
        );

        // Empty title: the custom keyword fires.
        let bad = serde_json::json!({ "title": "" });
        let findings = engine.validate(&set.rules[0], &bad).unwrap();
        assert_eq!(
            findings.len(),
            1,
            "an empty title must violate the registered custom keyword, got {findings:?}"
        );
        assert_eq!(findings[0].rule, "title-non-empty");
        assert!(findings[0].message.contains("must not be empty"));
    }

    #[test]
    fn test_custom_keyword_engine_still_caches_by_schema() {
        // Registering custom keywords must not break schema-identity caching: the
        // keywords are fixed for the engine instance, so each distinct schema is
        // still compiled at most once.
        let engine = SchemaEngine::new().with_keyword("x-jit-non-empty", non_empty_string_factory);
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "title": { "type": "string", "x-jit-non-empty": true } }
        });
        let key = schema_key(&schema);
        let first = engine.validator_for(&key, "r", &schema).unwrap();
        let second = engine.validator_for(&key, "r", &schema).unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "a custom-keyword engine must still cache validators by schema identity"
        );
        assert_eq!(engine.cache.borrow().len(), 1);
    }

    #[test]
    fn test_with_keyword_invalidates_warm_cache() {
        // A keyword registered AFTER the engine has already compiled a validator
        // for a schema must still take effect: `with_keyword` clears the cache so
        // the schema recompiles with the new keyword set (warm-engine
        // correctness — the gap a prior review flagged).
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "title": { "type": "string", "x-jit-non-empty": true } },
            "required": ["title"]
        });
        let rule = Rule {
            name: "title-non-empty".to_string(),
            when: crate::validation::rules::Selector::default(),
            severity: Severity::Error,
            enforce: false,
            assert: crate::validation::rules::Assertion::JsonSchema(
                crate::validation::rules::SchemaSource {
                    reference: "inline".to_string(),
                    path: std::path::PathBuf::from("inline"),
                    schema,
                },
            ),
            scope: crate::validation::rules::Scope::Local,
        };

        // Warm the cache on an engine with NO custom keyword: the unknown
        // `x-jit-*` keyword degrades to an annotation, so an empty title passes.
        let engine = SchemaEngine::new();
        let bad = serde_json::json!({ "title": "" });
        assert!(
            engine.validate(&rule, &bad).unwrap().is_empty(),
            "without the keyword the empty title is accepted (annotation only)"
        );
        assert_eq!(engine.cache.borrow().len(), 1, "cache is warm");

        // Register the keyword on the SAME (warmed) engine. The stale validator
        // must be discarded so the schema recompiles with the keyword active.
        let engine = engine.with_keyword("x-jit-non-empty", non_empty_string_factory);
        assert!(
            engine.cache.borrow().is_empty(),
            "with_keyword must clear the warm cache"
        );
        let findings = engine.validate(&rule, &bad).unwrap();
        assert_eq!(
            findings.len(),
            1,
            "after registration the warmed engine enforces the keyword, got {findings:?}"
        );
    }

    // === Actionable message rendering (CC-4, task 5a25c590) =================

    /// The SDD-style spec-body schema: requires a `success_criteria` section with
    /// non-empty `items`, each shaped `[hard|aspirational] REQ-N: <text>`, and at
    /// least one `[hard]` item. Carries an `x-jit-section-heading` annotation so
    /// the renderer can name the heading on the empty-projection path.
    fn spec_body_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["sections"],
            "properties": {
                "sections": {
                    "type": "object",
                    "required": ["success_criteria"],
                    "properties": {
                        "success_criteria": {
                            "type": "object",
                            "x-jit-section-heading": "Success Criteria",
                            "required": ["items"],
                            "properties": {
                                "items": {
                                    "type": "array",
                                    "minItems": 1,
                                    "items": {
                                        "type": "string",
                                        "pattern": "^\\[(hard|aspirational)\\]\\s+REQ-[0-9]+:\\s+\\S"
                                    },
                                    "contains": { "type": "string", "pattern": "^\\[hard\\]" },
                                    "minContains": 1
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    fn rule_for(schema: serde_json::Value) -> Rule {
        use crate::validation::rules::{Assertion, SchemaSource, Selector};
        Rule {
            name: "spec".to_string(),
            when: Selector::default(),
            severity: Severity::Error,
            enforce: true,
            assert: Assertion::JsonSchema(SchemaSource {
                reference: "inline".to_string(),
                path: std::path::PathBuf::from("inline"),
                schema,
            }),
            scope: crate::validation::rules::Scope::Local,
        }
    }

    fn messages(rule: &Rule, projection: &serde_json::Value) -> Vec<String> {
        SchemaEngine::new()
            .validate(rule, projection)
            .unwrap()
            .into_iter()
            .map(|f| f.message)
            .collect()
    }

    #[test]
    fn test_empty_section_names_heading_and_demands_bullets() {
        // Prose-without-bullets: the section parsed but yielded an empty `items`
        // array. The message must name the heading and state that items must be
        // Markdown bullets — not the content-free `[] has less than 1 item`.
        let rule = rule_for(spec_body_schema());
        let projection = serde_json::json!({
            "sections": { "success_criteria": { "heading": "Success Criteria", "items": [] } }
        });
        let msgs = messages(&rule, &projection);
        let empty_msg = msgs
            .iter()
            .find(|m| m.contains("no list items"))
            .unwrap_or_else(|| panic!("expected an empty-section message, got {msgs:?}"));
        assert!(
            empty_msg.contains("section 'Success Criteria'"),
            "{empty_msg}"
        );
        assert!(
            empty_msg.contains("must be Markdown bullets (lines starting with '- ')"),
            "{empty_msg}"
        );
        // It must not leak the raw json-schema wording.
        assert!(
            !empty_msg.contains("less than") && !empty_msg.contains("None of"),
            "raw schema wording leaked: {empty_msg}"
        );
    }

    #[test]
    fn test_empty_section_falls_back_to_slug_without_annotation() {
        // A hand-written json-schema rule with no `x-jit-section-heading` falls
        // back to the slug rather than a lossy de-slugify.
        let schema = serde_json::json!({
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
        let rule = rule_for(schema);
        let projection = serde_json::json!({
            "sections": { "success_criteria": { "heading": "Success Criteria", "items": [] } }
        });
        let msgs = messages(&rule, &projection);
        assert!(
            msgs.iter()
                .any(|m| m.contains("section 'success_criteria' has no list items")),
            "expected slug fallback, got {msgs:?}"
        );
    }

    #[test]
    fn test_instance_path_is_prefixed_on_findings() {
        // A non-empty items array whose entry violates the item `pattern` must
        // carry the readable instance path of the failing field.
        let rule = rule_for(spec_body_schema());
        let projection = serde_json::json!({
            "sections": {
                "success_criteria": {
                    "heading": "Success Criteria",
                    "items": ["just some prose, no marker"]
                }
            }
        });
        let msgs = messages(&rule, &projection);
        assert!(
            msgs.iter()
                .any(|m| m.starts_with("at sections.success_criteria.items")),
            "expected a path-prefixed finding, got {msgs:?}"
        );
    }

    #[test]
    fn test_missing_section_emits_did_you_mean() {
        // A typo'd heading (`Sucess Criteria`) slugs to `sucess_criteria`; the
        // required `success_criteria` is absent. The renderer must suggest the
        // near-miss heading actually present.
        let rule = rule_for(spec_body_schema());
        let projection = serde_json::json!({
            "sections": {
                "sucess_criteria": { "heading": "Sucess Criteria", "items": ["[hard] REQ-01: x"] }
            }
        });
        let msgs = messages(&rule, &projection);
        assert!(
            msgs.iter()
                .any(|m| m.contains("did you mean 'Sucess Criteria'?")),
            "expected a did-you-mean hint, got {msgs:?}"
        );
    }

    #[test]
    fn test_missing_section_no_hint_when_no_near_miss() {
        // No present heading is close to the required one: no spurious hint.
        let rule = rule_for(spec_body_schema());
        let projection = serde_json::json!({
            "sections": { "goals": { "heading": "Goals", "items": ["ship"] } }
        });
        let msgs = messages(&rule, &projection);
        assert!(
            msgs.iter().any(|m| m.contains("success_criteria")),
            "the missing section is still named, got {msgs:?}"
        );
        assert!(
            !msgs.iter().any(|m| m.contains("did you mean")),
            "a far-off heading must not produce a hint, got {msgs:?}"
        );
    }

    #[test]
    fn test_humanize_regex_preserves_classes_and_escapes_control() {
        // `\s` survives verbatim; a literal tab and zero-width space become
        // visible escapes rather than raw control characters.
        assert_eq!(humanize_regex(r"^\s+REQ"), r"^\s+REQ");
        assert_eq!(humanize_regex("col1\tcol2"), r"col1\tcol2");
        assert_eq!(humanize_regex("a\u{200b}b"), r"a\u{200b}b");
        // The output of any pattern must never contain a raw control char.
        let rendered = humanize_regex("x\ty\nz\u{feff}");
        assert!(
            !rendered.chars().any(|c| c.is_control()),
            "humanized regex still contains a control char: {rendered:?}"
        );
    }

    #[test]
    fn test_pattern_finding_renders_regex_readably() {
        // A `pattern` failure whose regex contains whitespace classes must render
        // the class readably (no raw control characters in the message).
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "code": { "type": "string", "pattern": "^REQ-[0-9]+$" } }
        });
        let rule = rule_for(schema);
        let projection = serde_json::json!({ "code": "nope" });
        let msgs = messages(&rule, &projection);
        let pattern_msg = &msgs[0];
        assert!(pattern_msg.starts_with("at code:"), "{pattern_msg}");
        assert!(pattern_msg.contains("^REQ-[0-9]+$"), "{pattern_msg}");
        assert!(
            !pattern_msg.chars().any(|c| c.is_control()),
            "pattern message has a raw control char: {pattern_msg:?}"
        );
    }

    // --- contains / non-empty array: actionable message (review finding) ------

    /// Schema shaped like the production SDD spec-body schema but WITHOUT the
    /// `x-jit-section-heading` annotation, to test the slug-fallback path.
    fn sdd_shaped_schema_no_heading() -> serde_json::Value {
        serde_json::json!({
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
                            "properties": {
                                "items": {
                                    "type": "array",
                                    "contains": { "type": "string", "pattern": "^\\[hard\\]" },
                                    "minContains": 1
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    #[test]
    fn test_nonempty_contains_failure_names_heading_and_pattern() {
        // The production SDD path: all items are [aspirational]; none is [hard].
        // The message must (a) name the section heading, (b) state at least one
        // entry must match the [hard] pattern, and (c) NOT dump the full array /
        // contain 'None of'.
        let rule = rule_for(spec_body_schema());
        let projection = serde_json::json!({
            "sections": {
                "success_criteria": {
                    "heading": "Success Criteria",
                    "items": [
                        "[aspirational] REQ-01: nice to have",
                        "[aspirational] REQ-02: also nice"
                    ]
                }
            }
        });
        let msgs = messages(&rule, &projection);
        let contains_msg = msgs
            .iter()
            .find(|m| m.contains("must include at least one entry"))
            .unwrap_or_else(|| panic!("expected a contains message, got {msgs:?}"));

        // (a) names the section heading
        assert!(
            contains_msg.contains("section 'Success Criteria'"),
            "must name the heading: {contains_msg}"
        );
        // (b) states the required pattern (humanized)
        assert!(
            contains_msg.contains("^\\[hard\\]"),
            "must state the required pattern: {contains_msg}"
        );
        // (c) must not dump the full array
        assert!(
            !contains_msg.contains("None of"),
            "must not dump the array: {contains_msg}"
        );
        assert!(
            !contains_msg.contains("[aspirational]"),
            "must not dump array contents: {contains_msg}"
        );
    }

    #[test]
    fn test_nonempty_contains_failure_falls_back_to_slug_without_heading_annotation() {
        // Same as above but the schema has no `x-jit-section-heading` annotation.
        // The message must fall back to the slug rather than dumping the array.
        let rule = rule_for(sdd_shaped_schema_no_heading());
        let projection = serde_json::json!({
            "sections": {
                "success_criteria": {
                    "items": ["[aspirational] nice to have", "[soft] maybe"]
                }
            }
        });
        let msgs = messages(&rule, &projection);
        let contains_msg = msgs
            .iter()
            .find(|m| m.contains("must include at least one entry"))
            .unwrap_or_else(|| panic!("expected a contains message, got {msgs:?}"));

        // Falls back to slug
        assert!(
            contains_msg.contains("success_criteria"),
            "must name the slug: {contains_msg}"
        );
        // States the pattern
        assert!(
            contains_msg.contains("^\\[hard\\]"),
            "must state the required pattern: {contains_msg}"
        );
        // Must not dump the array
        assert!(
            !contains_msg.contains("None of"),
            "must not dump the array: {contains_msg}"
        );
    }

    #[test]
    fn test_nonempty_contains_failure_no_pattern_generic_message() {
        // A `contains` schema without a `pattern` — e.g. requires at least one
        // object item matching a shape. The message must name the path and say
        // "satisfying the required shape", without dumping the full array.
        let schema = serde_json::json!({
            "type": "object",
            "required": ["sections"],
            "properties": {
                "sections": {
                    "type": "object",
                    "required": ["success_criteria"],
                    "properties": {
                        "success_criteria": {
                            "type": "object",
                            "x-jit-section-heading": "Success Criteria",
                            "required": ["items"],
                            "properties": {
                                "items": {
                                    "type": "array",
                                    // contains subschema with no `pattern` — requires
                                    // an object item, not a string pattern match.
                                    "contains": { "type": "object" },
                                    "minContains": 1
                                }
                            }
                        }
                    }
                }
            }
        });
        let rule = rule_for(schema);
        let projection = serde_json::json!({
            "sections": {
                "success_criteria": {
                    "items": ["a string", "another string"]
                }
            }
        });
        let msgs = messages(&rule, &projection);
        let contains_msg = msgs
            .iter()
            .find(|m| m.contains("must include at least one entry"))
            .unwrap_or_else(|| panic!("expected a contains message, got {msgs:?}"));

        assert!(
            contains_msg.contains("section 'Success Criteria'"),
            "must name the heading: {contains_msg}"
        );
        assert!(
            contains_msg.contains("satisfying the required shape"),
            "must say 'required shape': {contains_msg}"
        );
        // Must not dump the array
        assert!(
            !contains_msg.contains("None of"),
            "must not dump the array: {contains_msg}"
        );
    }

    #[test]
    fn test_nonempty_contains_failure_non_section_path_names_path() {
        // A `contains` failure outside of `sections.<slug>.items` — the message
        // must still name the instance path and avoid dumping the array.
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "contains": { "type": "string", "pattern": "^important:" },
                    "minContains": 1
                }
            }
        });
        let rule = rule_for(schema);
        let projection = serde_json::json!({ "tags": ["other:a", "other:b"] });
        let msgs = messages(&rule, &projection);
        let contains_msg = msgs
            .iter()
            .find(|m| m.contains("must include at least one entry"))
            .unwrap_or_else(|| panic!("expected a contains message, got {msgs:?}"));

        // Must name the path
        assert!(
            contains_msg.contains("at tags:"),
            "must prefix with the path: {contains_msg}"
        );
        // Must state the pattern
        assert!(
            contains_msg.contains("^important:"),
            "must state the required pattern: {contains_msg}"
        );
        // Must not dump the array
        assert!(
            !contains_msg.contains("None of"),
            "must not dump the array: {contains_msg}"
        );
    }

    #[test]
    fn test_sloppy_epic_steering_reaches_valid_write_in_two_iterations() {
        // Simulate the sloppy-epic steering loop using ONLY the error text, per
        // the task: an agent must be able to self-correct in <=2 iterations.
        //
        // The projection mirrors what `jit` would project from each body; we
        // assert the message content drives the next fix at each step.
        let rule = rule_for(spec_body_schema());

        // --- Iteration 0: prose, no bullets. The section parsed but is empty.
        let step0 = serde_json::json!({
            "sections": {
                "success_criteria": {
                    "heading": "Success Criteria",
                    "items": []
                }
            }
        });
        let msgs0 = messages(&rule, &step0);
        // The message tells the agent EXACTLY what to do: use Markdown bullets.
        assert!(
            msgs0.iter().any(|m| m.contains("must be Markdown bullets")),
            "step 0 must guide toward bullets, got {msgs0:?}"
        );

        // --- Iteration 1: agent adds bullets, but they lack the [hard] marker
        // and REQ id shape (it followed "use bullets" but not the item shape yet).
        let step1 = serde_json::json!({
            "sections": {
                "success_criteria": {
                    "heading": "Success Criteria",
                    "items": ["the thing works"]
                }
            }
        });
        let msgs1 = messages(&rule, &step1);
        // The findings now name the failing field and the expected pattern, so the
        // agent knows the items themselves are malformed (not the section).
        assert!(
            msgs1
                .iter()
                .any(|m| m.starts_with("at sections.success_criteria.items")),
            "step 1 must name the failing items field, got {msgs1:?}"
        );
        assert!(
            !msgs1.iter().any(|m| m.contains("no list items")),
            "the empty-section error must be gone once bullets exist, got {msgs1:?}"
        );

        // --- Iteration 2: agent fixes item shape per the surfaced pattern.
        let step2 = serde_json::json!({
            "sections": {
                "success_criteria": {
                    "heading": "Success Criteria",
                    "items": ["[hard] REQ-01: the thing works"]
                }
            }
        });
        let msgs2 = messages(&rule, &step2);
        assert!(
            msgs2.is_empty(),
            "a valid write must be reached by iteration 2, got {msgs2:?}"
        );
    }
}
