//! Declarative batch issue creation with dependency wiring from a JSON file.
//!
//! This module backs `jit issue batch-create --from-json <file>`, replacing
//! hand-written create+dependency loops with one declarative array of issue
//! definitions that reference each other by symbolic `key`. The whole file is
//! FULLY pre-validated (duplicate/unknown keys, unknown `depends_on` references,
//! cycle detection over the symbolic graph, type/label/gate validity)
//! BEFORE any write, so a malformed batch never leaves partially-created
//! issues behind.
//!
//! ## Atomicity
//!
//! - **Pre-validation** is atomic: on ANY validation failure ZERO issues are
//!   created and a [`BatchValidationError`] enumerating every offending entry is
//!   returned (mapped to exit code 2, `InvalidArgument`).
//! - **Publication is atomic.** Each draft carries the id the mutation context
//!   derived for its entry, its dependencies resolved to those ids, and its
//!   derived readiness; the finalizer publishes issues and audit events in one
//!   recoverable repository delta.

use super::*;
use crate::domain::identity::GateKey;
use std::collections::{HashMap, HashSet};
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
    /// Priority. Defaults to `normal`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Additional labels in `namespace:value` format.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Quality gate keys to require; each must exist in the gate registry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gates: Vec<GateKey>,
    /// Symbolic `key`s of other entries in the same file this issue depends on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Opaque authoring metadata accepted from planning manifests. It is never
    /// interpreted, stored on an issue, or included in batch export.
    #[serde(default, skip_serializing)]
    pub planning: Option<serde_json::Value>,
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
    /// An entry's coverage-credit label references (`{<key>.short_id}`) a key
    /// not defined anywhere in the file.
    UnknownLabelKey {
        key: String,
        label: String,
        missing: String,
    },
    /// The symbolic `depends_on` graph contains a cycle (the keys forming it).
    Cycle { keys: Vec<String> },
    /// An entry's type is not a known type in the project's type hierarchy.
    UnknownType { key: String, value: String },
    /// An entry requires a gate key absent from the gate registry.
    UnknownGate { key: String, gate: GateKey },
    /// An entry's FINAL issue shape would be rejected by the same write-time
    /// validation [`create_issue`](crate::commands::IssueOps::create_issue)
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
            Self::UnknownLabelKey {
                key,
                label,
                missing,
            } => write!(
                f,
                "entry '{key}' label '{label}' references key '{missing}', which is not defined \
                 in the file"
            ),
            Self::Cycle { keys } => {
                write!(f, "dependency cycle among keys: {}", keys.join(" -> "))
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
    pub key_to_id: Vec<(String, IssueId)>,
}

/// Successful validation-only result for `batch-create --dry-run`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub struct BatchDryRunOutcome {
    /// Always true for a successful validation result.
    pub valid: bool,
    /// Distinguishes this result from creation output.
    pub dry_run: bool,
    /// Number of validated issue definitions.
    pub issue_count: usize,
    /// Number of validated symbolic dependency edges.
    pub dependency_count: usize,
    /// Semantic keys in input order.
    pub keys: Vec<String>,
}

impl BatchCreateOutcome {
    /// Materialize the `{key: id}` pairs as a map for `--json` output and lookup.
    pub fn as_map(&self) -> HashMap<String, IssueId> {
        self.key_to_id.iter().cloned().collect()
    }
}

