//! Declarative batch issue creation with dependency wiring from a JSON file.
//!
//! This module backs `jit issue batch-create --from-json <file>`, replacing
//! hand-written create+dependency loops with one declarative array of issue
//! definitions that reference each other by symbolic `key`. The whole file is
//! FULLY pre-validated (duplicate/unknown keys, unknown `depends_on` references,
//! cycle detection over the symbolic graph, type/label/gate validity, priority
//! parse) BEFORE any write, so a malformed batch never leaves partially-created
//! issues behind.
//!
//! ## Atomicity
//!
//! - **Pre-validation** is atomic: on ANY validation failure ZERO issues are
//!   created and a [`BatchValidationError`] enumerating every offending entry is
//!   returned (mapped to exit code 2, `InvalidArgument`).
//! - **Publication is atomic.** The finalizer allocates all ids, resolves every
//!   symbolic edge, and publishes issues, index membership, and audit events in
//!   one recoverable repository delta.

use super::*;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use thiserror::Error;

/// One issue definition in a batch-create file.
///
/// Issues reference each other through symbolic `key`s in [`depends_on`], which
/// are resolved to created issue ids only after all issues exist. The `type`
/// field is applied as a `type:<t>` label; when omitted, the project's configured
/// default type is used (the same convenience `create_issue` applies).
///
/// [`depends_on`]: BatchIssueDef::depends_on
///
/// # Examples
///
/// ```rust
/// use jit::commands::BatchIssueDef;
///
/// let json = r#"{
///   "key": "api",
///   "title": "Design the API",
///   "type": "story",
///   "priority": "high",
///   "labels": ["component:core"],
///   "gates": ["code-review"],
///   "depends_on": ["spec"]
/// }"#;
/// let def: BatchIssueDef = serde_json::from_str(json).unwrap();
/// assert_eq!(def.key, "api");
/// assert_eq!(def.title, "Design the API");
/// assert_eq!(def.r#type.as_deref(), Some("story"));
/// assert_eq!(def.depends_on, vec!["spec".to_string()]);
///
/// // Only `key` and `title` are required; the rest default.
/// let minimal: BatchIssueDef =
///     serde_json::from_str(r#"{ "key": "x", "title": "Minimal" }"#).unwrap();
/// assert_eq!(minimal.description, "");
/// assert!(minimal.r#type.is_none());
/// assert!(minimal.depends_on.is_empty());
/// ```
/// The type also *serializes* so it doubles as the output shape of `jit graph
/// export --format batch`: the export is the exact array batch creation
/// consumes, giving schema symmetry (the batch-create inverse). Defaultable
/// fields are skipped when empty so a serialized def re-parses identically to a
/// hand-written one.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct BatchIssueDef {
    /// Symbolic key, unique within the file. Other entries reference it via
    /// `depends_on`.
    pub key: String,
    /// Issue title (required).
    pub title: String,
    /// Issue description body. Defaults to the empty string.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Issue type, applied as a `type:<t>` label. Defaults to the project's
    /// configured default type when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    /// Priority string (e.g. `low`, `normal`, `high`, `critical`). Defaults to
    /// `normal`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Additional labels in `namespace:value` format.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Quality gate keys to require; each must exist in the gate registry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gates: Vec<String>,
    /// Symbolic `key`s of other entries in the same file this issue depends on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
}

/// A single pre-validation problem, attributed to the offending entry/entries.
///
/// Collected exhaustively (validation does not stop at the first problem) and
/// rendered together by [`BatchValidationError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchValidationProblem {
    /// A `key` value appears on more than one entry.
    DuplicateKey { key: String },
    /// An entry's `depends_on` names a key not defined anywhere in the file.
    UnknownDependency { key: String, missing: String },
    /// The symbolic `depends_on` graph contains a cycle (the keys forming it).
    Cycle { keys: Vec<String> },
    /// An entry's priority string does not parse.
    InvalidPriority { key: String, value: String },
    /// An entry's type is not a known type in the project's type hierarchy.
    UnknownType { key: String, value: String },
    /// An entry requires a gate key absent from the gate registry.
    UnknownGate { key: String, gate: String },
    /// An entry's FINAL issue shape would be rejected by the same write-time
    /// validation [`create_issue`](crate::commands::CommandExecutor::create_issue)
    /// runs (label format, namespace-uniqueness such as a single `type:*`, and
    /// every other enforcing local rule). Catching this before finalization gives
    /// callers a complete, entry-attributed validation report.
    WriteValidation { key: String, message: String },
}

