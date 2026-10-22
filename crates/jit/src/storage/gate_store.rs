//! Loading the gate registry (`.jit/gates.toml`).
//!
//! Authored semantics and pure parsing live in [`crate::declarations`]. This
//! module owns only the `.jit/gates.toml` read boundary and delegates parsing
//! to that neutral owner. Mutation sessions own publication.

use crate::declarations::{parse_gate_registry, GateRegistry};
use anyhow::{Context, Result};
use std::path::Path;

/// The gate registry file, relative to the `.jit` root.
const GATES_FILE: &str = "gates.toml";

/// Load the gate registry from `<jit_root>/gates.toml`.
///
/// An absent file yields an empty registry — `jit init` always scaffolds the
/// file, so this only matters for a hand-deleted store (mirrors the tolerant
/// absent-file handling elsewhere in storage).
///
/// Two `[[gates]]` rows carrying the same `key` are an error: the key is the
/// registry identity (and the `gate` item kind's addressable self-id), and
/// silently keeping one row would discard the other. A parse failure keeps its
/// typed source for exit-code classification and names the offending key.
pub fn load_gate_registry(jit_root: &Path) -> Result<GateRegistry> {
    let path = jit_root.join(GATES_FILE);
    if !path.exists() {
        return Ok(GateRegistry::default());
    }
    let content =
        std::fs::read(&path).with_context(|| format!("Failed to read file: {}", path.display()))?;
    Ok(parse_gate_registry(&content)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_gate_registry_absent_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let registry = load_gate_registry(dir.path()).unwrap();
        assert!(registry.gates.is_empty());
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

[[gates]]
version = 1
key = \"cargo-ci\"
title = \"Second\"
description = \"duplicate key\"
stage = \"postcheck\"
mode = \"manual\"
priority = 100
";
        std::fs::write(dir.path().join("gates.toml"), registry_toml).unwrap();

        let err = load_gate_registry(dir.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("duplicate gate key 'cargo-ci'"),
            "error names the duplicate key: {msg}"
        );
    }

    #[test]
    fn test_load_gate_registry_unknown_key_is_named_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("gates.toml"),
            "[[gates]]\nkey = \"review\"\ntitle = \"Review\"\ndescription = \"Review\"\n\
             stage = \"postcheck\"\nmode = \"auto\"\nundeclared_key = true\n",
        )
        .unwrap();

        let err = load_gate_registry(dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("unknown field `undeclared_key`"),
            "error names the unknown key: {err}"
        );
        assert_eq!(
            crate::output::failure::classify(&err).error.code,
            crate::output::ErrorCode::ParseError
        );
    }
}
