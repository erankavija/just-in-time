//! Persistence for `config.toml` (the repo `.jit/config.toml` and the
//! user-global `~/.config/jit/config.toml`).
//!
//! Mirrors [`crate::storage::gate_store`] / [`crate::storage::ruleset_store`]:
//! module-level functions that own ALL config-file IO. Callers (the `jit config
//! set` command, `jit init`'s project-identity seeding) produce the in-memory
//! [`toml_edit::DocumentMut`] mutations or the identity value; this module reads
//! and writes the files. Every write goes through the shared atomic writer
//! ([`crate::storage::atomic_write`]), preserving the temp-file + rename
//! invariant (@/inv/atomic-writes), so no config-file path or write primitive
//! leaks into the command/CLI layers.

use crate::config::ProjectName;
use crate::storage::atomic_write::write_file_atomic;
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

/// The config file name, relative to the `.jit` root (repo) or the user config
/// directory (global).
const CONFIG_FILE: &str = "config.toml";

/// The repo config path (`<jit_root>/config.toml`).
///
/// # Examples
///
/// ```
/// use jit::storage::config_store::repo_config_path;
/// use std::path::Path;
///
/// let path = repo_config_path(Path::new("/repo/.jit"));
/// assert!(path.ends_with("config.toml"));
/// ```
pub fn repo_config_path(jit_root: &Path) -> PathBuf {
    jit_root.join(CONFIG_FILE)
}

/// Read the config TOML at `path` into an editable document, preserving comments
/// and formatting. An absent file yields an empty document (the caller then
/// seeds keys into it), matching the tolerant absent-file handling elsewhere in
/// storage.
///
/// # Examples
///
/// ```
/// use jit::storage::config_store::read_config_document;
///
/// let dir = tempfile::tempdir().unwrap();
/// // Absent file -> empty document.
/// let doc = read_config_document(&dir.path().join("config.toml")).unwrap();
/// assert!(doc.as_table().is_empty());
/// ```
pub fn read_config_document(path: &Path) -> Result<toml_edit::DocumentMut> {
    if path.exists() {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", path.display()))?;
        content
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| anyhow!("Failed to parse config: {}", e))
    } else {
        Ok(toml_edit::DocumentMut::new())
    }
}

/// Persist a config document to `path` atomically (temp file + rename),
/// creating the parent directory first when it does not exist (the user-global
/// `~/.config/jit` case on a first write).
///
/// # Examples
///
/// ```
/// use jit::storage::config_store::{read_config_document, save_config_document};
///
/// let dir = tempfile::tempdir().unwrap();
/// let path = dir.path().join("nested/config.toml");
/// let mut doc = read_config_document(&path).unwrap();
/// doc["coordination"]["default_ttl_secs"] = toml_edit::value(3600);
/// save_config_document(&path, &doc).unwrap();
///
/// let reloaded = read_config_document(&path).unwrap();
/// assert_eq!(reloaded["coordination"]["default_ttl_secs"].as_integer(), Some(3600));
/// ```
pub fn save_config_document(path: &Path, doc: &toml_edit::DocumentMut) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }
    write_file_atomic(path, &doc.to_string())
}

/// Seed a fresh repo `config.toml`: the template-generated `base_config_toml`
/// body followed by the `[project]` identity table carrying `project_name`,
/// written atomically.
///
/// This owns the `[project]` block's on-disk format (its guiding comment and the
/// `name = "..."` line) so no config-file construction lives in the command or
/// CLI layers. The command-layer seeding orchestration
/// (`CommandExecutor::seed_project_config`) invokes it only when `config.toml`
/// does not yet exist, keeping `jit init` idempotent (a re-init leaves an
/// existing `[project]` table untouched).
///
/// # Examples
///
/// ```
/// use jit::config::{JitConfig, ProjectName};
/// use jit::storage::config_store::seed_repo_config;
///
/// let dir = tempfile::tempdir().unwrap();
/// let name: ProjectName = "my-project".parse().unwrap();
/// seed_repo_config(dir.path(), "[coordination]\ndefault_ttl_secs = 3600\n", &name).unwrap();
///
/// let config = JitConfig::load(dir.path()).unwrap();
/// assert_eq!(config.project.unwrap().name.unwrap().as_str(), "my-project");
/// ```
pub fn seed_repo_config(
    jit_root: &Path,
    base_config_toml: &str,
    project_name: &ProjectName,
) -> Result<()> {
    // The exact scaffold format: template body, then the project-identity table
    // with its guiding comment. Kept byte-for-byte to preserve the generated
    // file a user first sees after `jit init`.
    let content = format!(
        "{}\n# =============================================================================\n# PROJECT IDENTITY\n# =============================================================================\n# Canonical, human-editable project name: the `@<project>` scope token in the\n# multi-jit addressing scheme. Must match ^[a-z][a-z0-9-]*$. Defaults to a\n# slug of this repository's directory name; edit freely.\n[project]\nname = \"{}\"\n",
        base_config_toml,
        project_name.as_str()
    );
    write_file_atomic(&repo_config_path(jit_root), &content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_config_document_absent_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let doc = read_config_document(&dir.path().join("config.toml")).unwrap();
        assert!(doc.as_table().is_empty());
    }

    #[test]
    fn test_save_then_read_round_trips_and_preserves_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = repo_config_path(dir.path());
        let mut doc = read_config_document(&path).unwrap();
        doc["project"]["name"] = toml_edit::value("renamed-project");
        save_config_document(&path, &doc).unwrap();

        let reloaded = read_config_document(&path).unwrap();
        assert_eq!(
            reloaded["project"]["name"].as_str(),
            Some("renamed-project")
        );
    }

    #[test]
    fn test_save_config_document_creates_missing_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        // Mirrors the user-global first-write case: parent dir absent.
        let path = dir.path().join("config/jit/config.toml");
        let doc = "[worktree]\nenforce_leases = \"off\"\n"
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        save_config_document(&path, &doc).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_seed_repo_config_writes_project_table() {
        let dir = tempfile::tempdir().unwrap();
        let name: ProjectName = "seeded-name".parse().unwrap();
        seed_repo_config(
            dir.path(),
            "[coordination]\ndefault_ttl_secs = 3600\n",
            &name,
        )
        .unwrap();

        let content = std::fs::read_to_string(repo_config_path(dir.path())).unwrap();
        assert!(content.contains("[project]"), "{content}");
        assert!(content.contains("name = \"seeded-name\""), "{content}");
        // The template body precedes the seeded identity table.
        let coord = content.find("default_ttl_secs").unwrap();
        let project = content.find("[project]").unwrap();
        assert!(
            coord < project,
            "template body must precede [project]: {content}"
        );
    }
}
