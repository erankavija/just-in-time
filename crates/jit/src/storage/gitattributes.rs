//! `.gitattributes` merge-driver setup for `jit init`.
//!
//! Owns the git-repository detection and the `.gitattributes` read/append/
//! create for the `.jit/events.jsonl` / `.jit/claims.jsonl` union-merge
//! drivers, so the CLI layer (`main.rs`) only calls [`setup_gitattributes`]
//! and reports its [`GitattributesOutcome`] — it never touches git or the
//! filesystem itself (AGENTS.md "Separation of Concerns"). Mirrors the
//! [`crate::storage::worktree_paths::WorktreePaths::detect`] precedent for
//! git-subprocess-backed storage setup.

use anyhow::Result;
use std::fs;
use std::path::Path;
use std::process::Command;

use super::atomic_write::write_file_atomic;

/// What [`setup_gitattributes`] did to `.gitattributes` this run. Lets the
/// caller (`jit init --json`) report it precisely instead of guessing: a
/// fresh file is a `created_paths` entry, an appended one is a
/// `modified_paths` entry, and an already-configured or absent-git-repo run
/// reports nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitattributesOutcome {
    /// Not in a git repository; nothing done.
    NotGitRepo,
    /// `.gitattributes` did not exist; created with the jit merge-driver block.
    Created,
    /// `.gitattributes` existed without the jit merge-driver block; appended.
    Modified,
    /// `.gitattributes` already had the jit merge-driver block; no-op.
    AlreadyConfigured,
}

/// Set up `.gitattributes` with merge drivers for jit files.
/// Only runs if we're in a git repository.
///
/// # Examples
///
/// ```no_run
/// use jit::storage::gitattributes::setup_gitattributes;
///
/// let outcome = setup_gitattributes().unwrap();
/// println!("{:?}", outcome);
/// ```
pub fn setup_gitattributes() -> Result<GitattributesOutcome> {
    // Check if we're in a git repository
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();

    let repo_root = match output {
        Ok(o) if o.status.success() => String::from_utf8(o.stdout)?.trim().to_string(),
        _ => return Ok(GitattributesOutcome::NotGitRepo),
    };

    let gitattributes_path = Path::new(&repo_root).join(".gitattributes");
    let jit_marker = "# JIT merge drivers";
    let jit_config = format!(
        "{}\n.jit/events.jsonl merge=union\n.jit/claims.jsonl merge=union\n",
        jit_marker
    );

    if gitattributes_path.exists() {
        let content = fs::read_to_string(&gitattributes_path)?;

        // Check if already configured (idempotent)
        if content.contains(jit_marker) {
            return Ok(GitattributesOutcome::AlreadyConfigured);
        }

        // Append to existing file, through the atomic temp-file + rename
        // primitive (@/inv/atomic-writes) so a concurrent reader never observes
        // a partially written file.
        let new_content = if content.ends_with('\n') {
            format!("{}\n{}", content, jit_config)
        } else {
            format!("{}\n\n{}", content, jit_config)
        };
        write_file_atomic(&gitattributes_path, &new_content)?;
        Ok(GitattributesOutcome::Modified)
    } else {
        write_file_atomic(&gitattributes_path, &jit_config)?;
        Ok(GitattributesOutcome::Created)
    }
}
