//! Gate checking and execution operations

use super::*;
use crate::declarations::{GateMode, GateStage, REVIEW_PLACEHOLDER_WARNING};
use crate::domain::{
    GateContext, GateFinding, GateFindings, GateRunResult, GateRunStatus, GateStatus,
};
use crate::errors::TransitionBlockedError;
use crate::gate_execution;
use crate::output::IssueShowResponse;
use std::collections::HashMap;

/// Maximum prompt file size in bytes (~1000 lines of 80 chars).
const MAX_PROMPT_FILE_SIZE: u64 = 100_000;

pub(super) struct PrecheckExecution {
    pub(super) runs: Vec<GateRunResult>,
    pub(super) error: Option<anyhow::Error>,
}

struct CapturedGateExecution<'a> {
    image: &'a crate::repository_state::RepositoryImage,
    issue: &'a Issue,
    issues: &'a [Issue],
    gate_key: &'a str,
    gate: &'a crate::declarations::GateDefinition,
    runs: &'a [GateRunResult],
    prompt: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GateEvaluationEvidence {
    gate: crate::declarations::GateDefinition,
    inputs: Vec<(
        crate::repository_state::VirtualPath,
        Option<crate::repository_state::EntryIdentity>,
    )>,
    pinned: std::collections::BTreeMap<
        (String, String),
        crate::repository_state::PinnedDocumentEvidence,
    >,
    linked: std::collections::BTreeMap<
        crate::repository_state::VirtualPath,
        crate::repository_state::LinkedWorktreeEvidence,
    >,
    validation_view: Option<super::PrecheckValidationView>,
}

struct CapturedGateEvaluation {
    evidence: GateEvaluationEvidence,
    prompt: Option<String>,
}

struct CachedGateEvaluation {
    evidence: GateEvaluationEvidence,
    result: GateRunResult,
}

#[derive(Default)]
struct CapturedGateRuns {
    results: Vec<GateRunResult>,
    inputs: Vec<crate::repository_state::VirtualPath>,
}

fn captured_repository_index(
    image: &crate::repository_state::RepositoryImage,
) -> Result<crate::repository_state::RepositoryIndex> {
    use crate::repository_state::{RepositoryEntry, VirtualPath};

    let path = VirtualPath::data("index.json")?;
    match image.entry(&path)? {
        RepositoryEntry::File { bytes, .. } => crate::storage::json::parse_repository_index(bytes)
            .context("failed to parse captured issue index"),
        RepositoryEntry::Absent => Err(anyhow!("captured repository has no .jit/index.json")),
        _ => Err(anyhow!("captured .jit/index.json is not a regular file")),
    }
}

fn captured_issue_record(
    image: &crate::repository_state::RepositoryImage,
    issue_id: &str,
) -> Result<Option<Issue>> {
    use crate::repository_state::{RepositoryEntry, VirtualPath};

    let path = VirtualPath::data(format!("issues/{issue_id}.json"))?;
    match image.entry(&path)? {
        RepositoryEntry::File { bytes, .. } => {
            let issue: Issue = serde_json::from_slice(bytes)
                .with_context(|| format!("failed to parse captured issue at {path:?}"))?;
            if issue.id != issue_id {
                anyhow::bail!(
                    "captured issue {issue_id} contains mismatched embedded id {}",
                    issue.id
                );
            }
            Ok(Some(issue))
        }
        RepositoryEntry::Absent => Ok(None),
        _ => Err(anyhow!("captured issue at {path:?} is not a regular file")),
    }
}

fn captured_bound_issue(
    image: &crate::repository_state::RepositoryImage,
    issue_id: &str,
) -> Result<Issue> {
    let index = captured_repository_index(image)?;
    if !index.all_ids.iter().any(|id| id == issue_id) {
        return Err(crate::storage::IssueNotFoundError::new(issue_id).into());
    }
    captured_issue_record(image, issue_id)?
        .ok_or_else(|| crate::storage::IssueNotFoundError::new(issue_id).into())
}

fn captured_gate_registry(
    image: &crate::repository_state::RepositoryImage,
) -> Result<crate::declarations::GateRegistry> {
    use crate::repository_state::{RepositoryEntry, VirtualPath};

    let path = VirtualPath::data("gates.toml")?;
    match image.entry(&path)? {
        RepositoryEntry::File { bytes, .. } => crate::declarations::parse_gate_registry(bytes)
            .context("failed to parse captured gate registry"),
        RepositoryEntry::Absent => Ok(crate::declarations::GateRegistry::default()),
        _ => Err(anyhow!("captured .jit/gates.toml is not a regular file")),
    }
}

fn captured_direct_dependency_issues(
    image: &crate::repository_state::RepositoryImage,
    issue: &Issue,
) -> Result<Vec<Issue>> {
    Ok(issue
        .dependencies
        .iter()
        .map(|id| captured_issue_record(image, id))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect())
}

fn checker_needs_validation_image(checker: &crate::declarations::GateChecker) -> bool {
    matches!(
        checker,
        crate::declarations::GateChecker::RepositoryValidation
            | crate::declarations::GateChecker::IssueValidation
            | crate::declarations::GateChecker::LabelTargetValidation { .. }
    )
}

fn required_automated_gate<'a>(
    issue: &Issue,
    registry: &'a crate::declarations::GateRegistry,
    gate_key: &str,
) -> Result<&'a crate::declarations::GateDefinition> {
    if !issue.gates_required.iter().any(|key| key == gate_key) {
        anyhow::bail!(
            "Gate '{}' is not required for issue '{}'",
            gate_key,
            issue.id
        );
    }
    let gate = registry
        .gates
        .get(gate_key)
        .ok_or_else(|| crate::storage::GateNotFoundError::single(gate_key))?;
    if gate.mode != GateMode::Auto {
        anyhow::bail!(
            "Gate '{}' is manual and cannot be automatically checked",
            gate_key
        );
    }
    if gate.checker.is_none() {
        anyhow::bail!("Gate '{}' has no checker configured", gate_key);
    }
    Ok(gate)
}

pub(super) fn gate_state_from_run(
    result: &GateRunResult,
) -> Result<(GateState, Option<crate::domain::Assignee>)> {
    let by = result
        .by
        .as_deref()
        .map(str::parse::<crate::domain::Assignee>)
        .transpose()?;
    let status = match result.status {
        GateRunStatus::Passed => GateStatus::Passed,
        GateRunStatus::Failed | GateRunStatus::Error => GateStatus::Failed,
        _ => GateStatus::Pending,
    };
    Ok((
        GateState {
            status,
            updated_by: by.clone(),
            updated_at: chrono::DateTime::default(),
        },
        by,
    ))
}

/// Select and sanitize the latest run for one gate before it enters checker context.
///
/// Stored results are never mutated. Structured findings replace verbose stdout;
/// legacy unstructured runs retain stdout as their compatibility signal. Stderr is
/// always diagnostic noise and is removed from the context copy.
fn compact_run_history_for_context(runs: &[GateRunResult], gate_key: &str) -> Vec<GateRunResult> {
    runs.iter()
        .filter(|run| run.gate_key == gate_key)
        .max_by_key(|run| run.started_at)
        .cloned()
        .map(|mut run| {
            run.stderr.clear();
            if run.findings.is_some() {
                run.stdout.clear();
            }
            run
        })
        .into_iter()
        .collect()
}

fn captured_gate_runs(
    image: &crate::repository_state::RepositoryImage,
    issue_id: &str,
) -> Result<CapturedGateRuns> {
    use crate::repository_state::RepositoryEntry;

    let paths = crate::repository_state::captured_gate_run_result_paths(image)?
        .ok_or_else(|| anyhow!("captured gate-run root has no complete listing"))?;
    let mut captured = paths
        .into_iter()
        .map(|path| {
            let entry = image.entry(&path)?;
            let identity = entry.identity().cloned();
            let result = match entry {
                RepositoryEntry::File { bytes, .. } => Some(
                    serde_json::from_slice::<GateRunResult>(bytes).with_context(|| {
                        format!("failed to parse captured gate run at {path:?}")
                    })?,
                ),
                RepositoryEntry::Absent => None,
                _ => {
                    return Err(anyhow!(
                        "captured gate run at {path:?} is not a regular file"
                    ))
                }
            };
            Ok((path, identity, result))
        })
        .collect::<Result<Vec<_>>>()?;
    captured.sort_by(|left, right| left.0.cmp(&right.0));
    let mut results = captured
        .iter()
        .filter_map(|(_, _, result)| result.clone())
        .filter(|run| run.issue_id == issue_id)
        .collect::<Vec<_>>();
    results.sort_by(|left, right| {
        left.started_at
            .cmp(&right.started_at)
            .then_with(|| left.run_id.cmp(&right.run_id))
    });
    let inputs = captured
        .into_iter()
        .filter(|(_, _, result)| result.as_ref().is_some_and(|run| run.issue_id == issue_id))
        .map(|(path, _, _)| path)
        .collect();
    Ok(CapturedGateRuns { results, inputs })
}

