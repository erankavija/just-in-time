//! Document reference operations

use super::*;

const DOCUMENT_CAPTURE_BUDGET: crate::repository_state::CaptureBudget =
    crate::repository_state::CaptureBudget {
        max_paths: 256,
        max_listings: 0,
        max_bytes: 64 * 1024 * 1024,
        max_depth: 8,
    };

struct DerivedDocumentMutation<T> {
    outcome: T,
    intents: Vec<crate::repository_state::MutationIntent>,
}

struct CapturedDocumentScan {
    format: Option<String>,
    assets: Vec<crate::document::Asset>,
    required_paths: std::collections::BTreeSet<crate::repository_state::VirtualPath>,
    warning: Option<DocumentScanWarning>,
}

enum DocumentScanWarning {
    Missing(String),
    Failed(String),
}

enum DocumentScanSource {
    Worktree(crate::repository_state::VirtualPath),
    Pinned { revision: String, path: String },
}

impl DocumentScanWarning {
    fn message(&self) -> &str {
        match self {
            Self::Missing(message) | Self::Failed(message) => message,
        }
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    #[allow(clippy::too_many_arguments)] // CLI command parameters - refactoring would reduce clarity
    pub fn add_document_reference(
        &self,
        issue_id: &str,
        path: &str,
        commit: Option<&str>,
        label: Option<&str>,
        doc_type: Option<&str>,
        skip_scan: bool,
    ) -> Result<(DocumentAddResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        self.publish_document_add(&full_id, path, commit, label, doc_type, skip_scan)
    }

    pub fn list_document_references(
        &self,
        issue_id: &str,
    ) -> Result<crate::commands::DocumentListResult> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;

        Ok(crate::commands::DocumentListResult {
            issue_id: full_id,
            documents: issue.documents.clone(),
            count: issue.documents.len(),
        })
    }

    /// Resolve the canonical artifact directory `issue_id` owns in `area`.
    ///
    /// The caller names the area; the directory inside it is derived by
    /// [`resolve_artifact_directory`](crate::domain::artifact_directory::resolve_artifact_directory)
    /// from the issue's labels and the repository's configured type-to-namespace
    /// mapping, so no caller composes the name itself. The answer is a name:
    /// nothing is read from or written to the directory, and an issue whose
    /// artifacts still sit flat in the area resolves the same directory.
    ///
    /// # Errors
    ///
    /// An [`InvalidArgumentError`](crate::errors::InvalidArgumentError)
    /// carrying [`ArtifactDirectoryError`](crate::domain::artifact_directory::ArtifactDirectoryError)'s
    /// message when `area` is absent from the configured issue-scoped registry,
    /// so an undeclared area is an argument failure rather than a path. Also
    /// errors when `issue_id` resolves to no issue, when the issue cannot be
    /// loaded, or when the repository's configuration cannot be read.
    pub fn resolve_issue_artifact_directory(
        &self,
        issue_id: &str,
        area: &str,
    ) -> Result<crate::output::ArtifactDirectoryResponse> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;
        let documentation = self
            .config_manager
            .load()?
            .documentation
            .unwrap_or_default();
        let hierarchy = crate::config_manager::get_hierarchy_config(&self.storage)?;

        let directory = crate::domain::artifact_directory::resolve_artifact_directory(
            &issue,
            area,
            &documentation,
            &hierarchy,
        )
        .map_err(|error| crate::errors::InvalidArgumentError::new(error.to_string()))?;

