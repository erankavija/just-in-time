//! JSON file-based storage implementation.
//!
//! All data is stored as JSON files in a `.jit/` directory with atomic writes.
//! The directory location can be overridden with the `JIT_DATA_DIR` environment variable.

use crate::domain::{Event, Issue};
use crate::storage::{FileLocker, GateRegistry, IssueStore};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const ISSUES_DIR: &str = "issues";
const INDEX_FILE: &str = "index.json";
const GATES_FILE: &str = "gates.json";
const EVENTS_FILE: &str = "events.jsonl";

/// Index of all issues in the repository
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Index {
    /// Schema version for future migrations
    schema_version: u32,
    /// List of all issue IDs
    all_ids: Vec<String>,
    /// List of deleted issue IDs (schema v2+)
    #[serde(default)]
    deleted_ids: Vec<String>,
}

impl Default for Index {
    fn default() -> Self {
        Self {
            schema_version: 2,
            all_ids: Vec::new(),
            deleted_ids: Vec::new(),
        }
    }
}

/// JSON file-based storage for issues, gates, and events.
///
/// This implementation stores each issue as a separate JSON file in `.jit/issues/`,
/// gate definitions in `.jit/gates.json`, and events in `.jit/events.jsonl`.
/// All file writes are atomic (write to temp file, then rename).
///
/// File locking is used to prevent race conditions in concurrent access:
/// - Index updates are protected with exclusive locks
/// - Individual issue updates use per-file locks
/// - Gate registry and event log use exclusive locks for writes
#[derive(Clone)]
pub struct JsonFileStorage {
    root: PathBuf,
    locker: FileLocker,
}

impl JsonFileStorage {
    /// Create a new JSON file storage instance at the given root path.
    /// The root should be the `.jit` directory (or custom directory from JIT_DATA_DIR).
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        let timeout = std::env::var("JIT_LOCK_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(5));

        Self {
            root: root.as_ref().to_path_buf(),
            locker: FileLocker::new(timeout),
        }
    }

    /// Check if the storage directory exists and is initialized.
    /// Returns an error with a helpful message if not.
    pub fn validate(&self) -> Result<()> {
        if !self.root.exists() {
            anyhow::bail!(
                "JIT repository not found at '{}'\n\n\
                 Initialize a repository with: jit init\n\
                 Or set JIT_DATA_DIR environment variable to point to an existing repository.",
                self.root.display()
            );
        }

        let index_path = self.root.join(INDEX_FILE);
        if !index_path.exists() {
            anyhow::bail!(
                "JIT repository at '{}' is not properly initialized (missing index.json)\n\n\
                 Initialize with: jit init",
                self.root.display()
            );
        }

        Ok(())
    }

    fn issue_path(&self, id: &str) -> PathBuf {
        self.root.join(ISSUES_DIR).join(format!("{}.json", id))
    }

    fn write_json<T: Serialize>(&self, path: &Path, data: &T) -> Result<()> {
        let json = serde_json::to_string_pretty(data).context("Failed to serialize data")?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create parent directory")?;
        }

        // Atomic write: write to temp file, then rename
        let temp_path = path.with_extension("json.tmp");
        fs::write(&temp_path, json).context("Failed to write temporary file")?;
        fs::rename(&temp_path, path).context("Failed to rename temporary file")?;

