//! Snapshot export command implementation

use crate::commands::{CommandExecutor, SnapshotExportResult};
use crate::document::{AdapterRegistry, AssetScanner};
use crate::domain::{DocumentReference, Issue, State};
use crate::snapshot::{
    compute_sha256, AssetSnapshot, DocumentSnapshot, IssuesInfo, MetadataInfo, RepoInfo,
    SnapshotFormat, SnapshotManifest, SnapshotScope, SourceInfo, SourceMode, VerificationInfo,
};
use crate::storage::{IssueStore, RepositoryStateStore};
use anyhow::{anyhow, Context, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

type SnapshotDirectories = BTreeSet<crate::repository_state::RootRelativePath>;
type SnapshotFiles = BTreeMap<crate::repository_state::RootRelativePath, Vec<u8>>;
const MAX_REPOSITORY_SNAPSHOT_BYTES: u64 = 128 * 1024 * 1024;

fn origin_remote_url(repo: &git2::Repository) -> Option<String> {
    repo.find_remote("origin")
        .ok()
        .and_then(|remote| remote.url().ok().map(str::to_owned))
}

fn current_branch_name(repo: &git2::Repository) -> Option<String> {
    repo.head()
        .ok()
        .and_then(|head| head.shorthand().ok().map(str::to_owned))
}

struct SnapshotExporter<'a, S: IssueStore> {
    storage: &'a S,
    worktree_root: PathBuf,
    data_root: PathBuf,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone)]
struct CapturedFile {
    source: SourceInfo,
    hash_sha256: String,
    size_bytes: usize,
}

impl<S: IssueStore + RepositoryStateStore> CommandExecutor<S> {
    /// Determine source mode based on options and git availability
    pub fn determine_snapshot_source_mode(
        &self,
        at_commit: Option<&str>,
        working_tree: bool,
        committed_only: bool,
    ) -> Result<SourceMode> {
        let layout = self.require_layout()?;
        let open_repo = || {
            git2::Repository::open(layout.worktree_root())
                .map_err(|_| anyhow!("snapshot source requires git repository"))
        };
        let resolve = |reference: &str| -> Result<String> {
            let repo = open_repo()?;
            let oid = repo
                .revparse_single(reference)?
                .peel_to_commit()?
                .id()
                .to_string();
            Ok(oid)
        };
        match (at_commit, working_tree, committed_only) {
            (Some(_), true, _) => Err(anyhow!("Cannot use both --at and --working-tree")),
            (Some(commit), _, _) => Ok(SourceMode::Git {
                commit: resolve(commit).context("--at requires git repository")?,
            }),
            (_, true, _) => Ok(SourceMode::WorkingTree),
            (None, false, true) => Ok(SourceMode::Git {
                commit: resolve("HEAD").context("--committed-only requires git repository")?,
            }),
            (None, false, false) => Ok(resolve("HEAD")
                .map(|commit| SourceMode::Git { commit })
                .unwrap_or(SourceMode::WorkingTree)),
        }
    }
}

impl<'a, S: IssueStore> SnapshotExporter<'a, S> {
    fn new(
        storage: &'a S,
        worktree_root: PathBuf,
        data_root: PathBuf,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        Self {
            storage,
            worktree_root,
            data_root,
            created_at,
        }
    }