        Ok(crate::output::ArtifactDirectoryResponse {
            short_id: issue.short_id(),
            issue_id: issue.id,
            area: area.to_string(),
            directory,
        })
    }

    /// Report the artifacts under the declared issue-scoped areas whose
    /// location disagrees with their owning issue's canonical directory.
    ///
    /// The walked registry is
    /// [`DocumentationConfig::issue_scoped_areas`](crate::config::DocumentationConfig::issue_scoped_areas)
    /// and nothing else, so a repository that declares no issue-scoped area is
    /// reported over no areas at all. Each area is listed with the archival
    /// planner's no-follow recursive walk at
    /// [`ArtifactListingScope::RecursiveEntries`](crate::domain::artifact_discovery::ArtifactListingScope::RecursiveEntries),
    /// which names the directories it descends through: a misplaced directory
    /// is an artifact whether or not it holds a file, and one holding nothing
    /// but further directories is reported the same way. The verdicts come from
    /// [`report_area_artifacts`](crate::domain::artifact_conformance::report_area_artifacts),
    /// which names the topmost offending component, so naming a directory and
    /// the files beneath it still reports that directory once.
    ///
    /// Read-only and advisory (`@/issue/8e071e18/decision/D-7`): the findings
    /// are the return value rather than a status a caller could gate on, and
    /// the run publishes, rewrites, and unlinks no repository content. The
    /// issue set the report resolves owners against — both the short ids those
    /// issues answer to and the document references they hold — is read through
    /// [`IssueStore::read_issues`](crate::storage::IssueStore::read_issues),
    /// which carries no index maintenance, rather than `list_issues`, which
    /// carries the sidecar sweep. A declared area that does not exist yet
    /// contributes no findings.
    ///
    /// # Errors
    ///
    /// An unreadable declared area, which is a failure of the walk rather than
    /// a finding, and the usual configuration and storage read failures.
    pub fn report_artifact_conformance(
        &self,
    ) -> Result<crate::output::ArtifactConformanceResponse> {
        use crate::domain::artifact_discovery::{ArtifactEvidence, ArtifactListingScope};
        use crate::domain::artifact_plan::normalize_artifact_path;

        let documentation = self
            .config_manager
            .load()?
            .documentation
            .unwrap_or_default();
        let hierarchy = crate::config_manager::get_hierarchy_config(&self.storage)?;
        let issues = self.storage.read_issues()?;
        // The declared registry, through the accessor that decides area
        // membership everywhere else: a declared entry that normalizes to
        // nothing names no area, so it is neither walked nor reported as
        // walked. The surviving entries are reported in their normalized
        // spelling, which is the spelling every finding carries.
        let areas = documentation
            .issue_scoped_areas()
            .into_iter()
            .filter(|area| documentation.is_issue_scoped_area(area))
            .map(|area| normalize_artifact_path(&area))
            .collect::<Vec<_>>();

        let artifacts = areas
            .iter()
            .map(|area| {
                let paths = match crate::storage::artifact_planning::inspect_artifact_evidence(
                    &self.storage,
                    area,
                    ArtifactListingScope::RecursiveEntries,
                )? {
                    ArtifactEvidence::Directory { entries, .. } => entries,
                    // An area that is absent, a file, or a symlink holds no
                    // per-issue artifact tree to walk.
                    _ => Vec::new(),
                };
                crate::domain::artifact_conformance::report_area_artifacts(
                    area,
                    &paths,
                    &issues,
                    &documentation,
                    &hierarchy,
                )
                .map_err(anyhow::Error::from)
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .map(crate::output::ArtifactConformanceEntry::from)
            .collect::<Vec<_>>();

        Ok(crate::output::ArtifactConformanceResponse {
            areas,
            count: artifacts.len(),
            artifacts,
        })
    }

    pub fn remove_document_reference(
        &self,
        issue_id: &str,
        path: &str,
    ) -> Result<DocumentRemoveResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        self.publish_document_remove(&full_id, path)
    }

    /// Add or refresh one document reference from a coherent repository image.
    /// Asset discovery may expand the capture once; a changed document that
    /// reveals additional assets restarts from a fresh session.
    #[allow(clippy::too_many_arguments)]
    fn publish_document_add(
        &self,
        issue_id: &str,
        path: &str,
        commit: Option<&str>,
        label: Option<&str>,
        doc_type: Option<&str>,
        skip_scan: bool,
    ) -> Result<(DocumentAddResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, MutationContext, VirtualPath};
        use crate::storage::validate_repo_relative_path;
        use std::collections::BTreeSet;

        validate_repo_relative_path(path)?;
        let layout = self.require_layout()?;
        let issue_path = VirtualPath::data(format!("issues/{issue_id}.json"))?;
        let events_path = VirtualPath::EVENTS;
        let document_path = VirtualPath::worktree(path)
            .map_err(|error| crate::storage::PathReadError::InvalidPath(error.to_string()))?;
        let source = match commit {
            Some(revision) => DocumentScanSource::Pinned {
                revision: revision.to_string(),
                path: path.to_string(),
            },
            None => DocumentScanSource::Worktree(document_path),
        };
        let context = MutationContext::production();

        with_mutation_session(&self.storage, &layout, "document add", |session| {
            let initial_paths = BTreeSet::from([issue_path.clone(), events_path.clone()]);
            let initial_spec = if skip_scan {
                document_capture_spec(initial_paths)?
            } else {
                document_scan_capture_spec(initial_paths, &source, &BTreeSet::new())?
            };
            let Some(image) = capture_or_retry(session.capture(initial_spec))? else {
                return Ok(SessionStep::Retry);
            };

            let initial_scan = if skip_scan {
                CapturedDocumentScan::empty()
            } else {
                scan_document_source(&layout, &image, path, &source)?
            };
            let captured_asset_paths = initial_scan.required_paths.clone();
            let (image, scan) = if captured_asset_paths.is_empty() {
                (image, initial_scan)
            } else {
                let expanded_paths = BTreeSet::from([issue_path.clone(), events_path.clone()]);
                let spec =
                    document_scan_capture_spec(expanded_paths, &source, &captured_asset_paths)?;
                let Some(image) = capture_or_retry(session.capture(spec))? else {
                    return Ok(SessionStep::Retry);
                };
                let scan = scan_document_source(&layout, &image, path, &source)?;
                if !scan.required_paths.is_subset(&captured_asset_paths) {
                    return Ok(SessionStep::Retry);
                }
                (image, scan)
            };
            let scan = hydrate_document_scan(&layout, &image, &source, scan)?;

            let issue = captured_issue(&image, &issue_path, issue_id)?;
            let derived = derive_document_add(
                issue,
                path,
                commit,
                label,
                doc_type,
                scan.format,
                scan.assets,
            );
            let warnings = scan
                .warning
                .as_ref()
                .map(|warning| warning.message().to_string())
                .into_iter()
                .collect();
            if derived.intents.is_empty() {
                return Ok(SessionStep::Done((derived.outcome, warnings)));
            }
            let plan = finalize(&layout, &image, &context, &derived.intents)?;
            Ok(SessionStep::Apply(plan, (derived.outcome, warnings)))
        })
    }

    /// Remove one path-selected document reference and its event atomically.
    fn publish_document_remove(&self, issue_id: &str, path: &str) -> Result<DocumentRemoveResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, MutationContext, VirtualPath};
        use std::collections::BTreeSet;

        let layout = self.require_layout()?;
        let issue_path = VirtualPath::data(format!("issues/{issue_id}.json"))?;
        let events_path = VirtualPath::EVENTS;
        let context = MutationContext::production();
        with_mutation_session(&self.storage, &layout, "document removal", |session| {
            let paths = BTreeSet::from([issue_path.clone(), events_path.clone()]);
            let Some(image) = capture_or_retry(session.capture(document_capture_spec(paths)?))?
            else {
                return Ok(SessionStep::Retry);
            };
            let issue = captured_issue(&image, &issue_path, issue_id)?;
            let derived = derive_document_remove(issue, path)?;
            let plan = finalize(&layout, &image, &context, &derived.intents)?;
            Ok(SessionStep::Apply(plan, derived.outcome))
        })
    }

    pub fn show_document_content(
        &self,
        issue_id: &str,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<DocumentContentResult> {
        use git2::Repository;

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;

        let doc = issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                crate::errors::NotFoundError::new(format!(
                    "Document reference {} not found in issue {}",
                    path, full_id
                ))
            })?;

        // Determine which commit to view
        let reference = if let Some(at) = at_commit {
            at
        } else if let Some(ref commit) = doc.commit {
            commit.as_str()
        } else {
            "HEAD"
        };

        // Read content: try git first, fall back to filesystem if git unavailable
        let content = if at_commit.is_some() || doc.commit.is_some() {
            // Explicit version requested - require git
            let repo = Repository::open(self.require_layout()?.worktree_root())
                .context("Git repository required when viewing specific commit version")?;
            self.read_file_from_git(&repo, &doc.path, reference)
                .with_context(|| format!("Failed to read {} from git at {}", doc.path, reference))?
        } else {
            // No specific version - try git, fall back to filesystem
            match Repository::open(self.require_layout()?.worktree_root()) {
                Ok(repo) => {
                    // Git available - read from git
                    self.read_file_from_git(&repo, &doc.path, "HEAD")
                        .with_context(|| format!("Failed to read {} from git", doc.path))?
                }
                Err(_) => {
                    // Git not available - read from filesystem via storage layer
                    self.storage
                        .read_path_text(&doc.path, None)
                        .map(|(text, _)| text)
                        .map_err(|e| {
                            anyhow!("Failed to read {} from filesystem: {}", doc.path, e)
                        })?
                }
            }
        };

        Ok(DocumentContentResult {
            path: doc.path.clone(),
            label: doc.label.clone(),
            commit: reference.to_string(),
            doc_type: doc.doc_type.clone(),
            content,
        })
    }

    pub fn document_history(
        &self,
        issue_id: &str,
        path: &str,
    ) -> Result<crate::commands::DocumentHistory> {
        use git2::Repository;

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;

        // Verify document reference exists
        issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                crate::errors::NotFoundError::new(format!(
                    "Document reference {} not found in issue {}",
                    path, full_id
                ))
            })?;

        let layout = self.require_layout()?;
        let repo = Repository::open(layout.worktree_root())
            .map_err(|e| anyhow!("Not a git repository: {}", e))?;

        let commits = self.get_file_history(&repo, path)?;

        Ok(crate::commands::DocumentHistory {
            path: path.to_string(),
            commits,
        })
    }

    pub fn document_diff(
        &self,
        issue_id: &str,
        path: &str,
        from: &str,
        to: Option<&str>,
    ) -> Result<DocumentDiffResult> {
        use git2::Repository;

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;

        // Verify document reference exists
        issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                crate::errors::NotFoundError::new(format!(
                    "Document reference {} not found in issue {}",
                    path, full_id
                ))
            })?;

        let layout = self.require_layout()?;
        let repo = Repository::open(layout.worktree_root())
            .map_err(|e| anyhow!("Not a git repository: {}", e))?;

        let to_ref = to.unwrap_or("HEAD");

        // Get content at both commits
        let from_content = self.read_file_from_git(&repo, path, from)?;
        let to_content = self.read_file_from_git(&repo, path, to_ref)?;

        // Generate unified diff
        let mut diff_output = String::new();
        diff_output.push_str(&format!("diff --git a/{} b/{}\n", path, path));
        diff_output.push_str(&format!("--- a/{} ({})\n", path, from));
        diff_output.push_str(&format!("+++ b/{} ({})\n", path, to_ref));
        diff_output.push('\n');

        // Use similar crate for diff generation
        use similar::{ChangeTag, TextDiff};
        let diff = TextDiff::from_lines(&from_content, &to_content);

        for change in diff.iter_all_changes() {
            let sign = match change.tag() {
                ChangeTag::Delete => "-",
                ChangeTag::Insert => "+",
                ChangeTag::Equal => " ",
            };
            diff_output.push_str(&format!("{}{}", sign, change));
        }

        Ok(DocumentDiffResult {
            path: path.to_string(),
            from_commit: from.to_string(),
            to_commit: to_ref.to_string(),
            diff: diff_output,
        })
    }

    /// Load an issue, returning a typed [`PathReadError`].
    ///
    /// Delegates to [`IssueStore::load_issue_or_not_found`], which each
    /// backend implements without string-matching: `JsonFileStorage` checks for
    /// file existence structurally, and `InMemoryStorage` checks the HashMap.
    fn load_issue_typed(
        &self,
        issue_id: &str,
    ) -> Result<crate::domain::Issue, crate::storage::PathReadError> {
        self.storage.load_issue_or_not_found(issue_id)
    }

    /// Read document content from git or filesystem.
    ///
    /// Note: This method is part of the public API used by jit-server.
    /// It's not called from the CLI binary, hence the dead_code warning.
    ///
    /// Returns typed [`PathReadError`] so route handlers can distinguish 404
    /// (file or document reference not found) from 500 (storage failure)
    /// without string-matching on error messages.
    #[allow(dead_code)]
    pub fn read_document_content(
        &self,
        issue_id: &str,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<(String, String), crate::storage::PathReadError> {
        use crate::storage::PathReadError;

        let issue = self.load_issue_typed(issue_id)?;

        let doc = issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                PathReadError::NotFound(format!(
                    "Document reference {} not found in issue {}",
                    path, issue_id
                ))
            })?;

        // Prefer explicit at_commit; fall back to the doc's pinned commit; then
        // None (working-tree read).
        let effective_commit = at_commit.or(doc.commit.as_deref());

        // The storage layer now enforces repo-relative paths and canonicalizes
        // working-tree reads against the repo root (see
        // `JsonFileStorage::read_path_bytes`), so we pass the stored path
        // through unchanged.
        self.storage.read_path_text(&doc.path, effective_commit)
    }

    /// Read raw document bytes for an issue-scoped path (byte-faithful variant).
    ///
    /// Unlike [`read_document_content`], this method does not convert file
    /// content to `String`, so binary artifacts are round-tripped without loss.
    ///
    /// Steps:
    /// 1. Load the issue and verify that `path` is linked as a `DocumentReference`.
    /// 2. Resolve the effective commit: explicit `at_commit` → doc's pinned commit → `None`.
    /// 3. For working-tree reads, resolve the document path relative to the repo
    ///    root so callers do not need to know the process CWD.
    /// 4. Delegate to `IssueStore::read_path_bytes` with the resolved path and commit.
    ///
    /// Note: Part of public API used by jit-server.
    #[allow(dead_code)]
    pub fn read_document_bytes(
        &self,
        issue_id: &str,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<(Vec<u8>, String), crate::storage::PathReadError> {
        use crate::storage::PathReadError;

        let issue = self.load_issue_typed(issue_id)?;

        let doc = issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                PathReadError::NotFound(format!(
                    "Document reference {} not found in issue {}",
                    path, issue_id
                ))
            })?;

        // Prefer explicit at_commit; fall back to the doc's pinned commit; then
        // None (working-tree read).
        let effective_commit = at_commit.or(doc.commit.as_deref());

        // The storage layer now enforces repo-relative paths uniformly (see
        // `JsonFileStorage::read_path_bytes`): empty, absolute, or `..`-bearing
        // paths are rejected with `PathReadError::InvalidPath`, and working-tree
        // reads are canonicalized + containment-checked against the repo root.
        // That makes pre-resolving to an absolute path here both unnecessary
        // and actively harmful, so we simply pass the stored repo-relative path
        // through.
        self.storage.read_path_bytes(&doc.path, effective_commit)
    }

    /// Get document history from git.
    ///
    /// Note: Part of public API used by jit-server.
    ///
    /// Returns [`PathReadError::NotFound`] when the document reference does not
    /// exist on the issue so that route handlers can return HTTP 404 without
    /// string-matching on the error message.
    #[allow(dead_code)]
    pub fn get_document_history(
        &self,
        issue_id: &str,
        path: &str,
    ) -> Result<Vec<CommitInfo>, crate::storage::PathReadError> {
        use crate::storage::PathReadError;
        use git2::Repository;

        let issue = self.load_issue_typed(issue_id)?;

        // Verify document reference exists
        issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                PathReadError::NotFound(format!(
                    "Document reference {} not found in issue {}",
                    path, issue_id
                ))
            })?;

        let layout = self.require_layout().map_err(PathReadError::Other)?;
        let repo_root = layout.worktree_root();

        // Try to get history from git, return empty list if not available
        if let Ok(repo) = Repository::open(repo_root) {
            if let Ok(history) = self.get_file_history(&repo, path) {
                return Ok(history);
            }
        }

        // No git or no history available - return empty list
        Ok(Vec::new())
    }

    /// Get diff between document versions.
    ///
    /// Note: Part of public API used by jit-server.
    ///
    /// Returns [`PathReadError::NotFound`] when the document reference does not
    /// exist on the issue so that route handlers can return HTTP 404 without
    /// string-matching on the error message.
    #[allow(dead_code)]
    pub fn get_document_diff(
        &self,
        issue_id: &str,
        path: &str,
        from: &str,
        to: Option<&str>,
    ) -> Result<String, crate::storage::PathReadError> {
        use crate::storage::PathReadError;
        use git2::Repository;
        use similar::{ChangeTag, TextDiff};

        let issue = self.load_issue_typed(issue_id)?;

        // Verify document reference exists
        issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                PathReadError::NotFound(format!(
                    "Document reference {} not found in issue {}",
                    path, issue_id
                ))
            })?;

        let layout = self.require_layout().map_err(PathReadError::Other)?;
        let repo_root = layout.worktree_root();

        // Try to get diff from git, return error message if not available
        if let Ok(repo) = Repository::open(repo_root) {
            let to_ref = to.unwrap_or("HEAD");

            // Try to get content at both commits
            if let (Ok(from_content), Ok(to_content)) = (
                self.read_file_from_git(&repo, path, from),
                self.read_file_from_git(&repo, path, to_ref),
            ) {
                // Generate unified diff
                let mut diff_output = format!("diff --git a/{} b/{}\n", path, path);
                diff_output.push_str(&format!("--- a/{} ({})\n", path, from));
                diff_output.push_str(&format!("+++ b/{} ({})\n\n", path, to_ref));

                let diff = TextDiff::from_lines(&from_content, &to_content);

                for change in diff.iter_all_changes() {
                    let sign = match change.tag() {
                        ChangeTag::Delete => "-",
                        ChangeTag::Insert => "+",
                        ChangeTag::Equal => " ",
                    };
                    diff_output.push_str(&format!("{}{}", sign, change));
                }

                return Ok(diff_output);
            }
        }

        // No git or diff not available
        Err(PathReadError::Other(anyhow!(
            "Document diff not available (requires git repository with history)"
        )))
    }

    /// Get all document paths referenced by issues.
    ///
    /// Note: Part of public API used by jit-server search functionality.
    #[allow(dead_code)]
    pub fn get_linked_document_paths(&self) -> Result<Vec<String>> {
        let issues = self.storage.list_issues()?;

        let mut paths = std::collections::HashSet::new();
        for issue in issues {
            for doc in &issue.documents {
                paths.insert(doc.path.clone());
            }
        }

        let mut result: Vec<String> = paths.into_iter().collect();
        result.sort();

        Ok(result)
    }

    /// Read a file as raw bytes from the repository, optionally at a git commit.
    ///
    /// Delegates to `IssueStore::read_path_bytes` so that all filesystem/git I/O
    /// stays in the storage layer.  Returns the raw byte content and a
    /// commit-hash string (`"working-tree"` for filesystem reads).
    ///
    /// Used by the server's raw document endpoints to serve binary-faithful
    /// content without UTF-8 conversion.
    pub fn read_path_bytes(
        &self,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<(Vec<u8>, String), crate::storage::PathReadError> {
        self.storage.read_path_bytes(path, at_commit)
    }

    fn read_file_from_git(
        &self,
        repo: &git2::Repository,
        path: &str,
        reference: &str,
    ) -> Result<String> {
        let obj = repo.revparse_single(reference)?;
        let commit = obj.peel_to_commit()?;
        let tree = commit.tree()?;
        let entry = tree.get_path(std::path::Path::new(path))?;
        let blob = repo.find_blob(entry.id())?;

        let content = std::str::from_utf8(blob.content())?;
        Ok(content.to_string())
    }

    fn get_file_history(&self, repo: &git2::Repository, path: &str) -> Result<Vec<CommitInfo>> {
        let mut revwalk = repo.revwalk()?;
        revwalk.push_head()?;

        let mut commits = Vec::new();
        let file_path = std::path::Path::new(path);

        for oid in revwalk {
            let oid = oid?;
            let commit = repo.find_commit(oid)?;

            // Check if this commit touches the file
            let tree = commit.tree()?;
            if tree.get_path(file_path).is_ok() {
                // Check if this commit modified the file (not just has it)
                let parent_count = commit.parent_count();
                let mut modified = parent_count == 0; // Root commit always counts

                if !modified && parent_count > 0 {
                    let parent = commit.parent(0)?;
                    let parent_tree = parent.tree()?;

                    // Compare file content with parent
                    let current_entry = tree.get_path(file_path).ok();
                    let parent_entry = parent_tree.get_path(file_path).ok();

                    modified = match (current_entry, parent_entry) {
                        (Some(curr), Some(par)) => curr.id() != par.id(),
                        (Some(_), None) => true, // File added
                        _ => false,
                    };
                }

                if modified {
                    let author = commit.author();
                    let time = commit.time();
                    let datetime =
                        chrono::DateTime::from_timestamp(time.seconds(), 0).unwrap_or_default();

                    commits.push(CommitInfo {
                        sha: format!("{:.7}", oid),
                        author: author.name().unwrap_or("Unknown").to_string(),
                        date: datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
                        message: commit.message().unwrap_or("").trim().to_string(),
                    });
                }
            }
        }

        Ok(commits)
    }

    pub fn list_document_assets(
        &self,
        issue_id: &str,
        path: &str,
        rescan: bool,
    ) -> Result<crate::commands::AssetListResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::document::AssetType;
        use anyhow::anyhow;

        let mut warnings = Vec::new();

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;

        // Find the document in the issue
        let document = issue
            .documents
            .iter()
            .find(|document| document.path == path)
            .ok_or_else(|| anyhow!("Document '{}' not linked to issue {}", path, issue_id))?;

        // Rescan if requested
        let assets = if rescan {
            self.rescan_document_assets(&full_id, path, &mut warnings)?
        } else {
            document.assets.clone()
        };

        // Categorize assets
        let total = assets.len();
        let per_doc_count = assets
            .iter()
            .filter(|a| !a.is_shared && a.asset_type == AssetType::Local)
            .count();
        let shared_count = assets
            .iter()
            .filter(|a| a.is_shared && a.asset_type == AssetType::Local)
            .count();
        let external_count = assets
            .iter()
            .filter(|a| a.asset_type == AssetType::External)
            .count();
        let missing_count = assets
            .iter()
            .filter(|a| a.asset_type == AssetType::Missing)
            .count();

        Ok(crate::commands::AssetListResult {
            issue_id: full_id,
            document_path: path.to_string(),
            count: total,
            assets,
            summary: crate::commands::AssetSummary {
                total,
                per_doc: per_doc_count,
                shared: shared_count,
                external: external_count,
                missing: missing_count,
            },
            warnings,
        })
    }

    /// Rescan one unpinned document from a coherent captured image and publish
    /// the changed inventory plus its audit event in the same delta. Identical
    /// inventories are a true no-op: no id, timestamp, event, or issue bytes.
    fn rescan_document_assets(
        &self,
        issue_id: &str,
        path: &str,
        warnings: &mut Vec<String>,
    ) -> Result<Vec<crate::document::Asset>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, MutationContext, MutationIntent, VirtualPath};

        let layout = self.require_layout()?;
        let issue_path = VirtualPath::data(format!("issues/{issue_id}.json"))?;
        let document_path = VirtualPath::worktree(path)?;
        let events_path = VirtualPath::EVENTS;
        let source = DocumentScanSource::Worktree(document_path);

        // Operation-scoped so a fresh-session retry cannot resample the update
        // timestamp or audit-event identity.
        let context = MutationContext::production();
        with_mutation_session(&self.storage, &layout, "document rescan", |session| {
            let initial = document_scan_capture_spec(
                [issue_path.clone(), events_path.clone()]
                    .into_iter()
                    .collect(),
                &source,
                &Default::default(),
            )?;
            let Some(image) = capture_or_retry(session.capture(initial))? else {
                return Ok(SessionStep::Retry);
            };
            let issue = captured_issue(&image, &issue_path, issue_id)?;
            let current_assets = issue
                .documents
                .iter()
                .find(|document| document.path == path)
                .ok_or_else(|| anyhow!("document '{path}' changed during rescan"))?
                .assets
                .clone();
            let initial_scan = scan_document_source(&layout, &image, path, &source)?;
            if let Some(warning) = initial_scan.warning {
                warnings.push(warning.message().to_string());
                return Ok(SessionStep::Done(match warning {
                    DocumentScanWarning::Missing(_) => current_assets,
                    DocumentScanWarning::Failed(_) => Vec::new(),
                }));
            }
            let expanded_paths = initial_scan.required_paths;
            let expanded = document_scan_capture_spec(
                [issue_path.clone(), events_path.clone()]
                    .into_iter()
                    .collect(),
                &source,
                &expanded_paths,
            )?;
            let Some(image) = capture_or_retry(session.capture(expanded))? else {
                return Ok(SessionStep::Retry);
            };
            let mut issue = captured_issue(&image, &issue_path, issue_id)?;
            let document = issue
                .documents
                .iter_mut()
                .find(|document| document.path == path)
                .ok_or_else(|| anyhow!("document '{path}' changed during rescan"))?;
            let scan = scan_document_source(&layout, &image, path, &source)?;
            if !scan.required_paths.is_subset(&expanded_paths) {
                return Ok(SessionStep::Retry);
            }
            if let Some(warning) = scan.warning {
                warnings.push(warning.message().to_string());
                return Ok(SessionStep::Done(match warning {
                    DocumentScanWarning::Missing(_) => document.assets.clone(),
                    DocumentScanWarning::Failed(_) => Vec::new(),
                }));
            }
            let scanned = hydrate_document_scan(&layout, &image, &source, scan)?.assets;
            if document.assets == scanned {
                return Ok(SessionStep::Done(scanned));
            }
            document.assets = scanned.clone();
            let intents = vec![
                MutationIntent::UpdateIssue {
                    issue: Box::new(issue),
                },
                MutationIntent::RecordEvent {
                    phase: 1,
                    event: Box::new(crate::domain::Event::draft_issue_updated(
                        issue_id.to_string(),
                        "doc-rescan".to_string(),
                        vec!["documents.assets".to_string()],
                    )),
                },
            ];
            let plan = finalize(&layout, &image, &context, &intents)?;
            Ok(SessionStep::Apply(plan, scanned))
        })
    }

    /// Validate that an external URL is reachable
    ///
    /// Returns Ok(true) if URL is reachable, Ok(false) if not reachable,
    /// or Err if validation failed (network error, timeout, etc.)
    fn validate_external_url(url: &str) -> Result<bool> {
        // Quick HEAD request
        // In ureq 3.x, use Agent for configuration
        let agent = ureq::Agent::new_with_defaults();
        let response = agent.head(url).call();

        match response {
            Ok(resp) => {
                // Check if status is success or redirection (2xx or 3xx)
                let status = resp.status();
                Ok(status.is_success() || status.is_redirection())
            }
            Err(_e) => {
                // Any error (IO, DNS, SSL, HTTP 4xx/5xx, etc.) means unreachable
                // Note: ureq 3.x doesn't distinguish between error types easily
                // We consider all errors as "unreachable"
                Ok(false)
            }
        }
    }

    /// Check document links and assets for validity
    pub fn check_document_links(
        &self,
        scope: &crate::document::DocumentScope,
    ) -> Result<crate::commands::LinkCheckResult> {
        use crate::document::{AssetType, DocumentScope, LinkValidationResult, LinkValidator};
        use std::path::PathBuf;

        let layout = self.require_layout()?;
        let repo_root = layout.worktree_root();

        // Resolve the scope to the set of issues whose documents to check.
        let issues = match scope {
            DocumentScope::All => self.storage.list_issues()?,
            DocumentScope::Issue(issue_id) => {
                let full_id = self.storage.resolve_issue_id(issue_id)?;
                let issue = self.storage.load_issue(&full_id)?;
                vec![issue]
            }
        };

        // Collect all documents to check
        let mut all_documents = Vec::new();
        let mut all_document_paths = Vec::new();
        for issue in &issues {
            for doc in &issue.documents {
                all_documents.push((issue.id.clone(), doc));
                all_document_paths.push(PathBuf::from(&doc.path));
            }
        }

        if all_documents.is_empty() {
            return Ok(crate::commands::LinkCheckResult {
                valid: true,
                errors: Vec::new(),
                warnings: Vec::new(),
                exit_code: 0,
                scope: scope.to_string(),
                summary: crate::commands::LinkCheckSummary {
                    total_documents: 0,
                    valid: 0,
                    errors: 0,
                    warnings: 0,
                },
            });
        }

        // Create link validator
        let link_validator = LinkValidator::new(repo_root.to_path_buf(), all_document_paths);

        // Try to open git repository for checking versioned assets
        let git_repo = git2::Repository::discover(repo_root).ok();

        // Validate each document
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        for (issue_id, doc) in &all_documents {
            let doc_path = repo_root.join(&doc.path);

            // Check if document file exists
            if !doc_path.exists() {
                errors.push(serde_json::json!({
                    "issue_id": issue_id,
                    "document": doc.path,
                    "type": "missing_document",
                    "message": format!("Document file not found: {}", doc.path),
                }));
                continue;
            }

            // Check assets
            for asset in &doc.assets {
                match asset.asset_type {
                    AssetType::Local => {
                        if let Some(ref resolved) = asset.resolved_path {
                            let asset_path = repo_root.join(resolved);
                            let exists_in_working_tree = asset_path.exists();
                            let exists_in_git = if !exists_in_working_tree {
                                // Check if asset exists in git
                                check_asset_in_git(&git_repo, resolved)
                            } else {
                                false
                            };

                            if !exists_in_working_tree && !exists_in_git {
                                errors.push(serde_json::json!({
                                    "issue_id": issue_id,
                                    "document": doc.path,
                                    "type": "missing_asset",
                                    "asset": asset.original_path,
                                    "resolved": resolved.display().to_string(),
                                    "message": format!("Asset not found: {}", asset.original_path),
                                }));
                            } else {
                                // Check if path is risky (deep relative traversal)
                                let parent_count = asset.original_path.matches("../").count();
                                if parent_count >= 2 {
                                    warnings.push(serde_json::json!({
                                        "issue_id": issue_id,
                                        "document": doc.path,
                                        "type": "risky_asset_path",
                                        "asset": asset.original_path,
                                        "message": format!(
                                            "Deep relative path '{}' may break if document is moved",
                                            asset.original_path
                                        ),
                                    }));
                                }
                            }
                        }
                    }
                    AssetType::Missing => {
                        errors.push(serde_json::json!({
                            "issue_id": issue_id,
                            "document": doc.path,
                            "type": "missing_asset",
                            "asset": asset.original_path,
                            "message": format!("Asset classified as missing: {}", asset.original_path),
                        }));
                    }
                    AssetType::External => {
                        // Validate external URLs
                        match Self::validate_external_url(&asset.original_path) {
                            Ok(true) => {
                                // URL is reachable - all good
                            }
                            Ok(false) => {
                                // URL exists but returned error or is unreachable
                                errors.push(serde_json::json!({
                                    "issue_id": issue_id,
                                    "document": doc.path,
                                    "type": "unreachable_url",
                                    "asset": asset.original_path,
                                    "message": format!("External URL is unreachable: {}", asset.original_path),
                                }));
                            }
                            Err(e) => {
                                // Validation failed (network error, timeout, etc.)
                                warnings.push(serde_json::json!({
                                    "issue_id": issue_id,
                                    "document": doc.path,
                                    "type": "url_validation_failed",
                                    "asset": asset.original_path,
                                    "message": format!(
                                        "Could not validate external URL ({}): {}",
                                        e, asset.original_path
                                    ),
                                }));
                            }
                        }
                    }
                }
            }

            // Check internal document links
            let doc_path_rel = PathBuf::from(&doc.path);
            match link_validator.scan_document_links(&doc_path_rel) {
                Ok(links) => {
                    for link in links {
                        match link_validator.validate_link(&doc_path_rel, &link) {
                            LinkValidationResult::Broken { reason } => {
                                errors.push(serde_json::json!({
                                    "issue_id": issue_id,
                                    "document": doc.path,
                                    "type": "broken_link",
                                    "link": link.target,
                                    "line": link.line_number,
                                    "message": reason,
                                }));
                            }
                            LinkValidationResult::Risky { warning } => {
                                warnings.push(serde_json::json!({
                                    "issue_id": issue_id,
                                    "document": doc.path,
                                    "type": "risky_link",
                                    "link": link.target,
                                    "line": link.line_number,
                                    "message": warning,
                                }));
                            }
                            LinkValidationResult::Valid => {}
                        }
                    }
                }
                Err(e) => {
                    warnings.push(serde_json::json!({
                        "issue_id": issue_id,
                        "document": doc.path,
                        "type": "scan_error",
                        "message": format!("Failed to scan document for links: {}", e),
                    }));
                }
            }
        }

        // Count unique documents with errors (a document might have multiple errors)
        let mut error_docs = std::collections::HashSet::new();
        for error in &errors {
            if let Some(doc) = error["document"].as_str() {
                error_docs.insert(doc);
            }
        }

        let error_count = error_docs.len();
        let warning_count = warnings.len();

        // Determine exit code
        let exit_code = if !errors.is_empty() {
            1 // Errors found
        } else if !warnings.is_empty() {
            2 // Only warnings
        } else {
            0 // All valid
        };

        Ok(crate::commands::LinkCheckResult {
            valid: errors.is_empty(),
            errors,
            warnings,
            exit_code,
            scope: scope.to_string(),
            summary: crate::commands::LinkCheckSummary {
                total_documents: all_documents.len(),
                valid: all_documents.len() - error_count,
                errors: error_count,
                warnings: warning_count,
            },
        })
    }
}

