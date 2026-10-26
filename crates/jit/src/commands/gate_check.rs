//! Gate checking and execution operations

use super::*;
use crate::declarations::{GateMode, GateStage, REVIEW_PLACEHOLDER_WARNING};
use crate::domain::gate_findings::{
    FindingDisposition, FindingOrigin, FindingSeverity, GateFinding, GateFindings,
};
use crate::domain::{
    Assignee, GateContext, GateEvaluationStability, GateRunResult, GateRunStatus, GateStatus,
};
use crate::errors::TransitionBlockedError;
use crate::gate_execution;
use crate::output::IssueShowResponse;
use std::collections::HashMap;

/// Maximum prompt file size in bytes (~1000 lines of 80 chars).
const MAX_PROMPT_FILE_SIZE: u64 = 100_000;

/// Where an evaluation is allowed to obtain its verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictSource {
    /// Take the verdict of a prior run of this gate whose declared inputs
    /// digest to the same value, executing the checker only when no such run
    /// exists. A gate declaring no inputs has nothing to match, so it executes.
    ReuseUnchangedInputs,
    /// Execute the checker, whatever any prior run recorded.
    Execute,
}

/// Build the record for a verdict taken from `source` rather than derived by
/// executing a checker.
///
/// The verdict, the command behind it, and the checker's structured findings
/// come from the run that produced them; the timing is this record's own, and
/// no exit code is claimed, because no process ran here. The raw report text
/// stays at the source run, which [`GateVerdictOrigin::Reused`] names, rather
/// than being copied into every record that carries the verdict.
fn reused_gate_run(source: &GateRunResult, issue_id: &str, stage: GateStage) -> GateRunResult {
    let started_at = chrono::Utc::now();
    GateRunResult {
        schema_version: crate::domain::GATE_RUN_SCHEMA_VERSION,
        // Repository mutation finalization assigns the durable run identity.
        run_id: String::new(),
        gate_key: source.gate_key.clone(),
        stage,
        issue_id: issue_id.to_string(),
        commit: None,
        branch: None,
        status: source.status,
        started_at,
        completed_at: Some(started_at),
        duration_ms: Some(0),
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        command: source.command.clone(),
        by: Some(Assignee::AUTO_EXECUTOR),
        message: None,
        findings: source.findings.clone(),
        inputs_digest: source.inputs_digest.clone(),
        origin: crate::domain::GateVerdictOrigin::Reused(source.run_id.clone()),
        evaluation_stability: GateEvaluationStability::Stable,
    }
}

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
    validation_view: Option<super::PrecheckValidationView>,
}

struct CapturedGateEvaluation {
    evidence: GateEvaluationEvidence,
    prompt: Option<String>,
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