        Ok(())
    }

    fn read_json<T: for<'de> Deserialize<'de>>(&self, path: &Path) -> Result<T> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", path.display()))?;
        serde_json::from_str(&contents).context("Failed to deserialize data")
    }

    fn load_index(&self) -> Result<Index> {
        let index_path = self.root.join(INDEX_FILE);
        self.read_json(&index_path)
    }

    fn save_index(&self, index: &Index) -> Result<()> {
        let index_path = self.root.join(INDEX_FILE);
        self.write_json(&index_path, index)
    }

    /// Load aggregated index from all sources (local + git + main worktree).
    ///
    /// This aggregates issue IDs from:
    /// 1. Local .jit/index.json
    /// 2. Git HEAD:.jit/index.json (if in git)
    /// 3. Main worktree .jit/index.json (if in secondary worktree)
    ///
    /// Deduplicates IDs across sources.
    fn load_aggregated_index(&self) -> Result<Index> {
        use std::collections::HashSet;

        let mut all_ids = HashSet::new();
        let mut deleted_ids = HashSet::new();

        // 1. Load local index
        if let Ok(local_index) = self.load_index() {
            all_ids.extend(local_index.all_ids);
            deleted_ids.extend(local_index.deleted_ids);
        }

        // 2. Try loading index from git
        if let Ok(git_index) = self.load_index_from_git() {
            all_ids.extend(git_index.all_ids);
            deleted_ids.extend(git_index.deleted_ids);
        }

        // 3. Try loading index from main worktree
        if let Ok(main_index) = self.load_index_from_main_worktree() {
            all_ids.extend(main_index.all_ids);
            deleted_ids.extend(main_index.deleted_ids);
        }

        // Filter out deleted IDs from the aggregated set
        all_ids.retain(|id| !deleted_ids.contains(id));

        Ok(Index {
            schema_version: 2,
            all_ids: all_ids.into_iter().collect(),
            deleted_ids: vec![], // Don't propagate deleted_ids in aggregated index
        })
    }

    /// Load index from git HEAD.
    fn load_index_from_git(&self) -> Result<Index> {
        // Run git from repository root, not from .jit directory
        let repo_root = self
            .root
            .parent()
            .ok_or_else(|| anyhow!("Invalid .jit path"))?;

        let output = Command::new("git")
            .arg("show")
            .arg("HEAD:.jit/index.json")
            .current_dir(repo_root)
            .output()
            .context("Failed to execute git command")?;

        if !output.status.success() {
            bail!("Index not in git");
        }

        serde_json::from_slice(&output.stdout).context("Failed to parse index from git")
    }

    /// Load index from main worktree.
    fn load_index_from_main_worktree(&self) -> Result<Index> {
        let repo_root = self
            .root
            .parent()
            .ok_or_else(|| anyhow!("Invalid .jit path"))?;

        let output = Command::new("git")
            .args(["rev-parse", "--git-common-dir"])
            .current_dir(repo_root)
            .output();

        if output.is_err() || !output.as_ref().unwrap().status.success() {
            bail!("Not in a git repository");
        }

        let common_dir = PathBuf::from(String::from_utf8(output.unwrap().stdout)?.trim());

        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(repo_root)
            .output()?;

        if !output.status.success() {
            bail!("Failed to get worktree root");
        }

        let worktree_root = PathBuf::from(String::from_utf8(output.stdout)?.trim());

        // Check if we're in main worktree
        let is_main = common_dir == worktree_root.join(".git");
        if is_main {
            bail!("Already in main worktree");
        }

        let main_worktree_root = if common_dir.file_name().unwrap() == ".git" {
            common_dir.parent().unwrap().to_path_buf()
        } else {
            bail!("Cannot determine main worktree location");
        };

        let main_index_path = main_worktree_root.join(".jit/index.json");

        if !main_index_path.exists() {
            bail!("Index not in main worktree");
        }

        self.read_json(&main_index_path)
    }

    /// Load an issue from git HEAD.
    ///
    /// This is used as a fallback when an issue doesn't exist in local storage
    /// but may be committed in git (e.g., reading from a secondary worktree).
    fn load_issue_from_git(&self, id: &str) -> Result<Issue> {
        // Run git from repository root, not from .jit directory
        let repo_root = self
            .root
            .parent()
            .ok_or_else(|| anyhow!("Invalid .jit path"))?;

        let git_path = format!("HEAD:.jit/issues/{}.json", id);
        let output = Command::new("git")
            .arg("show")
            .arg(&git_path)
            .current_dir(repo_root)
            .output()
            .context("Failed to execute git command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Issue not in git: {}", stderr.trim());
        }

        serde_json::from_slice(&output.stdout).context("Failed to parse issue from git")
    }

    /// Load an issue from the main worktree's .jit/ directory.
    ///
    /// This is used when in a secondary worktree to read uncommitted issues
    /// from the main worktree.
    fn load_issue_from_main_worktree(&self, id: &str) -> Result<Issue> {
        // self.root is .jit/, we need to go up one level to the repo root
        let repo_root = self
            .root
            .parent()
            .ok_or_else(|| anyhow!("Invalid .jit path"))?;

        // We need to detect worktree context from git commands
        // First check if we're in a git repo at all
        let output = Command::new("git")
            .args(["rev-parse", "--git-common-dir"])
            .current_dir(repo_root)
            .output();

        if output.is_err() {
            bail!("Not in a git repository");
        }

        let output = output.unwrap();
        if !output.status.success() {
            bail!("Not in a git repository");
        }

        let common_dir = PathBuf::from(String::from_utf8(output.stdout)?.trim());

        // Get worktree root
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(repo_root)
            .output()?;

        if !output.status.success() {
            bail!("Failed to get worktree root");
        }

        let worktree_root = PathBuf::from(String::from_utf8(output.stdout)?.trim());

        // Check if we're in main worktree
        let is_main = common_dir == worktree_root.join(".git");
        if is_main {
            bail!("Already in main worktree");
        }

        // Calculate main worktree path
        // The common_dir (.git) is shared, so we need to find the main worktree root
        // Main worktree is typically the parent of the .git directory
        let main_worktree_root = if common_dir.file_name().unwrap() == ".git" {
            common_dir.parent().unwrap().to_path_buf()
        } else {
            // Bare repo or non-standard setup
            bail!("Cannot determine main worktree location");
        };

        let main_issue_path = main_worktree_root
            .join(".jit/issues")
            .join(format!("{}.json", id));

        if !main_issue_path.exists() {
            bail!("Issue not in main worktree");
        }

        // Read directly from main worktree (no lock needed for read-only access)
        self.read_json(&main_issue_path)
    }

    /// Check if the current worktree is a secondary git worktree.
    ///
    /// Returns true if this is a secondary worktree, false if main worktree or not in git.
    pub fn is_secondary_worktree(&self) -> bool {
        let repo_root = match self.root.parent() {
            Some(root) => root,
            None => return false,
        };

        // Check if .git exists
        let git_path = repo_root.join(".git");
        if !git_path.exists() {
            return false;
        }

        // Secondary worktrees have .git as a file (pointing to worktree metadata)
        // Main worktree has .git as a directory
        git_path.is_file()
    }
}

impl IssueStore for JsonFileStorage {
    fn init(&self) -> Result<()> {
        let issues_dir = self.root.join(ISSUES_DIR);

        fs::create_dir_all(&issues_dir).context("Failed to create issues directory")?;

        // Create index.json if it doesn't exist
        let index_path = self.root.join(INDEX_FILE);
        if !index_path.exists() {
            let index = Index::default();
            self.write_json(&index_path, &index)?;
        }

        // Create gates.json if it doesn't exist
        let gates_path = self.root.join(GATES_FILE);
        if !gates_path.exists() {
            let registry = GateRegistry::default();
            self.write_json(&gates_path, &registry)?;
        }

        // Create events.jsonl if it doesn't exist
        let events_path = self.root.join(EVENTS_FILE);
        if !events_path.exists() {
            fs::File::create(&events_path).context("Failed to create events file")?;
        }

        // Config.toml is managed by humans, not auto-created
        // Use ConfigManager to access configuration

        Ok(())
    }