impl CapturedDocumentScan {
    fn empty() -> Self {
        Self {
            format: None,
            assets: Vec::new(),
            required_paths: std::collections::BTreeSet::new(),
            warning: None,
        }
    }
}

fn document_capture_spec(
    paths: std::collections::BTreeSet<crate::repository_state::VirtualPath>,
) -> Result<crate::repository_state::CaptureSpec> {
    use crate::repository_state::{CaptureSpec, RepositoryRootClass};

    let (worktree_paths, data_paths): (Vec<_>, Vec<_>) = paths
        .into_iter()
        .partition(|path| path.root_class() == RepositoryRootClass::Worktree);
    let mut spec = CaptureSpec::phase_one(data_paths, DOCUMENT_CAPTURE_BUDGET)?;
    spec.discover_paths(worktree_paths)?;
    Ok(spec)
}

fn document_scan_capture_spec(
    data_paths: std::collections::BTreeSet<crate::repository_state::VirtualPath>,
    source: &DocumentScanSource,
    asset_paths: &std::collections::BTreeSet<crate::repository_state::VirtualPath>,
) -> Result<crate::repository_state::CaptureSpec> {
    let mut spec = document_capture_spec(data_paths)?;
    match source {
        DocumentScanSource::Worktree(document_path) => {
            spec.discover_paths(asset_paths.iter().cloned().chain([document_path.clone()]))?
        }
        DocumentScanSource::Pinned { revision, path } => {
            spec.discover_pinned(revision, path)?;
            for asset_path in asset_paths {
                spec.discover_pinned(revision, asset_path.relative().as_path().to_string_lossy())?;
            }
        }
    }
    Ok(spec)
}

