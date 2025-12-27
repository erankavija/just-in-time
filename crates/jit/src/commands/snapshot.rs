//! Snapshot export command implementation

use crate::domain::Issue;
use crate::snapshot::{SnapshotFormat, SnapshotScope, SourceMode};
use crate::storage::IssueStore;
use anyhow::{anyhow, Result};
use std::path::PathBuf;

/// Options for snapshot export
pub struct SnapshotExportOptions {
    /// Output path (default: snapshot-YYYYMMDD-HHMMSS)
    pub out: Option<PathBuf>,
    /// Output format
    pub format: SnapshotFormat,
    /// Scope of export
    pub scope: SnapshotScope,
    /// Git commit/tag to export (requires git)
    pub at: Option<String>,
    /// Export from working tree instead of git
    pub working_tree: bool,
    /// Reject if uncommitted docs/assets (requires git, implies --at HEAD)
    pub committed_only: bool,
    /// Skip repository validation
    pub force: bool,
    /// Output metadata in JSON
    pub json: bool,
}

/// Snapshot exporter
pub struct SnapshotExporter<S: IssueStore> {
    storage: S,
}

impl<S: IssueStore> SnapshotExporter<S> {
    /// Create new snapshot exporter
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    /// Determine source mode based on options and git availability
    pub fn determine_source_mode(
        at_commit: Option<&str>,
        working_tree: bool,
        committed_only: bool,
    ) -> Result<SourceMode> {
        match (at_commit, working_tree, committed_only) {
            (Some(_), true, _) => {
                Err(anyhow!("Cannot use both --at and --working-tree"))
            }
            (Some(commit), _, _) => {
                // Explicit commit requires git
                if git2::Repository::open(".").is_err() {
                    return Err(anyhow!("--at requires git repository"));
                }
                Ok(SourceMode::Git {
                    commit: commit.to_string(),
                })
            }
            (_, true, _) => {
                // Explicit working tree - no git needed
                Ok(SourceMode::WorkingTree)
            }
            (None, false, true) => {
                // --committed-only implies --at HEAD
                if git2::Repository::open(".").is_err() {
                    return Err(anyhow!("--committed-only requires git repository"));
                }
                Ok(SourceMode::Git {
                    commit: "HEAD".to_string(),
                })
            }
            (None, false, false) => {
                // Default: use git if available, else working tree
                if let Ok(repo) = git2::Repository::open(".") {
                    if let Ok(head) = repo.head() {
                        if let Ok(commit) = head.peel_to_commit() {
                            return Ok(SourceMode::Git {
                                commit: commit.id().to_string(),
                            });
                        }
                    }
                }
                Ok(SourceMode::WorkingTree)
            }
        }
    }

