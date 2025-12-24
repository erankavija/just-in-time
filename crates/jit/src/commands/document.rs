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
    ) -> Result<()> {
        use crate::document::{AdapterRegistry, AssetScanner};
        use crate::domain::DocumentReference;
        use crate::output::{JsonError, JsonOutput};
        use anyhow::anyhow;
        use std::fs;
        use std::path::Path;

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id).inspect_err(|_| {
            if json {
                let err = JsonError::issue_not_found(issue_id);
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
                        eprintln!("Warning: Failed to scan assets: {}", e);
                        Vec::new()
                    })
            } else {
                Vec::new()
            };

            (format, assets)
        } else {
            // File doesn't exist or can't be read - skip scanning but don't fail
            eprintln!(
                "Warning: Could not read document at {} - skipping asset scan",
                path
            );
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
        self.storage.save_issue(&issue)?;

        if json {
            let output = JsonOutput::success(serde_json::json!({
                "issue_id": full_id,
                "document": doc_ref,
            }));
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

        Ok(())
    }

    pub fn list_document_references(&self, issue_id: &str, json: bool) -> Result<()> {
        use crate::output::{JsonError, JsonOutput};

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id).inspect_err(|_| {
            if json {
                let err = JsonError::issue_not_found(issue_id);
                println!("{}", err.to_json_string().unwrap());
            }
        })?;

        if json {
            let output = JsonOutput::success(serde_json::json!({
                "issue_id": full_id,
                "documents": issue.documents,
                "count": issue.documents.len(),
            }));
            println!("{}", output.to_json_string()?);
        } else if issue.documents.is_empty() {
            println!("No document references for issue {}", full_id);
        } else {
            println!("Document references for issue {}:", full_id);
            for doc in &issue.documents {
                print!("  - {}", doc.path);
                if let Some(ref label) = doc.label {
                    print!(" ({})", label);
                }
                if let Some(ref commit) = doc.commit {
                    print!(" [{}]", &commit[..7.min(commit.len())]);
                } else {
                    print!(" [HEAD]");
                }
                if let Some(ref doc_type) = doc.doc_type {
                    print!(" <{}>", doc_type);
                }
                println!();
            }
            println!("\nTotal: {}", issue.documents.len());
        }

        Ok(())
    }

    pub fn remove_document_reference(&self, issue_id: &str, path: &str, json: bool) -> Result<()> {
        use crate::output::{JsonError, JsonOutput};

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id).inspect_err(|_| {
            if json {
                let err = JsonError::issue_not_found(issue_id);
                println!("{}", err.to_json_string().unwrap());
            }
        })?;

        let original_len = issue.documents.len();
        issue.documents.retain(|doc| doc.path != path);

        if issue.documents.len() == original_len {
            let err_msg = format!("Document reference {} not found in issue {}", path, full_id);
            if json {
                let err = JsonError::new("DOCUMENT_NOT_FOUND", &err_msg)
                    .with_suggestion("Run 'jit doc list <issue-id>' to see available documents");
                println!("{}", err.to_json_string()?);
            }
            return Err(anyhow!(err_msg));
        }

        self.storage.save_issue(&issue)?;

        if json {
            let output = JsonOutput::success(serde_json::json!({
                "issue_id": full_id,
                "removed_path": path,
            }));
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

        // Try to read content from git
        let repo = Repository::open(".").map_err(|e| anyhow!("Not a git repository: {}", e))?;

        let content = self
            .read_file_from_git(&repo, &doc.path, reference)
            .map_err(|e| anyhow!("Error reading file from git: {}", e))?;

        println!("{}", content);

        Ok(())
    }

    pub fn document_history(&self, issue_id: &str, path: &str, json: bool) -> Result<()> {
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

        if json {
            let json_output = serde_json::to_string_pretty(&commits)?;
            println!("{}", json_output);
        } else {
            println!("History for {}:", path);
            println!();
            for commit in commits {
                println!("commit {}", commit.sha);
                println!("Author: {}", commit.author);
                println!("Date:   {}", commit.date);
                println!();
                println!("    {}", commit.message);
                println!();
            }
        }

        Ok(())
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
        json: bool,
    ) -> Result<()> {
        use crate::document::{AdapterRegistry, AssetScanner, AssetType};
        use crate::output::{JsonError, JsonOutput};
        use anyhow::anyhow;
        use std::fs;
        use std::path::Path;

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id).inspect_err(|_| {
            if json {
                let err = JsonError::issue_not_found(issue_id);
                println!("{}", err.to_json_string().unwrap());
            }
        })?;

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
                        eprintln!("Warning: Failed to scan assets: {}", e);
                        Vec::new()
                    });

                // Update the document with rescanned assets
                issue.documents[doc_index].assets = scanned_assets.clone();
                self.storage.save_issue(&issue)?;

                scanned_assets
            } else {
                eprintln!("Warning: Could not read document at {}", path);
                issue.documents[doc_index].assets.clone()
            }
        } else {
            issue.documents[doc_index].assets.clone()
        };

        // Check if assets actually exist
        let assets_with_status: Vec<_> = assets
            .iter()
            .map(|asset| {
                let exists = match &asset.resolved_path {
                    Some(p) => repo_root.join(p).exists(),
                    None => false,
                };
                (asset, exists)
            })
            .collect();

        // Categorize assets
        let per_doc: Vec<_> = assets_with_status
            .iter()
            .filter(|(a, _)| !a.is_shared && a.asset_type == AssetType::Local)
            .collect();
        let shared: Vec<_> = assets_with_status
            .iter()
            .filter(|(a, _)| a.is_shared && a.asset_type == AssetType::Local)
            .collect();
        let external: Vec<_> = assets_with_status
            .iter()
            .filter(|(a, _)| a.asset_type == AssetType::External)
            .collect();
        let missing: Vec<_> = assets_with_status
            .iter()
            .filter(|(a, _)| a.asset_type == AssetType::Missing)
            .collect();

        if json {
            let output = JsonOutput::success(serde_json::json!({
                "issue_id": full_id,
                "document_path": path,
                "assets": assets,
                "summary": {
                    "total": assets.len(),
                    "per_doc": per_doc.len(),
                    "shared": shared.len(),
                    "external": external.len(),
                    "missing": missing.len(),
                },
            }));
            println!("{}", output.to_json_string()?);
        } else {
            println!("Assets for document {} (issue {}):", path, &full_id[..8]);

            if assets.is_empty() {
                println!("  No assets found for this document");
                return Ok(());
            }

            if !per_doc.is_empty() {
                println!("\nPer-document assets:");
                for (asset, exists) in &per_doc {
                    let status = if *exists { "✓" } else { "✗" };
                    println!("  {} {}", status, asset.original_path);
                    if let Some(ref resolved) = asset.resolved_path {
                        println!("     → {}", resolved.display());
                    }
                    if let Some(ref mime) = asset.mime_type {
                        println!("     MIME: {}", mime);
                    }
                }
            }

            if !shared.is_empty() {
                println!("\nShared assets:");
                for (asset, exists) in &shared {
                    let status = if *exists { "✓" } else { "✗" };
                    println!("  {} {}", status, asset.original_path);
                    if let Some(ref resolved) = asset.resolved_path {
                        println!("     → {}", resolved.display());
                    }
                }
            }

            if !external.is_empty() {
                println!("\nExternal URLs:");
                for (asset, _) in &external {
                    println!("  🌐 {}", asset.original_path);
                }
            }

            if !missing.is_empty() {
                println!("\n⚠ Missing assets:");
                for (asset, _) in &missing {
                    println!("  ✗ {}", asset.original_path);
                    if let Some(ref resolved) = asset.resolved_path {
                        println!("     Expected at: {}", resolved.display());
                    }
                }
            }

            println!(
                "\nSummary: {} total ({} per-doc, {} shared, {} external, {} missing)",
                assets.len(),
                per_doc.len(),
                shared.len(),
                external.len(),
                missing.len()
            );
        }

        Ok(())
    }

    /// Check document links and assets for validity
    ///
    /// Returns exit code: 0 (all valid), 1 (errors), 2 (warnings only)
    pub fn check_document_links(&self, scope: &str, json: bool) -> Result<i32> {
        use crate::document::{AssetType, LinkValidationResult, LinkValidator};
        use crate::output::{JsonError, JsonOutput};
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
            let issue = self.storage.load_issue(&full_id).inspect_err(|_| {
                if json {
                    let err = JsonError::issue_not_found(issue_id);
                    println!("{}", err.to_json_string().unwrap());
                }
            })?;
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
            if json {
                let output = JsonOutput::success(serde_json::json!({
                    "valid": true,
                    "errors": [],
                    "warnings": [],
                    "summary": {
                        "total_documents": 0,
                        "valid": 0,
                        "errors": 0,
                        "warnings": 0,
                    }
                }));
                println!("{}", output.to_json_string()?);
            } else {
                println!("No documents found to check");
            }
            return Ok(0);
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
                        // External URLs are just warnings
                        warnings.push(serde_json::json!({
                            "issue_id": issue_id,
                            "document": doc.path,
                            "type": "external_asset",
                            "asset": asset.original_path,
                            "message": format!("External URL (not validated): {}", asset.original_path),
                        }));
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

        // Determine exit code
        let exit_code = if !errors.is_empty() {
            1 // Errors found
        } else if !warnings.is_empty() {
            2 // Only warnings
        } else {
            0 // All valid
        };

        // Output results
        if json {
            let output = JsonOutput::success(serde_json::json!({
                "valid": errors.is_empty(),
                "errors": errors,
                "warnings": warnings,
                "summary": {
                    "total_documents": all_documents.len(),
                    "valid": all_documents.len() - errors.len(),
                    "errors": errors.len(),
                    "warnings": warnings.len(),
                }
            }));
            println!("{}", output.to_json_string()?);
        } else {
            println!(
                "Checking {} document(s) in scope '{}'...\n",
                all_documents.len(),
                scope
            );

            if !errors.is_empty() {
                println!("❌ Errors found ({}):", errors.len());
                for error in &errors {
                    println!(
                        "  {} ({}): {}",
                        error["document"].as_str().unwrap_or(""),
                        error["type"].as_str().unwrap_or(""),
                        error["message"].as_str().unwrap_or("")
                    );
                }
                println!();
            }

            if !warnings.is_empty() {
                println!("⚠️  Warnings ({}):", warnings.len());
                for warning in &warnings {
                    println!(
                        "  {} ({}): {}",
                        warning["document"].as_str().unwrap_or(""),
                        warning["type"].as_str().unwrap_or(""),
                        warning["message"].as_str().unwrap_or("")
                    );
                }
                println!();
            }

            if errors.is_empty() && warnings.is_empty() {
                println!("✅ All documents valid!");
            }

            println!(
                "Summary: {} document(s) checked, {} error(s), {} warning(s)",
                all_documents.len(),
                errors.len(),
                warnings.len()
            );
        }

        Ok(exit_code)
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
