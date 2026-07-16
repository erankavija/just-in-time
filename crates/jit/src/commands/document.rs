//! Document reference operations

use super::*;

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
    ) -> Result<(DocumentAddResult, Vec<String>)> {
        use crate::document::{AdapterRegistry, AssetScanner};
        use crate::domain::DocumentReference;
        use std::path::Path;

        let mut warnings = Vec::new();

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id)?;

        // Get repository root (parent of .jit directory)
        let repo_root = self
            .storage
            .root()
            .parent()
            .ok_or_else(|| crate::errors::InvalidArgumentError::new("Invalid storage path"))?;

        // Detect format and scan assets unless --skip-scan
        let (format, assets) = if skip_scan {
            (None, Vec::new())
        } else if let Ok((content, _)) = self.storage.read_path_text(path, None) {
            // Detect format using adapter registry
            let registry = AdapterRegistry::with_builtins();
            let format = registry
                .resolve(path, &content)
                .map(|adapter| adapter.id().to_string());

            // Scan for assets
            let assets = if format.is_some() {
                let scanner = AssetScanner::new(registry, repo_root);
                scanner
                    .scan_document(Path::new(path), &content)
                    .unwrap_or_else(|e| {
                        warnings.push(format!("Failed to scan assets: {}", e));
                        Vec::new()
                    })
            } else {
                Vec::new()
            };

            (format, assets)
        } else {
            // File doesn't exist or can't be read - skip scanning but don't fail
            warnings.push(format!(
                "Could not read document at {} - skipping asset scan",
                path
            ));
            (None, Vec::new())
        };

        // Identity is (issue, path): re-adding a path already linked to this
        // issue refreshes that entry in place instead of appending a
        // duplicate. `commit`/`label`/`doc_type` follow the same
        // partial-update convention as `issue update` — an omitted flag
        // (`None`) leaves the existing value alone rather than clearing it —
        // while `format`/`assets` are always the freshly computed scan
        // result (or empty, under `--skip-scan`), mirroring a fresh add.
        let existing_index = issue.documents.iter().position(|d| d.path == path);
        let is_update = existing_index.is_some();

        let doc_ref = match existing_index {
            Some(idx) => {
                let existing = &issue.documents[idx];
                DocumentReference {
                    path: path.to_string(),
                    commit: commit.map(String::from).or_else(|| existing.commit.clone()),
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

        match existing_index {
            Some(idx) => issue.documents[idx] = doc_ref.clone(),
            None => issue.documents.push(doc_ref.clone()),
        }
        self.storage.save_issue(issue)?;

        // Log the mutation (after the save), mirroring `issue.rs`/`bulk_update.rs`
        // so every `documents` change is captured in the event log (@/inv/event-log).
        // A re-add of an already-linked path is tagged `doc-update` rather than
        // `doc-add`, so the log distinguishes a genuine new link from a refresh
        // of an existing one.
        let event = crate::domain::Event::new_issue_updated(
            full_id.clone(),
            if is_update { "doc-update" } else { "doc-add" }.to_string(),
            vec!["documents".to_string()],
        );
        self.storage.append_event(&event)?;

        Ok((
            DocumentAddResult {
                issue_id: full_id,
                document: doc_ref,
                updated: is_update,
            },
            warnings,
        ))
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

    pub fn remove_document_reference(
        &self,
        issue_id: &str,
        path: &str,
    ) -> Result<DocumentRemoveResult> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id)?;

        let original_len = issue.documents.len();
        issue.documents.retain(|doc| doc.path != path);

        if issue.documents.len() == original_len {
            // Generic NotFoundError: a document-reference not-found has no dedicated
            // domain type (unlike issue/gate/preset/gate-run/repository/lease). Still
            // downcastable -> exit 3; message preserved verbatim.
            return Err(crate::errors::NotFoundError::new(format!(
                "Document reference {} not found in issue {}",
                path, full_id
            ))
            .into());
        }

        self.storage.save_issue(issue)?;

        // Log the mutation (after the save), mirroring `issue.rs`/`bulk_update.rs`
        // so every `documents` change is captured in the event log (@/inv/event-log).
        let event = crate::domain::Event::new_issue_updated(
            full_id.clone(),
            "doc-remove".to_string(),
            vec!["documents".to_string()],
        );
        self.storage.append_event(&event)?;

        Ok(DocumentRemoveResult {
            issue_id: full_id,
            path: path.to_string(),
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
            let repo = Repository::open(".")
                .context("Git repository required when viewing specific commit version")?;
            self.read_file_from_git(&repo, &doc.path, reference)
                .with_context(|| format!("Failed to read {} from git at {}", doc.path, reference))?
        } else {
            // No specific version - try git, fall back to filesystem
            match Repository::open(".") {
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

        let repo = Repository::open(".").map_err(|e| anyhow!("Not a git repository: {}", e))?;

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

        let repo = Repository::open(".").map_err(|e| anyhow!("Not a git repository: {}", e))?;

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

        // Get repository root (parent of .jit directory)
        let repo_root = self.storage.root().parent().ok_or_else(|| {
            PathReadError::Other(
                crate::errors::InvalidArgumentError::new("Invalid storage path").into(),
            )
        })?;

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

        // Get repository root (parent of .jit directory)
        let repo_root = self.storage.root().parent().ok_or_else(|| {
            PathReadError::Other(
                crate::errors::InvalidArgumentError::new("Invalid storage path").into(),
            )
        })?;

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
    ) -> Result<crate::commands::AssetListResult> {
        use crate::document::{AdapterRegistry, AssetScanner, AssetType};
        use anyhow::anyhow;
        use std::path::Path;

        let mut warnings = Vec::new();

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id)?;

        // Find the document in the issue
        let doc_index = issue
            .documents
            .iter()
            .position(|d| d.path == path)
            .ok_or_else(|| anyhow!("Document '{}' not linked to issue {}", path, issue_id))?;

        // Get repository root
        let repo_root = self
            .storage
            .root()
            .parent()
            .ok_or_else(|| crate::errors::InvalidArgumentError::new("Invalid storage path"))?;

        // Rescan if requested
        let assets = if rescan {
            if let Ok((content, _)) = self.storage.read_path_text(path, None) {
                let registry = AdapterRegistry::with_builtins();
                let scanner = AssetScanner::new(registry, repo_root);
                let scanned_assets = scanner
                    .scan_document(Path::new(path), &content)
                    .unwrap_or_else(|e| {
                        warnings.push(format!("Failed to scan assets: {}", e));
                        Vec::new()
                    });

                // Update the document with rescanned assets
                issue.documents[doc_index].assets = scanned_assets.clone();
                self.storage.save_issue(issue)?;

                scanned_assets
            } else {
                warnings.push(format!("Could not read document at {}", path));
                issue.documents[doc_index].assets.clone()
            }
        } else {
            issue.documents[doc_index].assets.clone()
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

        // Get repository root
        let repo_root = self
            .storage
            .root()
            .parent()
            .ok_or_else(|| crate::errors::InvalidArgumentError::new("Invalid storage path"))?;

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