fn captured_issue(
    image: &crate::repository_state::RepositoryImage,
    issue_path: &crate::repository_state::VirtualPath,
    expected_id: &str,
) -> Result<Issue> {
    use crate::repository_state::RepositoryEntry;

    let issue = match image.entry(issue_path)? {
        RepositoryEntry::File { bytes, .. } => serde_json::from_slice::<Issue>(bytes)
            .with_context(|| format!("failed to parse captured issue {expected_id}"))?,
        RepositoryEntry::Absent => {
            return Err(crate::storage::IssueNotFoundError::new(expected_id).into())
        }
        _ => return Err(anyhow!("captured issue path is not an ordinary file")),
    };
    if issue.id != expected_id {
        return Err(anyhow!(
            "captured issue identity mismatch: requested {expected_id}, found {}",
            issue.id
        ));
    }
    Ok(issue)
}

/// Discover one document exclusively from the supplied image.
/// The returned paths are the complete local closure observed in that image.
fn scan_document_from_image(
    layout: &crate::repository_state::RepositoryLayout,
    image: &crate::repository_state::RepositoryImage,
    path: &str,
    document_path: &crate::repository_state::VirtualPath,
) -> Result<CapturedDocumentScan> {
    let Some(bytes) = image.file_bytes(document_path)? else {
        return Ok(CapturedDocumentScan {
            warning: Some(DocumentScanWarning::Missing(format!(
                "Could not read document at {path} - skipping asset scan"
            ))),
            ..CapturedDocumentScan::empty()
        });
    };
    scan_document_bytes(layout, path, bytes)
}