impl<S: IssueStore> IssueOps<'_, S> {
    /// Run native batch pre-validation without allocating ids or writing state.
    pub fn validate_batch_from_json(&self, defs: &[BatchIssueDef]) -> Result<BatchDryRunOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let problems = self.collect_batch_problems(defs)?;
        if !problems.is_empty() {
            return Err(BatchValidationError { problems }.into());
        }
        Ok(BatchDryRunOutcome {
            valid: true,
            dry_run: true,
            issue_count: defs.len(),
            dependency_count: defs.iter().map(|def| def.depends_on.len()).sum(),
            keys: defs.iter().map(|def| def.key.clone()).collect(),
        })
    }

    /// Create a batch of issues with symbolic dependency wiring from declarative
    /// definitions.
    ///
    /// Runs FULL pre-validation (duplicate/unknown keys, unknown `depends_on`
    /// references, cycle detection over the symbolic graph, type/label/gate
    /// validity) BEFORE any write. On any validation failure it
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
    ///         planning: None,
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
    ///         planning: None,
    ///     },
    /// ];
    /// let outcome = executor.issues().batch_create_from_json(defs).unwrap();
    /// println!("spec -> {}", outcome.as_map()["spec"]);
    /// ```
    pub fn batch_create_from_json(&self, defs: Vec<BatchIssueDef>) -> Result<BatchCreateOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, MutationIntent, VirtualPath};

        self.validate_batch_from_json(&defs)?;

        let layout = self.exec.require_layout()?;
        let context = super::production_mutation_context();
        let created_issue_ids = batch_issue_ids(&context, defs.len());
        let mut issue_paths = created_issue_ids
            .iter()
            .map(VirtualPath::issue_record)
            .collect::<Vec<_>>();
        issue_paths.push(VirtualPath::ISSUES);

        with_mutation_session(
            &self.exec.storage,
            &layout,
            "batch issue creation",
            |session| {
                let Some(base) = self
                    .exec
                    .capture_mutation_base(session, &issue_paths, &[])?
                else {
                    return Ok(SessionStep::Retry);
                };
                let declarations = base.declarations()?;
                let problems = Self::collect_batch_problems_from_declarations(
                    &defs,
                    &created_issue_ids,
                    declarations,
                    &base.issues,
                )?;
                if !problems.is_empty() {
                    return Err(BatchValidationError { problems }.into());
                }
                let drafts =
                    Self::batch_drafts(&defs, &created_issue_ids, declarations, &base.issues)?
                        .into_iter()
                        .map(|draft| {
                            draft.map_err(|_| {
                                anyhow!("validated batch contains an unresolved label key")
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                let intents = [MutationIntent::CreateIssueBatch { drafts }];
                let plan = finalize(&layout, &base.image, &context, &intents, None)?;
                Ok(SessionStep::Apply(
                    plan,
                    BatchCreateOutcome {
                        key_to_id: defs
                            .iter()
                            .map(|def| def.key.clone())
                            .zip(created_issue_ids.iter().cloned())
                            .collect(),
                    },
                ))
            },
        )
    }

    /// Collect EVERY pre-validation problem for a batch (does not stop at the
    /// first). Returns an empty vec when the whole batch is valid.
    ///
    /// Reads the gate registry and config (type hierarchy) once for the batch.
    /// Each entry is validated as the issue with its position's id in `ids`.
    fn collect_batch_problems_from_declarations(
        defs: &[BatchIssueDef],
        ids: &[IssueId],
        declarations: &crate::repository_state::CapturedRepositoryDeclarations,
        issues: &[Issue],
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

        // 2a. Unknown keys referenced by coverage-credit labels. Each known key
        //     resolves to its entry's short id, so write validation below sees the
        //     final label shape.
        let drafts: Vec<Option<Issue>> = Self::batch_drafts(defs, ids, declarations, issues)?
            .into_iter()
            .zip(defs)
            .map(|(draft, def)| {
                draft
                    .map_err(|unknown| {
                        problems.extend(unknown.into_iter().map(|(label, missing)| {
                            BatchValidationProblem::UnknownLabelKey {
                                key: def.key.clone(),
                                label,
                                missing,
                            }
                        }))
                    })
                    .ok()
            })
            .collect();

        // 2b. Unknown `depends_on` references.
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

        // 4-7. Per-entry field validity: type, gates, and the
        //       FULL write-time validation against each entry's final issue shape.
        let known_types = declarations
            .config()
            .type_hierarchy
            .as_ref()
            .map(|hierarchy| {
                hierarchy
                    .types
                    .keys()
                    .map(String::as_str)
                    .collect::<HashSet<_>>()
            });
        let gate_registry = declarations.gates();

        for (def, draft) in defs.iter().zip(drafts) {
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
            // This derivation does not save; a blocking finding is returned as an
            // error whose message we attribute to this entry's key.
            let Some(candidate) = draft else {
                continue;
            };
            if let Err(err) = derive_write_validation(&candidate, declarations, false) {
                problems.push(BatchValidationProblem::WriteValidation {
                    key: def.key.clone(),
                    message: err.to_string(),
                });
            }
        }

        Ok(problems)
    }

    /// Each entry's draft at its position's id in `ids`, or the label key
    /// references it leaves unresolved. Readiness is derived over `issues` plus
    /// the drafts, so write validation sees the persisted shape.
    fn batch_drafts(
        defs: &[BatchIssueDef],
        ids: &[IssueId],
        declarations: &crate::repository_state::CapturedRepositoryDeclarations,
        issues: &[Issue],
    ) -> Result<Vec<BatchDraft>> {
        let credit = Self::batch_credit_namespaces(declarations)?;
        let issue_ids = defs
            .iter()
            .zip(ids)
            .map(|(def, id)| (def.key.as_str(), id))
            .collect::<HashMap<_, _>>();
        let mut drafts = defs
            .iter()
            .zip(ids)
            .map(|(def, id)| {
                let labels =
                    resolve_labels(def, &credit, |key| issue_ids.get(key).copied().cloned())?;
                let mut draft = Self::batch_candidate_issue(
                    &BatchIssueDef {
                        labels,
                        ..def.clone()
                    },
                    id.clone(),
                    declarations.config(),
                );
                draft.dependencies = def
                    .depends_on
                    .iter()
                    .filter_map(|key| issue_ids.get(key.as_str()).copied().cloned())
                    .collect();
                draft.dependencies.sort();
                draft.dependencies.dedup();
                Ok(draft)
            })
            .collect::<Vec<_>>();
        let projected = issues
            .iter()
            .cloned()
            .chain(drafts.iter().flatten().cloned())
            .collect::<Vec<_>>();
        let promoted = super::derive_draft_promotions(issues, &projected);
        drafts
            .iter_mut()
            .flatten()
            .filter(|draft| promoted.contains(&draft.id))
            .for_each(|draft| draft.state = State::Ready);
        Ok(drafts)
    }

    /// Build the `Backlog` candidate [`Issue`] for an entry as
    /// [`create_issue`](crate::commands::IssueOps::create_issue) assembles it:
    /// the config default `type:<t>` label applied when the entry carries no
    /// `type:` label, and the requested priority / gates / labels.
    fn batch_candidate_issue(def: &BatchIssueDef, id: IssueId, config: &JitConfig) -> Issue {
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
        if let Some(default_type) = config
            .validation
            .as_ref()
            .and_then(|v| v.default_type.as_deref())
        {
            let has_type = label_utils::type_label_value(&labels).is_some();
            if !has_type {
                labels.push(label_utils::type_label(default_type));
            }
        }

        let mut issue = Issue::draft(id, def.title.clone(), def.description.clone());
        issue.priority = def.priority.unwrap_or(Priority::Normal);
        issue.gates_required = def.gates.clone();
        issue.labels = labels;
        issue
    }

    /// The label namespaces in which a `{<key>.short_id}` reference resolves:
    /// those that credit an issue-scoped item under the captured declarations.
    fn batch_credit_namespaces(
        declarations: &crate::repository_state::CapturedRepositoryDeclarations,
    ) -> Result<HashSet<String>> {
        let kinds =
            crate::domain::item::resolve_item_kinds(declarations.config().item_kinds.as_ref())?;
        Ok(crate::validation::graph::issue_credit_namespaces(
            &declarations.rules().rules,
            &kinds,
        ))
    }

    fn collect_batch_problems(&self, defs: &[BatchIssueDef]) -> Result<Vec<BatchValidationProblem>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let layout = self.exec.require_layout()?;
        super::with_read_session(
            self.exec.storage(),
            &layout,
            "batch validation",
            |session| {
                let Some(base) = self.exec.capture_mutation_base(session, &[], &[])? else {
                    return Ok(None);
                };
                let ids = batch_issue_ids(
                    &crate::repository_state::MutationContext::preview(),
                    defs.len(),
                );
                Ok(Some(Self::collect_batch_problems_from_declarations(
                    defs,
                    &ids,
                    base.declarations()?,
                    &base.issues,
                )?))
            },
        )
    }
}

/// An entry's draft, or the `(label, missing key)` references it leaves
/// unresolved.
type BatchDraft = std::result::Result<Issue, Vec<(String, String)>>;

/// The ids `context` derives for a batch of `count` created issues, in entry
/// order.
fn batch_issue_ids(
    context: &crate::repository_state::MutationContext,
    count: usize,
) -> Vec<IssueId> {
    (0..count as u64)
        .map(|index| context.issue_id_at(index))
        .collect()
}

/// `def`'s labels with every manifest-key reference in a `credit` namespace
/// resolved through `issue_id_of`, or every `(label, missing key)` that does not
/// resolve.
fn resolve_labels(
    def: &BatchIssueDef,
    credit: &HashSet<String>,
    issue_id_of: impl Fn(&str) -> Option<IssueId>,
) -> std::result::Result<Vec<String>, Vec<(String, String)>> {
    let (resolved, unknown): (Vec<_>, Vec<_>) = def
        .labels
        .iter()
        .map(|label| {
            crate::domain::key_reference::resolve_key_reference(label, credit, &issue_id_of)
                .map_err(|missing| (label.clone(), missing.0))
        })
        .partition(std::result::Result::is_ok);
    if unknown.is_empty() {
        Ok(resolved
            .into_iter()
            .filter_map(std::result::Result::ok)
            .collect())
    } else {
        Err(unknown
            .into_iter()
            .filter_map(std::result::Result::err)
            .collect())
    }
}

/// Detect a cycle in the symbolic `depends_on` graph over the given entries.
///
/// Projects the entries onto a keyed adjacency and hands it to
/// [`crate::graph::find_keyed_cycle`], which ignores edges to keys the batch never defines, so
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
            planning: None,
        }
    }

    #[test]
    fn test_validate_batch_from_json_does_not_wait_for_held_writer() {
        let storage = InMemoryStorage::new();
        let exec = crate::commands::test_helpers::memory_executor(storage.clone());
        let guard = storage.acquire_repo_write_lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            tx.send(exec.issues().validate_batch_from_json(&[]))
                .unwrap()
        });
        let result = rx.recv_timeout(std::time::Duration::from_secs(2));
        drop(guard);
        thread.join().unwrap();
        let result = result.expect("batch preview waited for the writer");
        assert!(
            result.is_ok()
                || matches!(
                    result
                        .unwrap_err()
                        .downcast_ref::<crate::storage::RepositoryStateStoreError>(),
                    Some(crate::storage::RepositoryStateStoreError::RetryableConflict { .. })
                )
        );
    }

    #[test]
    fn test_collect_problems_duplicate_key() {
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        );
        let defs = vec![def("a", &[]), def("a", &[])];
        let problems = exec.issues().collect_batch_problems(&defs).unwrap();
        assert!(problems
            .iter()
            .any(|p| matches!(p, BatchValidationProblem::DuplicateKey { key } if key == "a")));
    }

    #[test]
    fn test_collect_problems_unknown_dependency() {
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        );
        let defs = vec![def("a", &["ghost"])];
        let problems = exec.issues().collect_batch_problems(&defs).unwrap();
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
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        );
        let mut d = def("a", &[]);
        d.labels = vec!["NoColon".to_string()];
        let problems = exec.issues().collect_batch_problems(&[d]).unwrap();
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
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        );
        // Two distinct kinds the fixture repository declares, read from the
        // declaration it was configured from.
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let mut d = def("bad", &[]);
        d.r#type = Some(taxonomy.type_at_level(4).to_string());
        d.labels = vec![format!("type:{}", taxonomy.type_at_level(3))];
        let problems = exec.issues().collect_batch_problems(&[d]).unwrap();
        assert!(problems.iter().any(
            |p| matches!(p, BatchValidationProblem::WriteValidation { key, .. } if key == "bad")
        ));
    }

    #[test]
    fn test_collect_problems_unknown_gate() {
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        );
        let mut d = def("a", &[]);
        d.gates = vec!["nonexistent-gate".parse().unwrap()];
        let problems = exec.issues().collect_batch_problems(&[d]).unwrap();
        assert!(problems
            .iter()
            .any(|p| matches!(p, BatchValidationProblem::UnknownGate { .. })));
    }

    #[test]
    fn test_collect_problems_unknown_type_with_hierarchy() {
        // The fixture repository declares a type hierarchy, so type validity
        // is enforced against it.
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        );
        let mut d = def("a", &[]);
        d.r#type = Some("widget".to_string());
        let problems = exec.issues().collect_batch_problems(&[d]).unwrap();
        assert!(problems.iter().any(
            |p| matches!(p, BatchValidationProblem::UnknownType { value, .. } if value == "widget")
        ));
    }

    #[test]
    fn test_collect_problems_valid_batch_is_empty() {
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        );
        let defs = vec![def("a", &["b"]), def("b", &[])];
        assert!(exec
            .issues()
            .collect_batch_problems(&defs)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_batch_create_valid_creates_issues_and_edges() {
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        );
        let defs = vec![def("spec", &[]), def("impl", &["spec"])];
        let outcome = exec.issues().batch_create_from_json(defs).unwrap();
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
    fn test_validate_batch_from_json_has_no_issue_or_event_side_effects() {
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        );
        let defs = vec![def("spec", &[]), def("impl", &["spec"])];
        let before_events = exec.storage().read_events().unwrap();
        let outcome = exec.issues().validate_batch_from_json(&defs).unwrap();

        assert_eq!(
            outcome,
            BatchDryRunOutcome {
                valid: true,
                dry_run: true,
                issue_count: 2,
                dependency_count: 1,
                keys: vec!["spec".into(), "impl".into()],
            }
        );
        assert!(exec.storage().list_issues().unwrap().is_empty());
        assert_eq!(exec.storage().read_events().unwrap(), before_events);
    }

    #[test]
    fn test_validate_batch_from_json_empty_batch_returns_zero_counts() {
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let outcome = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        )
        .issues()
        .validate_batch_from_json(&[])
        .unwrap();
        assert_eq!(outcome.issue_count, 0);
        assert_eq!(outcome.dependency_count, 0);
        assert!(outcome.keys.is_empty());
    }

    #[test]
    fn test_batch_issue_def_planning_deserializes_but_never_serializes() {
        let def: BatchIssueDef =
            serde_json::from_str(r#"{"key":"task","title":"Task","planning":{"outcome":"done"}}"#)
                .unwrap();
        assert!(def.planning.is_some());
        assert!(serde_json::to_value(def).unwrap().get("planning").is_none());
    }

    #[test]
    fn test_collect_batch_problems_matches_shared_structural_conformance_fixture() {
        #[derive(serde::Deserialize)]
        struct Suite {
            cases: Vec<Case>,
        }
        #[derive(serde::Deserialize)]
        struct Case {
            name: String,
            expected: Vec<String>,
            issues: Vec<BatchIssueDef>,
        }

        let suite: Suite = serde_json::from_str(include_str!(
            "../../tests/fixtures/batch-structural-conformance.json"
        ))
        .unwrap();
        for case in suite.cases {
            let config = format!(
                "[worktree]\nenforce_leases = \"off\"\n\n{}",
                crate::test_taxonomy::test_taxonomy().config_fragment()
            );
            let actual = crate::commands::test_helpers::memory_executor_declaring(
                InMemoryStorage::new(),
                crate::commands::test_helpers::MemoryDeclarations {
                    config: Some(&config),
                    ..Default::default()
                },
            )
            .issues()
            .collect_batch_problems(&case.issues)
            .unwrap()
            .into_iter()
            .filter_map(|problem| match problem {
                BatchValidationProblem::DuplicateKey { .. } => Some("duplicate-key"),
                BatchValidationProblem::UnknownDependency { .. } => Some("unknown-dependency"),
                BatchValidationProblem::Cycle { .. } => Some("cycle"),
                _ => None,
            })
            .map(str::to_string)
            .collect::<Vec<_>>();
            assert_eq!(actual, case.expected, "shared case '{}'", case.name);
        }
    }

    #[test]
    fn test_batch_create_validation_failure_creates_nothing() {
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        );
        let defs = vec![def("a", &[]), def("a", &[])]; // duplicate key
        let err = exec.issues().batch_create_from_json(defs).unwrap_err();
        assert!(err.downcast_ref::<BatchValidationError>().is_some());
        // Zero issues created.
        assert_eq!(exec.storage().list_issues().unwrap().len(), 0);
    }

    #[test]
    fn test_batch_create_unknown_dependency_returns_typed_error_without_writes() {
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        );
        let err = exec
            .issues()
            .batch_create_from_json(vec![def("a", &["ghost"])])
            .unwrap_err();
        let validation = err.downcast_ref::<BatchValidationError>().unwrap();
        assert!(validation.problems.iter().any(|problem| matches!(
            problem,
            BatchValidationProblem::UnknownDependency { key, missing }
                if key == "a" && missing == "ghost"
        )));
        assert!(exec.storage().list_issues().unwrap().is_empty());
    }

    fn typed_def(key: &str, level: u8, deps: &[&str]) -> BatchIssueDef {
        BatchIssueDef {
            r#type: Some(
                crate::test_taxonomy::test_taxonomy()
                    .type_at_level(level)
                    .to_string(),
            ),
            ..def(key, deps)
        }
    }

    #[test]
    fn test_batch_create_key_reference_credits_container_created_in_same_manifest() {
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let rules = format!(
            "[[rules]]\nname = \"story-coverage\"\nwhen = {{ type = \"{}\" }}\n\
             severity = \"error\"\nassert = {{ label-coverage = {{ child-link = \"dependencies\" }} }}\n",
            crate::test_taxonomy::test_taxonomy().type_at_level(3)
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                rules: Some(&rules),
                ..Default::default()
            },
        );
        let mut story = typed_def("story", 3, &["task"]);
        story.description = "## Success Criteria\n\n- [hard] REQ-01: works\n".to_string();
        let mut task = typed_def("task", 4, &[]);
        task.labels = vec!["satisfies:{story.short_id}/REQ-01".to_string()];

        let map = exec
            .issues()
            .batch_create_from_json(vec![story, task])
            .unwrap()
            .as_map();

        let task = exec.storage().load_issue(&map["task"]).unwrap();
        assert!(task
            .labels
            .contains(&format!("satisfies:{}/REQ-01", map["story"].short_id())));
        let report = exec
            .validation()
            .validate_repository_report()
            .unwrap()
            .rule_report();
        assert!(
            !report.findings.iter().any(|f| f.rule == "story-coverage"),
            "{:?}",
            report.findings
        );
    }

    #[test]
    fn test_batch_create_unknown_label_key_returns_typed_error_without_writes() {
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let rules = format!(
            "[[rules]]\nname = \"story-coverage\"\nwhen = {{ type = \"{}\" }}\n\
             severity = \"error\"\nassert = {{ label-coverage = {{ child-link = \"dependencies\" }} }}\n",
            crate::test_taxonomy::test_taxonomy().type_at_level(3)
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                rules: Some(&rules),
                ..Default::default()
            },
        );
        let mut task = def("task", &[]);
        task.labels = vec!["satisfies:{ghost.short_id}/REQ-01".to_string()];
        let err = exec
            .issues()
            .batch_create_from_json(vec![task])
            .unwrap_err();
        let validation = err.downcast_ref::<BatchValidationError>().unwrap();
        assert!(validation.problems.iter().any(|problem| matches!(
            problem,
            BatchValidationProblem::UnknownLabelKey { key, missing, .. }
                if key == "task" && missing == "ghost"
        )));
        assert!(exec.storage().list_issues().unwrap().is_empty());
    }

    #[test]
    fn test_batch_create_cycle_creates_nothing() {
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        );
        let exec = crate::commands::test_helpers::memory_executor_declaring(
            InMemoryStorage::new(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        );
        let defs = vec![def("a", &["b"]), def("b", &["a"])];
        let err = exec.issues().batch_create_from_json(defs).unwrap_err();
        let verr = err.downcast_ref::<BatchValidationError>().unwrap();
        let cycle = verr
            .problems
            .iter()
            .find(|p| matches!(p, BatchValidationProblem::Cycle { .. }))
            .unwrap()
            .to_string();
        assert!(cycle.contains("cycle") && cycle.contains('a') && cycle.contains('b'));
        assert_eq!(exec.storage().list_issues().unwrap().len(), 0);
    }
}