    fn save_issue(&self, issue: &Issue) -> Result<()> {
        let issue_path = self.issue_path(&issue.id);
        let index_lock_path = self.root.join(".index.lock");
        let issue_lock_path = issue_path.with_extension("lock");

        // Lock order: index first (to prevent deadlock), then issue
        // Use separate .lock files to avoid conflicts with atomic writes
        let _index_lock = self.locker.lock_exclusive(&index_lock_path)?;
        let mut index = self.load_index()?;
        let needs_index_update = !index.all_ids.contains(&issue.id);

        // Lock the issue (exclusive) and write
        let _issue_lock = self.locker.lock_exclusive(&issue_lock_path)?;
        self.write_json(&issue_path, issue)?;

        // Update index if this is a new issue
        if needs_index_update {
            index.all_ids.push(issue.id.clone());
            self.save_index(&index)?;
        }

        Ok(())
    }

    fn load_issue(&self, id: &str) -> Result<Issue> {
        // Try local .jit/issues/ first (current behavior)
        let issue_path = self.issue_path(id);
        if issue_path.exists() {
            let issue_lock_path = issue_path.with_extension("lock");
            let _lock = self.locker.lock_shared(&issue_lock_path)?;
            return self.read_json(&issue_path);
        }

        // Fallback 1: Try reading from git HEAD
        if let Ok(issue) = self.load_issue_from_git(id) {
            return Ok(issue);
        }

        // Fallback 2: Try reading from main worktree (if in secondary)
        if let Ok(issue) = self.load_issue_from_main_worktree(id) {
            return Ok(issue);
        }

        // Issue not found in any source
        Err(anyhow!(
            "Issue {} not found in local storage, git, or main worktree",
            id
        ))
    }

    fn resolve_issue_id(&self, partial_id: &str) -> Result<String> {
        // Normalize input: lowercase and remove hyphens
        let normalized = partial_id.to_lowercase().replace('-', "");

        // Full UUID check (fast path) - 32 hex chars without hyphens
        if normalized.len() == 32 {
            // Try loading to verify it exists
            return self
                .load_issue(partial_id)
                .map(|issue| issue.id)
                .map_err(|_| anyhow!("Issue not found: {}", partial_id));
        }

        // Minimum length check
        if normalized.len() < 4 {
            return Err(anyhow!("Issue ID prefix must be at least 4 characters"));
        }

        // Load aggregated index to search across all sources
        let index = self.load_aggregated_index()?;
        let matches: Vec<String> = index
            .all_ids
            .iter()
            .filter(|id| id.replace('-', "").to_lowercase().starts_with(&normalized))
            .cloned()
            .collect();

        match matches.len() {
            0 => Err(anyhow!("Issue not found: {}", partial_id)),
            1 => Ok(matches[0].clone()),
            _ => {
                // Load issue titles for better error message
                let issue_list: Vec<String> = matches
                    .iter()
                    .filter_map(|id| {
                        self.load_issue(id)
                            .ok()
                            .map(|issue| format!("{} | {}", issue.short_id(), issue.title))
                    })
                    .collect();
                Err(anyhow!(
                    "Ambiguous ID '{}' matches multiple issues:\n  {}",
                    partial_id,
                    issue_list.join("\n  ")
                ))
            }
        }
    }

    fn delete_issue(&self, id: &str) -> Result<()> {
        let issue_path = self.issue_path(id);
        let index_lock_path = self.root.join(".index.lock");
        let issue_lock_path = issue_path.with_extension("lock");

        // Lock in order: index first, then issue
        let _index_lock = self.locker.lock_exclusive(&index_lock_path)?;
        let _issue_lock = self.locker.lock_exclusive(&issue_lock_path)?;

        fs::remove_file(&issue_path).context("Failed to delete issue file")?;
        // Clean up lock file too
        let _ = fs::remove_file(&issue_lock_path);

        // Update index: remove from all_ids and add to deleted_ids
        let mut index = self.load_index()?;
        index.all_ids.retain(|i| i != id);

        // Add to deleted_ids if not already there (idempotent)
        if !index.deleted_ids.contains(&id.to_string()) {
            index.deleted_ids.push(id.to_string());
        }

        self.save_index(&index)?;

        Ok(())
    }

    fn list_issues(&self) -> Result<Vec<Issue>> {
        let index_lock_path = self.root.join(".index.lock");
        let _lock = self.locker.lock_shared(&index_lock_path)?;

        // Use aggregated index to see all issues across sources
        let index = self.load_aggregated_index()?;

        // Load issues using fallback chain (load_issue handles local/git/main)
        let issues = index
            .all_ids
            .iter()
            .filter_map(|id| {
                // Use load_issue which has fallback logic
                self.load_issue(id).ok()
            })
            .collect();

        Ok(issues)
    }

    fn load_gate_registry(&self) -> Result<GateRegistry> {
        let gates_lock_path = self.root.join(".gates.lock");
        let _lock = self.locker.lock_shared(&gates_lock_path)?;
        let gates_path = self.root.join(GATES_FILE);
        self.read_json(&gates_path)
    }

    fn save_gate_registry(&self, registry: &GateRegistry) -> Result<()> {
        let gates_lock_path = self.root.join(".gates.lock");
        let _lock = self.locker.lock_exclusive(&gates_lock_path)?;
        let gates_path = self.root.join(GATES_FILE);
        self.write_json(&gates_path, registry)
    }

    fn append_event(&self, event: &Event) -> Result<()> {
        let events_lock_path = self.root.join(".events.lock");
        let _lock = self.locker.lock_exclusive(&events_lock_path)?;

        let events_path = self.root.join(EVENTS_FILE);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&events_path)
            .context("Failed to open events file")?;

