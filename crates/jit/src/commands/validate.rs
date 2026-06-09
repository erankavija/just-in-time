//! Validation and status operations

use super::*;
use crate::domain::SHORT_ID_LENGTH;
use crate::type_hierarchy::{
    detect_validation_issues, generate_fixes, ValidationFix, ValidationIssue,
};
use anyhow::Context;

impl<S: IssueStore> CommandExecutor<S> {
    /// Validate with optional fix mode.
    ///
    /// # Arguments
    ///
    /// * `fix` - If true, attempt to fix validation issues
    /// * `dry_run` - If true, show what would be fixed without applying changes
    ///
    /// # Returns
    ///
    /// Returns Ok with (count of fixes applied, messages), or Err if validation fails
    pub fn validate_with_fix(&mut self, fix: bool, dry_run: bool) -> Result<(usize, Vec<String>)> {
        // First run standard validation
        let validation_result = self.validate_silent();

        if validation_result.is_ok() && !fix {
            return Ok((0, vec![]));
        }

        // If fix mode is enabled, detect and fix issues
        if fix {
            let mut total_fixes = 0;
            let mut all_messages = Vec::new();

            // Fix type hierarchy issues
            let (hierarchy_fixes, mut messages) = self.detect_and_fix_hierarchy_issues(dry_run)?;
            total_fixes += hierarchy_fixes;
            all_messages.append(&mut messages);

            // Fix transitive reduction violations (both dry-run and actual fix)
            let (reduction_fixes, mut messages) = self.fix_all_transitive_reductions(dry_run)?;
            total_fixes += reduction_fixes;
            all_messages.append(&mut messages);

            // Fix pending state transitions
            let (transition_fixes, mut messages) = self.check_pending_transitions(dry_run)?;
            total_fixes += transition_fixes;
            all_messages.append(&mut messages);

            // Add summary message
            if dry_run {
                all_messages.push(format!(
                    "\nDry run complete. {} fixes would be applied.",
                    total_fixes
                ));
            } else if total_fixes > 0 {
                all_messages.push(format!("\n✓ Applied {} fixes", total_fixes));
                // Re-run validation to verify fixes worked
                self.validate_silent()?;
                all_messages.push("✓ Repository is now valid".to_string());
            } else {
                all_messages.push("\n✓ No fixes needed".to_string());
            }

            return Ok((total_fixes, all_messages));
        }

        // Not in fix mode, just propagate the validation error
        validation_result?;
        Ok((0, vec![]))
    }

    fn detect_and_fix_hierarchy_issues(&mut self, dry_run: bool) -> Result<(usize, Vec<String>)> {
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
            return Ok((0, vec![]));
        }

        // Generate fixes
        let fixes = generate_fixes(&all_validation_issues);

        if fixes.is_empty() {
            // We found issues but can't auto-fix them
            let mut messages = vec![format!(
                "Found {} validation issues but no automatic fixes available:",
                all_validation_issues.len()
            )];
            for issue in &all_validation_issues {
                match issue {
                    ValidationIssue::UnknownType {
                        issue_id,
                        unknown_type,
                        ..
                    } => {
                        messages.push(format!(
                            "  • Issue {} has unknown type '{}'",
                            issue_id, unknown_type
                        ));
                    }
                    ValidationIssue::InvalidMembershipReference {
                        issue_id,
                        label,
                        reason,
                        ..
                    } => {
                        messages.push(format!(
                            "  • Issue {} has invalid membership label '{}': {}",
                            issue_id, label, reason
                        ));
                    }
                }
            }
            return Ok((0, messages));
        }

        // Apply or preview fixes
        let mut fixes_applied = 0;
        let mut messages = Vec::new();

        for fix in &fixes {
            match fix {
                ValidationFix::ReplaceType {
                    issue_id,
                    old_type,
                    new_type,
                } => {
                    if dry_run {
                        messages.push(format!(
                            "Would replace type '{}' with '{}' for issue {}",
                            old_type, new_type, issue_id
                        ));
                    } else {
                        self.apply_type_fix(issue_id, old_type, new_type)?;
                        messages.push(format!(
                            "✓ Replaced type '{}' with '{}' for issue {}",
                            old_type, new_type, issue_id
                        ));
                    }
                    fixes_applied += 1;
                }
            }
        }

