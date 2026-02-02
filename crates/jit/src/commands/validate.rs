//! Validation and status operations

use super::*;
use crate::type_hierarchy::{
    detect_validation_issues, generate_fixes, ValidationFix, ValidationIssue,
};
use anyhow::Context;

/// Validation configuration flags loaded from config.toml.
#[derive(Debug, Clone)]
struct ValidationConfigFlags {
    #[allow(dead_code)] // Reserved for future strictness levels (strict/loose/permissive)
    strictness: String,
    warn_orphaned_leaves: bool,
    warn_strategic_consistency: bool,
}

impl Default for ValidationConfigFlags {
    fn default() -> Self {
        Self {
            strictness: "loose".to_string(),
            warn_orphaned_leaves: true,
            warn_strategic_consistency: true,
        }
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    #[allow(dead_code)] // Used internally by validate_with_options
    pub fn validate(&self) -> Result<()> {
        self.validate_silent()?;
        println!("✓ Repository is valid");
        Ok(())
    }

    /// Validate with optional fix mode.
    ///
    /// # Arguments
    ///
    /// * `fix` - If true, attempt to fix validation issues
    /// * `dry_run` - If true, show what would be fixed without applying changes
    /// * `quiet` - If true, suppress progress messages (for JSON output)
    ///
    /// # Returns
    ///
    /// Returns Ok with count of fixes applied, or Err if validation fails
    pub fn validate_with_fix(&mut self, fix: bool, dry_run: bool, quiet: bool) -> Result<usize> {
        // First run standard validation
        let validation_result = self.validate_silent();

        if validation_result.is_ok() && !fix {
            return Ok(0);
        }

        // If fix mode is enabled, detect and fix issues
        if fix {
            let mut total_fixes = 0;

            // Fix type hierarchy issues
            let hierarchy_fixes = self.detect_and_fix_hierarchy_issues(dry_run, quiet)?;
            total_fixes += hierarchy_fixes;

            // Fix transitive reduction violations (both dry-run and actual fix)
            let reduction_fixes = self.fix_all_transitive_reductions(dry_run, quiet)?;
            total_fixes += reduction_fixes;

            if !quiet {
                if dry_run {
                    println!(
                        "\nDry run complete. {} fixes would be applied.",
                        total_fixes
                    );
                } else if total_fixes > 0 {
                    println!("\n✓ Applied {} fixes", total_fixes);

                    // Re-run validation to verify fixes worked
                    self.validate_silent()?;
                    println!("✓ Repository is now valid");
                } else {
                    println!("\n✓ No fixes needed");
                }
            }

            return Ok(total_fixes);
        }

        // Not in fix mode, just propagate the validation error
        validation_result?;
        Ok(0)
    }

    fn detect_and_fix_hierarchy_issues(&mut self, dry_run: bool, quiet: bool) -> Result<usize> {
        use crate::hierarchy_templates::get_hierarchy_config;

        let config = get_hierarchy_config(&self.storage)?;
        let issues = self.storage.list_issues()?;

        // Collect all validation issues
        let mut all_validation_issues = Vec::new();

        for issue in &issues {
            let validation_issues = detect_validation_issues(&config, &issue.id, &issue.labels);
            all_validation_issues.extend(validation_issues);
        }

        if all_validation_issues.is_empty() {
            return Ok(0);
        }

        // Generate fixes
        let fixes = generate_fixes(&all_validation_issues);

        if fixes.is_empty() {
            // We found issues but can't auto-fix them
            if !quiet {
                println!(
                    "\nFound {} validation issues but no automatic fixes available:",
                    all_validation_issues.len()
                );
                for issue in &all_validation_issues {
                    match issue {
                        ValidationIssue::UnknownType {
                            issue_id,
                            unknown_type,
                            ..
                        } => {
                            println!("  • Issue {} has unknown type '{}'", issue_id, unknown_type);
                        }
                        ValidationIssue::InvalidMembershipReference {
                            issue_id,
                            label,
                            reason,
                            ..
                        } => {
                            println!(
                                "  • Issue {} has invalid membership label '{}': {}",
                                issue_id, label, reason
                            );
                        }
                    }
                }
            }
            return Ok(0);
        }

        // Apply or preview fixes
        let mut fixes_applied = 0;

        for fix in &fixes {
            match fix {
                ValidationFix::ReplaceType {
                    issue_id,
                    old_type,
                    new_type,
                } => {
                    if !quiet {
                        if dry_run {
                            println!(
                                "Would replace type '{}' with '{}' for issue {}",
                                old_type, new_type, issue_id
                            );
                        } else {
                            self.apply_type_fix(issue_id, old_type, new_type)?;
                            println!(
                                "✓ Replaced type '{}' with '{}' for issue {}",
                                old_type, new_type, issue_id
                            );
                        }
                    } else if !dry_run {
                        self.apply_type_fix(issue_id, old_type, new_type)?;
                    }
                    fixes_applied += 1;
                }
            }
        }

        Ok(fixes_applied)
    }

    fn apply_type_fix(&mut self, issue_id: &str, old_type: &str, new_type: &str) -> Result<()> {
        let mut issue = self.storage.load_issue(issue_id)?;

        // Replace the type label
        let old_label = format!("type:{}", old_type);
        let new_label = format!("type:{}", new_type);

        issue.labels.retain(|l| l != &old_label);
        issue.labels.push(new_label);

        self.storage.save_issue(&issue)?;
        Ok(())
    }

    // Note: apply_dependency_reversal is removed - we don't reverse dependencies
    // Type hierarchy is orthogonal to DAG structure

    pub fn validate_silent(&self) -> Result<()> {
        let issues = self.storage.list_issues()?;

        // Build lookup map of valid issue IDs
        let valid_ids: std::collections::HashSet<String> =
            issues.iter().map(|i| i.id.clone()).collect();

        // Check for broken dependency references
        for issue in &issues {
            for dep in &issue.dependencies {
                if !valid_ids.contains(dep) {
                    return Err(anyhow!(
                        "Invalid dependency: issue '{}' depends on '{}' which does not exist",
                        issue.id,
                        dep
                    ));
                }
            }
        }

        // Check for invalid gate references
        let registry = self.storage.load_gate_registry()?;
        for issue in &issues {
            for gate_key in &issue.gates_required {
                if !registry.gates.contains_key(gate_key) {
                    return Err(anyhow!(
                        "Gate '{}' required by issue '{}' is not defined in registry",
                        gate_key,
                        issue.id
                    ));
                }
            }
        }

        // Validate labels
        self.validate_labels(&issues)?;

        // Validate document references (git integration)
        self.validate_document_references(&issues)?;

        // Validate type hierarchy
        self.validate_type_hierarchy(&issues)?;

        // Validate DAG (no cycles)
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);
        graph.validate_dag()?;

        // Validate transitive reduction (no redundant dependencies)
        self.validate_transitive_reduction(&graph, &issues)?;

        // Validate claims index (if worktree mode is active and not in test mode)
        if std::env::var("JIT_TEST_MODE").is_err() {
            let index_issues = validate_claims_index()
                .unwrap_or_else(|e| vec![format!("Failed to validate claims index: {}", e)]);
            if !index_issues.is_empty() {
                return Err(anyhow!(
                    "Claims index validation failed:\n  {}",
                    index_issues.join("\n  ")
                ));
            }
        }

        Ok(())
    }