    let path = VirtualPath::INDEX;
    match image.entry(&path)? {
        RepositoryEntry::File { content, .. } => {
            crate::storage::json::parse_repository_index(content.bytes())
                .context("failed to parse captured issue index")
        }
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
        RepositoryEntry::File { content, .. } => {
            let issue: Issue = serde_json::from_slice(content.bytes())
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

    let path = VirtualPath::GATES;
    match image.entry(&path)? {
        RepositoryEntry::File { content, .. } => {
            crate::declarations::parse_gate_registry(content.bytes())
                .context("failed to parse captured gate registry")
        }
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
            | crate::declarations::GateChecker::RuleValidation { .. }
    )
}

fn required_automated_gate<'a>(
    issue: &Issue,
    registry: &'a crate::declarations::GateRegistry,
    gate_key: &str,
) -> Result<&'a crate::declarations::GateDefinition> {
    if !issue.gates_required.iter().any(|key| key == gate_key) {
        return Err(super::gate::GateNotRequiredError {
            issue_id: issue.id.to_string(),
            gate_key: gate_key.to_string(),
        }
        .into());
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

pub(super) fn gate_state_from_run(result: &GateRunResult) -> GateState {
    let status = match result.status {
        GateRunStatus::Passed => GateStatus::Passed,
        GateRunStatus::Failed | GateRunStatus::Error => GateStatus::Failed,
        _ => GateStatus::Pending,
    };
    GateState {
        status,
        updated_by: result.by.clone(),
        updated_at: chrono::DateTime::default(),
        run_id: Some(result.run_id.clone()),
    }
}

/// Select and sanitize the run `state` was projected from, the gate's latest
/// published run, before it enters checker context.
///
/// Stored results are never mutated. Structured findings replace verbose stdout;
/// legacy unstructured runs retain stdout as their compatibility signal. Stderr is
/// always diagnostic noise and is removed from the context copy.
fn compact_run_history_for_context(
    runs: &[GateRunResult],
    gate_key: &str,
    state: Option<&GateState>,
) -> Vec<GateRunResult> {
    state
        .and_then(|state| state.projected_run(gate_key, runs))
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
                RepositoryEntry::File { content, .. } => Some(
                    serde_json::from_slice::<GateRunResult>(content.bytes()).with_context(
                        || format!("failed to parse captured gate run at {path:?}"),
                    )?,
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
            let path = image
                .layout()
                .classify_repository_relative(path)
                .map_err(|_| {
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
        .filter(|(key, _)| broad_validation || wanted_pinned.contains(*key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let validation_view = broad_validation.then(|| super::PrecheckValidationView {
        listings: image.directory_listings().clone(),
    });
    Ok(CapturedGateEvaluation {
        evidence: GateEvaluationEvidence {
            gate: gate.clone(),
            inputs,
            pinned,
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
    let declarations = crate::repository_state::declarations_from_image(image)?;
    let config = crate::repository_state::assemble_config(image)?;
    let repo_format = config.content_format();
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
    let namespaces = crate::config_manager::namespaces_from_config(&config);
    let hierarchy = crate::domain::type_taxonomy::HierarchyConfig::from_declarations(&namespaces)?;
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
    let declarations = crate::repository_state::declarations_from_image(image)?;
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
    let repo_format = config.content_format();
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
        let namespaces = crate::config_manager::namespaces_from_config(&config);
        let hierarchy =
            crate::domain::type_taxonomy::HierarchyConfig::from_declarations(&namespaces)?;
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
        .map(|finding| {
            let (severity, disposition) = if finding.is_error() {
                (FindingSeverity::High, FindingDisposition::Blocking)
            } else {
                (FindingSeverity::Low, FindingDisposition::Advisory)
            };
            GateFinding::builtin(
                "",
                severity,
                disposition,
                FindingOrigin::IssueImpact,
                format!("[{}] {}", finding.rule, finding.message),
            )
        })
        .collect()
}

/// Convert one whole-repository validation result into the built-in gate finding
/// contract.
///
/// A structural failure or an unloadable rules declaration is a fault the checker
/// cannot judge through, so it returns `Err`; rule findings about content convert through the
/// same severity/disposition mapping as any validation report.
fn gate_findings_from_report(
    result: std::result::Result<
        crate::validation::repository::RepositoryValidationReport,
        crate::validation::repository::RepositoryValidationFailure,
    >,
) -> Result<(Vec<GateFinding>, bool)> {
    let report = result.map_err(|failure| failure.into_error())?;
    if !report
        .passes
        .contains(&crate::validation::repository::RepositoryValidationPass::RulesAndSchemas)
    {
        let causes = report
            .rule_report
            .findings
            .iter()
            .filter(|finding| finding.is_error())
            .map(|finding| finding.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::bail!("rules declaration did not load: {causes}");
    }
    let failed = report.rule_report.has_errors();
    Ok((gate_findings_from_rule_report(&report.rule_report), failed))
}

/// Validate the scope named by the issue's single `<namespace>:<target-id>` label.
///
/// A missing, repeated, or empty target is a configuration fault.
fn label_target_report(
    image: &crate::repository_state::RepositoryImage,
    issue_id: &str,
    label_namespace: &str,
) -> Result<crate::validation::report::RuleReport> {
    let issues = super::captured_active_issues(image)?;
    let issue = issues
        .iter()
        .find(|issue| issue.id == issue_id)
        .ok_or_else(|| crate::storage::IssueNotFoundError::new(issue_id))?;
    let prefix = format!("{label_namespace}:");
    let targets: Vec<&str> = issue
        .labels
        .iter()
        .filter_map(|label| label.strip_prefix(&prefix))
        .collect();
    if label_namespace.is_empty() {
        anyhow::bail!("label_namespace is empty");
    }
    let [target] = targets[..] else {
        anyhow::bail!(
            "expected exactly one non-empty '{label_namespace}:<target-id>' label on issue {issue_id}, found {}",
            targets.len()
        );
    };
    if target.is_empty() {
        anyhow::bail!(
            "expected exactly one non-empty '{label_namespace}:<target-id>' label on issue {issue_id}, found an empty target"
        );
    }
    let target = super::resolve_issue_from_capture(&issues, &IssueIdPrefix::from_str(target)?)?;
    captured_scope_rule_report(image, &target)
}

/// The `error` run of a built-in checker that could not judge the repository,
/// distinct from a `failed` verdict on its content.
fn builtin_fault_run(
    gate_key: &str,
    issue_id: &str,
    stage: GateStage,
    command: &str,
    started_at: chrono::DateTime<chrono::Utc>,
    start: std::time::Instant,
    fault: &anyhow::Error,
) -> GateRunResult {
    let summary = format!("Built-in checker could not judge the repository: {fault:#}");
    GateRunResult {
        schema_version: crate::domain::GATE_RUN_SCHEMA_VERSION,
        // Repository mutation finalization assigns the durable run identity.
        run_id: String::new(),
        gate_key: gate_key.to_string(),
        stage,
        issue_id: issue_id.to_string(),
        commit: None,
        branch: None,
        status: GateRunStatus::Error,
        started_at,
        completed_at: Some(chrono::Utc::now()),
        duration_ms: Some(start.elapsed().as_millis() as u64),
        exit_code: Some(gate_execution::CHECKER_TEMPFAIL_EXIT),
        stdout: String::new(),
        stderr: summary,
        command: command.to_string(),
        by: Some(Assignee::AUTO_EXECUTOR),
        message: None,
        findings: None,
        inputs_digest: None,
        origin: crate::domain::GateVerdictOrigin::Executed,
        evaluation_stability: crate::domain::GateEvaluationStability::Stable,
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Repository root used as the checker working directory and as the `git`
    /// context for stamping [`GateRunResult::commit`](crate::domain::GateRunResult).
    ///
    /// File-backed stores use the explicit repository layout, never the selected
    /// data-root parent. In-memory tests retain the current-directory fallback.
    pub(crate) fn checker_repo_root(&self) -> Result<std::path::PathBuf> {
        if !self.storage.is_file_backed() {
            return std::env::current_dir().context("Failed to resolve checker working directory");
        }
        self.require_layout()
            .map(|layout| layout.worktree_root().to_path_buf())
    }

    /// The explicit on-disk repository layout, or `None` for a non-file-backed
    /// store.
    ///
    /// Distinct from [`checker_repo_root`](Self::checker_repo_root), which
    /// falls back to the current working directory so the checker process
    /// always has SOME directory to run in. A caller that must describe a real
    /// repository — digesting a gate's declared inputs, for instance — takes
    /// this instead, so storage naming no real repository yields `None` rather
    /// than whatever repository the host process happens to be executing
    /// inside.
    pub(crate) fn real_repo_layout(&self) -> Option<crate::repository_state::RepositoryLayout> {
        if !self.storage.is_file_backed() {
            return None;
        }
        self.require_layout().ok()
    }

    /// Capture what evaluating `gate_key` reads, in one round that first
    /// binds `requested` to its issue, and return the image with the bound id.
    fn capture_gate_evaluation_image(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
        prefix: &crate::domain::identity::IssueIdPrefix,
        gate_key: &str,
        run_paths: &[crate::repository_state::VirtualPath],
    ) -> Result<Option<(crate::repository_state::RepositoryImage, String)>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::declarations::GateChecker;
        use crate::repository_state::{CaptureSpec, VirtualPath};
        use std::collections::BTreeMap;

        let mut candidates = Vec::new();
        let spec = CaptureSpec::new(
            [VirtualPath::INDEX],
            super::validate::MUTATION_CAPTURE_BUDGET,
        )?;
        let Some(bound) = super::capture_discovering(session, spec, |image, spec| {
            candidates = captured_repository_index(image)?
                .all_ids
                .into_iter()
                .filter(|id| prefix.matches(id))
                .collect();
            Ok(spec.discover_paths(
                candidates
                    .iter()
                    .map(|id| VirtualPath::data(format!("issues/{id}.json")))
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            )?)
        })?
        else {
            return Ok(None);
        };
        let issues = candidates
            .iter()
            .map(|id| {
                captured_issue_record(&bound, id)?
                    .ok_or_else(|| crate::storage::IssueNotFoundError::new(id).into())
            })
            .collect::<Result<Vec<_>>>()?;
        let target_id = crate::storage::resolve_issue_id_among(prefix, &issues)?;
        let mut paths = vec![
            VirtualPath::INDEX,
            VirtualPath::data(format!("issues/{target_id}.json"))?,
            VirtualPath::GATES,
            VirtualPath::EVENTS,
            VirtualPath::GATE_RUNS,
        ];
        paths.extend(run_paths.iter().cloned());
        let mut spec = bound.capture_spec().clone();
        spec.discover_paths(paths)?;
        let Some(initial) = capture_or_retry(session.extend(bound, spec))? else {
            return Ok(None);
        };
        let issue = captured_bound_issue(&initial, &target_id)?;
        let registry = captured_gate_registry(&initial)?;
        let gate = required_automated_gate(&issue, &registry, gate_key)?;
        let checker = gate
            .checker
            .as_ref()
            .ok_or_else(|| anyhow!("Gate '{}' has no checker configured", gate_key))?;
        if checker_needs_validation_image(checker) {
            return Ok(self
                .capture_proposed_base(session, &BTreeMap::new(), run_paths, Some(initial))?
                .map(|image| (image, target_id)));
        }
        let layout = initial.layout().clone();
        let image = super::grow_discovering(session, initial, |image, spec| {
            spec.discover_paths(
                issue
                    .dependencies
                    .iter()
                    .map(|id| VirtualPath::data(format!("issues/{id}.json")))
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            )?;
            if matches!(checker, GateChecker::Exec { .. }) {
                for document in &issue.documents {
                    let requests =
                        crate::document::DocumentReferenceRequests::for_reference(document)?;
                    if let Some(worktree) = requests.worktree() {
                        spec.discover_paths([worktree.clone()])?;
                    }
                    let (revision, path) = requests.pinned();
                    spec.discover_pinned(revision, path)?;
                }
            }
            if let GateChecker::Exec {
                pass_context: true,
                prompt_file: Some(path),
                ..
            } = checker
            {
                let prompt = layout.classify_repository_relative(path).map_err(|_| {
                    anyhow!("prompt_file '{}' resolves outside the repository", path)
                })?;
                spec.discover_paths([prompt])?;
            }
            if super::checker_consumes_run_history(checker) {
                super::discover_gate_run_results(image, spec)?;
            }
            Ok(())
        })?;
        Ok(image.map(|image| (image, target_id)))
    }

    /// Check a single gate for an issue
    ///
    /// Runs the configured native or `exec` checker for an automated gate,
    /// updates the issue status, and returns the run result. When an `exec`
    /// checker has `pass_context: true`, builds structured context (issue data,
    /// gate definition, prompt, run history) and passes it to the checker
    /// process via a temp file.
    ///
    /// The checker executes at most once per invocation. Its verdict is recorded against the
    /// image captured before the checker started; if the repository changes while it runs, the
    /// record reports that drift.
    ///
    /// Equivalent to [`check_gate_with`](Self::check_gate_with) under
    /// [`VerdictSource::ReuseUnchangedInputs`]: a gate declaring inputs whose
    /// digest a prior run of that gate already recorded takes that run's
    /// verdict instead of executing its checker again.
    pub fn check_gate(&self, issue_id: &str, gate_key: &str) -> Result<GateRunResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.check_gate_with(issue_id, gate_key, VerdictSource::ReuseUnchangedInputs)
    }

    /// Check a single gate, choosing whether a recorded verdict may stand in
    /// for executing the checker.
    ///
    /// See [`check_gate`](Self::check_gate) for what an evaluation does;
    /// `source` governs only whether reuse is considered.
    /// [`VerdictSource::Execute`] runs the checker whatever any prior run
    /// recorded, which is what an explicit re-run request asks for. The checker still executes
    /// at most once during this invocation: publication races retry the cheap record operation
    /// without re-running it. The verdict is recorded against the image captured before the
    /// checker started, with any post-run input drift reported on the record.
    pub fn check_gate_with(
        &self,
        issue_id: &str,
        gate_key: &str,
        source: VerdictSource,
    ) -> Result<GateRunResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, MutationIntent};

        let requested = IssueIdPrefix::from_str(issue_id)?;
        let layout = self.require_layout()?;
        let mutation = super::production_mutation_context();
        let run_id = mutation.identifier_at(0);
        let run_paths = super::gate_run_record_paths(&mutation, 1)?;
        let (target_id, image, issue, issues, registry, runs, captured) =
            with_mutation_attempts("gate evaluation inputs", || {
                let mut session = self.storage.open_mutation_session(layout.clone())?;
                let Some((image, target_id)) = self.capture_gate_evaluation_image(
                    session.as_mut(),
                    &requested,
                    gate_key,
                    &run_paths,
                )?
                else {
                    return Ok(AttemptOutcome::Retry);
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
                Ok(AttemptOutcome::Done((
                    target_id, image, issue, issues, registry, runs, captured,
                )))
            })?;

        let gate = registry
            .gates
            .get(gate_key)
            .ok_or_else(|| crate::storage::GateNotFoundError::single(gate_key))?;
        // Digest the declared inputs BEFORE the checker runs: the value must name the content
        // the verdict was derived over, not what the checker left behind.
        let digest = self.declared_inputs_digest(gate)?;
        let reused = match (source, digest.as_ref()) {
            (VerdictSource::ReuseUnchangedInputs, Some(digest)) => {
                self.storage.find_reusable_gate_run(gate_key, digest)?
            }
            _ => None,
        };
        let mut result = match reused {
            Some(source_run) => reused_gate_run(&source_run, &issue.id, gate.stage),
            None => self.execute_captured_gate(CapturedGateExecution {
                image: &image,
                issue: &issue,
                issues: &issues,
                gate_key,
                gate,
                runs: &runs.results,
                prompt: captured.prompt.as_deref(),
            })?,
        };
        result.inputs_digest = digest;

        with_mutation_attempts("gate evaluation record", || {
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some((final_image, _)) = self.capture_gate_evaluation_image(
                session.as_mut(),
                &IssueIdPrefix::from(&target_id),
                gate_key,
                &run_paths,
            )?
            else {
                return Ok(AttemptOutcome::Retry);
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
            result.evaluation_stability = if final_captured.evidence == captured.evidence {
                GateEvaluationStability::Stable
            } else {
                GateEvaluationStability::InputsMoved
            };
            let state = gate_state_from_run(&result);
            let by = result.by.clone();
            final_issue.gates_status.insert(gate_key.to_string(), state);
            let event = if result.status == GateRunStatus::Passed {
                EventBody::draft_gate_passed(target_id.clone(), gate_key.to_string(), by)
            } else {
                EventBody::draft_gate_failed(target_id.clone(), gate_key.to_string(), by)
            };
            let intents = [
                MutationIntent::UpdateIssue {
                    issue: Box::new(final_issue),
                },
                MutationIntent::RecordGateRun {
                    draft: Box::new(result.clone()),
                },
                MutationIntent::RecordEvent {
                    phase: 1,
                    event: Box::new(event),
                },
            ];
            let plan = finalize(&layout, &final_image, &mutation, &intents)?;
            if let AttemptOutcome::Retry = classify_apply(session.apply(&plan), ())? {
                return Ok(AttemptOutcome::Retry);
            }
            let mut recorded_result = result.clone();
            recorded_result.run_id = run_id.clone();
            Ok(AttemptOutcome::Done(recorded_result))
        })
    }

    /// Digest the repository files `gate` declares its checker reads.
    ///
    /// The digest is bound to the gate's own declaration
    /// ([`GateDefinition::verdict_declaration`](crate::declarations::GateDefinition::verdict_declaration)),
    /// so an edit to the checker yields a different digest over identical
    /// files and cannot inherit a verdict the edited checker never produced.
    ///
    /// `None` when the gate declares no inputs — the case that keeps a checker
    /// whose verdict is not a function of repository content running every
    /// time — or when this executor is not rooted at a real worktree, where
    /// there is no content to digest and so nothing a later run could prove
    /// identical.
    fn declared_inputs_digest(
        &self,
        gate: &crate::declarations::GateDefinition,
    ) -> Result<Option<crate::domain::repository_inputs::InputsDigest>> {
        let (Some(inputs), Some(layout)) = (gate.inputs.as_ref(), self.real_repo_layout()) else {
            return Ok(None);
        };
        let declaration = gate
            .verdict_declaration()
            .context("Failed to encode the gate declaration the digest is bound to")?;
        gate_execution::digest_declared_inputs(
            layout.worktree_root(),
            inputs,
            &layout.tracker_written_paths(),
            &declaration,
        )
        .map(Some)
    }

    /// Execute a built-in checker against one exact repository image.
    fn execute_builtin_checker_with_repository_view(
        &self,
        gate_key: &str,
        issue_id: &IssueId,
        stage: GateStage,
        checker: &crate::declarations::GateChecker,
        repository_image: &crate::repository_state::RepositoryImage,
    ) -> Result<GateRunResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::declarations::GateChecker;

        let started_at = chrono::Utc::now();
        let start = std::time::Instant::now();

        let command = match checker {
            GateChecker::RepositoryValidation => "builtin:repository_validation",
            GateChecker::IssueValidation => "builtin:issue_validation",
            GateChecker::RuleValidation { .. } => "builtin:rule_validation",
            GateChecker::LabelTargetValidation { .. } => "builtin:label_target_validation",
            GateChecker::ReviewPlaceholder => "builtin:review_placeholder",
            GateChecker::Exec { .. } => unreachable!("exec checker is dispatched separately"),
        };
        // An `Err` is a fault: the checker could not judge the repository.
        // An `Ok` carries a verdict on the content.
        let judged: Result<(Vec<GateFinding>, bool)> = match checker {
            GateChecker::RepositoryValidation => gate_findings_from_report(
                crate::validation::repository::validate_repository(repository_image),
            ),
            GateChecker::IssueValidation => captured_issue_rule_report(repository_image, issue_id)
                .map(|report| (gate_findings_from_rule_report(&report), report.has_errors())),
            GateChecker::RuleValidation { rule } => super::validate::rule_validation_outcome(
                repository_image,
                &IssueIdPrefix::from(issue_id),
                rule,
            )
            .and_then(|outcome| outcome.map_err(|fault| anyhow!(fault.message)))
            .map(|report| (gate_findings_from_rule_report(&report), report.has_errors())),
            GateChecker::LabelTargetValidation { label_namespace } => {
                label_target_report(repository_image, issue_id, label_namespace)
                    .map(|report| (gate_findings_from_rule_report(&report), report.has_errors()))
            }
            GateChecker::ReviewPlaceholder => Ok((
                vec![GateFinding::builtin(
                    "review-placeholder",
                    FindingSeverity::High,
                    FindingDisposition::Advisory,
                    FindingOrigin::PreExisting,
                    REVIEW_PLACEHOLDER_WARNING,
                )],
                false,
            )),
            GateChecker::Exec { .. } => unreachable!("exec checker is dispatched separately"),
        };
        let (mut findings, explicit_failure) = match judged {
            Ok(judged) => judged,
            Err(fault) => {
                return Ok(builtin_fault_run(
                    gate_key, issue_id, stage, command, started_at, start, &fault,
                ))
            }
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
                    let label = if finding.is_blocking() {
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
            by: Some(Assignee::AUTO_EXECUTOR),
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
            // Stamped by the evaluator, which holds the gate declaration the
            // digest is taken over.
            inputs_digest: None,
            origin: crate::domain::GateVerdictOrigin::Executed,
            evaluation_stability: crate::domain::GateEvaluationStability::Stable,
        })
    }

    /// Return the latest published run of `gate_key` on an issue: the run its
    /// recorded gate status was projected from, so this view and readiness agree.
    ///
    /// Returns `Ok(None)` if no run has been recorded since the gate became
    /// required.
    pub fn get_last_gate_run(
        &self,
        issue_id: &str,
        gate_key: &str,
    ) -> Result<Option<GateRunResult>> {
        let full_id = self
            .storage
            .resolve_issue_id(&IssueIdPrefix::from_str(issue_id)?)?;
        let issue = self.storage.load_issue(&full_id)?;
        self.projected_gate_run(&issue, gate_key)
    }

    /// Load the run `issue`'s recorded status for `gate_key` names.
    fn projected_gate_run(&self, issue: &Issue, gate_key: &str) -> Result<Option<GateRunResult>> {
        issue
            .gates_status
            .get(gate_key)
            .and_then(|state| state.run_id.as_deref())
            .map(|run_id| self.storage.load_gate_run_result(run_id))
            .transpose()
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
            run_history: compact_run_history_for_context(
                input.runs,
                input.gate_key,
                input.issue.gates_status.get(input.gate_key),
            ),
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
        let repo_root = self.checker_repo_root()?;
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

    /// Return the latest published run for each registered required gate on an
    /// issue, an automated gate's checker run or a manual gate's attestation, as
    /// [`Self::get_last_gate_run`] selects it.
    ///
    /// Results are ordered by gate priority, preserving insertion order for ties.
    /// The second vector contains the gate keys that have no recorded run.
    pub fn get_last_gate_runs_for_issue(
        &self,
        issue_id: &str,
    ) -> Result<(Vec<GateRunResult>, Vec<String>)> {
        let full_id = self
            .storage
            .resolve_issue_id(&IssueIdPrefix::from_str(issue_id)?)?;
        let issue = self.storage.load_issue(&full_id)?;
        let registry = self.storage.load_gate_registry()?;

        // Sort by priority (stable sort preserves insertion order for ties)
        let mut gates: Vec<_> = issue
            .gates_required
            .iter()
            .filter_map(|key| registry.gates.get(key).map(|g| (key, g)))
            .collect();
        gates.sort_by_key(|(_, gate)| gate.priority);

        gates.into_iter().try_fold(
            (Vec::new(), Vec::new()),
            |(mut results, mut not_run), (gate_key, _)| {
                match self.projected_gate_run(&issue, gate_key)? {
                    Some(result) => results.push(result),
                    None => not_run.push(gate_key.clone()),
                }
                Ok((results, not_run))
            },
        )
    }

    /// Return the per-issue readiness status of every REQUIRED gate on an issue
    /// (automated AND manual), ordered by gate priority (insertion order for
    /// ties).
    ///
    /// The authoritative status is the issue's recorded `gates_status`, which
    /// each gate run — an automated gate's checker run or a manual gate's
    /// attestation — writes together with its run record and names that run. A gate with no
    /// recorded state is reported as [`GateStatus::Pending`]. `status-all`
    /// folds these statuses into its green/not-green readiness verdict.
    pub fn get_required_gate_statuses_for_issue(
        &self,
        issue_id: &str,
    ) -> Result<Vec<(String, GateStatus)>> {
        let full_id = self
            .storage
            .resolve_issue_id(&IssueIdPrefix::from_str(issue_id)?)?;
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
                        .insert(gate_key.clone(), gate_state_from_run(&result));
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

    /// Run all postchecks for an issue.
    ///
    /// Production reachability has one direct edge: the `State::Gated` branch of
    /// [`IssueOps::update_issue_state`]. The shipped CLI's only call to that
    /// state-only method is `issue reject`, with `State::Rejected`, so current CLI
    /// dispatch does not enter this loop; library callers explicitly requesting
    /// `State::Gated` do.
    ///
    /// Runs all automated postchecks and auto-transitions to Done if all pass. A
    /// checker's ordinary failing verdict is a successful [`GateRunResult`] with
    /// [`GateRunStatus::Failed`], so it remains recorded and leaves the issue Gated.
    /// An evaluation or evidence-persistence error is returned to the caller.
    pub(crate) fn run_postchecks(&self, issue_id: &str) -> Result<()>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let full_id = self
            .storage
            .resolve_issue_id(&IssueIdPrefix::from_str(issue_id)?)?;
        let issue = self.storage.load_issue(&full_id)?;
        let registry = self.storage.load_gate_registry()?;

        for gate_key in &issue.gates_required {
            if let Some(gate) = registry.gates.get(gate_key) {
                if gate.stage == GateStage::Postcheck && gate.mode == GateMode::Auto {
                    self.check_gate(&full_id, gate_key)?;
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
    use crate::commands::test_helpers::{seed_gate_registry, seed_issue, seed_repo_file};
    use crate::commands::{CommandExecutor, GateUpdate};
    use crate::declarations::{GateChecker, GateMode, GateStage};
    use crate::domain::gate_findings::GateFindings;
    use crate::domain::{GateRunResult, GateRunStatus, State, GATE_RUN_SCHEMA_VERSION};
    use crate::storage::{InMemoryStorage, IssueStore, JsonFileStorage, MutationPoint, RedoPoint};
    use crate::test_utils::document_reference;
    use chrono::{TimeZone, Utc};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    struct FailNthPublication {
        remaining: Mutex<usize>,
    }

    impl FailNthPublication {
        fn new(nth: usize) -> Arc<Self> {
            Arc::new(Self {
                remaining: Mutex::new(nth),
            })
        }
    }

    impl crate::storage::MutationInterruptionInjector for FailNthPublication {
        fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
            if point != &MutationPoint::Redo(RedoPoint::BeforeApply) {
                return Ok(());
            }

            let mut remaining = self.remaining.lock().unwrap();
            if *remaining == 0 {
                return Ok(());
            }
            *remaining -= 1;
            if *remaining == 0 {
                return Err(std::io::Error::other(
                    "injected postcheck persistence failure",
                ));
            }
            Ok(())
        }
    }

    fn add_builtin_gate(
        executor: &CommandExecutor<InMemoryStorage>,
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        issue.labels = labels;
        issue.gates_required.push(gate_key.to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        issue_id
    }

    fn setup() -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();
        // Create config with enforcement off for test backward compatibility
        let config_toml = r#"
[worktree]
enforce_leases = "off"
"#;
        storage.add_data_file("config.toml", config_toml);

        crate::commands::test_helpers::memory_executor(storage)
    }

    const RULE_VALIDATION_TEST_RULES: &str = r#"
[[rules]]
name = "coverage-preview"
when = { type = "breakdown" }
severity = "error"
assert = { label-coverage = { marker = "[hard]", id-pattern = "REQ-[0-9]+", satisfies-namespace = "satisfies", child-link = "dependencies", container-from-label = "brackets" } }

[[rules]]
name = "unrelated-local-failure"
when = { type = "task" }
severity = "error"
assert = { require-label = { label = "required:*", min = 1 } }

[[rules]]
name = "unrelated-graph-failure"
when = { type = "epic" }
severity = "error"
assert = { label-coverage = { marker = "[hard]", child-link = "any" } }

[[rules]]
name = "unrelated-label-reference"
when = { type = "task" }
severity = "error"
assert = { label-reference = { from = "satisfies", to = "req" } }
"#;

    fn seed_rule_validation_rules(executor: &CommandExecutor<InMemoryStorage>) {
        std::fs::write(
            executor.storage.root().join("rules.toml"),
            RULE_VALIDATION_TEST_RULES,
        )
        .unwrap();
        seed_repo_file(
            &executor.storage,
            ".jit/rules.toml",
            RULE_VALIDATION_TEST_RULES,
        );
        let rules_path = executor
            .storage
            .repository_layout()
            .resolve(&crate::repository_state::VirtualPath::RULES)
            .unwrap();
        std::fs::create_dir_all(rules_path.parent().unwrap()).unwrap();
        std::fs::write(rules_path, RULE_VALIDATION_TEST_RULES).unwrap();
    }

    fn seed_unrelated_manual_gate(executor: &CommandExecutor<InMemoryStorage>) {
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "unrelated-pending".to_string(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "unrelated-pending".to_string(),
                title: "Unrelated pending gate".to_string(),
                description: "A deliberately unpassed unrelated gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
                priority: 100,
                reserved: HashMap::new(),
                inputs: None,
            },
        );
        seed_gate_registry(&executor.storage, &registry);
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
            status: GateRunStatus::Failed,
            started_at: Utc.timestamp_opt(started_at, 0).unwrap(),
            completed_at: Some(Utc.timestamp_opt(started_at + 1, 0).unwrap()),
            duration_ms: Some(1_000),
            exit_code: Some(1),
            stdout: format!("stdout-{run_id}"),
            stderr: format!("stderr-{run_id}"),
            command: "review".to_string(),
            by: Some("agent:reviewer".parse().unwrap()),
            message: Some("review complete".to_string()),
            findings,
            inputs_digest: None,
            origin: crate::domain::GateVerdictOrigin::Executed,
            evaluation_stability: crate::domain::GateEvaluationStability::Stable,
        }
    }

    #[test]
    fn test_compact_run_history_selects_the_run_the_gate_status_names() {
        let named = prior_run("named", "review", 20, None);
        let runs = vec![
            named.clone(),
            prior_run("other-gate", "cargo-ci", 40, None),
            prior_run("started-later", "review", 30, None),
            prior_run("oldest", "review", 10, None),
        ];

        let compact = compact_run_history_for_context(
            &runs,
            "review",
            Some(&super::gate_state_from_run(&named)),
        );

        assert_eq!(compact.len(), 1);
        assert_eq!(compact[0].run_id, named.run_id);
        assert_eq!(compact[0].stdout, named.stdout);
        assert!(compact[0].stderr.is_empty());
        assert_eq!(runs[0], named, "source run must remain unchanged");
    }

    #[test]
    fn test_compact_run_history_prefers_structured_findings_over_stdout() {
        let findings = GateFindings {
            verdict: "fail".to_string(),
            summary: "one defect".to_string(),
            findings: Vec::new(),
        };
        let source = prior_run("structured", "review", 10, Some(findings.clone()));

        let compact = compact_run_history_for_context(
            std::slice::from_ref(&source),
            "review",
            Some(&super::gate_state_from_run(&source)),
        );

        assert_eq!(compact.len(), 1);
        assert_eq!(compact[0].findings, Some(findings));
        assert!(compact[0].stdout.is_empty());
        assert!(compact[0].stderr.is_empty());
        assert_eq!(source.stdout, "stdout-structured");
        assert_eq!(source.stderr, "stderr-structured");
    }

    #[test]
    fn test_compact_run_history_is_empty_without_matching_runs() {
        let run = prior_run("cargo", "cargo-ci", 10, None);
        let state = super::gate_state_from_run(&run);

        assert!(compact_run_history_for_context(&[run], "review", Some(&state)).is_empty());
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
        let mut issue = crate::domain::fixture_issue("Test".into(), String::new());
        issue.id = "issue-1".into();
        issue.gates_required = vec!["first".into(), "second".into()];
        let mut run = prior_run("", "first", 10, None);
        run.status = GateRunStatus::Passed;
        issue
            .gates_status
            .insert("first".into(), super::gate_state_from_run(&run));

        let second = issue.gates_status.get("second").cloned();
        let mut view = serde_json::to_value(crate::output::IssueShowResponse::from_issue(
            issue,
            Vec::new(),
            std::slice::from_ref(&run),
        ))
        .unwrap();
        omit_current_gate_projection(&mut view, "second");

        assert_eq!(view["gates"][0]["status"], "passed");
        assert!(view["gates"][0]["last_run_at"].is_string());
        assert!(compact_run_history_for_context(&[run], "second", second.as_ref()).is_empty());
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue with gate
        let issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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

    /// REQ-04 (jit:7ca9a0dd): the evaluation publishes the gate run, the issue's
    /// gate status, and the event as one coupled plan before it returns, so a
    /// caller that sees success can read all three. A path that reported success
    /// ahead of any one of them would fail here.
    #[test]
    fn test_check_gate_publishes_run_status_and_event_before_returning() {
        let executor = setup();
        let issue_id = add_builtin_gate(
            &executor,
            "durability-check",
            GateChecker::Exec {
                command: "exit 0".to_string(),
                timeout_seconds: 10,
                working_dir: None,
                env: HashMap::new(),
                pass_context: false,
                prompt: None,
                prompt_file: None,
            },
            Vec::new(),
        );

        let result = executor.check_gate(&issue_id, "durability-check").unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);

        let runs = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap();
        assert!(
            runs.iter().any(|run| run.run_id == result.run_id
                && run.gate_key == "durability-check"
                && run.status == GateRunStatus::Passed),
            "the returned run must already be recorded: {runs:?}"
        );
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().gates_status["durability-check"].status,
            crate::domain::GateStatus::Passed,
            "the issue must already carry the passing verdict"
        );
        assert!(
            executor
                .storage
                .read_events()
                .unwrap()
                .iter()
                .any(|event| matches!(
                    event.body(),
                    crate::domain::EventBody::GatePassed { issue_id: subject, gate_key, .. }
                        if subject == &issue_id && gate_key == "durability-check"
                )),
            "the evaluation must already have logged its gate-passed event"
        );
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

    fn late_repository_rule(taxonomy: &crate::test_taxonomy::TestTaxonomy) -> String {
        format!(
            r#"
[[rules]]
name = "planned-task-needs-summary"
when = {{ type = "{}" }}
severity = "error"
enforce = false
assert = {{ require-section = {{ heading = "Summary" }} }}
"#,
            taxonomy.type_at_level(4)
        )
    }

    fn setup_file_repository() -> (tempfile::TempDir, CommandExecutor<JsonFileStorage>, String) {
        let repo = tempfile::tempdir().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        std::fs::create_dir_all(storage.root()).unwrap();
        std::fs::write(
            storage.root().join("config.toml"),
            format!(
                "[worktree]\nenforce_leases = \"off\"\n\n{}",
                taxonomy.config_fragment()
            ),
        )
        .unwrap();
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        let executor = CommandExecutor::new(storage).with_layout(layout);
        executor
            .initialize_fresh_repository(repo.path(), None)
            .unwrap();
        executor
            .define_gate(
                "not-a-reserved-repository-key".to_string(),
                "Portable check".to_string(),
                "Portable check".to_string(),
                GateStage::Postcheck,
                GateMode::Auto,
                Some(GateChecker::RepositoryValidation),
                100,
            )
            .unwrap();
        executor
            .define_gate(
                "editable-review".to_string(),
                "Portable check".to_string(),
                "Portable check".to_string(),
                GateStage::Postcheck,
                GateMode::Auto,
                Some(GateChecker::ReviewPlaceholder),
                100,
            )
            .unwrap();
        let (issue_id, _) = executor
            .issues()
            .create_issue(
                "Test".to_string(),
                "Test".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                vec![format!("type:{}", taxonomy.type_at_level(4))],
                None,
                None,
                false,
            )
            .unwrap();
        executor
            .add_gate(&issue_id, "not-a-reserved-repository-key".to_string())
            .unwrap();
        (repo, executor, issue_id)
    }

    #[test]
    fn test_captured_issue_rule_report_preserves_invalid_hierarchy_error() {
        let (_repo, executor, issue_id) = setup_file_repository();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::invalid_hierarchy_config_fragment()
        );
        std::fs::write(executor.storage().root().join("config.toml"), &config).unwrap();
        let image = executor
            .capture_validation_image_with(&std::collections::BTreeMap::new())
            .unwrap();

        let error = super::captured_issue_rule_report(&image, &issue_id)
            .expect_err("invalid hierarchy must fail gate rule evaluation");

        assert_eq!(
            error.downcast_ref::<crate::domain::type_taxonomy::ConfigError>(),
            Some(&crate::domain::type_taxonomy::ConfigError::InvalidLevel(0))
        );
    }

    /// A file-backed repository whose declared gate inputs live under
    /// `workspace/`, with an `exec` checker that appends one byte per execution
    /// to a log OUTSIDE every declared root — so executions can be counted
    /// without the counting itself changing the digest.
    struct ReuseFixture {
        repo: tempfile::TempDir,
        executor: CommandExecutor<JsonFileStorage>,
        gate_key: String,
    }

    impl ReuseFixture {
        /// `roots`/`exclude` are the gate's declared inputs; `None` declares
        /// none. `verdict_exit` is the checker's exit status.
        fn new(declaration: Option<(&[&str], &[&str])>, verdict_exit: u8) -> Self {
            let repo = tempfile::tempdir().expect("a temp repository");
            let storage = JsonFileStorage::new(repo.path().join(".jit"));
            let taxonomy = crate::test_taxonomy::test_taxonomy();
            std::fs::create_dir_all(storage.root()).expect("create data root");
            std::fs::write(
                storage.root().join("config.toml"),
                format!(
                    "[worktree]\nenforce_leases = \"off\"\n\n{}",
                    taxonomy.config_fragment()
                ),
            )
            .expect("write config");
            let layout = crate::storage::discover_repository_layout(repo.path(), storage.root())
                .expect("a repository layout");
            let executor = CommandExecutor::new(storage).with_layout(layout);
            executor
                .initialize_fresh_repository(repo.path(), None)
                .expect("a fresh repository");

            std::fs::create_dir_all(repo.path().join("workspace/src")).expect("create workspace");
            std::fs::write(repo.path().join("workspace/src/lib.rs"), "fn main() {}\n")
                .expect("write source");
            std::fs::write(repo.path().join("elsewhere.txt"), "unread\n").expect("write outsider");

            let gate_key = "workspace-check".to_string();
            let log = repo.path().join("executions.log");
            let mut registry = executor
                .storage
                .load_gate_registry()
                .expect("the seeded registry");
            registry.gates.insert(
                gate_key.clone(),
                crate::declarations::GateDefinition {
                    version: 1,
                    key: gate_key.clone(),
                    title: "Workspace check".to_string(),
                    description: "Workspace check".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: format!(
                            "printf x >> '{}'; exit {verdict_exit}",
                            log.to_string_lossy()
                        ),
                        timeout_seconds: 30,
                        working_dir: None,
                        env: HashMap::new(),
                        pass_context: false,
                        prompt: None,
                        prompt_file: None,
                    }),
                    inputs: declaration.map(|(roots, exclude)| {
                        crate::domain::repository_inputs::RepositoryInputs::parse(roots, exclude)
                            .expect("a declarable input set")
                    }),
                    priority: 100,
                    reserved: HashMap::new(),
                },
            );
            let bytes = crate::declarations::serialize_gate_registry(&registry)
                .expect("the registry serializes");
            std::fs::write(repo.path().join(".jit/gates.toml"), bytes).expect("write registry");

            Self {
                repo,
                executor,
                gate_key,
            }
        }

        /// A new issue requiring the gate under test.
        fn issue(&self) -> String {
            let taxonomy = crate::test_taxonomy::test_taxonomy();
            let (issue_id, _) = self
                .executor
                .issues()
                .create_issue(
                    "Test".to_string(),
                    "Test".to_string(),
                    crate::domain::Priority::Normal,
                    Vec::new(),
                    vec![format!("type:{}", taxonomy.type_at_level(4))],
                    None,
                    None,
                    false,
                )
                .expect("an issue");
            self.executor
                .add_gate(&issue_id, self.gate_key.clone())
                .expect("the gate is required");
            issue_id
        }

        fn evaluate(&self, issue_id: &str) -> GateRunResult {
            self.executor
                .check_gate(issue_id, &self.gate_key)
                .expect("the gate evaluates")
        }

        /// How many times the checker process has run.
        fn executions(&self) -> usize {
            std::fs::read(self.repo.path().join("executions.log"))
                .map(|bytes| bytes.len())
                .unwrap_or(0)
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.repo.path().join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent");
            }
            std::fs::write(path, contents).expect("write file");
        }

        fn remove(&self, relative: &str) {
            std::fs::remove_file(self.repo.path().join(relative)).expect("remove file");
        }

        /// Replace the gate's checker command, leaving its declared inputs and
        /// every repository file alone.
        fn rewrite_checker_command(&self, command: &str) {
            let mut registry = self
                .executor
                .storage
                .load_gate_registry()
                .expect("the registry loads");
            let gate = registry
                .gates
                .get_mut(&self.gate_key)
                .expect("the gate under test is registered");
            gate.checker = Some(GateChecker::Exec {
                command: command.to_string(),
                timeout_seconds: 30,
                working_dir: None,
                env: HashMap::new(),
                pass_context: false,
                prompt: None,
                prompt_file: None,
            });
            let bytes = crate::declarations::serialize_gate_registry(&registry)
                .expect("the registry serializes");
            std::fs::write(self.repo.path().join(".jit/gates.toml"), bytes)
                .expect("write registry");
        }
    }

    /// REQ-03/REQ-04: a second issue's evaluation over an unchanged declared
    /// input set carries the first run's verdict, names it, and never reaches
    /// the checker.
    #[test]
    fn test_check_gate_reuses_a_recorded_verdict_over_identical_declared_inputs() {
        let fixture = ReuseFixture::new(Some((&["workspace"], &[])), 0);
        let first_issue = fixture.issue();
        let second_issue = fixture.issue();

        let executed = fixture.evaluate(&first_issue);
        let reused = fixture.evaluate(&second_issue);

        assert_eq!(fixture.executions(), 1, "the checker must run exactly once");
        assert!(executed.origin.is_executed());
        assert_eq!(reused.origin.source_run(), Some(executed.run_id.as_str()));
        assert_eq!(reused.status, executed.status);
        assert_eq!(reused.inputs_digest, executed.inputs_digest);
        assert!(
            executed.inputs_digest.is_some(),
            "a gate declaring inputs must record their digest"
        );
        // Each issue carries its own record of the gate's outcome.
        assert_eq!(reused.issue_id, second_issue);
        assert_ne!(reused.run_id, executed.run_id);
    }

    /// REQ-04: a reused verdict carries the source run's status whether it
    /// passed or failed — a recorded failure is not quietly dropped.
    #[test]
    fn test_check_gate_reuses_a_recorded_failure_and_names_its_run() {
        let fixture = ReuseFixture::new(Some((&["workspace"], &[])), 1);
        let first_issue = fixture.issue();
        let second_issue = fixture.issue();

        let executed = fixture.evaluate(&first_issue);
        let reused = fixture.evaluate(&second_issue);

        assert_eq!(executed.status, GateRunStatus::Failed);
        assert_eq!(fixture.executions(), 1);
        assert_eq!(reused.status, GateRunStatus::Failed);
        assert_eq!(reused.origin.source_run(), Some(executed.run_id.as_str()));
    }

    /// REQ-06: an edit, an addition, and a removal beneath a declared root each
    /// change the digest, so the next evaluation executes the checker.
    #[test]
    fn test_check_gate_executes_again_after_declared_inputs_change() {
        let fixture = ReuseFixture::new(Some((&["workspace"], &[])), 0);
        let first = fixture.evaluate(&fixture.issue());
        assert_eq!(fixture.executions(), 1);

        fixture.write("workspace/src/lib.rs", "fn main() { /* edited */ }\n");
        let after_edit = fixture.evaluate(&fixture.issue());
        assert_eq!(fixture.executions(), 2, "an edited input must re-execute");
        assert!(after_edit.origin.is_executed());
        assert_ne!(after_edit.inputs_digest, first.inputs_digest);

        fixture.write("workspace/src/added.rs", "fn added() {}\n");
        let after_add = fixture.evaluate(&fixture.issue());
        assert_eq!(fixture.executions(), 3, "an added input must re-execute");
        assert_ne!(after_add.inputs_digest, after_edit.inputs_digest);

        fixture.remove("workspace/src/lib.rs");
        let after_remove = fixture.evaluate(&fixture.issue());
        assert_eq!(fixture.executions(), 4, "a removed input must re-execute");
        assert_ne!(after_remove.inputs_digest, after_add.inputs_digest);
    }

    /// Restoring an input set a checker has already been run over reuses that
    /// run: the digest identifies content, not the sequence of edits that
    /// produced it.
    #[test]
    fn test_check_gate_reuses_after_declared_inputs_return_to_a_verified_state() {
        let fixture = ReuseFixture::new(Some((&["workspace"], &[])), 0);
        let original = fixture.evaluate(&fixture.issue());

        fixture.write("workspace/src/lib.rs", "fn main() { /* edited */ }\n");
        fixture.evaluate(&fixture.issue());
        assert_eq!(fixture.executions(), 2);

        fixture.write("workspace/src/lib.rs", "fn main() {}\n");
        let restored = fixture.evaluate(&fixture.issue());

        assert_eq!(fixture.executions(), 2, "the restored content was verified");
        assert_eq!(restored.inputs_digest, original.inputs_digest);
        assert_eq!(restored.origin.source_run(), Some(original.run_id.as_str()));
    }

    /// REQ-07: a change to a file no declared root covers leaves the digest
    /// alone, so the next evaluation reuses instead of re-executing.
    #[test]
    fn test_check_gate_reuses_after_a_change_no_declared_root_covers() {
        let fixture = ReuseFixture::new(Some((&["workspace"], &[])), 0);
        let executed = fixture.evaluate(&fixture.issue());

        fixture.write("elsewhere.txt", "rewritten\n");
        let reused = fixture.evaluate(&fixture.issue());

        assert_eq!(fixture.executions(), 1);
        assert_eq!(reused.inputs_digest, executed.inputs_digest);
        assert_eq!(reused.origin.source_run(), Some(executed.run_id.as_str()));
    }

    /// REQ-07: an excluded path beneath a declared root is outside the set too,
    /// so changing it reuses.
    #[test]
    fn test_check_gate_reuses_after_a_change_a_declared_exclusion_matches() {
        let fixture = ReuseFixture::new(Some((&["workspace"], &["workspace/notes/**"])), 0);
        fixture.write("workspace/notes/scratch.md", "before\n");
        let executed = fixture.evaluate(&fixture.issue());

        fixture.write("workspace/notes/scratch.md", "after\n");
        let reused = fixture.evaluate(&fixture.issue());

        assert_eq!(fixture.executions(), 1);
        assert_eq!(reused.origin.source_run(), Some(executed.run_id.as_str()));
    }

    /// REQ-08: a gate declaring no inputs has nothing to prove unchanged, so
    /// every evaluation executes its checker and records no digest.
    #[test]
    fn test_check_gate_executes_every_time_for_a_gate_declaring_no_inputs() {
        let fixture = ReuseFixture::new(None, 0);

        let first = fixture.evaluate(&fixture.issue());
        let second = fixture.evaluate(&fixture.issue());

        assert_eq!(fixture.executions(), 2);
        assert!(first.inputs_digest.is_none());
        assert!(second.origin.is_executed());
    }

    /// REQ-08: an explicit re-run request executes the checker even where the
    /// declared inputs would otherwise license reuse.
    #[test]
    fn test_check_gate_with_execute_source_ignores_a_reusable_verdict() {
        let fixture = ReuseFixture::new(Some((&["workspace"], &[])), 0);
        let executed = fixture.evaluate(&fixture.issue());

        let forced = fixture
            .executor
            .check_gate_with(
                &fixture.issue(),
                &fixture.gate_key,
                super::VerdictSource::Execute,
            )
            .expect("the forced evaluation runs");

        assert_eq!(fixture.executions(), 2);
        assert!(forced.origin.is_executed());
        assert_eq!(
            forced.inputs_digest, executed.inputs_digest,
            "a forced run still records the digest it was derived over"
        );
    }

    /// REQ-09: a wave of issues evaluated over one unchanged source state
    /// executes the checker once, and every other issue's record names that one
    /// run rather than claiming a verification of its own.
    #[test]
    fn test_check_gate_executes_once_across_a_wave_over_one_unchanged_tree() {
        let fixture = ReuseFixture::new(Some((&["workspace"], &[])), 0);
        let wave: Vec<GateRunResult> = (0..5).map(|_| fixture.evaluate(&fixture.issue())).collect();

        assert_eq!(fixture.executions(), 1);
        let executed: Vec<&GateRunResult> =
            wave.iter().filter(|run| run.origin.is_executed()).collect();
        assert_eq!(executed.len(), 1, "one checker execution across the wave");
        assert!(
            wave.iter()
                .filter(|run| !run.origin.is_executed())
                .all(|run| run.origin.source_run() == Some(executed[0].run_id.as_str())),
            "every reused record must name the one run that executed"
        );
        assert!(
            wave.iter().all(|run| run.status == GateRunStatus::Passed),
            "every issue in the wave still reports the gate's verdict"
        );
    }

    /// A run that errored reached no verdict about the inputs, so it is never
    /// reused: the next evaluation executes the checker again.
    #[test]
    fn test_check_gate_never_reuses_a_run_that_errored() {
        // Exit 127 is the shell's "command not found": a runner error, not a
        // checker verdict.
        let fixture = ReuseFixture::new(Some((&["workspace"], &[])), 127);

        let errored = fixture.evaluate(&fixture.issue());
        let retried = fixture.evaluate(&fixture.issue());

        assert_eq!(errored.status, GateRunStatus::Error);
        assert_eq!(fixture.executions(), 2, "an errored run must not be reused");
        assert!(retried.origin.is_executed());
    }

    /// A verdict is a function of the checker as well as its inputs: editing
    /// the checker over an unchanged tree executes rather than inheriting a
    /// verdict the edited checker never produced.
    #[test]
    fn test_check_gate_executes_again_after_the_checker_declaration_changes() {
        let fixture = ReuseFixture::new(Some((&["workspace"], &[])), 0);
        let before = fixture.evaluate(&fixture.issue());

        fixture.rewrite_checker_command(&format!(
            "printf xx >> '{}'; exit 0",
            fixture.repo.path().join("executions.log").to_string_lossy()
        ));
        let after = fixture.evaluate(&fixture.issue());

        assert_eq!(
            fixture.executions(),
            3,
            "the edited checker must run rather than inherit the old verdict"
        );
        assert!(after.origin.is_executed());
        assert_ne!(after.inputs_digest, before.inputs_digest);
    }

    /// REQ-05: the machine-readable status of a reused verdict is
    /// distinguishable from a derived one and names the run behind it.
    #[test]
    fn test_gate_run_summary_distinguishes_a_reused_verdict_from_a_derived_one() {
        let fixture = ReuseFixture::new(Some((&["workspace"], &[])), 0);
        let executed = fixture.evaluate(&fixture.issue());
        let reused = fixture.evaluate(&fixture.issue());

        let rendered = |run: &GateRunResult| {
            serde_json::to_value(crate::output::GateRunSummary::lean(run))
                .expect("the summary serializes")
        };
        let executed_json = rendered(&executed);
        let reused_json = rendered(&reused);

        assert_ne!(
            executed_json["origin"], reused_json["origin"],
            "the two derivations must not read identically"
        );
        assert!(
            reused_json["origin"].to_string().contains(&executed.run_id),
            "the reused summary must name the run it was taken from: {reused_json}"
        );
        assert_eq!(
            executed_json["inputs_digest"], reused_json["inputs_digest"],
            "both summaries describe the same declared input content"
        );
    }

    fn add_exec_gate<S: IssueStore + crate::storage::RepositoryStateStore>(
        executor: &CommandExecutor<S>,
        issue_id: &str,
        gate_key: &str,
        title: &str,
        command: String,
        prompt_file: Option<String>,
    ) {
        executor
            .define_gate(
                gate_key.to_string(),
                title.to_string(),
                "Race fixture".to_string(),
                GateStage::Postcheck,
                GateMode::Auto,
                Some(GateChecker::Exec {
                    command,
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: Some("inline prompt".to_string()),
                    prompt_file,
                }),
                100,
            )
            .unwrap();
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

        let layout = executor.require_layout().unwrap();
        let result =
            crate::repository_state::gate_run_result_relative_path("evidence-probe").unwrap();
        let run_paths = [
            crate::repository_state::VirtualPath::data("gate-runs/evidence-probe").unwrap(),
            crate::repository_state::VirtualPath::data(result.as_path()).unwrap(),
        ];
        let mut session = executor.storage.open_mutation_session(layout).unwrap();
        let image = executor
            .capture_gate_evaluation_image(
                session.as_mut(),
                &issue_id
                    .parse::<crate::domain::identity::IssueIdPrefix>()
                    .unwrap(),
                gate_key,
                &run_paths,
            )
            .unwrap()
            .unwrap()
            .0;
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
        executor
            .update_gate(
                "no-context",
                GateUpdate {
                    pass_context: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
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
        let (repo, executor, issue_id) = setup_file_repository();
        executor
            .add_gate(&issue_id, "editable-review".to_string())
            .unwrap();
        let mut issue = executor.storage.load_issue(&issue_id).unwrap();
        issue.documents.push(document_reference("../../outside"));
        let bytes = crate::repository_state::serialize_issue(&issue).unwrap();
        std::fs::write(
            repo.path().join(format!(".jit/issues/{issue_id}.json")),
            bytes,
        )
        .unwrap();

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
        let late_rule = late_repository_rule(&crate::test_taxonomy::test_taxonomy());
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
        let overrides = crate::commands::overrides_from_repo_changes(
            live_image.layout(),
            [
                (
                    PathBuf::from(".jit/rules.toml"),
                    Some(late_rule.as_bytes().to_vec()),
                ),
                (
                    PathBuf::from(format!(".jit/issues/{issue_id}.json")),
                    Some(serde_json::to_vec_pretty(&planned_issue).unwrap()),
                ),
            ],
        )
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
        assert_eq!(planned.status, GateRunStatus::Error);
        assert!(
            planned.stderr.contains("does not exist"),
            "{}",
            planned.stderr
        );
    }

    #[test]
    fn test_check_gate_file_repository_ignores_legacy_cached_rules() {
        let (repo, executor, issue_id) = setup_file_repository();
        let late_rule = late_repository_rule(&crate::test_taxonomy::test_taxonomy());
        let cached = executor.run_rules(None).unwrap();
        assert!(!cached
            .findings
            .iter()
            .any(|finding| finding.rule == "planned-task-needs-summary"));
        std::fs::write(repo.path().join(".jit/rules.toml"), late_rule).unwrap();

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

        assert_eq!(result.status, GateRunStatus::Error);
        assert!(result.stderr.contains("index.json"), "{}", result.stderr);
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
    fn test_label_target_validation_records_missing_configured_pointer_as_error_run() {
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

        assert_eq!(result.status, GateRunStatus::Error);
        assert_eq!(
            result.exit_code,
            Some(crate::gate_execution::CHECKER_TEMPFAIL_EXIT)
        );
        assert!(result.stderr.contains("parent-pointer:<target-id>"));
    }

    #[test]
    fn test_label_target_validation_records_empty_namespace_as_error_run() {
        let executor = setup();
        let issue_id = add_builtin_gate(
            &executor,
            "arbitrary-coverage-key",
            GateChecker::LabelTargetValidation {
                label_namespace: String::new(),
            },
            vec!["type:task".to_string()],
        );

        let result = executor
            .check_gate(&issue_id, "arbitrary-coverage-key")
            .unwrap();

        assert_eq!(result.status, GateRunStatus::Error);
    }

    #[test]
    fn test_label_target_validation_records_unresolvable_target_as_error_run() {
        let executor = setup();
        let issue_id = add_builtin_gate(
            &executor,
            "arbitrary-coverage-key",
            GateChecker::LabelTargetValidation {
                label_namespace: "coverage-target".to_string(),
            },
            vec![
                "type:task".to_string(),
                "coverage-target:no-such-issue".to_string(),
            ],
        );

        let result = executor
            .check_gate(&issue_id, "arbitrary-coverage-key")
            .unwrap();

        assert_eq!(result.status, GateRunStatus::Error);
    }

    #[test]
    fn test_label_target_validation_passes_for_configured_target() {
        let executor = setup();
        let mut target =
            crate::domain::fixture_issue("Container".to_string(), "Container".to_string());
        target.labels = vec!["type:task".to_string()];
        let target_id = target.id.clone();
        seed_issue(executor.storage(), target);

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
    fn test_rule_validation_checker_ignores_unrelated_validation_failures() {
        let executor = setup();
        let config = r#"
[worktree]
enforce_leases = "off"

[item_kinds.requirement]
section = "success_criteria"
id-pattern = "[A-Z][A-Z0-9]*-[0-9]+"
markers = ["[hard]"]
link-namespaces = ["satisfies"]
scope = "issue"
source-of-truth = "markdown-first"
"#;
        let config_path = executor
            .repository_layout()
            .unwrap()
            .resolve(&crate::repository_state::VirtualPath::CONFIG)
            .unwrap();
        std::fs::write(config_path, config).unwrap();
        seed_repo_file(&executor.storage, ".jit/config.toml", config);
        seed_rule_validation_rules(&executor);

        let mut container = crate::domain::fixture_issue(
            "Selected container".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: selected work\n".to_string(),
        );
        container.labels = vec!["type:epic".to_string()];
        let container_short = container.short_id();

        let mut child =
            crate::domain::fixture_issue("Selected implementation".to_string(), String::new());
        child.labels = vec![
            "type:task".to_string(),
            format!("satisfies:{container_short}/REQ-01"),
        ];
        container.dependencies = vec![child.id.clone()];
        seed_issue(&executor.storage, child);
        seed_issue(&executor.storage, container);

        let selected_id = add_builtin_gate(
            &executor,
            "coverage-preview",
            GateChecker::RuleValidation {
                rule: "coverage-preview".to_string(),
            },
            vec![
                "type:breakdown".to_string(),
                format!("brackets:{container_short}"),
            ],
        );

        let mut local_failure = crate::domain::fixture_issue(
            "Unrelated local failure".to_string(),
            "See @/missing/item".to_string(),
        );
        local_failure.labels = vec![
            "type:task".to_string(),
            "satisfies:@/requirement/missing".to_string(),
        ];
        local_failure.dependencies = vec!["missing-dependency".to_string()];
        local_failure.gates_required = vec!["unrelated-pending".to_string()];
        seed_issue(&executor.storage, local_failure);

        let mut unrelated_graph = crate::domain::fixture_issue(
            "Unrelated graph failure".to_string(),
            "## Success Criteria\n\n- [hard] REQ-99: uncovered\n".to_string(),
        );
        unrelated_graph.labels = vec!["type:epic".to_string()];
        seed_issue(&executor.storage, unrelated_graph);
        seed_unrelated_manual_gate(&executor);

        let broad_report = executor
            .run_rules(None)
            .expect("the representative unrelated failures should be observable");
        for rule in [
            "unrelated-local-failure",
            "unrelated-graph-failure",
            "unrelated-label-reference",
            crate::commands::DANGLING_LINK_RULE,
        ] {
            assert!(
                broad_report
                    .findings
                    .iter()
                    .any(|finding| finding.rule == rule),
                "broad validation fixture must contain {rule}: {:?}",
                broad_report.findings
            );
        }

        let result = executor
            .check_gate(&selected_id, "coverage-preview")
            .expect("captured native rule validation should run");

        assert_eq!(result.command, "builtin:rule_validation");
        assert_eq!(result.status, GateRunStatus::Passed);
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.findings.unwrap().verdict, "pass");
    }

    #[test]
    fn test_rule_validation_checker_reports_missing_and_ambiguous_targets() {
        let missing_executor = setup();
        seed_rule_validation_rules(&missing_executor);
        let missing_id = add_builtin_gate(
            &missing_executor,
            "coverage-preview",
            GateChecker::RuleValidation {
                rule: "coverage-preview".to_string(),
            },
            vec![
                "type:breakdown".to_string(),
                "brackets:missing-target".to_string(),
            ],
        );
        let missing = missing_executor
            .check_gate(&missing_id, "coverage-preview")
            .expect("missing target should be a native validation failure");
        assert_eq!(missing.status, GateRunStatus::Failed);
        assert!(missing
            .findings
            .unwrap()
            .findings
            .iter()
            .any(|finding| finding.summary.contains("names no known issue")));

        let ambiguous_executor = setup();
        seed_rule_validation_rules(&ambiguous_executor);
        for id in ["deadbeef-a", "deadbeef-b"] {
            let mut target =
                crate::domain::fixture_issue(format!("Ambiguous target {id}"), String::new());
            target.id = id.to_string();
            target.labels = vec!["type:epic".to_string()];
            seed_issue(&ambiguous_executor.storage, target);
        }
        let ambiguous_id = add_builtin_gate(
            &ambiguous_executor,
            "coverage-preview",
            GateChecker::RuleValidation {
                rule: "coverage-preview".to_string(),
            },
            vec![
                "type:breakdown".to_string(),
                "brackets:deadbeef".to_string(),
            ],
        );
        let ambiguous = ambiguous_executor
            .check_gate(&ambiguous_id, "coverage-preview")
            .expect("ambiguous target should be a native validation failure");
        assert_eq!(ambiguous.status, GateRunStatus::Failed);
        assert!(ambiguous
            .findings
            .unwrap()
            .findings
            .iter()
            .any(|finding| finding.summary.contains("ambiguous")));
    }

    #[test]
    fn test_rule_validation_checker_records_missing_rule_as_error_run() {
        let executor = setup();
        seed_rule_validation_rules(&executor);
        let issue_id = add_builtin_gate(
            &executor,
            "coverage-preview",
            GateChecker::RuleValidation {
                rule: "no-such-rule".to_string(),
            },
            vec!["type:task".to_string()],
        );

        let result = executor.check_gate(&issue_id, "coverage-preview").unwrap();

        assert_eq!(result.command, "builtin:rule_validation");
        assert_eq!(result.status, GateRunStatus::Error);
        assert!(result.stderr.contains("no-such-rule"), "{}", result.stderr);
    }

    #[test]
    fn test_builtin_checker_records_unloadable_declarations_as_error_run() {
        let (repo, executor, issue_id) = setup_file_repository();
        std::fs::write(repo.path().join(".jit/rules.toml"), "[[rules\nnot toml").unwrap();
        for checker in [
            GateChecker::IssueValidation,
            GateChecker::RepositoryValidation,
            GateChecker::RuleValidation {
                rule: "any".to_string(),
            },
        ] {
            let image = executor
                .capture_validation_image_with(&std::collections::BTreeMap::new())
                .unwrap();
            let result = executor
                .execute_builtin_checker_with_repository_view(
                    "not-a-reserved-repository-key",
                    &issue_id,
                    GateStage::Postcheck,
                    &checker,
                    &image,
                )
                .unwrap();

            assert_eq!(result.status, GateRunStatus::Error, "{checker:?}");
        }
    }

    #[test]
    fn test_builtin_checker_records_content_finding_as_failed_run() {
        let (repo, executor, issue_id) = setup_file_repository();
        let late_rule = late_repository_rule(&crate::test_taxonomy::test_taxonomy());
        std::fs::write(repo.path().join(".jit/rules.toml"), late_rule).unwrap();

        let result = executor
            .check_gate(&issue_id, "not-a-reserved-repository-key")
            .unwrap();

        assert_eq!(result.status, GateRunStatus::Failed);
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue with gate
        let issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue with gate
        let issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
                inputs: None,
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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

    fn make_manual_gate(key: &str) -> crate::declarations::GateDefinition {
        crate::declarations::GateDefinition {
            mode: GateMode::Manual,
            checker: None,
            ..make_auto_gate(key, "")
        }
    }

    /// An issue requiring the manual gate `review`, whose registry also declares
    /// the automated gate `tests`.
    fn seed_review_issue(executor: &CommandExecutor<InMemoryStorage>) -> String {
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry
            .gates
            .insert("review".to_string(), make_manual_gate("review"));
        registry
            .gates
            .insert("tests".to_string(), make_auto_gate("tests", "exit 0"));
        seed_gate_registry(executor.storage(), &registry);
        let mut issue = crate::domain::fixture_issue("T".to_string(), String::new());
        issue.gates_required = vec!["review".to_string(), "tests".to_string()];
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        issue_id
    }

    #[test]
    fn test_pass_gate_manual_attestation_publishes_attested_gate_run() {
        let executor = setup();
        let issue_id = seed_review_issue(&executor);
        let before = Utc::now();

        executor
            .pass_gate(
                &issue_id,
                "review".to_string(),
                Some("human:me".to_string()),
                false,
            )
            .unwrap();

        let run = executor
            .get_last_gate_run(&issue_id, "review")
            .unwrap()
            .expect("an attestation is a recorded gate run");
        assert_eq!(run.status, GateRunStatus::Passed);
        assert_eq!(
            run.by.as_ref().map(ToString::to_string).as_deref(),
            Some("human:me")
        );
        assert_eq!(run.origin, crate::domain::GateVerdictOrigin::Attested);
        assert!(run.started_at >= before && run.started_at <= Utc::now());
        assert_eq!(
            executor.get_gate_run_result(&run.run_id).unwrap(),
            run,
            "the attestation is addressable by its run id"
        );
    }

    #[test]
    fn test_list_gate_runs_lists_manual_attestations_alongside_automated_runs() {
        let executor = setup();
        let issue_id = seed_review_issue(&executor);

        executor.check_gate(&issue_id, "tests").unwrap();
        for _ in 0..2 {
            executor
                .pass_gate(
                    &issue_id,
                    "review".to_string(),
                    Some("human:me".to_string()),
                    false,
                )
                .unwrap();
        }

        let runs = executor.list_gate_runs(&issue_id, None).unwrap();
        let attested = runs
            .iter()
            .filter(|run| run.origin == crate::domain::GateVerdictOrigin::Attested)
            .count();
        assert_eq!(attested, 2, "every attestation is its own record");
        assert!(runs
            .iter()
            .any(|run| run.gate_key == "tests" && run.origin.is_executed()));
    }

    #[test]
    fn test_get_last_gate_runs_for_issue_agrees_with_required_gate_statuses() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        let gates = [
            make_auto_gate("auto-pass", "exit 0"),
            make_auto_gate("auto-fail", "exit 1"),
            make_auto_gate("auto-pending", "exit 0"),
            make_manual_gate("manual-pass"),
            make_manual_gate("manual-fail"),
            make_manual_gate("manual-pending"),
        ];
        for gate in &gates {
            registry.gates.insert(gate.key.clone(), gate.clone());
        }
        seed_gate_registry(executor.storage(), &registry);
        let mut issue = crate::domain::fixture_issue("T".to_string(), String::new());
        issue.gates_required = gates.iter().map(|gate| gate.key.clone()).collect();
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);

        executor.check_gate(&issue_id, "auto-pass").unwrap();
        executor.check_gate(&issue_id, "auto-fail").unwrap();
        let attestor = Some("human:me".to_string());
        executor
            .pass_gate(
                &issue_id,
                "manual-pass".to_string(),
                attestor.clone(),
                false,
            )
            .unwrap();
        executor
            .fail_gate(&issue_id, "manual-fail".to_string(), attestor)
            .unwrap();

        let (runs, not_run) = executor.get_last_gate_runs_for_issue(&issue_id).unwrap();
        let latest: HashMap<_, _> = runs
            .iter()
            .map(|run| (run.gate_key.clone(), run.clone()))
            .collect();
        let readiness = executor
            .get_required_gate_statuses_for_issue(&issue_id)
            .unwrap();
        assert_eq!(readiness.len(), gates.len());
        for (key, status) in readiness {
            let from_run = match latest.get(&key) {
                Some(run) => super::gate_state_from_run(run).status,
                None => {
                    assert!(not_run.contains(&key), "{key} is neither run nor not-run");
                    crate::domain::GateStatus::Pending
                }
            };
            assert_eq!(from_run, status, "views disagree on {key}");
        }
    }

    /// Runs publish in an order their start times do not follow: automated run A
    /// starts first and publishes after a later-started B, and attestation A
    /// retries its publication after a concurrent attestation B. The latest-run
    /// view must select what readiness reflects, the last publication.
    #[cfg(unix)]
    #[test]
    fn test_get_last_gate_runs_for_issue_follows_publication_order_across_run_kinds() {
        use crate::commands::test_helpers::{memory_executor, with_open_race, SessionRaceAction};

        let base = setup();
        let marker_dir = tempfile::tempdir().unwrap();
        let marker = marker_dir.path().join("b-started");
        let mut registry = base.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "auto".to_string(),
            make_auto_gate("auto", &format!("test -e {}", marker.display())),
        );
        registry
            .gates
            .insert("review".to_string(), make_manual_gate("review"));
        seed_gate_registry(base.storage(), &registry);
        let mut issue = crate::domain::fixture_issue("T".to_string(), String::new());
        issue.gates_required = vec!["auto".to_string(), "review".to_string()];
        let issue_id = issue.id.clone();
        seed_issue(base.storage(), issue);

        // Automated B starts after A's checker ran and publishes before A's
        // record session captures.
        let concurrent = issue_id.clone();
        let evaluating = with_open_race(
            base.storage.clone(),
            2,
            SessionRaceAction::Run(Box::new(move |storage| {
                std::fs::write(&marker, "").unwrap();
                let run = memory_executor(storage.clone())
                    .check_gate(&concurrent, "auto")
                    .unwrap();
                assert_eq!(run.status, GateRunStatus::Passed);
            })),
        );
        let run = memory_executor(evaluating)
            .check_gate(&issue_id, "auto")
            .unwrap();
        assert_eq!(run.status, GateRunStatus::Failed);

        // Attestation A conflicts on its first apply; B publishes before A's
        // retry captures.
        let concurrent = issue_id.clone();
        let attesting = with_open_race(
            base.storage.clone(),
            2,
            SessionRaceAction::Run(Box::new(move |storage| {
                memory_executor(storage.clone())
                    .pass_gate(
                        &concurrent,
                        "review".to_string(),
                        Some("human:b".to_string()),
                        false,
                    )
                    .unwrap();
            })),
        );
        attesting.inject_repository_state_apply_conflicts(1);
        memory_executor(attesting)
            .fail_gate(&issue_id, "review".to_string(), Some("human:a".to_string()))
            .unwrap();

        let readiness = base
            .get_required_gate_statuses_for_issue(&issue_id)
            .unwrap();
        assert_eq!(
            readiness,
            vec![
                ("auto".to_string(), crate::domain::GateStatus::Failed),
                ("review".to_string(), crate::domain::GateStatus::Failed),
            ],
            "each A published last"
        );
        let (latest, not_run) = base.get_last_gate_runs_for_issue(&issue_id).unwrap();
        assert!(not_run.is_empty());
        let latest_status = latest
            .iter()
            .map(|run| {
                let state = super::gate_state_from_run(run);
                (run.gate_key.clone(), state.status)
            })
            .collect::<Vec<_>>();
        assert_eq!(latest_status, readiness);
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
                    inputs: None,
                },
            );
        }
        seed_gate_registry(executor.storage(), &registry);

        // Create issue with both gates
        let issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
                inputs: None,
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue with precheck
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::Ready;
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .add_gate(&issue_id, "precheck".to_string())
            .unwrap();
        executor
            .add_gate(&issue_id, "postcheck-with-unsafe-prompt".to_string())
            .unwrap();

        // Try to start work - should run prechecks
        executor
            .issues()
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
        executor
            .update_gate(
                "precheck",
                GateUpdate {
                    stage: Some(GateStage::Precheck),
                    ..Default::default()
                },
            )
            .unwrap();
        executor
            .issues()
            .update_issue_state(&issue_id, State::Ready)
            .unwrap();

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
            .issues()
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
        executor
            .update_gate(
                "precheck-no-context",
                GateUpdate {
                    stage: Some(GateStage::Precheck),
                    pass_context: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        executor
            .issues()
            .update_issue_state(&issue_id, State::Ready)
            .unwrap();
        write_corrupt_gate_run(repo.path());

        executor
            .issues()
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);
        let mut issue = crate::domain::fixture_issue("Test".into(), String::new());
        issue.state = State::Ready;
        issue.gates_required.push("precheck".to_string());
        let id = issue.id.clone();
        seed_issue(executor.storage(), issue);

        let error = executor
            .issues()
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue with failing precheck
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::Ready;
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .add_gate(&issue_id, "precheck-fail".to_string())
            .unwrap();

        // Try to start work - should fail
        let result = executor
            .issues()
            .update_issue_state(&issue_id, State::InProgress);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("precheck"));

        // Verify issue didn't transition
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Ready);
    }

    #[cfg(unix)]
    #[test]
    fn test_lifecycle_precheck_records_one_moved_run_when_checker_mutates_repository() {
        let (repo, executor, issue_id) = setup_file_repository();
        add_exec_gate(
            &executor,
            &issue_id,
            "precheck-race",
            "Precheck race",
            format!(
                "if [ ! -f .precheck-race-seen ]; then : > .precheck-race-seen; sed -i 's/\\\"title\\\": \\\"Test\\\"/\\\"title\\\": \\\"Moved\\\"/' .jit/issues/{issue_id}.json; fi; printf x >> checker-count; true"
            ),
            None,
        );
        executor
            .update_gate(
                "precheck-race",
                GateUpdate {
                    stage: Some(GateStage::Precheck),
                    ..Default::default()
                },
            )
            .unwrap();
        executor
            .issues()
            .update_issue_state(&issue_id, State::Ready)
            .unwrap();

        executor
            .issues()
            .update_issue_state(&issue_id, State::InProgress)
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(repo.path().join("checker-count")).unwrap(),
            "x"
        );
        let runs = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .into_iter()
            .filter(|run| run.gate_key == "precheck-race")
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].evaluation_stability,
            crate::domain::GateEvaluationStability::InputsMoved
        );
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().state,
            State::InProgress
        );
    }