impl std::fmt::Display for BatchValidationProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateKey { key } => write!(f, "duplicate key '{key}'"),
            Self::UnknownDependency { key, missing } => write!(
                f,
                "entry '{key}' depends_on '{missing}', which is not defined in the file"
            ),
            Self::Cycle { keys } => {
                write!(f, "dependency cycle among keys: {}", keys.join(" -> "))
            }
            Self::InvalidPriority { key, value } => {
                write!(f, "entry '{key}' has invalid priority '{value}'")
            }
            Self::UnknownType { key, value } => {
                write!(f, "entry '{key}' has unknown type '{value}'")
            }
            Self::UnknownGate { key, gate } => write!(
                f,
                "entry '{key}' requires gate '{gate}', which is not in the gate registry"
            ),
            Self::WriteValidation { key, message } => {
                write!(f, "entry '{key}' fails validation: {message}")
            }
        }
    }
}

/// Pre-validation failed: ZERO issues were created.
///
/// Carries EVERY offending entry discovered (validation collects all problems
/// rather than stopping at the first). Maps to exit code 2 (`InvalidArgument`).
#[derive(Debug, Clone, Error)]
pub struct BatchValidationError {
    /// Every pre-validation problem found, in discovery order.
    pub problems: Vec<BatchValidationProblem>,
}

impl std::fmt::Display for BatchValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "invalid batch-create file: {} problem(s), no issues were created:",
            self.problems.len()
        )?;
        for problem in &self.problems {
            writeln!(f, "  - {problem}")?;
        }
        Ok(())
    }
}

/// Successful outcome of a batch create: the symbolic `key` to created issue id
/// map, in input order.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BatchCreateOutcome {
    /// `{key: full_id}` pairs in the order the entries appeared in the file.
    pub key_to_id: Vec<(String, String)>,
}