    /// Enumerate issues based on scope
    pub fn enumerate_issues(&self, scope: &SnapshotScope) -> Result<Vec<Issue>> {
        match scope {
            SnapshotScope::All => self.storage.list_issues(),
            SnapshotScope::Issue(id) => {
                let issue = self.storage.load_issue(id)?;
                Ok(vec![issue])
            }
            SnapshotScope::Label { namespace, value } => {
                // Filter issues by label "namespace:value"
                let target_label = format!("{}:{}", namespace, value);
                let all_issues = self.storage.list_issues()?;
                let matching_issues: Vec<Issue> = all_issues
                    .into_iter()
                    .filter(|issue| issue.labels.contains(&target_label))
                    .collect();
                
                Ok(matching_issues)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Issue;
    use crate::storage::InMemoryStorage;

    #[test]
    fn test_source_mode_both_at_and_working_tree() {
        let result = SnapshotExporter::<InMemoryStorage>::determine_source_mode(
            Some("abc123"),
            true,
            false,
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Cannot use both --at and --working-tree"));
    }

    #[test]
    fn test_source_mode_explicit_working_tree() {
        let result = SnapshotExporter::<InMemoryStorage>::determine_source_mode(
            None, true, false,
        );
        assert!(result.is_ok());
        matches!(result.unwrap(), SourceMode::WorkingTree);
    }

    #[test]
    fn test_source_mode_default_no_git() {
        // In a non-git directory, should fall back to working tree
        let result = SnapshotExporter::<InMemoryStorage>::determine_source_mode(
            None, false, false,
        );
        assert!(result.is_ok());
        // Result depends on whether we're in a git repo, so we just verify it doesn't error
    }

    #[test]
    fn test_enumerate_issues_all() {
        let mut storage = InMemoryStorage::new();
        storage.init().unwrap();
        
        // Create a couple of issues
        let issue1 = Issue::new("Issue 1".to_string(), String::new());
        let issue2 = Issue::new("Issue 2".to_string(), String::new());
        storage.save_issue(&issue1).unwrap();
        storage.save_issue(&issue2).unwrap();
        
        let exporter = SnapshotExporter::new(storage.clone());
        let issues = exporter.enumerate_issues(&SnapshotScope::All).unwrap();
        
        assert_eq!(issues.len(), 2);
    }

    #[test]
    fn test_enumerate_issues_single() {
        let mut storage = InMemoryStorage::new();
        storage.init().unwrap();
        
        let issue = Issue::new("Test Issue".to_string(), String::new());
        let issue_id = issue.id.clone();
        storage.save_issue(&issue).unwrap();
        
        let exporter = SnapshotExporter::new(storage.clone());
        let issues = exporter
            .enumerate_issues(&SnapshotScope::Issue(issue_id.clone()))
            .unwrap();
        
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, issue_id);
    }

    #[test]
    fn test_enumerate_issues_label_epic() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        
        // Create issues with epic labels
        let mut issue1 = Issue::new("Issue 1".to_string(), String::new());
        issue1.labels.push("epic:auth".to_string());
        storage.save_issue(&issue1).unwrap();
        
        let mut issue2 = Issue::new("Issue 2".to_string(), String::new());
        issue2.labels.push("epic:auth".to_string());
        storage.save_issue(&issue2).unwrap();
        
        // Create issue with different epic
        let mut issue3 = Issue::new("Issue 3".to_string(), String::new());
        issue3.labels.push("epic:billing".to_string());
        storage.save_issue(&issue3).unwrap();
        
        // Create unrelated issue
        let issue4 = Issue::new("Issue 4".to_string(), String::new());
        storage.save_issue(&issue4).unwrap();
        
        let exporter = SnapshotExporter::new(storage.clone());
        let issues = exporter
            .enumerate_issues(&SnapshotScope::Label {
                namespace: "epic".to_string(),
                value: "auth".to_string(),
            })
            .unwrap();
        
        assert_eq!(issues.len(), 2);
        assert!(issues.iter().all(|i| i.labels.contains(&"epic:auth".to_string())));
    }

    #[test]
    fn test_enumerate_issues_label_milestone() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        
        let mut issue1 = Issue::new("Issue 1".to_string(), String::new());
        issue1.labels.push("milestone:v1.0".to_string());
        storage.save_issue(&issue1).unwrap();
        
        let mut issue2 = Issue::new("Issue 2".to_string(), String::new());
        issue2.labels.push("milestone:v2.0".to_string());
        storage.save_issue(&issue2).unwrap();
        
        let exporter = SnapshotExporter::new(storage.clone());
        let issues = exporter
            .enumerate_issues(&SnapshotScope::Label {
                namespace: "milestone".to_string(),
                value: "v1.0".to_string(),
            })
            .unwrap();
        
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].labels, vec!["milestone:v1.0"]);
    }

    #[test]
    fn test_enumerate_issues_label_no_matches() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        
        let issue = Issue::new("Issue".to_string(), String::new());
        storage.save_issue(&issue).unwrap();
        
        let exporter = SnapshotExporter::new(storage.clone());
        let issues = exporter
            .enumerate_issues(&SnapshotScope::Label {
                namespace: "epic".to_string(),
                value: "nonexistent".to_string(),
            })
            .unwrap();
        
        assert_eq!(issues.len(), 0);
    }
}
