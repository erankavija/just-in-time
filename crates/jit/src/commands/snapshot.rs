//! Snapshot export command implementation

use crate::document::{AdapterRegistry, AssetScanner};
use crate::domain::{DocumentReference, Issue};
use crate::snapshot::{
    compute_sha256, AssetSnapshot, DocumentSnapshot, SnapshotFormat, SnapshotScope, SourceInfo,
    SourceMode,
};
use crate::storage::IssueStore;
use anyhow::{anyhow, Context, Result};
use std::path::Path;

/// Options for snapshot export
#[allow(dead_code)]
pub struct SnapshotExportOptions {
    /// Output path (default: snapshot-YYYYMMDD-HHMMSS)
    pub out: Option<std::path::PathBuf>,
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

    /// Extract document references from issues
    pub fn extract_documents(&self, issues: &[Issue]) -> Vec<DocumentReference> {
        let mut docs = Vec::new();
        let mut seen_paths = std::collections::HashSet::new();
        
        for issue in issues {
            for doc_ref in &issue.documents {
                // Deduplicate by path
                if seen_paths.insert(doc_ref.path.clone()) {
                    docs.push(doc_ref.clone());
                }
            }
        }
        
        docs
    }

    /// Read file content from git at specific commit
    fn read_from_git(
        &self,
        repo: &git2::Repository,
        path: &str,
        reference: &str,
    ) -> Result<(Vec<u8>, SourceInfo)> {
        let obj = repo
            .revparse_single(reference)
            .with_context(|| format!("Failed to resolve git reference: {}", reference))?;
        let commit = obj
            .peel_to_commit()
            .with_context(|| format!("Reference '{}' is not a commit", reference))?;
        let tree = commit.tree()?;
        
        let entry = tree
            .get_path(Path::new(path))
            .with_context(|| format!("Path '{}' not found in commit {}", path, reference))?;
        let blob = repo.find_blob(entry.id())?;
        
        let content = blob.content().to_vec();
        
        Ok((
            content,
            SourceInfo {
                type_: "git".to_string(),
                commit: Some(commit.id().to_string()),
                blob_sha: Some(entry.id().to_string()),
            },
        ))
    }

    /// Read file content from working tree filesystem
    fn read_from_filesystem(&self, path: &str) -> Result<(Vec<u8>, SourceInfo)> {
        // Get repository root (parent of .jit directory)
        let repo_root = self
            .storage
            .root()
            .parent()
            .ok_or_else(|| anyhow!("Invalid storage path"))?;
        
        let full_path = repo_root.join(path);
        let content = std::fs::read(&full_path)
            .with_context(|| format!("Failed to read file: {}", path))?;
        
        Ok((
            content,
            SourceInfo {
                type_: "filesystem".to_string(),
                commit: None,
                blob_sha: None,
            },
        ))
    }

    /// Read content based on source mode
    fn read_content(&self, path: &str, mode: &SourceMode) -> Result<(Vec<u8>, SourceInfo)> {
        match mode {
            SourceMode::Git { commit } => {
                let repo = git2::Repository::open(".")?;
                self.read_from_git(&repo, path, commit)
            }
            SourceMode::WorkingTree => self.read_from_filesystem(path),
        }
    }

    /// Create document snapshot with assets
    pub fn create_document_snapshot(
        &self,
        doc_ref: &DocumentReference,
        mode: &SourceMode,
    ) -> Result<DocumentSnapshot> {
        // Read document content
        let (content, source) = self.read_content(&doc_ref.path, mode)?;
        let hash = compute_sha256(&content);
        
        // Scan for assets if not already in doc_ref
        let repo_root = self
            .storage
            .root()
            .parent()
            .ok_or_else(|| anyhow!("Invalid storage path"))?;
        
        // Create new adapter registry for scanning
        let registry = AdapterRegistry::with_builtins();
        let scanner = AssetScanner::new(registry, repo_root);
        let content_str = String::from_utf8_lossy(&content);
        let assets = scanner
            .scan_document(Path::new(&doc_ref.path), &content_str)
            .unwrap_or_default();
        
        // Create asset snapshots
        let mut asset_snapshots = Vec::new();
        for asset in &assets {
            if let Some(resolved_path) = &asset.resolved_path {
                // Only include local assets that exist
                if matches!(asset.asset_type, crate::document::AssetType::Local) {
                    if let Ok((asset_content, asset_source)) =
                        self.read_content(&resolved_path.to_string_lossy(), mode)
                    {
                        asset_snapshots.push(AssetSnapshot {
                            path: resolved_path.to_string_lossy().to_string(),
                            source: asset_source,
                            hash_sha256: compute_sha256(&asset_content),
                            mime: asset.mime_type.clone().unwrap_or_else(|| "application/octet-stream".to_string()),
                            size_bytes: asset_content.len(),
                            shared: asset.is_shared,
                        });
                    }
                }
            }
        }
        
        Ok(DocumentSnapshot {
            path: doc_ref.path.clone(),
            format: doc_ref
                .format
                .clone()
                .unwrap_or_else(|| "markdown".to_string()),
            size_bytes: content.len(),
            source,
            hash_sha256: hash,
            assets: asset_snapshots,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DocumentReference, Issue};
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

    #[test]
    fn test_extract_documents() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        
        let mut issue1 = Issue::new("Issue 1".to_string(), String::new());
        issue1.documents.push(DocumentReference {
            path: "docs/design.md".to_string(),
            commit: None,
            label: None,
            doc_type: None,
            format: None,
            assets: vec![],
        });
        storage.save_issue(&issue1).unwrap();
        
        let mut issue2 = Issue::new("Issue 2".to_string(), String::new());
        issue2.documents.push(DocumentReference {
            path: "docs/impl.md".to_string(),
            commit: None,
            label: None,
            doc_type: None,
            format: None,
            assets: vec![],
        });
        // Duplicate document reference
        issue2.documents.push(DocumentReference {
            path: "docs/design.md".to_string(),
            commit: None,
            label: None,
            doc_type: None,
            format: None,
            assets: vec![],
        });
        storage.save_issue(&issue2).unwrap();
        
        let exporter = SnapshotExporter::new(storage.clone());
        let issues = vec![issue1, issue2];
        let docs = exporter.extract_documents(&issues);
        
        // Should be deduplicated
        assert_eq!(docs.len(), 2);
        let paths: Vec<_> = docs.iter().map(|d| d.path.as_str()).collect();
        assert!(paths.contains(&"docs/design.md"));
        assert!(paths.contains(&"docs/impl.md"));
    }

    #[test]
    fn test_extract_documents_empty() {
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        
        let exporter = SnapshotExporter::new(storage.clone());
        let docs = exporter.extract_documents(&[]);
        
        assert_eq!(docs.len(), 0);
    }
}