impl BatchCreateOutcome {
    /// Materialize the `{key: id}` pairs as a map for `--json` output and lookup.
    pub fn as_map(&self) -> HashMap<String, String> {
        self.key_to_id.iter().cloned().collect()
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Create a batch of issues with symbolic dependency wiring from declarative
    /// definitions.
    ///
    /// Runs FULL pre-validation (duplicate/unknown keys, unknown `depends_on`
    /// references, cycle detection over the symbolic graph, type/label/gate
    /// validity, priority parse) BEFORE any write. On any validation failure it
    /// returns a [`BatchValidationError`] enumerating every offender and creates
    /// ZERO issues. On success it finalizes every issue and dependency edge into
    /// one recoverable delta, returning the `{key: id}` map.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::{BatchIssueDef, CommandExecutor};
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let defs = vec![
    ///     BatchIssueDef {
    ///         key: "spec".into(),
    ///         title: "Write the spec".into(),
    ///         description: String::new(),
    ///         r#type: Some("story".into()),
    ///         priority: None,
    ///         labels: vec![],
    ///         gates: vec![],
    ///         depends_on: vec![],
    ///     },
    ///     BatchIssueDef {
    ///         key: "impl".into(),
    ///         title: "Implement it".into(),
    ///         description: String::new(),
    ///         r#type: Some("task".into()),
    ///         priority: None,
    ///         labels: vec![],
    ///         gates: vec![],
    ///         depends_on: vec!["spec".into()],
    ///     },
    /// ];
    /// let outcome = executor.batch_create_from_json(defs).unwrap();
    /// println!("spec -> {}", outcome.as_map()["spec"]);
    /// ```
    pub fn batch_create_from_json(&self, defs: Vec<BatchIssueDef>) -> Result<BatchCreateOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // FULL pre-validation: collect every problem, write nothing on failure.
        let problems = self.collect_batch_problems(&defs)?;
        if !problems.is_empty() {
            return Err(BatchValidationError { problems }.into());
        }

        let drafts = defs
            .iter()
            .map(|def| {
                let priority = def
                    .priority
                    .as_deref()
                    .map(Priority::from_str)
                    .transpose()?
                    .unwrap_or(Priority::Normal);
                self.batch_candidate_issue(def, priority)
            })
            .collect::<Result<Vec<_>>>()?;
        let positions = defs
            .iter()
            .enumerate()
            .map(|(index, def)| (def.key.as_str(), index))
            .collect::<HashMap<_, _>>();
        let mut dependencies = Vec::new();
        for (dependent, def) in defs.iter().enumerate() {
            dependencies.extend(
                def.depends_on
                    .iter()
                    .map(|dependency| (dependent, positions[dependency.as_str()])),
            );
        }
        let intent = crate::repository_state::MutationIntent::CreateIssueBatch {
            drafts,
            dependencies,
        };
        let publication = self.publish_repository_mutation(vec![intent])?;
        Ok(BatchCreateOutcome {
            key_to_id: defs
                .into_iter()
                .zip(publication.created_issue_ids)
                .map(|(def, id)| (def.key, id))
                .collect(),
        })
    }

    /// Collect EVERY pre-validation problem for a batch (does not stop at the
    /// first). Returns an empty vec when the whole batch is valid.
    ///
    /// Reads the gate registry and config (type hierarchy) once for the batch.
    fn collect_batch_problems(
        &self,
        defs: &[BatchIssueDef],
    ) -> Result<Vec<BatchValidationProblem>> {
        let mut problems = Vec::new();

        // 1. Duplicate keys (each duplicated key reported once).
        let mut seen: HashSet<&str> = HashSet::new();
        let mut reported_dup: HashSet<&str> = HashSet::new();
        for def in defs {
            if !seen.insert(def.key.as_str()) && reported_dup.insert(def.key.as_str()) {
                problems.push(BatchValidationProblem::DuplicateKey {
                    key: def.key.clone(),
                });
            }
        }

        // The set of defined keys, for resolving `depends_on` references.
        let defined: HashSet<&str> = defs.iter().map(|d| d.key.as_str()).collect();

        // 2. Unknown `depends_on` references.
        for def in defs {
            for dep in &def.depends_on {
                if !defined.contains(dep.as_str()) {
                    problems.push(BatchValidationProblem::UnknownDependency {
                        key: def.key.clone(),
                        missing: dep.clone(),
                    });
                }
            }
        }

        // 3. Cycle detection over the symbolic graph (only over edges whose
        //    endpoints are both defined, so missing-ref problems aren't
        //    double-reported as cycles).
        if let Some(cycle) = detect_cycle(defs) {
            problems.push(BatchValidationProblem::Cycle { keys: cycle });
        }

        // 4-7. Per-entry field validity: priority parse, type, gates, and the
        //       FULL write-time validation against each entry's final issue shape.
        let known_types = self.batch_known_types()?;
        let gate_registry = self
            .storage
            .load_gate_registry()
            .context("Failed to load gate registry for batch validation")?;

        for def in defs {
            // Priority must parse before we can build the candidate issue; a parse
            // failure is reported and we skip write-time validation for this entry
            // (there is no valid shape to validate).
            let priority = match &def.priority {
                Some(p) => match Priority::from_str(p) {
                    Ok(priority) => priority,
                    Err(_) => {
                        problems.push(BatchValidationProblem::InvalidPriority {
                            key: def.key.clone(),
                            value: p.clone(),
                        });
                        continue;
                    }
                },
                None => Priority::Normal,
            };

            // Type validity: only enforced when the project configures a type
            // hierarchy; an empty set means any type is accepted. (The write-time
            // `type-hierarchy-known` rule (origin = "default") only WARNS, so this
            // explicit check is what makes an unknown type a blocking
            // pre-validation error.)
            if let (Some(t), Some(known)) = (&def.r#type, known_types.as_ref()) {
                if !known.contains(t.as_str()) {
                    problems.push(BatchValidationProblem::UnknownType {
                        key: def.key.clone(),
                        value: t.clone(),
                    });
                }
            }

            for gate in &def.gates {
                if !gate_registry.gates.contains_key(gate) {
                    problems.push(BatchValidationProblem::UnknownGate {
                        key: def.key.clone(),
                        gate: gate.clone(),
                    });
                }
            }

            // FULL write-time validation against the FINAL issue shape, exactly
            // as `create_issue` assembles and validates it (label format,
            // namespace-uniqueness like a single `type:*`, every enforcing local
            // rule). This is what catches write-time-only violations BEFORE any
            // publication, so all entry-attributed problems are reported together.
            // `validate_for_write` does not save; with `force = false` a blocking finding is returned as an
            // error whose message we attribute to this entry's key.
            let candidate = self.batch_candidate_issue(def, priority)?;
            if let Err(err) = self.validate_for_write(&candidate, false) {
                problems.push(BatchValidationProblem::WriteValidation {
                    key: def.key.clone(),
                    message: err.to_string(),
                });
            }
        }

        Ok(problems)
    }

    /// Build the candidate [`Issue`] for an entry EXACTLY as
    /// [`create_issue`](Self::create_issue) assembles it before validating, so
    /// pre-validation sees the shape that would actually be persisted: the config
    /// default `type:<t>` label applied when the entry carries no `type:` label,
    /// the requested priority / gates / labels / content format, and the state
    /// implied by its declared dependencies.
    fn batch_candidate_issue(&self, def: &BatchIssueDef, priority: Priority) -> Result<Issue> {
        // Assemble labels as the write path does: the `type` field becomes a
        // `type:<t>` label, followed by the entry's explicit labels.
        let mut labels: Vec<String> = def
            .r#type
            .as_deref()
            .map(label_utils::type_label)
            .into_iter()
            .chain(def.labels.iter().cloned())
            .collect();

        // Apply the configured default type when no `type:*` label is present
        // (mirrors create_issue's write-time convenience).
        if let Some(default_type) = self
            .cached_config()?
            .validation
            .as_ref()
            .and_then(|v| v.default_type.as_deref())
        {
            let has_type = label_utils::type_label_value(&labels).is_some();
            if !has_type {
                labels.push(label_utils::type_label(default_type));
            }
        }

        let mut issue = Issue::draft(def.title.clone(), def.description.clone());
        issue.priority = priority;
        issue.gates_required = def.gates.clone();
        issue.labels = labels;
        issue.content_format = None;
        // The finalizer publishes this same dependency-derived state while wiring
        // all symbolic edges in the aggregate delta.
        issue.state = if def.depends_on.is_empty() {
            State::Ready
        } else {
            State::Backlog
        };
        Ok(issue)
    }

    /// The set of known type names from the project's `[type_hierarchy]`, or
    /// `None` when no hierarchy is configured (in which case any type is valid,
    /// matching the write-path `type-hierarchy-known` rule's behavior).
    fn batch_known_types(&self) -> Result<Option<HashSet<String>>> {
        let config = self.cached_config()?;
        Ok(config
            .type_hierarchy
            .as_ref()
            .map(|h| h.types.keys().cloned().collect()))
    }
}

/// Detect a cycle in the symbolic `depends_on` graph over the given entries.
///
/// Projects the entries onto a keyed adjacency and hands it to
/// [`find_keyed_cycle`], which ignores edges to keys the batch never defines, so
/// an unknown-reference problem is not double-reported as a cycle. Returns the
/// keys forming one detected cycle (in traversal order, closing back to the
/// start), or `None` when the graph is acyclic.
fn detect_cycle(defs: &[BatchIssueDef]) -> Option<Vec<String>> {
    let adjacency: Vec<(&str, Vec<&str>)> = defs
        .iter()
        .map(|def| {
            let edges = def.depends_on.iter().map(String::as_str).collect();
            (def.key.as_str(), edges)
        })
        .collect();

    crate::graph::find_keyed_cycle(&adjacency)
        .map(|cycle| cycle.into_iter().map(str::to_string).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::InMemoryStorage;

    fn def(key: &str, deps: &[&str]) -> BatchIssueDef {
        BatchIssueDef {
            key: key.to_string(),
            title: format!("Issue {key}"),
            description: String::new(),
            r#type: None,
            priority: None,
            labels: vec![],
            gates: vec![],
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn executor() -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();
        crate::commands::test_helpers::memory_executor(storage)
    }

    #[test]
    fn test_detect_cycle_finds_simple_cycle() {
        let defs = vec![def("a", &["b"]), def("b", &["a"])];
        let cycle = detect_cycle(&defs).expect("cycle expected");
        assert!(cycle.contains(&"a".to_string()));
        assert!(cycle.contains(&"b".to_string()));
    }

    #[test]
    fn test_detect_cycle_self_loop() {
        let defs = vec![def("a", &["a"])];
        let cycle = detect_cycle(&defs).expect("self-loop is a cycle");
        assert_eq!(cycle.first().map(String::as_str), Some("a"));
    }

    #[test]
    fn test_detect_cycle_acyclic_returns_none() {
        let defs = vec![def("a", &["b"]), def("b", &["c"]), def("c", &[])];
        assert!(detect_cycle(&defs).is_none());
    }

    #[test]
    fn test_detect_cycle_ignores_undefined_edges() {
        // b is undefined; that is an unknown-ref problem, not a cycle.
        let defs = vec![def("a", &["b"])];
        assert!(detect_cycle(&defs).is_none());
    }

    #[test]
    fn test_collect_problems_duplicate_key() {
        let exec = executor();
        let defs = vec![def("a", &[]), def("a", &[])];
        let problems = exec.collect_batch_problems(&defs).unwrap();
        assert!(problems
            .iter()
            .any(|p| matches!(p, BatchValidationProblem::DuplicateKey { key } if key == "a")));
    }

    #[test]
    fn test_collect_problems_unknown_dependency() {
        let exec = executor();
        let defs = vec![def("a", &["ghost"])];
        let problems = exec.collect_batch_problems(&defs).unwrap();
        assert!(problems.iter().any(|p| matches!(
            p,
            BatchValidationProblem::UnknownDependency { key, missing }
                if key == "a" && missing == "ghost"
        )));
    }

    #[test]
    fn test_collect_problems_invalid_label_via_write_validation() {
        // A malformed label is now caught by the full write-time validation
        // (`label-format`, origin = "default"), surfaced as a WriteValidation
        // problem.
        let exec = executor();
        let mut d = def("a", &[]);
        d.labels = vec!["NoColon".to_string()];
        let problems = exec.collect_batch_problems(&[d]).unwrap();
        assert!(problems.iter().any(
            |p| matches!(p, BatchValidationProblem::WriteValidation { key, .. } if key == "a")
        ));
    }

    #[test]
    fn test_collect_problems_duplicate_type_label_via_write_validation() {
        // The `type` field plus an explicit `type:*` label yields two `type:`
        // labels, which write-time namespace-uniqueness validation rejects
        // (`namespace-unique-type`, origin = "default"). This is the
        // write-time-only violation that previously slipped past the batch's
        // validation report.
        let exec = executor();
        let mut d = def("bad", &[]);
        d.r#type = Some("task".to_string());
        d.labels = vec!["type:story".to_string()];
        let problems = exec.collect_batch_problems(&[d]).unwrap();
        assert!(problems.iter().any(
            |p| matches!(p, BatchValidationProblem::WriteValidation { key, .. } if key == "bad")
        ));
    }

    #[test]
    fn test_collect_problems_invalid_priority() {
        let exec = executor();
        let mut d = def("a", &[]);
        d.priority = Some("urgentish".to_string());
        let problems = exec.collect_batch_problems(&[d]).unwrap();
        assert!(problems
            .iter()
            .any(|p| matches!(p, BatchValidationProblem::InvalidPriority { .. })));
    }

    #[test]
    fn test_collect_problems_unknown_gate() {
        let exec = executor();
        let mut d = def("a", &[]);
        d.gates = vec!["nonexistent-gate".to_string()];
        let problems = exec.collect_batch_problems(&[d]).unwrap();
        assert!(problems
            .iter()
            .any(|p| matches!(p, BatchValidationProblem::UnknownGate { .. })));
    }

    #[test]
    fn test_collect_problems_unknown_type_with_hierarchy() {
        let exec = executor();
        // Configure a type hierarchy so type validity is enforced.
        std::fs::write(
            exec.storage().root().join("config.toml"),
            "[type_hierarchy]\ntypes = { epic = 2, task = 4 }\n",
        )
        .unwrap();
        let mut d = def("a", &[]);
        d.r#type = Some("widget".to_string());
        let problems = exec.collect_batch_problems(&[d]).unwrap();
        assert!(problems.iter().any(
            |p| matches!(p, BatchValidationProblem::UnknownType { value, .. } if value == "widget")
        ));
    }

    #[test]
    fn test_collect_problems_valid_batch_is_empty() {
        let exec = executor();
        let defs = vec![def("a", &["b"]), def("b", &[])];
        assert!(exec.collect_batch_problems(&defs).unwrap().is_empty());
    }

    #[test]
    fn test_batch_create_valid_creates_issues_and_edges() {
        let exec = executor();
        let defs = vec![def("spec", &[]), def("impl", &["spec"])];
        let outcome = exec.batch_create_from_json(defs).unwrap();
        let map = outcome.as_map();
        assert_eq!(map.len(), 2);

        // Both issues exist.
        let impl_id = &map["impl"];
        let spec_id = &map["spec"];
        let impl_issue = exec.storage().load_issue(impl_id).unwrap();
        // The edge impl -> spec was wired.
        assert!(impl_issue.dependencies.contains(spec_id));
    }

    #[test]
    fn test_batch_create_validation_failure_creates_nothing() {
        let exec = executor();
        let defs = vec![def("a", &[]), def("a", &[])]; // duplicate key
        let err = exec.batch_create_from_json(defs).unwrap_err();
        assert!(err.downcast_ref::<BatchValidationError>().is_some());
        // Zero issues created.
        assert_eq!(exec.storage().list_issues().unwrap().len(), 0);
    }

    #[test]
    fn test_batch_create_cycle_creates_nothing() {
        let exec = executor();
        let defs = vec![def("a", &["b"]), def("b", &["a"])];
        let err = exec.batch_create_from_json(defs).unwrap_err();
        let verr = err.downcast_ref::<BatchValidationError>().unwrap();
        assert!(verr
            .problems
            .iter()
            .any(|p| matches!(p, BatchValidationProblem::Cycle { .. })));
        assert_eq!(exec.storage().list_issues().unwrap().len(), 0);
    }
}
