//! Document reference operations

use super::*;

impl<S: IssueStore> CommandExecutor<S> {
    pub fn add_document_reference(
        &self,
        issue_id: &str,
        path: &str,
        commit: Option<&str>,
        label: Option<&str>,
        doc_type: Option<&str>,
        json: bool,
    ) -> Result<()> {
        use crate::domain::DocumentReference;
        use crate::output::{JsonError, JsonOutput};

        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut issue = self.storage.load_issue(&full_id).inspect_err(|_| {
            if json {
                let err = JsonError::issue_not_found(issue_id);
                println!("{}", err.to_json_string().unwrap());
            }
        })?;

        let doc_ref = DocumentReference {
            path: path.to_string(),
            commit: commit.map(String::from),
            label: label.map(String::from),
            doc_type: doc_type.map(String::from),
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
}
