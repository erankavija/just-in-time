//! Snapshot export command implementation

use crate::commands::{ArchiveOps, SessionStep, SnapshotExportResult};
use crate::document::{AdapterRegistry, AssetScanner};
use crate::domain::identity::IssueId;
use crate::domain::{DocumentReference, Issue, State};
use crate::repository_state::{
    CaptureBudget, CaptureSpec, RepositoryEntry, RepositoryImage, VirtualPath,
};
use crate::snapshot::{
    compute_sha256, AssetSnapshot, DocumentSnapshot, IssuesInfo, MetadataInfo, RepoInfo,
    SnapshotFormat, SnapshotManifest, SnapshotScope, SourceInfo, SourceMode, VerificationInfo,
};
use crate::storage::git::{GitCheckout, GitObjectReader};
use crate::storage::{IssueStore, RepositoryStateStore};
use anyhow::{anyhow, Context, Result};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

type SnapshotDirectories = BTreeSet<crate::domain::relative_path::RootRelativePath>;
type SnapshotFiles = BTreeMap<crate::domain::relative_path::RootRelativePath, Vec<u8>>;
const MAX_REPOSITORY_SNAPSHOT_BYTES: u64 = 128 * 1024 * 1024;

struct SnapshotExporter {
    checkout: Option<GitCheckout>,
    object_reader: RefCell<Option<GitObjectReader>>,
    worktree_root: PathBuf,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone)]
struct CapturedFile {
    source: SourceInfo,
    hash_sha256: String,
    size_bytes: usize,
}