    fn validate_labels(&self, issues: &[Issue]) -> Result<()> {
        let namespaces = self.config_manager.get_namespaces()?;

        for issue in issues {
            // Check label format
            for label in &issue.labels {
                label_utils::validate_label(label)
                    .map_err(|e| anyhow!("Invalid label format in issue '{}': {}", issue.id, e))?;
            }

            // Check namespace exists in registry
            for label in &issue.labels {
                if let Ok((namespace, _)) = label_utils::parse_label(label) {
                    if !namespaces.namespaces.contains_key(&namespace) {
                        return Err(anyhow!(
                            "Issue '{}' has label with unknown namespace '{}'. \
                             Label: '{}'. Available namespaces: {}",
                            issue.id,
                            namespace,
                            label,
                            namespaces
                                .namespaces
                                .keys()
                                .map(|k| k.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                }
            }

            // Check uniqueness constraints
            let mut unique_namespaces_seen = std::collections::HashMap::new();
            for label in &issue.labels {
                if let Ok((namespace, _)) = label_utils::parse_label(label) {
                    if let Some(ns_config) = namespaces.namespaces.get(&namespace) {
                        if ns_config.unique {
                            if let Some(first_label) = unique_namespaces_seen.get(&namespace) {
                                return Err(anyhow!(
                                    "Issue '{}' has multiple labels from unique namespace '{}': '{}' and '{}'",
                                    issue.id,
                                    namespace,
                                    first_label,
                                    label
                                ));
                            }
                            unique_namespaces_seen.insert(namespace, label.clone());
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn validate_type_hierarchy(&self, issues: &[Issue]) -> Result<()> {
        use crate::hierarchy_templates::get_hierarchy_config;
        use crate::type_hierarchy::{detect_validation_issues, ValidationIssue};

        let config = get_hierarchy_config(&self.storage)?;

        for issue in issues {
            let validation_issues = detect_validation_issues(&config, &issue.id, &issue.labels);

            // Report first validation issue found
            if let Some(val_issue) = validation_issues.into_iter().next() {
                match val_issue {
                    ValidationIssue::UnknownType {
                        issue_id,
                        unknown_type,
                        suggested_fix,
                    } => {
                        let suggestion = suggested_fix
                            .map(|s| format!(" (did you mean '{}'?)", s))
                            .unwrap_or_default();
                        return Err(anyhow!(
                            "Issue '{}' has unknown type '{}'{}",
                            issue_id,
                            unknown_type,
                            suggestion
                        ));
                    }
                    ValidationIssue::InvalidMembershipReference {
                        issue_id,
                        label,
                        reason,
                        ..
                    } => {
                        return Err(anyhow!(
                            "Issue '{}' has invalid membership label '{}': {}",
                            issue_id,
                            label,
                            reason
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    fn validate_document_references(&self, issues: &[Issue]) -> Result<()> {
        use git2::Repository;

        // Try to open git repository
        let repo = match Repository::open(".") {
            Ok(r) => r,
            Err(_) => {
                // If not a git repo, skip document validation
                return Ok(());
            }
        };

        // Check if repository has any commits (HEAD exists)
        let has_commits = repo.head().is_ok();

        for issue in issues {
            for doc in &issue.documents {
                // Validate commit hash if specified
                if let Some(ref commit_hash) = doc.commit {
                    // Use revparse to resolve short hashes
                    let resolved_oid = repo
                        .revparse_single(commit_hash)
                        .and_then(|obj| obj.peel_to_commit())
                        .map(|commit| commit.id());

                    if resolved_oid.is_err() {
                        return Err(anyhow!(
                            "Invalid document reference in issue '{}': commit '{}' not found for '{}'",
                            issue.id,
                            commit_hash,
                            doc.path
                        ));
                    }

                    // Validate file exists at the specified commit
                    if self
                        .check_file_exists_in_git(&repo, &doc.path, commit_hash)
                        .is_err()
                    {
                        return Err(anyhow!(
                            "Invalid document reference in issue '{}': file '{}' not found at commit {}",
                            issue.id,
                            doc.path,
                            commit_hash
                        ));
                    }
                } else {
                    // No commit specified - check working tree or HEAD
                    if has_commits {
                        // Repository has commits - validate against HEAD
                        if self
                            .check_file_exists_in_git(&repo, &doc.path, "HEAD")
                            .is_err()
                        {
                            return Err(anyhow!(
                                "Invalid document reference in issue '{}': file '{}' not found at HEAD",
                                issue.id,
                                doc.path
                            ));
                        }
                    } else {
                        // Repository has no commits - check working tree only
                        let path = std::path::Path::new(&doc.path);
                        if !path.exists() {
                            return Err(anyhow!(
                                "Invalid document reference in issue '{}': file '{}' not found in working tree (repository has no commits yet)",
                                issue.id,
                                doc.path
                            ));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn check_file_exists_in_git(
        &self,
        repo: &git2::Repository,
        path: &str,
        reference: &str,
    ) -> Result<()> {
        let obj = repo.revparse_single(reference)?;
        let commit = obj.peel_to_commit()?;
        let tree = commit.tree()?;

        // Try to find the file in the tree
        tree.get_path(std::path::Path::new(path))?;

        Ok(())
    }

    pub fn status(&self) -> Result<()> {
        let summary = self.get_status()?;

        println!("Status:");
        println!("  Open: {}", summary.open);
        println!("  Ready: {}", summary.ready);
        println!("  In Progress: {}", summary.in_progress);
        println!("  Done: {}", summary.done);
        println!("  Rejected: {}", summary.rejected);
        println!("  Blocked: {}", summary.blocked);

        Ok(())
    }

    pub fn get_status(&self) -> Result<StatusSummary> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let resolved: HashMap<String, &Issue> =
            issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

        let backlog = issues.iter().filter(|i| i.state == State::Backlog).count();
        let ready = issues.iter().filter(|i| i.state == State::Ready).count();
        let in_progress = issues
            .iter()
            .filter(|i| i.state == State::InProgress)
            .count();
        let gated = issues.iter().filter(|i| i.state == State::Gated).count();
        let done = issues.iter().filter(|i| i.state == State::Done).count();
        let rejected = issues.iter().filter(|i| i.state == State::Rejected).count();
        let blocked = issues.iter().filter(|i| i.is_blocked(&resolved)).count();

        Ok(StatusSummary {
            open: backlog, // Keep 'open' field name for backward compatibility
            ready,
            in_progress,
            done,
            rejected,
            blocked,
            gated,
            total: issues.len(),
        })
    }

    /// Check for validation warnings on a specific issue.
    ///
    /// Returns warnings for:
    /// - Strategic types (epic, milestone) missing identifying labels
    /// - Leaf types (task) without parent associations
    ///
    /// # Arguments
    ///
    /// * `issue_id` - The ID of the issue to check
    ///
    /// # Returns
    ///
    /// A vector of validation warnings (empty if no warnings)
    pub fn check_warnings(
        &self,
        issue_id: &str,
    ) -> Result<Vec<crate::type_hierarchy::ValidationWarning>> {
        use crate::type_hierarchy::{validate_orphans, validate_strategic_labels};

        // Load the issue
        let issue = self.storage.load_issue(issue_id)?;

        // Load hierarchy config from file or use defaults
        let config = self.load_hierarchy_config()?;

        // Load validation config to check toggles
        let validation_config = self.load_validation_config()?;

        // Collect all warnings
        let mut warnings = Vec::new();

        // Check strategic label consistency (if enabled)
        if validation_config.warn_strategic_consistency {
            warnings.extend(validate_strategic_labels(&config, &issue));
        }

        // Check for orphaned leaves (if enabled)
        if validation_config.warn_orphaned_leaves {
            warnings.extend(validate_orphans(&config, &issue));
        }

        Ok(warnings)
    }

    /// Load hierarchy configuration from config.toml or fall back to defaults.
    fn load_hierarchy_config(&self) -> Result<crate::type_hierarchy::HierarchyConfig> {
        use crate::config::JitConfig;
        use crate::type_hierarchy::HierarchyConfig;

        let jit_config = JitConfig::load(self.storage.root())?;

        if let Some(hierarchy_toml) = jit_config.type_hierarchy {
            // Use config from file
            let label_associations = hierarchy_toml.label_associations.unwrap_or_default();
            HierarchyConfig::new(hierarchy_toml.types, label_associations)
                .map_err(|e| anyhow::anyhow!("Invalid hierarchy config: {}", e))
        } else {
            // Fall back to defaults
            Ok(HierarchyConfig::default())
        }
    }

    /// Load validation configuration from config.toml or use defaults.
    fn load_validation_config(&self) -> Result<ValidationConfigFlags> {
        use crate::config::JitConfig;

        let jit_config = JitConfig::load(self.storage.root())?;

        let flags = if let Some(validation) = jit_config.validation {
            ValidationConfigFlags {
                strictness: validation.strictness.unwrap_or_else(|| "loose".to_string()),
                warn_orphaned_leaves: validation.warn_orphaned_leaves.unwrap_or(true),
                warn_strategic_consistency: validation.warn_strategic_consistency.unwrap_or(true),
            }
        } else {
            ValidationConfigFlags::default()
        };

        Ok(flags)
    }

    /// Collect all validation warnings for all issues in the repository.
    ///
    /// Returns a vector of (issue_id, warnings) pairs for all issues with warnings.
    pub fn collect_all_warnings(
        &self,
    ) -> Result<Vec<(String, Vec<crate::type_hierarchy::ValidationWarning>)>> {
        let issues = self.storage.list_issues()?;
        let mut all_warnings = Vec::new();

        for issue in issues {
            let warnings = self.check_warnings(&issue.id)?;
            if !warnings.is_empty() {
                all_warnings.push((issue.id.clone(), warnings));
            }
        }

        Ok(all_warnings)
    }

    /// Validate transitive reduction of dependency graph.
    ///
    /// Checks that no issue has redundant dependencies - dependencies that are
    /// already reachable through other dependency paths.
    fn validate_transitive_reduction(
        &self,
        graph: &DependencyGraph<Issue>,
        issues: &[Issue],
    ) -> Result<()> {
        use std::collections::HashSet;

        for issue in issues {
            if issue.dependencies.is_empty() {
                continue;
            }

            // Compute minimal dependency set
            let reduced = graph.compute_transitive_reduction(&issue.id);
            let reduced_set: HashSet<_> = reduced.iter().collect();

            // Find redundant edges (in current but not in reduction)
            for dep_id in &issue.dependencies {
                if !reduced_set.contains(dep_id) {
                    // This edge is redundant - find the transitive path
                    let path = graph.find_shortest_path(&issue.id, dep_id);
                    let path_str = if path.is_empty() {
                        "unknown path".to_string()
                    } else {
                        path.iter()
                            .map(|id| &id[..8.min(id.len())])
                            .collect::<Vec<_>>()
                            .join(" → ")
                    };

                    return Err(anyhow!(
                        "Transitive reduction violation: Issue {} has redundant dependency on {} \
                         (already reachable via: {}). Run 'jit validate --fix' to remove redundant edges.",
                        &issue.id[..8],
                        &dep_id[..8],
                        path_str
                    ));
                }
            }
        }

        Ok(())
    }

    /// Fix transitive reduction violations by removing redundant dependencies.
    ///
    /// Returns the number of redundant edges removed (not issues fixed).
    fn fix_transitive_reduction(
        &mut self,
        graph: &DependencyGraph<Issue>,
        issue_id: &str,
        dry_run: bool,
    ) -> Result<usize> {
        let mut issue = self.storage.load_issue(issue_id)?;

        if issue.dependencies.is_empty() {
            return Ok(0);
        }

        let reduced = graph.compute_transitive_reduction(issue_id);
        let current_len = issue.dependencies.len();
        let reduced_len = reduced.len();

        if current_len == reduced_len {
            // No redundancies
            return Ok(0);
        }

        let redundant_count = current_len - reduced_len;

        if !dry_run {
            // Actually apply the fix
            issue.dependencies = reduced.into_iter().collect();
            self.storage.save_issue(&issue)?;
        }

        // Note: Event logging for transitive reduction fixes could be added
        // via a new Event variant in future if needed for audit trail

        Ok(redundant_count)
    }

    /// Fix transitive reduction violations for all issues.
    ///
    /// Returns count of redundant edges fixed (or that would be fixed if dry_run).
    fn fix_all_transitive_reductions(&mut self, dry_run: bool, quiet: bool) -> Result<usize> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        let mut total_redundancies = 0;

        for issue in &issues {
            if issue.dependencies.is_empty() {
                continue;
            }

            let fixed_count = self.fix_transitive_reduction(&graph, &issue.id, dry_run)?;
            if fixed_count > 0 {
                total_redundancies += fixed_count;
                if !quiet && !dry_run {
                    println!(
                        "Fixed {} redundant {} in issue {}",
                        fixed_count,
                        if fixed_count == 1 {
                            "dependency"
                        } else {
                            "dependencies"
                        },
                        &issue.id[..8.min(issue.id.len())]
                    );
                }
            }
        }

        Ok(total_redundancies)
    }

    /// Validate branch hasn't diverged from main.
    ///
    /// Checks that the current branch shares common history with origin/main
    /// by comparing merge-base with the main commit.
    ///
    /// # Returns
    /// Ok(()) if branch is up-to-date, Err with helpful message if diverged
    pub fn validate_divergence(&self) -> Result<()> {
        use std::process::Command;

        // Get merge-base between HEAD and origin/main
        let merge_base_output = Command::new("git")
            .args(["merge-base", "HEAD", "origin/main"])
            .output()
            .context("Failed to execute git merge-base")?;

        if !merge_base_output.status.success() {
            let stderr = String::from_utf8_lossy(&merge_base_output.stderr);
            anyhow::bail!(
                "Failed to get merge-base with origin/main: {}",
                stderr.trim()
            );
        }

        let merge_base = String::from_utf8(merge_base_output.stdout)?
            .trim()
            .to_string();

        // Get current origin/main commit
        let main_commit_output = Command::new("git")
            .args(["rev-parse", "origin/main"])
            .output()
            .context("Failed to execute git rev-parse")?;

        if !main_commit_output.status.success() {
            let stderr = String::from_utf8_lossy(&main_commit_output.stderr);
            anyhow::bail!("Failed to get origin/main commit: {}", stderr.trim());
        }

        let main_commit = String::from_utf8(main_commit_output.stdout)?
            .trim()
            .to_string();

        // If merge-base != main commit, branch has diverged
        if merge_base != main_commit {
            anyhow::bail!(
                "Branch has diverged from origin/main\n\
                 Merge base: {}\n\
                 Main commit: {}\n\
                 Fix: git rebase origin/main",
                merge_base,
                main_commit
            );
        }

        Ok(())
    }

    /// Validate all active leases are consistent and not stale.
    ///
    /// Checks claims.index.json for:
    /// - Expired leases (TTL exceeded)
    /// - Leases referencing non-existent worktrees
    /// - Leases referencing non-existent issues
    ///
    /// # Returns
    /// Vector of invalid lease descriptions with fix suggestions
    pub fn validate_leases(&self) -> Result<Vec<String>> {
        use crate::storage::claim_coordinator::ClaimsIndex;
        use crate::storage::worktree_paths::WorktreePaths;
        use chrono::Utc;

        let paths = WorktreePaths::detect().context("Failed to detect worktree paths")?;

        // Load claims index
        let claims_index_path = paths.shared_jit.join("claims.index.json");
        if !claims_index_path.exists() {
            // No claims index means no leases - valid
            return Ok(vec![]);
        }

        let contents =
            std::fs::read_to_string(&claims_index_path).context("Failed to read claims index")?;
        let index: ClaimsIndex =
            serde_json::from_str(&contents).context("Failed to parse claims index")?;

        let mut invalid_leases = Vec::new();
        let now = Utc::now();

        for lease in &index.leases {
            // Check if lease has expired
            if let Some(expires_at) = lease.expires_at {
                if expires_at < now {
                    let duration = now.signed_duration_since(expires_at);
                    invalid_leases.push(format!(
                        "Lease {} (Issue {}): Expired {} ago\n  Fix: jit claim release {}",
                        lease.lease_id,
                        &lease.issue_id[..8.min(lease.issue_id.len())],
                        format_duration(duration),
                        &lease.issue_id[..8.min(lease.issue_id.len())]
                    ));
                    continue;
                }
            }

            // Check if worktree still exists
            if !check_worktree_exists(&lease.worktree_id)? {
                invalid_leases.push(format!(
                    "Lease {} (Issue {}): Worktree {} no longer exists\n  Fix: jit claim force-evict {}",
                    lease.lease_id,
                    &lease.issue_id[..8.min(lease.issue_id.len())],
                    lease.worktree_id,
                    lease.lease_id
                ));
                continue;
            }

            // Check if issue still exists (use storage layer for proper resolution)
            if self.storage.load_issue(&lease.issue_id).is_err() {
                invalid_leases.push(format!(
                    "Lease {} (Issue {}): Issue no longer exists\n  Fix: jit claim release {}",
                    lease.lease_id,
                    &lease.issue_id[..8.min(lease.issue_id.len())],
                    &lease.issue_id[..8.min(lease.issue_id.len())]
                ));
            }
        }

        Ok(invalid_leases)
    }
}

/// Format duration in human-readable form
fn format_duration(duration: chrono::Duration) -> String {
    let secs = duration.num_seconds();
    if secs < 60 {
        if secs == 1 {
            "1 second".to_string()
        } else {
            format!("{} seconds", secs)
        }
    } else if secs < 3600 {
        let mins = secs / 60;
        if mins == 1 {
            "1 minute".to_string()
        } else {
            format!("{} minutes", mins)
        }
    } else if secs < 86400 {
        let hours = secs / 3600;
        if hours == 1 {
            "1 hour".to_string()
        } else {
            format!("{} hours", hours)
        }
    } else {
        let days = secs / 86400;
        if days == 1 {
            "1 day".to_string()
        } else {
            format!("{} days", days)
        }
    }
}

/// Check if a worktree with the given ID still exists
fn check_worktree_exists(worktree_id: &str) -> Result<bool> {
    use std::path::PathBuf;
    use std::process::Command;

    // Get all git worktrees
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .output()
        .context("Failed to execute git worktree list")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git worktree list failed: {}", stderr);
    }

    let porcelain_output =
        String::from_utf8(output.stdout).context("Invalid UTF-8 in git worktree output")?;

    // Parse worktree paths
    let worktree_paths = porcelain_output
        .lines()
        .filter(|line| line.starts_with("worktree "))
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect::<Vec<_>>();

    // Check each worktree for matching ID
    for worktree_path in worktree_paths {
        let identity_path = worktree_path.join(".jit/worktree.json");
        if let Ok(contents) = std::fs::read_to_string(&identity_path) {
            // Parse just enough to get the worktree_id
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) {
                if let Some(id) = json.get("worktree_id").and_then(|v| v.as_str()) {
                    if id == worktree_id {
                        return Ok(true);
                    }
                }
            }
        }
    }

    Ok(false)
}

/// Validate claims index consistency
///
/// Checks for structural corruption:
/// - Duplicate leases for the same issue (invariant violation)
/// - Schema version mismatches (incompatibility)
/// - Sequence gaps (data loss indicator)
///
/// Note: Does NOT check for expired leases - those are normal state handled by
/// evict_expired(). Use validate_leases() for expiration checks.
///
/// Returns vector of corruption issues found (empty if structurally valid)
pub fn validate_claims_index() -> Result<Vec<String>> {
    use crate::storage::claim_coordinator::ClaimsIndex;
    use crate::storage::worktree_paths::WorktreePaths;
    use std::collections::HashSet;

    let paths = WorktreePaths::detect().context("Failed to detect worktree paths")?;

    // Load claims index
    let claims_index_path = paths.shared_jit.join("claims.index.json");
    if !claims_index_path.exists() {
        // No claims index - valid (no claims coordination active)
        return Ok(vec![]);
    }

    let contents =
        std::fs::read_to_string(&claims_index_path).context("Failed to read claims index")?;
    let index: ClaimsIndex =
        serde_json::from_str(&contents).context("Failed to parse claims index")?;

    let mut issues = Vec::new();

    // Check schema version
    if index.schema_version != 1 {
        issues.push(format!(
            "Invalid schema version: expected 1, found {}",
            index.schema_version
        ));
    }

    // Check for duplicate leases (same issue claimed twice)
    let mut seen_issues = HashSet::new();
    for lease in &index.leases {
        if !seen_issues.insert(&lease.issue_id) {
            issues.push(format!(
                "Duplicate lease detected for issue {}: Multiple leases exist for the same issue",
                &lease.issue_id[..8.min(lease.issue_id.len())]
            ));
        }
    }

    // Note: Expired leases are NOT considered corruption - they are normal state
    // handled by startup_recovery's evict_expired(). Use validate_leases() to check expiration.

    // Report sequence gaps (if any were detected during rebuild)
    if !index.sequence_gaps.is_empty() {
        issues.push(format!(
            "Sequence gaps detected in claims log: missing sequences {:?}",
            index.sequence_gaps
        ));
    }

    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    // Note: validate_leases() and validate_divergence() require git repository setup
    // and are integration-tested through manual testing and real usage.
    // Unit tests focus on pure functions like format_duration().

    #[test]
    fn test_format_duration_seconds() {
        assert_eq!(format_duration(Duration::seconds(1)), "1 second");
        assert_eq!(format_duration(Duration::seconds(30)), "30 seconds");
        assert_eq!(format_duration(Duration::seconds(59)), "59 seconds");
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_duration(Duration::seconds(60)), "1 minute");
        assert_eq!(format_duration(Duration::seconds(90)), "1 minute");
        assert_eq!(format_duration(Duration::seconds(120)), "2 minutes");
        assert_eq!(format_duration(Duration::seconds(3599)), "59 minutes");
    }

    #[test]
    fn test_format_duration_hours() {
        assert_eq!(format_duration(Duration::seconds(3600)), "1 hour");
        assert_eq!(format_duration(Duration::seconds(3700)), "1 hour");
        assert_eq!(format_duration(Duration::seconds(7200)), "2 hours");
        assert_eq!(format_duration(Duration::seconds(86399)), "23 hours");
    }

    #[test]
    fn test_format_duration_days() {
        assert_eq!(format_duration(Duration::seconds(86400)), "1 day");
        assert_eq!(format_duration(Duration::seconds(90000)), "1 day");
        assert_eq!(format_duration(Duration::seconds(172800)), "2 days");
        assert_eq!(format_duration(Duration::seconds(604800)), "7 days");
    }
}
