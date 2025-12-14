//! Validation and status operations

use super::*;
use crate::type_hierarchy::{
    detect_validation_issues, generate_fixes, ValidationFix, ValidationIssue,
};

impl<S: IssueStore> CommandExecutor<S> {
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

        // If fix mode is enabled, detect and fix type hierarchy issues
        if fix {
            let fixes_applied = self.detect_and_fix_hierarchy_issues(dry_run, quiet)?;

            if !quiet {
                if dry_run {
                    println!(
                        "\nDry run complete. {} fixes would be applied.",
                        fixes_applied
                    );
                } else if fixes_applied > 0 {
                    println!("\n✓ Applied {} fixes", fixes_applied);

                    // Re-run validation to verify fixes worked
                    self.validate_silent()?;
                    println!("✓ Repository is now valid");
                } else {
                    println!("\n✓ No fixes needed");
                }
            }

            return Ok(fixes_applied);
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
            // Build dependency map for this issue
            let deps: Vec<(String, Vec<String>)> = issue
                .dependencies
                .iter()
                .filter_map(|dep_id| {
                    self.storage
                        .load_issue(dep_id)
                        .ok()
                        .map(|dep| (dep.id.clone(), dep.labels.clone()))
                })
                .collect();

            let validation_issues =
                detect_validation_issues(&config, &issue.id, &issue.labels, &deps);
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
                        ValidationIssue::InvalidHierarchyDep {
                            from_issue_id,
                            to_issue_id,
                            from_type,
                            to_type,
                            ..
                        } => {
                            println!(
                                "  • Issue {} (type:{}) depends on {} (type:{})",
                                from_issue_id, from_type, to_issue_id, to_type
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
                ValidationFix::ReverseDependency {
                    from_issue_id,
                    to_issue_id,
                } => {
                    if !quiet {
                        if dry_run {
                            println!(
                                "Would reverse dependency: {} -> {} becomes {} -> {}",
                                from_issue_id, to_issue_id, to_issue_id, from_issue_id
                            );
                        } else {
                            self.apply_dependency_reversal(from_issue_id, to_issue_id)?;
                            println!(
                                "✓ Reversed dependency: {} now depends on {}",
                                to_issue_id, from_issue_id
                            );
                        }
                    } else if !dry_run {
                        self.apply_dependency_reversal(from_issue_id, to_issue_id)?;
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

    fn apply_dependency_reversal(&mut self, from_id: &str, to_id: &str) -> Result<()> {
        // Load both issues
        let mut from_issue = self.storage.load_issue(from_id)?;
        let mut to_issue = self.storage.load_issue(to_id)?;

        // Remove from_id -> to_id dependency
        from_issue.dependencies.retain(|dep| dep != to_id);

        // Add to_id -> from_id dependency (if not already present)
        if !to_issue.dependencies.contains(&from_id.to_string()) {
            to_issue.dependencies.push(from_id.to_string());
        }

        // Save both issues atomically (order matters for locking)
        let (first_id, _second_id) = if from_id < to_id {
            (from_id, to_id)
        } else {
            (to_id, from_id)
        };

        if first_id == from_id {
            self.storage.save_issue(&from_issue)?;
            self.storage.save_issue(&to_issue)?;
        } else {
            self.storage.save_issue(&to_issue)?;
            self.storage.save_issue(&from_issue)?;
        }

        Ok(())
    }

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

        Ok(())
    }

    fn validate_labels(&self, issues: &[Issue]) -> Result<()> {
        let namespaces = self.storage.load_label_namespaces()?;

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
            // Build dependency map for this issue
            let deps: Vec<(String, Vec<String>)> = issue
                .dependencies
                .iter()
                .filter_map(|dep_id| {
                    self.storage
                        .load_issue(dep_id)
                        .ok()
                        .map(|dep| (dep.id.clone(), dep.labels.clone()))
                })
                .collect();

            let validation_issues =
                detect_validation_issues(&config, &issue.id, &issue.labels, &deps);

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
                    ValidationIssue::InvalidHierarchyDep {
                        from_issue_id,
                        from_type,
                        to_issue_id,
                        to_type,
                        from_level,
                        to_level,
                    } => {
                        return Err(anyhow!(
                            "Type hierarchy violation: Issue '{}' (type:{}, level {}) depends on '{}' (type:{}, level {}). \
                             Higher-level types cannot depend on lower-level types.",
                            from_issue_id,
                            from_type,
                            from_level,
                            to_issue_id,
                            to_type,
                            to_level
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

        for issue in issues {
            for doc in &issue.documents {
                // Validate commit hash if specified
                if let Some(ref commit_hash) = doc.commit {
                    if repo.find_commit(git2::Oid::from_str(commit_hash)?).is_err() {
                        return Err(anyhow!(
                            "Invalid document reference in issue '{}': commit '{}' not found for '{}'",
                            issue.id,
                            commit_hash,
                            doc.path
                        ));
                    }
                }

                // Validate file exists (at HEAD if no commit specified)
                let reference = if let Some(ref commit_hash) = doc.commit {
                    commit_hash.as_str()
                } else {
                    "HEAD"
                };

                if self
                    .check_file_exists_in_git(&repo, &doc.path, reference)
                    .is_err()
                {
                    return Err(anyhow!(
                        "Invalid document reference in issue '{}': file '{}' not found at {}",
                        issue.id,
                        doc.path,
                        reference
                    ));
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
        let blocked = issues.iter().filter(|i| i.is_blocked(&resolved)).count();

        Ok(StatusSummary {
            open: backlog, // Keep 'open' field name for backward compatibility
            ready,
            in_progress,
            done,
            blocked,
            gated,
            total: issues.len(),
        })
    }
}
