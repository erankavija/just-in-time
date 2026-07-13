//! Persistence for the gate registry (`.jit/gates.toml`).
//!
//! Mirrors [`crate::storage::ruleset_store`]'s atomic-write pattern: this store
//! is the sole source of truth for the gate registry, replacing the prior
//! JSON-based gate registry file. `Gate` already derives `Serialize`/`Deserialize` (unlike
//! [`Rule`](crate::validation::rules::Rule), which needed a hand-rolled
//! renderer), so this module serializes/deserializes it directly through a
//! thin `[[gates]]` array-of-tables wrapper — the same shape
//! `rules.toml`/`invariants.toml` use (`RawRulesFile`/`RawInvariantsFile`'s
//! `Vec<T>` pattern) — converting to/from [`GateRegistry`]'s in-memory
//! `HashMap<String, Gate>` at this boundary. All writes go through the shared
//! atomic writer ([`crate::storage::atomic_write`]), preserving the
//! temp-file + rename invariant.
//!
//! # TOML cannot carry a JSON `null`
//!
//! `Gate.reserved` is a `HashMap<String, serde_json::Value>` and may hold a
//! `serde_json::Value::Null` entry (round-tripped from JSON-authored gates).
//! TOML has no `null`, and the `toml` crate errors when asked to serialize
//! one — unlike a top-level `Option::None` field (e.g. `Gate.checker`,
//! `GateChecker::Exec::working_dir`), which the toml serializer omits
//! transparently. [`save_gate_registry`] strips null-valued `reserved` entries
//! before writing so a gate carrying one still persists successfully.

use crate::domain::Gate;
use crate::storage::atomic_write::write_file_atomic;
use crate::storage::GateRegistry;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The gate registry file, relative to the `.jit` root.
const GATES_FILE: &str = "gates.toml";

/// On-disk shape of `.jit/gates.toml`: a `[[gates]]` array-of-tables, mirroring
/// `RawRulesFile`/`RawInvariantsFile`'s `Vec<T>` wrapper pattern.
#[derive(Debug, Default, Serialize, Deserialize)]
struct GatesFile {
    #[serde(default)]
    gates: Vec<Gate>,
}

