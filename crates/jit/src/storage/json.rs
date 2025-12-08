//! JSON file-based storage implementation.
//!
//! All data is stored as JSON files in a `.jit/` directory with atomic writes.
//! The directory location can be overridden with the `JIT_DATA_DIR` environment variable.

use crate::domain::{Event, Issue};
use crate::storage::{FileLocker, GateRegistry, IssueStore};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
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
}

impl Default for Index {
    fn default() -> Self {
        Self {
            schema_version: 1,
            all_ids: Vec::new(),
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

        // Create label-namespaces.json with defaults if it doesn't exist
        let namespaces_path = self.root.join("label-namespaces.json");
        if !namespaces_path.exists() {
            let namespaces = crate::domain::LabelNamespaces::with_defaults();
            self.save_label_namespaces(&namespaces)?;
        }

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
        let issue_path = self.issue_path(id);
        let issue_lock_path = issue_path.with_extension("lock");
        let _lock = self.locker.lock_shared(&issue_lock_path)?;
        self.read_json(&issue_path)
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

        // Update index
        let mut index = self.load_index()?;
        index.all_ids.retain(|i| i != id);
        self.save_index(&index)?;

        Ok(())
    }

    fn list_issues(&self) -> Result<Vec<Issue>> {
        let index_lock_path = self.root.join(".index.lock");
        let _lock = self.locker.lock_shared(&index_lock_path)?;
        let index = self.load_index()?;

        // Load issues with individual shared locks
        index
            .all_ids
            .iter()
            .map(|id| {
                let issue_path = self.issue_path(id);
                let issue_lock_path = issue_path.with_extension("lock");
                let _lock = self.locker.lock_shared(&issue_lock_path)?;
                self.read_json(&issue_path)
            })
            .collect()
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

    fn load_label_namespaces(&self) -> Result<crate::domain::LabelNamespaces> {
        let path = self.root.join("label-namespaces.json");

        if !path.exists() {
            return Ok(crate::domain::LabelNamespaces::new());
        }

        let data = fs::read_to_string(&path).context("Failed to read label-namespaces.json")?;
        let namespaces: crate::domain::LabelNamespaces =
            serde_json::from_str(&data).context("Failed to deserialize label-namespaces.json")?;
        Ok(namespaces)
    }

    fn save_label_namespaces(&self, namespaces: &crate::domain::LabelNamespaces) -> Result<()> {
        let path = self.root.join("label-namespaces.json");
        let temp_path = self.root.join("label-namespaces.json.tmp");

        let json = serde_json::to_string_pretty(namespaces)
            .context("Failed to serialize label namespaces")?;

        // Atomic write: temp file + rename
        fs::write(&temp_path, json).context("Failed to write temporary label-namespaces.json")?;
        fs::rename(&temp_path, &path)
            .context("Failed to rename temporary label-namespaces.json")?;

        Ok(())
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
            key: "review".to_string(),
            title: "Code Review".to_string(),
            description: "Manual code review".to_string(),
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
}