impl<S: IssueStore + RepositoryStateStore> ArchiveOps<'_, S> {
    /// Determine source mode based on options and git availability
    pub fn determine_snapshot_source_mode(
        &self,
        at_commit: Option<&str>,
        working_tree: bool,
        committed_only: bool,
    ) -> Result<SourceMode> {
        let checkout = self.exec.require_worktree_paths()?.code_checkout();
        let resolve = |reference: &str| -> Result<String> {
            let checkout = checkout
                .as_ref()
                .ok_or_else(|| anyhow!("snapshot source requires git repository"))?;
            Ok(checkout.resolve_commit(reference)?.as_str().to_owned())
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

/// The issue ids whose records `scope` reads: its one issue, or every issue
/// the captured listing names.
fn scope_issue_ids(image: &RepositoryImage, scope: &SnapshotScope) -> Result<Vec<IssueId>> {
    match scope {
        SnapshotScope::Issue(id) => Ok(vec![id.clone()]),
        SnapshotScope::All | SnapshotScope::Label { .. } => {
            Ok(crate::repository_state::captured_issue_ids(image)?)
        }
    }
}

/// The declaration files and each `*.json` file a captured declaration
/// directory lists; a directory with any other occupant is refused.
fn declaration_paths(image: &RepositoryImage) -> Result<Vec<VirtualPath>> {
    let listed = VirtualPath::DECLARATION_DIRECTORIES
        .iter()
        .map(|directory| match image.entry(directory)? {
            RepositoryEntry::Absent | RepositoryEntry::Directory { .. } => {
                Ok(image.listed_json_files(directory)?)
            }
            _ => Err(anyhow!(
                "declaration directory '{}' must be an ordinary directory",
                directory.repository_relative()
            )),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(VirtualPath::DECLARATION_FILES
        .into_iter()
        .chain(listed.into_iter().flatten())
        .collect())
}

/// Capture `scope`'s issue records and the declaration set in one image;
/// `None` is a retryable conflict.
fn capture_snapshot_image(
    session: &mut dyn crate::storage::RepositoryMutationSession,
    scope: &SnapshotScope,
) -> Result<Option<RepositoryImage>> {
    let directories = VirtualPath::DECLARATION_DIRECTORIES;
    // The issue listing and one listing per declaration directory.
    let budget = CaptureBudget {
        max_listings: 1 + directories.len(),
        max_bytes: super::CAPTURE_MAX_BYTES,
        max_depth: 32,
    };
    let mut spec = CaptureSpec::new(
        VirtualPath::DECLARATION_FILES
            .into_iter()
            .chain(directories.clone()),
        budget,
    )?;
    spec.discover_listings(directories)?;
    match scope {
        SnapshotScope::Issue(id) => spec.discover_paths([VirtualPath::issue_record(id)])?,
        SnapshotScope::All | SnapshotScope::Label { .. } => {
            spec.discover_listing(VirtualPath::ISSUES)?
        }
    }
    super::capture_discovering(session, spec, |image, spec| {
        Ok(spec.discover_paths(
            declaration_paths(image)?.into_iter().chain(
                scope_issue_ids(image, scope)?
                    .iter()
                    .map(VirtualPath::issue_record),
            ),
        )?)
    })
}

/// Select the captured `issues` `scope` exports.
fn enumerate_issues(issues: Vec<Issue>, scope: &SnapshotScope) -> Vec<Issue> {
    match scope {
        SnapshotScope::All => issues,
        SnapshotScope::Issue(id) => issues.into_iter().filter(|issue| &issue.id == id).collect(),
        SnapshotScope::Label { namespace, value } => {
            let target_label = format!("{}:{}", namespace, value);
            issues
                .into_iter()
                .filter(|issue| issue.labels.contains(&target_label))
                .collect()
        }
    }
}

/// Write the stored bytes of each issue record in `issues` and of each
/// declaration file `image` holds below `base`.
fn copy_jit_state(base: &Path, image: &RepositoryImage, issues: &[Issue]) -> Result<()> {
    issues
        .iter()
        .map(|issue| VirtualPath::issue_record(&issue.id))
        .chain(declaration_paths(image)?)
        .try_for_each(|path| {
            let Some(bytes) = image.file_bytes(&path)? else {
                return Ok(());
            };
            let relative =
                crate::domain::relative_path::RootRelativePath::parse(path.repository_relative())?;
            let destination = base.join(relative.as_path());
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(destination, bytes)?;
            Ok(())
        })
}

impl SnapshotExporter {
    fn new(
        checkout: Option<GitCheckout>,
        worktree_root: PathBuf,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        Self {
            checkout,
            object_reader: RefCell::new(None),
            worktree_root,
            created_at,
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

    /// Read file content from working tree filesystem
    fn read_from_filesystem(&self, path: &str) -> Result<(Vec<u8>, SourceInfo)> {
        let relative = crate::domain::relative_path::RootRelativePath::parse(path)?;
        let content = crate::storage::capability_fs::read_repository_file_nofollow(
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
                let checkout = self
                    .checkout
                    .as_ref()
                    .ok_or_else(|| anyhow!("snapshot source requires git repository"))?;
                let mut reader = self.object_reader.borrow_mut();
                if reader.is_none() {
                    *reader = Some(checkout.object_reader()?);
                }
                let reader = reader
                    .as_mut()
                    .ok_or_else(|| anyhow!("snapshot git object reader is unavailable"))?;
                let object_name = format!("{commit}:{path}");
                let object = reader.read(&object_name)?.ok_or_else(|| {
                    crate::errors::NotFoundError::new(format!(
                        "Path '{}' not found in commit {}",
                        path, commit
                    ))
                })?;
                if object.object_type() != crate::storage::git::GitObjectType::Blob {
                    return Err(anyhow!(
                        "Path '{}' in commit {} is not a file",
                        path,
                        commit
                    ));
                }

                Ok((
                    object.bytes().to_vec(),
                    SourceInfo {
                        type_: "git".to_string(),
                        commit: Some(commit.clone()),
                        blob_sha: Some(object.id().as_str().to_owned()),
                    },
                ))
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
        let relative = crate::domain::relative_path::RootRelativePath::parse(path)?;
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
                match self.capture_file(&asset_path, mode, base, captured) {
                    Ok((_, captured_asset)) => asset_snapshots.push(AssetSnapshot {
                        path: asset_path.into_owned(),
                        source: captured_asset.source,
                        hash_sha256: captured_asset.hash_sha256,
                        mime: AssetScanner::detect_mime_type(resolved_path)
                            .unwrap_or_else(|| "application/octet-stream".to_string()),
                        size_bytes: captured_asset.size_bytes,
                        shared: asset.is_shared,
                    }),
                    Err(error)
                        if matches!(mode, SourceMode::Git { .. })
                            && error.is::<crate::errors::NotFoundError>() =>
                    {
                        return Err(error);
                    }
                    Err(_) => {}
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
            .map(|issue| {
                crate::repository_state::VirtualPath::issue_record(&issue.id).repository_relative()
            })
            .collect();

        let issues_info = IssuesInfo {
            files: crate::domain::list_envelope::ListEnvelope::with_extras(
                issue_files,
                crate::snapshot::IssueStates { states },
            ),
        };

        // Build documents info
        let documents_info = crate::snapshot::DocumentsInfo {
            items: crate::domain::list_envelope::ListEnvelope::new(docs.to_vec()),
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
                let checkout = self
                    .checkout
                    .as_ref()
                    .ok_or_else(|| anyhow!("snapshot source requires git repository"))?;
                let remote = checkout.origin_url()?;
                let branch = checkout.head()?.map(|head| head.branch_label().to_owned());
                let dirty = !checkout.dirty_paths()?.is_empty();

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
- The repository's declaration files, as stored (.jit/)

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
            issue_count = manifest.issues.files.count(),
            doc_count = manifest.documents.items.count(),
        )
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

    /// Stage the snapshot of `scope` from `image` and the documents `mode` reads.
    fn assemble(
        &self,
        image: &RepositoryImage,
        scope: &SnapshotScope,
        mode: &SourceMode,
    ) -> Result<(tempfile::TempDir, SnapshotManifest, Vec<String>)> {
        let issues = enumerate_issues(
            super::captured_issues(image, &scope_issue_ids(image, scope)?)?,
            scope,
        );
        if issues.is_empty() {
            return Err(anyhow!("No issues found in scope: {}", scope));
        }
        let doc_refs = self.extract_documents(&issues);
        let temp_dir = tempfile::TempDir::new()?;
        let base = temp_dir.path();
        copy_jit_state(base, image, &issues)?;

        let mut doc_snapshots = Vec::new();
        let mut warnings = Vec::new();
        let mut captured = HashMap::new();
        for doc_ref in &doc_refs {
            match self.create_document_snapshot(doc_ref, mode, base, &mut captured) {
                Ok(snapshot) => doc_snapshots.push(snapshot),
                Err(error)
                    if matches!(mode, SourceMode::Git { .. })
                        && error.is::<crate::errors::NotFoundError>() =>
                {
                    return Err(error);
                }
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
            .map(crate::domain::relative_path::RootRelativePath::parse)
            .collect::<std::result::Result<BTreeSet<_>, _>>()?;
        let files = self
            .files
            .iter()
            .map(|relative| {
                Ok((
                    crate::domain::relative_path::RootRelativePath::parse(relative)?,
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

impl<S: IssueStore + RepositoryStateStore> ArchiveOps<'_, S> {
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
            classify_repository_export, RepositoryExportDestination, RepositoryExportIntent,
        };

        let layout = self.exec.require_layout()?;
        let exporter = SnapshotExporter::new(
            self.exec.require_worktree_paths()?.code_checkout(),
            layout.worktree_root().to_path_buf(),
            chrono::Utc::now(),
        );
        let output_path = exporter.determine_output_path(out_path)?;
        let image = super::with_mutation_session(
            &self.exec.storage,
            &layout,
            "snapshot export",
            |session| {
                Ok(match capture_snapshot_image(session, scope)? {
                    Some(image) => SessionStep::Done(image),
                    None => SessionStep::Retry,
                })
            },
        )?;
        let (staging, manifest, mut warnings) = exporter.assemble(&image, scope, mode)?;
        let tree = SnapshotTree::scan(staging.path())?;
        let destination = classify_repository_export(&layout, invocation_dir, &output_path)?;
        let budget = CaptureBudget {
            max_listings: 1,
            max_bytes: super::CAPTURE_MAX_BYTES,
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
                            self.exec
                                .publish_repository_export(&layout, &intent, budget),
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
                            self.exec
                                .publish_repository_export(&layout, &intent, budget),
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
                issue_count: manifest.issues.files.count(),
                document_count: manifest.documents.items.count(),
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
    use crate::snapshot::{
        DocumentSnapshot, IssuesInfo, MetadataInfo, RepoInfo, SnapshotManifest, SourceInfo,
        VerificationInfo,
    };
    use crate::storage::InMemoryStorage;
    use crate::test_utils::document_reference;
    use std::collections::HashMap;

    fn exporter() -> SnapshotExporter {
        SnapshotExporter::new(None, PathBuf::from("."), chrono::Utc::now())
    }

    fn labelled_issue(title: &str, labels: &[&str]) -> Issue {
        let mut issue = crate::test_utils::fixture_issue(title.to_string(), String::new());
        issue.labels = labels.iter().map(|label| label.to_string()).collect();
        issue
    }

    fn label_scope(namespace: &str, value: &str) -> SnapshotScope {
        SnapshotScope::Label {
            namespace: namespace.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn test_enumerate_issues_all() {
        let issues = vec![
            labelled_issue("Issue 1", &[]),
            labelled_issue("Issue 2", &[]),
        ];

        assert_eq!(
            enumerate_issues(issues.clone(), &SnapshotScope::All),
            issues
        );
    }

    #[test]
    fn test_enumerate_issues_single() {
        let issue = labelled_issue("Test Issue", &[]);
        let issues = vec![issue.clone(), labelled_issue("Other Issue", &[])];

        let selected = enumerate_issues(issues, &SnapshotScope::Issue(issue.id.clone()));

        assert_eq!(selected, vec![issue]);
    }

    #[test]
    fn test_enumerate_issues_label_epic() {
        let issues = vec![
            labelled_issue("Issue 1", &["epic:auth"]),
            labelled_issue("Issue 2", &["epic:auth"]),
            labelled_issue("Issue 3", &["epic:billing"]),
            labelled_issue("Issue 4", &[]),
        ];

        let selected = enumerate_issues(issues.clone(), &label_scope("epic", "auth"));

        assert_eq!(selected, issues[..2].to_vec());
    }

    #[test]
    fn test_enumerate_issues_label_milestone() {
        let issues = vec![
            labelled_issue("Issue 1", &["milestone:v1.0"]),
            labelled_issue("Issue 2", &["milestone:v2.0"]),
        ];

        let selected = enumerate_issues(issues.clone(), &label_scope("milestone", "v1.0"));

        assert_eq!(selected, issues[..1].to_vec());
    }

    #[test]
    fn test_enumerate_issues_label_no_matches() {
        let issues = vec![labelled_issue("Issue", &[])];

        assert!(enumerate_issues(issues, &label_scope("epic", "nonexistent")).is_empty());
    }

    #[test]
    fn test_copy_jit_state_rejects_symlinked_declaration() {
        let symlink = RepositoryEntry::Symlink {
            target: crate::repository_state::CapturedBytes::new(
                ".jit/rules.toml",
                b"../outside.toml".to_vec(),
            )
            .unwrap(),
            mode: crate::repository_state::FileMode::Regular,
        };
        let image = VirtualPath::DECLARATION_FILES
            .into_iter()
            .chain(VirtualPath::DECLARATION_DIRECTORIES)
            .fold(
                crate::test_harness::image::ImageBuilder::new(),
                |builder, path| builder.entry(path, RepositoryEntry::Absent),
            )
            .entry(VirtualPath::RULES, symlink);
        let image = VirtualPath::DECLARATION_DIRECTORIES
            .into_iter()
            .fold(image, |builder, directory| {
                builder.listing(
                    directory,
                    crate::repository_state::DirectoryListing::new([]).unwrap(),
                )
            })
            .build();
        let base = tempfile::tempdir().unwrap();

        let error = copy_jit_state(base.path(), &image, &[]).unwrap_err();

        assert!(matches!(
            error.downcast_ref::<crate::repository_state::CaptureError>(),
            Some(crate::repository_state::CaptureError::UnsafeCapturedOccupant(path))
                if *path == VirtualPath::RULES
        ));
    }

    #[test]
    fn test_extract_documents() {
        let mut issue1 = crate::test_utils::fixture_issue("Issue 1".to_string(), String::new());
        issue1.documents.push(document_reference("docs/design.md"));

        let mut issue2 = crate::test_utils::fixture_issue("Issue 2".to_string(), String::new());
        issue2.documents.push(document_reference("docs/impl.md"));
        // Duplicate document reference
        issue2.documents.push(document_reference("docs/design.md"));

        let exporter = exporter();
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
        let exporter = exporter();
        let docs = exporter.extract_documents(&[]);

        assert_eq!(docs.len(), 0);
    }

    #[test]
    fn test_generate_readme() {
        let exporter = exporter();

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
                files: crate::domain::list_envelope::ListEnvelope::with_extras(
                    (0..5)
                        .map(|index| format!(".jit/issues/{index}.json"))
                        .collect(),
                    crate::snapshot::IssueStates {
                        states: HashMap::new(),
                    },
                ),
            },
            documents: crate::snapshot::DocumentsInfo {
                items: crate::domain::list_envelope::ListEnvelope::new(
                    (0..10)
                        .map(|index| DocumentSnapshot {
                            path: format!("docs/{index}.md"),
                            format: "markdown".to_owned(),
                            size_bytes: 0,
                            source: SourceInfo {
                                type_: "filesystem".to_owned(),
                                commit: None,
                                blob_sha: None,
                            },
                            hash_sha256: String::new(),
                            assets: vec![],
                        })
                        .collect(),
                ),
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
        let exporter = exporter();

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
    fn test_determine_snapshot_source_mode_resolves_requested_commit_and_defaults_to_head() {
        let temp = tempfile::tempdir().unwrap();
        let repository = crate::test_harness::git::GitRepository::init(temp.path().join("code"));
        repository.write("tracked", "initial");
        let initial = repository.commit("unique needle");
        repository.write("tracked", "updated");
        let head = repository.commit("updated");
        let data = crate::test_harness::git::GitRepository::init(temp.path().join("data"));
        data.write("tracked", "unrelated")
            .commit("unrelated data history");
        let storage = InMemoryStorage::rooted_at(repository.root());
        let layout = storage.repository_layout();
        let executor = crate::commands::CommandExecutor::new(storage)
            .with_layout(layout)
            .unwrap()
            .with_worktree_paths(
                crate::storage::WorktreePaths::detect_from(data.root())
                    .unwrap()
                    .with_worktree_root(repository.root())
                    .unwrap(),
            );

        for (at, committed_only, expected) in [
            (Some(initial.as_str()), false, &initial),
            (Some(":/unique needle"), false, &initial),
            (None, true, &head),
            (None, false, &head),
        ] {
            let mode = executor
                .archive()
                .determine_snapshot_source_mode(at, false, committed_only)
                .unwrap();
            assert!(matches!(mode, SourceMode::Git { commit } if &commit == expected));
        }
        assert!(matches!(
            executor
                .archive()
                .determine_snapshot_source_mode(None, true, false)
                .unwrap(),
            SourceMode::WorkingTree
        ));
        assert!(executor
            .archive()
            .determine_snapshot_source_mode(Some(&initial), true, false)
            .is_err());
    }

    #[test]
    fn test_determine_snapshot_source_mode_without_checkout_uses_working_tree_and_rejects_git_options(
    ) {
        let temp = tempfile::tempdir().unwrap();
        let repository = crate::test_harness::git::GitRepository::init(temp.path());
        repository.write("tracked", "initial");
        repository.commit("initial");
        let storage = InMemoryStorage::rooted_at(repository.root());
        let layout = storage.repository_layout();
        let executor = crate::commands::CommandExecutor::new(storage)
            .with_layout(layout)
            .unwrap();

        assert!(matches!(
            executor
                .archive()
                .determine_snapshot_source_mode(None, false, false)
                .unwrap(),
            SourceMode::WorkingTree
        ));
        for (at, committed_only) in [(Some("HEAD"), false), (None, true)] {
            assert!(executor
                .archive()
                .determine_snapshot_source_mode(at, false, committed_only)
                .is_err());
        }
    }

    #[test]
    fn test_get_repo_info_ignored_file_exports_clean() {
        let temp = tempfile::tempdir().unwrap();
        let repository = crate::test_harness::git::GitRepository::init(temp.path());
        repository.write(".gitignore", "ignored\n");
        let commit = repository.commit("initial");
        repository.write("ignored", "local cache");
        let exporter = SnapshotExporter::new(
            crate::storage::WorktreePaths::detect_from(repository.root())
                .unwrap()
                .git(),
            repository.root().to_path_buf(),
            chrono::Utc::now(),
        );

        let info = exporter.get_repo_info(&SourceMode::Git { commit }).unwrap();

        assert!(
            !info.dirty,
            "ignored files must not make the snapshot dirty"
        );
    }

    #[test]
    fn test_get_repo_info_extracts_remote_and_head_branch_label() {
        let temp = tempfile::tempdir().unwrap();
        let repository = crate::test_harness::git::GitRepository::init(temp.path());
        repository.git(&[
            "remote",
            "add",
            "origin",
            "https://example.com/repository.git",
        ]);
        repository.git(&["branch", "--move", "main"]);
        let exporter = SnapshotExporter::new(
            crate::storage::WorktreePaths::detect_from(repository.root())
                .unwrap()
                .git(),
            repository.root().to_path_buf(),
            chrono::Utc::now(),
        );

        repository.write("tracked", "content");
        let commit = repository.commit("initial");
        let mode = SourceMode::Git { commit };
        let info = exporter.get_repo_info(&mode).unwrap();
        assert_eq!(
            info.remote.as_deref(),
            Some("https://example.com/repository.git")
        );
        assert_eq!(info.branch.as_deref(), Some("main"));

        repository.git(&["checkout", "--quiet", "--detach", "HEAD"]);
        let detached = exporter.get_repo_info(&mode).unwrap();
        assert_eq!(detached.branch.as_deref(), Some("HEAD"));

        repository.git(&["symbolic-ref", "HEAD", "refs/heads/unborn"]);
        let unborn = exporter.get_repo_info(&mode).unwrap();
        assert!(unborn.branch.is_none());
    }
}
