//! Persistence for the validation ruleset files (`rules.toml` + `schemas/*.json`).
//!
//! The validation layer ([`crate::validation::serialize`]) produces the CONTENT
//! (the `rules.toml` body and the schema file name/content pairs); this storage
//! module owns the on-disk `rules.toml` + `schemas/` layout and performs the
//! writes, so those storage paths live only in the storage layer. All writes go
//! through the shared atomic writer ([`crate::storage::atomic_write`]),
//! preserving the temp-file + rename invariant.

use crate::storage::atomic_write::write_file_atomic;
use anyhow::{Context, Result};
use std::path::Path;

/// The operative validation ruleset file, relative to the `.jit` root.
const RULES_FILE: &str = "rules.toml";
/// The directory (relative to the `.jit` root) holding rule-referenced schemas.
const SCHEMAS_DIR: &str = "schemas";

/// Whether this repository already has a materialized validation ruleset
/// (`<jit_root>/rules.toml`).
///
/// `jit init` scaffolds the default ruleset only when this is `false`, so a
/// user-edited `rules.toml` (the sole source when present) is never clobbered.
///
/// # Examples
///
/// ```
/// use jit::storage::ruleset_store::{has_validation_ruleset, write_validation_ruleset};
///
/// let dir = tempfile::tempdir().unwrap();
/// assert!(!has_validation_ruleset(dir.path()));
/// write_validation_ruleset(dir.path(), "# rules\n", &[]).unwrap();
/// assert!(has_validation_ruleset(dir.path()));
/// ```
pub fn has_validation_ruleset(jit_root: &Path) -> bool {
    jit_root.join(RULES_FILE).exists()
}

/// Persist a serialized validation ruleset: `rules.toml` plus the
/// `schemas/<name>.json` files it references, each written atomically (temp
/// file + rename in the target's directory).
///
/// `schema_files` is a list of `(file_name, content)` pairs produced by the
/// validation serializer; this function owns where they land (`schemas/`).
/// Writing is the LAST step so a reader that observes `rules.toml` also sees the
/// schema files it references.
///
/// # Examples
///
/// ```
/// use jit::storage::ruleset_store::write_validation_ruleset;
///
/// let dir = tempfile::tempdir().unwrap();
/// write_validation_ruleset(
///     dir.path(),
///     "# rules\n",
///     &[("label.json".to_string(), "{}\n".to_string())],
/// )
/// .unwrap();
/// assert_eq!(
///     std::fs::read_to_string(dir.path().join("rules.toml")).unwrap(),
///     "# rules\n"
/// );
/// assert_eq!(
///     std::fs::read_to_string(dir.path().join("schemas/label.json")).unwrap(),
///     "{}\n"
/// );
/// ```
pub fn write_validation_ruleset(
    jit_root: &Path,
    rules_toml: &str,
    schema_files: &[(String, String)],
) -> Result<()> {
    if !schema_files.is_empty() {
        let schemas_dir = jit_root.join(SCHEMAS_DIR);
        std::fs::create_dir_all(&schemas_dir)
            .with_context(|| format!("creating {}", schemas_dir.display()))?;
        for (name, content) in schema_files {
            write_file_atomic(&schemas_dir.join(name), content)?;
        }
    }
    write_file_atomic(&jit_root.join(RULES_FILE), rules_toml)
}