fn scan_document_source(
    layout: &crate::repository_state::RepositoryLayout,
    image: &crate::repository_state::RepositoryImage,
    path: &str,
    source: &DocumentScanSource,
) -> Result<CapturedDocumentScan> {
    match source {
        DocumentScanSource::Worktree(document_path) => {
            scan_document_from_image(layout, image, path, document_path)
        }
        DocumentScanSource::Pinned { revision, path } => {
            let evidence = image
                .pinned_evidence()
                .get(&(revision.clone(), path.clone()))
                .ok_or_else(|| anyhow!("pinned evidence for '{path}' at '{revision}' is absent"))?;
            let Some(bytes) = evidence.bytes() else {
                return Ok(CapturedDocumentScan {
                    warning: Some(DocumentScanWarning::Missing(format!(
                        "Could not read document at {path} from {revision} - skipping asset scan"
                    ))),
                    ..CapturedDocumentScan::empty()
                });
            };
            scan_document_bytes(layout, path, bytes)
        }
    }
}

fn scan_document_bytes(
    layout: &crate::repository_state::RepositoryLayout,
    path: &str,
    bytes: &[u8],
) -> Result<CapturedDocumentScan> {
    use crate::document::{AdapterRegistry, AssetScanner};
    use crate::repository_state::VirtualPath;
    use std::collections::BTreeSet;
    use std::path::Path;

    let content = std::str::from_utf8(bytes)
        .map_err(|error| anyhow!("document '{path}' is not UTF-8: {error}"))?;
    let registry = AdapterRegistry::with_builtins();
    let format = registry
        .resolve(path, content)
        .map(|adapter| adapter.id().to_string());
    let Some(_) = format else {
        return Ok(CapturedDocumentScan::empty());
    };
    let scanner = AssetScanner::new(registry, layout.worktree_root());
    let discovered = match scanner.discover_assets(Path::new(path), content) {
        Ok(discovered) => discovered,
        Err(error) => {
            return Ok(CapturedDocumentScan {
                format,
                warning: Some(DocumentScanWarning::Failed(format!(
                    "Failed to scan assets: {error}"
                ))),
                ..CapturedDocumentScan::empty()
            })
        }
    };
    let required_paths = discovered
        .iter()
        .filter_map(|asset| asset.resolved_path.as_ref())
        .map(VirtualPath::worktree)
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(CapturedDocumentScan {
        format,
        assets: discovered,
        required_paths,
        warning: None,
    })
}

