//! Persistence for the user-global `~/.config/jit/config.toml` control plane.
//!
//! Repository-owned configuration is published only through
//! [`RepositoryStateStore`](crate::storage::RepositoryStateStore). This module's
//! typed root capability prevents its atomic writer from accepting a repository
//! path (or any other caller-selected target).

use crate::storage::atomic_write::write_file_atomic;
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

const CONFIG_FILE: &str = "config.toml";

/// Capability for the directory containing one user's global JIT config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserConfigRoot {
    path: PathBuf,
}

impl UserConfigRoot {
    /// Derive the JIT user-config directory from a user's home directory.
    pub fn from_home(home: &Path) -> Self {
        Self {
            path: home.join(".config/jit"),
        }
    }

    /// The sole config document governed by this capability.
    pub fn config_path(&self) -> PathBuf {
        self.path.join(CONFIG_FILE)
    }
}

/// Read the user-global config into an editable, formatting-preserving document.
///
/// An absent document yields an empty value so the first `config set --global`
/// can create it.
pub fn read_user_config_document(root: &UserConfigRoot) -> Result<toml_edit::DocumentMut> {
    let path = root.config_path();
    if path.exists() {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read user config: {}", path.display()))?;
        content
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| anyhow!("Failed to parse user config: {error}"))
    } else {
        Ok(toml_edit::DocumentMut::new())
    }
}

/// Atomically persist the user-global config governed by `root`.
pub fn save_user_config_document(
    root: &UserConfigRoot,
    document: &toml_edit::DocumentMut,
) -> Result<()> {
    std::fs::create_dir_all(&root.path).with_context(|| {
        format!(
            "Failed to create user config directory: {}",
            root.path.display()
        )
    })?;
    write_file_atomic(&root.config_path(), &document.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_config_round_trip_creates_fixed_config_path() {
        let home = tempfile::tempdir().unwrap();
        let root = UserConfigRoot::from_home(home.path());
        let mut document = read_user_config_document(&root).unwrap();
        document["worktree"]["enforce_leases"] = toml_edit::value("off");

        save_user_config_document(&root, &document).unwrap();

        assert_eq!(
            root.config_path(),
            home.path().join(".config/jit/config.toml")
        );
        assert_eq!(
            read_user_config_document(&root).unwrap()["worktree"]["enforce_leases"].as_str(),
            Some("off")
        );
    }

    #[test]
    fn test_read_user_config_document_absent_file_is_empty() {
        let home = tempfile::tempdir().unwrap();
        let document = read_user_config_document(&UserConfigRoot::from_home(home.path())).unwrap();
        assert!(document.as_table().is_empty());
    }
}