/// Load the gate registry from `<jit_root>/gates.toml`.
///
/// An absent file yields an empty registry — `jit init` always scaffolds the
/// file, so this only matters for a hand-deleted store (mirrors the tolerant
/// absent-file handling elsewhere in storage).
///
/// Two `[[gates]]` rows carrying the same `key` are an error: the key is the
/// registry identity (and the `gate` item kind's addressable self-id), and
/// silently keeping one row would drop the other on the next save.
pub fn load_gate_registry(jit_root: &Path) -> Result<GateRegistry> {
    let path = jit_root.join(GATES_FILE);
    if !path.exists() {
        return Ok(GateRegistry::default());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;
    let file: GatesFile =
        toml::from_str(&content).context("Failed to deserialize gate registry")?;
    let mut gates = std::collections::HashMap::with_capacity(file.gates.len());
    for gate in file.gates {
        let key = gate.key.clone();
        if gates.insert(key.clone(), gate).is_some() {
            return Err(anyhow::anyhow!(
                "duplicate gate key '{key}' in {}: the key is the registry identity; \
                 merge or rename the duplicate [[gates]] entry",
                path.display()
            ));
        }
    }
    Ok(GateRegistry { gates })
}

/// Persist the gate registry to `<jit_root>/gates.toml` as a `[[gates]]`
/// array-of-tables, written atomically (temp file + rename).
///
/// Gates are ordered by key for a deterministic, diff-friendly file. Any
/// null-valued `reserved` entry is stripped first (see module docs) so a gate
/// carrying one still writes successfully.
///
/// # Examples
///
/// ```
/// use jit::domain::{Gate, GateMode, GateStage};
/// use jit::storage::gate_store::{load_gate_registry, save_gate_registry};
/// use jit::storage::GateRegistry;
/// use std::collections::HashMap;
///
/// let dir = tempfile::tempdir().unwrap();
/// let gate = Gate {
///     version: 1,
///     key: "review".to_string(),
///     title: "Code Review".to_string(),
///     description: "Manual code review".to_string(),
///     stage: GateStage::Postcheck,
///     mode: GateMode::Manual,
///     checker: None,
///     priority: 100,
///     reserved: HashMap::new(),
///     auto: false,
///     example_integration: None,
/// };
/// let mut registry = GateRegistry::default();
/// registry.gates.insert(gate.key.clone(), gate);
/// save_gate_registry(dir.path(), &registry).unwrap();
///
/// let loaded = load_gate_registry(dir.path()).unwrap();
/// assert_eq!(loaded.gates["review"].title, "Code Review");
/// assert!(
///     std::fs::read_to_string(dir.path().join("gates.toml"))
///         .unwrap()
///         .starts_with("[[gates]]"),
///     "gates persist as an array-of-tables, not a keyed [gates.<key>] table"
/// );
/// ```
pub fn save_gate_registry(jit_root: &Path, registry: &GateRegistry) -> Result<()> {
    let mut gates: Vec<Gate> = registry.gates.values().cloned().collect();
    gates.sort_by(|a, b| a.key.cmp(&b.key));
    for gate in &mut gates {
        gate.reserved.retain(|_, value| !value.is_null());
    }
    let file = GatesFile { gates };
    let toml_str = toml::to_string_pretty(&file).context("Failed to serialize gate registry")?;
    write_file_atomic(&jit_root.join(GATES_FILE), &toml_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{GateMode, GateStage};
    use std::collections::HashMap;

    fn sample_gate(key: &str) -> Gate {
        Gate {
            version: 1,
            key: key.to_string(),
            title: format!("{key} title"),
            description: format!("{key} description"),
            stage: GateStage::Postcheck,
            mode: GateMode::Manual,
            checker: None,
            priority: 100,
            reserved: HashMap::new(),
            auto: false,
            example_integration: None,
        }
    }

    #[test]
    fn test_load_gate_registry_absent_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let registry = load_gate_registry(dir.path()).unwrap();
        assert!(registry.gates.is_empty());
    }

    #[test]
    fn test_save_then_load_round_trips_full_exec_checker_schema() {
        // Exercise every GateChecker::Exec field (REQ-01) plus the deprecated
        // `auto`/`example_integration` fields, through a real save+load cycle.
        let dir = tempfile::tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert("KEY".to_string(), "value".to_string());

        let gate = Gate {
            version: 2,
            key: "clippy".to_string(),
            title: "Clippy".to_string(),
            description: "Lints".to_string(),
            stage: GateStage::Precheck,
            mode: GateMode::Auto,
            checker: Some(crate::domain::GateChecker::Exec {
                command: "cargo clippy --workspace --all-targets -- -D warnings".to_string(),
                timeout_seconds: 300,
                working_dir: Some("crates/jit".to_string()),
                env,
                pass_context: true,
                prompt: Some("check for lint regressions".to_string()),
                prompt_file: Some("prompts/clippy.md".to_string()),
            }),
            priority: 10,
            reserved: HashMap::new(),
            auto: true,
            example_integration: Some("ci.yml".to_string()),
        };

        let mut registry = GateRegistry::default();
        registry.gates.insert(gate.key.clone(), gate.clone());
        save_gate_registry(dir.path(), &registry).unwrap();

        let loaded = load_gate_registry(dir.path()).unwrap();
        assert_eq!(loaded.gates.len(), 1);
        assert_eq!(loaded.gates["clippy"], gate);
    }

    #[test]
    fn test_save_gate_registry_writes_array_of_tables_not_keyed_table() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = GateRegistry::default();
        let gate = sample_gate("tests");
        registry.gates.insert(gate.key.clone(), gate);
        save_gate_registry(dir.path(), &registry).unwrap();

        let content = std::fs::read_to_string(dir.path().join("gates.toml")).unwrap();
        assert!(
            content.contains("[[gates]]"),
            "expected an array-of-tables header, got:\n{content}"
        );
        assert!(
            !content.contains("[gates.tests]"),
            "must not persist as a keyed [gates.<key>] table:\n{content}"
        );
    }

    #[test]
    fn test_save_gate_registry_strips_null_reserved_entry() {
        // REQ-03: `Gate.reserved` may carry a `serde_json::Value::Null` (e.g.
        // round-tripped from a JSON-authored gate). TOML cannot represent it, so
        // the store must strip it rather than fail the write.
        let dir = tempfile::tempdir().unwrap();
        let mut gate = sample_gate("null-guard");
        gate.reserved
            .insert("future_field".to_string(), serde_json::Value::Null);
        gate.reserved.insert(
            "keep_me".to_string(),
            serde_json::Value::String("kept".to_string()),
        );

        let mut registry = GateRegistry::default();
        registry.gates.insert(gate.key.clone(), gate);

        save_gate_registry(dir.path(), &registry)
            .expect("a null-valued reserved entry must not fail the write");

        let loaded = load_gate_registry(dir.path()).unwrap();
        let loaded_gate = &loaded.gates["null-guard"];
        assert!(
            !loaded_gate.reserved.contains_key("future_field"),
            "null-valued entry must be stripped: {:?}",
            loaded_gate.reserved
        );
        assert_eq!(
            loaded_gate.reserved.get("keep_me"),
            Some(&serde_json::Value::String("kept".to_string())),
            "non-null entries must survive the round trip"
        );
    }

    #[test]
    fn test_save_gate_registry_orders_gates_by_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = GateRegistry::default();
        for key in ["zeta", "alpha", "mid"] {
            let gate = sample_gate(key);
            registry.gates.insert(gate.key.clone(), gate);
        }
        save_gate_registry(dir.path(), &registry).unwrap();

        let content = std::fs::read_to_string(dir.path().join("gates.toml")).unwrap();
        let alpha_pos = content.find("key = \"alpha\"").unwrap();
        let mid_pos = content.find("key = \"mid\"").unwrap();
        let zeta_pos = content.find("key = \"zeta\"").unwrap();
        assert!(alpha_pos < mid_pos && mid_pos < zeta_pos, "{content}");
    }

    #[test]
    fn test_load_gate_registry_rejects_duplicate_keys() {
        let dir = tempfile::tempdir().unwrap();
        let registry_toml = "\
[[gates]]
version = 1
key = \"cargo-ci\"
title = \"First\"
description = \"first entry\"
stage = \"postcheck\"
mode = \"manual\"
priority = 100
auto = false

[[gates]]
version = 1
key = \"cargo-ci\"
title = \"Second\"
description = \"duplicate key\"
stage = \"postcheck\"
mode = \"manual\"
priority = 100
auto = false
";
        std::fs::write(dir.path().join("gates.toml"), registry_toml).unwrap();

        let err = load_gate_registry(dir.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("duplicate gate key 'cargo-ci'"),
            "error names the duplicate key: {msg}"
        );
    }
}