/// Regenerate the single baked schema file `<jit_root>/schemas/<file_name>` from
/// `content`, but only when a materialized `schemas/` layout already exists.
///
/// A `rules.toml`-less repo builds its schemas in memory (the read path) and has
/// no on-disk `schemas/` to keep in sync, so this is a no-op there: it writes
/// only when the target file OR the `schemas/` directory already exists. Returns
/// `true` when a file was written. Idempotent (rewriting current content yields
/// identical bytes) and atomic (temp + rename).
///
/// # Examples
///
/// ```
/// use jit::storage::ruleset_store::write_baked_schema;
///
/// let dir = tempfile::tempdir().unwrap();
/// // No schemas/ layout yet: a no-op that writes nothing.
/// assert!(!write_baked_schema(dir.path(), "known.json", "{}\n").unwrap());
/// std::fs::create_dir_all(dir.path().join("schemas")).unwrap();
/// // Now the materialized layout exists, so the file is written.
/// assert!(write_baked_schema(dir.path(), "known.json", "{}\n").unwrap());
/// ```
pub fn write_baked_schema(jit_root: &Path, file_name: &str, content: &str) -> Result<bool> {
    let schemas_dir = jit_root.join(SCHEMAS_DIR);
    let target = schemas_dir.join(file_name);
    // Only refresh a materialized layout: if neither the file nor its directory
    // exists, the repo has no baked schemas (read path builds them in memory), so
    // there is nothing to keep in sync.
    if !target.exists() && !schemas_dir.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(&schemas_dir)
        .with_context(|| format!("creating {}", schemas_dir.display()))?;
    write_file_atomic(&target, content)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{LabelNamespace, LabelNamespaces};
    use crate::validation::defaults::{default_ruleset, TYPE_HIERARCHY_SCHEMA_FILE};
    use crate::validation::serialize::{serialize_ruleset, type_hierarchy_schema_content};
    use std::collections::HashMap;

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

    /// Build a registry carrying an explicit type hierarchy.
    fn registry_with_hierarchy(types: &[(&str, u8)]) -> LabelNamespaces {
        let mut hierarchy = HashMap::new();
        for (name, level) in types {
            hierarchy.insert(name.to_string(), *level);
        }
        LabelNamespaces {
            schema_version: 2,
            namespaces: HashMap::new(),
            type_hierarchy: Some(hierarchy),
            label_associations: None,
            strategic_types: None,
        }
    }

    /// Materialize the default ruleset to `jit_root` (the `jit init` scaffold:
    /// validation produces content, storage persists it).
    fn scaffold(jit_root: &Path, reg: &LabelNamespaces) {
        let serialized = serialize_ruleset(&default_ruleset(reg));
        let schema_files: Vec<(String, String)> = serialized
            .schema_files
            .into_iter()
            .map(|f| (f.name, f.content))
            .collect();
        write_validation_ruleset(jit_root, &serialized.rules_toml, &schema_files).unwrap();
    }

    /// Regenerate the baked type-hierarchy schema from `reg`.
    fn regenerate(jit_root: &Path, reg: &LabelNamespaces) -> bool {
        write_baked_schema(
            jit_root,
            TYPE_HIERARCHY_SCHEMA_FILE,
            &type_hierarchy_schema_content(reg),
        )
        .unwrap()
    }

    #[test]
    fn test_regenerate_type_hierarchy_schema_writes_declared_types() {
        // After `jit init`, the baked schema exists. Regenerating from a hierarchy
        // that declares a NEW type must include that type in the enum.
        let dir = tempfile::tempdir().unwrap();
        scaffold(dir.path(), &registry(vec![]));
        let target = dir.path().join("schemas").join(TYPE_HIERARCHY_SCHEMA_FILE);
        assert!(target.exists(), "scaffold writes the baked schema");

        let reg = registry_with_hierarchy(&[("epic", 2), ("planning", 3), ("task", 4)]);
        assert!(regenerate(dir.path(), &reg), "existing layout is refreshed");

        let content = std::fs::read_to_string(&target).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();
        let enum_values = &json["properties"]["labels"]["properties"]["type"]["items"]["enum"];
        let names: Vec<&str> = enum_values
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(
            names.contains(&"planning"),
            "declared type present: {names:?}"
        );
        assert!(names.contains(&"epic"));
        assert!(names.contains(&"task"));
    }

    #[test]
    fn test_regenerate_type_hierarchy_schema_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        scaffold(dir.path(), &registry(vec![]));
        let reg = registry_with_hierarchy(&[("epic", 2), ("task", 4)]);
        regenerate(dir.path(), &reg);
        let target = dir.path().join("schemas").join(TYPE_HIERARCHY_SCHEMA_FILE);
        let first = std::fs::read_to_string(&target).unwrap();
        regenerate(dir.path(), &reg);
        let second = std::fs::read_to_string(&target).unwrap();
        assert_eq!(first, second, "regeneration is byte-identical (idempotent)");
    }

    #[test]
    fn test_regenerate_is_noop_without_baked_schemas() {
        // A repo with no `schemas/` dir (read path builds rules in memory) has
        // nothing to refresh: the regenerator is a no-op and creates no file.
        let dir = tempfile::tempdir().unwrap();
        let reg = registry_with_hierarchy(&[("epic", 2)]);
        assert!(!regenerate(dir.path(), &reg), "no baked layout => no write");
        assert!(!dir.path().join("schemas").exists());
    }

    #[test]
    fn test_regenerated_schema_matches_scaffolded_default() {
        // The regenerator and the `jit init` scaffold must produce the SAME baked
        // type-hierarchy schema for the same hierarchy (one source, not two: R5).
        let reg = registry_with_hierarchy(&[("epic", 2), ("planning", 3), ("task", 4)]);

        let scaffold_dir = tempfile::tempdir().unwrap();
        scaffold(scaffold_dir.path(), &reg);
        let scaffolded = std::fs::read_to_string(
            scaffold_dir
                .path()
                .join("schemas")
                .join(TYPE_HIERARCHY_SCHEMA_FILE),
        )
        .unwrap();

        let regen_dir = tempfile::tempdir().unwrap();
        // Seed an empty-default baked layout, then regenerate from the same hierarchy.
        scaffold(regen_dir.path(), &registry(vec![]));
        regenerate(regen_dir.path(), &reg);
        let regenerated = std::fs::read_to_string(
            regen_dir
                .path()
                .join("schemas")
                .join(TYPE_HIERARCHY_SCHEMA_FILE),
        )
        .unwrap();

        assert_eq!(scaffolded, regenerated, "scaffold and regen must agree");
    }
}