fn hydrate_document_scan(
    layout: &crate::repository_state::RepositoryLayout,
    image: &crate::repository_state::RepositoryImage,
    source: &DocumentScanSource,
    mut scan: CapturedDocumentScan,
) -> Result<CapturedDocumentScan> {
    if scan.warning.is_none() {
        match source {
            DocumentScanSource::Worktree(_) => {
                let scanner = crate::document::AssetScanner::new(
                    crate::document::AdapterRegistry::with_builtins(),
                    layout.worktree_root(),
                );
                scan.assets = scanner
                    .hydrate_assets_from_image(scan.assets, image)
                    .map_err(|error| anyhow!("failed to scan assets: {error}"))?;
            }
            DocumentScanSource::Pinned { revision, .. } => {
                use crate::document::AssetType;
                use sha2::{Digest, Sha256};

                for asset in &mut scan.assets {
                    let Some(path) = asset.resolved_path.as_ref() else {
                        continue;
                    };
                    let key = (revision.clone(), path.to_string_lossy().into_owned());
                    let evidence = image.pinned_evidence().get(&key).ok_or_else(|| {
                        anyhow!("pinned evidence for '{}' at '{revision}' is absent", key.1)
                    })?;
                    if let Some(bytes) = evidence.bytes() {
                        asset.asset_type = AssetType::Local;
                        asset.mime_type = crate::document::AssetScanner::detect_mime_type(path);
                        asset.content_hash = Some(format!("{:x}", Sha256::digest(bytes)));
                    }
                }
            }
        }
    }
    Ok(scan)
}

