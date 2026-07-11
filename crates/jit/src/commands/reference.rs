//! Rules-and-gates reference projection command (`jit reference render`).
//!
//! `render` projects the effective rule set and the gate registry into the
//! documentation target declared by `[rules_gates_projection]` (default: a
//! separate jit-owned file). It is a thin boundary: it pulls the cached config
//! (projection target/mode/style), the effective ruleset, and the gate registry,
//! then delegates ALL rendering / region-splicing / atomic-write logic to the pure
//! engine ([`project_rules_and_gates`](crate::validation::rules_gates_projection::project_rules_and_gates)).
//! It owns no CLI parsing or output formatting (the layer boundary in AGENTS.md
//! "Separation of Concerns").

use super::*;
use crate::config::RulesGatesProjectionConfig;
use crate::validation::rules_gates_projection::project_rules_and_gates;

/// Result of a `jit reference render` projection.
///
/// Returned by [`CommandExecutor::render_rules_and_gates`] and serialized as the
/// `--json` payload: the repo-relative `target` that was written, the `mode` used
/// (`separate-file`|`region`), and the `rules`/`gates` counts rendered.
///
/// # Examples
///
/// ```
/// use jit::commands::RulesGatesRenderResult;
///
/// // The fields mirror the rendered projection (here built by hand to show the
/// // serialized shape).
/// let result = RulesGatesRenderResult {
///     target: "docs/reference/rules-and-gates.md".to_string(),
///     mode: "region".to_string(),
///     rules: 9,
///     gates: 3,
/// };
/// let json = serde_json::to_value(&result).unwrap();
/// assert_eq!(json["target"], "docs/reference/rules-and-gates.md");
/// assert_eq!(json["mode"], "region");
/// assert_eq!(json["rules"], 9);
/// assert_eq!(json["gates"], 3);
/// ```
#[derive(Debug, Serialize)]
pub struct RulesGatesRenderResult {
    /// The repo-relative documentation target that was written (from config).
    pub target: String,
    /// The projection mode used, as its config token (`separate-file`|`region`).
    pub mode: String,
    /// Number of rules rendered into the target.
    pub rules: usize,
    /// Number of gates rendered into the target.
    pub gates: usize,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Render the effective rule set and gate registry into their configured
    /// documentation target and return what was written.
    ///
    /// Reads the `[rules_gates_projection]` table from the cached config (falling
    /// back to the shipped default — separate-file mode targeting a jit-owned file
    /// — when the table is absent), the
    /// [`effective_rules`](CommandExecutor::effective_rules), and the loaded gate
    /// registry, then delegates to
    /// [`project_rules_and_gates`](crate::validation::rules_gates_projection::project_rules_and_gates),
    /// which path-validates the config-driven target and writes atomically through
    /// the storage boundary. The target path comes ONLY from config.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let result = executor.render_rules_and_gates()?;
    /// println!(
    ///     "wrote {} rule(s) and {} gate(s) to {}",
    ///     result.rules, result.gates, result.target
    /// );
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn render_rules_and_gates(&self) -> Result<RulesGatesRenderResult> {
        let config = self.cached_config()?;
        let default = RulesGatesProjectionConfig::default();
        let projection = config.rules_gates_projection.as_ref().unwrap_or(&default);
        let rules = self.effective_rules()?;
        let gates = self.storage().load_gate_registry()?;

        let target = project_rules_and_gates(self.storage(), projection, rules, &gates)
            .map_err(|err| anyhow!("rules/gates projection failed: {err}"))?;

        Ok(RulesGatesRenderResult {
            target,
            mode: match projection.mode() {
                crate::config::ProjectionMode::SeparateFile => "separate-file".to_string(),
                crate::config::ProjectionMode::Region => "region".to_string(),
            },
            rules: rules.rules.len(),
            gates: gates.gates.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{GateRegistry, InMemoryStorage, IssueStore};

    /// Build an executor over an in-memory `.jit` carrying the given `rules.toml`,
    /// `config.toml`, and a gate registry with `gate_keys`.
    fn exec(
        rules_toml: &str,
        config_toml: &str,
        gate_keys: &[&str],
    ) -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();
        std::fs::write(storage.root().join("rules.toml"), rules_toml).unwrap();
        std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();
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

    const RULES: &str =
        "[[rules]]\nname = \"label-format\"\nseverity = \"error\"\nenforce = true\n\
                         assert = { require-label = { label = \"type:*\" } }\n";

    #[test]
    fn test_render_separate_file_default_reports_counts() {
        // No [rules_gates_projection] table -> the shipped separate-file default.
        let executor = exec(RULES, "", &["cargo-ci", "code-review"]);
        let result = executor.render_rules_and_gates().unwrap();
        assert_eq!(result.target, ".jit/rules-and-gates.md");
        assert_eq!(result.mode, "separate-file");
        assert_eq!(result.rules, 1);
        assert_eq!(result.gates, 2);

        let written = executor
            .storage()
            .read_repo_file(".jit/rules-and-gates.md")
            .unwrap()
            .expect("separate-file target should be written");
        assert!(written.contains("@/rule/label-format"), "{written}");
        assert!(written.contains("@/gate/cargo-ci"));
    }

    #[test]
    fn test_render_result_serializes_to_json() {
        let executor = exec(RULES, "", &["cargo-ci"]);
        let result = executor.render_rules_and_gates().unwrap();
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["rules"], 1);
        assert_eq!(json["gates"], 1);
        assert_eq!(json["mode"], "separate-file");
    }
}
