//! Invariant projection and drift commands (`jit invariant render` / `check`).
//!
//! `render` projects the loaded `.jit/invariants.toml` registry into the
//! documentation target declared by `[invariant_projection]` (default: a
//! separate jit-owned file). `check` computes the
//! enforcement-drift between the invariant registry and the declared
//! rules/gates. Both are thin boundaries: they pull the cached config (registry +
//! projection target) plus the effective ruleset and gate registry, and delegate
//! ALL rendering / drift logic to the pure engine
//! ([`projection`](crate::validation::projection),
//! [`drift`](crate::validation::drift)). They own no CLI parsing or output
//! formatting (the layer boundary in CLAUDE.md "Separation of Concerns").

use super::*;
use crate::config::InvariantProjectionConfig;
use crate::validation::drift::DriftFinding;
use crate::validation::projection::project_invariants;

/// Result of a `jit invariant render` projection.
///
/// Returned by [`CommandExecutor::render_invariants`] and serialized as the
/// `--json` payload: the repo-relative `target` that was written, the `mode` used
/// (`separate-file`|`region`), and the `count` of invariants rendered.
///
/// # Examples
///
/// ```
/// use jit::commands::InvariantRenderResult;
///
/// // The fields mirror the rendered projection (here built by hand to show the
/// // serialized shape).
/// let result = InvariantRenderResult {
///     target: ".jit/invariants.md".to_string(),
///     mode: "separate-file".to_string(),
///     count: 2,
/// };
/// let json = serde_json::to_value(&result).unwrap();
/// assert_eq!(json["target"], ".jit/invariants.md");
/// assert_eq!(json["mode"], "separate-file");
/// assert_eq!(json["count"], 2);
/// ```
#[derive(Debug, Serialize)]
pub struct InvariantRenderResult {
    /// The repo-relative documentation target that was written (from config).
    pub target: String,
    /// The projection mode used, as its config token (`separate-file`|`region`).
    pub mode: String,
    /// Number of invariants rendered into the target.
    pub count: usize,
}

/// Result of a `jit invariant check` enforcement-drift run.
///
/// Returned by [`CommandExecutor::check_invariants`] and serialized as the
/// `--json` payload: the list of [`DriftFinding`]s (each carrying the offending
/// invariant id and the dangling subject) and the total `count`. An empty
/// `findings` list means the registry and the declared rules/gates are
/// consistent.
///
/// # Examples
///
/// ```
/// use jit::commands::InvariantCheckResult;
/// use jit::validation::drift::DriftFinding;
///
/// // The shape mirrors the computed drift (built by hand to show the JSON).
/// let result = InvariantCheckResult {
///     findings: vec![DriftFinding {
///         invariant_id: "INV-01".to_string(),
///         subject: "ghost-rule".to_string(),
///         unloadable: false,
///     }],
///     count: 1,
/// };
/// assert!(result.has_drift());
/// let json = serde_json::to_value(&result).unwrap();
/// assert_eq!(json["count"], 1);
/// assert_eq!(json["findings"][0]["invariant_id"], "INV-01");
/// assert_eq!(json["findings"][0]["subject"], "ghost-rule");
/// ```
#[derive(Debug, Serialize)]
pub struct InvariantCheckResult {
    /// Every drift finding (declared-but-unenforced, in the invariants' authored
    /// order).
    pub findings: Vec<DriftFinding>,
    /// The number of drift findings (mirrors `findings.len()`).
    pub count: usize,
}

