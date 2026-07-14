//! Config accessor and mutation commands (`jit config get` / `jit config set`).
//!
//! `jit config set` (see [`CommandExecutor::set_config`]): target-file
//! resolution (repo vs user-global), key parsing, per-type value dispatch, and
//! typed validation of the incoming value (notably `project.name` through
//! [`ProjectName`], REQ-03 of the multi-jit story: an invalid identity must
//! never reach the file). ALL config-file IO — reading the TOML document and
//! the atomic write — is delegated to [`crate::storage::config_store`], so no
//! persistence lives in this command module (the layer boundary in AGENTS.md
//! "Separation of Concerns").
//!
//! `jit config get` (see [`CommandExecutor::get_config`]): a dotted-key
//! accessor covering the WHOLE configuration surface (jit:043ae624), built by
//! walking [`EffectiveConfig::full_snapshot`]'s generic JSON snapshot with
//! [`resolve_dotted_key`] rather than hand-mapping each recognised key —
//! adding a field to a config section needs no change here, only to that
//! section's own `Serialize` derive.
//!
//! The CLI layer only parses args and formats the returned
//! [`ConfigSetOutcome`] / [`ConfigGetOutcome`].

use super::*;
use crate::config::{EffectiveConfig, ProjectName};
use crate::errors::InvalidArgumentError;
use crate::storage::config_store;
use std::path::{Path, PathBuf};

/// Result of a `jit config set` write, carrying what the CLI needs to print.
///
/// Returned by [`CommandExecutor::set_config`]. `file` is the config file that
/// was written and `scope` is its origin token (`"user"` for a global write,
/// `"repo"` otherwise), mirroring the tokens the `--json` payload reports.
#[derive(Debug, Serialize)]
pub struct ConfigSetOutcome {
    /// The `section.field` key that was set.
    pub key: String,
    /// The value as written (the raw string the user supplied).
    pub value: String,
    /// The config file that was written.
    pub file: PathBuf,
    /// Config scope token: `"user"` for a global write, `"repo"` otherwise.
    pub scope: &'static str,
}

/// Result of a `jit config get` lookup, carrying what the CLI needs to print.
///
/// Returned by [`CommandExecutor::get_config`]. `value` is the raw
/// [`serde_json::Value`] resolved at `key` — a scalar for a leaf, or an
/// object/array for an intermediate key (`jit config get documentation`
/// returns the whole section).
#[derive(Debug, Serialize)]
pub struct ConfigGetOutcome {
    /// The dotted key that was resolved.
    pub key: String,
    /// The resolved value.
    pub value: serde_json::Value,
}

/// Error resolving a dotted `jit config get` key against an assembled
/// configuration JSON snapshot ([`resolve_dotted_key`]).
///
/// Kept independent of IO and of [`anyhow::Error`] so [`resolve_dotted_key`]
/// stays a pure, directly unit-tested function. [`CommandExecutor::get_config`]
/// converts either variant's message into the shared
/// [`InvalidArgumentError`] (exit 2) — the same family every other CLI usage
/// error in this codebase uses — rather than introducing a parallel
/// classification path.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigKeyError {
    /// The first (top-level) path segment is not a section of the
    /// configuration surface. `sections` lists what IS valid, computed from
    /// the snapshot itself (never a separately hand-maintained list, so it
    /// can't drift out of sync with it), sorted for determinism.
    #[error("unknown config key '{key}'; valid top-level sections: {sections}")]
    UnknownSection {
        /// The full dotted key the caller asked for.
        key: String,
        /// Comma-separated, sorted list of valid top-level section names.
        sections: String,
    },
    /// A later path segment does not exist under its parent — whether the
    /// parent is a config section's field or a user-declared map (e.g. a
    /// namespace or item-kind name) — or the parent is a scalar/array with
    /// nothing further to descend into.
    #[error("unknown config key '{key}': no '{segment}' under '{parent}'")]
    UnknownField {
        /// The full dotted key the caller asked for.
        key: String,
        /// The dotted path of the parent that was successfully resolved.
        parent: String,
        /// The segment that does not exist under `parent`.
        segment: String,
    },
}

