//! Invariant projection commands (`jit invariant render`).
//!
//! `render` projects the loaded `.jit/invariants.toml` registry into the
//! documentation target declared by `[invariant_projection]` (default: a
//! separate jit-owned file). This module is a thin boundary: it pulls the cached
//! config (registry + projection target) and delegates ALL rendering and
//! persistence to the pure/storage-backed engine in
//! [`projection`](crate::validation::projection). It owns no CLI parsing or
//! output formatting (the layer boundary in CLAUDE.md "Separation of Concerns").
//!
//! The drift `check` verb is delivered by the sibling issue; only `render` ships
//! here.

use super::*;
use crate::config::InvariantProjectionConfig;
use crate::validation::projection::project_invariants;

/// Result of a `jit invariant render` projection.
#[derive(Debug, Serialize)]
pub struct InvariantRenderResult {
    /// The repo-relative documentation target that was written (from config).
    pub target: String,
    /// The projection mode used, as its config token (`separate-file`|`region`).
    pub mode: String,
    /// Number of invariants rendered into the target.
    pub count: usize,
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
}