        let json = serde_json::to_string(event).context("Failed to serialize event")?;
        writeln!(file, "{}", json).context("Failed to write event")?;
        Ok(())
    }

    fn read_events(&self) -> Result<Vec<Event>> {
        let events_path = self.root.join(EVENTS_FILE);
        if !events_path.exists() {
            return Ok(Vec::new());
        }

        let events_lock_path = self.root.join(".events.lock");
        let _lock = self.locker.lock_shared(&events_lock_path)?;

        let file = fs::File::open(&events_path).context("Failed to open events file")?;
        let reader = BufReader::new(file);

        let mut events = Vec::new();
        for line in reader.lines() {
            let line = line.context("Failed to read line from events file")?;
            if line.trim().is_empty() {
                continue;
            }
            let event: Event =
                serde_json::from_str(&line).context("Failed to deserialize event")?;
            events.push(event);
        }

        Ok(events)
    }

    fn save_gate_run_result(&self, result: &crate::domain::GateRunResult) -> Result<()> {
        // Create gate-runs directory if it doesn't exist
        let gate_runs_dir = self.root.join("gate-runs");
        fs::create_dir_all(&gate_runs_dir).context("Failed to create gate-runs directory")?;

        // Create directory for this run
        let run_dir = gate_runs_dir.join(&result.run_id);
        fs::create_dir_all(&run_dir).context("Failed to create run directory")?;

        // Save result.json
        let result_path = run_dir.join("result.json");
        let json =
            serde_json::to_string_pretty(result).context("Failed to serialize gate run result")?;
        fs::write(&result_path, json).context("Failed to write gate run result")?;

        Ok(())
    }

    fn load_gate_run_result(&self, run_id: &str) -> Result<crate::domain::GateRunResult> {
        let result_path = self.root.join("gate-runs").join(run_id).join("result.json");

        if !result_path.exists() {
            anyhow::bail!("Gate run '{}' not found", run_id);
        }

        let contents =
            fs::read_to_string(&result_path).context("Failed to read gate run result")?;
        let result =
            serde_json::from_str(&contents).context("Failed to deserialize gate run result")?;

        Ok(result)
    }

    fn list_gate_runs_for_issue(
        &self,
        issue_id: &str,
    ) -> Result<Vec<crate::domain::GateRunResult>> {
        let gate_runs_dir = self.root.join("gate-runs");

        if !gate_runs_dir.exists() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        for entry in fs::read_dir(&gate_runs_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let result_path = entry.path().join("result.json");
                if result_path.exists() {
                    let contents = fs::read_to_string(&result_path)?;
                    if let Ok(result) =
                        serde_json::from_str::<crate::domain::GateRunResult>(&contents)
                    {
                        if result.issue_id == issue_id {
                            results.push(result);
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn list_gate_presets(&self) -> Result<Vec<crate::gate_presets::PresetInfo>> {
        let manager = crate::gate_presets::PresetManager::new(self.root.clone())?;
        Ok(manager.list_presets())
    }

    fn get_gate_preset(&self, name: &str) -> Result<crate::gate_presets::GatePresetDefinition> {
        let manager = crate::gate_presets::PresetManager::new(self.root.clone())?;
        let preset = manager.get_preset(name)?;
        Ok(preset.clone())
    }

    fn save_gate_preset(
        &self,
        preset: &crate::gate_presets::GatePresetDefinition,
    ) -> Result<std::path::PathBuf> {
        use std::fs;

        // Validate preset
        preset.validate()?;

        // Create presets directory if needed
        let presets_dir = self.root.join("config").join("gate-presets");
        fs::create_dir_all(&presets_dir)?;

        // Save preset
        let preset_path = presets_dir.join(format!("{}.json", preset.name));
        let json = serde_json::to_string_pretty(&preset)?;
        fs::write(&preset_path, json)?;

        Ok(preset_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Gate;
    use crate::storage::IssueStore;
    use tempfile::TempDir;

    fn setup_storage() -> (TempDir, JsonFileStorage) {
        let temp_dir = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp_dir.path());
        (temp_dir, storage)
    }

    #[test]
    fn test_init_creates_directory_structure() {
        let (_temp, storage) = setup_storage();

        storage.init().unwrap();

        assert!(storage.root.join(ISSUES_DIR).exists());
        assert!(storage.root.join(INDEX_FILE).exists());
        assert!(storage.root.join(GATES_FILE).exists());
    }

    #[test]
    fn test_init_is_idempotent() {
        let (_temp, storage) = setup_storage();

        storage.init().unwrap();
        storage.init().unwrap();

        assert!(storage.root.join(ISSUES_DIR).exists());
    }

    #[test]
    fn test_save_and_load_issue() {
        let (_temp, storage) = setup_storage();
        storage.init().unwrap();

        let issue = Issue::new("Test Issue".to_string(), "Description".to_string());
        let issue_id = issue.id.clone();

        storage.save_issue(&issue).unwrap();
        let loaded = storage.load_issue(&issue_id).unwrap();

        assert_eq!(loaded.id, issue.id);
        assert_eq!(loaded.title, issue.title);
        assert_eq!(loaded.description, issue.description);
    }

    #[test]
    fn test_save_issue_updates_index() {
        let (_temp, storage) = setup_storage();
        storage.init().unwrap();

        let issue = Issue::new("Test".to_string(), "Desc".to_string());
        storage.save_issue(&issue).unwrap();

        let index = storage.load_index().unwrap();
        assert!(index.all_ids.contains(&issue.id));
    }

    #[test]
    fn test_save_issue_twice_doesnt_duplicate_in_index() {
        let (_temp, storage) = setup_storage();
        storage.init().unwrap();

        let mut issue = Issue::new("Test".to_string(), "Desc".to_string());
        storage.save_issue(&issue).unwrap();

        issue.title = "Updated".to_string();
        storage.save_issue(&issue).unwrap();

        let index = storage.load_index().unwrap();
        assert_eq!(
            index.all_ids.iter().filter(|id| *id == &issue.id).count(),
            1
        );
    }

    #[test]
    fn test_list_issues_returns_all_issues() {
        let (_temp, storage) = setup_storage();
        storage.init().unwrap();

        let issue1 = Issue::new("Issue 1".to_string(), "Desc 1".to_string());
        let issue2 = Issue::new("Issue 2".to_string(), "Desc 2".to_string());

        storage.save_issue(&issue1).unwrap();
        storage.save_issue(&issue2).unwrap();

        let issues = storage.list_issues().unwrap();
        assert_eq!(issues.len(), 2);
        assert!(issues.iter().any(|i| i.id == issue1.id));
        assert!(issues.iter().any(|i| i.id == issue2.id));
    }

    #[test]
    fn test_delete_issue_removes_file_and_updates_index() {
        let (_temp, storage) = setup_storage();
        storage.init().unwrap();

        let issue = Issue::new("Test".to_string(), "Desc".to_string());
        let issue_id = issue.id.clone();

        storage.save_issue(&issue).unwrap();
        assert!(storage.issue_path(&issue_id).exists());

        storage.delete_issue(&issue_id).unwrap();
        assert!(!storage.issue_path(&issue_id).exists());

        let index = storage.load_index().unwrap();
        assert!(!index.all_ids.contains(&issue_id));
    }

    #[test]
    fn test_load_nonexistent_issue_returns_error() {
        let (_temp, storage) = setup_storage();
        storage.init().unwrap();

        let result = storage.load_issue("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_gate_registry_operations() {
        let (_temp, storage) = setup_storage();
        storage.init().unwrap();

        let mut registry = storage.load_gate_registry().unwrap();
        assert!(registry.gates.is_empty());

        let gate = Gate {
            version: 1,
            key: "review".to_string(),
            title: "Code Review".to_string(),
            description: "Manual code review".to_string(),
            stage: crate::domain::GateStage::Postcheck,
            mode: crate::domain::GateMode::Manual,
            checker: None,
            reserved: std::collections::HashMap::new(),
            auto: false,
            example_integration: None,
        };

        registry.gates.insert(gate.key.clone(), gate.clone());
        storage.save_gate_registry(&registry).unwrap();

        let loaded = storage.load_gate_registry().unwrap();
        assert_eq!(loaded.gates.len(), 1);
        assert_eq!(loaded.gates.get("review").unwrap().title, "Code Review");
    }

    // Concurrent access tests

    #[test]
    fn test_concurrent_issue_creates_no_corruption() {
        use std::sync::Arc;
        use std::thread;

        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(JsonFileStorage::new(temp_dir.path()));
        storage.init().unwrap();

        let num_threads = 10;
        let issues_per_thread = 5;

        let handles: Vec<_> = (0..num_threads)
            .map(|thread_id| {
                let storage = Arc::clone(&storage);
                thread::spawn(move || {
                    for i in 0..issues_per_thread {
                        let issue = Issue::new(
                            format!("Thread {} Issue {}", thread_id, i),
                            format!("Description {}-{}", thread_id, i),
                        );
                        storage.save_issue(&issue).unwrap();
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // Verify: exactly 50 issues, no duplicates in index
        let issues = storage.list_issues().unwrap();
        assert_eq!(issues.len(), num_threads * issues_per_thread);

        let index = storage.load_index().unwrap();
        assert_eq!(index.all_ids.len(), num_threads * issues_per_thread);

        // Check for duplicates
        let mut ids = index.all_ids.clone();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), num_threads * issues_per_thread);
    }

    #[test]
    fn test_concurrent_updates_to_different_issues() {
        use std::sync::Arc;
        use std::thread;

        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(JsonFileStorage::new(temp_dir.path()));
        storage.init().unwrap();

        // Create two issues
        let issue1 = Issue::new("Issue 1".to_string(), "Desc 1".to_string());
        let issue2 = Issue::new("Issue 2".to_string(), "Desc 2".to_string());
        let id1 = issue1.id.clone();
        let id2 = issue2.id.clone();

        storage.save_issue(&issue1).unwrap();
        storage.save_issue(&issue2).unwrap();

        // Update them concurrently
        let storage1 = Arc::clone(&storage);
        let storage2 = Arc::clone(&storage);
        let id1_clone = id1.clone();
        let id2_clone = id2.clone();

        let handle1 = thread::spawn(move || {
            for i in 0..10 {
                let mut issue = storage1.load_issue(&id1_clone).unwrap();
                issue.title = format!("Updated 1 - {}", i);
                storage1.save_issue(&issue).unwrap();
            }
        });

        let handle2 = thread::spawn(move || {
            for i in 0..10 {
                let mut issue = storage2.load_issue(&id2_clone).unwrap();
                issue.title = format!("Updated 2 - {}", i);
                storage2.save_issue(&issue).unwrap();
            }
        });

        handle1.join().unwrap();
        handle2.join().unwrap();

        // Both issues should exist and be loadable
        let loaded1 = storage.load_issue(&id1).unwrap();
        let loaded2 = storage.load_issue(&id2).unwrap();
        assert!(loaded1.title.starts_with("Updated 1"));
        assert!(loaded2.title.starts_with("Updated 2"));
    }

    #[test]
    fn test_concurrent_updates_to_same_issue() {
        use std::sync::Arc;
        use std::thread;

        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(JsonFileStorage::new(temp_dir.path()));
        storage.init().unwrap();

        let issue = Issue::new("Test Issue".to_string(), "Description".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(&issue).unwrap();

        let num_threads = 5;
        let updates_per_thread = 3;

        let handles: Vec<_> = (0..num_threads)
            .map(|thread_id| {
                let storage = Arc::clone(&storage);
                let id = issue_id.clone();
                thread::spawn(move || {
                    for i in 0..updates_per_thread {
                        let mut issue = storage.load_issue(&id).unwrap();
                        issue.title = format!("Thread {} Update {}", thread_id, i);
                        storage.save_issue(&issue).unwrap();
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // Issue should still be loadable and valid
        let final_issue = storage.load_issue(&issue_id).unwrap();
        assert!(final_issue.title.starts_with("Thread"));
        assert_eq!(final_issue.id, issue_id);
    }

    #[test]
    fn test_concurrent_read_write_issue() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(JsonFileStorage::new(temp_dir.path()));
        storage.init().unwrap();

        let issue = Issue::new("Test".to_string(), "Desc".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(&issue).unwrap();

        let barrier = Arc::new(Barrier::new(6)); // 1 writer + 5 readers

        // Writer thread
        let storage_writer = Arc::clone(&storage);
        let id_writer = issue_id.clone();
        let barrier_writer = Arc::clone(&barrier);
        let writer = thread::spawn(move || {
            barrier_writer.wait();
            for i in 0..5 {
                let mut issue = storage_writer.load_issue(&id_writer).unwrap();
                issue.title = format!("Updated {}", i);
                storage_writer.save_issue(&issue).unwrap();
                thread::sleep(std::time::Duration::from_millis(10));
            }
        });

        // Reader threads
        let mut readers = vec![];
        for _ in 0..5 {
            let storage_reader = Arc::clone(&storage);
            let id_reader = issue_id.clone();
            let barrier_reader = Arc::clone(&barrier);
            readers.push(thread::spawn(move || {
                barrier_reader.wait();
                for _ in 0..10 {
                    let issue = storage_reader.load_issue(&id_reader).unwrap();
                    // Should always get valid data (not corrupted)
                    assert!(!issue.title.is_empty());
                    assert_eq!(issue.id, id_reader);
                }
            }));
        }

        writer.join().unwrap();
        for reader in readers {
            reader.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_dependency_operations() {
        use std::sync::Arc;
        use std::thread;

        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(JsonFileStorage::new(temp_dir.path()));
        storage.init().unwrap();

        // Create base issue
        let base = Issue::new("Base".to_string(), "Desc".to_string());
        let base_id = base.id.clone();
        storage.save_issue(&base).unwrap();

        // Add dependencies concurrently
        let handles: Vec<_> = (0..5)
            .map(|i| {
                let storage = Arc::clone(&storage);
                let base_id = base_id.clone();
                thread::spawn(move || {
                    let dep = Issue::new(format!("Dep {}", i), "Desc".to_string());
                    let dep_id = dep.id.clone();
                    storage.save_issue(&dep).unwrap();

                    let mut base = storage.load_issue(&base_id).unwrap();
                    base.dependencies.push(dep_id);
                    storage.save_issue(&base).unwrap();
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // Load and verify
        let final_base = storage.load_issue(&base_id).unwrap();
        // Note: Due to concurrent updates, we may lose some dependencies
        // (last write wins), but the data should not be corrupted
        assert!(!final_base.dependencies.is_empty());
        assert!(final_base.dependencies.len() <= 5);
    }

    #[test]
    fn test_concurrent_list_and_create() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(JsonFileStorage::new(temp_dir.path()));
        storage.init().unwrap();

        let barrier = Arc::new(Barrier::new(4));

        // Create some initial issues
        for i in 0..3 {
            let issue = Issue::new(format!("Initial {}", i), "Desc".to_string());
            storage.save_issue(&issue).unwrap();
        }

        // Concurrent readers
        let mut handles = vec![];
        for _ in 0..3 {
            let storage = Arc::clone(&storage);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                for _ in 0..5 {
                    let issues = storage.list_issues().unwrap();
                    assert!(issues.len() >= 3); // At least initial issues
                }
            }));
        }

        // Concurrent writer
        let storage_writer = Arc::clone(&storage);
        let barrier_writer = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier_writer.wait();
            for i in 0..5 {
                let issue = Issue::new(format!("New {}", i), "Desc".to_string());
                storage_writer.save_issue(&issue).unwrap();
            }
        }));

        for handle in handles {
            handle.join().unwrap();
        }

        // Final check
        let final_issues = storage.list_issues().unwrap();
        assert_eq!(final_issues.len(), 8); // 3 initial + 5 new
    }

    // Tests for cross-worktree issue visibility (TDD)
    mod cross_worktree_tests {
        use super::*;
        use std::fs;
        use std::process::Command;

        fn setup_git_repo() -> (TempDir, PathBuf) {
            let temp_dir = TempDir::new().unwrap();
            let repo_path = temp_dir.path().to_path_buf();

            // Initialize git repository
            Command::new("git")
                .args(["init"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            Command::new("git")
                .args(["config", "user.name", "Test"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            Command::new("git")
                .args(["config", "user.email", "test@example.com"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            (temp_dir, repo_path)
        }

        #[test]
        fn test_load_issue_from_local_first() {
            // Setup: Create a local .jit directory
            let (_temp, storage) = setup_storage();
            storage.init().unwrap();

            // Create and save an issue locally
            let issue = Issue::new("Local Issue".to_string(), "Description".to_string());
            let issue_id = issue.id.clone();
            storage.save_issue(&issue).unwrap();

            // Should read from local storage
            let loaded = storage.load_issue(&issue_id).unwrap();
            assert_eq!(loaded.title, "Local Issue");
        }

        #[test]
        fn test_load_issue_from_git_when_not_local() {
            // Setup: Create git repo with committed issue
            let (_temp_dir, repo_path) = setup_git_repo();
            let jit_dir = repo_path.join(".jit");
            let storage = JsonFileStorage::new(&jit_dir);
            storage.init().unwrap();

            // Create an issue and commit it
            let issue = Issue::new("Committed Issue".to_string(), "From git".to_string());
            let issue_id = issue.id.clone();
            storage.save_issue(&issue).unwrap();

            // Commit to git
            Command::new("git")
                .args(["add", ".jit"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            Command::new("git")
                .args(["commit", "-m", "Add issue"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            // Remove from local .jit (simulate reading from different worktree)
            let issue_path = jit_dir.join("issues").join(format!("{}.json", issue_id));
            fs::remove_file(&issue_path).unwrap();

            // Update index to not include the issue
            let index = Index {
                schema_version: 2,
                all_ids: vec![],
                deleted_ids: vec![],
            };
            storage
                .write_json(&jit_dir.join(INDEX_FILE), &index)
                .unwrap();

            // Should fall back to reading from git
            let loaded = storage.load_issue(&issue_id).unwrap();
            assert_eq!(loaded.title, "Committed Issue");
            assert_eq!(loaded.description, "From git");
        }

        #[test]
        fn test_load_issue_from_main_worktree_when_not_in_git() {
            // Setup: Create git repo with main worktree
            let (_temp_dir, repo_path) = setup_git_repo();
            let main_jit = repo_path.join(".jit");
            let main_storage = JsonFileStorage::new(&main_jit);
            main_storage.init().unwrap();

            // Create an issue in main worktree (not committed)
            let issue = Issue::new("Main WT Issue".to_string(), "Uncommitted".to_string());
            let issue_id = issue.id.clone();
            main_storage.save_issue(&issue).unwrap();

            // Create secondary worktree with unique name
            use std::time::SystemTime;
            let timestamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let worktree_name = format!("secondary-{}", timestamp);
            let branch_name = format!("feature-{}", timestamp);
            let secondary_rel_path = format!("../{}", worktree_name);

            Command::new("git")
                .args(["worktree", "add", "-b", &branch_name, &secondary_rel_path])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            let secondary_path = repo_path.parent().unwrap().join(&worktree_name);
            let secondary_jit = secondary_path.join(".jit");
            fs::create_dir_all(secondary_jit.join("issues")).unwrap();

            // Initialize secondary storage
            let secondary_storage = JsonFileStorage::new(&secondary_jit);
            secondary_storage.init().unwrap();

            // Should fall back to reading from main worktree
            let loaded = secondary_storage.load_issue(&issue_id).unwrap();
            assert_eq!(loaded.title, "Main WT Issue");
            assert_eq!(loaded.description, "Uncommitted");
        }

        #[test]
        fn test_load_aggregated_index_includes_git_issues() {
            // Setup: Create git repo with committed issue
            let (_temp_dir, repo_path) = setup_git_repo();
            let jit_dir = repo_path.join(".jit");
            let storage = JsonFileStorage::new(&jit_dir);
            storage.init().unwrap();

            // Create and commit issue
            let issue = Issue::new("Committed Issue".to_string(), "In git".to_string());
            let issue_id = issue.id.clone();
            storage.save_issue(&issue).unwrap();

            Command::new("git")
                .args(["add", ".jit"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            Command::new("git")
                .args(["commit", "-m", "Add issue"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            // Remove from local index and storage (simulate fresh worktree)
            let issue_path = jit_dir.join("issues").join(format!("{}.json", issue_id));
            fs::remove_file(&issue_path).unwrap();

            let index = Index {
                schema_version: 2,
                all_ids: vec![],
                deleted_ids: vec![],
            };
            storage
                .write_json(&jit_dir.join(INDEX_FILE), &index)
                .unwrap();

            // load_aggregated_index should find it in git
            let aggregated = storage.load_aggregated_index().unwrap();
            assert!(aggregated.all_ids.contains(&issue_id));
        }

        #[test]
        fn test_load_aggregated_index_includes_main_worktree_issues() {
            // Setup: Create git repo with main worktree
            let (_temp_dir, repo_path) = setup_git_repo();
            let main_jit = repo_path.join(".jit");
            let main_storage = JsonFileStorage::new(&main_jit);
            main_storage.init().unwrap();

            // Create issue in main worktree (not committed)
            let issue = Issue::new("Main WT Issue".to_string(), "Uncommitted".to_string());
            let issue_id = issue.id.clone();
            main_storage.save_issue(&issue).unwrap();

            // Create secondary worktree
            use std::time::SystemTime;
            let timestamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let worktree_name = format!("secondary-{}", timestamp);
            let branch_name = format!("feature-{}", timestamp);
            let secondary_rel_path = format!("../{}", worktree_name);

            Command::new("git")
                .args(["worktree", "add", "-b", &branch_name, &secondary_rel_path])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            let secondary_path = repo_path.parent().unwrap().join(&worktree_name);
            let secondary_jit = secondary_path.join(".jit");
            fs::create_dir_all(secondary_jit.join("issues")).unwrap();

            let secondary_storage = JsonFileStorage::new(&secondary_jit);
            secondary_storage.init().unwrap();

            // Aggregated index should include main worktree issue
            let aggregated = secondary_storage.load_aggregated_index().unwrap();
            assert!(aggregated.all_ids.contains(&issue_id));
        }

        #[test]
        fn test_load_aggregated_index_deduplicates() {
            // Setup: Create git repo
            let (_temp_dir, repo_path) = setup_git_repo();
            let jit_dir = repo_path.join(".jit");
            let storage = JsonFileStorage::new(&jit_dir);
            storage.init().unwrap();

            // Create and commit issue
            let issue = Issue::new("Duplicate Issue".to_string(), "Test".to_string());
            let issue_id = issue.id.clone();
            storage.save_issue(&issue).unwrap();

            Command::new("git")
                .args(["add", ".jit"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            Command::new("git")
                .args(["commit", "-m", "Add issue"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            // Issue is now in both local AND git
            // Aggregated index should deduplicate
            let aggregated = storage.load_aggregated_index().unwrap();
            let count = aggregated
                .all_ids
                .iter()
                .filter(|id| *id == &issue_id)
                .count();
            assert_eq!(count, 1, "Issue ID should appear exactly once");
        }

        #[test]
        fn test_load_issue_prefers_local_over_git() {
            // Setup: Create git repo with committed issue
            let (_temp_dir, repo_path) = setup_git_repo();
            let jit_dir = repo_path.join(".jit");
            let storage = JsonFileStorage::new(&jit_dir);
            storage.init().unwrap();

            // Create and commit an issue
            let mut issue = Issue::new("Original".to_string(), "Old version".to_string());
            let issue_id = issue.id.clone();
            storage.save_issue(&issue).unwrap();

            Command::new("git")
                .args(["add", ".jit"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            Command::new("git")
                .args(["commit", "-m", "Add issue"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            // Update issue locally (not committed)
            issue.title = "Updated Locally".to_string();
            issue.description = "New version".to_string();
            storage.save_issue(&issue).unwrap();

            // Should prefer local version over git version
            let loaded = storage.load_issue(&issue_id).unwrap();
            assert_eq!(loaded.title, "Updated Locally");
            assert_eq!(loaded.description, "New version");
        }

        #[test]
        fn test_load_issue_fails_when_not_found_anywhere() {
            let (_temp, storage) = setup_storage();
            storage.init().unwrap();

            let fake_id = "00000000-0000-0000-0000-000000000000";
            let result = storage.load_issue(fake_id);

            assert!(result.is_err());
            let err_msg = result.unwrap_err().to_string();
            assert!(err_msg.contains("not found") || err_msg.contains("Issue"));
        }

        #[test]
        fn test_load_issue_from_git_handles_invalid_json() {
            // Setup: Create git repo with invalid JSON committed
            let (_temp_dir, repo_path) = setup_git_repo();
            let jit_dir = repo_path.join(".jit");
            fs::create_dir_all(jit_dir.join("issues")).unwrap();

            let storage = JsonFileStorage::new(&jit_dir);
            storage.init().unwrap();

            // Create invalid JSON file
            let issue_id = "11111111-1111-1111-1111-111111111111";
            let issue_path = jit_dir.join("issues").join(format!("{}.json", issue_id));
            fs::write(&issue_path, "{ invalid json }").unwrap();

            // Commit it
            Command::new("git")
                .args(["add", ".jit"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            Command::new("git")
                .args(["commit", "-m", "Add invalid issue"])
                .current_dir(&repo_path)
                .output()
                .unwrap();

            // Remove from local
            fs::remove_file(&issue_path).unwrap();

            // Should fail gracefully with parse error (not panic)
            let result = storage.load_issue(issue_id);
            assert!(result.is_err());
        }
    }

    // Tests for schema v2 with deletion tracking
    mod schema_v2_tests {
        use super::*;

        #[test]
        fn test_index_v2_has_deleted_ids_field() {
            // Create a new index - should be v2
            let index = Index::default();

            assert_eq!(index.schema_version, 2);
            assert_eq!(index.all_ids, Vec::<String>::new());
            assert_eq!(index.deleted_ids, Vec::<String>::new());
        }

        #[test]
        fn test_index_v2_save_and_load() {
            let temp_dir = TempDir::new().unwrap();
            let storage = JsonFileStorage::new(temp_dir.path());
            storage.init().unwrap();

            // Create index with deleted IDs
            let index = Index {
                schema_version: 2,
                all_ids: vec!["issue-1".to_string(), "issue-2".to_string()],
                deleted_ids: vec!["issue-3".to_string()],
            };

            // Save index
            storage.save_index(&index).unwrap();

            // Load it back
            let loaded = storage.load_index().unwrap();

            assert_eq!(loaded.schema_version, 2);
            assert_eq!(loaded.all_ids.len(), 2);
            assert!(loaded.all_ids.contains(&"issue-1".to_string()));
            assert!(loaded.all_ids.contains(&"issue-2".to_string()));
            assert_eq!(loaded.deleted_ids.len(), 1);
            assert!(loaded.deleted_ids.contains(&"issue-3".to_string()));
        }

        #[test]
        fn test_init_creates_v2_index() {
            let temp_dir = TempDir::new().unwrap();
            let storage = JsonFileStorage::new(temp_dir.path());
            storage.init().unwrap();

            let index = storage.load_index().unwrap();
            assert_eq!(index.schema_version, 2);
            assert!(index.deleted_ids.is_empty());
        }

        #[test]
        fn test_delete_issue_adds_to_deleted_ids() {
            let temp_dir = TempDir::new().unwrap();
            let storage = JsonFileStorage::new(temp_dir.path());
            storage.init().unwrap();

            // Create an issue
            let issue = Issue::new("Test".to_string(), "Description".to_string());
            let issue_id = issue.id.clone();
            storage.save_issue(&issue).unwrap();

            // Verify it's in all_ids
            let index_before = storage.load_index().unwrap();
            assert!(index_before.all_ids.contains(&issue_id));
            assert!(!index_before.deleted_ids.contains(&issue_id));

            // Delete the issue
            storage.delete_issue(&issue_id).unwrap();

            // Verify it's now in deleted_ids and removed from all_ids
            let index_after = storage.load_index().unwrap();
            assert!(!index_after.all_ids.contains(&issue_id));
            assert!(index_after.deleted_ids.contains(&issue_id));
        }

        #[test]
        fn test_aggregated_index_filters_deleted_ids() {
            let temp_dir = TempDir::new().unwrap();
            let storage = JsonFileStorage::new(temp_dir.path());
            storage.init().unwrap();

            // Create and delete an issue
            let issue = Issue::new("Test".to_string(), "Description".to_string());
            let issue_id = issue.id.clone();
            storage.save_issue(&issue).unwrap();
            storage.delete_issue(&issue_id).unwrap();

            // Aggregated index should NOT include the deleted issue
            let aggregated = storage.load_aggregated_index().unwrap();
            assert!(!aggregated.all_ids.contains(&issue_id));
        }
    }

    // Tests for worktree deletion safety (Phase 3)
    mod deletion_safety_tests {
        use super::*;

        #[test]
        fn test_is_secondary_worktree_detection() {
            // Main worktree: .git is a directory
            let main_temp = TempDir::new().unwrap();
            let main_storage = JsonFileStorage::new(main_temp.path());
            
            // Initialize git
            Command::new("git")
                .arg("init")
                .current_dir(main_temp.path().parent().unwrap())
                .output()
                .unwrap();
            
            main_storage.init().unwrap();
            
            // Should detect as main worktree
            assert!(!main_storage.is_secondary_worktree());
        }
    }
}