    /// A precheck validating the whole repository runs over the issue's
    /// documents, which the transition's own base does not read.
    #[test]
    fn test_update_issue_state_broad_precheck_over_issue_documents_records_a_stable_run() {
        let (repo, executor, issue_id) = setup_file_repository();
        let gate_key = "not-a-reserved-repository-key";
        executor
            .update_gate(
                gate_key,
                GateUpdate {
                    stage: Some(GateStage::Precheck),
                    ..Default::default()
                },
            )
            .unwrap();
        std::fs::create_dir_all(repo.path().join("dev")).unwrap();
        std::fs::write(repo.path().join("dev/notes.md"), "# Notes\n").unwrap();
        executor
            .add_document_reference(&issue_id, "dev/notes.md", None, None, None, true)
            .unwrap();
        executor
            .issues()
            .update_issue_state(&issue_id, State::Ready)
            .unwrap();

        executor
            .issues()
            .update_issue_state(&issue_id, State::InProgress)
            .unwrap();

        let runs = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .into_iter()
            .filter(|run| run.gate_key == gate_key)
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].evaluation_stability,
            crate::domain::GateEvaluationStability::Stable
        );
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue in progress with postcheck
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .add_gate(&issue_id, "postcheck".to_string())
            .unwrap();

        // Complete work
        executor
            .issues()
            .update_issue_state(&issue_id, State::Gated)
            .unwrap();

        // Verify postchecks ran and issue transitioned to done
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Done);

        let gate_state = issue.gates_status.get("postcheck").unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Passed);
    }

    #[test]
    fn test_update_issue_state_failing_postcheck_succeeds_and_keeps_issue_gated() {
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue in progress with failing postcheck
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .add_gate(&issue_id, "postcheck-fail".to_string())
            .unwrap();

        // Complete work
        let result = executor
            .issues()
            .update_issue_state(&issue_id, State::Gated);
        assert!(
            result.is_ok(),
            "an ordinary checker failure remains a successful invocation: {result:?}"
        );

        // Verify postchecks ran but issue stayed in gated
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Gated);

        let gate_state = issue.gates_status.get("postcheck-fail").unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Failed);
        let runs = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, GateRunStatus::Failed);
    }

    #[test]
    fn test_update_issue_state_surfaces_postcheck_persistence_failure() {
        let executor = setup();
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .add_gate(&issue_id, "postcheck".to_string())
            .unwrap();

        // The Gated transition is the first publication. The automated
        // postcheck's coupled issue/run/event publication is the second.
        let failures = FailNthPublication::new(2);
        let storage = executor.storage().with_mutation_interruption_view(failures);
        let executor = crate::commands::test_helpers::memory_executor(storage);

        let error = executor
            .issues()
            .update_issue_state(&issue_id, State::Gated)
            .expect_err("postcheck persistence failure must reach the state-transition caller");

        assert!(
            error
                .to_string()
                .contains("injected postcheck persistence failure"),
            "unexpected error: {error:#}"
        );
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Gated);
        assert_eq!(
            issue.gates_status["postcheck"].status,
            crate::domain::GateStatus::Pending,
            "failed persistence must not leave an unrecorded checker verdict"
        );
        assert!(executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_update_issue_state_surfaces_postcheck_evaluation_failure() {
        let executor = setup();
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
                    pass_context: true,
                    prompt: None,
                    prompt_file: Some("missing-prompt.md".to_string()),
                }),
                priority: 100,
                reserved: HashMap::new(),
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .add_gate(&issue_id, "postcheck".to_string())
            .unwrap();

        let error = executor
            .issues()
            .update_issue_state(&issue_id, State::Gated)
            .expect_err("postcheck evaluation failure must reach the state-transition caller");

        assert!(
            error.to_string().contains("missing-prompt.md"),
            "unexpected error: {error:#}"
        );
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Gated);
        assert_eq!(
            issue.gates_status["postcheck"].status,
            crate::domain::GateStatus::Pending
        );
        assert!(executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .is_empty());
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create an issue with the gate
        let mut issue = crate::domain::fixture_issue(
            "Implement feature X".to_string(),
            "Add the X feature".to_string(),
        );
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
    fn test_check_gate_with_repository_relative_prompt_file() {
        let executor = setup();

        // Write a prompt file relative to repo root
        seed_repo_file(
            executor.storage(),
            "contrib/gates/code-review-prompt.md",
            "You are a senior engineer. Review for security issues.",
        );

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
                    prompt_file: Some("./contrib/gates/code-review-prompt.md".to_string()),
                }),
                priority: 100,
                reserved: HashMap::new(),
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue
        let issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
    fn test_check_gate_records_inputs_moved_without_reexecuting_when_prompt_changes_during_checker()
    {
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

        assert_eq!(context["prompt"], "old prompt");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("checker-count")).unwrap(),
            "x"
        );
        let runs = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .into_iter()
            .filter(|run| run.gate_key == "prompt-race")
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].evaluation_stability,
            crate::domain::GateEvaluationStability::InputsMoved
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_records_pre_run_verdict_when_issue_changes_during_checker() {
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

        assert_eq!(context["issue"]["title"], "Test");
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().title,
            "Changed concurrently"
        );
        assert_eq!(
            std::fs::read_to_string(repo.path().join("checker-count")).unwrap(),
            "x"
        );
        let runs = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .into_iter()
            .filter(|run| run.gate_key == "issue-race")
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].evaluation_stability,
            crate::domain::GateEvaluationStability::InputsMoved
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_records_pre_run_verdict_when_direct_dependency_changes() {
        let (repo, executor, issue_id) = setup_file_repository();
        let (dependency_id, _) = executor
            .issues()
            .create_issue(
                "Dependency".into(),
                "Test".into(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )
            .unwrap();
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

        assert_eq!(context["issue"]["dependencies"][0]["title"], "Dependency");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("checker-count")).unwrap(),
            "x"
        );
        let runs = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .into_iter()
            .filter(|run| run.gate_key == "dependency-race")
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].evaluation_stability,
            crate::domain::GateEvaluationStability::InputsMoved
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_records_pre_run_verdict_when_gate_definition_changes_during_checker() {
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

        assert_eq!(context["gate"]["title"], "Original race title");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("checker-count")).unwrap(),
            "x"
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
        let run = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .into_iter()
            .find(|run| run.gate_key == "gate-race")
            .unwrap();
        assert_eq!(
            run.evaluation_stability,
            crate::domain::GateEvaluationStability::InputsMoved
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
    fn test_check_gate_resolves_noncanonical_full_issue_ids() {
        let (_repo, executor, issue_id) = setup_file_repository();
        add_exec_gate(
            &executor,
            &issue_id,
            "canonical-id",
            "Canonical id",
            "true".to_string(),
            None,
        );

        let requests = [issue_id.to_uppercase(), issue_id.replace('-', "")];
        for requested in &requests {
            let run = executor.check_gate(requested, "canonical-id").unwrap();
            assert_eq!(run.issue_id, issue_id, "{requested} must bind the issue");
        }
        assert_eq!(
            executor
                .storage
                .list_gate_runs_for_issue(&issue_id)
                .unwrap()
                .len(),
            requests.len()
        );
    }

    #[test]
    fn test_check_gate_rejects_a_prefix_below_the_minimum_length() {
        let (_repo, executor, issue_id) = setup_file_repository();
        add_exec_gate(
            &executor,
            &issue_id,
            "short-id",
            "Short id",
            "true".to_string(),
            None,
        );

        let error = executor
            .check_gate(
                &issue_id[..crate::domain::identity::IssueIdPrefix::MIN_ID_PREFIX_LENGTH - 1],
                "short-id",
            )
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::domain::identity::InvalidIdPrefixError>()
                .is_some(),
            "{error:?}"
        );
        assert!(executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_classifies_same_issue_run_drift_but_not_unrelated_run() {
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
            "x"
        );
        assert_eq!(context["run_history"], serde_json::json!([]));
        let run = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .into_iter()
            .find(|run| run.gate_key == "run-race")
            .unwrap();
        assert_eq!(
            run.evaluation_stability,
            crate::domain::GateEvaluationStability::InputsMoved
        );

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
        let run = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .into_iter()
            .find(|run| run.gate_key == "run-race")
            .unwrap();
        assert_eq!(
            run.evaluation_stability,
            crate::domain::GateEvaluationStability::Stable
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_reuses_checker_result_after_apply_only_conflict() {
        let executor = setup();
        let issue = crate::domain::fixture_issue("Apply race".into(), "Test".into());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
            runs[0].evaluation_stability,
            crate::domain::GateEvaluationStability::Stable
        );
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
                        event.body(),
                        crate::domain::EventBody::GatePassed {
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

    #[cfg(unix)]
    #[test]
    fn test_check_gate_records_one_run_and_event_when_inputs_move_during_checker() {
        let (repo, executor, issue_id) = setup_file_repository();
        add_exec_gate(
            &executor,
            &issue_id,
            "single-record-race",
            "Single record race",
            format!(
                "if [ ! -f .single-record-race-seen ]; then : > .single-record-race-seen; sed -i 's/\\\"title\\\": \\\"Test\\\"/\\\"title\\\": \\\"Moved\\\"/' .jit/issues/{issue_id}.json; fi; printf x >> checker-count; true"
            ),
            None,
        );

        executor
            .check_gate(&issue_id, "single-record-race")
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(repo.path().join("checker-count")).unwrap(),
            "x"
        );
        let runs = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .into_iter()
            .filter(|run| run.gate_key == "single-record-race")
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].evaluation_stability,
            crate::domain::GateEvaluationStability::InputsMoved
        );
        assert_eq!(
            executor
                .storage
                .read_events()
                .unwrap()
                .into_iter()
                .filter(|event| {
                    matches!(
                        event.body(),
                        crate::domain::EventBody::GatePassed { issue_id: event_issue, gate_key, .. }
                            if event_issue == &issue_id && gate_key == "single-record-race"
                    )
                })
                .count(),
            1
        );
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().gates_status["single-record-race"]
                .status,
            crate::domain::GateStatus::Passed
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_records_failing_verdict_and_findings_when_inputs_move_during_checker() {
        let (repo, executor, issue_id) = setup_file_repository();
        add_exec_gate(
            &executor,
            &issue_id,
            "failing-race",
            "Failing race",
            format!(
                r#"if [ ! -f .failing-race-seen ]; then : > .failing-race-seen; sed -i 's/"title": "Test"/"title": "Moved"/' .jit/issues/{issue_id}.json; fi; printf x >> checker-count; printf '%s\n' '<<<JIT-FINDINGS-JSON' '{{"verdict":"fail","summary":"blocking finding","findings":[{{"id":"F1","severity":"high","disposition":"blocking","summary":"broken contract"}}]}}' 'JIT-FINDINGS-JSON>>>'; exit 1"#
            ),
            None,
        );

        let result = executor
            .check_gate(&issue_id, "failing-race")
            .expect("a checker failure is a recorded verdict");

        assert_eq!(result.status, GateRunStatus::Failed);
        assert_eq!(
            std::fs::read_to_string(repo.path().join("checker-count")).unwrap(),
            "x"
        );
        let run = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .into_iter()
            .find(|run| run.gate_key == "failing-race")
            .unwrap();
        assert_eq!(run.status, GateRunStatus::Failed);
        assert_eq!(
            run.evaluation_stability,
            crate::domain::GateEvaluationStability::InputsMoved
        );
        assert_eq!(
            run.findings.as_ref().unwrap().findings[0].summary,
            "broken contract"
        );
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().gates_status["failing-race"].status,
            crate::domain::GateStatus::Failed
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
        seed_repo_file(executor.storage(), "huge-prompt.md", &big_content);

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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
        let dep1 = crate::domain::fixture_issue(
            "Setup database schema".to_string(),
            "Create tables".to_string(),
        );
        let dep1_id = dep1.id.clone();
        seed_issue(executor.storage(), dep1);

        let dep2 = crate::domain::fixture_issue(
            "Implement auth module".to_string(),
            "OAuth2 flow".to_string(),
        );
        let dep2_id = dep2.id.clone();
        seed_issue(executor.storage(), dep2);

        // Create main issue that depends on both
        let mut main_issue = crate::domain::fixture_issue(
            "Build user dashboard".to_string(),
            "Dashboard feature".to_string(),
        );
        main_issue.state = State::InProgress;
        main_issue.dependencies.push(dep1_id.clone());
        main_issue.dependencies.push(dep2_id.clone());
        let main_id = main_issue.id.clone();
        seed_issue(executor.storage(), main_issue);

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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

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
                    inputs: None,
                },
            );
        }
        seed_gate_registry(executor.storage(), &registry);

        // Create issue with gates added in priority 30, 10, 20 order
        let issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
                    inputs: None,
                },
            );
        }
        seed_gate_registry(executor.storage(), &registry);

        // Add gates in specific insertion order
        let issue = crate::domain::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
            "mode": "manual"
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
            inputs: None,
        }
    }

    #[test]
    fn test_get_last_gate_run_returns_none_when_no_runs() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry
            .gates
            .insert("g".to_string(), make_auto_gate("g", "exit 0"));
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::domain::fixture_issue("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::domain::fixture_issue("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor.add_gate(&issue_id, "g".to_string()).unwrap();

        let first = executor.check_gate(&issue_id, "g").unwrap();
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
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::domain::fixture_issue("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::domain::fixture_issue("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
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
}
