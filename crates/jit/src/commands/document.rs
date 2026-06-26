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

        let doc_ref = DocumentReference {
            path: path.to_string(),
            commit: commit.map(String::from),
            label: label.map(String::from),
            doc_type: doc_type.map(String::from),
            format,
            assets,
        };

        issue.documents.push(doc_ref.clone());
        self.storage.save_issue(issue)?;

        Ok((
            DocumentAddResult {
                issue_id: full_id,
                document: doc_ref,
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
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let storage = JsonFileStorage::new(".jit");
    /// let executor = CommandExecutor::new(storage);
    ///
    /// // Read working-tree bytes for a document linked to issue "abc123":
    /// let (bytes, label) = executor
    ///     .read_document_bytes("abc123", "docs/spec.md", None)
    ///     .unwrap();
    /// assert_eq!(label, "working-tree");
    ///
    /// // Read bytes at a specific git commit:
    /// // let (bytes, hash) = executor
    /// //     .read_document_bytes("abc123", "docs/spec.md", Some("HEAD"))
    /// //     .unwrap();
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::InMemoryStorage;
    ///
    /// let storage = InMemoryStorage::new();
    /// let executor = CommandExecutor::new(storage);
    ///
    /// // Read a file from the working tree:
    /// let (bytes, commit) = executor.read_path_bytes("/path/to/file.md", None).unwrap();
    /// assert_eq!(commit, "working-tree");
    ///
    /// // Read a file at a specific git commit:
    /// // let (bytes, short_hash) = executor.read_path_bytes("README.md", Some("HEAD")).unwrap();
    /// ```
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
    pub fn check_document_links(&self, scope: &str) -> Result<crate::commands::LinkCheckResult> {
        use crate::document::{AssetType, LinkValidationResult, LinkValidator};
        use std::path::PathBuf;

        // Get repository root
        let repo_root = self
            .storage
            .root()
            .parent()
            .ok_or_else(|| crate::errors::InvalidArgumentError::new("Invalid storage path"))?;

        // Parse scope and get documents to check
        let issues = if scope == "all" {
            self.storage.list_issues()?
        } else if let Some(issue_id) = scope.strip_prefix("issue:") {
            let full_id = self.storage.resolve_issue_id(issue_id)?;
            let issue = self.storage.load_issue(&full_id)?;
            vec![issue]
        } else {
            return Err(crate::errors::InvalidArgumentError::new(format!(
                "Invalid scope '{}'. Use 'all' or 'issue:ID'",
                scope
            ))
            .into());
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

impl<S: IssueStore> CommandExecutor<S> {
    /// Archive a document and its per-doc assets to a configured archive directory.
    ///
    /// Archival is **reference-aware**: it relocates the document (and its
    /// per-doc assets) to the archive root AND re-links every issue
    /// doc-reference that points at the old path to the new archive path (the
    /// relink is [`update_issue_metadata`](Self::update_issue_metadata)).
    ///
    /// The guarantee is **referential consistency**, not all-or-nothing
    /// transactionality. True atomicity across the document file, the N issue
    /// JSON files, and the append-only event log is not achievable, so this is
    /// *not* a transaction that either fully commits or fully aborts. Instead the
    /// invariant is: **no single failure leaves a dangling reference, and no file
    /// is removed while any reference could still point at it.** Under any single
    /// failure every reference still resolves to a file that exists, though a
    /// failed rollback may leave a reported duplicate (both source and archive
    /// copy) for manual cleanup.
    ///
    /// The move is committed in stages so the invariant holds at every failure
    /// point:
    ///
    /// 1. The document and per-doc assets are **copied** to the destination; the
    ///    sources are left in place, so references still resolve to the source.
    /// 2. References are re-linked, then the archive event is appended. Both
    ///    source and destination exist throughout, so no reference can dangle. On
    ///    failure, [`rollback_archive`](Self::rollback_archive) restores the
    ///    references to the source and removes the destination copies **only
    ///    after confirming no reference still points at them**; if that cannot be
    ///    confirmed it keeps both files (a reported duplicate) and errors loudly.
    /// 3. Only after both the relink and the event persist are the old sources
    ///    removed, committing the move. A removal failure here is non-fatal:
    ///    references already resolve to the destination, so a leftover source is
    ///    reported as a warning rather than an error.
    ///
    /// Two conflicts make the archive a **no-op**, both checked up front before
    /// any filesystem or `.jit` mutation so neither moves a file, touches an
    /// asset, or rewrites a reference: a missing source surfaces as
    /// [`ArchiveError::SourceMissing`], and an already-occupied destination
    /// surfaces as [`ArchiveError::DestinationOccupied`] (archiving onto it would
    /// overwrite a file another reference may resolve to). Both are typed errors
    /// that never leave or create a dangling reference.
    ///
    /// Returns (result, warnings) tuple where result contains archival details and
    /// warnings contains any non-fatal issues encountered.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::{ArchiveError, CommandExecutor};
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// // Archiving a missing source is a typed no-op error.
    /// let err = executor
    ///     .archive_document("dev/active/missing.md", "design", false, false)
    ///     .unwrap_err();
    /// assert!(err.downcast_ref::<ArchiveError>().is_some());
    /// ```
    pub fn archive_document(
        &self,
        path: &str,
        category: &str,
        dry_run: bool,
        force: bool,
    ) -> Result<(ArchiveResult, Vec<String>)> {
        use crate::config::JitConfig;
        use anyhow::anyhow;
        use std::path::Path;

        // 1. Load configuration
        let config = JitConfig::load(self.storage.root())?;
        let doc_config = config
            .documentation
            .ok_or_else(|| anyhow!("No [documentation] configuration found in .jit/config.toml"))?;

        // 2. Validate category exists
        let categories = doc_config.categories();
        let archive_subdir = categories.get(category).ok_or_else(|| {
            anyhow!(
                "Category '{}' not configured in [documentation.categories]",
                category
            )
        })?;

        // 3. Get repository root
        let repo_root = self
            .storage
            .root()
            .parent()
            .ok_or_else(|| crate::errors::InvalidArgumentError::new("Invalid storage path"))?;

        let doc_path = Path::new(path);

        // 4. Validate document is archivable
        self.validate_archivable(doc_path, &doc_config, repo_root)?;

        // 5. Compute destination path
        let dest_path = self.compute_destination(doc_path, archive_subdir, &doc_config)?;

        // 5a. Scan document for assets
        let per_doc_assets = self.scan_and_classify_assets(path, repo_root)?;

        // 5b. Validate link integrity - check for risky relative links to shared assets
        self.validate_archive_link_integrity(
            path,
            &per_doc_assets,
            doc_path,
            &dest_path,
            repo_root,
        )?;

        if dry_run {
            // Return dry-run result without printing
            let result = ArchiveResult {
                source_path: path.to_string(),
                dest_path: dest_path.to_str().unwrap().to_string(),
                category: category.to_string(),
                assets_moved: per_doc_assets.len(),
                updated_issues: Vec::new(), // No updates in dry-run
                dry_run: true,
            };
            return Ok((result, Vec::new()));
        }

        // 6. Check for active issue links (unless --force)
        if !force {
            let active_issues = self.check_active_issue_links(path)?;
            if !active_issues.is_empty() {
                return Err(anyhow!(
                    "⚠️  Warning: document linked to {} active issue(s):\n{}\n\nUse --force to archive anyway.",
                    active_issues.len(),
                    active_issues.join("\n")
                ));
            }
        }

        // 7. Copy the document and per-doc assets to the destination, leaving
        //    the sources in place. References still resolve to the source, so a
        //    failure here is consistent (the copy helper rolls back any partial
        //    destination itself).
        let moved = self.copy_to_archive(doc_path, &dest_path, &per_doc_assets, repo_root)?;
        let assets_moved = per_doc_assets.len();
        let dest_str = dest_path.to_str().unwrap();

        // 8. Verify links resolve at the new location before committing. A
        //    failure rolls back the destination copies, leaving the source and
        //    every reference untouched.
        if let Err(e) = self.verify_post_archival_links(&dest_path, &per_doc_assets, repo_root) {
            self.remove_archive_copies(&moved);
            return Err(e);
        }

        // 9. Commit the move: re-link references, then append the archive event.
        //    Both source and destination exist throughout, so any failure here
        //    is rolled back to a state where every reference resolves to an
        //    existing file — the destination copies are removed only once the
        //    rollback confirms no reference still points at them.
        let updated_issues = match self.update_issue_metadata(path, dest_str) {
            Err(e) => return Err(self.rollback_archive(e, path, dest_str, &moved)),
            Ok(updated) => {
                let event = crate::domain::Event::new_document_archived(
                    path.to_string(),
                    dest_str.to_string(),
                    category.to_string(),
                    updated.len(),
                );
                match self.storage.append_event(&event) {
                    Err(e) => return Err(self.rollback_archive(e, path, dest_str, &moved)),
                    Ok(()) => updated,
                }
            }
        };

        // 10. The relink and event have persisted; remove the old sources to
        //     complete the move (the commit point). A removal failure is
        //     non-fatal: references already resolve to the existing destination,
        //     so a leftover source is only a warning.
        let warnings = self.remove_archived_sources(&moved);

        // Return result with warnings (instead of printing)
        let result = ArchiveResult {
            source_path: path.to_string(),
            dest_path: dest_path.to_str().unwrap().to_string(),
            category: category.to_string(),
            assets_moved,
            updated_issues,
            dry_run: false,
        };

        Ok((result, warnings))
    }

    /// Validate that a document can be archived
    fn validate_archivable(
        &self,
        doc_path: &std::path::Path,
        doc_config: &crate::config::DocumentationConfig,
        repo_root: &std::path::Path,
    ) -> Result<()> {
        use anyhow::anyhow;

        let path_str = doc_path.to_str().unwrap_or("");

        // Check if path is in managed paths
        let managed_paths = doc_config.managed_paths();
        let is_managed = managed_paths.iter().any(|p| path_str.starts_with(p));

        if !is_managed {
            return Err(anyhow!(
                "❌ Cannot archive: document not in managed path\n\n  Document: {}\n\n  Only documents in managed paths can be archived: {}",
                path_str,
                managed_paths.join(", ")
            ));
        }

        // Check if path is in permanent paths
        let permanent_paths = doc_config.permanent_paths();
        let is_permanent = permanent_paths.iter().any(|p| path_str.starts_with(p));

        if is_permanent {
            return Err(anyhow!(
                "❌ Cannot archive: document is in permanent path\n\n  Document: {}\n\n  Permanent paths cannot be archived: {}",
                path_str,
                permanent_paths.join(", ")
            ));
        }

        // Check if file exists. A missing source is the documented no-op case:
        // this runs BEFORE any filesystem or `.jit` mutation, so surfacing the
        // typed `ArchiveError::SourceMissing` here guarantees archival makes no
        // changes and never leaves or creates a dangling reference.
        let full_path = repo_root.join(doc_path);
        if !full_path.exists() {
            return Err(ArchiveError::SourceMissing {
                path: path_str.to_string(),
            }
            .into());
        }

        Ok(())
    }

    /// Compute destination path for archived document
    fn compute_destination(
        &self,
        doc_path: &std::path::Path,
        archive_subdir: &str,
        doc_config: &crate::config::DocumentationConfig,
    ) -> Result<std::path::PathBuf> {
        use anyhow::anyhow;
        use std::path::Path;

        let path_str = doc_path.to_str().unwrap_or("");
        let managed_paths = doc_config.managed_paths();

        // Strip managed path prefix
        let relative_path = managed_paths
            .iter()
            .find_map(|prefix| {
                if path_str.starts_with(prefix) {
                    Some(
                        path_str
                            .strip_prefix(prefix)
                            .unwrap()
                            .trim_start_matches('/'),
                    )
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow!("Path not in managed paths"))?;

        // Build destination: archive_root/category/relative_path
        let archive_root = doc_config.archive_root();
        Ok(Path::new(&archive_root)
            .join(archive_subdir)
            .join(relative_path))
    }

    /// Check for active issues linking to this document
    fn check_active_issue_links(&self, path: &str) -> Result<Vec<String>> {
        let issues = self.storage.list_issues()?;

        let active_issues: Vec<String> = issues
            .iter()
            .filter(|issue| {
                !issue.state.is_terminal() && issue.documents.iter().any(|doc| doc.path == path)
            })
            .map(|issue| {
                format!(
                    "  - {}: {} (state: {:?})",
                    issue.short_id(),
                    issue.title,
                    issue.state
                )
            })
            .collect();

        Ok(active_issues)
    }

    /// Re-link every issue doc-reference from `old_path` to `new_path`,
    /// returning the ids of the issues that were changed.
    ///
    /// This is a plain forward relink: a per-issue save failure is propagated
    /// immediately, so a mid-batch failure can leave the references partially
    /// moved. The archive transaction recovers from that with
    /// [`rollback_archive`](Self::rollback_archive), which restores references
    /// and confirms the result before any file is removed; this function does
    /// not attempt its own rollback.
    fn update_issue_metadata(&self, old_path: &str, new_path: &str) -> Result<Vec<String>> {
        use anyhow::Context;

        let mut updated = Vec::new();
        for mut issue in self.storage.list_issues()? {
            let changed = issue
                .documents
                .iter_mut()
                .filter(|doc| doc.path == old_path)
                .fold(false, |_, doc| {
                    doc.path = new_path.to_string();
                    true
                });

            if changed {
                let id = issue.id.clone();
                self.storage
                    .save_issue(issue)
                    .with_context(|| format!("Failed to re-link issue {id} during archive"))?;
                updated.push(id);
            }
        }

        Ok(updated)
    }

    /// Count the issues whose document references still point at `path`.
    ///
    /// Used by [`rollback_archive`](Self::rollback_archive) to confirm that no
    /// reference points at a destination copy before that copy may be removed.
    fn count_references_to(&self, path: &str) -> Result<usize> {
        Ok(self
            .storage
            .list_issues()?
            .iter()
            .filter(|issue| issue.documents.iter().any(|doc| doc.path == path))
            .count())
    }

    /// Roll an in-flight archive back to the pre-archive state after a relink or
    /// event-append failure, preserving the referential-consistency invariant:
    /// *never remove a file while any reference could still point at it.*
    ///
    /// Re-links any reference that points at `dest_str` back to the still-present
    /// source, then removes the destination copies **only after confirming that
    /// no reference points at `dest_str` any more**. If any reference may still
    /// point at the destination (the restore failed, or the confirmation check
    /// itself errored), both the source and the destination copies are kept, so
    /// every reference resolves to an existing file (possibly a duplicate), and a
    /// loud, contextful error is returned. The original `cause` is preserved as
    /// the error source.
    fn rollback_archive(
        &self,
        cause: anyhow::Error,
        src_str: &str,
        dest_str: &str,
        moved: &[(std::path::PathBuf, std::path::PathBuf)],
    ) -> anyhow::Error {
        // Best effort: move any reference still at the destination back to the
        // source. Whether this fully succeeds is decided by the count below, so
        // its own error is folded into the diagnostic rather than trusted.
        let restore = self.update_issue_metadata(dest_str, src_str);

        match self.count_references_to(dest_str) {
            Ok(0) => {
                // Confirmed: nothing points at the destination, so removing the
                // copies cannot dangle a reference.
                self.remove_archive_copies(moved);
                cause.context(format!(
                    "archive aborted and rolled back: every reference restored to {src_str}"
                ))
            }
            Ok(remaining) => cause.context(format!(
                "archive aborted but rollback is incomplete: {remaining} reference(s) still \
                 point at {dest_str}, so the archive copy was kept alongside the source — \
                 every reference still resolves, but a duplicate at {dest_str} needs manual \
                 cleanup (restore attempt: {restore:?})"
            )),
            Err(check_err) => cause.context(format!(
                "archive aborted and the rollback could not be verified ({check_err:#}), so the \
                 archive copy was kept alongside the source — every reference still resolves, \
                 but a possible duplicate at {dest_str} needs manual cleanup"
            )),
        }
    }

    /// Scan document and classify assets as per-doc vs shared
    fn scan_and_classify_assets(
        &self,
        doc_path: &str,
        repo_root: &std::path::Path,
    ) -> Result<Vec<std::path::PathBuf>> {
        use crate::document::{AdapterRegistry, AssetScanner};
        use std::path::Path;

        // Storage owns the repo-root resolution + containment check now; pass
        // the repo-relative `doc_path` through rather than pre-joining with
        // `repo_root` (which would produce an absolute path that the storage
        // layer rejects as `PathReadError::InvalidPath`).
        let (content, _) = self
            .storage
            .read_path_text(doc_path, None)
            .map_err(|e| anyhow::anyhow!("Failed to read document {}: {}", doc_path, e))?;

        // Scan for assets
        let registry = AdapterRegistry::with_builtins();
        let scanner = AssetScanner::new(registry, repo_root);
        let assets = scanner
            .scan_document(Path::new(doc_path), &content)
            .map_err(|e| anyhow::anyhow!("Asset scan failed: {}", e))?;

        // Identify per-doc assets (in assets/ or <doc-name>_assets/ subdirectories)
        let doc_dir = Path::new(doc_path).parent().unwrap_or(Path::new(""));

        // Extract document name without extension for matching <doc-name>_assets/ pattern
        let doc_name = Path::new(doc_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        let per_doc_assets: Vec<std::path::PathBuf> = assets
            .iter()
            .filter_map(|asset| {
                if let Some(ref resolved) = asset.resolved_path {
                    // Check if asset is in document's directory
                    if let Ok(stripped) = resolved.strip_prefix(doc_dir) {
                        let path_str = stripped.to_string_lossy();
                        // Asset is per-doc if it's in:
                        // 1. assets/ subdirectory
                        // 2. <doc-name>_assets/ subdirectory
                        if path_str.starts_with("assets/")
                            || path_str.starts_with("assets\\")
                            || path_str.starts_with(&format!("{}_assets/", doc_name))
                            || path_str.starts_with(&format!("{}_assets\\", doc_name))
                        {
                            return Some(resolved.clone());
                        }
                    }
                }
                None
            })
            .collect();

        Ok(per_doc_assets)
    }

    /// Move per-doc assets to archive destination
    /// Validate that archiving won't break links
    ///
    /// This checks for the critical case: relative links to shared assets (not being moved)
    /// would break after the document is moved to the archive.
    fn validate_archive_link_integrity(
        &self,
        doc_path: &str,
        per_doc_assets: &[std::path::PathBuf],
        source_path: &std::path::Path,
        dest_path: &std::path::Path,
        repo_root: &std::path::Path,
    ) -> Result<()> {
        use crate::document::{AdapterRegistry, AssetScanner};
        use anyhow::anyhow;
        use std::collections::HashSet;
        use std::path::Path;

        // Storage owns the repo-root resolution + containment check; pass
        // the repo-relative `doc_path` directly rather than pre-joining with
        // `repo_root`.
        let (content, _) = self.storage.read_path_text(doc_path, None).map_err(|e| {
            anyhow!(
                "Failed to read document for link validation {}: {}",
                doc_path,
                e
            )
        })?;

        // Scan for all assets
        let registry = AdapterRegistry::with_builtins();
        let scanner = AssetScanner::new(registry, repo_root);
        let all_assets = scanner
            .scan_document(Path::new(doc_path), &content)
            .map_err(|e| anyhow!("Asset scan failed: {}", e))?;

        // Build set of per-doc assets for quick lookup
        let per_doc_set: HashSet<_> = per_doc_assets.iter().collect();

        // Check each asset
        let mut risky_links = Vec::new();
        for asset in &all_assets {
            // Skip external assets
            if matches!(asset.asset_type, crate::document::AssetType::External) {
                continue;
            }

            // Check if this is a shared asset (not in per-doc set)
            let is_shared = if let Some(ref resolved) = asset.resolved_path {
                !per_doc_set.contains(resolved)
            } else {
                false
            };

            if is_shared {
                // Shared asset - check if link is relative (not root-relative)
                if !asset.original_path.starts_with('/')
                    && !asset.original_path.starts_with("http://")
                    && !asset.original_path.starts_with("https://")
                {
                    // This is a relative link to a shared asset
                    // It will break when the document moves to a different directory depth

                    // Compute path depths to determine if link will break
                    let source_depth = source_path.components().count();
                    let dest_depth = dest_path.components().count();

                    // If depth changes, relative links to shared assets will break
                    if source_depth != dest_depth {
                        risky_links.push(format!(
                            "  - {} (relative link to shared asset)",
                            asset.original_path
                        ));
                    }
                }
            }
        }

        if !risky_links.is_empty() {
            let source_depth = source_path.components().count();
            let dest_depth = dest_path.components().count();
            let doc_dir = source_path.parent().and_then(|p| p.to_str()).unwrap_or("");
            let doc_name = source_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("doc");

            return Err(anyhow!(
                "❌ Cannot archive: would break {} relative link(s) to shared assets\n\n\
                Document: {}\n\
                Destination: {}\n\
                Depth change: {} → {} (relative links will break)\n\n\
                Problematic links:\n{}\n\n\
                Solutions:\n\
                1. Move shared assets to per-doc location:\n\
                   mkdir {}/assets\n\
                   mv <shared-asset> {}/assets/\n\
                   Update link: ![...](assets/<filename>)\n\n\
                2. Change to root-relative link (starts with /):\n\
                   ![...](/<path-from-repo-root>)\n\n\
                3. Remove the links before archiving",
                risky_links.len(),
                doc_path,
                dest_path.display(),
                source_depth,
                dest_depth,
                risky_links.join("\n"),
                doc_dir,
                doc_name
            ));
        }

        Ok(())
    }

    /// Copy the document and its per-doc assets to the archive destination,
    /// leaving the sources in place.
    ///
    /// The copy is staged through a unique temp directory and finalized with
    /// atomic renames, so a partial failure leaves no half-written destination:
    /// any destination already created is removed before the error is returned.
    ///
    /// The returned vector pairs, for every copied file, its `(source_full,
    /// dest_full)` absolute paths. The caller commits the move with
    /// [`remove_archived_sources`](Self::remove_archived_sources) (delete the
    /// sources) only after the `.jit` relink and event have persisted, or rolls
    /// it back with [`remove_archive_copies`](Self::remove_archive_copies)
    /// (delete the destinations) if that step fails. Until one of those runs,
    /// both copies exist, so no reference can dangle.
    fn copy_to_archive(
        &self,
        source_doc: &std::path::Path,
        dest_doc: &std::path::Path,
        assets: &[std::path::PathBuf],
        repo_root: &std::path::Path,
    ) -> Result<Vec<(std::path::PathBuf, std::path::PathBuf)>> {
        use anyhow::Context;
        use std::path::Path;

        // Create temp directory in repo root (same filesystem for atomic rename)
        let temp_base = repo_root.join(".jit").join("tmp");
        std::fs::create_dir_all(&temp_base).context("Failed to create temp base directory")?;

        let temp_id = uuid::Uuid::new_v4().to_string();
        let temp_dir = temp_base.join(format!("archive-{}", temp_id));
        std::fs::create_dir(&temp_dir).context("Failed to create temporary directory")?;

        // Ensure cleanup on error
        let cleanup_temp = || {
            let _ = std::fs::remove_dir_all(&temp_dir);
        };

        // Prepare list of all files to move: (source_path, dest_path, temp_path)
        let source_dir = source_doc.parent().unwrap_or(Path::new(""));
        let dest_dir = dest_doc.parent().unwrap_or(Path::new(""));

        let mut files_to_move = Vec::new();

        // Add document
        let doc_temp = temp_dir.join(format!("doc-{}", temp_id));
        files_to_move.push((source_doc.to_path_buf(), dest_doc.to_path_buf(), doc_temp));

        // Add assets with unique temp names
        for (idx, asset_path) in assets.iter().enumerate() {
            let relative_to_source = asset_path
                .strip_prefix(source_dir)
                .context("Asset not under source directory")?;
            let dest_asset = dest_dir.join(relative_to_source);
            let asset_temp = temp_dir.join(format!("asset-{}-{}", idx, temp_id));
            files_to_move.push((asset_path.clone(), dest_asset, asset_temp));
        }

        // Step 0: Refuse to clobber. If any destination is already occupied,
        // moving onto it would overwrite a file that another reference may
        // resolve to (and a later rollback would then delete it). Reject before
        // creating anything, so the archive is a true no-op on conflict.
        if let Some((_, dest_rel, _)) = files_to_move
            .iter()
            .find(|(_, dest_rel, _)| repo_root.join(dest_rel).exists())
        {
            cleanup_temp();
            return Err(crate::commands::ArchiveError::DestinationOccupied {
                path: dest_rel.to_string_lossy().into_owned(),
            }
            .into());
        }

        // Step 1: Copy all files to temp
        for (source_rel, _dest_rel, temp_file) in &files_to_move {
            let source_full = repo_root.join(source_rel);

            if let Err(e) = std::fs::copy(&source_full, temp_file) {
                cleanup_temp();
                return Err(e).with_context(|| {
                    format!(
                        "Failed to copy to temp: {} -> {}",
                        source_full.display(),
                        temp_file.display()
                    )
                });
            }
        }

        // Step 2: Move from temp to final destinations, recording the
        // (source_full, dest_full) pair for each created file so the caller can
        // later commit (remove sources) or roll back (remove destinations).
        let mut created: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();

        let move_result: Result<()> = (|| {
            for (source_rel, dest_rel, temp_file) in &files_to_move {
                let dest_full = repo_root.join(dest_rel);

                // Create parent directory
                if let Some(parent) = dest_full.parent() {
                    std::fs::create_dir_all(parent).with_context(|| {
                        format!(
                            "Failed to create destination directory: {}",
                            parent.display()
                        )
                    })?;
                }

                // Move from temp to destination (atomic rename)
                std::fs::rename(temp_file, &dest_full).with_context(|| {
                    format!(
                        "Failed to move from temp to destination: {} -> {}",
                        temp_file.display(),
                        dest_full.display()
                    )
                })?;

                created.push((repo_root.join(source_rel), dest_full));
            }
            Ok(())
        })();

        // Step 3: Handle errors with rollback
        if let Err(e) = move_result {
            // Rollback: delete any files we created in destination
            for (_source, dest) in &created {
                let _ = std::fs::remove_file(dest);
            }
            cleanup_temp();
            return Err(e);
        }

        // The sources are intentionally left in place: removing them is the
        // caller's commit step, run only after the `.jit` relink and event
        // persist. Until then both copies exist, so no reference can dangle.
        cleanup_temp();

        Ok(created)
    }

    /// Commit an archive move by removing the old source files recorded by
    /// [`copy_to_archive`](Self::copy_to_archive).
    ///
    /// Called only after the `.jit` relink and archive event have persisted, so
    /// every reference already resolves to the destination. A removal failure is
    /// therefore non-fatal: the destination is valid and a leftover source is
    /// returned as a warning rather than an error.
    fn remove_archived_sources(
        &self,
        moved: &[(std::path::PathBuf, std::path::PathBuf)],
    ) -> Vec<String> {
        moved
            .iter()
            .filter_map(|(source_full, _dest_full)| {
                std::fs::remove_file(source_full).err().map(|e| {
                    format!(
                        "Failed to remove source file {}: {}",
                        source_full.display(),
                        e
                    )
                })
            })
            .collect()
    }

    /// Roll back an archive move by removing the destination copies recorded by
    /// [`copy_to_archive`](Self::copy_to_archive), leaving the sources in place.
    ///
    /// Used when the relink/event step fails: deleting the just-created copies
    /// restores the pre-archive state where every reference resolves to the
    /// still-present source, so nothing dangles. Best effort; failures are
    /// ignored because the sources remain valid regardless.
    fn remove_archive_copies(&self, moved: &[(std::path::PathBuf, std::path::PathBuf)]) {
        for (_source_full, dest_full) in moved {
            let _ = std::fs::remove_file(dest_full);
        }
    }

    /// Verify that links still work after archival.
    ///
    /// This function scans the archived document and verifies that all per-doc
    /// asset references are valid at the new location.
    ///
    /// # Arguments
    ///
    /// * `dest_doc` - Path to the archived document (relative to repo root)
    /// * `per_doc_assets` - List of per-doc assets that were moved with document
    /// * `repo_root` - Repository root path
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Document cannot be read at new location
    /// - Asset scanning fails
    /// - Any per-doc asset reference is broken (file doesn't exist)
    fn verify_post_archival_links(
        &self,
        dest_doc: &std::path::Path,
        per_doc_assets: &[std::path::PathBuf],
        repo_root: &std::path::Path,
    ) -> Result<()> {
        use crate::document::{AdapterRegistry, AssetScanner};
        use std::collections::HashSet;

        // Storage resolves repo-relative paths against the repo root (and
        // enforces containment), so pass `dest_doc` directly rather than
        // pre-joining it with `repo_root`.  `repo_root` is still used below
        // for the downstream `AssetScanner` and for asset-existence checks.
        let dest_doc_str = dest_doc.to_string_lossy();
        let (content, _) = self
            .storage
            .read_path_text(&dest_doc_str, None)
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to read archived document for verification {}: {}",
                    dest_doc.display(),
                    e
                )
            })?;

        // Scan for asset references in archived document
        let registry = AdapterRegistry::with_builtins();
        let scanner = AssetScanner::new(registry, repo_root);
        let assets = scanner
            .scan_document(dest_doc, &content)
            .map_err(|e| anyhow::anyhow!("Post-archival asset scan failed: {}", e))?;

        // Build set of expected per-doc assets at new location
        let per_doc_set: HashSet<_> = per_doc_assets.iter().collect();

        // Check each asset reference
        let mut broken_links = Vec::new();
        for asset in &assets {
            if let Some(ref resolved) = asset.resolved_path {
                // If this is a per-doc asset that should have moved
                if per_doc_set.contains(resolved) {
                    // Verify the file exists at the new location
                    let asset_full = repo_root.join(resolved);
                    if !asset_full.exists() {
                        broken_links.push(format!(
                            "  - {} → {} (file not found)",
                            asset.original_path,
                            resolved.display()
                        ));
                    }
                }
            }
        }

        if !broken_links.is_empty() {
            return Err(anyhow::anyhow!(
                "❌ Post-archival verification failed: {} broken link(s)\n\n\
                Document: {}\n\n\
                Broken links:\n{}\n\n\
                This indicates an error in the archival process. The archive has been\n\
                rolled back (the source is unchanged); some asset links would not resolve\n\
                at the new location. Please report this as a bug.",
                broken_links.len(),
                dest_doc.display(),
                broken_links.join("\n")
            ));
        }

        Ok(())
    }
}