#[allow(clippy::too_many_arguments)]
fn derive_document_add(
    mut issue: Issue,
    path: &str,
    commit: Option<&str>,
    label: Option<&str>,
    doc_type: Option<&str>,
    format: Option<String>,
    assets: Vec<crate::document::Asset>,
) -> DerivedDocumentMutation<DocumentAddResult> {
    use crate::domain::DocumentReference;
    use crate::repository_state::MutationIntent;

    let existing_index = issue
        .documents
        .iter()
        .position(|document| document.path == path);
    let updated = existing_index.is_some();
    let document = match existing_index {
        Some(index) => {
            let existing = &issue.documents[index];
            DocumentReference {
                path: path.to_string(),
                commit: commit.map(String::from),
                label: label.map(String::from).or_else(|| existing.label.clone()),
                doc_type: doc_type
                    .map(String::from)
                    .or_else(|| existing.doc_type.clone()),
                format,
                assets,
            }
        }
        None => DocumentReference {
            path: path.to_string(),
            commit: commit.map(String::from),
            label: label.map(String::from),
            doc_type: doc_type.map(String::from),
            format,
            assets,
        },
    };
    let outcome = DocumentAddResult {
        issue_id: issue.id.clone(),
        document: document.clone(),
        updated,
    };
    if existing_index.is_some_and(|index| issue.documents[index] == document) {
        return DerivedDocumentMutation {
            outcome,
            intents: Vec::new(),
        };
    }
    match existing_index {
        Some(index) => issue.documents[index] = document,
        None => issue.documents.push(document),
    }
    let event = crate::domain::Event::draft_issue_updated(
        issue.id.clone(),
        if updated { "doc-update" } else { "doc-add" }.to_string(),
        vec!["documents".to_string()],
    );
    DerivedDocumentMutation {
        outcome,
        intents: vec![
            MutationIntent::UpdateIssue {
                issue: Box::new(issue),
            },
            MutationIntent::RecordEvent {
                phase: 1,
                event: Box::new(event),
            },
        ],
    }
}

fn derive_document_remove(
    mut issue: Issue,
    path: &str,
) -> Result<DerivedDocumentMutation<DocumentRemoveResult>> {
    use crate::repository_state::MutationIntent;

    if !issue.documents.iter().any(|document| document.path == path) {
        return Err(crate::errors::NotFoundError::new(format!(
            "Document reference {path} not found in issue {}",
            issue.id
        ))
        .into());
    }
    issue.documents.retain(|document| document.path != path);
    let outcome = DocumentRemoveResult {
        issue_id: issue.id.clone(),
        path: path.to_string(),
    };
    let event = crate::domain::Event::draft_issue_updated(
        issue.id.clone(),
        "doc-remove".to_string(),
        vec!["documents".to_string()],
    );
    Ok(DerivedDocumentMutation {
        outcome,
        intents: vec![
            MutationIntent::UpdateIssue {
                issue: Box::new(issue),
            },
            MutationIntent::RecordEvent {
                phase: 1,
                event: Box::new(event),
            },
        ],
    })
}

