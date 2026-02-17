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
        json: bool,
    ) -> Result<Vec<String>> {
        use crate::document::{AdapterRegistry, AssetScanner};
        use crate::domain::DocumentReference;
        use crate::output::{JsonError, JsonOutput};
        use anyhow::anyhow;
        use std::fs;
        use std::path::Path;

        let mut warnings = Vec::new();

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id).inspect_err(|_| {
            if json {
                let err = JsonError::issue_not_found(issue_id, "doc add");
                println!("{}", err.to_json_string().unwrap());
            }
        })?;

        // Get repository root (parent of .jit directory)
        let repo_root = self
            .storage
            .root()
            .parent()
            .ok_or_else(|| anyhow!("Invalid storage path"))?;

        let doc_path = repo_root.join(path);

        // Detect format and scan assets unless --skip-scan
        let (format, assets) = if skip_scan {
            (None, Vec::new())
        } else if let Ok(content) = fs::read_to_string(&doc_path) {
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

        if json {
            let output = JsonOutput::success(
                serde_json::json!({
                    "issue_id": full_id,
                    "document": doc_ref,
                }),
                "doc add",
            );
            println!("{}", output.to_json_string()?);
        } else {
            println!("Added document reference to issue {}", full_id);
            println!("  Path: {}", path);
            if let Some(c) = commit {
                println!("  Commit: {}", c);
            }
            if let Some(l) = label {
                println!("  Label: {}", l);
            }
            if let Some(t) = doc_type {
                println!("  Type: {}", t);
            }
            if let Some(f) = &doc_ref.format {
                println!("  Format: {}", f);
            }
            if !doc_ref.assets.is_empty() {
                println!("  Assets: {} discovered", doc_ref.assets.len());
            }
        }

        Ok(warnings)
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

    pub fn remove_document_reference(&self, issue_id: &str, path: &str, json: bool) -> Result<()> {
        use crate::output::{JsonError, JsonOutput};

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id).inspect_err(|_| {
            if json {
                let err = JsonError::issue_not_found(issue_id, "doc remove");
                println!("{}", err.to_json_string().unwrap());
            }
        })?;

        let original_len = issue.documents.len();
        issue.documents.retain(|doc| doc.path != path);

        if issue.documents.len() == original_len {
            let err_msg = format!("Document reference {} not found in issue {}", path, full_id);
            if json {
                let err = JsonError::new("DOCUMENT_NOT_FOUND", &err_msg, "doc remove")
                    .with_suggestion("Run 'jit doc list <issue-id>' to see available documents");
                println!("{}", err.to_json_string()?);
            }
            return Err(anyhow!(err_msg));
        }

        self.storage.save_issue(issue)?;

        if json {
            let output = JsonOutput::success(
                serde_json::json!({
                    "issue_id": full_id,
                    "removed_path": path,
                }),
                "doc remove",
            );
            println!("{}", output.to_json_string()?);
        } else {
            println!("Removed document reference {} from issue {}", path, full_id);
        }

        Ok(())
    }

    pub fn show_document_content(
        &self,
        issue_id: &str,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<()> {
        use git2::Repository;

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;

        let doc = issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| anyhow!("Document reference {} not found in issue {}", path, full_id))?;

        // Determine which commit to view
        let reference = if let Some(at) = at_commit {
            at
        } else if let Some(ref commit) = doc.commit {
            commit.as_str()
        } else {
            "HEAD"
        };

        // Display metadata
        println!("Document: {}", doc.path);
        if let Some(ref label) = doc.label {
            println!("Label: {}", label);
        }
        println!("Commit: {}", reference);
        if let Some(ref doc_type) = doc.doc_type {
            println!("Type: {}", doc_type);
        }
        println!("\n---\n");

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
                    // Git not available - read from filesystem
                    std::fs::read_to_string(&doc.path)
                        .with_context(|| format!("Failed to read {} from filesystem", doc.path))?
                }
            }
        };

        println!("{}", content);

        Ok(())
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
            .ok_or_else(|| anyhow!("Document reference {} not found in issue {}", path, full_id))?;

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
    ) -> Result<()> {
        use git2::Repository;

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;

        // Verify document reference exists
        issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| anyhow!("Document reference {} not found in issue {}", path, full_id))?;

        let repo = Repository::open(".").map_err(|e| anyhow!("Not a git repository: {}", e))?;

        let to_ref = to.unwrap_or("HEAD");

        // Get content at both commits
        let from_content = self.read_file_from_git(&repo, path, from)?;
        let to_content = self.read_file_from_git(&repo, path, to_ref)?;

        // Generate unified diff
        println!("diff --git a/{} b/{}", path, path);
        println!("--- a/{} ({})", path, from);
        println!("+++ b/{} ({})", path, to_ref);
        println!();

        // Use similar crate for diff generation
        use similar::{ChangeTag, TextDiff};
        let diff = TextDiff::from_lines(&from_content, &to_content);

        for change in diff.iter_all_changes() {
            let sign = match change.tag() {
                ChangeTag::Delete => "-",
                ChangeTag::Insert => "+",
                ChangeTag::Equal => " ",
            };
            print!("{}{}", sign, change);
        }

        Ok(())
    }

    /// Read document content from git or filesystem.
    ///
    /// Note: This method is part of the public API used by jit-server.
    /// It's not called from the CLI binary, hence the dead_code warning.
    #[allow(dead_code)]
    pub fn read_document_content(
        &self,
        issue_id: &str,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<(String, String)> {
        use git2::Repository;

        let issue = self.storage.load_issue(issue_id)?;

        let doc = issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                anyhow!(
                    "Document reference {} not found in issue {}",
                    path,
                    issue_id
                )
            })?;

        // Get repository root (parent of .jit directory)
        let repo_root = self
            .storage
            .root()
            .parent()
            .ok_or_else(|| anyhow!("Invalid storage path"))?;

        // Try git first if available
        if let Ok(repo) = Repository::open(repo_root) {
            // Determine which commit to view
            let reference = if let Some(at) = at_commit {
                at
            } else if let Some(ref commit) = doc.commit {
                commit.as_str()
            } else {
                "HEAD"
            };

            // Try to read content from git
            if let Ok(content) = self.read_file_from_git(&repo, &doc.path, reference) {
                // Resolve the actual commit hash
                if let Ok(obj) = repo.revparse_single(reference) {
                    if let Ok(commit) = obj.peel_to_commit() {
                        let commit_hash = format!("{}", commit.id());
                        return Ok((content, commit_hash));
                    }
                }
            }
        }

        // Fallback: read directly from filesystem
        let file_path = repo_root.join(&doc.path);
        if !file_path.exists() {
            return Err(anyhow!("Document file not found: {}", path));
        }

        let content = std::fs::read_to_string(file_path)
            .map_err(|e| anyhow!("Failed to read document file: {}", e))?;

        // Return "working-tree" as commit hash to indicate non-git content
        Ok((content, "working-tree".to_string()))
    }

    /// Get document history from git.
    ///
    /// Note: Part of public API used by jit-server.
    #[allow(dead_code)]
    pub fn get_document_history(&self, issue_id: &str, path: &str) -> Result<Vec<CommitInfo>> {
        use git2::Repository;

        let issue = self.storage.load_issue(issue_id)?;

        // Verify document reference exists
        issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                anyhow!(
                    "Document reference {} not found in issue {}",
                    path,
                    issue_id
                )
            })?;

        // Get repository root (parent of .jit directory)
        let repo_root = self
            .storage
            .root()
            .parent()
            .ok_or_else(|| anyhow!("Invalid storage path"))?;

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
    #[allow(dead_code)]
    pub fn get_document_diff(
        &self,
        issue_id: &str,
        path: &str,
        from: &str,
        to: Option<&str>,
    ) -> Result<String> {
        use git2::Repository;
        use similar::{ChangeTag, TextDiff};

        let issue = self.storage.load_issue(issue_id)?;

        // Verify document reference exists
        issue
            .documents
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| {
                anyhow!(
                    "Document reference {} not found in issue {}",
                    path,
                    issue_id
                )
            })?;

        // Get repository root (parent of .jit directory)
        let repo_root = self
            .storage
            .root()
            .parent()
            .ok_or_else(|| anyhow!("Invalid storage path"))?;

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
        Err(anyhow!(
            "Document diff not available (requires git repository with history)"
        ))
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
        use std::fs;
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
            .ok_or_else(|| anyhow!("Invalid storage path"))?;

        let doc_path = repo_root.join(path);

        // Rescan if requested
        let assets = if rescan {
            if let Ok(content) = fs::read_to_string(&doc_path) {
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
        use anyhow::anyhow;
        use std::path::PathBuf;

        // Get repository root
        let repo_root = self
            .storage
            .root()
            .parent()
            .ok_or_else(|| anyhow!("Invalid storage path"))?;

        // Parse scope and get documents to check
        let issues = if scope == "all" {
            self.storage.list_issues()?
        } else if let Some(issue_id) = scope.strip_prefix("issue:") {
            let full_id = self.storage.resolve_issue_id(issue_id)?;
            let issue = self.storage.load_issue(&full_id)?;
            vec![issue]
        } else {
            return Err(anyhow!(
                "Invalid scope '{}'. Use 'all' or 'issue:ID'",
                scope
            ));
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
    /// Archive a document with its assets
    /// Archive a document and its per-doc assets to a configured archive directory.
    ///
    /// Returns (result, warnings) tuple where result contains archival details and
    /// warnings contains any non-fatal issues encountered.
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
            .ok_or_else(|| anyhow!("Invalid storage path"))?;

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

        // 7. Perform atomic move with temp directory pattern
        let warnings =
            self.atomic_archive_move(doc_path, &dest_path, &per_doc_assets, repo_root)?;
        let assets_moved = per_doc_assets.len();

        // 8. Verify links still work post-move
        self.verify_post_archival_links(&dest_path, &per_doc_assets, repo_root)?;

        // 9. Update issue metadata
        let updated_issues = self.update_issue_metadata(path, dest_path.to_str().unwrap())?;

        // 10. Log event
        let event = crate::domain::Event::new_document_archived(
            path.to_string(),
            dest_path.to_str().unwrap().to_string(),
            category.to_string(),
            updated_issues.len(),
        );
        self.storage.append_event(&event)?;

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

        // Check if file exists
        let full_path = repo_root.join(doc_path);
        if !full_path.exists() {
            return Err(anyhow!("❌ Document not found: {}", path_str));
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

    /// Update issue metadata to reflect new document path
    fn update_issue_metadata(&self, old_path: &str, new_path: &str) -> Result<Vec<String>> {
        let mut issues = self.storage.list_issues()?;
        let mut updated = Vec::new();

        for issue in &mut issues {
            let mut changed = false;
            for doc in &mut issue.documents {
                if doc.path == old_path {
                    doc.path = new_path.to_string();
                    changed = true;
                }
            }

            if changed {
                let issue_id = issue.id.clone();
                self.storage.save_issue(issue.clone())?;
                updated.push(issue_id);
            }
        }

        Ok(updated)
    }

    /// Scan document and classify assets as per-doc vs shared
    fn scan_and_classify_assets(
        &self,
        doc_path: &str,
        repo_root: &std::path::Path,
    ) -> Result<Vec<std::path::PathBuf>> {
        use crate::document::{AdapterRegistry, AssetScanner};
        use anyhow::Context;
        use std::fs;
        use std::path::Path;

        let full_path = repo_root.join(doc_path);
        let content = fs::read_to_string(&full_path)
            .with_context(|| format!("Failed to read document: {}", doc_path))?;

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
        use anyhow::{anyhow, Context};
        use std::collections::HashSet;
        use std::fs;
        use std::path::Path;

        let full_path = repo_root.join(doc_path);
        let content = fs::read_to_string(&full_path).with_context(|| {
            format!("Failed to read document for link validation: {}", doc_path)
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

    /// Perform atomic archive move using temp directory pattern.
    ///
    /// All-or-nothing file moves with automatic rollback on error.
    ///
    /// # Implementation Steps
    ///
    /// 1. Copy all files (doc + assets) to temp directory
    /// 2. Move from temp to final destinations (atomic renames)
    /// 3. On error: rollback (delete any partial destinations)
    /// 4. Delete sources only after all destinations verified
    ///
    /// Perform atomic move of document and assets using temp directory pattern.
    ///
    /// Returns warnings if any source files couldn't be removed (non-fatal).
    fn atomic_archive_move(
        &self,
        source_doc: &std::path::Path,
        dest_doc: &std::path::Path,
        assets: &[std::path::PathBuf],
        repo_root: &std::path::Path,
    ) -> Result<Vec<String>> {
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

        // Step 2: Move from temp to final destinations
        // Track destinations for rollback
        let mut created_destinations = Vec::new();

        let move_result: Result<()> = (|| {
            for (_source_rel, dest_rel, temp_file) in &files_to_move {
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

                created_destinations.push(dest_full);
            }
            Ok(())
        })();

        // Step 3: Handle errors with rollback
        if let Err(e) = move_result {
            // Rollback: delete any files we created in destination
            for dest in created_destinations {
                let _ = std::fs::remove_file(&dest);
            }
            cleanup_temp();
            return Err(e);
        }

        // Step 4: Delete sources only after all destinations verified
        let mut warnings = Vec::new();
        for (source_rel, _, _) in &files_to_move {
            let source_full = repo_root.join(source_rel);
            if let Err(e) = std::fs::remove_file(&source_full) {
                // Critical: we've moved files but can't delete source
                // Log warning but don't fail - destination is valid
                warnings.push(format!(
                    "Failed to remove source file {}: {}",
                    source_full.display(),
                    e
                ));
            }
        }

        // Cleanup temp directory
        cleanup_temp();

        Ok(warnings)
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
        use anyhow::Context;
        use std::collections::HashSet;
        use std::fs;

        // Read archived document content
        let full_path = repo_root.join(dest_doc);
        let content = fs::read_to_string(&full_path).with_context(|| {
            format!(
                "Failed to read archived document for verification: {}",
                dest_doc.display()
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
                This indicates an error in the archival process. The document has been moved\n\
                but some asset links are broken. Please report this as a bug.",
                broken_links.len(),
                dest_doc.display(),
                broken_links.join("\n")
            ));
        }

        Ok(())
    }
}