fn captured_gate_evaluation(
    image: &crate::repository_state::RepositoryImage,
    issue: &Issue,
    gate_key: &str,
    gate: &crate::declarations::GateDefinition,
    gate_runs: &CapturedGateRuns,
) -> Result<CapturedGateEvaluation> {
    use crate::declarations::GateChecker;
    use crate::repository_state::VirtualPath;
    use std::collections::{BTreeMap, BTreeSet};

    let checker = gate
        .checker
        .as_ref()
        .ok_or_else(|| anyhow!("Gate '{}' has no checker configured", gate_key))?;
    let broad_validation = checker_needs_validation_image(checker);
    let mut paths = if broad_validation {
        image.entries().keys().cloned().collect::<BTreeSet<_>>()
    } else {
        BTreeSet::from([VirtualPath::data(format!("issues/{}.json", issue.id))?])
    };
    paths.extend(
        issue
            .dependencies
            .iter()
            .map(|id| VirtualPath::data(format!("issues/{id}.json")))
            .collect::<std::result::Result<Vec<_>, _>>()?,
    );
    paths.extend(gate_runs.inputs.iter().cloned());

    let mut wanted_pinned = BTreeSet::new();
    if matches!(checker, GateChecker::Exec { .. }) {
        for document in &issue.documents {
            match &document.commit {
                Some(commit) => {
                    wanted_pinned.insert((commit.clone(), document.path.clone()));
                }
                None => {
                    paths.insert(VirtualPath::worktree(&document.path)?);
                    wanted_pinned.insert(("HEAD".to_string(), document.path.clone()));
                }
            }
        }
    }

    let prompt = match checker {
        GateChecker::Exec {
            pass_context: true,
            prompt_file: Some(path),
            ..
        } => {
            let configured_path = path;
            let path = super::repo_rel_virtual_path(path).map_err(|_| {
                anyhow!(
                    "prompt_file '{}' resolves outside the repository",
                    configured_path
                )
            })?;
            let bytes = image
                .file_bytes(&path)?
                .ok_or_else(|| anyhow!("captured gate prompt file '{path:?}' is missing"))?;
            if bytes.len() as u64 > MAX_PROMPT_FILE_SIZE {
                anyhow::bail!(
                    "prompt_file '{}' exceeds size limit ({} bytes > {} byte limit)",
                    configured_path,
                    bytes.len(),
                    MAX_PROMPT_FILE_SIZE
                );
            }
            paths.insert(path);
            Some(String::from_utf8(bytes.to_vec())?)
        }
        _ => None,
    };
    let inputs = paths
        .into_iter()
        .map(|path| {
            image
                .entry(&path)
                .map(|entry| (path, entry.identity().cloned()))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let pinned = image
        .pinned_evidence()
        .iter()
        .filter(|(key, _)| wanted_pinned.contains(*key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let linked = image
        .linked_worktree_evidence()
        .iter()
        .filter(|(path, _)| inputs.iter().any(|(input, _)| input == *path))
        .map(|(path, value)| (path.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let validation_view = broad_validation.then(|| super::PrecheckValidationView {
        listings: image.listing_fingerprints().clone(),
        pinned: image.pinned_evidence().clone(),
        linked: image.linked_worktree_evidence().clone(),
    });
    Ok(CapturedGateEvaluation {
        evidence: GateEvaluationEvidence {
            gate: gate.clone(),
            inputs,
            pinned,
            linked,
            validation_view,
        },
        prompt,
    })
}

/// Remove the gate currently being evaluated from the issue's gate projections.
///
/// Its recorded status necessarily predates the in-flight evaluation and is not
/// current evidence. The gate definition identifies the active gate, while its
/// compact prior result remains available through `run_history`.
fn omit_current_gate_projection(issue: &mut serde_json::Value, gate_key: &str) {
    if let Some(gates) = issue
        .get_mut("gates")
        .and_then(serde_json::Value::as_array_mut)
    {
        gates.retain(|gate| gate.get("key").and_then(serde_json::Value::as_str) != Some(gate_key));
    }
}

fn repository_rule_report(
    image: &crate::repository_state::RepositoryImage,
) -> crate::validation::report::RuleReport {
    let report = match crate::validation::repository::validate_repository(image) {
        Ok(report) => report,
        Err(error) => error.into_parts().1,
    };
    report.rule_report
}

fn captured_issue_rule_report(
    image: &crate::repository_state::RepositoryImage,
    issue_id: &str,
) -> Result<crate::validation::report::RuleReport> {
    use crate::validation::report::{ReportedFinding, RuleReport};

    let issues = super::captured_active_issues(image)?;
    let issue = issues
        .iter()
        .find(|issue| issue.id == issue_id)
        .ok_or_else(|| crate::storage::IssueNotFoundError::new(issue_id))?;
    let declarations = super::declarations_from_image(image)?;
    let config = crate::repository_state::assemble_config(image)?;
    let repo_format = config
        .validation
        .as_ref()
        .map(crate::config::ValidationConfig::content_format)
        .transpose()?
        .unwrap_or(crate::domain::ContentFormat::Markdown);
    let local = crate::validation::evaluate_local(issue, &declarations.rules, repo_format)
        .map_err(|error| anyhow!("Local rule evaluation failed: {error}"))?;
    let mut findings = local
        .findings()
        .iter()
        .map(|finding| ReportedFinding::new(Some(issue.id.clone()), finding))
        .collect::<Vec<_>>();
    let applicable = declarations
        .rules
        .matching_rules(issue)
        .into_iter()
        .map(|rule| rule.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    let graph_rules = declarations
        .rules
        .rules
        .iter()
        .filter(|rule| rule.scope == crate::declarations::rules::RuleScope::Graph)
        .collect::<Vec<_>>();
    let hierarchy = crate::repository_state::hierarchy_config(
        &crate::config_manager::namespaces_from_config(&config),
    );
    let plan_content = super::validate::plan_content_from_image(image, &issues)?;
    findings.extend(
        crate::validation::graph::evaluate_graph(
            &graph_rules,
            &issues,
            &hierarchy,
            repo_format,
            chrono::Utc::now(),
            &plan_content,
        )
        .into_iter()
        .filter(|finding| {
            finding.issue_id.as_deref() == Some(issue_id)
                || (finding.is_config_error() && applicable.contains(finding.finding.rule.as_str()))
        })
        .map(|finding| ReportedFinding::new(Some(issue.id.clone()), &finding.finding)),
    );
    findings.extend(
        repository_rule_report(image)
            .findings
            .into_iter()
            .filter(|finding| {
                finding.rule == super::DANGLING_LINK_RULE
                    && finding.issue_id.as_deref() == Some(issue_id)
            }),
    );
    Ok(RuleReport { findings })
}

fn captured_scope_rule_report(
    image: &crate::repository_state::RepositoryImage,
    container_id: &str,
) -> Result<crate::validation::report::RuleReport> {
    use crate::declarations::rules::{RuleScope, Severity};
    use crate::validation::report::{ReportedFinding, RuleReport};

    let all = super::captured_active_issues(image)?;
    let config = crate::repository_state::assemble_config(image)?;
    let declarations = super::declarations_from_image(image)?;
    let container_type = all
        .iter()
        .find(|issue| issue.id == container_id)
        .and_then(|issue| label_utils::type_label_value(&issue.labels));
    let breakdown_type = container_type
        .and_then(|kind| config.templates.template_for_container(kind))
        .and_then(|template| template.breakdown_type(&config.templates.roles));
    let scope_ids = crate::domain::queries::bracket_scope_ids(container_id, &all, breakdown_type);
    let slice = all
        .iter()
        .filter(|issue| scope_ids.contains(&issue.id))
        .cloned()
        .collect::<Vec<_>>();
    let repo_format = config
        .validation
        .as_ref()
        .map(crate::config::ValidationConfig::content_format)
        .transpose()?
        .unwrap_or(crate::domain::ContentFormat::Markdown);
    let mut findings = Vec::new();
    for issue in &slice {
        let evaluation = crate::validation::evaluate_local(issue, &declarations.rules, repo_format)
            .map_err(|error| anyhow!("Local rule evaluation failed: {error}"))?;
        findings.extend(
            evaluation
                .findings()
                .iter()
                .map(|finding| ReportedFinding::new(Some(issue.id.clone()), finding)),
        );
    }
    let graph_rules = declarations
        .rules
        .rules
        .iter()
        .filter(|rule| rule.scope == RuleScope::Graph && rule.severity != Severity::Off)
        .filter(|rule| !rule.assert.is_repo_wide_at_transition())
        .filter(|rule| slice.iter().any(|issue| rule.when.matches(issue)))
        .collect::<Vec<_>>();
    if !graph_rules.is_empty() {
        let hierarchy = crate::repository_state::hierarchy_config(
            &crate::config_manager::namespaces_from_config(&config),
        );
        let plan_content = super::validate::plan_content_from_image(image, &slice)?;
        findings.extend(
            crate::validation::graph::evaluate_graph_scoped(
                &graph_rules,
                &slice,
                &all,
                &hierarchy,
                repo_format,
                chrono::Utc::now(),
                &plan_content,
            )
            .iter()
            .map(|finding| ReportedFinding::new(finding.issue_id.clone(), &finding.finding)),
        );
    }
    findings.extend(
        repository_rule_report(image)
            .findings
            .into_iter()
            .filter(|finding| {
                (finding.rule == super::DANGLING_LINK_RULE
                    && finding
                        .issue_id
                        .as_ref()
                        .is_some_and(|id| scope_ids.contains(id)))
                    || finding.rule == super::ENFORCEMENT_DRIFT_RULE
            }),
    );
    Ok(RuleReport { findings })
}

/// Convert validation findings into the gate-run finding contract.
fn gate_findings_from_rule_report(
    report: &crate::validation::report::RuleReport,
) -> Vec<GateFinding> {
    use crate::declarations::rules::Severity;

    report
        .findings
        .iter()
        .filter(|finding| finding.severity != Severity::Off)
        .map(|finding| GateFinding {
            id: String::new(),
            severity: if finding.is_error() { "high" } else { "low" }.to_string(),
            disposition: Some(
                if finding.is_error() {
                    "blocking"
                } else {
                    "advisory"
                }
                .to_string(),
            ),
            origin: Some("issue-impact".to_string()),
            summary: format!("[{}] {}", finding.rule, finding.message),
            file: None,
            line: None,
            references: Vec::new(),
        })
        .collect()
}

/// Convert one whole-repository validation result into the built-in gate finding
/// contract.
///
/// Structural failure remains authoritative while its partial semantic report
/// is converted once through the same severity/disposition mapping as a clean
/// validation result.
fn gate_findings_from_report(
    result: std::result::Result<
        crate::validation::repository::RepositoryValidationReport,
        crate::validation::repository::RepositoryValidationFailure,
    >,
) -> (Vec<GateFinding>, bool) {
    let (structural_error, report) = match result {
        Ok(report) => (None, report),
        Err(failure) => {
            let (error, report) = failure.into_parts();
            (Some(error), report)
        }
    };
    let mut findings = gate_findings_from_rule_report(&report.rule_report);
    findings.extend(structural_error.as_ref().map(|error| GateFinding {
        id: "repository-integrity".to_string(),
        severity: "high".to_string(),
        disposition: Some("blocking".to_string()),
        origin: Some("issue-impact".to_string()),
        summary: format!("{error:#}"),
        file: None,
        line: None,
        references: Vec::new(),
    }));
    let failed = report.rule_report.has_errors() || structural_error.is_some();
    (findings, failed)
}

/// Compare the running binary's own build provenance
/// ([`build_info::version_info`](crate::build_info::version_info)) against
/// `repo_root`'s current `HEAD`, returning why it is stale, or `None` when it
/// is fresh or the comparison does not apply (REQ-03).
///
/// I/O boundary for [`domain::build_provenance`](crate::domain::build_provenance):
/// resolves `repo_root`'s `HEAD` and whether the build commit is a known
/// commit there via [`GitRevisionResolver`](crate::storage::GitRevisionResolver)'s
/// `git rev-parse --verify <rev>^{commit}` semantics (the REQ-03 identity
/// predicate — a plain string inequality is not enough, see the module docs),
/// then hands both to
/// [`assess_binary_provenance`](crate::domain::build_provenance::assess_binary_provenance)
/// for the actual decision. The second resolution (whether the build commit
/// is known) is skipped entirely when `HEAD` itself does not resolve, so a
/// non-git or git-unavailable `repo_root` costs a single git invocation.
///
/// Given an explicit `repo_root` rather than deriving one, so it is directly
/// testable against synthetic repositories; [`CommandExecutor::stale_binary_reason`]
/// is the production entry point that supplies `repo_root` from storage.
fn stale_binary_reason_for_repo(
    repo_root: &std::path::Path,
) -> Option<crate::domain::build_provenance::StaleBinaryReason> {
    use crate::domain::build_provenance::{assess_binary_provenance, BinaryProvenance};
    use crate::storage::GitRevisionResolver;

    let info = crate::build_info::version_info();
    let resolver = GitRevisionResolver::new(repo_root);
    let repo_head = resolver.resolve_commit("HEAD").ok();
    let known_in_repo = repo_head.is_some() && resolver.resolve_commit(info.git_commit).is_ok();

    match assess_binary_provenance(
        Some(info.git_commit),
        info.git_dirty,
        repo_head.as_ref().map(|v| v.as_str()),
        known_in_repo,
    ) {
        BinaryProvenance::Stale(reason) => Some(reason),
        BinaryProvenance::Fresh | BinaryProvenance::NotApplicable => None,
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Repository root used as the checker working directory and as the `git`
    /// context for stamping [`GateRunResult::commit`](crate::domain::GateRunResult).
    ///
    /// This is the parent of the `.jit` directory. For `InMemoryStorage` in tests
    /// the root is `"."`, whose parent is the empty path, so we fall back to the
    /// current working directory to keep the path usable.
    pub(crate) fn checker_repo_root(&self) -> std::path::PathBuf {
        self.storage
            .root()
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            })
    }

    /// The repository's real on-disk root (parent of `.jit`), or `None` when
    /// storage names no real repository at all — e.g. `InMemoryStorage`,
    /// whose `root()` is the placeholder `"."`.
    ///
    /// Distinct from [`checker_repo_root`](Self::checker_repo_root), which
    /// falls back to the current working directory so the checker process
    /// always has SOME directory to run in. The stale-binary check (REQ-03)
    /// must never take that fallback: doing so would compare the running
    /// binary against whatever repository the test/host process happens to be
    /// executing inside, instead of staying silent for storage that names no
    /// real repository.
    pub(crate) fn real_repo_root(&self) -> Option<std::path::PathBuf> {
        self.storage
            .root()
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
    }

    /// Compare the running binary's own build provenance against the
    /// repository this executor is rooted at ([`real_repo_root`](Self::real_repo_root)),
    /// returning why it is stale, or `None` when it is fresh, the comparison
    /// does not apply (REQ-03), or storage names no real on-disk repository
    /// (e.g. `InMemoryStorage`).
    ///
    /// The single production entry point for the stale-binary check
    /// (jit:7446af34): used by [`check_gate`](Self::check_gate) for `exec`
    /// checkers (REQ-01, guards the evaluator's own binary before it spawns a
    /// checker process)
    /// and by the binary crate's startup dispatch (REQ-02, guards a `jit`
    /// process spawned BY a checker — e.g. a checker script that itself
    /// shells out to `jit` — which resolves its own binary from `PATH`
    /// independently of the evaluator and so needs the identical check
    /// applied to ITSELF).
    pub fn stale_binary_reason(
        &self,
    ) -> Option<crate::domain::build_provenance::StaleBinaryReason> {
        let real_root = self.real_repo_root()?;
        stale_binary_reason_for_repo(&real_root)
    }

    fn bind_gate_target(
        &self,
        layout: &crate::repository_state::RepositoryLayout,
        requested: &str,
    ) -> Result<String>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{CaptureBudget, CaptureSpec, VirtualPath};
        use crate::storage::RepositoryStateStoreError;

        let normalized = requested.to_lowercase().replace('-', "");
        if normalized.len() < crate::storage::MIN_ID_PREFIX_LENGTH {
            return Err(crate::storage::InvalidIdPrefixError::new(requested).into());
        }
        let budget = CaptureBudget {
            max_paths: 1 << 16,
            max_listings: 256,
            max_bytes: 512 * 1024 * 1024,
            max_depth: 32,
        };
        for _ in 0..8 {
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let mut spec = CaptureSpec::phase_one([VirtualPath::data("index.json")?], budget)?;
            let index_image = match session.capture(spec.clone()) {
                Ok(image) => image,
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            };
            let index = captured_repository_index(&index_image)?;
            let candidates = index
                .all_ids
                .iter()
                .filter(|id| {
                    if normalized.len() == 32 {
                        id.as_str() == requested
                    } else {
                        id.to_lowercase().replace('-', "").starts_with(&normalized)
                    }
                })
                .cloned()
                .collect::<Vec<_>>();
            spec.discover_paths(
                candidates
                    .iter()
                    .map(|id| VirtualPath::data(format!("issues/{id}.json")))
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            )?;
            let image = match session.capture(spec) {
                Ok(image) => image,
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            };
            let issues = candidates
                .iter()
                .map(|id| {
                    captured_issue_record(&image, id)?
                        .ok_or_else(|| crate::storage::IssueNotFoundError::new(id).into())
                })
                .collect::<Result<Vec<_>>>()?;
            return super::resolve_issue_from_capture(&issues, requested);
        }
        Err(anyhow!(
            "gate target binding did not converge after repeated capture conflicts"
        ))
    }

    fn capture_gate_evaluation_image(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
        target_id: &str,
        gate_key: &str,
        run_paths: &[crate::repository_state::VirtualPath],
    ) -> Result<Option<crate::repository_state::RepositoryImage>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::declarations::GateChecker;
        use crate::repository_state::{CaptureBudget, CaptureSpec, VirtualPath};
        use crate::storage::RepositoryStateStoreError;
        use std::collections::BTreeMap;

        let budget = CaptureBudget {
            max_paths: 1 << 16,
            max_listings: 256,
            max_bytes: 512 * 1024 * 1024,
            max_depth: 32,
        };
        let mut paths = vec![
            VirtualPath::data("index.json")?,
            VirtualPath::data(format!("issues/{target_id}.json"))?,
            VirtualPath::data("gates.toml")?,
            VirtualPath::data("events.jsonl")?,
            VirtualPath::data("gate-runs")?,
        ];
        paths.extend(run_paths.iter().cloned());
        let mut spec = CaptureSpec::phase_one(paths, budget)?;
        let initial = match session.capture(spec.clone()) {
            Ok(image) => image,
            Err(RepositoryStateStoreError::RetryableConflict { .. }) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let issue = captured_bound_issue(&initial, target_id)?;
        let registry = captured_gate_registry(&initial)?;
        let gate = required_automated_gate(&issue, &registry, gate_key)?;
        let checker = gate
            .checker
            .as_ref()
            .ok_or_else(|| anyhow!("Gate '{}' has no checker configured", gate_key))?;
        if checker_needs_validation_image(checker) {
            let Some(image) =
                self.capture_proposed_base(session, &BTreeMap::new(), run_paths, None)?
            else {
                return Ok(None);
            };
            return Ok(Some(image));
        }

        spec.discover_paths(
            issue
                .dependencies
                .iter()
                .map(|id| VirtualPath::data(format!("issues/{id}.json")))
                .collect::<std::result::Result<Vec<_>, _>>()?,
        )?;
        if matches!(checker, GateChecker::Exec { .. }) {
            for document in &issue.documents {
                match &document.commit {
                    Some(commit) => spec.discover_pinned(commit.clone(), document.path.clone())?,
                    None => {
                        spec.discover_paths([VirtualPath::worktree(&document.path)?])?;
                        spec.discover_pinned("HEAD", document.path.clone())?;
                    }
                }
            }
        }
        if let GateChecker::Exec {
            pass_context: true,
            prompt_file: Some(path),
            ..
        } = checker
        {
            let prompt = super::repo_rel_virtual_path(path)
                .map_err(|_| anyhow!("prompt_file '{}' resolves outside the repository", path))?;
            spec.discover_paths([prompt])?;
        }
        match session.capture(spec) {
            Ok(image) if super::checker_consumes_run_history(checker) => {
                super::capture_gate_run_results(session, image)
            }
            Ok(image) => Ok(Some(image)),
            Err(RepositoryStateStoreError::RetryableConflict { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Check a single gate for an issue
    ///
    /// Runs the configured native or `exec` checker for an automated gate,
    /// updates the issue status, and returns the run result. When an `exec`
    /// checker has `pass_context: true`, builds structured context (issue data,
    /// gate definition, prompt, run history) and passes it to the checker
    /// process via a temp file.
    pub fn check_gate(&self, issue_id: &str, gate_key: &str) -> Result<GateRunResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, MutationContext, MutationIntent, VirtualPath};
        use crate::storage::RepositoryStateStoreError;

        let layout = self.require_layout()?;
        let target_id = self.bind_gate_target(&layout, issue_id)?;
        let mutation = MutationContext::production();
        let mut cached = None::<CachedGateEvaluation>;
        let run_id = mutation.identifier_at(0);
        let result_path = crate::repository_state::gate_run_result_relative_path(&run_id)?;
        let run_dir = result_path
            .as_path()
            .parent()
            .ok_or_else(|| anyhow!("canonical gate-run result path has no parent"))?;
        let run_paths = [
            VirtualPath::data("gate-runs")?,
            VirtualPath::data(run_dir)?,
            VirtualPath::data(result_path.as_path())?,
        ];
        for _ in 0..8 {
            let (image, issue, issues, registry, runs, captured) = {
                let mut session = self.storage.open_mutation_session(layout.clone())?;
                let Some(image) = self.capture_gate_evaluation_image(
                    session.as_mut(),
                    &target_id,
                    gate_key,
                    &run_paths,
                )?
                else {
                    continue;
                };
                let issue = captured_bound_issue(&image, &target_id)?;
                let registry = captured_gate_registry(&image)?;
                let gate = required_automated_gate(&issue, &registry, gate_key)?;
                let checker = gate
                    .checker
                    .as_ref()
                    .ok_or_else(|| anyhow!("Gate '{}' has no checker configured", gate_key))?;
                let issues = if checker_needs_validation_image(checker) {
                    super::captured_active_issues(&image)?
                } else {
                    std::iter::once(issue.clone())
                        .chain(captured_direct_dependency_issues(&image, &issue)?)
                        .collect()
                };
                let runs = if super::checker_consumes_run_history(checker) {
                    captured_gate_runs(&image, &target_id)?
                } else {
                    CapturedGateRuns::default()
                };
                let captured = captured_gate_evaluation(&image, &issue, gate_key, gate, &runs)?;
                (image, issue, issues, registry, runs, captured)
            };
            if !cached
                .as_ref()
                .is_some_and(|cached| cached.evidence == captured.evidence)
            {
                let gate = registry
                    .gates
                    .get(gate_key)
                    .ok_or_else(|| crate::storage::GateNotFoundError::single(gate_key))?;
                let result = self.execute_captured_gate(CapturedGateExecution {
                    image: &image,
                    issue: &issue,
                    issues: &issues,
                    gate_key,
                    gate,
                    runs: &runs.results,
                    prompt: captured.prompt.as_deref(),
                })?;
                cached = Some(CachedGateEvaluation {
                    evidence: captured.evidence.clone(),
                    result,
                });
            }
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some(final_image) = self.capture_gate_evaluation_image(
                session.as_mut(),
                &target_id,
                gate_key,
                &run_paths,
            )?
            else {
                continue;
            };
            let mut final_issue = captured_bound_issue(&final_image, &target_id)?;
            let final_registry = captured_gate_registry(&final_image)?;
            let final_gate = required_automated_gate(&final_issue, &final_registry, gate_key)?;
            let final_runs = if final_gate
                .checker
                .as_ref()
                .is_some_and(super::checker_consumes_run_history)
            {
                captured_gate_runs(&final_image, &target_id)?
            } else {
                CapturedGateRuns::default()
            };
            let final_captured = captured_gate_evaluation(
                &final_image,
                &final_issue,
                gate_key,
                final_gate,
                &final_runs,
            )?;
            let Some(cached) = cached.as_ref() else {
                continue;
            };
            if cached.evidence != final_captured.evidence {
                continue;
            }

            let (state, by) = gate_state_from_run(&cached.result)?;
            final_issue.gates_status.insert(gate_key.to_string(), state);
            let event = if cached.result.status == GateRunStatus::Passed {
                Event::draft_gate_passed(target_id.clone(), gate_key.to_string(), by)
            } else {
                Event::draft_gate_failed(target_id.clone(), gate_key.to_string(), by)
            };
            let intents = [
                MutationIntent::UpdateIssue {
                    issue: Box::new(final_issue),
                },
                MutationIntent::RecordGateRun {
                    draft: Box::new(cached.result.clone()),
                },
                MutationIntent::RecordEvent {
                    phase: 1,
                    event: Box::new(event),
                },
            ];
            let plan = finalize(&layout, &final_image, &mutation, &intents)?;
            match session.apply(&plan) {
                Ok(_) => {
                    let mut result = cached.result.clone();
                    result.run_id = run_id;
                    return Ok(result);
                }
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(anyhow!(
            "gate evaluation did not converge after repeated capture conflicts"
        ))
    }

    /// Execute a built-in checker against one exact repository image.
    fn execute_builtin_checker_with_repository_view(
        &self,
        gate_key: &str,
        issue_id: &str,
        stage: GateStage,
        checker: &crate::declarations::GateChecker,
        repository_image: &crate::repository_state::RepositoryImage,
    ) -> Result<GateRunResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::declarations::GateChecker;
        use crate::validation::report::RuleReport;

        let started_at = chrono::Utc::now();
        let start = std::time::Instant::now();

        let (command, mut findings, explicit_failure) = match checker {
            GateChecker::RepositoryValidation => {
                let (findings, failed) = gate_findings_from_report(
                    crate::validation::repository::validate_repository(repository_image),
                );
                ("builtin:repository_validation", findings, failed)
            }
            GateChecker::IssueValidation => {
                let report = captured_issue_rule_report(repository_image, issue_id)?;
                let failed = report.has_errors();
                (
                    "builtin:issue_validation",
                    gate_findings_from_rule_report(&report),
                    failed,
                )
            }
            GateChecker::LabelTargetValidation { label_namespace } => {
                let captured_issues = super::captured_active_issues(repository_image)?;
                let issue = captured_issues
                    .iter()
                    .find(|issue| issue.id == issue_id)
                    .cloned()
                    .ok_or_else(|| crate::storage::IssueNotFoundError::new(issue_id))?;
                let prefix = format!("{label_namespace}:");
                let targets: Vec<&str> = issue
                    .labels
                    .iter()
                    .filter_map(|label| label.strip_prefix(&prefix))
                    .collect();
                if label_namespace.is_empty() || targets.len() != 1 || targets[0].is_empty() {
                    let detail = if label_namespace.is_empty() {
                        "label_namespace is empty".to_string()
                    } else {
                        format!(
                            "expected exactly one non-empty '{}:<target-id>' label on issue {}, found {}",
                            label_namespace,
                            issue_id,
                            targets.len()
                        )
                    };
                    (
                        "builtin:label_target_validation",
                        vec![GateFinding {
                            id: "label-target".to_string(),
                            severity: "high".to_string(),
                            disposition: Some("blocking".to_string()),
                            origin: Some("issue-impact".to_string()),
                            summary: detail,
                            file: None,
                            line: None,
                            references: Vec::new(),
                        }],
                        true,
                    )
                } else {
                    let target = super::resolve_issue_from_capture(&captured_issues, targets[0])?;
                    let report: RuleReport = captured_scope_rule_report(repository_image, &target)?;
                    let failed = report.has_errors();
                    (
                        "builtin:label_target_validation",
                        gate_findings_from_rule_report(&report),
                        failed,
                    )
                }
            }
            GateChecker::ReviewPlaceholder => (
                "builtin:review_placeholder",
                vec![GateFinding {
                    id: "review-placeholder".to_string(),
                    severity: "high".to_string(),
                    disposition: Some("advisory".to_string()),
                    origin: Some("pre-existing".to_string()),
                    summary: REVIEW_PLACEHOLDER_WARNING.to_string(),
                    file: None,
                    line: None,
                    references: Vec::new(),
                }],
                false,
            ),
            GateChecker::Exec { .. } => unreachable!("exec checker is dispatched separately"),
        };

        // Stable ids make findings useful as data regardless of the configured
        // gate key. Preserve explicit ids used for configuration/integrity
        // failures and number ordinary rule findings in evaluation order.
        for (index, finding) in findings.iter_mut().enumerate() {
            if finding.id.is_empty() {
                finding.id = format!("F{}", index + 1);
            }
        }

        let status = if explicit_failure {
            GateRunStatus::Failed
        } else {
            GateRunStatus::Passed
        };
        let finding_count = findings.len();
        let summary = match checker {
            GateChecker::ReviewPlaceholder => REVIEW_PLACEHOLDER_WARNING.to_string(),
            _ if status == GateRunStatus::Passed => {
                format!("Built-in validation passed with {finding_count} advisory finding(s)")
            }
            _ => format!("Built-in validation failed with {finding_count} finding(s)"),
        };
        let stdout = if findings.is_empty() {
            summary.clone()
        } else {
            std::iter::once(summary.clone())
                .chain(findings.iter().map(|finding| {
                    let label = if finding.disposition.as_deref() == Some("blocking") {
                        "ERROR"
                    } else {
                        "WARNING"
                    };
                    format!("{label} [{}] {}", finding.id, finding.summary)
                }))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let message = matches!(checker, GateChecker::ReviewPlaceholder)
            .then(|| REVIEW_PLACEHOLDER_WARNING.to_string());

        Ok(GateRunResult {
            schema_version: crate::domain::GATE_RUN_SCHEMA_VERSION,
            // Repository mutation finalization assigns the durable run identity.
            run_id: String::new(),
            gate_key: gate_key.to_string(),
            stage,
            issue_id: issue_id.to_string(),
            commit: None,
            branch: None,
            // In-process built-in checker: it stamps no commit, so there is no
            // named commit for the tree to match.
            tree_dirty: None,
            status,
            started_at,
            completed_at: Some(chrono::Utc::now()),
            duration_ms: Some(start.elapsed().as_millis() as u64),
            exit_code: Some(if status == GateRunStatus::Passed {
                0
            } else {
                4
            }),
            stdout,
            stderr: String::new(),
            command: command.to_string(),
            by: Some(gate_execution::AUTO_EXECUTOR.to_string()),
            message,
            findings: Some(GateFindings {
                verdict: if status == GateRunStatus::Passed {
                    "pass".to_string()
                } else {
                    "fail".to_string()
                },
                summary,
                findings,
            }),
        })
    }

    /// Return the most recent gate run result for a given issue and gate key.
    ///
    /// Returns `Ok(None)` if no runs have been recorded yet.
    pub fn get_last_gate_run(
        &self,
        issue_id: &str,
        gate_key: &str,
    ) -> Result<Option<GateRunResult>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut runs = self.storage.list_gate_runs_for_issue(&full_id)?;
        runs.retain(|r| r.gate_key == gate_key);
        runs.sort_by_key(|r| r.started_at);
        Ok(runs.into_iter().next_back())
    }

    fn build_captured_gate_context(
        &self,
        checker: &crate::declarations::GateChecker,
        input: &CapturedGateExecution<'_>,
    ) -> Result<Option<GateContext>> {
        let (pass_context, inline_prompt, prompt_file) = match checker {
            crate::declarations::GateChecker::Exec {
                pass_context,
                prompt,
                prompt_file,
                ..
            } => (*pass_context, prompt.as_deref(), prompt_file.as_deref()),
            _ => return Ok(None),
        };
        if !pass_context {
            return Ok(None);
        }
        let prompt = match (prompt_file, input.prompt) {
            (Some(path), Some(prompt)) => {
                if prompt.len() as u64 > MAX_PROMPT_FILE_SIZE {
                    anyhow::bail!(
                        "prompt_file '{}' exceeds size limit ({} bytes > {} byte limit)",
                        path,
                        prompt.len(),
                        MAX_PROMPT_FILE_SIZE
                    );
                }
                Some(prompt.to_string())
            }
            (Some(path), None) => anyhow::bail!(
                "captured gate input is missing configured prompt_file '{}'",
                path
            ),
            (None, _) => inline_prompt.map(str::to_string),
        };
        let enriched = input
            .issue
            .dependencies
            .iter()
            .filter_map(|id| input.issues.iter().find(|candidate| candidate.id == *id))
            .map(crate::domain::MinimalIssue::from)
            .collect();
        let mut issue_json = serde_json::to_value(IssueShowResponse::from_issue(
            input.issue.clone(),
            enriched,
            input.runs,
        ))?;
        omit_current_gate_projection(&mut issue_json, input.gate_key);
        Ok(Some(GateContext {
            schema_version: 1,
            prompt,
            issue: issue_json,
            gate: serde_json::json!({
                "key": input.gate.key,
                "title": input.gate.title,
                "description": input.gate.description,
                "stage": input.gate.stage,
            }),
            run_history: compact_run_history_for_context(input.runs, input.gate_key),
        }))
    }

    fn execute_captured_gate(&self, input: CapturedGateExecution<'_>) -> Result<GateRunResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let checker = input
            .gate
            .checker
            .as_ref()
            .ok_or_else(|| anyhow!("Gate '{}' has no checker configured", input.gate_key))?;
        let repo_root = self.checker_repo_root();
        if matches!(checker, crate::declarations::GateChecker::Exec { .. }) {
            if let Some(reason) = self.stale_binary_reason() {
                return Err(crate::errors::StaleBinaryError::new(
                    &input.issue.id,
                    input.gate_key,
                    &reason,
                )
                .into());
            }
        }
        let working_dir = match checker {
            crate::declarations::GateChecker::Exec {
                working_dir: Some(subdir),
                ..
            } => repo_root.join(subdir),
            _ => repo_root,
        };
        let context = self.build_captured_gate_context(checker, &input)?;
        match checker {
            crate::declarations::GateChecker::Exec { .. } => {
                self.storage.run_external_process(|| {
                    gate_execution::execute_gate_checker_with_context(
                        input.gate_key,
                        &input.issue.id,
                        input.gate.stage,
                        checker,
                        &working_dir,
                        context.as_ref(),
                        &input.issue.documents,
                    )
                })
            }
            _ => self.execute_builtin_checker_with_repository_view(
                input.gate_key,
                &input.issue.id,
                input.gate.stage,
                checker,
                input.image,
            ),
        }
    }

    /// Return the most recent recorded run for each automated gate on an issue.
    ///
    /// Results are ordered by gate priority, preserving insertion order for ties.
    /// The second vector contains automated gate keys that have not been run yet.
    pub fn get_last_gate_runs_for_issue(
        &self,
        issue_id: &str,
    ) -> Result<(Vec<GateRunResult>, Vec<String>)> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;
        let registry = self.storage.load_gate_registry()?;

        // Collect auto gates and sort by priority (stable sort preserves insertion order for ties)
        let mut auto_gates: Vec<_> = issue
            .gates_required
            .iter()
            .filter_map(|key| registry.gates.get(key).map(|g| (key, g)))
            .filter(|(_, gate)| gate.mode == GateMode::Auto)
            .collect();
        auto_gates.sort_by_key(|(_, gate)| gate.priority);

        let latest_runs = self
            .storage
            .list_gate_runs_for_issue(&full_id)?
            .into_iter()
            .fold(
                HashMap::<String, GateRunResult>::new(),
                |mut latest_runs, run| {
                    match latest_runs.get(&run.gate_key) {
                        Some(existing) if existing.started_at >= run.started_at => {}
                        _ => {
                            latest_runs.insert(run.gate_key.clone(), run);
                        }
                    }
                    latest_runs
                },
            );

        let (results, not_run) = auto_gates.into_iter().fold(
            (Vec::new(), Vec::new()),
            |(mut results, mut not_run), (gate_key, _)| {
                if let Some(result) = latest_runs.get(gate_key).cloned() {
                    results.push(result);
                } else {
                    not_run.push(gate_key.clone());
                }
                (results, not_run)
            },
        );

        Ok((results, not_run))
    }

    /// Return the per-issue readiness status of every REQUIRED gate on an issue
    /// (automated AND manual), ordered by gate priority (insertion order for
    /// ties).
    ///
    /// The authoritative status is the issue's recorded `gates_status`: an auto
    /// gate's last run and a manual gate's attestation both write there. A gate
    /// with no recorded state — an auto gate never run, or a manual gate never
    /// attested — is reported as [`GateStatus::Pending`].
    ///
    /// This is the manual-aware companion to
    /// [`get_last_gate_runs_for_issue`](Self::get_last_gate_runs_for_issue),
    /// which covers automated gates only. `status-all` folds these statuses into
    /// its green/not-green readiness verdict.
    pub fn get_required_gate_statuses_for_issue(
        &self,
        issue_id: &str,
    ) -> Result<Vec<(String, GateStatus)>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;
        let registry = self.storage.load_gate_registry()?;

        // Order by gate priority; a required key missing from the registry sorts
        // last but is still reported (so it is never silently treated as green).
        let mut ordered: Vec<_> = issue
            .gates_required
            .iter()
            .map(|key| {
                let priority = registry.gates.get(key).map_or(u32::MAX, |g| g.priority);
                (key.clone(), priority)
            })
            .collect();
        ordered.sort_by_key(|(_, priority)| *priority);

        Ok(ordered
            .into_iter()
            .map(|(key, _)| {
                let status = issue
                    .gates_status
                    .get(&key)
                    .map_or(GateStatus::Pending, |state| state.status);
                (key, status)
            })
            .collect())
    }

    /// Execute prechecks from one closed repository image without publishing.
    pub(super) fn execute_captured_prechecks(
        &self,
        image: &crate::repository_state::RepositoryImage,
        issue: &Issue,
        registry: &crate::declarations::GateRegistry,
        captured_prompts: &HashMap<String, String>,
    ) -> Result<PrecheckExecution>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let issues = super::captured_active_issues(image)?;
        let mut projected_issue = issue.clone();
        let mut failed_gates = Vec::new();
        let mut runs = Vec::new();

        // Collect precheck gates and sort by priority (stable sort preserves insertion order for ties)
        let mut precheck_gates: Vec<_> = issue
            .gates_required
            .iter()
            .filter_map(|key| registry.gates.get(key).map(|g| (key, g)))
            .filter(|(_, gate)| gate.stage == GateStage::Precheck)
            .collect();
        precheck_gates.sort_by_key(|(_, gate)| gate.priority);
        let mut history = if precheck_gates.iter().any(|(_, gate)| {
            gate.mode == GateMode::Auto
                && gate
                    .checker
                    .as_ref()
                    .is_some_and(super::checker_consumes_run_history)
        }) {
            captured_gate_runs(image, &issue.id)?.results
        } else {
            Vec::new()
        };

        for (gate_key, gate) in precheck_gates {
            match gate.mode {
                GateMode::Auto => {
                    let checker_history = gate
                        .checker
                        .as_ref()
                        .filter(|checker| super::checker_consumes_run_history(checker))
                        .map_or(&[][..], |_| history.as_slice());
                    let result = match self.execute_captured_gate(CapturedGateExecution {
                        image,
                        issue: &projected_issue,
                        issues: &issues,
                        gate_key,
                        gate,
                        runs: checker_history,
                        prompt: captured_prompts.get(gate_key).map(String::as_str),
                    }) {
                        Ok(result) => result,
                        Err(error) => {
                            return Ok(PrecheckExecution {
                                runs,
                                error: Some(error),
                            })
                        }
                    };
                    if result.status != GateRunStatus::Passed {
                        failed_gates.push((gate_key.clone(), result.clone()));
                    }
                    projected_issue
                        .gates_status
                        .insert(gate_key.clone(), gate_state_from_run(&result)?.0);
                    history.push(result.clone());
                    runs.push(result);
                }
                GateMode::Manual => {
                    // Check if manual precheck already passed
                    let gate_status = issue.gates_status.get(gate_key);
                    if !matches!(gate_status, Some(state) if state.status == GateStatus::Passed) {
                        let status = gate_status
                            .map(|state| state.status)
                            .unwrap_or(GateStatus::Pending);
                        return Ok(PrecheckExecution {
                            runs,
                            error: Some(
                                TransitionBlockedError::gates(
                                    issue.id.clone(),
                                    State::InProgress,
                                    issue.state,
                                    vec![(gate_key.clone(), status, GateMode::Manual)],
                                )
                                .into(),
                            ),
                        });
                    }
                }
            }
        }

        if !failed_gates.is_empty() {
            return Ok(PrecheckExecution {
                runs,
                error: Some(
                    TransitionBlockedError::gates(
                        issue.id.clone(),
                        State::InProgress,
                        issue.state,
                        failed_gates
                            .into_iter()
                            .map(|(key, _)| (key, GateStatus::Failed, GateMode::Auto))
                            .collect(),
                    )
                    .into(),
                ),
            });
        }

        Ok(PrecheckExecution { runs, error: None })
    }

    /// Run all postchecks for an issue
    ///
    /// Runs all automated postchecks and auto-transitions to Done if all pass.
    pub(crate) fn run_postchecks(&self, issue_id: &str) -> Result<()>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;
        let registry = self.storage.load_gate_registry()?;

        for gate_key in &issue.gates_required {
            if let Some(gate) = registry.gates.get(gate_key) {
                if gate.stage == GateStage::Postcheck && gate.mode == GateMode::Auto {
                    // Run automated postcheck (errors are logged but don't fail)
                    let _ = self.check_gate(&full_id, gate_key);
                }
            }
        }

        // Try to auto-transition to done
        self.auto_transition_to_done(&full_id)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{compact_run_history_for_context, omit_current_gate_projection};
    use crate::commands::CommandExecutor;
    use crate::declarations::{GateChecker, GateMode, GateStage};
    use crate::domain::{
        GateFindings, GateRunResult, GateRunStatus, State, GATE_RUN_SCHEMA_VERSION,
    };
    use crate::hierarchy_templates::HierarchyTemplate;
    use crate::storage::{InMemoryStorage, IssueStore, JsonFileStorage};
    use chrono::{TimeZone, Utc};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn add_builtin_gate<S: IssueStore>(
        executor: &CommandExecutor<S>,
        gate_key: &str,
        checker: GateChecker,
        labels: Vec<String>,
    ) -> String {
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            gate_key.to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: gate_key.to_string(),
                title: "Portable check".to_string(),
                description: "Portable check".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(checker),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        let mut issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        issue.labels = labels;
        issue.gates_required.push(gate_key.to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        issue_id
    }

    fn setup() -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        // Create config with enforcement off for test backward compatibility
        std::fs::create_dir_all(storage.root()).unwrap();
        let config_toml = r#"
[worktree]
enforce_leases = "off"
"#;
        std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

        crate::commands::test_helpers::memory_executor(storage)
    }

    fn prior_run(
        run_id: &str,
        gate_key: &str,
        started_at: i64,
        findings: Option<GateFindings>,
    ) -> GateRunResult {
        GateRunResult {
            schema_version: GATE_RUN_SCHEMA_VERSION,
            run_id: run_id.to_string(),
            gate_key: gate_key.to_string(),
            stage: GateStage::Postcheck,
            issue_id: "issue-1".to_string(),
            commit: Some("abc123".to_string()),
            branch: Some("main".to_string()),
            tree_dirty: None,
            status: GateRunStatus::Failed,
            started_at: Utc.timestamp_opt(started_at, 0).unwrap(),
            completed_at: Some(Utc.timestamp_opt(started_at + 1, 0).unwrap()),
            duration_ms: Some(1_000),
            exit_code: Some(1),
            stdout: format!("stdout-{run_id}"),
            stderr: format!("stderr-{run_id}"),
            command: "review".to_string(),
            by: Some("agent:reviewer".to_string()),
            message: Some("review complete".to_string()),
            findings,
        }
    }

    #[test]
    fn test_compact_run_history_selects_latest_matching_run_by_timestamp() {
        let latest = prior_run("latest", "review", 30, None);
        let runs = vec![
            latest.clone(),
            prior_run("other-gate", "cargo-ci", 40, None),
            prior_run("oldest", "review", 10, None),
            prior_run("middle", "review", 20, None),
        ];

        let compact = compact_run_history_for_context(&runs, "review");

        assert_eq!(compact.len(), 1);
        assert_eq!(compact[0].run_id, latest.run_id);
        assert_eq!(compact[0].stdout, latest.stdout);
        assert!(compact[0].stderr.is_empty());
        assert_eq!(runs[0], latest, "source run must remain unchanged");
    }

    #[test]
    fn test_compact_run_history_prefers_structured_findings_over_stdout() {
        let findings = GateFindings {
            verdict: "fail".to_string(),
            summary: "one defect".to_string(),
            findings: Vec::new(),
        };
        let source = prior_run("structured", "review", 10, Some(findings.clone()));

        let compact = compact_run_history_for_context(std::slice::from_ref(&source), "review");

        assert_eq!(compact.len(), 1);
        assert_eq!(compact[0].findings, Some(findings));
        assert!(compact[0].stdout.is_empty());
        assert!(compact[0].stderr.is_empty());
        assert_eq!(source.stdout, "stdout-structured");
        assert_eq!(source.stderr, "stderr-structured");
    }

    #[test]
    fn test_compact_run_history_is_empty_without_matching_runs() {
        let runs = vec![prior_run("cargo", "cargo-ci", 10, None)];

        assert!(compact_run_history_for_context(&runs, "review").is_empty());
    }

    #[test]
    fn test_omit_current_gate_projection_retains_other_gate_evidence() {
        let mut issue = serde_json::json!({
            "gates": [
                {"key": "cargo-ci", "status": "passed", "exit_code": 0},
                {"key": "review", "status": "failed", "exit_code": 1}
            ]
        });

        omit_current_gate_projection(&mut issue, "review");

        assert_eq!(
            issue["gates"],
            serde_json::json!([
                {"key": "cargo-ci", "status": "passed", "exit_code": 0}
            ])
        );
    }

    #[test]
    fn test_projected_precheck_run_matches_next_checker_issue_view() {
        let mut issue = crate::domain::types::fixture_issue("Test".into(), String::new());
        issue.id = "issue-1".into();
        issue.gates_required = vec!["first".into(), "second".into()];
        let mut run = prior_run("", "first", 10, None);
        run.status = GateRunStatus::Passed;
        issue
            .gates_status
            .insert("first".into(), super::gate_state_from_run(&run).unwrap().0);

        let mut view = serde_json::to_value(crate::output::IssueShowResponse::from_issue(
            issue,
            Vec::new(),
            std::slice::from_ref(&run),
        ))
        .unwrap();
        omit_current_gate_projection(&mut view, "second");

        assert_eq!(view["gates"][0]["status"], "passed");
        assert!(view["gates"][0]["last_run_at"].is_string());
        assert!(compact_run_history_for_context(&[run], "second").is_empty());
    }

    #[test]
    fn test_check_gate_automated_success() {
        let executor = setup();

        // Define an automated gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "test-gate".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "test-gate".to_string(),
                title: "Test Gate".to_string(),
                description: "Test gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "exit 0".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with gate
        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "test-gate".to_string())
            .unwrap();

        // Check the gate
        let result = executor.check_gate(&issue_id, "test-gate").unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
        assert_eq!(result.exit_code, Some(0));

        // Verify gate status was updated on issue
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        let gate_state = issue.gates_status.get("test-gate").unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Passed);
    }

    #[test]
    fn test_review_placeholder_passes_with_unmistakable_structured_warning() {
        let executor = setup();
        let issue_id = add_builtin_gate(
            &executor,
            "arbitrary-review-key",
            GateChecker::ReviewPlaceholder,
            vec!["type:task".to_string()],
        );

        let result = executor
            .check_gate(&issue_id, "arbitrary-review-key")
            .unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
        assert!(result
            .stdout
            .contains("WARNING: EXTERNAL REVIEW PLACEHOLDER"));
        assert!(result
            .message
            .as_deref()
            .unwrap()
            .contains("WITHOUT RUNNING A REVIEWER"));
        let findings = result.findings.unwrap();
        assert_eq!(findings.verdict, "pass");
        assert_eq!(findings.findings.len(), 1);
        assert_eq!(
            findings.findings[0].disposition.as_deref(),
            Some("advisory")
        );
        assert!(findings.findings[0]
            .summary
            .contains("Replace this checker"));
    }

    #[test]
    fn test_pass_gate_surfaces_review_placeholder_warning_on_success() {
        let executor = setup();
        let issue_id = add_builtin_gate(
            &executor,
            "editable-review",
            GateChecker::ReviewPlaceholder,
            vec!["type:task".to_string()],
        );

        let outcome = executor
            .pass_gate(&issue_id, "editable-review".to_string(), None, true)
            .unwrap();

        assert!(!outcome.already_passed);
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("WITHOUT RUNNING A REVIEWER"));
    }

    #[test]
    fn test_issue_validation_runs_in_process_under_arbitrary_gate_key() {
        let executor = setup();
        let issue_id = add_builtin_gate(
            &executor,
            "not-a-reserved-validation-key",
            GateChecker::IssueValidation,
            vec!["type:task".to_string()],
        );

        let result = executor
            .check_gate(&issue_id, "not-a-reserved-validation-key")
            .unwrap();

        assert_eq!(result.command, "builtin:issue_validation");
        assert_ne!(result.status, GateRunStatus::Error);
        assert!(result.findings.is_some());
    }

    #[test]
    fn test_repository_validation_runs_in_process_under_arbitrary_gate_key() {
        let executor = setup();
        let issue_id = add_builtin_gate(
            &executor,
            "not-a-reserved-repository-key",
            GateChecker::RepositoryValidation,
            vec!["type:task".to_string()],
        );

        let result = executor
            .check_gate(&issue_id, "not-a-reserved-repository-key")
            .unwrap();

        assert_eq!(result.command, "builtin:repository_validation");
        assert_ne!(result.status, GateRunStatus::Error);
        assert!(result.findings.is_some());
    }

    const LATE_REPOSITORY_RULE: &str = r#"
[[rules]]
name = "planned-task-needs-summary"
when = { type = "task" }
severity = "error"
enforce = false
assert = { require-section = { heading = "Summary" } }
"#;

    fn setup_file_repository() -> (tempfile::TempDir, CommandExecutor<JsonFileStorage>, String) {
        let repo = tempfile::tempdir().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        std::fs::create_dir_all(storage.root()).unwrap();
        std::fs::write(
            storage.root().join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        let executor = CommandExecutor::new(storage).with_layout(layout);
        executor
            .initialize_fresh_repository(repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();
        let issue_id = add_builtin_gate(
            &executor,
            "not-a-reserved-repository-key",
            GateChecker::RepositoryValidation,
            vec!["type:task".to_string()],
        );
        let mut registry = executor.storage.load_gate_registry().unwrap();
        let mut placeholder = registry
            .gates
            .get("not-a-reserved-repository-key")
            .unwrap()
            .clone();
        placeholder.key = "editable-review".to_string();
        placeholder.checker = Some(GateChecker::ReviewPlaceholder);
        registry.gates.insert(placeholder.key.clone(), placeholder);
        executor.storage.save_gate_registry(&registry).unwrap();
        (repo, executor, issue_id)
    }

    fn add_exec_gate<S: IssueStore + crate::storage::RepositoryStateStore>(
        executor: &CommandExecutor<S>,
        issue_id: &str,
        gate_key: &str,
        title: &str,
        command: String,
        prompt_file: Option<String>,
    ) {
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            gate_key.to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: gate_key.to_string(),
                title: title.to_string(),
                description: "Race fixture".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command,
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: Some("inline prompt".to_string()),
                    prompt_file,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();
        executor.add_gate(issue_id, gate_key.to_string()).unwrap();
    }

    fn write_corrupt_gate_run(repo: &std::path::Path) {
        let path = repo.join(".jit/gate-runs/corrupt/result.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "{ corrupt").unwrap();
    }

    fn capture_gate_evidence(
        executor: &CommandExecutor<JsonFileStorage>,
        issue_id: &str,
        gate_key: &str,
    ) -> super::GateEvaluationEvidence {
        use crate::storage::RepositoryStateStore;

        let layout = crate::storage::discover_repository_layout(
            executor.storage.root().parent().unwrap(),
            executor.storage.root(),
        )
        .unwrap();
        let result =
            crate::repository_state::gate_run_result_relative_path("evidence-probe").unwrap();
        let run_paths = [
            crate::repository_state::VirtualPath::data("gate-runs/evidence-probe").unwrap(),
            crate::repository_state::VirtualPath::data(result.as_path()).unwrap(),
        ];
        let mut session = executor.storage.open_mutation_session(layout).unwrap();
        let image = executor
            .capture_gate_evaluation_image(session.as_mut(), issue_id, gate_key, &run_paths)
            .unwrap()
            .unwrap();
        let issue = super::captured_bound_issue(&image, issue_id).unwrap();
        let registry = super::captured_gate_registry(&image).unwrap();
        let runs = if registry.gates[gate_key]
            .checker
            .as_ref()
            .is_some_and(crate::commands::checker_consumes_run_history)
        {
            super::captured_gate_runs(&image, issue_id).unwrap()
        } else {
            super::CapturedGateRuns::default()
        };
        super::captured_gate_evaluation(&image, &issue, gate_key, &registry.gates[gate_key], &runs)
            .unwrap()
            .evidence
    }

    #[test]
    fn test_exec_gate_ignores_unrelated_malformed_issue_and_unsafe_prompt() {
        let (repo, executor, issue_id) = setup_file_repository();
        add_exec_gate(
            &executor,
            &issue_id,
            "selected-safe",
            "Selected safe",
            "true".to_string(),
            None,
        );
        add_exec_gate(
            &executor,
            &issue_id,
            "unselected-unsafe",
            "Unselected unsafe",
            "true".to_string(),
            Some("../../outside".to_string()),
        );
        let unrelated = "00000000-0000-4000-8000-000000000001";
        std::fs::write(
            repo.path().join(format!(".jit/issues/{unrelated}.json")),
            [],
        )
        .unwrap();
        let index_path = repo.path().join(".jit/index.json");
        let mut index: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&index_path).unwrap()).unwrap();
        index["all_ids"]
            .as_array_mut()
            .unwrap()
            .push(unrelated.into());
        std::fs::write(index_path, serde_json::to_vec_pretty(&index).unwrap()).unwrap();

        let result = executor.check_gate(&issue_id, "selected-safe").unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
    }

    #[test]
    fn test_exec_without_context_ignores_unrelated_corrupt_gate_run() {
        let (repo, executor, issue_id) = setup_file_repository();
        add_exec_gate(
            &executor,
            &issue_id,
            "no-context",
            "No context",
            "true".to_string(),
            None,
        );
        let mut registry = executor.storage.load_gate_registry().unwrap();
        let Some(GateChecker::Exec { pass_context, .. }) = registry
            .gates
            .get_mut("no-context")
            .and_then(|gate| gate.checker.as_mut())
        else {
            panic!("test gate must use the exec checker")
        };
        *pass_context = false;
        executor.storage.save_gate_registry(&registry).unwrap();
        write_corrupt_gate_run(repo.path());

        let result = executor.check_gate(&issue_id, "no-context").unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
    }

    #[test]
    fn test_exec_with_context_rejects_corrupt_gate_run_with_path() {
        let (repo, executor, issue_id) = setup_file_repository();
        add_exec_gate(
            &executor,
            &issue_id,
            "with-context",
            "With context",
            "true".to_string(),
            None,
        );
        write_corrupt_gate_run(repo.path());

        let error = executor.check_gate(&issue_id, "with-context").unwrap_err();

        assert!(format!("{error:#}").contains("gate-runs/corrupt/result.json"));
    }

    #[test]
    fn test_review_placeholder_ignores_unsupported_target_documents() {
        let (_repo, executor, issue_id) = setup_file_repository();
        executor
            .add_gate(&issue_id, "editable-review".to_string())
            .unwrap();
        let mut issue = executor.storage.load_issue(&issue_id).unwrap();
        issue.documents.push(crate::domain::DocumentReference::new(
            "../../outside".to_string(),
        ));
        executor.storage.save_issue(issue).unwrap();

        let result = executor.check_gate(&issue_id, "editable-review").unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
    }

    #[test]
    fn test_builtin_gate_evidence_changes_with_broad_validation_input() {
        let (repo, executor, issue_id) = setup_file_repository();
        let before = capture_gate_evidence(&executor, &issue_id, "not-a-reserved-repository-key");
        std::fs::write(
            repo.path().join(".jit/rules.toml"),
            "# changed broad validation input\n",
        )
        .unwrap();

        let after = capture_gate_evidence(&executor, &issue_id, "not-a-reserved-repository-key");

        assert_ne!(before, after);
    }

    #[test]
    fn test_repository_validation_checker_uses_injected_overlay_result_path() {
        let (_repo, executor, issue_id) = setup_file_repository();
        let live_image = executor
            .capture_validation_image_with(&std::collections::BTreeMap::new())
            .unwrap();
        let live = executor
            .execute_builtin_checker_with_repository_view(
                "not-a-reserved-repository-key",
                &issue_id,
                GateStage::Postcheck,
                &GateChecker::RepositoryValidation,
                &live_image,
            )
            .unwrap();
        assert_eq!(live.status, GateRunStatus::Passed);
        assert!(live.findings.unwrap().findings.iter().any(|finding| {
            finding.severity == "low"
                && finding.disposition.as_deref() == Some("advisory")
                && finding.summary.contains("review-placeholder")
                && finding.summary.contains("editable-review")
        }));

        let mut planned_issue = executor.storage.load_issue(&issue_id).unwrap();
        planned_issue.dependencies.push("nonexistent".to_string());
        let overrides = crate::commands::overrides_from_repo_changes([
            (
                PathBuf::from(".jit/rules.toml"),
                Some(LATE_REPOSITORY_RULE.as_bytes().to_vec()),
            ),
            (
                PathBuf::from(format!(".jit/issues/{issue_id}.json")),
                Some(serde_json::to_vec_pretty(&planned_issue).unwrap()),
            ),
        ])
        .unwrap();
        let planned_image = executor.capture_validation_image_with(&overrides).unwrap();
        let planned = executor
            .execute_builtin_checker_with_repository_view(
                "not-a-reserved-repository-key",
                &issue_id,
                GateStage::Postcheck,
                &GateChecker::RepositoryValidation,
                &planned_image,
            )
            .unwrap();

        assert_eq!(planned.command, "builtin:repository_validation");
        assert_eq!(planned.status, GateRunStatus::Failed);
        assert_eq!(planned.exit_code, Some(4));
        let findings = planned.findings.unwrap().findings;
        assert!(
            findings.iter().any(|finding| {
                finding.id == "repository-integrity"
                    && finding.severity == "high"
                    && finding.disposition.as_deref() == Some("blocking")
                    && finding.summary.contains("does not exist")
            }),
            "{findings:#?}"
        );
        assert!(findings.iter().any(|finding| {
            finding.severity == "high"
                && finding.disposition.as_deref() == Some("blocking")
                && finding.summary.contains("planned-task-needs-summary")
        }));
    }

    #[test]
    fn test_check_gate_file_repository_ignores_legacy_cached_rules() {
        let (repo, executor, issue_id) = setup_file_repository();
        let cached = executor.run_rules(None).unwrap();
        assert!(!cached
            .findings
            .iter()
            .any(|finding| finding.rule == "planned-task-needs-summary"));
        std::fs::write(repo.path().join(".jit/rules.toml"), LATE_REPOSITORY_RULE).unwrap();

        let result = executor
            .check_gate(&issue_id, "not-a-reserved-repository-key")
            .unwrap();

        assert_eq!(result.command, "builtin:repository_validation");
        assert_eq!(result.status, GateRunStatus::Failed);
        assert_eq!(result.exit_code, Some(4));
        assert!(result
            .findings
            .unwrap()
            .findings
            .iter()
            .any(|finding| finding.summary.contains("planned-task-needs-summary")));
    }

    #[test]
    fn test_repository_validation_file_backend_reports_missing_index_through_view() {
        let (repo, executor, issue_id) = setup_file_repository();
        std::fs::remove_file(repo.path().join(".jit/index.json")).unwrap();
        let image = executor
            .capture_validation_image_with(&std::collections::BTreeMap::new())
            .unwrap();

        let result = executor
            .execute_builtin_checker_with_repository_view(
                "not-a-reserved-repository-key",
                &issue_id,
                GateStage::Postcheck,
                &GateChecker::RepositoryValidation,
                &image,
            )
            .unwrap();

        assert_eq!(result.status, GateRunStatus::Failed);
        assert_eq!(result.exit_code, Some(4));
        assert!(result
            .findings
            .unwrap()
            .findings
            .iter()
            .any(|finding| finding.summary.contains("index.json")));
    }

    #[test]
    fn test_repository_validation_warns_for_sorted_placeholder_gate_keys() {
        let executor = setup();
        add_builtin_gate(
            &executor,
            "z-review",
            GateChecker::ReviewPlaceholder,
            vec!["type:task".to_string()],
        );
        add_builtin_gate(
            &executor,
            "a-review",
            GateChecker::ReviewPlaceholder,
            vec!["type:task".to_string()],
        );

        let findings = executor.review_placeholder_findings().unwrap();

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, crate::commands::REVIEW_PLACEHOLDER_RULE);
        assert_eq!(
            findings[0].severity,
            crate::declarations::rules::Severity::Warn
        );
        assert!(findings[0].message.contains("a-review, z-review"));
    }

    #[test]
    fn test_label_target_validation_reports_missing_configured_pointer_as_finding() {
        let executor = setup();
        let issue_id = add_builtin_gate(
            &executor,
            "arbitrary-coverage-key",
            GateChecker::LabelTargetValidation {
                label_namespace: "parent-pointer".to_string(),
            },
            vec!["type:task".to_string()],
        );

        let result = executor
            .check_gate(&issue_id, "arbitrary-coverage-key")
            .unwrap();

        assert_eq!(result.status, GateRunStatus::Failed);
        assert_eq!(result.exit_code, Some(4));
        let findings = result.findings.unwrap();
        assert_eq!(findings.verdict, "fail");
        assert!(findings.findings[0]
            .summary
            .contains("parent-pointer:<target-id>"));
    }

    #[test]
    fn test_label_target_validation_passes_for_configured_target() {
        let executor = setup();
        let mut target =
            crate::domain::types::fixture_issue("Container".to_string(), "Container".to_string());
        target.labels = vec!["type:task".to_string()];
        let target_id = target.id.clone();
        executor.storage.save_issue(target).unwrap();

        let issue_id = add_builtin_gate(
            &executor,
            "arbitrary-coverage-key",
            GateChecker::LabelTargetValidation {
                label_namespace: "coverage-target".to_string(),
            },
            vec![
                "type:task".to_string(),
                format!("coverage-target:{target_id}"),
            ],
        );

        let result = executor
            .check_gate(&issue_id, "arbitrary-coverage-key")
            .unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
        assert_eq!(result.command, "builtin:label_target_validation");
        assert_eq!(result.findings.unwrap().verdict, "pass");
    }

    #[test]
    fn test_check_gate_automated_failure() {
        let executor = setup();

        // Define an automated gate that fails
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "failing-gate".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "failing-gate".to_string(),
                title: "Failing Gate".to_string(),
                description: "Gate that fails".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "exit 1".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with gate
        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "failing-gate".to_string())
            .unwrap();

        // Check the gate
        let result = executor.check_gate(&issue_id, "failing-gate").unwrap();

        assert_eq!(result.status, GateRunStatus::Failed);
        assert_eq!(result.exit_code, Some(1));

        // Verify gate status was updated on issue
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        let gate_state = issue.gates_status.get("failing-gate").unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Failed);
    }

    #[test]
    fn test_check_gate_manual_not_checkable() {
        let executor = setup();

        // Define a manual gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "manual-gate".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "manual-gate".to_string(),
                title: "Manual Gate".to_string(),
                description: "Manual gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
                priority: 100,
                reserved: HashMap::new(),
                auto: false,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with gate
        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "manual-gate".to_string())
            .unwrap();

        // Try to check the gate - should fail
        let result = executor.check_gate(&issue_id, "manual-gate");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("manual"));
    }

    #[test]
    fn test_get_required_gate_statuses_covers_auto_and_manual() {
        use crate::domain::GateStatus;
        let executor = setup();

        // One auto gate (priority 10) and one manual gate (priority 20).
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "auto-gate".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "auto-gate".to_string(),
                title: "Auto".to_string(),
                description: "Test".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "exit 0".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 10,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        registry.gates.insert(
            "manual-gate".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "manual-gate".to_string(),
                title: "Manual".to_string(),
                description: "Test".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
                priority: 20,
                reserved: HashMap::new(),
                auto: false,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "auto-gate".to_string())
            .unwrap();
        executor
            .add_gate(&issue_id, "manual-gate".to_string())
            .unwrap();

        // Before any evaluation: both required gates are pending, ordered by priority.
        let statuses = executor
            .get_required_gate_statuses_for_issue(&issue_id)
            .unwrap();
        assert_eq!(
            statuses,
            vec![
                ("auto-gate".to_string(), GateStatus::Pending),
                ("manual-gate".to_string(), GateStatus::Pending),
            ]
        );

        // Running the auto gate flips only its status; the manual gate stays pending.
        executor.check_gate(&issue_id, "auto-gate").unwrap();
        let statuses = executor
            .get_required_gate_statuses_for_issue(&issue_id)
            .unwrap();
        assert_eq!(
            statuses,
            vec![
                ("auto-gate".to_string(), GateStatus::Passed),
                ("manual-gate".to_string(), GateStatus::Pending),
            ]
        );

        // Attesting the manual gate makes every required gate green.
        executor
            .pass_gate(
                &issue_id,
                "manual-gate".to_string(),
                Some("human:tester".to_string()),
                false,
            )
            .unwrap();
        let statuses = executor
            .get_required_gate_statuses_for_issue(&issue_id)
            .unwrap();
        assert!(statuses.iter().all(|(_, s)| *s == GateStatus::Passed));
    }

    #[test]
    fn test_get_last_gate_runs_for_issue() {
        let executor = setup();

        // Define two automated gates
        let mut registry = executor.storage.load_gate_registry().unwrap();
        for (key, exit_code) in [("gate-1", 0), ("gate-2", 0)] {
            registry.gates.insert(
                key.to_string(),
                crate::declarations::GateDefinition {
                    version: 1,
                    key: key.to_string(),
                    title: format!("Gate {}", key),
                    description: "Test".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: format!("exit {}", exit_code),
                        timeout_seconds: 10,
                        working_dir: None,
                        env: HashMap::new(),
                        pass_context: false,
                        prompt: None,
                        prompt_file: None,
                    }),
                    priority: 100,
                    reserved: HashMap::new(),
                    auto: true,
                    example_integration: None,
                },
            );
        }
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with both gates
        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "gate-1".to_string()).unwrap();
        executor.add_gate(&issue_id, "gate-2".to_string()).unwrap();

        executor.check_gate(&issue_id, "gate-1").unwrap();
        executor.check_gate(&issue_id, "gate-2").unwrap();

        let (results, warnings) = executor.get_last_gate_runs_for_issue(&issue_id).unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.status == GateRunStatus::Passed));
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_get_last_gate_runs_for_issue_reports_not_run_gates() {
        let executor = setup();

        let mut registry = executor.storage.load_gate_registry().unwrap();
        for key in ["gate-1", "gate-2"] {
            registry
                .gates
                .insert(key.to_string(), make_auto_gate(key, "exit 0"));
        }
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "gate-1".to_string()).unwrap();
        executor.add_gate(&issue_id, "gate-2".to_string()).unwrap();

        executor.check_gate(&issue_id, "gate-1").unwrap();

        let (results, not_run) = executor.get_last_gate_runs_for_issue(&issue_id).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].gate_key, "gate-1");
        assert_eq!(not_run, vec!["gate-2".to_string()]);
    }

    #[test]
    fn test_run_prechecks_before_starting_work() {
        let executor = setup();

        // Define a precheck gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "precheck".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "precheck".to_string(),
                title: "Precheck".to_string(),
                description: "Precheck".to_string(),
                stage: GateStage::Precheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "exit 0".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        registry.gates.insert(
            "postcheck-with-unsafe-prompt".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "postcheck-with-unsafe-prompt".to_string(),
                title: "Postcheck".to_string(),
                description: "Must not enter precheck capture".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "exit 0".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: None,
                    prompt_file: Some("../../outside".to_string()),
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with precheck
        let mut issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::Ready;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "precheck".to_string())
            .unwrap();
        executor
            .add_gate(&issue_id, "postcheck-with-unsafe-prompt".to_string())
            .unwrap();

        // Try to start work - should run prechecks
        executor
            .update_issue_state(&issue_id, State::InProgress)
            .unwrap();

        // Verify prechecks ran and issue transitioned
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::InProgress);

        let gate_state = issue.gates_status.get("precheck").unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Passed);
    }

    #[cfg(unix)]
    #[test]
    fn test_lifecycle_precheck_captures_large_history_and_ignores_root_clutter() {
        let (repo, executor, issue_id) = setup_file_repository();
        add_exec_gate(
            &executor,
            &issue_id,
            "precheck",
            "Precheck",
            "true".to_string(),
            None,
        );
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.get_mut("precheck").unwrap().stage = GateStage::Precheck;
        executor.storage.save_gate_registry(&registry).unwrap();
        let mut issue = executor.storage.load_issue(&issue_id).unwrap();
        issue.state = State::Ready;
        executor.storage.save_issue(issue).unwrap();

        let root = repo.path().join(".jit/gate-runs");
        for index in 0..257 {
            let run_id = format!("history-{index}");
            let run_dir = root.join(&run_id);
            std::fs::create_dir_all(&run_dir).unwrap();
            let mut run = prior_run(&run_id, "history", index, None);
            run.issue_id = issue_id.clone();
            std::fs::write(
                run_dir.join("result.json"),
                serde_json::to_vec_pretty(&run).unwrap(),
            )
            .unwrap();
        }
        std::fs::write(root.join("clutter-file"), "ignored").unwrap();
        std::os::unix::fs::symlink("clutter-file", root.join("clutter-link")).unwrap();
        assert!(std::process::Command::new("mkfifo")
            .arg(root.join("clutter-fifo"))
            .status()
            .unwrap()
            .success());

        executor
            .update_issue_state(&issue_id, State::InProgress)
            .unwrap();

        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::InProgress);
        assert_eq!(
            issue.gates_status["precheck"].status,
            crate::domain::GateStatus::Passed
        );
    }

    #[test]
    fn test_lifecycle_precheck_without_context_ignores_corrupt_gate_run() {
        let (repo, executor, issue_id) = setup_file_repository();
        add_exec_gate(
            &executor,
            &issue_id,
            "precheck-no-context",
            "Precheck no context",
            "true".to_string(),
            None,
        );
        let mut registry = executor.storage.load_gate_registry().unwrap();
        let gate = registry.gates.get_mut("precheck-no-context").unwrap();
        gate.stage = GateStage::Precheck;
        let Some(GateChecker::Exec { pass_context, .. }) = gate.checker.as_mut() else {
            panic!("test gate must use the exec checker")
        };
        *pass_context = false;
        executor.storage.save_gate_registry(&registry).unwrap();
        let mut issue = executor.storage.load_issue(&issue_id).unwrap();
        issue.state = State::Ready;
        executor.storage.save_issue(issue).unwrap();
        write_corrupt_gate_run(repo.path());

        executor
            .update_issue_state(&issue_id, State::InProgress)
            .unwrap();

        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::InProgress);
        assert_eq!(
            issue.gates_status["precheck-no-context"].status,
            crate::domain::GateStatus::Passed
        );
    }

    #[test]
    fn test_strict_lease_rejection_precedes_external_precheck() {
        let executor = crate::commands::test_helpers::setup_with_enforcement("strict");
        let marker = executor
            .storage
            .root()
            .parent()
            .unwrap()
            .join("unauthorized-precheck-marker");
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "precheck".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "precheck".to_string(),
                title: "Precheck".to_string(),
                description: String::new(),
                stage: GateStage::Precheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "touch unauthorized-precheck-marker".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();
        let mut issue = crate::domain::types::fixture_issue("Test".into(), String::new());
        issue.state = State::Ready;
        issue.gates_required.push("precheck".to_string());
        let id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();

        let error = executor
            .update_issue_state(&id, State::InProgress)
            .expect_err("strict lease enforcement must reject before checker execution");

        assert!(error.to_string().contains("No active lease"));
        assert!(!marker.exists());
    }

    #[test]
    fn test_precheck_failure_blocks_transition() {
        let executor = setup();

        // Define a failing precheck gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "precheck-fail".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "precheck-fail".to_string(),
                title: "Precheck".to_string(),
                description: "Precheck".to_string(),
                stage: GateStage::Precheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "exit 1".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with failing precheck
        let mut issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::Ready;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "precheck-fail".to_string())
            .unwrap();

        // Try to start work - should fail
        let result = executor.update_issue_state(&issue_id, State::InProgress);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("precheck"));

        // Verify issue didn't transition
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Ready);
    }

    #[test]
    fn test_run_postchecks_on_completion() {
        let executor = setup();

        // Define a postcheck gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "postcheck".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "postcheck".to_string(),
                title: "Postcheck".to_string(),
                description: "Postcheck".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "exit 0".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue in progress with postcheck
        let mut issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "postcheck".to_string())
            .unwrap();

        // Complete work
        executor
            .update_issue_state(&issue_id, State::Gated)
            .unwrap();

        // Verify postchecks ran and issue transitioned to done
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Done);

        let gate_state = issue.gates_status.get("postcheck").unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Passed);
    }

    #[test]
    fn test_postcheck_failure_keeps_in_gated() {
        let executor = setup();

        // Define a failing postcheck gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "postcheck-fail".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "postcheck-fail".to_string(),
                title: "Postcheck".to_string(),
                description: "Postcheck".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "exit 1".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue in progress with failing postcheck
        let mut issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "postcheck-fail".to_string())
            .unwrap();

        // Complete work
        executor
            .update_issue_state(&issue_id, State::Gated)
            .unwrap();

        // Verify postchecks ran but issue stayed in gated
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Gated);

        let gate_state = issue.gates_status.get("postcheck-fail").unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Failed);
    }

    #[test]
    fn test_check_gate_with_pass_context_builds_context() {
        let executor = setup();

        // Define a gate with pass_context that reads the context file
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "review".to_string(),
                title: "Code Review".to_string(),
                description: "AI-powered code review".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "cat $JIT_CONTEXT_FILE".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: Some("Review the implementation for correctness.".to_string()),
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create an issue with the gate
        let mut issue = crate::domain::types::fixture_issue(
            "Implement feature X".to_string(),
            "Add the X feature".to_string(),
        );
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "review".to_string()).unwrap();

        // Check the gate - should build context and pass it
        let result = executor.check_gate(&issue_id, "review").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);

        // Parse the context JSON from stdout
        let context: serde_json::Value =
            serde_json::from_str(&result.stdout).expect("stdout should be valid context JSON");

        assert_eq!(context["schema_version"], 1);
        assert_eq!(
            context["prompt"],
            "Review the implementation for correctness."
        );
        assert_eq!(context["issue"]["title"], "Implement feature X");
        assert_eq!(context["gate"]["key"], "review");
        assert_eq!(context["gate"]["title"], "Code Review");
        assert!(
            context["issue"]["gates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|gate| gate["key"] != "review"),
            "the current gate must not receive its stale pre-run projection"
        );
        assert!(context["run_history"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_check_gate_with_prompt_file() {
        let executor = setup();

        // Write a prompt file relative to repo root
        executor
            .storage
            .write_repo_file(
                "review-prompt.md",
                "You are a senior engineer. Review for security issues.",
            )
            .unwrap();

        // Define a gate with prompt_file
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "review".to_string(),
                title: "Code Review".to_string(),
                description: "AI review".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "cat $JIT_CONTEXT_FILE".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: Some("This should be overridden by prompt_file".to_string()),
                    prompt_file: Some("review-prompt.md".to_string()),
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue
        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "review".to_string()).unwrap();

        let result = executor.check_gate(&issue_id, "review").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);

        let context: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        // prompt_file takes precedence over inline prompt
        assert_eq!(
            context["prompt"],
            "You are a senior engineer. Review for security issues."
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_reexecutes_when_prompt_changes_during_checker() {
        let (repo, executor, issue_id) = setup_file_repository();
        std::fs::write(repo.path().join("review-prompt.md"), "old prompt").unwrap();
        add_exec_gate(
            &executor,
            &issue_id,
            "prompt-race",
            "Prompt race",
            "if [ ! -f .prompt-race-seen ]; then : > .prompt-race-seen; printf 'new prompt' > review-prompt.md; fi; printf x >> checker-count; cat \"$JIT_CONTEXT_FILE\"".to_string(),
            Some("review-prompt.md".to_string()),
        );

        let result = executor.check_gate(&issue_id, "prompt-race").unwrap();
        let context: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();

        assert_eq!(context["prompt"], "new prompt");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("checker-count")).unwrap(),
            "xx"
        );
        assert_eq!(
            executor
                .storage
                .list_gate_runs_for_issue(&issue_id)
                .unwrap()
                .into_iter()
                .filter(|run| run.gate_key == "prompt-race")
                .count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_rebases_final_update_when_issue_changes_during_checker() {
        let (repo, executor, issue_id) = setup_file_repository();
        let issue_path = format!(".jit/issues/{issue_id}.json");
        add_exec_gate(
            &executor,
            &issue_id,
            "issue-race",
            "Issue race",
            format!(
                "if [ ! -f .issue-race-seen ]; then : > .issue-race-seen; sed -i 's/\"title\": \"Test\"/\"title\": \"Changed concurrently\"/' {issue_path}; fi; printf x >> checker-count; cat \"$JIT_CONTEXT_FILE\""
            ),
            None,
        );

        let result = executor.check_gate(&issue_id, "issue-race").unwrap();
        let context: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();

        assert_eq!(context["issue"]["title"], "Changed concurrently");
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().title,
            "Changed concurrently"
        );
        assert_eq!(
            std::fs::read_to_string(repo.path().join("checker-count")).unwrap(),
            "xx"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_reexecutes_when_direct_dependency_changes() {
        let (repo, executor, issue_id) = setup_file_repository();
        let dependency = crate::domain::types::fixture_issue("Dependency".into(), "Test".into());
        let dependency_id = dependency.id.clone();
        executor.storage.save_issue(dependency).unwrap();
        executor.add_dependency(&issue_id, &dependency_id).unwrap();
        let dependency_path = format!(".jit/issues/{dependency_id}.json");
        add_exec_gate(
            &executor,
            &issue_id,
            "dependency-race",
            "Dependency race",
            format!(
                "if [ ! -f .dependency-race-seen ]; then : > .dependency-race-seen; sed -i 's/\"title\": \"Dependency\"/\"title\": \"Changed dependency\"/' {dependency_path}; fi; printf x >> checker-count; cat \"$JIT_CONTEXT_FILE\""
            ),
            None,
        );

        let result = executor.check_gate(&issue_id, "dependency-race").unwrap();
        let context: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();

        assert_eq!(
            context["issue"]["dependencies"][0]["title"],
            "Changed dependency"
        );
        assert_eq!(
            std::fs::read_to_string(repo.path().join("checker-count")).unwrap(),
            "xx"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_reexecutes_when_gate_definition_changes_during_checker() {
        let (repo, executor, issue_id) = setup_file_repository();
        std::fs::write(
            repo.path().join("gate-race.sh"),
            "if [ ! -f .gate-race-seen ]; then : > .gate-race-seen; sed -i 's/Original race title/Changed race title/' .jit/gates.toml; fi\nprintf x >> checker-count\ncat \"$JIT_CONTEXT_FILE\"\n",
        )
        .unwrap();
        add_exec_gate(
            &executor,
            &issue_id,
            "gate-race",
            "Original race title",
            "sh gate-race.sh".to_string(),
            None,
        );

        let result = executor.check_gate(&issue_id, "gate-race").unwrap();
        let context: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();

        assert_eq!(context["gate"]["title"], "Changed race title");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("checker-count")).unwrap(),
            "xx"
        );
        assert_eq!(
            executor
                .storage
                .list_gate_runs_for_issue(&issue_id)
                .unwrap()
                .into_iter()
                .filter(|run| run.gate_key == "gate-race")
                .count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_never_retargets_bound_short_id_after_replacement() {
        let (repo, executor, issue_id) = setup_file_repository();
        let replacement_id = format!("{}-ffff-4fff-8fff-ffffffffffff", &issue_id[..8]);
        add_exec_gate(
            &executor,
            &issue_id,
            "target-race",
            "Target race",
            format!(
                "if [ ! -f .target-race-seen ]; then : > .target-race-seen; sed -i 's/{issue_id}/{replacement_id}/g' .jit/index.json; rm .jit/issues/{issue_id}.json; cp replacement.json .jit/issues/{replacement_id}.json; fi; true"
            ),
            None,
        );
        let mut replacement = executor.storage.load_issue(&issue_id).unwrap();
        replacement.id = replacement_id.clone();
        replacement.gates_status.clear();
        std::fs::write(
            repo.path().join("replacement.json"),
            serde_json::to_vec_pretty(&replacement).unwrap(),
        )
        .unwrap();

        let error = executor
            .check_gate(&issue_id[..8], "target-race")
            .unwrap_err();

        assert!(
            error
                .downcast_ref::<crate::storage::IssueNotFoundError>()
                .is_some(),
            "bound target disappearance must be typed, got {error:?}"
        );
        assert!(!executor
            .storage
            .load_issue(&replacement_id)
            .unwrap()
            .gates_status
            .contains_key("target-race"));
        assert!(executor
            .storage
            .list_gate_runs_for_issue(&replacement_id)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_check_gate_rejects_noncanonical_full_issue_ids() {
        let (_repo, executor, issue_id) = setup_file_repository();
        add_exec_gate(
            &executor,
            &issue_id,
            "canonical-id",
            "Canonical id",
            "true".to_string(),
            None,
        );

        for requested in [issue_id.to_uppercase(), issue_id.replace('-', "")] {
            let error = executor.check_gate(&requested, "canonical-id").unwrap_err();
            assert!(
                error
                    .downcast_ref::<crate::storage::IssueNotFoundError>()
                    .is_some(),
                "noncanonical full id {requested} must not resolve: {error:?}"
            );
        }
        assert!(executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_reexecutes_for_new_same_issue_run_but_not_unrelated_run() {
        let (repo, executor, issue_id) = setup_file_repository();
        let mut concurrent = prior_run(
            "concurrent",
            "run-race",
            chrono::Utc::now().timestamp(),
            None,
        );
        concurrent.issue_id = issue_id.clone();
        std::fs::write(
            repo.path().join("concurrent-run.json"),
            serde_json::to_vec_pretty(&concurrent).unwrap(),
        )
        .unwrap();
        add_exec_gate(
            &executor,
            &issue_id,
            "run-race",
            "Run race",
            "if [ ! -f .run-race-seen ]; then : > .run-race-seen; mkdir -p .jit/gate-runs/concurrent; cp concurrent-run.json .jit/gate-runs/concurrent/result.json; fi; printf x >> checker-count; cat \"$JIT_CONTEXT_FILE\"".to_string(),
            None,
        );

        let result = executor.check_gate(&issue_id, "run-race").unwrap();
        let context: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();

        assert_eq!(
            std::fs::read_to_string(repo.path().join("checker-count")).unwrap(),
            "xx"
        );
        assert_eq!(context["run_history"][0]["run_id"], "concurrent");

        let (repo, executor, issue_id) = setup_file_repository();
        concurrent.issue_id = "unrelated-issue".to_string();
        std::fs::write(
            repo.path().join("concurrent-run.json"),
            serde_json::to_vec_pretty(&concurrent).unwrap(),
        )
        .unwrap();
        add_exec_gate(
            &executor,
            &issue_id,
            "run-race",
            "Run race",
            "mkdir -p .jit/gate-runs/concurrent; cp concurrent-run.json .jit/gate-runs/concurrent/result.json; printf x >> checker-count; cat \"$JIT_CONTEXT_FILE\"".to_string(),
            None,
        );

        executor.check_gate(&issue_id, "run-race").unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.path().join("checker-count")).unwrap(),
            "x"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_reuses_checker_result_after_apply_only_conflict() {
        let executor = setup();
        let issue = crate::domain::types::fixture_issue("Apply race".into(), "Test".into());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let count = temp.path().join("checker-count");
        add_exec_gate(
            &executor,
            &issue_id,
            "apply-race",
            "Apply race",
            format!("printf x >> {}; true", count.display()),
            None,
        );
        executor.storage.inject_repository_state_apply_conflicts(1);

        let result = executor.check_gate(&issue_id, "apply-race").unwrap();

        assert_eq!(std::fs::read_to_string(count).unwrap(), "x");
        let runs = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .into_iter()
            .filter(|run| run.gate_key == "apply-race")
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, result.run_id);
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().gates_status["apply-race"].status,
            crate::domain::GateStatus::Passed
        );
        assert_eq!(
            executor
                .storage
                .read_events()
                .unwrap()
                .into_iter()
                .filter(|event| {
                    matches!(
                        event,
                        crate::domain::Event::GatePassed {
                            issue_id: event_issue,
                            gate_key,
                            ..
                        } if event_issue == &issue_id && gate_key == "apply-race"
                    )
                })
                .count(),
            1
        );
    }

    #[test]
    fn test_check_gate_run_history_keeps_only_latest_legacy_run() {
        let executor = setup();

        // Define a gate with pass_context that always fails (exit 1) but outputs context
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "review".to_string(),
                title: "Code Review".to_string(),
                description: "Review".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    // Output context to stdout, emit noise to stderr, then fail.
                    // The stderr noise must NOT reappear in the next run's context.
                    command: "cat $JIT_CONTEXT_FILE; echo 'noisy diagnostic' >&2; exit 1"
                        .to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: Some("Review".to_string()),
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue
        let mut issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "review".to_string()).unwrap();

        // First run - should have empty history
        let result1 = executor.check_gate(&issue_id, "review").unwrap();
        assert_eq!(result1.status, GateRunStatus::Failed);
        // The checker really did emit stderr (so the strip below is meaningful).
        assert!(
            result1.stderr.contains("noisy diagnostic"),
            "checker should produce stderr, got {:?}",
            result1.stderr
        );

        // stdout contains the context JSON (before the exit 1)
        let ctx1: serde_json::Value = serde_json::from_str(&result1.stdout).unwrap();
        assert_eq!(ctx1["run_history"].as_array().unwrap().len(), 0);

        // Second run - should include first run in history
        let result2 = executor.check_gate(&issue_id, "review").unwrap();
        let ctx2: serde_json::Value = serde_json::from_str(&result2.stdout).unwrap();
        let history2 = ctx2["run_history"].as_array().unwrap();
        assert_eq!(history2.len(), 1);
        assert_eq!(history2[0]["status"], "failed");
        // stderr is stripped from the context handed to the next checker.
        assert_eq!(
            history2[0]["stderr"], "",
            "stderr must be stripped from run_history context"
        );

        // Third run still includes only the latest previous run.
        let result3 = executor.check_gate(&issue_id, "review").unwrap();
        let ctx3: serde_json::Value = serde_json::from_str(&result3.stdout).unwrap();
        let history3 = ctx3["run_history"].as_array().unwrap();
        assert_eq!(history3.len(), 1);
        assert_eq!(history3[0]["run_id"], result2.run_id);
        assert!(!history3[0]["stdout"].as_str().unwrap().is_empty());
    }

    #[test]
    fn test_prompt_file_path_traversal_rejected() {
        let executor = setup();

        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "review".to_string(),
                title: "Review".to_string(),
                description: "Review".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "echo ok".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: None,
                    prompt_file: Some("../../etc/passwd".to_string()),
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "review".to_string()).unwrap();

        let result = executor.check_gate(&issue_id, "review");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("outside the repository"),
            "Expected path traversal error, got: {}",
            err
        );
    }

    #[test]
    fn test_run_history_is_compacted_to_one_latest_run() {
        let executor = setup();

        // Define a context-aware gate that always fails
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "review".to_string(),
                title: "Review".to_string(),
                description: "Review".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "cat $JIT_CONTEXT_FILE; exit 1".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: Some("Review".to_string()),
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        let mut issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "review".to_string()).unwrap();

        // Run the gate repeatedly to exercise history compaction.
        for _ in 0..7 {
            let _ = executor.check_gate(&issue_id, "review").unwrap();
        }

        // The next run receives exactly one latest prior run.
        let result = executor.check_gate(&issue_id, "review").unwrap();
        let ctx: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        let history = ctx["run_history"].as_array().unwrap();
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn test_prompt_file_size_limit_enforced() {
        let executor = setup();

        // Write a prompt file that exceeds the size limit (~100KB, well over 1000 lines)
        let big_content = (0..1100)
            .map(|_| "x".repeat(100))
            .collect::<Vec<_>>()
            .join("\n"); // 1100 lines of 100 chars = ~110KB
        executor
            .storage
            .write_repo_file("huge-prompt.md", &big_content)
            .unwrap();

        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "review".to_string(),
                title: "Review".to_string(),
                description: "Review".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "echo ok".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: None,
                    prompt_file: Some("huge-prompt.md".to_string()),
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "review".to_string()).unwrap();

        let result = executor.check_gate(&issue_id, "review");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("exceeds"),
            "Expected size limit error, got: {}",
            err
        );
    }

    #[test]
    fn test_check_gate_without_pass_context_unchanged() {
        let executor = setup();

        // Define a normal gate without pass_context
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "test-gate".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "test-gate".to_string(),
                title: "Test Gate".to_string(),
                description: "Test gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "echo \"CTX=${JIT_CONTEXT_FILE:-unset}\"".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "test-gate".to_string())
            .unwrap();

        let result = executor.check_gate(&issue_id, "test-gate").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);
        // JIT_CONTEXT_FILE should not be set when pass_context is false
        assert!(result.stdout.contains("CTX=unset"));
    }

    #[test]
    fn test_check_gate_exposes_linked_documents_via_env_var() {
        // REQ-01 (jit:4af511fd): a gate checker process receives the issue's
        // linked-document list (paths + doc types + labels) via JIT_ISSUE_DOCS,
        // wired end to end through `check_gate` from `issue.documents`.
        let executor = setup();

        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "test-gate".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "test-gate".to_string(),
                title: "Test Gate".to_string(),
                description: "Test gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "echo \"$JIT_ISSUE_DOCS\"".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "test-gate".to_string())
            .unwrap();

        // Link a document via the same accessor `jit doc add` uses.
        executor
            .add_document_reference(
                &issue_id,
                "dev/active/my-plan.md",
                None,
                Some("Implementation Plan"),
                Some("design"),
                true, // skip_scan: the path need not exist on disk for this test
            )
            .unwrap();

        let result = executor.check_gate(&issue_id, "test-gate").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);

        let docs: serde_json::Value = serde_json::from_str(result.stdout.trim())
            .expect("JIT_ISSUE_DOCS should be valid JSON");
        let entries = docs.as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["path"], "dev/active/my-plan.md");
        assert_eq!(entries[0]["doc_type"], "design");
        assert_eq!(entries[0]["label"], "Implementation Plan");
    }

    #[test]
    fn test_check_gate_empty_documents_yields_empty_json_array() {
        // REQ-01 empty case: an issue with no linked documents still gets
        // JIT_ISSUE_DOCS set, to an empty JSON array (never absent/unset).
        let executor = setup();

        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "test-gate".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "test-gate".to_string(),
                title: "Test Gate".to_string(),
                description: "Test gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "echo \"$JIT_ISSUE_DOCS\"".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "test-gate".to_string())
            .unwrap();

        let result = executor.check_gate(&issue_id, "test-gate").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);
        assert_eq!(result.stdout.trim(), "[]");
    }

    #[test]
    fn test_gate_context_includes_enriched_dependencies() {
        let executor = setup();

        // Create two dependency issues
        let dep1 = crate::domain::types::fixture_issue(
            "Setup database schema".to_string(),
            "Create tables".to_string(),
        );
        let dep1_id = dep1.id.clone();
        executor.storage.save_issue(dep1).unwrap();

        let dep2 = crate::domain::types::fixture_issue(
            "Implement auth module".to_string(),
            "OAuth2 flow".to_string(),
        );
        let dep2_id = dep2.id.clone();
        executor.storage.save_issue(dep2).unwrap();

        // Create main issue that depends on both
        let mut main_issue = crate::domain::types::fixture_issue(
            "Build user dashboard".to_string(),
            "Dashboard feature".to_string(),
        );
        main_issue.state = State::InProgress;
        main_issue.dependencies.push(dep1_id.clone());
        main_issue.dependencies.push(dep2_id.clone());
        let main_id = main_issue.id.clone();
        executor.storage.save_issue(main_issue).unwrap();

        // Define a context-aware gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "review".to_string(),
                title: "Code Review".to_string(),
                description: "Review".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "cat $JIT_CONTEXT_FILE".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: Some("Review the dashboard.".to_string()),
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        executor.add_gate(&main_id, "review".to_string()).unwrap();

        let result = executor.check_gate(&main_id, "review").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);

        let context: serde_json::Value =
            serde_json::from_str(&result.stdout).expect("stdout should be valid context JSON");

        let deps = context["issue"]["dependencies"].as_array().unwrap();
        assert_eq!(deps.len(), 2, "Expected 2 enriched dependencies");

        let dep_titles: Vec<&str> = deps.iter().filter_map(|d| d["title"].as_str()).collect();
        assert!(dep_titles.contains(&"Setup database schema"));
        assert!(dep_titles.contains(&"Implement auth module"));
    }

    #[test]
    fn test_check_all_gates_respects_priority_order() {
        let executor = setup();

        // Define 3 auto gates with priorities 30, 10, 20
        let mut registry = executor.storage.load_gate_registry().unwrap();
        for (key, priority) in [("gate-p30", 30u32), ("gate-p10", 10), ("gate-p20", 20)] {
            registry.gates.insert(
                key.to_string(),
                crate::declarations::GateDefinition {
                    version: 1,
                    key: key.to_string(),
                    title: format!("Gate {}", key),
                    description: format!("Priority {}", priority),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "exit 0".to_string(),
                        timeout_seconds: 10,
                        working_dir: None,
                        env: HashMap::new(),
                        pass_context: false,
                        prompt: None,
                        prompt_file: None,
                    }),
                    priority,
                    reserved: HashMap::new(),
                    auto: true,
                    example_integration: None,
                },
            );
        }
        executor.storage.save_gate_registry(&registry).unwrap();

        // Create issue with gates added in priority 30, 10, 20 order
        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "gate-p30".to_string())
            .unwrap();
        executor
            .add_gate(&issue_id, "gate-p10".to_string())
            .unwrap();
        executor
            .add_gate(&issue_id, "gate-p20".to_string())
            .unwrap();

        executor.check_gate(&issue_id, "gate-p30").unwrap();
        executor.check_gate(&issue_id, "gate-p10").unwrap();
        executor.check_gate(&issue_id, "gate-p20").unwrap();

        let (results, _) = executor.get_last_gate_runs_for_issue(&issue_id).unwrap();

        // Results should arrive in priority order: 10, 20, 30
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].gate_key, "gate-p10");
        assert_eq!(results[1].gate_key, "gate-p20");
        assert_eq!(results[2].gate_key, "gate-p30");
    }

    #[test]
    fn test_check_all_gates_stable_sort_same_priority() {
        let executor = setup();

        // Define 3 auto gates all with default priority 100
        let mut registry = executor.storage.load_gate_registry().unwrap();
        for key in ["alpha", "beta", "gamma"] {
            registry.gates.insert(
                key.to_string(),
                crate::declarations::GateDefinition {
                    version: 1,
                    key: key.to_string(),
                    title: format!("Gate {}", key),
                    description: "Same priority".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "exit 0".to_string(),
                        timeout_seconds: 10,
                        working_dir: None,
                        env: HashMap::new(),
                        pass_context: false,
                        prompt: None,
                        prompt_file: None,
                    }),
                    priority: 100,
                    reserved: HashMap::new(),
                    auto: true,
                    example_integration: None,
                },
            );
        }
        executor.storage.save_gate_registry(&registry).unwrap();

        // Add gates in specific insertion order
        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "alpha".to_string()).unwrap();
        executor.add_gate(&issue_id, "beta".to_string()).unwrap();
        executor.add_gate(&issue_id, "gamma".to_string()).unwrap();

        executor.check_gate(&issue_id, "alpha").unwrap();
        executor.check_gate(&issue_id, "beta").unwrap();
        executor.check_gate(&issue_id, "gamma").unwrap();

        let (results, _) = executor.get_last_gate_runs_for_issue(&issue_id).unwrap();

        // Same-priority gates should maintain insertion order
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].gate_key, "alpha");
        assert_eq!(results[1].gate_key, "beta");
        assert_eq!(results[2].gate_key, "gamma");
    }

    #[test]
    fn test_gate_priority_defaults_on_deserialization() {
        // Deserialize Gate JSON without priority field — should default to 100
        let json = r#"{
            "version": 1,
            "key": "old-gate",
            "title": "Old Gate",
            "description": "Pre-priority gate",
            "stage": "postcheck",
            "mode": "manual",
            "auto": false,
            "example_integration": null
        }"#;

        let gate: crate::declarations::GateDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(gate.priority, 100);
    }

    fn make_auto_gate(key: &str, command: &str) -> crate::declarations::GateDefinition {
        crate::declarations::GateDefinition {
            version: 1,
            key: key.to_string(),
            title: key.to_string(),
            description: String::new(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: Some(GateChecker::Exec {
                command: command.to_string(),
                timeout_seconds: 10,
                working_dir: None,
                env: HashMap::new(),
                pass_context: false,
                prompt: None,
                prompt_file: None,
            }),
            priority: 100,
            reserved: HashMap::new(),
            auto: true,
            example_integration: None,
        }
    }

    #[test]
    fn test_get_last_gate_run_returns_none_when_no_runs() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry
            .gates
            .insert("g".to_string(), make_auto_gate("g", "exit 0"));
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::types::fixture_issue("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "g".to_string()).unwrap();

        let result = executor.get_last_gate_run(&issue_id, "g").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_last_gate_run_returns_most_recent() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry
            .gates
            .insert("g".to_string(), make_auto_gate("g", "exit 0"));
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::types::fixture_issue("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "g".to_string()).unwrap();

        let first = executor.check_gate(&issue_id, "g").unwrap();
        // Small sleep so timestamps differ
        std::thread::sleep(std::time::Duration::from_millis(10));
        let second = executor.check_gate(&issue_id, "g").unwrap();

        let last = executor.get_last_gate_run(&issue_id, "g").unwrap().unwrap();
        assert_eq!(last.run_id, second.run_id);
        assert_ne!(last.run_id, first.run_id);
    }

    #[test]
    fn test_get_last_gate_run_filters_by_gate_key() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry
            .gates
            .insert("gate-a".to_string(), make_auto_gate("gate-a", "exit 0"));
        registry
            .gates
            .insert("gate-b".to_string(), make_auto_gate("gate-b", "exit 0"));
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::types::fixture_issue("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "gate-a".to_string()).unwrap();
        executor.add_gate(&issue_id, "gate-b".to_string()).unwrap();

        executor.check_gate(&issue_id, "gate-a").unwrap();
        let b_run = executor.check_gate(&issue_id, "gate-b").unwrap();

        let last_a = executor
            .get_last_gate_run(&issue_id, "gate-a")
            .unwrap()
            .unwrap();
        let last_b = executor
            .get_last_gate_run(&issue_id, "gate-b")
            .unwrap()
            .unwrap();

        assert_eq!(last_a.gate_key, "gate-a");
        assert_eq!(last_b.gate_key, "gate-b");
        assert_eq!(last_b.run_id, b_run.run_id);
        assert_ne!(last_a.run_id, last_b.run_id);
    }

    #[test]
    fn test_get_last_gate_run_shows_failure_details() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "fail-gate".to_string(),
            make_auto_gate("fail-gate", "echo 'oops' && exit 1"),
        );
        executor.storage.save_gate_registry(&registry).unwrap();

        let issue = crate::domain::types::fixture_issue("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor
            .add_gate(&issue_id, "fail-gate".to_string())
            .unwrap();

        executor.check_gate(&issue_id, "fail-gate").unwrap();

        let last = executor
            .get_last_gate_run(&issue_id, "fail-gate")
            .unwrap()
            .unwrap();
        assert_eq!(last.status, GateRunStatus::Failed);
        assert_eq!(last.exit_code, Some(1));
        assert!(
            last.stdout.contains("oops") || last.stderr.contains("oops"),
            "Expected 'oops' in output, got stdout={:?} stderr={:?}",
            last.stdout,
            last.stderr
        );
    }

    // --- Stale-binary check wiring (jit:7446af34) --------------------------
    //
    // `setup()` above uses `InMemoryStorage`, whose root is the placeholder
    // `"."`; `real_repo_root()` is always `None` there, so the stale-binary
    // check never runs — proven by every `InMemoryStorage`-backed test in
    // this file still passing unchanged. These tests instead use a real
    // `JsonFileStorage` rooted at a real (temporary) directory, so
    // `real_repo_root()` resolves and the check actually reaches git.

    /// A `JsonFileStorage`-backed executor with one automated gate `"g"`,
    /// rooted at `repo_root` (which the caller controls: git-initialized or
    /// not). Mirrors `setup()` above but with a real on-disk root.
    fn setup_at(repo_root: &std::path::Path) -> CommandExecutor<crate::storage::JsonFileStorage> {
        std::env::set_var("JIT_TEST_MODE", "1");
        let jit_root = repo_root.join(".jit");
        let storage = crate::storage::JsonFileStorage::new(&jit_root);
        std::fs::create_dir_all(&jit_root).unwrap();
        std::fs::write(
            jit_root.join("config.toml"),
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();

        let layout = crate::storage::discover_repository_layout(repo_root, &jit_root).unwrap();
        let executor = CommandExecutor::new(storage).with_layout(layout);
        executor
            .initialize_fresh_repository(repo_root, &HierarchyTemplate::default(), None)
            .unwrap();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry
            .gates
            .insert("g".to_string(), make_auto_gate("g", "echo ran"));
        executor.storage.save_gate_registry(&registry).unwrap();
        executor
    }

    /// Create an issue requiring gate `"g"` on `executor`.
    fn add_gated_issue(executor: &CommandExecutor<crate::storage::JsonFileStorage>) -> String {
        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        executor.storage.save_issue(issue).unwrap();
        executor.add_gate(&issue_id, "g".to_string()).unwrap();
        issue_id
    }

    /// REQ-03: outside a git repository entirely, the check stays silent and
    /// the checker runs normally.
    #[test]
    fn test_check_gate_stays_silent_outside_git_repository() {
        let temp = tempfile::TempDir::new().unwrap();
        let executor = setup_at(temp.path());
        let issue_id = add_gated_issue(&executor);

        let result = executor.check_gate(&issue_id, "g").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);
    }

    /// REQ-03: an ordinary, unrelated git repository — one that shares no
    /// history with the running binary's build commit, exactly the "installed
    /// release validating a different repository" case — stays silent and the
    /// checker runs normally.
    #[test]
    fn test_check_gate_stays_silent_for_unrelated_git_repository() {
        let temp = tempfile::TempDir::new().unwrap();
        let init = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(temp.path())
            .status()
            .unwrap();
        assert!(init.success());
        let commit = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--allow-empty",
                "-q",
                "-m",
                "unrelated init",
            ])
            .current_dir(temp.path())
            .status()
            .unwrap();
        assert!(commit.success());

        let executor = setup_at(temp.path());
        let issue_id = add_gated_issue(&executor);

        let result = executor.check_gate(&issue_id, "g").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);
    }

    /// Build a scratch git repository whose `HEAD` is one commit PAST the
    /// running test binary's own build commit — fetched from the real jit
    /// workspace this test binary was built from, so the REQ-03 identity
    /// predicate (the build commit is a known commit here) holds for real,
    /// entirely inside the disposable scratch repo (no ref in the real
    /// workspace is read, moved, or written).
    ///
    /// Returns `None` (the caller should skip) when the binary was built
    /// without git (`version_info().git_commit == "unknown"`, e.g. a tarball
    /// build) or any of the local git steps fails for environment reasons.
    fn scratch_repo_stale_against_own_build() -> Option<(tempfile::TempDir, String)> {
        let info = crate::build_info::version_info();
        if info.git_commit == "unknown" {
            eprintln!("SKIP: this binary was built without git; no build commit to compare");
            return None;
        }
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)?
            .to_str()?
            .to_string();

        let temp = tempfile::TempDir::new().ok()?;
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(temp.path())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };

        if !run(&["init", "-q"]) {
            eprintln!("SKIP: git init failed in scratch repo");
            return None;
        }
        if !run(&["fetch", "-q", &workspace_root, info.git_commit]) {
            eprintln!("SKIP: git fetch of the running binary's build commit failed");
            return None;
        }
        if !run(&["checkout", "-q", "FETCH_HEAD"]) {
            eprintln!("SKIP: checkout of the fetched build commit failed");
            return None;
        }
        if !run(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-q",
            "-m",
            "advance past the build commit",
        ]) {
            eprintln!("SKIP: advancing HEAD past the build commit failed");
            return None;
        }
        Some((temp, info.git_commit.to_string()))
    }

    /// REQ-01/02: the running binary's build commit is a known commit in the
    /// repository under validation, but is no longer at `HEAD` — `check_gate`
    /// refuses BEFORE spawning the checker, and no verdict is ever produced or
    /// persisted.
    #[test]
    fn test_check_gate_refuses_stale_binary_and_persists_no_run() {
        let Some((temp, built_from)) = scratch_repo_stale_against_own_build() else {
            return;
        };

        let executor = setup_at(temp.path());
        let issue_id = add_gated_issue(&executor);

        let err = executor.check_gate(&issue_id, "g").expect_err(
            "a binary whose build commit is a known, but no longer current, commit \
             in the repository under review must refuse to run the checker",
        );
        assert!(
            err.downcast_ref::<crate::errors::StaleBinaryError>()
                .is_some(),
            "expected StaleBinaryError, got: {err:?}"
        );
        assert!(
            err.to_string().contains(&built_from),
            "error message should name the build commit: {err}"
        );

        // No verdict was ever produced or persisted (REQ-02).
        assert!(
            executor
                .get_last_gate_run(&issue_id, "g")
                .unwrap()
                .is_none(),
            "no gate run should have been recorded for a refused, stale-binary check"
        );
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert!(
            !issue
                .gates_status
                .get("g")
                .is_some_and(|s| s.status == crate::domain::GateStatus::Passed),
            "the gate must not have been marked passed by a refused, stale-binary check"
        );
    }
}