impl InvariantCheckResult {
    /// Whether any enforcement drift was found (the caller exits non-zero iff so).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::commands::InvariantCheckResult;
    ///
    /// let clean = InvariantCheckResult { findings: vec![], count: 0 };
    /// assert!(!clean.has_drift());
    /// ```
    pub fn has_drift(&self) -> bool {
        !self.findings.is_empty()
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Render the loaded invariant registry into its configured documentation
    /// target and return what was written.
    ///
    /// Reads the `[invariant_projection]` table and the `.jit/invariants.toml`
    /// registry from the cached config (falling back to the shipped default —
    /// separate-file mode targeting a jit-owned file — when the table is absent),
    /// then delegates to
    /// [`project_invariants`](crate::validation::projection::project_invariants),
    /// which path-validates the config-driven target and writes atomically
    /// through the storage boundary. The target path comes ONLY from config.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let result = executor.render_invariants()?;
    /// println!("wrote {} invariant(s) to {}", result.count, result.target);
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn render_invariants(&self) -> Result<InvariantRenderResult> {
        let config = self.cached_config()?;
        let default = InvariantProjectionConfig::default();
        let projection = config.invariant_projection.as_ref().unwrap_or(&default);
        let registry = &config.invariants;

        let target = project_invariants(self.storage(), projection, registry)
            .map_err(|err| anyhow!("invariant projection failed: {err}"))?;

        Ok(InvariantRenderResult {
            target,
            mode: match projection.mode() {
                crate::config::ProjectionMode::SeparateFile => "separate-file".to_string(),
                crate::config::ProjectionMode::Region => "region".to_string(),
            },
            count: registry.invariants.len(),
        })
    }

    /// Compute the enforcement-drift between the invariant registry and the
    /// declared rules/gates, returning every drift finding.
    ///
    /// Resolves the three inputs at this boundary — the invariant registry (from
    /// the cached config), the loadable rule names (from
    /// [`effective_rules`](crate::commands::CommandExecutor::effective_rules)),
    /// and the known gate keys (from the gate registry) — and delegates to the
    /// pure [`enforcement_drift`](crate::validation::drift::enforcement_drift)
    /// core. This computes the SAME drift the built-in `jit validate` pass
    /// ([`enforcement_drift_findings`](crate::commands::CommandExecutor::enforcement_drift_findings))
    /// reports — the sole declared-but-unenforced direction — and exits non-zero
    /// on ANY drift. A genuine `rules.toml` load failure surfaces as an `Err`
    /// rather than silently reporting no drift.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let result = executor.check_invariants()?;
    /// if result.has_drift() {
    ///     eprintln!("{} enforcement-drift finding(s)", result.count);
    /// }
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn check_invariants(&self) -> Result<InvariantCheckResult> {
        // Share the SINGLE tolerant drift computation with the built-in validate
        // pass so both surfaces report identically, including the unloadable-source
        // case (REQ-01 "missing OR unloadable"): a malformed `.jit/rules.toml`
        // referenced by an `enforced-by` binding yields a declared-but-unenforced
        // finding here too, not a raw parse error.
        let findings: Vec<DriftFinding> = self.compute_drift_findings()?;
        Ok(InvariantCheckResult {
            count: findings.len(),
            findings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{GateRegistry, InMemoryStorage, IssueStore};

    /// Build an executor over an in-memory `.jit` carrying the given
    /// `invariants.toml`, `rules.toml`, and a gate registry with `gate_keys`.
    fn exec(
        invariants_toml: &str,
        rules_toml: &str,
        gate_keys: &[&str],
    ) -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();
        std::fs::write(storage.root().join("invariants.toml"), invariants_toml).unwrap();
        std::fs::write(storage.root().join("rules.toml"), rules_toml).unwrap();
        let mut registry = GateRegistry::default();
        for key in gate_keys {
            registry.gates.insert(
                (*key).to_string(),
                crate::domain::Gate {
                    version: 1,
                    key: (*key).to_string(),
                    title: (*key).to_string(),
                    description: String::new(),
                    stage: crate::domain::GateStage::Postcheck,
                    mode: crate::domain::GateMode::Manual,
                    checker: None,
                    priority: 100,
                    reserved: std::collections::HashMap::new(),
                    auto: false,
                    example_integration: None,
                },
            );
        }
        storage.save_gate_registry(&registry).unwrap();
        CommandExecutor::new(storage)
    }

    #[test]
    fn test_check_reports_declared_but_unenforced() {
        // INV-01 binds to a rule/gate that does not exist -> the sole drift.
        let inv = "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                   enforced-by = \"ghost-rule\"\n";
        let rules = "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
                     assert = { require-section = { heading = \"Goal\" } }\n";
        let executor = exec(inv, rules, &[]);
        let result = executor.check_invariants().unwrap();
        assert!(result.has_drift());
        assert_eq!(result.findings.len(), 1, "{:?}", result.findings);
        assert_eq!(result.findings[0].invariant_id, "INV-01");
        assert_eq!(result.findings[0].subject, "ghost-rule");
    }

    #[test]
    fn test_check_clean_with_unclaimed_rules_and_gates() {
        // A rule and a gate that NO invariant claims no longer drift (REQ-05): the
        // single resolving binding leaves the check clean despite the unclaimed
        // `real-rule` and `code-review` gate.
        let inv = "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                   enforced-by = \"real-rule\"\n";
        let rules = "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
                     assert = { require-section = { heading = \"Goal\" } }\n";
        let executor = exec(inv, rules, &["code-review"]);
        let result = executor.check_invariants().unwrap();
        assert!(!result.has_drift(), "{:?}", result.findings);
        assert_eq!(result.count, 0);
    }

    #[test]
    fn test_check_clean_when_consistent() {
        // Two invariants claim exactly the one rule and the one gate present.
        let inv = "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                   enforced-by = \"real-rule\"\n\
                   [[invariants]]\nid = \"INV-02\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                   enforced-by = \"code-review\"\n";
        let rules = "[[rules]]\nname = \"real-rule\"\nseverity = \"warn\"\n\
                     assert = { require-section = { heading = \"Goal\" } }\n";
        let executor = exec(inv, rules, &["code-review"]);
        let result = executor.check_invariants().unwrap();
        assert!(!result.has_drift(), "{:?}", result.findings);
        assert_eq!(result.count, 0);
    }

    #[test]
    fn test_check_result_serializes_to_json() {
        // INV-01's `ghost` binding dangles -> one declared-but-unenforced finding;
        // the unclaimed real rule `other` is NOT drift (REQ-05).
        let inv = "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n\
                   enforced-by = \"ghost\"\n";
        let rules = "[[rules]]\nname = \"other\"\nseverity = \"warn\"\n\
                     assert = { require-section = { heading = \"Goal\" } }\n";
        let executor = exec(inv, rules, &[]);
        let result = executor.check_invariants().unwrap();
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["count"], result.count);
        let findings = json["findings"].as_array().unwrap();
        assert_eq!(findings.len(), 1, "{json}");
        assert_eq!(findings[0]["invariant_id"], "INV-01");
        assert_eq!(findings[0]["subject"], "ghost");
    }
}
