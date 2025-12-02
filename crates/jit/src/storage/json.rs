//! JSON file-based storage implementation.
//!
//! All data is stored as JSON files in a `data/` directory with atomic writes.

use crate::domain::{Event, Issue};
use crate::storage::{GateRegistry, IssueStore};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const ISSUES_DIR: &str = "data/issues";
const INDEX_FILE: &str = "data/index.json";
const GATES_FILE: &str = "data/gates.json";
const EVENTS_FILE: &str = "data/events.jsonl";

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
/// This implementation stores each issue as a separate JSON file in `data/issues/`,
/// gate definitions in `data/gates.json`, and events in `data/events.jsonl`.
/// All file writes are atomic (write to temp file, then rename).
#[derive(Clone)]
pub struct JsonFileStorage {
    root: PathBuf,
}

impl JsonFileStorage {
    /// Create a new JSON file storage instance at the given root path
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    fn issue_path(&self, id: &str) -> PathBuf {
        self.root.join(ISSUES_DIR).join(format!("{}.json", id))
    }

    fn write_json<T: Serialize>(&self, path: &Path, data: &T) -> Result<()> {
        let json = serde_json::to_string_pretty(data).context("Failed to serialize data")?;

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

        Ok(())
    }

    fn save_issue(&self, issue: &Issue) -> Result<()> {
        let issue_path = self.issue_path(&issue.id);
        self.write_json(&issue_path, issue)?;

        // Update index
        let mut index = self.load_index()?;
        if !index.all_ids.contains(&issue.id) {
            index.all_ids.push(issue.id.clone());
            self.save_index(&index)?;
        }

        Ok(())
    }

    fn load_issue(&self, id: &str) -> Result<Issue> {
        let issue_path = self.issue_path(id);
        self.read_json(&issue_path)
    }

    fn delete_issue(&self, id: &str) -> Result<()> {
        let issue_path = self.issue_path(id);
        fs::remove_file(&issue_path).context("Failed to delete issue file")?;

        // Update index
        let mut index = self.load_index()?;
        index.all_ids.retain(|i| i != id);
        self.save_index(&index)?;

        Ok(())
    }

    fn list_issues(&self) -> Result<Vec<Issue>> {
        let index = self.load_index()?;
        index.all_ids.iter().map(|id| self.load_issue(id)).collect()
    }

    fn load_gate_registry(&self) -> Result<GateRegistry> {
        let gates_path = self.root.join(GATES_FILE);
        self.read_json(&gates_path)
    }

    fn save_gate_registry(&self, registry: &GateRegistry) -> Result<()> {
        let gates_path = self.root.join(GATES_FILE);
        self.write_json(&gates_path, registry)
    }

    fn append_event(&self, event: &Event) -> Result<()> {
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
}