/// Check if an asset exists in git repository
fn check_asset_in_git(repo: &Option<git2::Repository>, path: &std::path::Path) -> bool {
    if let Some(repo) = repo {
        // Try to find the file in HEAD
        if let Ok(head) = repo.head() {
            if let Some(target) = head.target() {
                if let Ok(commit) = repo.find_commit(target) {
                    if let Ok(tree) = commit.tree() {
                        let path_str = path.to_str().unwrap_or("");
                        return tree.get_path(std::path::Path::new(path_str)).is_ok();
                    }
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::DocumentReference;
    use crate::repository_state::{
        finalize, CaptureSpec, MutationContext, RepositoryAction, VirtualPath,
    };
    use crate::storage::{
        InMemoryStorage, IssueStore, RepositoryStateStore, RepositoryStateStoreError,
    };
    use std::collections::BTreeSet;

    fn capture_spec(paths: impl IntoIterator<Item = VirtualPath>) -> CaptureSpec {
        document_capture_spec(paths.into_iter().collect::<BTreeSet<_>>()).unwrap()
    }

    fn event_bytes(plan: &crate::repository_state::MaterializationPlan) -> Vec<u8> {
        plan.delta()
            .actions()
            .iter()
            .find_map(|action| match action {
                RepositoryAction::WriteFile { path, bytes, .. }
                    if path == &VirtualPath::data("events.jsonl").unwrap() =>
                {
                    Some(bytes.clone())
                }
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn test_document_rescan_uses_image_and_identical_inventory_is_noop() {
        let storage = InMemoryStorage::new();
        let mut issue = crate::domain::types::fixture_issue("Docs".into(), String::new());
        issue.documents.push(DocumentReference {
            path: "docs/guide.md".into(),
            commit: None,
            label: None,
            doc_type: None,
            format: None,
            assets: Vec::new(),
        });
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);
        storage.add_worktree_file("docs/guide.md", "![logo](./logo.png)\n");
        storage.add_worktree_file("docs/logo.png", "png bytes");
        let layout = storage.repository_layout();
        let executor = CommandExecutor::new(storage.clone()).with_layout(layout);

        let first = executor
            .list_document_assets(&id, "docs/guide.md", true)
            .unwrap();
        assert_eq!(first.assets.len(), 1);
        assert_eq!(
            first.assets[0].asset_type,
            crate::document::AssetType::Local
        );
        let event_count = storage.read_events().unwrap().len();
        let updated_at = storage.load_issue(&id).unwrap().updated_at;

        let second = executor
            .list_document_assets(&id, "docs/guide.md", true)
            .unwrap();
        assert_eq!(second.assets, first.assets);
        assert_eq!(storage.read_events().unwrap().len(), event_count);
        assert_eq!(storage.load_issue(&id).unwrap().updated_at, updated_at);
    }

    #[test]
    fn test_document_add_retry_rederives_assets_and_preserves_unrelated_issue_changes() {
        let storage = InMemoryStorage::new();
        let mut issue = crate::domain::types::fixture_issue("Docs".into(), String::new());
        issue.documents.push(DocumentReference {
            path: "docs/other.md".into(),
            commit: None,
            label: None,
            doc_type: None,
            format: None,
            assets: Vec::new(),
        });
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);
        storage.add_worktree_file("docs/guide.md", "![old](./old.png)\n");
        storage.add_worktree_file("docs/old.png", "old bytes");
        storage.add_worktree_file("docs/new.png", "new bytes");

        let layout = storage.repository_layout();
        let issue_path = VirtualPath::data(format!("issues/{id}.json")).unwrap();
        let events_path = VirtualPath::data("events.jsonl").unwrap();
        let document_path = VirtualPath::worktree("docs/guide.md").unwrap();
        let old_asset_path = VirtualPath::worktree("docs/old.png").unwrap();
        let expected_time = chrono::DateTime::parse_from_rfc3339("2026-07-20T13:14:15Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let context = MutationContext::deterministic([91; 32], expected_time);

        let mut first_session = storage.open_mutation_session(layout.clone()).unwrap();
        let first_image = first_session
            .capture(capture_spec([
                issue_path.clone(),
                events_path.clone(),
                document_path.clone(),
                old_asset_path,
            ]))
            .unwrap();
        let first_scan =
            scan_document_from_image(&layout, &first_image, "docs/guide.md", &document_path)
                .unwrap();
        assert!(first_scan
            .required_paths
            .contains(&VirtualPath::worktree("docs/old.png").unwrap()));
        let source = DocumentScanSource::Worktree(document_path.clone());
        let first_scan = hydrate_document_scan(&layout, &first_image, &source, first_scan).unwrap();
        let first = derive_document_add(
            captured_issue(&first_image, &issue_path, &id).unwrap(),
            "docs/guide.md",
            None,
            None,
            None,
            first_scan.format,
            first_scan.assets,
        );
        let first_plan = finalize(&layout, &first_image, &context, &first.intents).unwrap();

        let mut concurrent = storage.load_issue(&id).unwrap();
        concurrent.labels.push("owner:concurrent".into());
        crate::commands::test_helpers::seed_issue(&storage, concurrent);
        storage.add_worktree_file("docs/guide.md", "![new](./new.png)\n");
        assert!(matches!(
            first_session.apply(&first_plan),
            Err(RepositoryStateStoreError::RetryableConflict { .. })
        ));
        drop(first_session);

        let new_asset_path = VirtualPath::worktree("docs/new.png").unwrap();
        let mut retry_session = storage.open_mutation_session(layout.clone()).unwrap();
        let retry_image = retry_session
            .capture(capture_spec([
                issue_path.clone(),
                events_path,
                document_path.clone(),
                new_asset_path,
            ]))
            .unwrap();
        let retry_scan =
            scan_document_from_image(&layout, &retry_image, "docs/guide.md", &document_path)
                .unwrap();
        assert!(retry_scan
            .required_paths
            .contains(&VirtualPath::worktree("docs/new.png").unwrap()));
        let retry_scan = hydrate_document_scan(&layout, &retry_image, &source, retry_scan).unwrap();
        let retry = derive_document_add(
            captured_issue(&retry_image, &issue_path, &id).unwrap(),
            "docs/guide.md",
            None,
            None,
            None,
            retry_scan.format,
            retry_scan.assets,
        );
        let retry_plan = finalize(&layout, &retry_image, &context, &retry.intents).unwrap();
        assert_eq!(event_bytes(&first_plan), event_bytes(&retry_plan));
        retry_session.apply(&retry_plan).unwrap();

        let updated = storage.load_issue(&id).unwrap();
        assert!(updated
            .labels
            .iter()
            .any(|label| label == "owner:concurrent"));
        assert!(updated
            .documents
            .iter()
            .any(|document| document.path == "docs/other.md"));
        let guide = updated
            .documents
            .iter()
            .find(|document| document.path == "docs/guide.md")
            .unwrap();
        assert_eq!(guide.assets.len(), 1);
        assert_eq!(guide.assets[0].original_path, "./new.png");
        assert_eq!(
            guide.assets[0].asset_type,
            crate::document::AssetType::Local
        );
        assert_eq!(updated.updated_at, expected_time);
    }

    #[test]
    fn test_document_remove_from_concurrent_image_preserves_other_fields() {
        let mut issue = crate::domain::types::fixture_issue("Docs".into(), String::new());
        issue.labels.push("owner:concurrent".into());
        issue.documents.extend(
            ["docs/remove.md", "docs/keep.md"].map(|path| DocumentReference {
                path: path.into(),
                commit: None,
                label: None,
                doc_type: None,
                format: None,
                assets: Vec::new(),
            }),
        );

        let derived = derive_document_remove(issue, "docs/remove.md").unwrap();
        let updated = derived
            .intents
            .iter()
            .find_map(|intent| match intent {
                crate::repository_state::MutationIntent::UpdateIssue { issue } => Some(issue),
                _ => None,
            })
            .unwrap();
        assert!(updated
            .labels
            .iter()
            .any(|label| label == "owner:concurrent"));
        assert_eq!(updated.documents.len(), 1);
        assert_eq!(updated.documents[0].path, "docs/keep.md");
    }

    #[test]
    fn test_document_add_preserves_typed_invalid_path_error() {
        let storage = InMemoryStorage::new();
        let issue = crate::domain::types::fixture_issue("Docs".into(), String::new());
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);
        let executor =
            CommandExecutor::new(storage.clone()).with_layout(storage.repository_layout());

        let error = executor
            .add_document_reference(&id, "../outside.md", None, None, None, false)
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<crate::storage::PathReadError>(),
            Some(crate::storage::PathReadError::InvalidPath(_))
        ));
        assert!(storage.load_issue(&id).unwrap().documents.is_empty());
        assert!(storage.read_events().unwrap().is_empty());
    }
}
