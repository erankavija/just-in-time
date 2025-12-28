//! Snapshot export command implementation

use crate::document::{AdapterRegistry, AssetScanner};
use crate::domain::{DocumentReference, Issue};
use crate::snapshot::{
    compute_sha256, AssetSnapshot, DocumentSnapshot, IssuesInfo, MetadataInfo, RepoInfo,
    SnapshotFormat, SnapshotManifest, SnapshotScope, SourceInfo, SourceMode, VerificationInfo,
};
use crate::storage::IssueStore;
use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
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
            (Some(_), true, _) => Err(anyhow!("Cannot use both --at and --working-tree")),
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
        let content =
            std::fs::read(&full_path).with_context(|| format!("Failed to read file: {}", path))?;

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
                            mime: asset
                                .mime_type
                                .clone()
                                .unwrap_or_else(|| "application/octet-stream".to_string()),
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

    /// Generate snapshot manifest with full provenance
    pub fn generate_manifest(
        &self,
        issues: &[Issue],
        docs: &[DocumentSnapshot],
        mode: &SourceMode,
    ) -> Result<SnapshotManifest> {
        // Get repository info
        let repo_info = self.get_repo_info(mode)?;

        // Count issues by state
        let mut states = HashMap::new();
        for issue in issues {
            let state_str = format!("{:?}", issue.state).to_lowercase();
            *states.entry(state_str).or_insert(0) += 1;
        }

        // Build issue file paths
        let issue_files: Vec<String> = issues
            .iter()
            .map(|i| format!(".jit/issues/{}.json", i.id))
            .collect();

        let issues_info = IssuesInfo {
            count: issues.len(),
            states,
            files: issue_files,
        };

        // Build documents info
        let documents_info = crate::snapshot::DocumentsInfo {
            count: docs.len(),
            items: docs.to_vec(),
        };

        // Calculate total size
        let total_bytes: usize = docs.iter().map(|d| d.size_bytes).sum::<usize>()
            + docs
                .iter()
                .flat_map(|d| &d.assets)
                .map(|a| a.size_bytes)
                .sum::<usize>();

        let total_files = docs.len() + docs.iter().flat_map(|d| &d.assets).count() + issues.len();

        Ok(SnapshotManifest {
            version: "1".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            created_by: format!("jit snapshot export v{}", env!("CARGO_PKG_VERSION")),
            repo: repo_info,
            scope: "all".to_string(), // TODO: use actual scope
            issues: issues_info,
            documents: documents_info,
            metadata: MetadataInfo {
                link_policy: "preserve".to_string(),
                external_assets_policy: "exclude".to_string(),
                lfs_policy: "allow-pointers".to_string(),
            },
            verification: VerificationInfo {
                total_files,
                total_bytes,
                instructions: "Run 'sha256sum -c checksums.txt' to verify integrity".to_string(),
            },
        })
    }

    /// Get repository information based on source mode
    fn get_repo_info(&self, mode: &SourceMode) -> Result<RepoInfo> {
        let repo_root = self
            .storage
            .root()
            .parent()
            .ok_or_else(|| anyhow!("Invalid storage path"))?;

        let path = repo_root
            .canonicalize()
            .unwrap_or_else(|_| repo_root.to_path_buf())
            .to_string_lossy()
            .to_string();

        match mode {
            SourceMode::Git { commit } => {
                let repo = git2::Repository::open(".")?;

                // Get remote URL
                let remote = repo
                    .find_remote("origin")
                    .ok()
                    .and_then(|r| r.url().map(|s| s.to_string()));

                // Get branch name
                let branch = repo
                    .head()
                    .ok()
                    .and_then(|h| h.shorthand().map(|s| s.to_string()));

                // Check if working tree is dirty
                let statuses = repo.statuses(None)?;
                let dirty = !statuses.is_empty();

                Ok(RepoInfo {
                    path,
                    remote,
                    commit: Some(commit.clone()),
                    branch,
                    dirty,
                    source: "git".to_string(),
                })
            }
            SourceMode::WorkingTree => Ok(RepoInfo {
                path,
                remote: None,
                commit: None,
                branch: None,
                dirty: false,
                source: "working-tree".to_string(),
            }),
        }
    }

    /// Generate README.md for snapshot
    pub fn generate_readme(&self, manifest: &SnapshotManifest) -> String {
        format!(
            r#"# JIT Snapshot Export

**Created:** {created}  
**Repository:** {repo_path}  
**Commit:** {commit}  
**Scope:** {scope}

## Contents

This snapshot contains:
- {issue_count} issues (.jit/issues/)
- {doc_count} documents with associated assets
- Complete JIT configuration and gate definitions

## Verification

To verify snapshot integrity:

```bash
# Verify all file hashes match manifest
jq -r '.documents.items[] | .path + " " + .hash_sha256' manifest.json | sha256sum -c

# Or use the provided checksums file
sha256sum -c checksums.txt
```

## Structure

- `.jit/` - JIT repository state
- `docs/` - Product documentation
- `dev/` - Development documentation
- `manifest.json` - Complete provenance and verification data
- `checksums.txt` - SHA256 checksums for all files

## Import (Future)

This is a read-only export. Future versions may support:
- `jit snapshot import` - Restore snapshot to working repository
- `jit snapshot diff` - Compare snapshots
"#,
            created = manifest.created_at,
            repo_path = manifest.repo.path,
            commit = manifest.repo.commit.as_deref().unwrap_or("working-tree"),
            scope = manifest.scope,
            issue_count = manifest.issues.count,
            doc_count = manifest.documents.count,
        )
    }

    /// Write document and its assets to snapshot directory
    fn write_document_to_snapshot(
        &self,
        base: &Path,
        doc: &DocumentSnapshot,
        mode: &SourceMode,
    ) -> Result<()> {
        // Read document content
        let (content, _) = self.read_content(&doc.path, mode)?;

        // Write to snapshot preserving path
        let dest_path = base.join(&doc.path);
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest_path, content)?;

        // Write assets
        for asset in &doc.assets {
            let (asset_content, _) = self.read_content(&asset.path, mode)?;
            let asset_dest = base.join(&asset.path);
            if let Some(parent) = asset_dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&asset_dest, asset_content)?;
        }

        Ok(())
    }

    /// Copy .jit state files to snapshot
    fn copy_jit_state(&self, base: &Path, issues: &[Issue]) -> Result<()> {
        let jit_dir = base.join(".jit");
        std::fs::create_dir_all(jit_dir.join("issues"))?;

        // Copy issue files
        for issue in issues {
            let issue_json = serde_json::to_string_pretty(issue)?;
            let issue_path = jit_dir.join(format!("issues/{}.json", issue.id));
            std::fs::write(issue_path, issue_json)?;
        }

        // Copy config.toml if it exists
        let config_src = self.storage.root().join("config.toml");
        if config_src.exists() {
            let config_dest = jit_dir.join("config.toml");
            std::fs::copy(&config_src, &config_dest)?;
        }

        // Copy gates.json if it exists
        let gates_src = self.storage.root().join("gates.json");
        if gates_src.exists() {
            let gates_dest = jit_dir.join("gates.json");
            std::fs::copy(&gates_src, &gates_dest)?;
        }

        Ok(())
    }

    /// Generate checksums.txt file for verification
    fn generate_checksums_file(&self, base: &Path, docs: &[DocumentSnapshot]) -> Result<()> {
        let mut checksums = Vec::new();

        // Add document checksums
        for doc in docs {
            checksums.push(format!("{}  {}", doc.hash_sha256, doc.path));

            // Add asset checksums
            for asset in &doc.assets {
                checksums.push(format!("{}  {}", asset.hash_sha256, asset.path));
            }
        }

        let checksums_content = checksums.join("\n") + "\n";
        std::fs::write(base.join("checksums.txt"), checksums_content)?;

        Ok(())
    }

    /// Export snapshot with all phases
    pub fn export(
        &self,
        scope: &SnapshotScope,
        mode: &SourceMode,
        format: &SnapshotFormat,
        out_path: Option<&Path>,
    ) -> Result<String> {
        // Phase 2: Enumerate issues
        let issues = self.enumerate_issues(scope)?;

        if issues.is_empty() {
            return Err(anyhow!("No issues found in scope: {}", scope));
        }

        // Phase 2: Extract documents
        let doc_refs = self.extract_documents(&issues);

        // Phase 3: Create document snapshots
        let mut doc_snapshots = Vec::new();
        for doc_ref in &doc_refs {
            match self.create_document_snapshot(doc_ref, mode) {
                Ok(snapshot) => doc_snapshots.push(snapshot),
                Err(e) => {
                    eprintln!("Warning: Failed to snapshot {}: {}", doc_ref.path, e);
                    // Continue with other documents
                }
            }
        }

        // Phase 4: Assembly - create temp directory and populate it
        let temp_dir = tempfile::TempDir::new()?;
        let base = temp_dir.path();

        // Copy .jit state
        self.copy_jit_state(base, &issues)?;

        // Write documents and assets
        for doc in &doc_snapshots {
            self.write_document_to_snapshot(base, doc, mode)?;
        }

        // Generate manifest
        let manifest = self.generate_manifest(&issues, &doc_snapshots, mode)?;
        let manifest_json = serde_json::to_string_pretty(&manifest)?;
        std::fs::write(base.join("manifest.json"), manifest_json)?;

        // Generate README
        let readme = self.generate_readme(&manifest);
        std::fs::write(base.join("README.md"), readme)?;

        // Generate checksums
        self.generate_checksums_file(base, &doc_snapshots)?;

        // Phase 5: Package
        let output_path = self.determine_output_path(out_path)?;

        match format {
            SnapshotFormat::Directory => {
                self.package_as_directory(temp_dir, &output_path, &manifest)?;
            }
            SnapshotFormat::Tar => {
                self.package_as_tar(temp_dir, &output_path, &manifest)?;
            }
        }

        Ok(output_path.to_string_lossy().to_string())
    }

    /// Determine output path with default timestamp-based naming
    fn determine_output_path(&self, out_path: Option<&Path>) -> Result<std::path::PathBuf> {
        if let Some(path) = out_path {
            Ok(path.to_path_buf())
        } else {
            // Generate default name: snapshot-YYYYMMDD-HHMMSS
            let now = chrono::Local::now();
            let default_name = format!("snapshot-{}", now.format("%Y%m%d-%H%M%S"));
            Ok(std::path::PathBuf::from(default_name))
        }
    }

    /// Package snapshot as directory
    fn package_as_directory(
        &self,
        temp_dir: tempfile::TempDir,
        out_path: &Path,
        manifest: &SnapshotManifest,
    ) -> Result<()> {
        // Check if output path already exists
        if out_path.exists() {
            return Err(anyhow!(
                "Output path already exists: {}",
                out_path.display()
            ));
        }

        // Atomic rename from temp to final destination
        std::fs::rename(temp_dir.path(), out_path)?;

        println!("✓ Snapshot exported to: {}", out_path.display());
        println!("  {} issues", manifest.issues.count);
        println!("  {} documents", manifest.documents.count);

        Ok(())
    }

    /// Package snapshot as tar archive
    fn package_as_tar(
        &self,
        temp_dir: tempfile::TempDir,
        out_path: &Path,
        manifest: &SnapshotManifest,
    ) -> Result<()> {
        use std::fs::File;
        use tar::Builder;

        // Check if output path already exists
        if out_path.exists() {
            return Err(anyhow!(
                "Output path already exists: {}",
                out_path.display()
            ));
        }

        let tar_file = File::create(out_path)?;
        let mut tar = Builder::new(tar_file);

        // Get the base directory name from output path
        let base_name = out_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("snapshot");

        // Add all files from temp directory with base directory prefix
        for entry in std::fs::read_dir(temp_dir.path())? {
            let entry = entry?;
            let path = entry.path();
            let name = path
                .file_name()
                .ok_or_else(|| anyhow!("Invalid file name"))?;

            let archive_path = format!("{}/{}", base_name, name.to_string_lossy());

            if path.is_dir() {
                tar.append_dir_all(&archive_path, &path)?;
            } else {
                tar.append_path_with_name(&path, &archive_path)?;
            }
        }

        tar.finish()?;

        println!("✓ Snapshot exported to: {}", out_path.display());
        println!("  {} issues", manifest.issues.count);
        println!("  {} documents", manifest.documents.count);
        println!("  Archive: {} bytes", out_path.metadata()?.len());

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DocumentReference, Issue};
    use crate::snapshot::{IssuesInfo, MetadataInfo, RepoInfo, SnapshotManifest, VerificationInfo};
    use crate::storage::InMemoryStorage;
    use std::collections::HashMap;

    #[test]
    fn test_source_mode_both_at_and_working_tree() {
        let result =
            SnapshotExporter::<InMemoryStorage>::determine_source_mode(Some("abc123"), true, false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Cannot use both --at and --working-tree"));
    }

    #[test]
    fn test_source_mode_explicit_working_tree() {
        let result = SnapshotExporter::<InMemoryStorage>::determine_source_mode(None, true, false);
        assert!(result.is_ok());
        matches!(result.unwrap(), SourceMode::WorkingTree);
    }

    #[test]
    fn test_source_mode_default_no_git() {
        // In a non-git directory, should fall back to working tree
        let result = SnapshotExporter::<InMemoryStorage>::determine_source_mode(None, false, false);
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
        assert!(issues
            .iter()
            .all(|i| i.labels.contains(&"epic:auth".to_string())));
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

    #[test]
    fn test_generate_readme() {
        let storage = InMemoryStorage::new();
        let exporter = SnapshotExporter::new(storage);

        let manifest = SnapshotManifest {
            version: "1".to_string(),
            created_at: "2025-12-27T23:00:00Z".to_string(),
            created_by: "test".to_string(),
            repo: RepoInfo {
                path: "/test/repo".to_string(),
                remote: Some("https://github.com/test/repo".to_string()),
                commit: Some("abc123".to_string()),
                branch: Some("main".to_string()),
                dirty: false,
                source: "git".to_string(),
            },
            scope: "all".to_string(),
            issues: IssuesInfo {
                count: 5,
                states: HashMap::new(),
                files: vec![],
            },
            documents: crate::snapshot::DocumentsInfo {
                count: 10,
                items: vec![],
            },
            metadata: MetadataInfo {
                link_policy: "preserve".to_string(),
                external_assets_policy: "exclude".to_string(),
                lfs_policy: "allow-pointers".to_string(),
            },
            verification: VerificationInfo {
                total_files: 15,
                total_bytes: 100000,
                instructions: "test".to_string(),
            },
        };

        let readme = exporter.generate_readme(&manifest);

        assert!(readme.contains("# JIT Snapshot Export"));
        assert!(readme.contains("5 issues"));
        assert!(readme.contains("10 documents"));
        assert!(readme.contains("/test/repo"));
        assert!(readme.contains("abc123"));
    }

    #[test]
    fn test_determine_output_path() {
        let storage = InMemoryStorage::new();
        let exporter = SnapshotExporter::new(storage);

        // With explicit path
        let explicit = std::path::Path::new("my-snapshot");
        let result = exporter.determine_output_path(Some(explicit)).unwrap();
        assert_eq!(result, explicit);

        // With default (timestamp-based)
        let default = exporter.determine_output_path(None).unwrap();
        let name = default.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("snapshot-"));
    }
}