/// Walk a dotted key path (e.g. `documentation.development_root`,
/// `type_hierarchy.types.epic`, `namespaces.type.unique`) against an
/// already-assembled configuration JSON snapshot, returning the resolved
/// value.
///
/// Pure and total over any [`serde_json::Value`] object tree — no IO, no
/// knowledge of [`JitConfig`](crate::config::JitConfig)'s actual shape — so it
/// is unit-tested directly against hand-built snapshots, decoupled from
/// [`EffectiveConfig::full_snapshot`]'s IO-heavy assembly. An intermediate key
/// (fewer segments than the value's depth) returns the WHOLE subtree at that
/// point, not an error.
///
/// # Examples
///
/// ```
/// use jit::commands::resolve_dotted_key;
/// use serde_json::json;
///
/// let snapshot = json!({
///     "documentation": {"development_root": "dev"},
///     "type_hierarchy": {"types": {"epic": 2}},
/// });
///
/// assert_eq!(
///     resolve_dotted_key(&snapshot, "documentation.development_root").unwrap(),
///     json!("dev")
/// );
/// // An intermediate key returns the whole subtree, not an error.
/// assert_eq!(
///     resolve_dotted_key(&snapshot, "documentation").unwrap(),
///     json!({"development_root": "dev"})
/// );
/// // An unknown top-level key lists the sections that ARE valid.
/// let err = resolve_dotted_key(&snapshot, "bogus").unwrap_err();
/// assert!(err.to_string().contains("documentation, type_hierarchy"));
/// ```
pub fn resolve_dotted_key(
    snapshot: &serde_json::Value,
    key: &str,
) -> std::result::Result<serde_json::Value, ConfigKeyError> {
    let mut segments = key.split('.');
    let first = segments.next().unwrap_or("");
    let Some(mut current) = snapshot.get(first) else {
        let mut sections: Vec<&str> = snapshot
            .as_object()
            .map(|obj| obj.keys().map(String::as_str).collect())
            .unwrap_or_default();
        sections.sort_unstable();
        return Err(ConfigKeyError::UnknownSection {
            key: key.to_string(),
            sections: sections.join(", "),
        });
    };
    let mut resolved_path = first.to_string();
    for segment in segments {
        let Some(next) = current.get(segment) else {
            return Err(ConfigKeyError::UnknownField {
                key: key.to_string(),
                parent: resolved_path,
                segment: segment.to_string(),
            });
        };
        current = next;
        resolved_path.push('.');
        resolved_path.push_str(segment);
    }
    Ok(current.clone())
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Set a `section.field` key in the repo (or, with `global`, the user-global)
    /// `config.toml`, returning what the CLI needs to print.
    ///
    /// Resolves the target file, reads the existing TOML document (or starts an
    /// empty one) and writes it back through [`crate::storage::config_store`] —
    /// this command owns no file IO. `value` is parsed into the expected type for
    /// the key BEFORE the write: `project.name` is validated through
    /// [`ProjectName`] so an invalid identity is rejected and the file is left
    /// untouched (REQ-03). Numeric (`*_secs`/`*_pct`/`max_*`) and boolean
    /// (`enable_*`/`require_*`/`auto_*`) keys are parsed into their TOML types;
    /// any other key is stored as a string.
    pub fn set_config(&self, key: &str, value: &str, global: bool) -> Result<ConfigSetOutcome> {
        // Determine the target config file (path derivation only; the store owns
        // the read/write IO).
        let config_path = if global {
            let home =
                dirs::home_dir().ok_or_else(|| anyhow!("Could not determine home directory"))?;
            home.join(".config/jit").join("config.toml")
        } else {
            config_store::repo_config_path(self.storage.root())
        };

        // Load the existing config or start an empty document (through storage).
        let mut doc = config_store::read_config_document(&config_path)?;

        // Parse the key into section.field.
        let parts: Vec<&str> = key.split('.').collect();
        if parts.len() != 2 {
            anyhow::bail!("Config key must be in format 'section.field' (e.g., coordination.default_ttl_secs)");
        }
        let section = parts[0];
        let field = parts[1];

        // Ensure the section exists.
        if doc.get(section).is_none() {
            doc[section] = toml_edit::Item::Table(toml_edit::Table::new());
        }

        // Parse and set the value based on the expected type.
        let parsed_value: toml_edit::Item = match key {
            k if k.ends_with("_secs") || k.ends_with("_pct") || k.contains("max_") => {
                let num: i64 = value
                    .parse()
                    .map_err(|_| anyhow!("Expected numeric value for {}", key))?;
                toml_edit::value(num)
            }
            k if k.contains("enable_") || k.contains("require_") || k.contains("auto_") => {
                let b: bool = value
                    .parse()
                    .map_err(|_| anyhow!("Expected boolean (true/false) for {}", key))?;
                toml_edit::value(b)
            }
            _ => toml_edit::value(value),
        };

        doc[section][field] = parsed_value;

        // REQ-03: the canonical `[project].name` is validated on EVERY document
        // write, not only when it is the key being set. Validating the
        // POST-mutation document makes a valid replacement
        // (`config set project.name good-name`) the repair path, while an
        // unrelated set over a document that already holds an invalid name is
        // rejected. The typed error names the offending value, and nothing is
        // persisted before this check so a rejected write leaves the file
        // untouched.
        if let Some(name) = doc
            .get("project")
            .and_then(|project| project.get("name"))
            .and_then(|name| name.as_str())
        {
            let _validated: ProjectName = name.parse()?;
        }

        // Reject an unrecognized `[validation].strictness` eagerly, on the SAME
        // post-mutation document, so an invalid level never persists. This mirrors
        // the load-time `deserialize_strictness` guard: `config set
        // validation.strictness <valid>` is the repair path for a file that
        // already holds a bad value, while any set over a document carrying an
        // invalid strictness is rejected. Nothing is written before this check.
        if let Some(strictness) = doc
            .get("validation")
            .and_then(|validation| validation.get("strictness"))
            .and_then(|value| value.as_str())
        {
            let _validated: crate::validation::Strictness = strictness.parse()?;
        }

        // Persist through storage (atomic write, parent dir ensured for the
        // user-global first-write case).
        config_store::save_config_document(&config_path, &doc)?;

        // A repo `config.toml` write republishes the default-schema projections
        // from the (possibly changed) registry, keeping `schemas/default-*.json`
        // current for external consumers. The user-global config carries no repo
        // registry, so it never touches these files. A no-op when the repo has no
        // materialized `schemas/` layout.
        if !global {
            self.refresh_default_schema_projections()?;
        }

        Ok(ConfigSetOutcome {
            key: key.to_string(),
            value: value.to_string(),
            file: config_path,
            scope: if global { "user" } else { "repo" },
        })
    }

    /// Seed the repo `[project]` identity into a freshly-initialized `.jit`
    /// (REQ-01), returning the seeded name (or `None` when it was a no-op).
    ///
    /// Owns the seeding orchestration: the config-existence check, the
    /// default-name computation (slugifying `repo_dir`'s basename via
    /// [`slugify_project_name`](crate::config::slugify_project_name)), and the
    /// call to [`crate::storage::config_store::seed_repo_config`], which appends
    /// the `[project]` table to the template-generated `base_config_toml` body
    /// and writes it atomically. Idempotent: when `config.toml` already exists it
    /// is a no-op returning `Ok(None)`, so a re-init never disturbs an existing
    /// `[project]` table.
    pub fn seed_project_config(
        &self,
        repo_dir: &Path,
        base_config_toml: &str,
    ) -> Result<Option<ProjectName>> {
        let jit_root = self.storage.root();

        // Idempotent: an existing config.toml keeps its `[project]` table, so a
        // re-init never reseeds (REQ-03 preserved for free).
        if config_store::repo_config_path(jit_root).exists() {
            return Ok(None);
        }

        let dir_basename = repo_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let project_name: ProjectName =
            crate::config::slugify_project_name(dir_basename).parse()?;
        config_store::seed_repo_config(jit_root, base_config_toml, &project_name)?;
        Ok(Some(project_name))
    }

    /// Resolve a dotted `section[.field[.subfield...]]` key against the WHOLE
    /// configuration surface (REQ-01 of jit:043ae624), returning the value the
    /// CLI needs to print.
    ///
    /// Loads the same system/user/repo-layered [`EffectiveConfig`] `jit
    /// config show` uses via [`EffectiveConfig::load`] (which owns all the
    /// filesystem probing — this command touches no `std::fs` / `dirs`
    /// itself), snapshots it once via [`EffectiveConfig::full_snapshot`], then
    /// walks `key` with [`resolve_dotted_key`]. An unknown key (top-level or
    /// nested) surfaces as the shared [`InvalidArgumentError`] (exit 2) —
    /// naming the valid top-level sections for an unknown top-level key —
    /// rather than a bare `anyhow!` string, so `--json` callers get a
    /// machine-readable `INVALID_ARGUMENT` error and plain callers get exit
    /// code 2. A load/parse failure (e.g. a malformed `config.toml`) is left
    /// unconverted, so it is classified the same way every other config-load
    /// failure in this codebase is, not misreported as a bad argument.
    pub fn get_config(&self, key: &str) -> Result<ConfigGetOutcome> {
        // All filesystem interaction (system/user/repo existence probing,
        // home-directory resolution, each source's `JitConfig::load`) lives
        // behind `EffectiveConfig::load` in the config layer — this command
        // only orchestrates the already-assembled snapshot and the key walk.
        let effective = EffectiveConfig::load(self.storage.root())?;
        let snapshot = effective.full_snapshot()?;
        let value = resolve_dotted_key(&snapshot, key)
            .map_err(|e| anyhow::Error::from(InvalidArgumentError::new(e.to_string())))?;

        Ok(ConfigGetOutcome {
            key: key.to_string(),
            value,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_dotted_key_scalar_leaf() {
        let snapshot = serde_json::json!({
            "documentation": {"development_root": "dev"},
        });
        assert_eq!(
            resolve_dotted_key(&snapshot, "documentation.development_root").unwrap(),
            serde_json::json!("dev")
        );
    }

    #[test]
    fn test_resolve_dotted_key_array_leaf() {
        let snapshot = serde_json::json!({
            "documentation": {"managed_paths": ["dev/active", "dev/studies"]},
        });
        assert_eq!(
            resolve_dotted_key(&snapshot, "documentation.managed_paths").unwrap(),
            serde_json::json!(["dev/active", "dev/studies"])
        );
    }

    #[test]
    fn test_resolve_dotted_key_deeply_nested_map_entry() {
        let snapshot = serde_json::json!({
            "namespaces": {"type": {"description": "Issue type", "unique": true}},
        });
        assert_eq!(
            resolve_dotted_key(&snapshot, "namespaces.type.unique").unwrap(),
            serde_json::json!(true)
        );
    }

    #[test]
    fn test_resolve_dotted_key_intermediate_returns_whole_subtree() {
        let snapshot = serde_json::json!({
            "documentation": {"development_root": "dev", "archive_root": "dev/archive"},
        });
        assert_eq!(
            resolve_dotted_key(&snapshot, "documentation").unwrap(),
            serde_json::json!({"development_root": "dev", "archive_root": "dev/archive"})
        );
    }

    #[test]
    fn test_resolve_dotted_key_unknown_top_level_lists_sections() {
        let snapshot = serde_json::json!({"documentation": {}, "validation": {}});
        let err = resolve_dotted_key(&snapshot, "bogus").unwrap_err();
        assert_eq!(
            err,
            ConfigKeyError::UnknownSection {
                key: "bogus".to_string(),
                sections: "documentation, validation".to_string(),
            }
        );
        assert!(err.to_string().contains("documentation, validation"));
    }

    #[test]
    fn test_resolve_dotted_key_unknown_nested_field() {
        let snapshot = serde_json::json!({"documentation": {"development_root": "dev"}});
        let err = resolve_dotted_key(&snapshot, "documentation.bogus").unwrap_err();
        assert_eq!(
            err,
            ConfigKeyError::UnknownField {
                key: "documentation.bogus".to_string(),
                parent: "documentation".to_string(),
                segment: "bogus".to_string(),
            }
        );
    }

    #[test]
    fn test_resolve_dotted_key_cannot_descend_past_scalar() {
        let snapshot = serde_json::json!({"version": {"schema": 1}});
        let err = resolve_dotted_key(&snapshot, "version.schema.extra").unwrap_err();
        assert_eq!(
            err,
            ConfigKeyError::UnknownField {
                key: "version.schema.extra".to_string(),
                parent: "version.schema".to_string(),
                segment: "extra".to_string(),
            }
        );
    }

    #[test]
    fn test_get_config_covers_whole_surface_on_fresh_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let storage = crate::storage::JsonFileStorage::new(dir.path());
        let executor = CommandExecutor::new(storage);

        // No config.toml at all: every top-level section still resolves
        // (empty object), it's just not an unknown key.
        for section in [
            "version",
            "project",
            "type_hierarchy",
            "validation",
            "documentation",
            "namespaces",
            "item_kinds",
            "invariant_projection",
            "rules_gates_projection",
            "worktree",
            "coordination",
            "global_operations",
            "locks",
            "events",
        ] {
            let outcome = executor.get_config(section).unwrap();
            assert!(
                outcome.value.is_object(),
                "section {section} did not resolve to an object: {:?}",
                outcome.value
            );
        }

        // Defaulted merged-config leaf still resolves without any config.toml.
        let ttl = executor
            .get_config("coordination.default_ttl_secs")
            .unwrap();
        assert_eq!(ttl.value, serde_json::json!(600));
    }

    #[test]
    fn test_get_config_reads_whole_surface_from_repo_config_toml() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            r#"
[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4 }
strategic_types = ["milestone", "epic"]

[type_hierarchy.label_associations]
epic = "epic"

[documentation]
development_root = "dev"
managed_paths = ["dev/active"]

[namespaces.type]
description = "Issue type"
unique = true

[validation]
strictness = "loose"
default_type = "task"

[version]
schema = 1
"#,
        )
        .unwrap();
        let storage = crate::storage::JsonFileStorage::new(dir.path());
        let executor = CommandExecutor::new(storage);

        assert_eq!(
            executor
                .get_config("type_hierarchy.strategic_types")
                .unwrap()
                .value,
            serde_json::json!(["milestone", "epic"])
        );
        assert_eq!(
            executor
                .get_config("type_hierarchy.types.epic")
                .unwrap()
                .value,
            serde_json::json!(2)
        );
        assert_eq!(
            executor
                .get_config("documentation.development_root")
                .unwrap()
                .value,
            serde_json::json!("dev")
        );
        assert_eq!(
            executor.get_config("namespaces.type.unique").unwrap().value,
            serde_json::json!(true)
        );
        assert_eq!(
            executor.get_config("validation.strictness").unwrap().value,
            serde_json::json!("loose")
        );
        assert_eq!(
            executor.get_config("version.schema").unwrap().value,
            serde_json::json!(1)
        );
    }

    #[test]
    fn test_set_config_rejects_invalid_strictness() {
        // F1: `jit config set validation.strictness <invalid>` must be rejected
        // eagerly, and must not overwrite a previously-valid persisted value.
        let dir = tempfile::TempDir::new().unwrap();
        let executor = CommandExecutor::new(crate::storage::JsonFileStorage::new(dir.path()));

        // A recognized level is accepted and persisted.
        executor
            .set_config("validation.strictness", "strict", false)
            .unwrap();
        assert_eq!(
            executor.get_config("validation.strictness").unwrap().value,
            serde_json::json!("strict")
        );

        // An unrecognized level is rejected with a message naming the bad value.
        let err = executor
            .set_config("validation.strictness", "banana", false)
            .unwrap_err();
        assert!(err.to_string().contains("banana"), "{err}");

        // The rejected set persisted nothing: the good value survives on disk.
        assert_eq!(
            executor.get_config("validation.strictness").unwrap().value,
            serde_json::json!("strict")
        );
    }

    #[test]
    fn test_get_config_unknown_top_level_key_is_invalid_argument() {
        let dir = tempfile::TempDir::new().unwrap();
        let storage = crate::storage::JsonFileStorage::new(dir.path());
        let executor = CommandExecutor::new(storage);

        let err = executor.get_config("bogus_section").unwrap_err();
        assert!(err.downcast_ref::<InvalidArgumentError>().is_some());
        assert!(err.to_string().contains("valid top-level sections"));
    }
}