    /// Enumerate issues based on scope
    fn enumerate_issues(&self, scope: &SnapshotScope) -> Result<Vec<Issue>> {
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
    fn extract_documents(&self, issues: &[Issue]) -> Vec<DocumentReference> {
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

        let entry = tree.get_path(Path::new(path)).map_err(|_| {
            // Generic NotFoundError: a path-in-commit not-found has no dedicated
            // domain type. Still downcastable -> exit 3; message preserved verbatim.
            crate::errors::NotFoundError::new(format!(
                "Path '{}' not found in commit {}",
                path, reference
            ))
        })?;
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
        let relative = crate::repository_state::RootRelativePath::parse(path)?;
        let content = crate::storage::repository_state_store::read_repository_file_nofollow(
            &self.worktree_root,
            &relative,
        )?
        .ok_or_else(|| {
            crate::errors::NotFoundError::new(format!("Failed to read file: {}", path))
        })?;

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
                let repo = git2::Repository::open(&self.worktree_root)?;
                self.read_from_git(&repo, path, commit)
            }
            SourceMode::WorkingTree => self.read_from_filesystem(path),
        }
    }

    fn capture_file(
        &self,
        path: &str,
        mode: &SourceMode,
        base: &Path,
        captured: &mut HashMap<String, CapturedFile>,
    ) -> Result<(Vec<u8>, CapturedFile)> {
        let relative = crate::repository_state::RootRelativePath::parse(path)?;
        let destination = base.join(relative.as_path());
        if let Some(existing) = captured.get(path) {
            return Ok((std::fs::read(destination)?, existing.clone()));
        }
        let (content, source) = self.read_content(path, mode)?;
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(destination, &content)?;
        let captured_file = CapturedFile {
            source,
            hash_sha256: compute_sha256(&content),
            size_bytes: content.len(),
        };
        captured.insert(path.to_string(), captured_file.clone());
        Ok((content, captured_file))
    }

    /// Read, hash, and stage one document and its assets from the same bytes.
    fn create_document_snapshot(
        &self,
        doc_ref: &DocumentReference,
        mode: &SourceMode,
        base: &Path,
        captured: &mut HashMap<String, CapturedFile>,
    ) -> Result<DocumentSnapshot> {
        let (content, document) = self.capture_file(&doc_ref.path, mode, base, captured)?;

        // Scan for assets if not already in doc_ref
        // Create new adapter registry for scanning
        let registry = AdapterRegistry::with_builtins();
        let scanner = AssetScanner::new(registry, &self.worktree_root);
        let content_str = String::from_utf8_lossy(&content);
        let assets = scanner
            .discover_assets(Path::new(&doc_ref.path), &content_str)
            .unwrap_or_default();

        // Create asset snapshots
        let mut asset_snapshots = Vec::new();
        for asset in &assets {
            if let Some(resolved_path) = &asset.resolved_path {
                let asset_path = resolved_path.to_string_lossy();
                if let Ok((_, captured_asset)) =
                    self.capture_file(&asset_path, mode, base, captured)
                {
                    asset_snapshots.push(AssetSnapshot {
                        path: asset_path.into_owned(),
                        source: captured_asset.source,
                        hash_sha256: captured_asset.hash_sha256,
                        mime: AssetScanner::detect_mime_type(resolved_path)
                            .unwrap_or_else(|| "application/octet-stream".to_string()),
                        size_bytes: captured_asset.size_bytes,
                        shared: asset.is_shared,
                    });
                }
            }
        }

        Ok(DocumentSnapshot {
            path: doc_ref.path.clone(),
            format: doc_ref
                .format
                .clone()
                .unwrap_or_else(|| "markdown".to_string()),
            size_bytes: document.size_bytes,
            source: document.source,
            hash_sha256: document.hash_sha256,
            assets: asset_snapshots,
        })
    }

    /// Generate snapshot manifest with full provenance
    fn generate_manifest(
        &self,
        issues: &[Issue],
        docs: &[DocumentSnapshot],
        mode: &SourceMode,
        scope: &SnapshotScope,
    ) -> Result<SnapshotManifest> {
        // Get repository info
        let repo_info = self.get_repo_info(mode)?;

        // Count issues by state
        let mut states: HashMap<State, usize> = HashMap::new();
        for issue in issues {
            *states.entry(issue.state).or_insert(0) += 1;
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
            created_at: self.created_at.to_rfc3339(),
            created_by: format!("jit snapshot export v{}", env!("CARGO_PKG_VERSION")),
            repo: repo_info,
            scope: scope.to_string(),
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
        let path = self
            .worktree_root
            .canonicalize()
            .unwrap_or_else(|_| self.worktree_root.clone())
            .to_string_lossy()
            .to_string();

        match mode {
            SourceMode::Git { commit } => {
                let repo = git2::Repository::open(&self.worktree_root)?;

                // Get remote URL
                let remote = origin_remote_url(&repo);

                // Get branch name
                let branch = current_branch_name(&repo);

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
    fn generate_readme(&self, manifest: &SnapshotManifest) -> String {
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

    /// Copy .jit state files to snapshot
    fn copy_jit_state(&self, base: &Path, issues: &[Issue]) -> Result<()> {
        let jit_dir = base.join(".jit");
        std::fs::create_dir_all(jit_dir.join("issues"))?;

        // Copy issue files
        for issue in issues {
            let issue_json = serde_json::to_string_pretty(issue)?;
            let relative = crate::repository_state::RootRelativePath::parse(format!(
                ".jit/issues/{}.json",
                issue.id
            ))?;
            let issue_path = base.join(relative.as_path());
            std::fs::write(issue_path, issue_json)?;
        }

        for name in ["config.toml", "gates.toml"] {
            let relative = crate::repository_state::RootRelativePath::parse(name)?;
            if let Some(bytes) =
                crate::storage::repository_state_store::read_repository_file_nofollow(
                    &self.data_root,
                    &relative,
                )?
            {
                std::fs::write(jit_dir.join(name), bytes)?;
            }
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

    fn assemble(
        &self,
        scope: &SnapshotScope,
        mode: &SourceMode,
    ) -> Result<(tempfile::TempDir, SnapshotManifest, Vec<String>)> {
        let issues = self.enumerate_issues(scope)?;
        if issues.is_empty() {
            return Err(anyhow!("No issues found in scope: {}", scope));
        }
        let doc_refs = self.extract_documents(&issues);
        let temp_dir = tempfile::TempDir::new()?;
        let base = temp_dir.path();
        self.copy_jit_state(base, &issues)?;

        let mut doc_snapshots = Vec::new();
        let mut warnings = Vec::new();
        let mut captured = HashMap::new();
        for doc_ref in &doc_refs {
            match self.create_document_snapshot(doc_ref, mode, base, &mut captured) {
                Ok(snapshot) => doc_snapshots.push(snapshot),
                Err(e) => {
                    warnings.push(format!("Failed to snapshot {}: {}", doc_ref.path, e));
                }
            }
        }
        let manifest = self.generate_manifest(&issues, &doc_snapshots, mode, scope)?;
        let manifest_json = serde_json::to_string_pretty(&manifest)?;
        std::fs::write(base.join("manifest.json"), manifest_json)?;
        let readme = self.generate_readme(&manifest);
        std::fs::write(base.join("README.md"), readme)?;
        self.generate_checksums_file(base, &doc_snapshots)?;
        Ok((temp_dir, manifest, warnings))
    }

    /// Determine output path with default timestamp-based naming
    fn determine_output_path(&self, out_path: Option<&Path>) -> Result<std::path::PathBuf> {
        if let Some(path) = out_path {
            Ok(path.to_path_buf())
        } else {
            // Generate default name: snapshot-YYYYMMDD-HHMMSS
            let now = self.created_at.with_timezone(&chrono::Local);
            let default_name = format!("snapshot-{}", now.format("%Y%m%d-%H%M%S"));
            Ok(std::path::PathBuf::from(default_name))
        }
    }
}

#[derive(Debug)]
struct SnapshotTree {
    directories: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

impl SnapshotTree {
    fn scan(root: &Path) -> Result<Self> {
        fn visit(root: &Path, relative: &Path, tree: &mut SnapshotTree) -> Result<()> {
            let mut entries =
                std::fs::read_dir(root.join(relative))?.collect::<std::io::Result<Vec<_>>>()?;
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                let child = relative.join(entry.file_name());
                let metadata = entry.path().symlink_metadata()?;
                if metadata.file_type().is_symlink() {
                    anyhow::bail!("snapshot staging tree contains a symbolic link");
                }
                if metadata.is_dir() {
                    tree.directories.push(child.clone());
                    visit(root, &child, tree)?;
                } else if metadata.is_file() {
                    tree.files.push(child);
                } else {
                    anyhow::bail!("snapshot staging tree contains an unsupported entry");
                }
            }
            Ok(())
        }

        let mut tree = Self {
            directories: Vec::new(),
            files: Vec::new(),
        };
        visit(root, Path::new(""), &mut tree)?;
        tree.directories.sort_by(|left, right| {
            left.components()
                .count()
                .cmp(&right.components().count())
                .then(left.cmp(right))
        });
        tree.files.sort();
        Ok(tree)
    }

    fn repository_payload(&self, root: &Path) -> Result<(SnapshotDirectories, SnapshotFiles)> {
        let directories = self
            .directories
            .iter()
            .map(crate::repository_state::RootRelativePath::parse)
            .collect::<std::result::Result<BTreeSet<_>, _>>()?;
        let files = self
            .files
            .iter()
            .map(|relative| {
                Ok((
                    crate::repository_state::RootRelativePath::parse(relative)?,
                    std::fs::read(root.join(relative))?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        Ok((directories, files))
    }

    fn payload_bytes(&self, root: &Path) -> Result<u64> {
        self.files.iter().try_fold(0_u64, |total, relative| {
            total
                .checked_add(std::fs::metadata(root.join(relative))?.len())
                .ok_or_else(|| anyhow!("snapshot payload size overflow"))
        })
    }
}

fn ensure_repository_snapshot_size(size: u64) -> Result<()> {
    if size > MAX_REPOSITORY_SNAPSHOT_BYTES {
        return Err(crate::errors::InvalidArgumentError::new(format!(
            "Repository-contained snapshot payload is {size} bytes; the maximum is \
             {MAX_REPOSITORY_SNAPSHOT_BYTES} bytes. Export outside the repository for streamed publication."
        ))
        .into());
    }
    Ok(())
}

fn build_snapshot_tar(
    root: &Path,
    tree: &SnapshotTree,
    output: &Path,
) -> Result<tempfile::NamedTempFile> {
    use crate::tar_format::reproducible_tar_header;
    use std::io::Write as _;
    use tar::{Builder, EntryType};

    let mut staged = tempfile::NamedTempFile::new()?;
    let base = output
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("snapshot");
    {
        let mut archive = Builder::new(staged.as_file_mut());
        let append_directory =
            |archive: &mut Builder<&mut std::fs::File>, path: &Path| -> Result<()> {
                let mut header = reproducible_tar_header(EntryType::Directory, 0o755, 0);
                archive.append_data(&mut header, path, std::io::empty())?;
                Ok(())
            };
        append_directory(&mut archive, Path::new(base))?;
        for relative in &tree.directories {
            append_directory(&mut archive, &Path::new(base).join(relative))?;
        }
        for relative in &tree.files {
            let mut input = std::fs::File::open(root.join(relative))?;
            let mut header =
                reproducible_tar_header(EntryType::Regular, 0o644, input.metadata()?.len());
            archive.append_data(&mut header, Path::new(base).join(relative), &mut input)?;
        }
        archive.finish()?;
    }
    staged.as_file_mut().flush()?;
    staged.as_file().sync_all()?;
    Ok(staged)
}

impl<S: IssueStore + RepositoryStateStore> CommandExecutor<S> {
    /// Assemble and publish one snapshot through its classified destination.
    pub fn export_snapshot(
        &self,
        invocation_dir: &Path,
        scope: &SnapshotScope,
        mode: &SourceMode,
        format: &SnapshotFormat,
        out_path: Option<&Path>,
    ) -> Result<(SnapshotExportResult, Vec<String>)> {
        use crate::repository_state::{
            classify_repository_export, CaptureBudget, RepositoryExportDestination,
            RepositoryExportIntent,
        };

        let layout = self.require_layout()?;
        let exporter = SnapshotExporter::new(
            &self.storage,
            layout.worktree_root().to_path_buf(),
            layout.data_root().to_path_buf(),
            chrono::Utc::now(),
        );
        let output_path = exporter.determine_output_path(out_path)?;
        let (staging, manifest, mut warnings) = exporter.assemble(scope, mode)?;
        let tree = SnapshotTree::scan(staging.path())?;
        let destination = classify_repository_export(&layout, invocation_dir, &output_path)?;
        let budget = CaptureBudget {
            max_paths: tree
                .files
                .len()
                .saturating_add(tree.directories.len())
                .saturating_add(4096),
            max_listings: 1,
            max_bytes: 512 * 1024 * 1024,
            max_depth: 128,
        };

        let size_bytes = match format {
            SnapshotFormat::Directory => {
                match destination {
                    RepositoryExportDestination::Repository(target) => {
                        ensure_repository_snapshot_size(tree.payload_bytes(staging.path())?)?;
                        let (directories, files) = tree.repository_payload(staging.path())?;
                        let intent = RepositoryExportIntent::new_tree(target, directories, files);
                        super::map_occupied_export_error(
                            self.publish_repository_export(&layout, &intent, budget),
                            &output_path,
                        )?;
                    }
                    RepositoryExportDestination::External(path) => {
                        let outcome =
                            crate::storage::external_publish::publish_external_directory_noreplace(
                                &path,
                                staging.path(),
                                &tree.directories,
                                &tree.files,
                            )?;
                        warnings.extend(outcome.warnings);
                    }
                }
                None
            }
            SnapshotFormat::Tar => {
                let tar = build_snapshot_tar(staging.path(), &tree, &output_path)?;
                let size = tar.as_file().metadata()?.len();
                match destination {
                    RepositoryExportDestination::Repository(target) => {
                        ensure_repository_snapshot_size(size)?;
                        let bytes = std::fs::read(tar.path())?;
                        let intent = RepositoryExportIntent::new_absent_file(target, bytes);
                        super::map_occupied_export_error(
                            self.publish_repository_export(&layout, &intent, budget),
                            &output_path,
                        )?;
                    }
                    RepositoryExportDestination::External(path) => {
                        let outcome =
                            crate::storage::external_publish::publish_external_file_noreplace(
                                &path,
                                tar.path(),
                            )?;
                        warnings.extend(outcome.warnings);
                    }
                }
                Some(size)
            }
        };

        Ok((
            SnapshotExportResult {
                path: output_path.to_string_lossy().into_owned(),
                issue_count: manifest.issues.count,
                document_count: manifest.documents.count,
                format: match format {
                    SnapshotFormat::Directory => "directory",
                    SnapshotFormat::Tar => "tar",
                }
                .to_string(),
                size_bytes,
            },
            warnings,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::DocumentReference;
    use crate::snapshot::{IssuesInfo, MetadataInfo, RepoInfo, SnapshotManifest, VerificationInfo};
    use crate::storage::InMemoryStorage;
    use std::collections::HashMap;

    fn exporter(storage: &InMemoryStorage) -> SnapshotExporter<'_, InMemoryStorage> {
        SnapshotExporter::new(
            storage,
            PathBuf::from("."),
            PathBuf::from(".jit"),
            chrono::Utc::now(),
        )
    }

    #[test]
    fn test_enumerate_issues_all() {
        let storage = InMemoryStorage::new();

        // Create a couple of issues
        let issue1 = crate::domain::types::fixture_issue("Issue 1".to_string(), String::new());
        let issue2 = crate::domain::types::fixture_issue("Issue 2".to_string(), String::new());
        crate::commands::test_helpers::seed_issue(&storage, issue1.clone());
        crate::commands::test_helpers::seed_issue(&storage, issue2.clone());

        let exporter = exporter(&storage);
        let issues = exporter.enumerate_issues(&SnapshotScope::All).unwrap();

        assert_eq!(issues.len(), 2);
    }

    #[test]
    fn test_enumerate_issues_single() {
        let storage = InMemoryStorage::new();

        let issue = crate::domain::types::fixture_issue("Test Issue".to_string(), String::new());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);

        let exporter = exporter(&storage);
        let issues = exporter
            .enumerate_issues(&SnapshotScope::Issue(issue_id.clone()))
            .unwrap();

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, issue_id);
    }

    #[test]
    fn test_enumerate_issues_label_epic() {
        let storage = InMemoryStorage::new();

        // Create issues with epic labels
        let mut issue1 = crate::domain::types::fixture_issue("Issue 1".to_string(), String::new());
        issue1.labels.push("epic:auth".to_string());
        crate::commands::test_helpers::seed_issue(&storage, issue1.clone());

        let mut issue2 = crate::domain::types::fixture_issue("Issue 2".to_string(), String::new());
        issue2.labels.push("epic:auth".to_string());
        crate::commands::test_helpers::seed_issue(&storage, issue2.clone());

        // Create issue with different epic
        let mut issue3 = crate::domain::types::fixture_issue("Issue 3".to_string(), String::new());
        issue3.labels.push("epic:billing".to_string());
        crate::commands::test_helpers::seed_issue(&storage, issue3.clone());

        // Create unrelated issue
        let issue4 = crate::domain::types::fixture_issue("Issue 4".to_string(), String::new());
        crate::commands::test_helpers::seed_issue(&storage, issue4.clone());

        let exporter = exporter(&storage);
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

        let mut issue1 = crate::domain::types::fixture_issue("Issue 1".to_string(), String::new());
        issue1.labels.push("milestone:v1.0".to_string());
        crate::commands::test_helpers::seed_issue(&storage, issue1.clone());

        let mut issue2 = crate::domain::types::fixture_issue("Issue 2".to_string(), String::new());
        issue2.labels.push("milestone:v2.0".to_string());
        crate::commands::test_helpers::seed_issue(&storage, issue2.clone());

        let exporter = exporter(&storage);
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

        let issue = crate::domain::types::fixture_issue("Issue".to_string(), String::new());
        crate::commands::test_helpers::seed_issue(&storage, issue);

        let exporter = exporter(&storage);
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

        let mut issue1 = crate::domain::types::fixture_issue("Issue 1".to_string(), String::new());
        issue1.documents.push(DocumentReference {
            path: "docs/design.md".to_string(),
            commit: None,
            label: None,
            doc_type: None,
            format: None,
            assets: vec![],
        });
        crate::commands::test_helpers::seed_issue(&storage, issue1.clone());

        let mut issue2 = crate::domain::types::fixture_issue("Issue 2".to_string(), String::new());
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
        crate::commands::test_helpers::seed_issue(&storage, issue2.clone());

        let exporter = exporter(&storage);
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

        let exporter = exporter(&storage);
        let docs = exporter.extract_documents(&[]);

        assert_eq!(docs.len(), 0);
    }

    #[test]
    fn test_generate_readme() {
        let storage = InMemoryStorage::new();
        let exporter = exporter(&storage);

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
        let exporter = exporter(&storage);

        // With explicit path
        let explicit = std::path::Path::new("my-snapshot");
        let result = exporter.determine_output_path(Some(explicit)).unwrap();
        assert_eq!(result, explicit);

        // With default (timestamp-based)
        let default = exporter.determine_output_path(None).unwrap();
        let name = default.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("snapshot-"));
    }

    #[test]
    fn test_snapshot_tree_and_tar_are_deterministic() {
        let staging = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(staging.path().join("z/deep")).unwrap();
        std::fs::create_dir_all(staging.path().join("a")).unwrap();
        std::fs::write(staging.path().join("z/deep/item"), b"item").unwrap();
        std::fs::write(staging.path().join("a/file"), b"file").unwrap();

        let tree = SnapshotTree::scan(staging.path()).unwrap();
        assert_eq!(
            tree.directories,
            vec![
                PathBuf::from("a"),
                PathBuf::from("z"),
                PathBuf::from("z/deep")
            ]
        );
        assert_eq!(
            tree.files,
            vec![PathBuf::from("a/file"), PathBuf::from("z/deep/item")]
        );

        let first = build_snapshot_tar(staging.path(), &tree, Path::new("bundle.tar")).unwrap();
        let second = build_snapshot_tar(staging.path(), &tree, Path::new("bundle.tar")).unwrap();
        assert_eq!(
            std::fs::read(first.path()).unwrap(),
            std::fs::read(second.path()).unwrap()
        );
    }

    #[test]
    fn test_repository_snapshot_size_rejects_sparse_file_before_reading_payload() {
        let staging = tempfile::tempdir().unwrap();
        let oversized = staging.path().join("oversized");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_REPOSITORY_SNAPSHOT_BYTES + 1)
            .unwrap();
        let tree = SnapshotTree::scan(staging.path()).unwrap();
        let size = tree.payload_bytes(staging.path()).unwrap();
        let error = ensure_repository_snapshot_size(size).unwrap_err();
        assert!(error
            .downcast_ref::<crate::errors::InvalidArgumentError>()
            .is_some());
        assert!(error.to_string().contains("Export outside the repository"));
    }

    #[test]
    fn test_copy_jit_state_rejects_malformed_issue_id_without_escape() {
        let storage = InMemoryStorage::new();
        let exporter = exporter(&storage);
        let staging = tempfile::tempdir().unwrap();
        let mut issue = crate::domain::types::fixture_issue("Issue".into(), String::new());
        issue.id = "../../../escaped".into();

        assert!(exporter.copy_jit_state(staging.path(), &[issue]).is_err());
        assert!(!staging.path().join("escaped.json").exists());
    }

    #[test]
    fn test_git_metadata_extracts_remote_and_branch_with_fallible_text_accessors() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(temp.path()).unwrap();
        repo.remote("origin", "https://example.com/repository.git")
            .unwrap();
        repo.set_head("refs/heads/main").unwrap();

        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("JIT Test", "jit@example.com").unwrap();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "initial commit",
            &tree,
            &[],
        )
        .unwrap();

        assert_eq!(
            origin_remote_url(&repo).as_deref(),
            Some("https://example.com/repository.git")
        );
        assert_eq!(current_branch_name(&repo).as_deref(), Some("main"));
    }
}