        Ok((fixes_applied, messages))
    }

    fn apply_type_fix(&mut self, issue_id: &str, old_type: &str, new_type: &str) -> Result<()> {
        let mut issue = self.storage.load_issue(issue_id)?;

        // Replace the type label
        let old_label = format!("type:{}", old_type);
        let new_label = format!("type:{}", new_type);

        issue.labels.retain(|l| l != &old_label);
        issue.labels.push(new_label);

        self.storage.save_issue(issue)?;
        Ok(())
    }

    // Note: apply_dependency_reversal is removed - we don't reverse dependencies
    // Type hierarchy is orthogonal to DAG structure

    pub fn validate_silent(&self) -> Result<()> {
        // Repository-integrity checks (broken deps, gates, docs, DAG, isolated
        // nodes, transitive reduction, claims index). Label/type-label/namespace
        // checks are NO LONGER here: they are default rules evaluated below.
        self.validate_integrity_silent()?;

        // Local rules (built-in defaults + user rules) across every issue. The
        // former hard-coded label/type/namespace checks live here now: an
        // `error`-severity finding (e.g. a value outside a namespace enum, a bad
        // pattern, a missing required label) fails validation — matching the old
        // `validate_labels`/`validate_type_hierarchy` hard-reject behavior — while
        // `warn` findings never fail.
        let issues = self.storage.list_issues()?;
        if let Some(message) = self.local_rules_error_message(&issues)? {
            return Err(anyhow!(message));
        }

        // Cross-issue graph rules. An `error`-severity violation (including a
        // `config-error`, since the rule could not be applied) fails validation;
        // `warn`/`off` findings never fail here.
        let graph_findings = self.evaluate_graph_rules(&issues)?;
        if let Some(message) = graph_findings_error_message(&graph_findings) {
            return Err(anyhow!(message));
        }

        Ok(())
    }

    /// Evaluate the EFFECTIVE local rules for every issue and, if any produces an
    /// `error`-severity finding, return a single combined message; otherwise
    /// `None`. This is the migrated replacement for the former hard-coded
    /// `validate_labels`/`validate_type_hierarchy` whole-repo checks: those always
    /// hard-rejected on a violation, so any `error` finding here fails validation.
    /// `warn` findings are never fatal.
    fn local_rules_error_message(&self, issues: &[Issue]) -> Result<Option<String>> {
        use crate::validation::rules::Severity;

        let ruleset = self.effective_rules()?;
        let repo_format = self.repo_content_format()?;
        let mut errors: Vec<(String, String)> = Vec::new();
        for issue in issues {
            let evaluation = crate::validation::evaluate_local(issue, ruleset, repo_format)
                .map_err(|e| anyhow!("Local rule evaluation failed: {}", e))?;
            for finding in evaluation.findings() {
                if finding.severity == Severity::Error {
                    errors.push((
                        issue.id.clone(),
                        format!("[{}] {}", finding.rule, finding.message),
                    ));
                }
            }
        }

        if errors.is_empty() {
            return Ok(None);
        }
        let body = errors
            .iter()
            .map(|(id, msg)| format!("  issue {}: {}", &id[..8.min(id.len())], msg))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Some(format!(
            "Validation failed with {} rule error(s):\n{}",
            errors.len(),
            body
        )))
    }

    /// Run the repository-integrity checks ONLY, without evaluating declarative
    /// graph rules.
    ///
    /// This is the structural half of [`CommandExecutor::validate_silent`]:
    /// broken dependency references, invalid gate references, document
    /// references, DAG acyclicity, isolated nodes, transitive reduction, and
    /// claims-index consistency. Label validity and type-hierarchy checks are NO
    /// LONGER here — they were migrated to default rules and are evaluated by the
    /// local/graph rule engine in `validate_silent` (see the NOTE in that
    /// method). It deliberately
    /// excludes the `Scope::Graph` declarative rules so a caller can render those
    /// as structured findings (e.g. whole-repo `jit validate --json`) and decide
    /// the exit status AFTER output, rather than aborting before the rule report
    /// is built.
    ///
    /// Returns `Ok(())` when the repository is structurally sound, or an `Err`
    /// describing the first integrity violation found.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// // Repo-integrity passes (no broken deps, valid DAG, etc.).
    /// executor.validate_integrity_silent().unwrap();
    /// ```
    pub fn validate_integrity_silent(&self) -> Result<()> {
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

        // NOTE: label format, namespace registry, namespace value/pattern/
        // unique/required, type-label requirement, and unknown-type detection are
        // NO LONGER checked here. They are now default rules (see
        // `validation::defaults`) evaluated by `validate_silent` via
        // `local_rules_error_message` (a0f0f342 migration).

        // Validate document references (git integration)
        self.validate_document_references(&issues)?;

        // Validate DAG (no cycles)
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);
        graph.validate_dag()?;

        // Validate no isolated nodes (nodes outside the main DAG)
        // Exception: A single issue in the repository is not considered isolated
        if issues.len() > 1 {
            let isolated = graph.get_isolated_nodes();
            if !isolated.is_empty() {
                let isolated_ids: Vec<String> = isolated
                    .iter()
                    .map(|i| format!("'{}' ({})", &i.id[..8.min(i.id.len())], i.title))
                    .collect();
                return Err(anyhow!(
                    "Found {} isolated issue(s) not connected to the dependency graph:\n  {}\n\
                     Isolated issues have no dependencies and are not dependencies of any other issue.\n\
                     Either add dependencies with 'jit dep add' or delete these issues.",
                    isolated.len(),
                    isolated_ids.join("\n  ")
                ));
            }
        }

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

    /// Evaluate every `Scope::Graph` rule from `.jit/rules.toml` over the supplied
    /// issue set, returning one
    /// [`GraphFinding`](crate::validation::graph::GraphFinding) per violation
    /// (including `config-error` findings for malformed rules). Each finding
    /// carries the issue it pertains to (or `None` for a config-error), so
    /// per-issue reporting can attribute findings exactly rather than by matching
    /// substrings in the message.
    ///
    /// The ruleset is loaded via [`CommandExecutor::rules`](crate::commands::CommandExecutor::rules);
    /// a genuine `rules.toml` parse/load failure is surfaced as an `Err` rather
    /// than silently disabling enforcement. A missing `rules.toml` yields no
    /// findings. This method performs no filesystem writes; it reads the cached
    /// ruleset and the issues passed in.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::{IssueStore, JsonFileStorage};
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let issues = executor.storage().list_issues().unwrap();
    /// let findings = executor.evaluate_graph_rules(&issues).unwrap();
    /// println!("{} graph finding(s)", findings.len());
    /// ```
    pub fn evaluate_graph_rules(
        &self,
        issues: &[Issue],
    ) -> Result<Vec<crate::validation::graph::GraphFinding>> {
        use crate::validation::rules::Scope;

        // Surface a misconfigured rules.toml instead of swallowing it.
        let ruleset = self.effective_rules()?;

        let graph_rules: Vec<&crate::validation::rules::Rule> = ruleset
            .rules
            .iter()
            .filter(|rule| rule.scope == Scope::Graph)
            .collect();

        // The repo HierarchyConfig is injected into `type-hierarchy` rules at
        // evaluation time (D1); it is no longer stored in the parsed rule. Build
        // it from the same namespace registry the default rules derive from.
        let namespaces = self.cached_namespaces().map_err(|e| anyhow!("{e}"))?;
        let hierarchy = crate::validation::defaults::hierarchy_config(namespaces);
        let repo_format = self.repo_content_format()?;

        Ok(crate::validation::graph::evaluate_graph(
            &graph_rules,
            issues,
            &hierarchy,
            repo_format,
        ))
    }

    /// Run the declarative rule set (`.jit/rules.toml`) as a per-issue or
    /// whole-repo report.
    ///
    /// When `id` is `Some`, the issue is resolved (partial ids accepted) and its
    /// matching local rules are evaluated, then graph rules are evaluated across
    /// the whole store and filtered to those that pertain to this issue using
    /// structured attribution (each graph finding carries its issue id), plus any
    /// `config-error` finding for a graph rule whose selector applies to this
    /// issue — so a malformed graph rule is reported, never silently passed. When
    /// `id` is `None`, local rules run for every issue and graph rules run across
    /// the store, with every graph finding reported. The result is a pure
    /// [`RuleReport`](crate::validation::report::RuleReport); rendering and exit
    /// codes are the caller's concern.
    ///
    /// # Errors
    ///
    /// Returns an error if `.jit/rules.toml` is malformed, the issue id cannot be
    /// resolved, or a matching local rule's schema fails to compile (a
    /// misconfigured rule never silently disables enforcement).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// // Whole-repo rule report.
    /// let report = executor.run_rules(None).unwrap();
    /// println!("{} error(s)", report.error_count());
    /// ```
    pub fn run_rules(&self, id: Option<&str>) -> Result<crate::validation::report::RuleReport> {
        use crate::validation::report::{ReportedFinding, RuleReport};

        let ruleset = self.effective_rules()?;
        let repo_format = self.repo_content_format()?;
        let issues = self.storage.list_issues()?;

        let mut findings: Vec<ReportedFinding> = Vec::new();

        match id {
            Some(partial) => {
                // Resolve the target issue (partial ids accepted via storage).
                let issue = self.storage.load_issue(partial)?;

                // Local rules for this issue only.
                let evaluation = crate::validation::evaluate_local(&issue, ruleset, repo_format)
                    .map_err(|e| anyhow!("Local rule evaluation failed: {}", e))?;
                findings.extend(
                    evaluation
                        .findings()
                        .into_iter()
                        .map(|f| ReportedFinding::new(Some(issue.id.clone()), f)),
                );

                // Graph rules across the store. Keep, by EXACT structural
                // attribution (not substring matching): findings attributed to
                // this issue, plus any config-error for a graph rule whose
                // selector applies to this issue (a malformed graph rule must be
                // reported here, never silently dropped).
                let applicable_rules: std::collections::HashSet<String> = ruleset
                    .matching_rules(&issue)
                    .into_iter()
                    .map(|r| r.name.clone())
                    .collect();
                let graph_findings = self.evaluate_graph_rules(&issues)?;
                findings.extend(graph_findings.iter().filter_map(|gf| {
                    let pertains = gf.issue_id.as_deref() == Some(issue.id.as_str())
                        || (gf.is_config_error() && applicable_rules.contains(&gf.finding.rule));
                    pertains.then(|| ReportedFinding::new(Some(issue.id.clone()), &gf.finding))
                }));
            }
            None => {
                // Local rules for every issue.
                for issue in &issues {
                    let evaluation = crate::validation::evaluate_local(issue, ruleset, repo_format)
                        .map_err(|e| anyhow!("Local rule evaluation failed: {}", e))?;
                    findings.extend(
                        evaluation
                            .findings()
                            .into_iter()
                            .map(|f| ReportedFinding::new(Some(issue.id.clone()), f)),
                    );
                }

                // Graph rules across the store; every finding is reported,
                // carrying its structured issue attribution (None for config
                // errors).
                let graph_findings = self.evaluate_graph_rules(&issues)?;
                findings.extend(
                    graph_findings
                        .iter()
                        .map(|gf| ReportedFinding::new(gf.issue_id.clone(), &gf.finding)),
                );
            }
        }

        Ok(RuleReport { findings })
    }

    /// Build the `--explain` report for one issue: every rule whose selector
    /// matches the issue, paired with whether it passed and its messages.
    ///
    /// Local rules are evaluated against the issue; graph rules are evaluated
    /// across the whole store and a graph rule is considered to apply to this
    /// issue when its selector matches it. Graph findings are attributed to this
    /// issue by EXACT structural attribution (each finding carries its issue id),
    /// and any `config-error` finding for an applicable graph rule is also
    /// surfaced — so a malformed graph rule is reported as a FAILED outcome,
    /// never shown as passing. Each
    /// [`RuleOutcome`](crate::validation::report::RuleOutcome) records the matched
    /// selector (rendered), scope, severity, pass/fail, and any messages.
    ///
    /// # Errors
    ///
    /// Returns an error if `.jit/rules.toml` is malformed, the issue id cannot be
    /// resolved, or a matching local rule's schema fails to compile.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let report = executor.explain_rules("abc12345").unwrap();
    /// println!("{} matching rule(s)", report.outcomes.len());
    /// ```
    pub fn explain_rules(&self, id: &str) -> Result<crate::validation::report::ExplainReport> {
        use crate::validation::report::{ExplainReport, RuleOutcome};
        use crate::validation::rules::Scope;

        let ruleset = self.effective_rules()?;
        let repo_format = self.repo_content_format()?;
        let issue = self.storage.load_issue(id)?;
        let issues = self.storage.list_issues()?;

        // Local findings for this issue, grouped by rule name.
        let local_eval = crate::validation::evaluate_local(&issue, ruleset, repo_format)
            .map_err(|e| anyhow!("Local rule evaluation failed: {}", e))?;
        let local_messages = group_messages(
            local_eval
                .findings()
                .into_iter()
                .map(|f| (f.rule.clone(), f.message.clone())),
        );

        // Graph findings pertaining to this issue, grouped by rule name, using
        // EXACT structural attribution: a finding attributed to this issue, or a
        // config-error (which carries no issue id but must be surfaced so a
        // malformed graph rule fails rather than silently passes). The
        // selector-based "applies to this issue" decision is made per-rule below;
        // including all config-errors here is safe because the outcome list is
        // built only from rules whose selector matches the issue.
        let graph_findings = self.evaluate_graph_rules(&issues)?;
        let graph_messages = group_messages(graph_findings.iter().filter_map(|gf| {
            let pertains =
                gf.issue_id.as_deref() == Some(issue.id.as_str()) || gf.is_config_error();
            pertains.then(|| (gf.finding.rule.clone(), gf.finding.message.clone()))
        }));

        // Every rule whose selector matches the issue becomes one outcome.
        let outcomes: Vec<RuleOutcome> = ruleset
            .matching_rules(&issue)
            .into_iter()
            .map(|rule| {
                let scope = match rule.scope {
                    Scope::Local => "local",
                    Scope::Graph => "graph",
                };
                let messages = match rule.scope {
                    Scope::Local => local_messages.get(&rule.name).cloned().unwrap_or_default(),
                    Scope::Graph => graph_messages.get(&rule.name).cloned().unwrap_or_default(),
                };
                RuleOutcome {
                    rule: rule.name.clone(),
                    scope: scope.to_string(),
                    severity: rule.severity.token().to_string(),
                    selector: render_selector(&rule.when),
                    passed: messages.is_empty(),
                    messages,
                }
            })
            .collect();

        Ok(ExplainReport {
            issue_id: issue.id,
            outcomes,
        })
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
                        // Repository has commits - validate against HEAD, falling back to
                        // working tree so that newly added (not yet committed) files pass.
                        let in_git = self
                            .check_file_exists_in_git(&repo, &doc.path, "HEAD")
                            .is_ok();
                        let in_working_tree = std::path::Path::new(&doc.path).exists();
                        if !in_git && !in_working_tree {
                            return Err(anyhow!(
                                "Invalid document reference in issue '{}': file '{}' not found at HEAD or in working tree",
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

    pub fn get_status(&self) -> Result<StatusSummary> {
        let issues = self.storage.list_issues()?;
        let resolved = crate::domain::queries::build_issue_map(&issues);

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
                        issue.short_id(),
                        dep_id.chars().take(SHORT_ID_LENGTH).collect::<String>(),
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
            let reduced_set: std::collections::HashSet<String> = reduced.iter().cloned().collect();
            let removed_deps: Vec<String> = issue
                .dependencies
                .iter()
                .filter(|d| !reduced_set.contains(*d))
                .cloned()
                .collect();

            issue.dependencies = reduced.into_iter().collect();
            self.storage.save_issue(issue.clone())?;

            let event = Event::new_dependency_reduced(
                issue.id.clone(),
                current_len,
                reduced_len,
                removed_deps,
            );
            self.storage.append_event(&event)?;
        }

        Ok(redundant_count)
    }

    /// Fix transitive reduction violations for all issues.
    ///
    /// Returns count of redundant edges fixed (or that would be fixed if dry_run).
    fn fix_all_transitive_reductions(&mut self, dry_run: bool) -> Result<(usize, Vec<String>)> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        let mut total_redundancies = 0;
        let mut messages = Vec::new();

        for issue in &issues {
            if issue.dependencies.is_empty() {
                continue;
            }

            let fixed_count = self.fix_transitive_reduction(&graph, &issue.id, dry_run)?;
            if fixed_count > 0 {
                total_redundancies += fixed_count;
                if !dry_run {
                    messages.push(format!(
                        "Fixed {} redundant {} in issue {}",
                        fixed_count,
                        if fixed_count == 1 {
                            "dependency"
                        } else {
                            "dependencies"
                        },
                        &issue.id[..8.min(issue.id.len())]
                    ));
                }
            }
        }

        Ok((total_redundancies, messages))
    }

    /// Check for and fix pending state transitions.
    ///
    /// After worktree merges, issues in backlog state may have all dependencies
    /// completed but never auto-transition to ready. This method detects and fixes
    /// those pending transitions.
    ///
    /// Uses multiple passes to handle cascading transitions (e.g., when tasks complete,
    /// stories become ready, then epics that depend on those stories also become ready).
    ///
    /// # Arguments
    ///
    /// * `dry_run` - If true, report what would be fixed without applying changes
    ///
    /// # Returns
    ///
    /// Count of issues transitioned and informational messages
    fn check_pending_transitions(&mut self, dry_run: bool) -> Result<(usize, Vec<String>)> {
        let mut total_fixed = 0;
        let mut messages = Vec::new();
        let max_passes = 10; // Safety limit to prevent infinite loops

        // Keep checking until no more transitions found (cascading transitions)
        // In dry-run mode, only do one pass since we don't actually change state
        let num_passes = if dry_run { 1 } else { max_passes };

        for _pass in 0..num_passes {
            let issues = self.storage.list_issues()?;
            let resolved = crate::domain::queries::build_issue_map(&issues);

            // Find backlog issues that should transition to ready
            let backlog_issues: Vec<_> = issues
                .iter()
                .filter(|i| i.state == State::Backlog)
                .collect();

            let mut pass_fixed = 0;
            for issue in backlog_issues {
                if issue.should_auto_transition_to_ready(&resolved) {
                    messages.push(format!(
                        "  → Transitioning {} to ready (dependencies complete)",
                        &issue.id[..8.min(issue.id.len())]
                    ));

                    if !dry_run {
                        self.auto_transition_to_ready(&issue.id)?;
                    }
                    pass_fixed += 1;
                }
            }

            total_fixed += pass_fixed;

            // If no fixes this pass, we're done
            if pass_fixed == 0 {
                break;
            }
        }

        Ok((total_fixed, messages))
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

/// Build a human-readable error message from any `error`-severity graph-rule
/// findings, or `None` if none fail validation.
///
/// `warn`/`off` findings are intentionally excluded: only `Severity::Error`
/// findings (which include `config-error` findings, attributed to a rule with
/// error severity) make `jit validate` fail.
fn graph_findings_error_message(
    findings: &[crate::validation::graph::GraphFinding],
) -> Option<String> {
    use crate::validation::rules::Severity;

    let errors: Vec<&crate::validation::graph::GraphFinding> = findings
        .iter()
        .filter(|f| f.finding.severity == Severity::Error)
        .collect();

    if errors.is_empty() {
        return None;
    }

    let body = errors
        .iter()
        .map(|f| format!("  [{}] {}", f.finding.rule, f.finding.message))
        .collect::<Vec<_>>()
        .join("\n");

    Some(format!(
        "Graph rule validation failed with {} error(s):\n{}",
        errors.len(),
        body
    ))
}

/// Group `(rule_name, message)` pairs into a map of rule name to its messages,
/// preserving first-seen order within each rule.
fn group_messages(
    pairs: impl IntoIterator<Item = (String, String)>,
) -> std::collections::HashMap<String, Vec<String>> {
    pairs.into_iter().fold(
        std::collections::HashMap::new(),
        |mut acc, (rule, message)| {
            acc.entry(rule).or_default().push(message);
            acc
        },
    )
}

/// Render a rule selector as a compact, human-readable string for `--explain`.
///
/// An empty selector (matches everything) renders as `"*"`; otherwise the
/// present dimensions are joined with `", "` (e.g. `"type=epic, state=ready"`).
fn render_selector(selector: &crate::validation::rules::Selector) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = &selector.type_ {
        parts.push(format!("type={t}"));
    }
    if let Some(l) = &selector.label {
        parts.push(format!("label={l}"));
    }
    if let Some(s) = &selector.state {
        parts.push(format!("state={}", s.tokens().join("|")));
    }
    if let Some(d) = &selector.has_doc_type {
        parts.push(format!("has_doc_type={d}"));
    }
    if parts.is_empty() {
        "*".to_string()
    } else {
        parts.join(", ")
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
    fn test_render_selector_state_single() {
        use crate::validation::rules::{Selector, StatePredicate};
        let sel = Selector {
            type_: Some("epic".to_string()),
            state: Some(StatePredicate::Single("in_progress".to_string())),
            ..Default::default()
        };
        assert_eq!(render_selector(&sel), "type=epic, state=in_progress");
    }

    #[test]
    fn test_render_selector_state_list_joins_with_pipe() {
        use crate::validation::rules::{Selector, StatePredicate};
        let sel = Selector {
            state: Some(StatePredicate::List(vec![
                "ready".to_string(),
                "in_progress".to_string(),
            ])),
            ..Default::default()
        };
        assert_eq!(render_selector(&sel), "state=ready|in_progress");
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
