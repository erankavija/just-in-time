//! Gate checking and execution operations

use super::*;
use crate::commands::IssueShowResponse;
use crate::declarations::{GateMode, GateStage, REVIEW_PLACEHOLDER_WARNING};
use crate::domain::gate_findings::{
    FindingDisposition, FindingOrigin, FindingSeverity, GateFinding, GateFindings,
};
use crate::domain::identity::GateRunId;
use crate::domain::identity::{GateKey, IssueIdPrefix};
use crate::domain::{
    Assignee, GateContext, GateEvaluationStability, GateRunResult, GateRunStatus, GateStatus,
};
use crate::errors::TransitionBlockedError;
use crate::gate_execution::{self, GateRunSubject};
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
/// stays at the source run, which [`crate::domain::GateVerdictOrigin::Reused`] names, rather
/// than being copied into every record that carries the verdict.
fn reused_gate_run(
    run_id: &GateRunId,
    source: &GateRunResult,
    issue_id: &IssueId,
    stage: GateStage,
) -> GateRunResult {
    let started_at = chrono::Utc::now();
    GateRunResult {
        schema_version: crate::domain::GATE_RUN_SCHEMA_VERSION,
        run_id: run_id.clone(),
        gate_key: source.gate_key.clone(),
        stage,
        issue_id: issue_id.clone(),
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
        origin: crate::domain::GateVerdictOrigin::Reused {
            source_issue: source.issue_id.clone(),
            source_run: source.run_id.clone(),
        },
        evaluation_stability: GateEvaluationStability::Stable,
    }
}

pub(super) struct PrecheckExecution {
    pub(super) runs: Vec<GateRunResult>,
    pub(super) error: Option<anyhow::Error>,
}

struct CapturedGateExecution<'a> {
    run_id: &'a GateRunId,
    image: &'a crate::repository_state::RepositoryImage,
    issue: &'a Issue,
    issues: &'a [Issue],
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

fn captured_issue_record(
    image: &crate::repository_state::RepositoryImage,
    issue_id: &IssueId,
) -> Result<Option<Issue>> {
    use crate::repository_state::{RepositoryEntry, VirtualPath};

    let path = VirtualPath::issue_record(issue_id);
    match image.entry(&path)? {
        RepositoryEntry::File { content, .. } => {
            let issue = crate::repository_state::decode_issue_record(&path, content.bytes())?;
            if &issue.id != issue_id {
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
    issue_id: &IssueId,
) -> Result<Issue> {
    captured_issue_record(image, issue_id)?
        .ok_or_else(|| crate::storage::IssueNotFoundError::new(issue_id.clone()).into())
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
    gate_key: &GateKey,
) -> Result<&'a crate::declarations::GateDefinition> {
    if !issue.gates_required.iter().any(|key| key == gate_key) {
        return Err(super::gate::GateNotRequiredError {
            issue_id: issue.id.clone(),
            gate_key: gate_key.clone(),
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

/// Select and sanitize `gate_key`'s run among `runs`, which hold at most one
/// latest run per gate, before it enters checker context.
///
/// Stored results are never mutated. Structured findings replace verbose stdout;
/// legacy unstructured runs retain stdout as their compatibility signal. Stderr is
/// always diagnostic noise and is removed from the context copy.
fn compact_run_history_for_context(
    runs: &[GateRunResult],
    gate_key: &GateKey,
) -> Vec<GateRunResult> {
    runs.iter()
        .find(|run| &run.gate_key == gate_key)
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

/// Make `fresh` its gate's run in `history`, keeping one run per gate.
fn replace_named_run(history: &mut Vec<GateRunResult>, fresh: GateRunResult) {
    history.retain(|run| run.gate_key != fresh.gate_key);
    history.push(fresh);
}

/// The run the verdict record of `(gate_key, digest)` names, captured with that
/// record and nothing else. `Ok(None)` is a retryable conflict; the inner
/// `None` means no record names a run reusable for that key.
fn captured_reusable_gate_run(
    session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
    gate_key: &GateKey,
    digest: &crate::domain::repository_inputs::InputsDigest,
) -> Result<Option<Option<GateRunResult>>> {
    use crate::repository_state::{CaptureSpec, RepositoryImage, VirtualPath};

    let record_path = VirtualPath::gate_verdict_record(gate_key, digest);
    let record = |image: &RepositoryImage| -> Result<Option<crate::domain::GateVerdictRecord>> {
        image
            .file_bytes(&record_path)?
            .map(|bytes| {
                serde_json::from_slice(bytes)
                    .with_context(|| format!("failed to parse verdict record at {record_path:?}"))
            })
            .transpose()
    };
    let spec = CaptureSpec::new(
        [record_path.clone()],
        super::validate::MUTATION_CAPTURE_BUDGET,
    )?;
    let Some(image) = super::capture_discovering(session, spec, |image, spec| {
        if let Some(record) = record(image)? {
            spec.discover_paths([VirtualPath::gate_run_record(
                &record.issue_id,
                &record.run_id,
            )])?;
        }
        Ok(())
    })?
    else {
        return Ok(None);
    };
    let Some(record) = record(&image)? else {
        return Ok(Some(None));
    };
    let run_path = VirtualPath::gate_run_record(&record.issue_id, &record.run_id);
    let run = image
        .file_bytes(&run_path)?
        .map(|bytes| {
            serde_json::from_slice::<GateRunResult>(bytes)
                .with_context(|| format!("failed to parse captured gate run at {run_path:?}"))
        })
        .transpose()?;
    Ok(Some(run.filter(|run| {
        run.issue_id == record.issue_id
            && run.run_id == record.run_id
            && run.is_reusable_for(gate_key, digest)
    })))
}

/// Read the run each required gate's state on `issue` names from `image`.
///
/// A named record that is absent, or that does not record that gate's run on
/// `issue`, contributes no result; its path is still an input.
fn captured_gate_runs(
    image: &crate::repository_state::RepositoryImage,
    issue: &Issue,
) -> Result<CapturedGateRuns> {
    use crate::repository_state::RepositoryEntry;

    let named = super::named_gate_run_records(issue).collect::<Vec<_>>();
    let results = named
        .iter()
        .map(|(key, run_id, path)| {
            let run = match image.entry(path)? {
                RepositoryEntry::File { content, .. } => {
                    serde_json::from_slice::<GateRunResult>(content.bytes())
                        .with_context(|| format!("failed to parse captured gate run at {path:?}"))?
                }
                RepositoryEntry::Absent => return Ok(None),
                _ => {
                    return Err(anyhow!(
                        "captured gate run at {path:?} is not a regular file"
                    ))
                }
            };
            Ok(
                (run.issue_id == issue.id && &run.gate_key == *key && &run.run_id == *run_id)
                    .then_some(run),
            )
        })
        .filter_map(Result::transpose)
        .collect::<Result<Vec<_>>>()?;
    let inputs = named.into_iter().map(|(_, _, path)| path).collect();
    Ok(CapturedGateRuns { results, inputs })
}

fn captured_gate_evaluation(
    image: &crate::repository_state::RepositoryImage,
    issue: &Issue,
    gate_key: &GateKey,
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
        BTreeSet::from([VirtualPath::issue_record(&issue.id)])
    };
    paths.extend(
        issue
            .dependencies
            .iter()
            .map(VirtualPath::issue_record)
            .collect::<Vec<_>>(),
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
fn omit_current_gate_projection(issue: &mut IssueShowResponse, gate_key: &GateKey) {
    issue.gates.retain(|gate| &gate.key != gate_key);
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

/// The gate findings `selection` admits from `report`, and whether they fail the
/// gate.
///
/// A structural failure judging the repository wrong is a verdict, reported beside
/// the findings. A selection error, an unloadable rule set, and any other
/// structural failure are faults the checker cannot judge through, so they return
/// `Err`.
fn selected_gate_findings(
    report: &crate::validation::ValidationReport,
    selection: &crate::validation::report::Selection,
) -> Result<(Vec<GateFinding>, bool)> {
    use crate::validation::report::{BuiltInPass, StructuralFailure};

    let selected = match report.select(selection) {
        Ok(selected) => selected,
        Err(error) => {
            let StructuralFailure { failure, findings } = error.downcast()?;
            if !report.is_judged_invalid() {
                anyhow::bail!("{failure:#}");
            }
            let mut gate_findings = gate_findings_from_rule_report(&findings);
            gate_findings.push(GateFinding::builtin(
                "repository-invalid",
                FindingSeverity::High,
                FindingDisposition::Blocking,
                FindingOrigin::PreExisting,
                format!("{failure:#}"),
            ));
            return Ok((gate_findings, true));
        }
    };
    let rules_file = selected
        .findings
        .iter()
        .filter(|finding| finding.rule == BuiltInPass::RulesFile.as_str())
        .map(|finding| finding.message.as_str())
        .collect::<Vec<_>>();
    if !rules_file.is_empty() {
        anyhow::bail!("rules declaration did not load: {}", rules_file.join("; "));
    }
    Ok((
        gate_findings_from_rule_report(&selected),
        selected.has_errors(),
    ))
}

/// The target named by the gated issue's single `<namespace>:<target-id>` label:
/// a full id or unique prefix, as templates write short ids. A missing, repeated,
/// or malformed target is a configuration fault.
fn label_target(
    captured: &crate::validation::CapturedValidation<'_>,
    issue_id: &IssueId,
    label_namespace: &str,
) -> Result<IssueIdPrefix> {
    if label_namespace.is_empty() {
        anyhow::bail!("label_namespace is empty");
    }
    let issue = captured
        .issues
        .as_deref()
        .map_err(|error| anyhow!("{error:#}"))?
        .iter()
        .find(|issue| &issue.id == issue_id)
        .ok_or_else(|| crate::storage::IssueNotFoundError::new(issue_id.clone()))?;
    let prefix = format!("{label_namespace}:");
    let targets: Vec<&str> = issue
        .labels
        .iter()
        .filter_map(|label| label.strip_prefix(&prefix))
        .collect();
    let [target] = targets[..] else {
        anyhow::bail!(
            "expected exactly one '{label_namespace}:<target-id>' label on issue {issue_id}, found {}",
            targets.len()
        );
    };
    target
        .parse::<IssueIdPrefix>()
        .with_context(|| format!("label '{label_namespace}:{target}' on issue {issue_id}"))
}

/// The `error` run of a built-in checker that could not judge the repository,
/// distinct from a `failed` verdict on its content.
fn builtin_fault_run(
    GateRunSubject {
        run_id,
        gate_key,
        issue_id,
        stage,
    }: GateRunSubject<'_>,
    command: &str,
    started_at: chrono::DateTime<chrono::Utc>,
    start: std::time::Instant,
    fault: &anyhow::Error,
) -> GateRunResult {
    let summary = format!("Built-in checker could not judge the repository: {fault:#}");
    GateRunResult {
        schema_version: crate::domain::GATE_RUN_SCHEMA_VERSION,
        run_id: run_id.clone(),
        gate_key: gate_key.clone(),
        stage,
        issue_id: issue_id.clone(),
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

impl<S: IssueStore> GateOps<'_, S> {
    /// Repository root used as the checker working directory and as the `git`
    /// context for stamping [`GateRunResult::commit`](crate::domain::GateRunResult).
    ///
    /// File-backed stores use the explicit repository layout, never the selected
    /// data-root parent. In-memory tests retain the current-directory fallback.
    pub(crate) fn checker_repo_root(&self) -> Result<std::path::PathBuf> {
        if !self.exec.storage.is_file_backed() {
            return std::env::current_dir().context("Failed to resolve checker working directory");
        }
        self.exec
            .require_layout()
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
        if !self.exec.storage.is_file_backed() {
            return None;
        }
        self.exec.require_layout().ok()
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Capture the resolved issue and the inputs `gate_key` evaluates.
    fn capture_gate_evaluation_image(
        &self,
        session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
        issue_id: &IssueId,
        gate_key: &GateKey,
        run_paths: &[crate::repository_state::VirtualPath],
    ) -> Result<Option<(crate::repository_state::RepositoryImage, IssueId)>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::declarations::GateChecker;
        use crate::repository_state::{CaptureSpec, VirtualPath};
        use std::collections::BTreeMap;

        let spec = CaptureSpec::new(
            [VirtualPath::issue_record(issue_id)],
            super::validate::MUTATION_CAPTURE_BUDGET,
        )?;
        let Some(bound) = super::capture_discovering(session, spec, |_, _| Ok(()))? else {
            return Ok(None);
        };
        let issues = Vec::from_iter(captured_issue_record(&bound, issue_id)?);
        let target_id = super::require_issue_in_capture(&issues, issue_id)?;
        let mut paths = vec![
            VirtualPath::issue_record(&target_id),
            VirtualPath::GATES,
            VirtualPath::GATE_RUNS,
        ];
        paths.extend(run_paths.iter().cloned());
        let mut spec = bound.capture_spec().clone();
        spec.discover_paths(paths)?;
        spec.discover_tail(VirtualPath::EVENTS)?;
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
        let image = super::grow_discovering(session, initial, |_, spec| {
            spec.discover_paths(
                issue
                    .dependencies
                    .iter()
                    .map(VirtualPath::issue_record)
                    .collect::<Vec<_>>(),
            )?;
            if matches!(checker, GateChecker::Exec { .. }) {
                super::validate::discover_document_references(&issue.documents, spec)?;
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
                spec.discover_paths(
                    super::named_gate_run_records(&issue)
                        .map(|(_, _, path)| path)
                        .collect::<Vec<_>>(),
                )?;
            }
            Ok(())
        })?;
        Ok(image.map(|image| (image, target_id)))
    }
}

impl<S: IssueStore> GateOps<'_, S> {
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
    pub fn check_gate(&self, issue_id: &IssueId, gate_key: &GateKey) -> Result<GateRunResult>
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
        issue_id: &IssueId,
        gate_key: &GateKey,
        source: VerdictSource,
    ) -> Result<GateRunResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, MutationIntent};

        let requested = issue_id.clone();
        let layout = self.exec.require_layout()?;
        let mutation = super::production_mutation_context();
        let run_id = mutation.gate_run_id_at(0);
        let run_paths = super::gate_run_record_paths(&mutation, issue_id, 1);
        let (target_id, image, issue, issues, registry, runs, captured) =
            with_mutation_attempts("gate evaluation inputs", || {
                let mut session = self.exec.storage.open_mutation_session(layout.clone())?;
                let Some((image, target_id)) = self.exec.capture_gate_evaluation_image(
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
                    captured_gate_runs(&image, &issue)?
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
        let digest = self.exec.declared_inputs_digest(gate)?;
        let reused = match (source, digest.as_ref()) {
            (VerdictSource::ReuseUnchangedInputs, Some(digest)) => {
                with_mutation_attempts("gate verdict reuse", || {
                    let mut session = self.exec.storage.open_mutation_session(layout.clone())?;
                    Ok(
                        match captured_reusable_gate_run(session.as_mut(), &gate.key, digest)? {
                            Some(run) => AttemptOutcome::Done(run),
                            None => AttemptOutcome::Retry,
                        },
                    )
                })?
            }
            _ => None,
        };
        let mut result = match reused {
            Some(source_run) => reused_gate_run(&run_id, &source_run, &issue.id, gate.stage),
            None => self.exec.execute_captured_gate(CapturedGateExecution {
                run_id: &run_id,
                image: &image,
                issue: &issue,
                issues: &issues,
                gate,
                runs: &runs.results,
                prompt: captured.prompt.as_deref(),
            })?,
        };
        result.inputs_digest = digest;
        let record_paths = run_paths
            .iter()
            .cloned()
            .chain(super::gate_verdict_record_paths(&result)?)
            .collect::<Vec<_>>();

        with_mutation_attempts("gate evaluation record", || {
            let mut session = self.exec.storage.open_mutation_session(layout.clone())?;
            let Some((final_image, _)) = self.exec.capture_gate_evaluation_image(
                session.as_mut(),
                &target_id.clone(),
                gate_key,
                &record_paths,
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
                captured_gate_runs(&final_image, &final_issue)?
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
            final_issue.gates_status.insert(gate_key.clone(), state);
            let event = if result.status == GateRunStatus::Passed {
                EventBody::draft_gate_passed(target_id.clone(), final_gate.key.clone(), by)
            } else {
                EventBody::draft_gate_failed(target_id.clone(), final_gate.key.clone(), by)
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
            let plan = finalize(&layout, &final_image, &mutation, &intents, None)?;
            if let AttemptOutcome::Retry = classify_apply(session.apply(&plan), ())? {
                return Ok(AttemptOutcome::Retry);
            }
            Ok(AttemptOutcome::Done(result.clone()))
        })
    }
}

impl<S: IssueStore> CommandExecutor<S> {
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
    /// identical. Repository validation is also `None` whatever it declares: its
    /// verdict depends on the records the digest excludes.
    fn declared_inputs_digest(
        &self,
        gate: &crate::declarations::GateDefinition,
    ) -> Result<Option<crate::domain::repository_inputs::InputsDigest>> {
        if matches!(
            gate.checker,
            Some(crate::declarations::GateChecker::RepositoryValidation)
        ) {
            return Ok(None);
        }
        let (Some(inputs), Some(layout)) = (gate.inputs.as_ref(), self.gates().real_repo_layout())
        else {
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
    /// [`crate::declarations::GateChecker::RepositoryValidation`] instead runs the computation behind
    /// `jit validate`, which captures the evidence it needs from the live repository.
    fn execute_builtin_checker_with_repository_view(
        &self,
        subject: GateRunSubject<'_>,
        checker: &crate::declarations::GateChecker,
        repository_image: &crate::repository_state::RepositoryImage,
    ) -> Result<GateRunResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::declarations::GateChecker;
        use crate::validation::{report::Selection, CapturedValidation, ReportInputs};

        let GateRunSubject {
            run_id,
            gate_key,
            issue_id,
            stage,
        } = subject;
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
        let select_over_image = |captured: &CapturedValidation<'_>, selection: Selection| {
            let inputs = ReportInputs {
                now: chrono::Utc::now(),
                history: None,
            };
            selected_gate_findings(&crate::validation::report(captured, &inputs), &selection)
        };
        let judged: Result<(Vec<GateFinding>, bool)> = match checker {
            GateChecker::RepositoryValidation => self
                .validation()
                .validate_repository_report()
                .and_then(|report| selected_gate_findings(&report, &Selection::Whole)),
            GateChecker::IssueValidation => select_over_image(
                &CapturedValidation::from_image(repository_image),
                Selection::Issue(issue_id.into()),
            ),
            GateChecker::RuleValidation { rule } => select_over_image(
                &CapturedValidation::from_image(repository_image),
                Selection::Rule {
                    issue: issue_id.into(),
                    rule: rule.clone(),
                },
            ),
            GateChecker::LabelTargetValidation { label_namespace } => {
                let captured = CapturedValidation::from_image(repository_image);
                label_target(&captured, issue_id, label_namespace).and_then(|target| {
                    select_over_image(&captured, Selection::Scope(target.clone())).with_context(
                        || format!("label '{label_namespace}:{target}' on issue {issue_id}"),
                    )
                })
            }
            GateChecker::ReviewPlaceholder => Ok((
                vec![GateFinding::builtin(
                    crate::validation::report::BuiltInPass::ReviewPlaceholder.as_str(),
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
                    subject, command, started_at, start, &fault,
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
            run_id: run_id.clone(),
            gate_key: gate_key.clone(),
            stage,
            issue_id: issue_id.clone(),
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
}

impl<S: IssueStore> GateOps<'_, S> {
    /// Return the latest published run of `gate_key` on an issue: the run its
    /// recorded gate status was projected from, so this view and readiness agree.
    ///
    /// Returns `Ok(None)` if no run has been recorded since the gate became
    /// required.
    pub fn get_last_gate_run(
        &self,
        issue_id: &IssueId,
        gate_key: &GateKey,
    ) -> Result<Option<GateRunResult>> {
        let full_id = issue_id.clone();
        let issue = self.exec.storage.load_issue(&full_id)?;
        self.exec.projected_gate_run(&issue, gate_key)
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Load the run `issue`'s recorded status for `gate_key` names.
    fn projected_gate_run(
        &self,
        issue: &Issue,
        gate_key: &GateKey,
    ) -> Result<Option<GateRunResult>> {
        issue
            .gates_status
            .get(gate_key)
            .and_then(|state| state.run_id.as_ref())
            .map(|run_id| self.storage.load_gate_run_result(&issue.id, run_id))
            .transpose()
            .map(|run| run.filter(|run| &run.gate_key == gate_key))
    }

    fn build_captured_gate_context(
        &self,
        checker: &crate::declarations::GateChecker,
        input: &CapturedGateExecution<'_>,
        documents: &[crate::domain::DocumentReference],
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
        let mut issue_view =
            IssueShowResponse::from_issue(input.issue.clone(), enriched, input.runs);
        issue_view.documents = documents.to_vec();
        omit_current_gate_projection(&mut issue_view, &input.gate.key);
        Ok(Some(GateContext {
            schema_version: 1,
            prompt,
            issue: serde_json::to_value(issue_view)?,
            gate: serde_json::json!({
                "key": input.gate.key,
                "title": input.gate.title,
                "description": input.gate.description,
                "stage": input.gate.stage,
            }),
            run_history: compact_run_history_for_context(input.runs, &input.gate.key),
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
            .ok_or_else(|| anyhow!("Gate '{}' has no checker configured", input.gate.key))?;
        let repo_root = self.gates().checker_repo_root()?;
        let working_dir = match checker {
            crate::declarations::GateChecker::Exec {
                working_dir: Some(subdir),
                ..
            } => repo_root.join(subdir),
            _ => repo_root,
        };
        let documents = self.documents().issue_documents(input.issue)?;
        let context = self.build_captured_gate_context(checker, &input, &documents)?;
        let subject = GateRunSubject {
            run_id: input.run_id,
            gate_key: &input.gate.key,
            issue_id: &input.issue.id,
            stage: input.gate.stage,
        };
        match checker {
            crate::declarations::GateChecker::Exec { .. } => {
                let head = self
                    .require_worktree_paths()?
                    .git()
                    .map(|git| git.head())
                    .transpose()?
                    .flatten();
                self.storage.run_external_process(|| {
                    gate_execution::execute_gate_checker_with_context(
                        subject,
                        checker,
                        &working_dir,
                        head.as_ref(),
                        context.as_ref(),
                        &documents,
                    )
                })
            }
            _ => self.execute_builtin_checker_with_repository_view(subject, checker, input.image),
        }
    }
}

impl<S: IssueStore> GateOps<'_, S> {
    /// Load the run named by every gate state, without listing run history.
    ///
    /// Registered required gates come first on priority ties; unregistered
    /// states sort last. The second vector includes states and registered
    /// required gates with no run id, or whose named run belongs to another
    /// gate.
    pub fn get_last_gate_runs_for_issue(
        &self,
        issue_id: &IssueId,
    ) -> Result<(Vec<GateRunResult>, Vec<GateKey>)> {
        let full_id = issue_id.clone();
        let issue = self.exec.storage.load_issue(&full_id)?;
        let registry = self.exec.storage.load_gate_registry()?;

        let mut gates: Vec<_> = issue
            .gates_required
            .iter()
            .filter(|key| registry.gates.contains_key(*key))
            .collect();
        let remaining: Vec<_> = issue
            .gates_status
            .keys()
            .filter(|key| !gates.contains(key))
            .collect();
        gates.extend(remaining);
        gates.sort_by_key(|key| {
            let priority = registry.gates.get(*key).map(|gate| gate.priority);
            (priority.is_none(), priority)
        });

        gates.into_iter().try_fold(
            (Vec::new(), Vec::new()),
            |(mut results, mut not_run), gate_key| {
                match self.exec.projected_gate_run(&issue, gate_key)? {
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
        issue_id: &IssueId,
    ) -> Result<Vec<(GateKey, GateStatus)>> {
        let full_id = issue_id.clone();
        let issue = self.exec.storage.load_issue(&full_id)?;
        let registry = self.exec.storage.load_gate_registry()?;

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
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Execute prechecks from one closed repository image without publishing.
    pub(super) fn execute_captured_prechecks(
        &self,
        image: &crate::repository_state::RepositoryImage,
        issue: &Issue,
        registry: &crate::declarations::GateRegistry,
        captured_prompts: &HashMap<GateKey, String>,
        mutation: &crate::repository_state::MutationContext,
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
            captured_gate_runs(image, issue)?.results
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
                    let run_id = mutation.gate_run_id_at(runs.len() as u64);
                    let result = match self.execute_captured_gate(CapturedGateExecution {
                        run_id: &run_id,
                        image,
                        issue: &projected_issue,
                        issues: &issues,
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
                    replace_named_run(&mut history, result.clone());
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
}

#[cfg(test)]
mod tests {
    use super::{
        compact_run_history_for_context, omit_current_gate_projection, replace_named_run,
        IssueIdPrefix,
    };
    use crate::commands::test_helpers::gate_reuse::{seed_gate_issue, ReuseFixture};
    use crate::commands::test_helpers::{seed_gate_registry, seed_issue, seed_repo_file, setup};
    use crate::commands::IssueShowResponse;
    use crate::commands::{CommandExecutor, GateUpdate};
    use crate::declarations::{GateChecker, GateMode, GateStage};
    use crate::domain::gate_findings::GateFindings;
    use crate::domain::identity::{GateKey, GateRunId, IssueId};
    use crate::domain::{GateRunResult, GateRunStatus, GateStatus, State, GATE_RUN_SCHEMA_VERSION};
    use crate::gate_execution::GateRunSubject;
    use crate::repository_state::VirtualPath;
    use crate::storage::{InMemoryStorage, IssueStore, JsonFileStorage, MutationPoint, RedoPoint};
    use crate::test_utils::add_reducing_edge;
    use crate::test_utils::document_reference;
    use chrono::{TimeZone, Utc};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn validation_image<S: IssueStore + crate::storage::RepositoryStateStore>(
        executor: &CommandExecutor<S>,
    ) -> crate::repository_state::RepositoryImage {
        crate::commands::with_mutation_session(
            executor.storage(),
            &executor.require_layout().unwrap(),
            "checker fixture capture",
            |session| {
                Ok(match executor.validation().capture_validation(session)? {
                    Some(captured) => {
                        crate::commands::SessionStep::Done(captured.image.into_owned())
                    }
                    None => crate::commands::SessionStep::Retry,
                })
            },
        )
        .unwrap()
    }

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
        gate_key: &GateKey,
        checker: GateChecker,
        labels: Vec<String>,
    ) -> IssueId {
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            gate_key.clone(),
            crate::declarations::GateDefinition {
                version: 1,
                key: gate_key.clone(),
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

        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        issue.labels = labels;
        issue.state = State::Ready;
        issue.gates_required.push(gate_key.clone());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        issue_id
    }

    fn rule_validation_test_rules() -> String {
        use crate::declarations::rules::ChildLink;
        format!(
            r#"
[[rules]]
name = "coverage-preview"
when = {{ type = "breakdown" }}
severity = "error"
assert = {{ label-coverage = {{ marker = "[hard]", id-pattern = "REQ-[0-9]+", satisfies-namespace = "satisfies", child-link = "{dependencies}", container-from-label = "brackets" }} }}

[[rules]]
name = "unrelated-local-failure"
when = {{ type = "task" }}
severity = "error"
assert = {{ require-label = {{ label = "required:*", min = 1 }} }}

[[rules]]
name = "unrelated-graph-failure"
when = {{ type = "epic" }}
severity = "error"
assert = {{ label-coverage = {{ marker = "[hard]", child-link = "{any}" }} }}

[[rules]]
name = "unrelated-label-reference"
when = {{ type = "task" }}
severity = "error"
assert = {{ label-reference = {{ from = "satisfies", to = "req" }} }}
"#,
            dependencies = ChildLink::Dependencies,
            any = ChildLink::Any,
        )
    }

    fn seed_rule_validation_rules(executor: &CommandExecutor<InMemoryStorage>) {
        std::fs::write(
            executor.storage.root().join("rules.toml"),
            rule_validation_test_rules(),
        )
        .unwrap();
        seed_repo_file(
            &executor.storage,
            ".jit/rules.toml",
            &rule_validation_test_rules(),
        );
        let rules_path = executor
            .storage
            .repository_layout()
            .resolve(&crate::repository_state::VirtualPath::RULES)
            .unwrap();
        std::fs::create_dir_all(rules_path.parent().unwrap()).unwrap();
        std::fs::write(rules_path, rule_validation_test_rules()).unwrap();
    }

    fn prior_run(
        run_id: &GateRunId,
        gate_key: &GateKey,
        started_at: i64,
        findings: Option<GateFindings>,
    ) -> GateRunResult {
        GateRunResult {
            schema_version: GATE_RUN_SCHEMA_VERSION,
            run_id: run_id.clone(),
            gate_key: gate_key.clone(),
            stage: GateStage::Postcheck,
            issue_id: crate::test_utils::issue_id(0xc4e9_59a0_b8a4_228d_7550_22a2_872f_f725),
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
    fn test_compact_run_history_selects_the_gates_run_among_other_gates_runs() {
        let named = prior_run(
            &crate::test_utils::gate_run_id(1),
            &"review".parse().unwrap(),
            20,
            None,
        );
        let runs = vec![
            prior_run(
                &crate::test_utils::gate_run_id(2),
                &"cargo-ci".parse().unwrap(),
                40,
                None,
            ),
            named.clone(),
        ];

        let compact = compact_run_history_for_context(&runs, &"review".parse().unwrap());

        assert_eq!(compact.len(), 1);
        assert_eq!(compact[0].run_id, named.run_id);
        assert_eq!(compact[0].stdout, named.stdout);
        assert!(compact[0].stderr.is_empty());
        assert_eq!(runs[1], named, "source run must remain unchanged");
    }

    #[test]
    fn test_replace_named_run_makes_the_fresh_result_its_gates_history() {
        let review: GateKey = "review".parse().unwrap();
        let other = prior_run(
            &crate::test_utils::gate_run_id(2),
            &"cargo-ci".parse().unwrap(),
            10,
            None,
        );
        let mut history = vec![
            prior_run(&crate::test_utils::gate_run_id(1), &review, 10, None),
            other.clone(),
        ];
        let fresh = prior_run(&crate::test_utils::gate_run_id(3), &review, 20, None);

        replace_named_run(&mut history, fresh.clone());

        assert_eq!(
            compact_run_history_for_context(&history, &review)
                .into_iter()
                .map(|run| run.run_id)
                .collect::<Vec<_>>(),
            vec![fresh.run_id]
        );
        assert_eq!(history.len(), 2);
        assert!(history.contains(&other));
    }

    #[test]
    fn test_compact_run_history_prefers_structured_findings_over_stdout() {
        let findings = GateFindings {
            verdict: "fail".to_string(),
            summary: "one defect".to_string(),
            findings: Vec::new(),
        };
        let source = prior_run(
            &crate::test_utils::gate_run_id(5),
            &"review".parse().unwrap(),
            10,
            Some(findings.clone()),
        );

        let compact = compact_run_history_for_context(
            std::slice::from_ref(&source),
            &"review".parse().unwrap(),
        );

        assert_eq!(compact.len(), 1);
        assert_eq!(compact[0].findings, Some(findings));
        assert!(compact[0].stdout.is_empty());
        assert!(compact[0].stderr.is_empty());
        assert_eq!(source.stdout, format!("stdout-{}", source.run_id));
        assert_eq!(source.stderr, format!("stderr-{}", source.run_id));
    }

    #[test]
    fn test_compact_run_history_is_empty_for_a_gate_without_a_named_run() {
        let run = prior_run(
            &crate::test_utils::gate_run_id(6),
            &"cargo-ci".parse().unwrap(),
            10,
            None,
        );

        assert!(compact_run_history_for_context(&[run], &"review".parse().unwrap()).is_empty());
    }

    #[test]
    fn test_omit_current_gate_projection_retains_other_gate_evidence() {
        let retained = "cargo-ci".parse().unwrap();
        let removed: GateKey = "review".parse().unwrap();
        let mut issue = crate::test_utils::fixture_issue("Test".into(), String::new());
        issue.gates_required = vec![retained, removed.clone()];
        let mut view = IssueShowResponse::from_issue(issue, Vec::new(), &[]);
        let expected = serde_json::to_value(&view.gates[0]).unwrap();

        omit_current_gate_projection(&mut view, &removed);

        assert_eq!(
            serde_json::to_value(&view.gates).unwrap(),
            serde_json::json!([expected])
        );
    }

    #[test]
    fn test_check_gate_automated_success() {
        let executor = setup();

        // Define an automated gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "test-gate".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "test-gate".parse().unwrap(),
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
        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"test-gate".parse().unwrap())
            .unwrap();

        // Check the gate
        let result = executor
            .gates()
            .check_gate(&issue_id, &"test-gate".parse().unwrap())
            .unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
        assert_eq!(result.exit_code, Some(0));

        // Verify gate status was updated on issue
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        let gate_state = issue
            .gates_status
            .get(&"test-gate".parse().unwrap())
            .unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Passed);
    }

    #[test]
    fn test_check_gate_records_selected_worktree_head_from_nested_checker_repository() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (temp, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let worktree = crate::test_harness::git::GitRepository::init(temp.path());
        let selected_commit = worktree.commit("selected worktree");
        let selected_branch = worktree.git(&["branch", "--show-current"]);
        let nested = crate::test_harness::git::GitRepository::init(temp.path().join("nested"));
        nested.write("seed.txt", "nested\n");
        let nested_commit = nested.commit("nested checker repository");
        assert_ne!(selected_commit, nested_commit);
        let paths = crate::storage::WorktreePaths::detect_from(temp.path()).unwrap();
        let executor = executor.with_worktree_paths(paths);
        let gate_key: GateKey = "nested-check".parse().unwrap();
        executor
            .gates()
            .define_gate(
                &gate_key,
                "Nested check".to_string(),
                "Nested check".to_string(),
                GateStage::Postcheck,
                GateMode::Auto,
                Some(GateChecker::Exec {
                    command: "true".to_string(),
                    timeout_seconds: 10,
                    working_dir: Some("nested".to_string()),
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                100,
            )
            .unwrap();
        let issue_id = seed_gate_issue(&executor, &taxonomy, &gate_key, "Test", "Test");

        let run = executor.gates().check_gate(&issue_id, &gate_key).unwrap();

        assert_eq!(run.status, GateRunStatus::Passed);
        assert_eq!(run.commit.as_deref(), Some(selected_commit.as_str()));
        assert_eq!(run.branch.as_deref(), Some(selected_branch.as_str()));
    }

    #[test]
    fn test_check_gate_twice_names_the_second_run_in_gate_state() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "test-gate".parse().unwrap(),
            make_auto_gate(&"test-gate".parse().unwrap(), "exit 0"),
        );
        seed_gate_registry(executor.storage(), &registry);
        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        let gate_key: GateKey = "test-gate".parse().unwrap();
        executor.gates().add_gate(&issue_id, &gate_key).unwrap();

        let first = executor.gates().check_gate(&issue_id, &gate_key).unwrap();
        let second = executor.gates().check_gate(&issue_id, &gate_key).unwrap();

        assert_ne!(first.run_id, second.run_id);
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue.gates_status[&gate_key].run_id.as_ref(),
            Some(&second.run_id)
        );
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
            &"durability-check".parse().unwrap(),
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

        let result = executor
            .gates()
            .check_gate(&issue_id, &"durability-check".parse().unwrap())
            .unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);

        let runs = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap();
        assert!(
            runs.iter().any(|run| run.run_id == result.run_id
                && run.gate_key == "durability-check".parse().unwrap()
                && run.status == GateRunStatus::Passed),
            "the returned run must already be recorded: {runs:?}"
        );
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().gates_status
                [&"durability-check".parse().unwrap()]
                .status,
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
                        if subject == &issue_id && gate_key == &"durability-check".parse::<GateKey>().unwrap()
                )),
            "the evaluation must already have logged its gate-passed event"
        );
    }

    #[test]
    fn test_review_placeholder_passes_with_unmistakable_structured_warning() {
        let executor = setup();
        let issue_id = add_builtin_gate(
            &executor,
            &"arbitrary-review-key".parse().unwrap(),
            GateChecker::ReviewPlaceholder,
            vec!["type:task".to_string()],
        );

        let result = executor
            .gates()
            .check_gate(&issue_id, &"arbitrary-review-key".parse().unwrap())
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
            &"editable-review".parse().unwrap(),
            GateChecker::ReviewPlaceholder,
            vec!["type:task".to_string()],
        );

        let outcome = executor
            .gates()
            .pass_gate(&issue_id, &"editable-review".parse().unwrap(), None, true)
            .unwrap();

        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("WITHOUT RUNNING A REVIEWER"));
    }

    #[test]
    fn test_issue_validation_runs_in_process_under_arbitrary_gate_key() {
        let executor = setup();
        let issue_id = add_builtin_gate(
            &executor,
            &"not-a-reserved-validation-key".parse().unwrap(),
            GateChecker::IssueValidation,
            vec!["type:task".to_string()],
        );

        let result = executor
            .gates()
            .check_gate(&issue_id, &"not-a-reserved-validation-key".parse().unwrap())
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
            &"not-a-reserved-repository-key".parse().unwrap(),
            GateChecker::RepositoryValidation,
            vec!["type:task".to_string()],
        );

        let result = executor
            .gates()
            .check_gate(&issue_id, &"not-a-reserved-repository-key".parse().unwrap())
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

    fn seed_file_repository(
        executor: &CommandExecutor<JsonFileStorage>,
        taxonomy: &crate::test_taxonomy::TestTaxonomy,
        issue_title: &str,
        issue_description: &str,
    ) -> IssueId {
        executor
            .gates()
            .define_gate(
                &"not-a-reserved-repository-key".parse().unwrap(),
                "Portable check".to_string(),
                "Portable check".to_string(),
                GateStage::Postcheck,
                GateMode::Auto,
                Some(GateChecker::RepositoryValidation),
                100,
            )
            .unwrap();
        executor
            .gates()
            .define_gate(
                &"editable-review".parse().unwrap(),
                "Portable check".to_string(),
                "Portable check".to_string(),
                GateStage::Postcheck,
                GateMode::Auto,
                Some(GateChecker::ReviewPlaceholder),
                100,
            )
            .unwrap();
        seed_gate_issue(
            executor,
            taxonomy,
            &"not-a-reserved-repository-key".parse().unwrap(),
            issue_title,
            issue_description,
        )
    }

    #[test]
    fn test_issue_validation_records_a_failed_rule_pass_as_an_error_run() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (_repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::invalid_hierarchy_config_fragment()
        );
        std::fs::write(executor.storage().root().join("config.toml"), &config).unwrap();
        let image = validation_image(&executor);

        let run = executor
            .execute_builtin_checker_with_repository_view(
                GateRunSubject {
                    run_id: &crate::test_utils::gate_run_id(1),
                    gate_key: &"not-a-reserved-repository-key".parse().unwrap(),
                    issue_id: &issue_id,
                    stage: GateStage::Postcheck,
                },
                &GateChecker::IssueValidation,
                &image,
            )
            .unwrap();

        assert_eq!(
            run.status,
            GateRunStatus::Error,
            "{} {}",
            run.stdout,
            run.stderr
        );
    }

    /// REQ-03/REQ-04: a second issue's evaluation over an unchanged declared
    /// input set carries the first run's verdict, names it, and never reaches
    /// the checker.
    #[test]
    fn test_check_gate_reuses_a_recorded_verdict_over_identical_declared_inputs() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let fixture = ReuseFixture {
            repo,
            executor,
            gate_key: "workspace-check".parse().unwrap(),
        };
        fixture.seed(Some((&["workspace"], &[])), 0);
        let first_issue = fixture.issue("Test", "Test");
        let second_issue = fixture.issue("Test", "Test");

        let executed = fixture.evaluate(&first_issue);
        let reused = fixture.evaluate(&second_issue);

        assert_eq!(fixture.executions(), 1, "the checker must run exactly once");
        assert!(executed.origin.is_executed());
        assert_eq!(
            reused.origin,
            crate::domain::GateVerdictOrigin::Reused {
                source_issue: executed.issue_id.clone(),
                source_run: executed.run_id.clone(),
            }
        );
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
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let fixture = ReuseFixture {
            repo,
            executor,
            gate_key: "workspace-check".parse().unwrap(),
        };
        fixture.seed(Some((&["workspace"], &[])), 1);
        let first_issue = fixture.issue("Test", "Test");
        let second_issue = fixture.issue("Test", "Test");

        let executed = fixture.evaluate(&first_issue);
        let reused = fixture.evaluate(&second_issue);

        assert_eq!(executed.status, GateRunStatus::Failed);
        assert_eq!(fixture.executions(), 1);
        assert_eq!(reused.status, GateRunStatus::Failed);
        assert_eq!(reused.origin.source_run(), Some(&executed.run_id));
    }

    /// REQ-06: an edit, an addition, and a removal beneath a declared root each
    /// change the digest, so the next evaluation executes the checker.
    #[test]
    fn test_check_gate_executes_again_after_declared_inputs_change() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let fixture = ReuseFixture {
            repo,
            executor,
            gate_key: "workspace-check".parse().unwrap(),
        };
        fixture.seed(Some((&["workspace"], &[])), 0);
        let first = fixture.evaluate(&fixture.issue("Test", "Test"));
        assert_eq!(fixture.executions(), 1);

        fixture.write("workspace/src/lib.rs", "fn main() { /* edited */ }\n");
        let after_edit = fixture.evaluate(&fixture.issue("Test", "Test"));
        assert_eq!(fixture.executions(), 2, "an edited input must re-execute");
        assert!(after_edit.origin.is_executed());
        assert_ne!(after_edit.inputs_digest, first.inputs_digest);

        fixture.write("workspace/src/added.rs", "fn added() {}\n");
        let after_add = fixture.evaluate(&fixture.issue("Test", "Test"));
        assert_eq!(fixture.executions(), 3, "an added input must re-execute");
        assert_ne!(after_add.inputs_digest, after_edit.inputs_digest);

        fixture.remove("workspace/src/lib.rs");
        let after_remove = fixture.evaluate(&fixture.issue("Test", "Test"));
        assert_eq!(fixture.executions(), 4, "a removed input must re-execute");
        assert_ne!(after_remove.inputs_digest, after_add.inputs_digest);
    }

    /// Restoring an input set a checker has already been run over reuses that
    /// run: the digest identifies content, not the sequence of edits that
    /// produced it.
    #[test]
    fn test_check_gate_reuses_after_declared_inputs_return_to_a_verified_state() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let fixture = ReuseFixture {
            repo,
            executor,
            gate_key: "workspace-check".parse().unwrap(),
        };
        fixture.seed(Some((&["workspace"], &[])), 0);
        let original = fixture.evaluate(&fixture.issue("Test", "Test"));

        fixture.write("workspace/src/lib.rs", "fn main() { /* edited */ }\n");
        fixture.evaluate(&fixture.issue("Test", "Test"));
        assert_eq!(fixture.executions(), 2);

        fixture.write("workspace/src/lib.rs", "fn main() {}\n");
        let restored = fixture.evaluate(&fixture.issue("Test", "Test"));

        assert_eq!(fixture.executions(), 2, "the restored content was verified");
        assert_eq!(restored.inputs_digest, original.inputs_digest);
        assert_eq!(restored.origin.source_run(), Some(&original.run_id));
    }

    /// Reuse reads only the verdict record for its key and the run it names.
    #[test]
    fn test_check_gate_reuses_while_every_other_run_and_verdict_record_is_unparseable() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let fixture = ReuseFixture {
            repo,
            executor,
            gate_key: "workspace-check".parse().unwrap(),
        };
        fixture.seed(Some((&["workspace"], &[])), 0);
        let source = fixture.evaluate(&fixture.issue("Test", "Test"));
        fixture.write("workspace/src/lib.rs", "fn main() { /* edited */ }\n");
        fixture.evaluate(&fixture.issue("Test", "Test"));
        fixture.write("workspace/src/lib.rs", "fn main() {}\n");
        let reusing = fixture.issue("Test", "Test");
        let layout = fixture.executor.require_layout().unwrap();
        let kept = [
            crate::repository_state::VirtualPath::gate_verdict_record(
                &fixture.gate_key,
                source.inputs_digest.as_ref().unwrap(),
            ),
            crate::repository_state::VirtualPath::gate_run_record(&source.issue_id, &source.run_id),
        ]
        .map(|path| layout.resolve(&path).unwrap());
        fn corrupt_files_except(dir: &std::path::Path, kept: &[std::path::PathBuf]) -> usize {
            std::fs::read_dir(dir)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .map(|path| match path.is_dir() {
                    true => corrupt_files_except(&path, kept),
                    false if kept.contains(&path) => 0,
                    false => {
                        std::fs::write(&path, "{ not json").unwrap();
                        1
                    }
                })
                .sum()
        }
        let corrupted = corrupt_files_except(
            &layout
                .resolve(&crate::repository_state::VirtualPath::GATE_RUNS)
                .unwrap(),
            &kept,
        );
        assert_eq!(corrupted, 2, "the other run and its verdict record");

        let reused = fixture.evaluate(&reusing);

        assert_eq!(fixture.executions(), 2);
        assert_eq!(reused.origin.source_run(), Some(&source.run_id));
    }

    /// REQ-07: a change to a file no declared root covers leaves the digest
    /// alone, so the next evaluation reuses instead of re-executing.
    #[test]
    fn test_check_gate_reuses_after_a_change_no_declared_root_covers() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let fixture = ReuseFixture {
            repo,
            executor,
            gate_key: "workspace-check".parse().unwrap(),
        };
        fixture.seed(Some((&["workspace"], &[])), 0);
        let executed = fixture.evaluate(&fixture.issue("Test", "Test"));

        fixture.write("elsewhere.txt", "rewritten\n");
        let reused = fixture.evaluate(&fixture.issue("Test", "Test"));

        assert_eq!(fixture.executions(), 1);
        assert_eq!(reused.inputs_digest, executed.inputs_digest);
        assert_eq!(reused.origin.source_run(), Some(&executed.run_id));
    }

    /// REQ-07: an excluded path beneath a declared root is outside the set too,
    /// so changing it reuses.
    #[test]
    fn test_check_gate_reuses_after_a_change_a_declared_exclusion_matches() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let fixture = ReuseFixture {
            repo,
            executor,
            gate_key: "workspace-check".parse().unwrap(),
        };
        fixture.seed(Some((&["workspace"], &["workspace/notes/**"])), 0);
        fixture.write("workspace/notes/scratch.md", "before\n");
        let executed = fixture.evaluate(&fixture.issue("Test", "Test"));

        fixture.write("workspace/notes/scratch.md", "after\n");
        let reused = fixture.evaluate(&fixture.issue("Test", "Test"));

        assert_eq!(fixture.executions(), 1);
        assert_eq!(reused.origin.source_run(), Some(&executed.run_id));
    }

    /// REQ-08: a gate declaring no inputs has nothing to prove unchanged, so
    /// every evaluation executes its checker and records no digest.
    #[test]
    fn test_check_gate_executes_every_time_for_a_gate_declaring_no_inputs() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let fixture = ReuseFixture {
            repo,
            executor,
            gate_key: "workspace-check".parse().unwrap(),
        };
        fixture.seed(None, 0);

        let first = fixture.evaluate(&fixture.issue("Test", "Test"));
        let second = fixture.evaluate(&fixture.issue("Test", "Test"));

        assert_eq!(fixture.executions(), 2);
        assert!(first.inputs_digest.is_none());
        assert!(second.origin.is_executed());
    }

    /// REQ-08: an explicit re-run request executes the checker even where the
    /// declared inputs would otherwise license reuse.
    #[test]
    fn test_check_gate_with_execute_source_ignores_a_reusable_verdict() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let fixture = ReuseFixture {
            repo,
            executor,
            gate_key: "workspace-check".parse().unwrap(),
        };
        fixture.seed(Some((&["workspace"], &[])), 0);
        let executed = fixture.evaluate(&fixture.issue("Test", "Test"));

        let forced = fixture
            .executor
            .gates()
            .check_gate_with(
                &fixture.issue("Test", "Test"),
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

    /// Replaces a verdict record at the first commit check, as an evaluation
    /// racing to record the same key would.
    struct RacingVerdictWriter {
        path: std::path::PathBuf,
        bytes: Vec<u8>,
        commit_checks: std::sync::atomic::AtomicUsize,
    }

    impl crate::storage::MutationInterruptionInjector for RacingVerdictWriter {
        fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
            use std::sync::atomic::Ordering;
            if point == &MutationPoint::BeforeCommitCheck
                && self.commit_checks.fetch_add(1, Ordering::SeqCst) == 0
            {
                std::fs::write(&self.path, &self.bytes)?;
            }
            Ok(())
        }
    }

    #[test]
    fn test_check_gate_with_retries_a_verdict_record_replaced_by_a_racing_evaluation() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let fixture = ReuseFixture {
            repo,
            executor,
            gate_key: "workspace-check".parse().unwrap(),
        };
        fixture.seed(Some((&["workspace"], &[])), 0);
        let first = fixture.evaluate(&fixture.issue("Test", "Test"));
        let layout = fixture.executor.require_layout().unwrap();
        let record = layout
            .resolve(&crate::repository_state::VirtualPath::gate_verdict_record(
                &fixture.gate_key,
                first.inputs_digest.as_ref().unwrap(),
            ))
            .unwrap();
        let racer = Arc::new(RacingVerdictWriter {
            path: record.clone(),
            bytes: crate::repository_state::serialize_gate_verdict(
                &crate::domain::GateVerdictRecord {
                    issue_id: first.issue_id.clone(),
                    run_id: crate::test_utils::gate_run_id(9),
                },
            )
            .unwrap(),
            commit_checks: Default::default(),
        });
        let racing = CommandExecutor::new(JsonFileStorage::with_mutation_interruptions(
            fixture.executor.storage.root(),
            racer.clone(),
        ))
        .with_layout(layout)
        .unwrap();

        let forced = racing
            .gates()
            .check_gate_with(
                &fixture.issue("Test", "Test"),
                &fixture.gate_key,
                super::VerdictSource::Execute,
            )
            .expect("the evaluation retries its record");

        let stored: crate::domain::GateVerdictRecord =
            serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
        assert_eq!(stored.run_id, forced.run_id);
        assert_eq!(
            fixture.executions(),
            2,
            "the retry reuses the checker's verdict"
        );
        assert!(
            racer
                .commit_checks
                .load(std::sync::atomic::Ordering::SeqCst)
                > 1,
            "the replaced record must fail the first commit check"
        );
    }

    /// REQ-09: a wave of issues evaluated over one unchanged source state
    /// executes the checker once, and every other issue's record names that one
    /// run rather than claiming a verification of its own.
    #[test]
    fn test_check_gate_executes_once_across_a_wave_over_one_unchanged_tree() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let fixture = ReuseFixture {
            repo,
            executor,
            gate_key: "workspace-check".parse().unwrap(),
        };
        fixture.seed(Some((&["workspace"], &[])), 0);
        let wave: Vec<GateRunResult> = (0..5)
            .map(|_| fixture.evaluate(&fixture.issue("Test", "Test")))
            .collect();

        assert_eq!(fixture.executions(), 1);
        let executed: Vec<&GateRunResult> =
            wave.iter().filter(|run| run.origin.is_executed()).collect();
        assert_eq!(executed.len(), 1, "one checker execution across the wave");
        assert!(
            wave.iter()
                .filter(|run| !run.origin.is_executed())
                .all(|run| run.origin.source_run() == Some(&executed[0].run_id)),
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
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let fixture = ReuseFixture {
            repo,
            executor,
            gate_key: "workspace-check".parse().unwrap(),
        };
        fixture.seed(Some((&["workspace"], &[])), 127);

        let errored = fixture.evaluate(&fixture.issue("Test", "Test"));
        let retried = fixture.evaluate(&fixture.issue("Test", "Test"));

        assert_eq!(errored.status, GateRunStatus::Error);
        assert_eq!(fixture.executions(), 2, "an errored run must not be reused");
        assert!(retried.origin.is_executed());
    }

    /// A verdict is a function of the checker as well as its inputs: editing
    /// the checker over an unchanged tree executes rather than inheriting a
    /// verdict the edited checker never produced.
    #[test]
    fn test_check_gate_executes_again_after_the_checker_declaration_changes() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let fixture = ReuseFixture {
            repo,
            executor,
            gate_key: "workspace-check".parse().unwrap(),
        };
        fixture.seed(Some((&["workspace"], &[])), 0);
        let before = fixture.evaluate(&fixture.issue("Test", "Test"));

        fixture.rewrite_checker_command(&format!(
            "printf xx >> '{}'; exit 0",
            fixture.repo.path().join("executions.log").to_string_lossy()
        ));
        let after = fixture.evaluate(&fixture.issue("Test", "Test"));

        assert_eq!(
            fixture.executions(),
            3,
            "the edited checker must run rather than inherit the old verdict"
        );
        assert!(after.origin.is_executed());
        assert_ne!(after.inputs_digest, before.inputs_digest);
    }

    fn add_exec_gate<S: IssueStore + crate::storage::RepositoryStateStore>(
        executor: &CommandExecutor<S>,
        issue_id: &IssueId,
        gate_key: &GateKey,
        title: &str,
        command: String,
        prompt_file: Option<String>,
    ) {
        executor
            .gates()
            .define_gate(
                gate_key,
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
        executor.gates().add_gate(issue_id, gate_key).unwrap();
    }

    /// Make `issue_id`'s state of `gate_key` name `run_id`, written past the
    /// executor so no record of that run need exist.
    fn name_gate_run(
        executor: &CommandExecutor<JsonFileStorage>,
        issue_id: &IssueId,
        gate_key: &GateKey,
        run_id: &GateRunId,
    ) {
        let mut issue = executor.storage.load_issue(issue_id).unwrap();
        issue.gates_status.insert(
            gate_key.clone(),
            crate::domain::GateState {
                status: crate::domain::GateStatus::Failed,
                updated_by: None,
                updated_at: Utc::now(),
                run_id: Some(run_id.clone()),
            },
        );
        let issue_path = executor
            .require_layout()
            .unwrap()
            .resolve(&VirtualPath::issue_record(issue_id))
            .unwrap();
        std::fs::write(
            issue_path,
            crate::repository_state::serialize_issue(&issue).unwrap(),
        )
        .unwrap();
    }

    fn corrupt_gate_run_record(issue: &IssueId) -> crate::repository_state::VirtualPath {
        crate::repository_state::VirtualPath::gate_run_record(
            issue,
            &crate::test_utils::gate_run_id(0xc0),
        )
    }

    fn write_corrupt_gate_run(repo: &std::path::Path, issue: &IssueId) {
        let path = repo
            .join(".jit")
            .join(corrupt_gate_run_record(issue).relative().as_path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "{ corrupt").unwrap();
    }

    /// Shell commands a checker runs to publish `concurrent-run.json` as the
    /// record of `issue`'s run `run_id`.
    fn publish_concurrent_run_script(issue: &IssueId, run_id: &GateRunId) -> String {
        use crate::repository_state::VirtualPath;

        format!(
            "mkdir -p .jit/{}; cp concurrent-run.json .jit/{}",
            VirtualPath::gate_run_issue_directory(issue)
                .relative()
                .as_str(),
            VirtualPath::gate_run_record(issue, run_id)
                .relative()
                .as_str(),
        )
    }

    fn capture_gate_evidence(
        executor: &CommandExecutor<JsonFileStorage>,
        issue_id: &IssueId,
        gate_key: &GateKey,
    ) -> super::GateEvaluationEvidence {
        use crate::storage::RepositoryStateStore;

        let layout = executor.require_layout().unwrap();
        let probe = crate::test_utils::gate_run_id(0xe0);
        let run_paths = [
            crate::repository_state::VirtualPath::gate_run_issue_directory(issue_id),
            crate::repository_state::VirtualPath::gate_run_record(issue_id, &probe),
        ];
        let mut session = executor.storage.open_mutation_session(layout).unwrap();
        let image = executor
            .capture_gate_evaluation_image(session.as_mut(), issue_id, gate_key, &run_paths)
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
            super::captured_gate_runs(&image, &issue).unwrap()
        } else {
            super::CapturedGateRuns::default()
        };
        super::captured_gate_evaluation(&image, &issue, gate_key, &registry.gates[gate_key], &runs)
            .unwrap()
            .evidence
    }

    #[test]
    fn test_exec_gate_ignores_unrelated_malformed_issue_and_unsafe_prompt() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (_repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        add_exec_gate(
            &executor,
            &issue_id,
            &"selected-safe".parse().unwrap(),
            "Selected safe",
            "true".to_string(),
            None,
        );
        add_exec_gate(
            &executor,
            &issue_id,
            &"unselected-unsafe".parse().unwrap(),
            "Unselected unsafe",
            "true".to_string(),
            Some("../../outside".to_string()),
        );
        let unrelated = crate::test_utils::issue_id(0x4000_8000_0000_0000_0001_0000_0000);
        let unrelated_path = executor
            .require_layout()
            .unwrap()
            .resolve(&VirtualPath::issue_record(&unrelated))
            .unwrap();
        std::fs::write(unrelated_path, []).unwrap();

        let result = executor
            .gates()
            .check_gate(&issue_id, &"selected-safe".parse().unwrap())
            .unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
    }

    #[test]
    fn test_capture_gate_evaluation_image_holds_only_the_evaluated_issues_named_runs() {
        use crate::storage::RepositoryStateStore;

        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (_repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Evaluated", "Evaluated");
        let other_id = seed_gate_issue(
            &executor,
            &taxonomy,
            &"not-a-reserved-repository-key".parse().unwrap(),
            "Other",
            "Other",
        );
        let gate_key: GateKey = "review".parse().unwrap();
        add_exec_gate(
            &executor,
            &issue_id,
            &gate_key,
            "Review",
            "cat \"$JIT_CONTEXT_FILE\"".to_string(),
            None,
        );
        executor.gates().add_gate(&other_id, &gate_key).unwrap();
        executor.gates().check_gate(&other_id, &gate_key).unwrap();
        executor.gates().check_gate(&issue_id, &gate_key).unwrap();
        let latest = executor.gates().check_gate(&issue_id, &gate_key).unwrap();
        executor.gates().check_gate(&other_id, &gate_key).unwrap();

        let layout = executor.require_layout().unwrap();
        let mut session = executor.storage.open_mutation_session(layout).unwrap();
        let (image, _) = executor
            .capture_gate_evaluation_image(session.as_mut(), &issue_id, &gate_key, &[])
            .unwrap()
            .unwrap();
        drop(session);
        let captured_runs = image
            .entries()
            .keys()
            .filter(|path| {
                *path != &VirtualPath::GATE_RUNS
                    && path
                        .relative()
                        .as_path()
                        .starts_with(VirtualPath::GATE_RUNS.relative().as_path())
            })
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            captured_runs,
            vec![VirtualPath::gate_run_record(&issue_id, &latest.run_id)]
        );

        let result = executor.gates().check_gate(&issue_id, &gate_key).unwrap();
        let context: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        let history = context["run_history"].as_array().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0]["run_id"], latest.run_id.as_str());
    }

    #[test]
    fn test_exec_without_context_ignores_unrelated_corrupt_gate_run() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        add_exec_gate(
            &executor,
            &issue_id,
            &"no-context".parse().unwrap(),
            "No context",
            "true".to_string(),
            None,
        );
        executor
            .gates()
            .update_gate(
                &"no-context".parse().unwrap(),
                GateUpdate {
                    pass_context: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        write_corrupt_gate_run(repo.path(), &issue_id);

        let result = executor
            .gates()
            .check_gate(&issue_id, &"no-context".parse().unwrap())
            .unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
    }

    #[test]
    fn test_exec_with_context_rejects_corrupt_named_gate_run_with_path() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        add_exec_gate(
            &executor,
            &issue_id,
            &"with-context".parse().unwrap(),
            "With context",
            "true".to_string(),
            None,
        );
        write_corrupt_gate_run(repo.path(), &issue_id);
        name_gate_run(
            &executor,
            &issue_id,
            &"with-context".parse().unwrap(),
            &crate::test_utils::gate_run_id(0xc0),
        );

        let error = executor
            .gates()
            .check_gate(&issue_id, &"with-context".parse().unwrap())
            .unwrap_err();

        assert!(
            format!("{error:#}").contains(corrupt_gate_run_record(&issue_id).relative().as_str())
        );
    }

    #[test]
    fn test_review_placeholder_ignores_unsupported_target_documents() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (_repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        executor
            .gates()
            .add_gate(&issue_id, &"editable-review".parse().unwrap())
            .unwrap();
        let mut issue = executor.storage.load_issue(&issue_id).unwrap();
        issue.documents.push(document_reference("../../outside"));
        let bytes = crate::repository_state::serialize_issue(&issue).unwrap();
        let issue_path = executor
            .require_layout()
            .unwrap()
            .resolve(&VirtualPath::issue_record(&issue_id))
            .unwrap();
        std::fs::write(issue_path, bytes).unwrap();

        let result = executor
            .gates()
            .check_gate(&issue_id, &"editable-review".parse().unwrap())
            .unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
    }

    #[test]
    fn test_builtin_gate_evidence_changes_with_broad_validation_input() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        let before = capture_gate_evidence(
            &executor,
            &issue_id,
            &"not-a-reserved-repository-key".parse().unwrap(),
        );
        std::fs::write(
            repo.path().join(".jit/rules.toml"),
            "# changed broad validation input\n",
        )
        .unwrap();

        let after = capture_gate_evidence(
            &executor,
            &issue_id,
            &"not-a-reserved-repository-key".parse().unwrap(),
        );

        assert_ne!(before, after);
    }

    type RepositoryChange = fn(&std::path::Path, &IssueId);

    fn leave_unchanged(_: &std::path::Path, _: &IssueId) {}

    fn respell_created_at(root: &std::path::Path, issue_id: &IssueId) {
        let path = root.join(VirtualPath::issue_record(issue_id).repository_relative());
        let mut raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let created = raw["created_at"].as_str().unwrap().replace('Z', "+00:00");
        raw["created_at"] = created.into();
        std::fs::write(path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();
    }

    fn repeat_last_event(root: &std::path::Path, _: &IssueId) {
        let path = root.join(".jit/events.jsonl");
        let log = std::fs::read_to_string(&path).unwrap();
        let last = log.lines().last().unwrap().to_string();
        std::fs::write(path, format!("{log}{last}\n")).unwrap();
    }

    fn append_undecodable_event(root: &std::path::Path, _: &IssueId) {
        let path = root.join(".jit/events.jsonl");
        let log = std::fs::read_to_string(&path).unwrap();
        let line = r#"{"type":"issue_claimed","id":"not-an-event-id"}"#;
        std::fs::write(path, format!("{log}{line}\n")).unwrap();
    }

    fn add_rule_finding(root: &std::path::Path, _: &IssueId) {
        let rule = late_repository_rule(&crate::test_taxonomy::test_taxonomy());
        std::fs::write(root.join(".jit/rules.toml"), rule).unwrap();
    }

    fn break_rules_file(root: &std::path::Path, _: &IssueId) {
        std::fs::write(root.join(".jit/rules.toml"), "[[rules\nnot toml").unwrap();
    }

    fn remove_index(root: &std::path::Path, _: &IssueId) {
        std::fs::remove_file(root.join(".jit/index.json")).unwrap();
    }

    fn add_dangling_dependency(root: &std::path::Path, issue_id: &IssueId) {
        let path = root.join(VirtualPath::issue_record(issue_id).repository_relative());
        let mut raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let absent = crate::test_utils::issue_id(0x96bf59db932e7f49ee5cc6bb5a1f6645);
        raw["dependencies"] = serde_json::json!([absent]);
        std::fs::write(path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();
    }

    /// Whether `jit validate` passes after `change`, and the built-in checker's run.
    fn judge_repository_after(change: RepositoryChange) -> (bool, GateRunResult) {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        let image = validation_image(&executor);
        change(repo.path(), &issue_id);
        let validate = executor
            .validation()
            .validate_repository_report()
            .map(|report| report.structural_failure.is_none() && !report.rule_report().has_errors())
            .unwrap();
        let run = executor
            .execute_builtin_checker_with_repository_view(
                GateRunSubject {
                    run_id: &crate::test_utils::gate_run_id(1),
                    gate_key: &"not-a-reserved-repository-key".parse().unwrap(),
                    issue_id: &issue_id,
                    stage: GateStage::Postcheck,
                },
                &GateChecker::RepositoryValidation,
                &image,
            )
            .unwrap();
        (validate, run)
    }

    #[test]
    fn test_repository_validation_checker_passes_exactly_where_validate_passes() {
        let changes: [(RepositoryChange, bool); 8] = [
            (leave_unchanged, true),
            (respell_created_at, false),
            (repeat_last_event, false),
            (append_undecodable_event, false),
            (add_rule_finding, false),
            (break_rules_file, false),
            (remove_index, false),
            (add_dangling_dependency, false),
        ];
        for (index, (change, passes)) in changes.into_iter().enumerate() {
            let (validate, run) = judge_repository_after(change);
            assert_eq!(validate, passes, "change {index}: jit validate");
            assert_eq!(
                run.status == GateRunStatus::Passed,
                passes,
                "change {index}: {} {}",
                run.stdout,
                run.stderr
            );
        }
    }

    #[test]
    fn test_repository_validation_records_a_repository_judged_wrong_as_a_failed_run() {
        let changes: [RepositoryChange; 4] = [
            respell_created_at,
            repeat_last_event,
            append_undecodable_event,
            add_dangling_dependency,
        ];
        for (index, change) in changes.into_iter().enumerate() {
            let (_, run) = judge_repository_after(change);
            assert_eq!(run.status, GateRunStatus::Failed, "change {index}");
            assert_eq!(run.exit_code, Some(4), "change {index}");
            let findings = run.findings.unwrap().findings;
            assert!(
                findings.iter().any(|finding| finding.is_blocking()),
                "change {index}"
            );
        }
    }

    #[test]
    fn test_repository_validation_never_reuses_a_verdict_when_the_gate_declares_inputs() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        let gate_key: GateKey = "not-a-reserved-repository-key".parse().unwrap();
        std::fs::create_dir_all(repo.path().join("workspace")).unwrap();
        std::fs::write(repo.path().join("workspace/lib.rs"), "fn main() {}\n").unwrap();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.get_mut(&gate_key).unwrap().inputs = Some(
            crate::domain::repository_inputs::RepositoryInputs::parse(&["workspace"], &[]).unwrap(),
        );
        std::fs::write(
            repo.path().join(".jit/gates.toml"),
            crate::declarations::serialize_gate_registry(&registry).unwrap(),
        )
        .unwrap();

        let passed = executor.gates().check_gate(&issue_id, &gate_key).unwrap();
        assert_eq!(passed.status, GateRunStatus::Passed, "{}", passed.stdout);

        repeat_last_event(repo.path(), &issue_id);
        let after_break = executor.gates().check_gate(&issue_id, &gate_key).unwrap();
        assert_eq!(after_break.status, GateRunStatus::Failed);
        assert!(after_break.origin.is_executed());

        let log = repo.path().join(".jit/events.jsonl");
        let mut seen = std::collections::HashSet::new();
        let repaired = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .filter(|line| seen.insert(line.to_string()))
            .map(|line| format!("{line}\n"))
            .collect::<String>();
        std::fs::write(&log, repaired).unwrap();
        let after_repair = executor.gates().check_gate(&issue_id, &gate_key).unwrap();
        assert_eq!(
            after_repair.status,
            GateRunStatus::Passed,
            "{}",
            after_repair.stdout
        );
        assert!(after_repair.origin.is_executed());
    }

    #[test]
    fn test_repository_validation_records_a_missing_index_as_an_error_run() {
        let (_, run) = judge_repository_after(remove_index);
        assert_eq!(run.status, GateRunStatus::Error);
        assert!(run.stderr.contains("index.json"), "{}", run.stderr);
    }

    #[test]
    fn test_label_target_validation_records_each_target_label_fault_as_error_run() {
        let executor = setup();
        for id in [
            crate::test_utils::issue_id(0xdead_beef),
            crate::test_utils::issue_id(0x1_deadbeef),
        ] {
            let mut target =
                crate::test_utils::fixture_issue(format!("Ambiguous target {id}"), String::new());
            target.id = id;
            seed_issue(executor.storage(), target);
        }
        let cases: [(&str, &[&str], &[&str]); 5] = [
            (
                "coverage-target",
                &[],
                &["coverage-target:<target-id>", "found 0"],
            ),
            (
                "coverage-target",
                &["coverage-target:aaaa", "coverage-target:bbbb"],
                &["coverage-target:<target-id>", "found 2"],
            ),
            ("", &[], &["label_namespace is empty"]),
            (
                "coverage-target",
                &["coverage-target:no-such-issue"],
                &["coverage-target:no-such-issue"],
            ),
            (
                "coverage-target",
                &["coverage-target:deadbeef"],
                &["coverage-target:deadbeef", "ambiguous"],
            ),
        ];

        for (index, (label_namespace, targets, named)) in cases.into_iter().enumerate() {
            let gate_key: GateKey = format!("coverage-{index}").parse().unwrap();
            let labels = std::iter::once("type:task")
                .chain(targets.iter().copied())
                .map(str::to_string)
                .collect();
            let issue_id = add_builtin_gate(
                &executor,
                &gate_key,
                GateChecker::LabelTargetValidation {
                    label_namespace: label_namespace.to_string(),
                },
                labels,
            );

            let result = executor.gates().check_gate(&issue_id, &gate_key).unwrap();

            assert_eq!(result.status, GateRunStatus::Error, "case {index}");
            assert_eq!(
                result.exit_code,
                Some(crate::gate_execution::CHECKER_TEMPFAIL_EXIT),
                "case {index}"
            );
            let stderr = result.stderr.to_lowercase();
            for fragment in named {
                assert!(stderr.contains(fragment), "case {index}: {}", result.stderr);
            }
        }
    }

    /// REQ-02: over one image, each built-in validation checker reports the
    /// findings `jit validate` reports for its selection.
    #[test]
    fn test_execute_builtin_checker_reports_the_findings_of_its_validate_selection() {
        use crate::validation::report::{Selection, StructuralFailure};

        let executor = setup();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_utils::shipped_item_kind_tables(&["requirement"])
        );
        let config_path = executor
            .repository_layout()
            .unwrap()
            .resolve(&crate::repository_state::VirtualPath::CONFIG)
            .unwrap();
        std::fs::write(config_path, &config).unwrap();
        seed_repo_file(&executor.storage, ".jit/config.toml", &config);
        seed_rule_validation_rules(&executor);

        let mut container = crate::test_utils::fixture_issue(
            "Container".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: covered\n- [hard] REQ-02: uncovered\n"
                .to_string(),
        );
        container.labels = vec!["type:epic".to_string()];
        let container_id = container.id.clone();
        let mut child =
            crate::test_utils::fixture_issue("Implementation".to_string(), String::new());
        child.labels = vec![
            "type:task".to_string(),
            format!("satisfies:{}/REQ-01", container.short_id()),
            "satisfies:@/requirement/missing".to_string(),
        ];
        child.state = State::Ready;
        let child_id = child.id.clone();
        container.dependencies = vec![child.id.clone()];
        seed_issue(&executor.storage, child);
        seed_issue(&executor.storage, container);

        let breakdown_labels = vec![
            "type:breakdown".to_string(),
            format!("brackets:{}", container_id.short_id()),
        ];
        let gated = |key: &str, checker: GateChecker, labels: &[String]| {
            add_builtin_gate(&executor, &key.parse().unwrap(), checker, labels.to_vec())
        };
        let rule = GateChecker::RuleValidation {
            rule: "coverage-preview".to_string(),
        };
        let label_target = GateChecker::LabelTargetValidation {
            label_namespace: "brackets".to_string(),
        };
        let rule_gated = gated("rule-gate", rule.clone(), &breakdown_labels);
        let scope_gated = gated("scope-gate", label_target.clone(), &breakdown_labels);
        let repository_gated = gated(
            "repository-gate",
            GateChecker::RepositoryValidation,
            &["type:task".to_string()],
        );
        let mut unrelated = crate::test_utils::fixture_issue(
            "Unrelated".to_string(),
            "## Success Criteria\n\n- [hard] REQ-99: uncovered\n".to_string(),
        );
        unrelated.labels = vec!["type:epic".to_string()];
        unrelated.dependencies = vec![rule_gated.clone(), scope_gated.clone(), repository_gated];
        seed_issue(&executor.storage, unrelated);
        let image = validation_image(&executor);
        let validate = crate::validation::report(
            &crate::validation::CapturedValidation::from_image(&image),
            &crate::validation::ReportInputs {
                now: chrono::Utc::now(),
                history: None,
            },
        );
        // The repository checker judges its own history-reading capture, not
        // `image`; its expected side is that capture of the same unchanged
        // repository, whose findings match the report over `image`.
        let whole = executor.validation().validate_repository_report().unwrap();
        assert_eq!(whole.rule_report(), validate.rule_report());
        let cases = [
            (
                GateChecker::IssueValidation,
                child_id.clone(),
                validate.select(&Selection::Issue((&child_id).into())),
            ),
            (
                rule,
                rule_gated.clone(),
                validate.select(&Selection::Rule {
                    issue: (&rule_gated).into(),
                    rule: "coverage-preview".to_string(),
                }),
            ),
            (
                label_target,
                scope_gated,
                validate.select(&Selection::Scope((&container_id).into())),
            ),
            (
                GateChecker::RepositoryValidation,
                child_id,
                whole.select(&Selection::Whole),
            ),
        ];

        for (checker, gated, selected) in cases {
            let (selected, failure) = match selected {
                Ok(selected) => (selected, None),
                Err(error) => {
                    let StructuralFailure { failure, findings } = error.downcast().unwrap();
                    (findings, Some(format!("{failure:#}")))
                }
            };
            let expected = selected
                .findings
                .iter()
                .map(|finding| format!("[{}] {}", finding.rule, finding.message))
                .chain(failure)
                .collect::<Vec<_>>();
            let run = executor
                .execute_builtin_checker_with_repository_view(
                    GateRunSubject {
                        run_id: &crate::test_utils::gate_run_id(1),
                        gate_key: &"parity".parse().unwrap(),
                        issue_id: &gated,
                        stage: GateStage::Postcheck,
                    },
                    &checker,
                    &image,
                )
                .unwrap();
            let reported = run
                .findings
                .unwrap()
                .findings
                .into_iter()
                .map(|finding| finding.summary)
                .collect::<Vec<_>>();
            assert!(
                !expected.is_empty(),
                "{checker:?}: the image holds findings"
            );
            assert_eq!(reported, expected, "{checker:?}");
        }
    }

    #[test]
    fn test_rule_validation_checker_records_missing_rule_as_error_run() {
        let executor = setup();
        seed_rule_validation_rules(&executor);
        let issue_id = add_builtin_gate(
            &executor,
            &"coverage-preview".parse().unwrap(),
            GateChecker::RuleValidation {
                rule: "no-such-rule".to_string(),
            },
            vec!["type:task".to_string()],
        );

        let result = executor
            .gates()
            .check_gate(&issue_id, &"coverage-preview".parse().unwrap())
            .unwrap();

        assert_eq!(result.command, "builtin:rule_validation");
        assert_eq!(result.status, GateRunStatus::Error);
        assert!(result.stderr.contains("no-such-rule"), "{}", result.stderr);
    }

    #[test]
    fn test_builtin_checker_records_unloadable_declarations_as_error_run() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        let record = repo
            .path()
            .join(VirtualPath::issue_record(&issue_id).repository_relative());
        let mut raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
        raw["labels"]
            .as_array_mut()
            .unwrap()
            .push(format!("target:{issue_id}").into());
        std::fs::write(&record, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();
        std::fs::write(repo.path().join(".jit/rules.toml"), "[[rules\nnot toml").unwrap();
        for checker in [
            GateChecker::IssueValidation,
            GateChecker::RepositoryValidation,
            GateChecker::RuleValidation {
                rule: "any".to_string(),
            },
            GateChecker::LabelTargetValidation {
                label_namespace: "target".to_string(),
            },
        ] {
            let image = validation_image(&executor);
            let result = executor
                .execute_builtin_checker_with_repository_view(
                    GateRunSubject {
                        run_id: &crate::test_utils::gate_run_id(1),
                        gate_key: &"not-a-reserved-repository-key".parse().unwrap(),
                        issue_id: &issue_id,
                        stage: GateStage::Postcheck,
                    },
                    &checker,
                    &image,
                )
                .unwrap();

            assert_eq!(result.status, GateRunStatus::Error, "{checker:?}");
            assert!(
                result.stderr.contains("rules declaration did not load"),
                "{checker:?}: {}",
                result.stderr
            );
        }
    }

    #[test]
    fn test_builtin_checker_records_content_finding_as_failed_run() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        let late_rule = late_repository_rule(&crate::test_taxonomy::test_taxonomy());
        std::fs::write(repo.path().join(".jit/rules.toml"), late_rule).unwrap();

        let result = executor
            .gates()
            .check_gate(&issue_id, &"not-a-reserved-repository-key".parse().unwrap())
            .unwrap();

        assert_eq!(result.status, GateRunStatus::Failed);
    }

    #[test]
    fn test_check_gate_automated_failure() {
        let executor = setup();

        // Define an automated gate that fails
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "failing-gate".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "failing-gate".parse().unwrap(),
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
        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"failing-gate".parse().unwrap())
            .unwrap();

        // Check the gate
        let result = executor
            .gates()
            .check_gate(&issue_id, &"failing-gate".parse().unwrap())
            .unwrap();

        assert_eq!(result.status, GateRunStatus::Failed);
        assert_eq!(result.exit_code, Some(1));

        // Verify gate status was updated on issue
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        let gate_state = issue
            .gates_status
            .get(&"failing-gate".parse().unwrap())
            .unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Failed);
    }

    #[test]
    fn test_check_gate_manual_not_checkable() {
        let executor = setup();

        // Define a manual gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "manual-gate".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "manual-gate".parse().unwrap(),
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
        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"manual-gate".parse().unwrap())
            .unwrap();

        // Try to check the gate - should fail
        let result = executor
            .gates()
            .check_gate(&issue_id, &"manual-gate".parse().unwrap());
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
            "auto-gate".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "auto-gate".parse().unwrap(),
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
            "manual-gate".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "manual-gate".parse().unwrap(),
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

        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"auto-gate".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .add_gate(&issue_id, &"manual-gate".parse().unwrap())
            .unwrap();

        // Before any evaluation: both required gates are pending, ordered by priority.
        let statuses = executor
            .gates()
            .get_required_gate_statuses_for_issue(&issue_id)
            .unwrap();
        assert_eq!(
            statuses,
            vec![
                ("auto-gate".parse::<GateKey>().unwrap(), GateStatus::Pending),
                (
                    "manual-gate".parse::<GateKey>().unwrap(),
                    GateStatus::Pending
                ),
            ]
        );

        // Running the auto gate flips only its status; the manual gate stays pending.
        executor
            .gates()
            .check_gate(&issue_id, &"auto-gate".parse().unwrap())
            .unwrap();
        let statuses = executor
            .gates()
            .get_required_gate_statuses_for_issue(&issue_id)
            .unwrap();
        assert_eq!(
            statuses,
            vec![
                ("auto-gate".parse::<GateKey>().unwrap(), GateStatus::Passed),
                (
                    "manual-gate".parse::<GateKey>().unwrap(),
                    GateStatus::Pending
                ),
            ]
        );

        // Attesting the manual gate makes every required gate green.
        executor
            .gates()
            .pass_gate(
                &issue_id,
                &"manual-gate".parse().unwrap(),
                Some("human:tester".to_string()),
                false,
            )
            .unwrap();
        let statuses = executor
            .gates()
            .get_required_gate_statuses_for_issue(&issue_id)
            .unwrap();
        assert!(statuses.iter().all(|(_, s)| *s == GateStatus::Passed));
    }

    fn make_manual_gate(key: &GateKey) -> crate::declarations::GateDefinition {
        crate::declarations::GateDefinition {
            mode: GateMode::Manual,
            checker: None,
            ..make_auto_gate(key, "")
        }
    }

    /// An issue requiring the manual gate `review`, whose registry also declares
    /// the automated gate `tests`.
    fn seed_review_issue(executor: &CommandExecutor<InMemoryStorage>) -> IssueId {
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".parse().unwrap(),
            make_manual_gate(&"review".parse().unwrap()),
        );
        registry.gates.insert(
            "tests".parse().unwrap(),
            make_auto_gate(&"tests".parse().unwrap(), "exit 0"),
        );
        seed_gate_registry(executor.storage(), &registry);
        let mut issue = crate::test_utils::fixture_issue("T".to_string(), String::new());
        issue.gates_required = vec!["review".parse().unwrap(), "tests".parse().unwrap()];
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
            .gates()
            .pass_gate(
                &issue_id,
                &"review".parse().unwrap(),
                Some("human:me".to_string()),
                false,
            )
            .unwrap();

        let run = executor
            .gates()
            .get_last_gate_run(&issue_id, &"review".parse().unwrap())
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
            executor
                .gates()
                .get_gate_run_result(&issue_id, &run.run_id)
                .unwrap(),
            run,
            "the attestation is addressable by its issue and run id"
        );
    }

    #[test]
    fn test_pass_gate_manual_attestation_names_the_attestation_run_in_gate_state() {
        let executor = setup();
        let issue_id = seed_review_issue(&executor);
        let gate_key: GateKey = "review".parse().unwrap();

        executor
            .gates()
            .pass_gate(&issue_id, &gate_key, Some("human:me".to_string()), false)
            .unwrap();

        let issue = executor.storage.load_issue(&issue_id).unwrap();
        let named = issue.gates_status[&gate_key]
            .run_id
            .clone()
            .expect("a manual status write names its run");
        let run = executor
            .gates()
            .get_gate_run_result(&issue_id, &named)
            .unwrap();
        assert_eq!(run.origin, crate::domain::GateVerdictOrigin::Attested);
    }

    #[test]
    fn test_list_gate_runs_preserves_history_after_gate_and_binding_are_removed() {
        let executor = setup();
        let issue_id = seed_review_issue(&executor);
        let key = "review".parse().unwrap();
        executor
            .gates()
            .pass_gate(&issue_id, &key, Some("human:me".into()), false)
            .unwrap();
        let mut issue = executor.storage.load_issue(&issue_id).unwrap();
        issue.gates_required.retain(|gate| gate != &key);
        seed_issue(executor.storage(), issue);
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.remove(&key);
        seed_gate_registry(executor.storage(), &registry);
        let runs = executor
            .gates()
            .list_gate_runs(&issue_id, Some(&key))
            .unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].gate_key, key);
        assert_eq!(
            executor
                .gates()
                .get_gate_run_result(&issue_id, &runs[0].run_id)
                .unwrap()
                .gate_key,
            key
        );
    }

    #[test]
    fn test_list_gate_runs_lists_manual_attestations_alongside_automated_runs() {
        let executor = setup();
        let issue_id = seed_review_issue(&executor);

        executor
            .gates()
            .check_gate(&issue_id, &"tests".parse().unwrap())
            .unwrap();
        for _ in 0..2 {
            executor
                .gates()
                .pass_gate(
                    &issue_id,
                    &"review".parse().unwrap(),
                    Some("human:me".to_string()),
                    false,
                )
                .unwrap();
        }

        let runs = executor.gates().list_gate_runs(&issue_id, None).unwrap();
        let attested = runs
            .iter()
            .filter(|run| run.origin == crate::domain::GateVerdictOrigin::Attested)
            .count();
        assert_eq!(attested, 2, "every attestation is its own record");
        assert!(runs
            .iter()
            .any(|run| run.gate_key == "tests".parse().unwrap() && run.origin.is_executed()));
    }

    #[test]
    fn test_get_last_gate_runs_for_issue_agrees_with_required_gate_statuses() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        let gates = [
            make_auto_gate(&"auto-pass".parse().unwrap(), "exit 0"),
            make_auto_gate(&"auto-fail".parse().unwrap(), "exit 1"),
            make_auto_gate(&"auto-pending".parse().unwrap(), "exit 0"),
            make_manual_gate(&"manual-pass".parse().unwrap()),
            make_manual_gate(&"manual-fail".parse().unwrap()),
            make_manual_gate(&"manual-pending".parse().unwrap()),
        ];
        for gate in &gates {
            registry.gates.insert(gate.key.clone(), gate.clone());
        }
        seed_gate_registry(executor.storage(), &registry);
        let mut issue = crate::test_utils::fixture_issue("T".to_string(), String::new());
        issue.gates_required = gates.iter().map(|gate| gate.key.clone()).collect();
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);

        executor
            .gates()
            .check_gate(&issue_id, &"auto-pass".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .check_gate(&issue_id, &"auto-fail".parse().unwrap())
            .unwrap();
        let attestor = Some("human:me".to_string());
        executor
            .gates()
            .pass_gate(
                &issue_id,
                &"manual-pass".parse().unwrap(),
                attestor.clone(),
                false,
            )
            .unwrap();
        executor
            .gates()
            .fail_gate(&issue_id, &"manual-fail".parse().unwrap(), attestor)
            .unwrap();

        let (runs, not_run) = executor
            .gates()
            .get_last_gate_runs_for_issue(&issue_id)
            .unwrap();
        let latest: HashMap<_, _> = runs
            .iter()
            .map(|run| (run.gate_key.clone(), run.clone()))
            .collect();
        let readiness = executor
            .gates()
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
            "auto".parse().unwrap(),
            make_auto_gate(
                &"auto".parse().unwrap(),
                &format!("test -e {}", marker.display()),
            ),
        );
        registry.gates.insert(
            "review".parse().unwrap(),
            make_manual_gate(&"review".parse().unwrap()),
        );
        seed_gate_registry(base.storage(), &registry);
        let mut issue = crate::test_utils::fixture_issue("T".to_string(), String::new());
        issue.gates_required = vec!["auto".parse().unwrap(), "review".parse().unwrap()];
        let issue_id = issue.id.clone();
        seed_issue(base.storage(), issue);

        // Automated B starts after A's checker ran and publishes before A's
        // record session captures.
        let concurrent = issue_id.clone();
        let evaluating = with_open_race(
            base.storage.clone(),
            crate::storage::MutationPoint::SessionOpen,
            2,
            SessionRaceAction::Run(Box::new(move |storage| {
                std::fs::write(&marker, "").unwrap();
                let run = memory_executor(storage.clone())
                    .gates()
                    .check_gate(&concurrent, &"auto".parse().unwrap())
                    .unwrap();
                assert_eq!(run.status, GateRunStatus::Passed);
            })),
        );
        let run = memory_executor(evaluating)
            .gates()
            .check_gate(&issue_id, &"auto".parse().unwrap())
            .unwrap();
        assert_eq!(run.status, GateRunStatus::Failed);

        // Attestation A conflicts on its first apply; B publishes before A's
        // retry captures.
        let concurrent = issue_id.clone();
        let attesting = with_open_race(
            base.storage.clone(),
            crate::storage::MutationPoint::SessionOpen,
            2,
            SessionRaceAction::Run(Box::new(move |storage| {
                memory_executor(storage.clone())
                    .gates()
                    .pass_gate(
                        &concurrent,
                        &"review".parse().unwrap(),
                        Some("human:b".to_string()),
                        false,
                    )
                    .unwrap();
            })),
        );
        attesting.inject_repository_state_apply_conflicts(1);
        memory_executor(attesting)
            .gates()
            .fail_gate(
                &issue_id,
                &"review".parse().unwrap(),
                Some("human:a".to_string()),
            )
            .unwrap();

        let readiness = base
            .gates()
            .get_required_gate_statuses_for_issue(&issue_id)
            .unwrap();
        assert_eq!(
            readiness,
            vec![
                (
                    "auto".parse::<GateKey>().unwrap(),
                    crate::domain::GateStatus::Failed
                ),
                (
                    "review".parse::<GateKey>().unwrap(),
                    crate::domain::GateStatus::Failed
                ),
            ],
            "each A published last"
        );
        let (latest, not_run) = base
            .gates()
            .get_last_gate_runs_for_issue(&issue_id)
            .unwrap();
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
    fn test_get_last_gate_runs_for_issue_loads_named_runs_for_every_gate_state() {
        let executor = setup();
        let mut issue = crate::test_utils::fixture_issue("T".into(), String::new());
        let mut expected = Vec::new();
        for (index, key) in ["required", "optional"].into_iter().enumerate() {
            let key: GateKey = key.parse().unwrap();
            let older = GateRunResult::attestation(
                &crate::test_utils::gate_run_id(index as u128 * 2 + 1),
                &issue.id,
                &key,
                GateStage::Postcheck,
                GateStatus::Failed,
                None,
            );
            let mut named = older.clone();
            named.run_id = crate::test_utils::gate_run_id(index as u128 * 2 + 2);
            named.status = GateRunStatus::Passed;
            for run in [&older, &named] {
                executor.storage.add_data_file(
                    crate::repository_state::VirtualPath::gate_run_record(
                        &run.issue_id,
                        &run.run_id,
                    )
                    .relative()
                    .as_path(),
                    &serde_json::to_string(run).unwrap(),
                );
            }
            issue
                .gates_status
                .insert(key, super::gate_state_from_run(&named));
            expected.push(named.run_id);
        }
        issue.gates_required.push("required".parse().unwrap());
        seed_issue(executor.storage(), issue.clone());

        let (runs, not_run) = executor
            .gates()
            .get_last_gate_runs_for_issue(&issue.id)
            .unwrap();
        let actual: std::collections::BTreeSet<_> =
            runs.iter().map(|run| run.run_id.clone()).collect();
        assert_eq!(actual, expected.into_iter().collect());
        assert!(not_run.is_empty());
    }

    #[test]
    fn test_get_last_gate_runs_for_issue_omits_a_run_named_by_another_gates_state() {
        let executor = setup();
        let mut issue = crate::test_utils::fixture_issue("T".into(), String::new());
        let own: GateKey = "review".parse().unwrap();
        let other: GateKey = "lint".parse().unwrap();
        let run = GateRunResult::attestation(
            &crate::test_utils::gate_run_id(1),
            &issue.id,
            &other,
            GateStage::Postcheck,
            GateStatus::Passed,
            None,
        );
        executor.storage.add_data_file(
            crate::repository_state::VirtualPath::gate_run_record(&run.issue_id, &run.run_id)
                .relative()
                .as_path(),
            &serde_json::to_string(&run).unwrap(),
        );
        issue
            .gates_status
            .insert(own.clone(), super::gate_state_from_run(&run));
        seed_issue(executor.storage(), issue.clone());

        let (runs, not_run) = executor
            .gates()
            .get_last_gate_runs_for_issue(&issue.id)
            .unwrap();
        assert!(runs.is_empty());
        assert_eq!(not_run, vec![own]);
    }

    #[test]
    fn test_get_last_gate_runs_for_issue_ignores_history_without_state_run_id() {
        let executor = setup();
        let mut issue = crate::test_utils::fixture_issue("T".into(), String::new());
        let key: GateKey = "review".parse().unwrap();
        let run = GateRunResult::attestation(
            &crate::test_utils::gate_run_id(1),
            &issue.id,
            &key,
            GateStage::Postcheck,
            GateStatus::Passed,
            None,
        );
        executor.storage.add_data_file(
            crate::repository_state::VirtualPath::gate_run_record(&run.issue_id, &run.run_id)
                .relative()
                .as_path(),
            &serde_json::to_string(&run).unwrap(),
        );
        let mut state = super::gate_state_from_run(&run);
        state.run_id = None;
        issue.gates_status.insert(key.clone(), state);
        seed_issue(executor.storage(), issue.clone());

        let (runs, not_run) = executor
            .gates()
            .get_last_gate_runs_for_issue(&issue.id)
            .unwrap();
        assert!(runs.is_empty());
        assert_eq!(not_run, vec![key]);
    }

    #[test]
    fn test_get_last_gate_runs_for_issue() {
        let executor = setup();

        // Define two automated gates
        let mut registry = executor.storage.load_gate_registry().unwrap();
        for (key, exit_code) in [("gate-1", 0), ("gate-2", 0)]
            .map(|(key, code)| (key.parse::<GateKey>().unwrap(), code))
        {
            registry.gates.insert(
                key.clone(),
                crate::declarations::GateDefinition {
                    version: 1,
                    key: key.clone(),
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
        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"gate-1".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .add_gate(&issue_id, &"gate-2".parse().unwrap())
            .unwrap();

        executor
            .gates()
            .check_gate(&issue_id, &"gate-1".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .check_gate(&issue_id, &"gate-2".parse().unwrap())
            .unwrap();

        let (results, warnings) = executor
            .gates()
            .get_last_gate_runs_for_issue(&issue_id)
            .unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.status == GateRunStatus::Passed));
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_get_last_gate_runs_for_issue_reports_not_run_gates() {
        let executor = setup();

        let mut registry = executor.storage.load_gate_registry().unwrap();
        for key in ["gate-1", "gate-2"].map(|key| key.parse::<GateKey>().unwrap()) {
            registry
                .gates
                .insert(key.clone(), make_auto_gate(&key, "exit 0"));
        }
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"gate-1".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .add_gate(&issue_id, &"gate-2".parse().unwrap())
            .unwrap();

        executor
            .gates()
            .check_gate(&issue_id, &"gate-1".parse().unwrap())
            .unwrap();

        let (results, not_run) = executor
            .gates()
            .get_last_gate_runs_for_issue(&issue_id)
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].gate_key, "gate-1".parse().unwrap());
        assert_eq!(not_run, vec!["gate-2".parse::<GateKey>().unwrap()]);
    }

    #[test]
    fn test_run_prechecks_before_starting_work() {
        let executor = setup();

        // Define a precheck gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "precheck".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "precheck".parse().unwrap(),
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
            "postcheck-with-unsafe-prompt".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "postcheck-with-unsafe-prompt".parse().unwrap(),
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
        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::Ready;
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"precheck".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .add_gate(&issue_id, &"postcheck-with-unsafe-prompt".parse().unwrap())
            .unwrap();

        // Try to start work - should run prechecks
        executor
            .issues()
            .claim_issue(
                &IssueIdPrefix::from(&issue_id),
                "agent:worker".parse().unwrap(),
            )
            .unwrap();

        // Verify prechecks ran and issue transitioned
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::InProgress);

        let gate_state = issue
            .gates_status
            .get(&"precheck".parse().unwrap())
            .unwrap();
        assert_eq!(gate_state.status, crate::domain::GateStatus::Passed);
    }

    #[test]
    fn test_lifecycle_precheck_without_context_ignores_corrupt_gate_run() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        add_exec_gate(
            &executor,
            &issue_id,
            &"precheck-no-context".parse().unwrap(),
            "Precheck no context",
            "true".to_string(),
            None,
        );
        executor
            .gates()
            .update_gate(
                &"precheck-no-context".parse().unwrap(),
                GateUpdate {
                    stage: Some(GateStage::Precheck),
                    pass_context: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        crate::test_utils::transition_issue(&executor, &issue_id, State::Ready).unwrap();
        write_corrupt_gate_run(repo.path(), &issue_id);

        executor
            .issues()
            .claim_issue(
                &IssueIdPrefix::from(&issue_id),
                "agent:worker".parse().unwrap(),
            )
            .unwrap();

        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::InProgress);
        assert_eq!(
            issue.gates_status[&"precheck-no-context".parse().unwrap()].status,
            crate::domain::GateStatus::Passed
        );
    }

    #[test]
    fn test_precheck_failure_blocks_transition() {
        let executor = setup();

        // Define a failing precheck gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "precheck-fail".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "precheck-fail".parse().unwrap(),
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
        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::Ready;
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"precheck-fail".parse().unwrap())
            .unwrap();

        // Try to start work - should fail
        let result = executor.issues().claim_issue(
            &IssueIdPrefix::from(&issue_id),
            "agent:worker".parse().unwrap(),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("precheck"));

        // Verify issue didn't transition
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Ready);
    }

    #[cfg(unix)]
    #[test]
    fn test_lifecycle_precheck_records_one_moved_run_when_checker_mutates_repository() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        let issue_path = VirtualPath::issue_record(&issue_id).repository_relative();
        add_exec_gate(
            &executor,
            &issue_id,
            &"precheck-race".parse().unwrap(),
            "Precheck race",
            format!(
                "if [ ! -f .precheck-race-seen ]; then : > .precheck-race-seen; sed -i 's/\\\"title\\\": \\\"Test\\\"/\\\"title\\\": \\\"Moved\\\"/' {issue_path}; fi; printf x >> checker-count; true"
            ),
            None,
        );
        executor
            .gates()
            .update_gate(
                &"precheck-race".parse().unwrap(),
                GateUpdate {
                    stage: Some(GateStage::Precheck),
                    ..Default::default()
                },
            )
            .unwrap();
        crate::test_utils::transition_issue(&executor, &issue_id, State::Ready).unwrap();

        executor
            .issues()
            .claim_issue(
                &IssueIdPrefix::from(&issue_id),
                "agent:worker".parse().unwrap(),
            )
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
            .filter(|run| run.gate_key == "precheck-race".parse().unwrap())
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

    /// A broad precheck remains stable when the issue has document references.
    #[test]
    fn test_claim_broad_precheck_over_issue_documents_records_a_stable_run() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        let gate_key: GateKey = "not-a-reserved-repository-key".parse().unwrap();
        executor
            .gates()
            .update_gate(
                &gate_key,
                GateUpdate {
                    stage: Some(GateStage::Precheck),
                    ..Default::default()
                },
            )
            .unwrap();
        std::fs::create_dir_all(repo.path().join("dev")).unwrap();
        std::fs::write(repo.path().join("dev/notes.md"), "# Notes\n").unwrap();
        executor
            .documents()
            .add_document_reference(&issue_id, "dev/notes.md", None, None, None, true)
            .unwrap();
        crate::test_utils::transition_issue(&executor, &issue_id, State::Ready).unwrap();

        executor
            .issues()
            .claim_issue(
                &IssueIdPrefix::from(&issue_id),
                "agent:worker".parse().unwrap(),
            )
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

    fn seed_issue_requiring_exec_gates(
        executor: &CommandExecutor<InMemoryStorage>,
        state: State,
        gates: &[(&str, &str)],
    ) -> IssueId {
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry
            .gates
            .extend(gates.iter().map(|(key, command)| -> (GateKey, _) {
                let key: GateKey = key.parse().unwrap();
                let definition = crate::declarations::GateDefinition {
                    version: 1,
                    key: key.clone(),
                    title: "Postcheck".to_string(),
                    description: "Postcheck".to_string(),
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
                };
                (key, definition)
            }));
        seed_gate_registry(executor.storage(), &registry);

        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = state;
        issue.gates_required = gates.iter().map(|(key, _)| key.parse().unwrap()).collect();
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        issue_id
    }

    #[test]
    fn test_pass_gate_last_unpassed_auto_gate_completes_gated_issue_and_promotes_dependent() {
        let executor = setup();
        let issue_id = seed_issue_requiring_exec_gates(
            &executor,
            State::Gated,
            &[("first", "exit 0"), ("last", "exit 0")],
        );
        let mut dependent =
            crate::test_utils::fixture_issue("Dependent".to_string(), "Test".to_string());
        dependent.state = State::Backlog;
        dependent.dependencies.push(issue_id.clone());
        let dependent_id = dependent.id.clone();
        seed_issue(executor.storage(), dependent);
        let states = || {
            (
                executor.storage.load_issue(&issue_id).unwrap().state,
                executor.storage.load_issue(&dependent_id).unwrap().state,
            )
        };

        executor
            .gates()
            .pass_gate(&issue_id, &"first".parse().unwrap(), None, false)
            .unwrap();
        assert_eq!(states(), (State::Gated, State::Backlog));

        executor
            .gates()
            .pass_gate(&issue_id, &"last".parse().unwrap(), None, false)
            .unwrap();
        assert_eq!(states(), (State::Done, State::Ready));
    }

    #[test]
    fn test_pass_gate_failed_auto_gate_leaves_gated_issue_gated() {
        let executor = setup();
        let issue_id =
            seed_issue_requiring_exec_gates(&executor, State::Gated, &[("failing", "exit 1")]);

        let error = executor
            .gates()
            .pass_gate(&issue_id, &"failing".parse().unwrap(), None, false)
            .expect_err("a failing checker fails the evaluation");

        assert!(error
            .downcast_ref::<crate::commands::GatePassFailed>()
            .is_some());
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().state,
            State::Gated
        );
    }

    #[test]
    fn test_pass_gate_last_auto_gate_leaves_non_gated_issue_state_unchanged() {
        [
            State::Backlog,
            State::Ready,
            State::InProgress,
            State::Done,
            State::Rejected,
        ]
        .into_iter()
        .for_each(|state| {
            let executor = setup();
            let issue_id = seed_issue_requiring_exec_gates(&executor, state, &[("only", "exit 0")]);

            executor
                .gates()
                .pass_gate(&issue_id, &"only".parse().unwrap(), None, false)
                .unwrap();

            assert_eq!(executor.storage.load_issue(&issue_id).unwrap().state, state);
        });
    }

    #[test]
    fn test_pass_all_gates_completes_gated_issue_after_every_gate_passes() {
        let executor = setup();
        let issue_id = seed_issue_requiring_exec_gates(
            &executor,
            State::Gated,
            &[("first", "exit 0"), ("last", "exit 0")],
        );

        executor
            .gates()
            .pass_all_gates(&issue_id, None, false)
            .unwrap();

        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().state,
            State::Done
        );
    }

    /// `previously-passed` holds a recorded pass while its checker now fails, so
    /// the pending gate before it clears the last unpassed status mid-run.
    #[test]
    fn test_pass_all_gates_later_gate_failure_keeps_issue_gated() {
        let executor = setup();
        let issue_id = seed_issue_requiring_exec_gates(
            &executor,
            State::Gated,
            &[("pending", "exit 0"), ("previously-passed", "exit 1")],
        );
        let mut issue = executor.storage.load_issue(&issue_id).unwrap();
        issue.gates_status.insert(
            "previously-passed".parse().unwrap(),
            crate::domain::GateState {
                status: GateStatus::Passed,
                updated_by: None,
                updated_at: Utc::now(),
                run_id: None,
            },
        );
        seed_issue(executor.storage(), issue);

        executor
            .gates()
            .pass_all_gates(&issue_id, None, false)
            .expect_err("the later gate's failure fails the run");

        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().state,
            State::Gated
        );
    }

    #[test]
    fn test_pass_gate_surfaces_auto_gate_persistence_failure() {
        let executor = setup();
        let issue_id =
            seed_issue_requiring_exec_gates(&executor, State::Gated, &[("postcheck", "exit 0")]);

        let failures = FailNthPublication::new(1);
        let storage = executor.storage().with_mutation_interruption_view(failures);
        let executor = crate::commands::test_helpers::memory_executor(storage);

        let error = executor
            .gates()
            .pass_gate(&issue_id, &"postcheck".parse().unwrap(), None, false)
            .expect_err("gate-run persistence failure must reach the caller");

        assert!(
            error
                .to_string()
                .contains("injected postcheck persistence failure"),
            "unexpected error: {error:#}"
        );
        use crate::storage::RepositoryStateStore;
        let read_error = executor.storage.load_issue(&issue_id).unwrap_err();
        assert!(matches!(
            read_error.downcast_ref::<crate::storage::RepositoryStateStoreError>(),
            Some(crate::storage::RepositoryStateStoreError::RetryableConflict { .. })
        ));
        drop(
            executor
                .storage
                .open_mutation_session(executor.storage.repository_layout())
                .expect("explicit recovery completes the committed redo"),
        );
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Gated);
        let runs = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap();
        assert_eq!(runs.len(), 1);
        let state = &issue.gates_status[&"postcheck".parse().unwrap()];
        assert_eq!(state.run_id.as_ref(), Some(&runs[0].run_id));
        assert_eq!(state.status, GateStatus::Passed);
        assert_eq!(runs[0].status, crate::domain::GateRunStatus::Passed);
    }

    #[test]
    fn test_pass_gate_surfaces_auto_gate_evaluation_failure() {
        let executor = setup();
        let issue_id =
            seed_issue_requiring_exec_gates(&executor, State::Gated, &[("postcheck", "exit 0")]);
        let mut registry = executor.storage.load_gate_registry().unwrap();
        let gate = registry
            .gates
            .get_mut(&"postcheck".parse::<GateKey>().unwrap())
            .unwrap();
        gate.checker = Some(GateChecker::Exec {
            command: "exit 0".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
            pass_context: true,
            prompt: None,
            prompt_file: Some("missing-prompt.md".to_string()),
        });
        seed_gate_registry(executor.storage(), &registry);

        let error = executor
            .gates()
            .pass_gate(&issue_id, &"postcheck".parse().unwrap(), None, false)
            .expect_err("gate evaluation failure must reach the caller");

        assert!(
            error.to_string().contains("missing-prompt.md"),
            "unexpected error: {error:#}"
        );
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Gated);
        assert!(issue.gates_status.is_empty());
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
            "review".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "review".parse().unwrap(),
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
        let mut issue = crate::test_utils::fixture_issue(
            "Implement feature X".to_string(),
            "Add the X feature".to_string(),
        );
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"review".parse().unwrap())
            .unwrap();

        // Check the gate - should build context and pass it
        let result = executor
            .gates()
            .check_gate(&issue_id, &"review".parse().unwrap())
            .unwrap();
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
            "review".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "review".parse().unwrap(),
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
        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"review".parse().unwrap())
            .unwrap();

        let result = executor
            .gates()
            .check_gate(&issue_id, &"review".parse().unwrap())
            .unwrap();
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
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        std::fs::write(repo.path().join("review-prompt.md"), "old prompt").unwrap();
        add_exec_gate(
            &executor,
            &issue_id,
            &"prompt-race".parse().unwrap(),
            "Prompt race",
            "if [ ! -f .prompt-race-seen ]; then : > .prompt-race-seen; printf 'new prompt' > review-prompt.md; fi; printf x >> checker-count; cat \"$JIT_CONTEXT_FILE\"".to_string(),
            Some("review-prompt.md".to_string()),
        );

        let result = executor
            .gates()
            .check_gate(&issue_id, &"prompt-race".parse().unwrap())
            .unwrap();
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
            .filter(|run| run.gate_key == "prompt-race".parse().unwrap())
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
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        let issue_path = VirtualPath::issue_record(&issue_id).repository_relative();
        add_exec_gate(
            &executor,
            &issue_id,
            &"issue-race".parse().unwrap(),
            "Issue race",
            format!(
                "if [ ! -f .issue-race-seen ]; then : > .issue-race-seen; sed -i 's/\"title\": \"Test\"/\"title\": \"Changed concurrently\"/' {issue_path}; fi; printf x >> checker-count; cat \"$JIT_CONTEXT_FILE\""
            ),
            None,
        );

        let result = executor
            .gates()
            .check_gate(&issue_id, &"issue-race".parse().unwrap())
            .unwrap();
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
            .filter(|run| run.gate_key == "issue-race".parse().unwrap())
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
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        let dependency_id = executor
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
            .unwrap()
            .issue
            .id;
        add_reducing_edge(&executor, &issue_id, &dependency_id).unwrap();
        let dependency_path = VirtualPath::issue_record(&dependency_id).repository_relative();
        add_exec_gate(
            &executor,
            &issue_id,
            &"dependency-race".parse().unwrap(),
            "Dependency race",
            format!(
                "if [ ! -f .dependency-race-seen ]; then : > .dependency-race-seen; sed -i 's/\"title\": \"Dependency\"/\"title\": \"Changed dependency\"/' {dependency_path}; fi; printf x >> checker-count; cat \"$JIT_CONTEXT_FILE\""
            ),
            None,
        );

        let result = executor
            .gates()
            .check_gate(&issue_id, &"dependency-race".parse().unwrap())
            .unwrap();
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
            .filter(|run| run.gate_key == "dependency-race".parse().unwrap())
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
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        std::fs::write(
            repo.path().join("gate-race.sh"),
            "if [ ! -f .gate-race-seen ]; then : > .gate-race-seen; sed -i 's/Original race title/Changed race title/' .jit/gates.toml; fi\nprintf x >> checker-count\ncat \"$JIT_CONTEXT_FILE\"\n",
        )
        .unwrap();
        add_exec_gate(
            &executor,
            &issue_id,
            &"gate-race".parse().unwrap(),
            "Original race title",
            "sh gate-race.sh".to_string(),
            None,
        );

        let result = executor
            .gates()
            .check_gate(&issue_id, &"gate-race".parse().unwrap())
            .unwrap();
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
                .filter(|run| run.gate_key == "gate-race".parse().unwrap())
                .count(),
            1
        );
        let run = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .into_iter()
            .find(|run| run.gate_key == "gate-race".parse().unwrap())
            .unwrap();
        assert_eq!(
            run.evaluation_stability,
            crate::domain::GateEvaluationStability::InputsMoved
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_never_retargets_resolved_id_after_replacement() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let created_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        let issue_id = crate::test_utils::issue_id(0x4000_8000_0000_0000_0001_0f0e_0d0c);
        let replacement_id = crate::test_utils::issue_id(0xffff_4fff_8fff_ffff_ffff_ffff_0f0e_0d0c);
        let mut original = executor.storage.load_issue(&created_id).unwrap();
        original.id = issue_id.clone();
        let layout = executor.require_layout().unwrap();
        let created_path = layout
            .resolve(&VirtualPath::issue_record(&created_id))
            .unwrap();
        let issue_path = layout
            .resolve(&VirtualPath::issue_record(&issue_id))
            .unwrap();
        let issue_path_relative = VirtualPath::issue_record(&issue_id).repository_relative();
        let replacement_path = VirtualPath::issue_record(&replacement_id).repository_relative();
        std::fs::remove_file(created_path).unwrap();
        std::fs::write(
            issue_path,
            crate::repository_state::serialize_issue(&original).unwrap(),
        )
        .unwrap();
        add_exec_gate(
            &executor,
            &issue_id,
            &"target-race".parse().unwrap(),
            "Target race",
            format!(
                "if [ ! -f .target-race-seen ]; then : > .target-race-seen; sed -i 's/{issue_id}/{replacement_id}/g' .jit/index.json; rm {issue_path_relative}; cp replacement.json {replacement_path}; fi; true"
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
            .gates()
            .check_gate(&issue_id, &"target-race".parse().unwrap())
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
            .contains_key(&"target-race".parse().unwrap()));
        assert!(executor
            .storage
            .list_gate_runs_for_issue(&replacement_id)
            .unwrap()
            .is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_classifies_named_run_drift_but_not_unrelated_run() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        let mut concurrent = prior_run(
            &crate::test_utils::gate_run_id(0xcc),
            &"run-race".parse().unwrap(),
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
            &"run-race".parse().unwrap(),
            "Run race",
            format!(
                "if [ ! -f .run-race-seen ]; then : > .run-race-seen; {}; fi; printf x >> checker-count; cat \"$JIT_CONTEXT_FILE\"",
                publish_concurrent_run_script(&issue_id, &concurrent.run_id)
            ),
            None,
        );
        name_gate_run(
            &executor,
            &issue_id,
            &"run-race".parse().unwrap(),
            &concurrent.run_id,
        );

        let result = executor
            .gates()
            .check_gate(&issue_id, &"run-race".parse().unwrap())
            .unwrap();
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
            .find(|run| {
                run.gate_key == "run-race".parse().unwrap() && run.run_id != concurrent.run_id
            })
            .unwrap();
        assert_eq!(
            run.evaluation_stability,
            crate::domain::GateEvaluationStability::InputsMoved
        );

        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        concurrent.issue_id = crate::test_utils::issue_id(0x4000_8000_0000_0000_0002_0f0e_0d0c);
        std::fs::write(
            repo.path().join("concurrent-run.json"),
            serde_json::to_vec_pretty(&concurrent).unwrap(),
        )
        .unwrap();
        add_exec_gate(
            &executor,
            &issue_id,
            &"run-race".parse().unwrap(),
            "Run race",
            format!(
                "{}; printf x >> checker-count; cat \"$JIT_CONTEXT_FILE\"",
                publish_concurrent_run_script(&issue_id, &concurrent.run_id)
            ),
            None,
        );

        executor
            .gates()
            .check_gate(&issue_id, &"run-race".parse().unwrap())
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.path().join("checker-count")).unwrap(),
            "x"
        );
        let run = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .into_iter()
            .find(|run| run.gate_key == "run-race".parse().unwrap())
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
        let issue = crate::test_utils::fixture_issue("Apply race".into(), "Test".into());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        let temp = tempfile::tempdir().unwrap();
        let count = temp.path().join("checker-count");
        add_exec_gate(
            &executor,
            &issue_id,
            &"apply-race".parse().unwrap(),
            "Apply race",
            format!("printf x >> {}; true", count.display()),
            None,
        );
        executor.storage.inject_repository_state_apply_conflicts(1);

        let result = executor
            .gates()
            .check_gate(&issue_id, &"apply-race".parse().unwrap())
            .unwrap();

        assert_eq!(std::fs::read_to_string(count).unwrap(), "x");
        let runs = executor
            .storage
            .list_gate_runs_for_issue(&issue_id)
            .unwrap()
            .into_iter()
            .filter(|run| run.gate_key == "apply-race".parse().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, result.run_id);
        assert_eq!(
            runs[0].evaluation_stability,
            crate::domain::GateEvaluationStability::Stable
        );
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().gates_status
                [&"apply-race".parse().unwrap()]
                .status,
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
                        } if event_issue == &issue_id && gate_key == &"apply-race".parse::<GateKey>().unwrap()
                    )
                })
                .count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_records_one_run_and_event_when_inputs_move_during_checker() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        let issue_path = VirtualPath::issue_record(&issue_id).repository_relative();
        add_exec_gate(
            &executor,
            &issue_id,
            &"single-record-race".parse().unwrap(),
            "Single record race",
            format!(
                "if [ ! -f .single-record-race-seen ]; then : > .single-record-race-seen; sed -i 's/\\\"title\\\": \\\"Test\\\"/\\\"title\\\": \\\"Moved\\\"/' {issue_path}; fi; printf x >> checker-count; true"
            ),
            None,
        );

        executor
            .gates()
            .check_gate(&issue_id, &"single-record-race".parse().unwrap())
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
            .filter(|run| run.gate_key == "single-record-race".parse().unwrap())
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
                            if event_issue == &issue_id && gate_key == &"single-record-race".parse::<GateKey>().unwrap()
                    )
                })
                .count(),
            1
        );
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().gates_status
                [&"single-record-race".parse().unwrap()]
                .status,
            crate::domain::GateStatus::Passed
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_check_gate_records_failing_verdict_and_findings_when_inputs_move_during_checker() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config = format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let issue_id = seed_file_repository(&executor, &taxonomy, "Test", "Test");
        let issue_path = VirtualPath::issue_record(&issue_id).repository_relative();
        add_exec_gate(
            &executor,
            &issue_id,
            &"failing-race".parse().unwrap(),
            "Failing race",
            format!(
                r#"if [ ! -f .failing-race-seen ]; then : > .failing-race-seen; sed -i 's/"title": "Test"/"title": "Moved"/' {issue_path}; fi; printf x >> checker-count; printf '%s\n' '<<<JIT-FINDINGS-JSON' '{{"verdict":"fail","summary":"blocking finding","findings":[{{"id":"F1","severity":"high","disposition":"blocking","summary":"broken contract"}}]}}' 'JIT-FINDINGS-JSON>>>'; exit 1"#
            ),
            None,
        );

        let result = executor
            .gates()
            .check_gate(&issue_id, &"failing-race".parse().unwrap())
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
            .find(|run| run.gate_key == "failing-race".parse().unwrap())
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
            executor.storage.load_issue(&issue_id).unwrap().gates_status
                [&"failing-race".parse().unwrap()]
                .status,
            crate::domain::GateStatus::Failed
        );
    }

    #[test]
    fn test_check_gate_run_history_keeps_only_latest_legacy_run() {
        let executor = setup();

        // Define a gate with pass_context that always fails (exit 1) but outputs context
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "review".parse().unwrap(),
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
        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"review".parse().unwrap())
            .unwrap();

        // First run - should have empty history
        let result1 = executor
            .gates()
            .check_gate(&issue_id, &"review".parse().unwrap())
            .unwrap();
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
        let result2 = executor
            .gates()
            .check_gate(&issue_id, &"review".parse().unwrap())
            .unwrap();
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
        let result3 = executor
            .gates()
            .check_gate(&issue_id, &"review".parse().unwrap())
            .unwrap();
        let ctx3: serde_json::Value = serde_json::from_str(&result3.stdout).unwrap();
        let history3 = ctx3["run_history"].as_array().unwrap();
        assert_eq!(history3.len(), 1);
        assert_eq!(history3[0]["run_id"], result2.run_id.as_str());
        assert!(!history3[0]["stdout"].as_str().unwrap().is_empty());
    }

    #[test]
    fn test_prompt_file_path_traversal_rejected() {
        let executor = setup();

        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "review".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "review".parse().unwrap(),
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

        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"review".parse().unwrap())
            .unwrap();

        let result = executor
            .gates()
            .check_gate(&issue_id, &"review".parse().unwrap());
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
            "review".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "review".parse().unwrap(),
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

        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"review".parse().unwrap())
            .unwrap();

        // Run the gate repeatedly to exercise history compaction.
        for _ in 0..7 {
            let _ = executor
                .gates()
                .check_gate(&issue_id, &"review".parse().unwrap())
                .unwrap();
        }

        // The next run receives exactly one latest prior run.
        let result = executor
            .gates()
            .check_gate(&issue_id, &"review".parse().unwrap())
            .unwrap();
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
            "review".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "review".parse().unwrap(),
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

        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"review".parse().unwrap())
            .unwrap();

        let result = executor
            .gates()
            .check_gate(&issue_id, &"review".parse().unwrap());
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
            "test-gate".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "test-gate".parse().unwrap(),
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

        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"test-gate".parse().unwrap())
            .unwrap();

        let result = executor
            .gates()
            .check_gate(&issue_id, &"test-gate".parse().unwrap())
            .unwrap();
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
            "test-gate".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "test-gate".parse().unwrap(),
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

        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"test-gate".parse().unwrap())
            .unwrap();

        // Link a document via the same accessor `jit doc add` uses.
        executor
            .documents()
            .add_document_reference(
                &issue_id,
                "dev/active/my-plan.md",
                None,
                Some("Implementation Plan"),
                Some("design"),
                true, // skip_scan: the path need not exist on disk for this test
            )
            .unwrap();

        let result = executor
            .gates()
            .check_gate(&issue_id, &"test-gate".parse().unwrap())
            .unwrap();
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
            "test-gate".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "test-gate".parse().unwrap(),
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

        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"test-gate".parse().unwrap())
            .unwrap();

        let result = executor
            .gates()
            .check_gate(&issue_id, &"test-gate".parse().unwrap())
            .unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);
        assert_eq!(result.stdout.trim(), "[]");
    }

    #[test]
    fn test_gate_context_includes_enriched_dependencies() {
        let executor = setup();

        // Create two dependency issues
        let dep1 = crate::test_utils::fixture_issue(
            "Setup database schema".to_string(),
            "Create tables".to_string(),
        );
        let dep1_id = dep1.id.clone();
        seed_issue(executor.storage(), dep1);

        let dep2 = crate::test_utils::fixture_issue(
            "Implement auth module".to_string(),
            "OAuth2 flow".to_string(),
        );
        let dep2_id = dep2.id.clone();
        seed_issue(executor.storage(), dep2);

        // Create main issue that depends on both
        let mut main_issue = crate::test_utils::fixture_issue(
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
            "review".parse().unwrap(),
            crate::declarations::GateDefinition {
                version: 1,
                key: "review".parse().unwrap(),
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

        executor
            .gates()
            .add_gate(&main_id, &"review".parse().unwrap())
            .unwrap();

        let result = executor
            .gates()
            .check_gate(&main_id, &"review".parse().unwrap())
            .unwrap();
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
        for (key, priority) in [("gate-p30", 30u32), ("gate-p10", 10), ("gate-p20", 20)]
            .map(|(key, priority)| (key.parse::<GateKey>().unwrap(), priority))
        {
            registry.gates.insert(
                key.clone(),
                crate::declarations::GateDefinition {
                    version: 1,
                    key: key.clone(),
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
        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"gate-p30".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .add_gate(&issue_id, &"gate-p10".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .add_gate(&issue_id, &"gate-p20".parse().unwrap())
            .unwrap();

        executor
            .gates()
            .check_gate(&issue_id, &"gate-p30".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .check_gate(&issue_id, &"gate-p10".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .check_gate(&issue_id, &"gate-p20".parse().unwrap())
            .unwrap();

        let (results, _) = executor
            .gates()
            .get_last_gate_runs_for_issue(&issue_id)
            .unwrap();

        // Results should arrive in priority order: 10, 20, 30
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].gate_key, "gate-p10".parse().unwrap());
        assert_eq!(results[1].gate_key, "gate-p20".parse().unwrap());
        assert_eq!(results[2].gate_key, "gate-p30".parse().unwrap());
    }

    #[test]
    fn test_check_all_gates_stable_sort_same_priority() {
        let executor = setup();

        // Define 3 auto gates all with default priority 100
        let mut registry = executor.storage.load_gate_registry().unwrap();
        for key in ["alpha", "beta", "gamma"].map(|key| key.parse::<GateKey>().unwrap()) {
            registry.gates.insert(
                key.clone(),
                crate::declarations::GateDefinition {
                    version: 1,
                    key: key.clone(),
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
        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"alpha".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .add_gate(&issue_id, &"beta".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .add_gate(&issue_id, &"gamma".parse().unwrap())
            .unwrap();

        executor
            .gates()
            .check_gate(&issue_id, &"alpha".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .check_gate(&issue_id, &"beta".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .check_gate(&issue_id, &"gamma".parse().unwrap())
            .unwrap();

        let (results, _) = executor
            .gates()
            .get_last_gate_runs_for_issue(&issue_id)
            .unwrap();

        // Same-priority gates should maintain insertion order
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].gate_key, "alpha".parse().unwrap());
        assert_eq!(results[1].gate_key, "beta".parse().unwrap());
        assert_eq!(results[2].gate_key, "gamma".parse().unwrap());
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

    fn make_auto_gate(key: &GateKey, command: &str) -> crate::declarations::GateDefinition {
        crate::declarations::GateDefinition {
            version: 1,
            key: key.clone(),
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
        registry.gates.insert(
            "g".parse().unwrap(),
            make_auto_gate(&"g".parse().unwrap(), "exit 0"),
        );
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::test_utils::fixture_issue("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"g".parse().unwrap())
            .unwrap();

        let result = executor
            .gates()
            .get_last_gate_run(&issue_id, &"g".parse().unwrap())
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_last_gate_run_returns_most_recent() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "g".parse().unwrap(),
            make_auto_gate(&"g".parse().unwrap(), "exit 0"),
        );
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::test_utils::fixture_issue("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"g".parse().unwrap())
            .unwrap();

        let first = executor
            .gates()
            .check_gate(&issue_id, &"g".parse().unwrap())
            .unwrap();
        let second = executor
            .gates()
            .check_gate(&issue_id, &"g".parse().unwrap())
            .unwrap();

        let last = executor
            .gates()
            .get_last_gate_run(&issue_id, &"g".parse().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(last.run_id, second.run_id);
        assert_ne!(last.run_id, first.run_id);
    }

    #[test]
    fn test_get_last_gate_run_filters_by_gate_key() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "gate-a".parse().unwrap(),
            make_auto_gate(&"gate-a".parse().unwrap(), "exit 0"),
        );
        registry.gates.insert(
            "gate-b".parse().unwrap(),
            make_auto_gate(&"gate-b".parse().unwrap(), "exit 0"),
        );
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::test_utils::fixture_issue("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"gate-a".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .add_gate(&issue_id, &"gate-b".parse().unwrap())
            .unwrap();

        executor
            .gates()
            .check_gate(&issue_id, &"gate-a".parse().unwrap())
            .unwrap();
        let b_run = executor
            .gates()
            .check_gate(&issue_id, &"gate-b".parse().unwrap())
            .unwrap();

        let last_a = executor
            .gates()
            .get_last_gate_run(&issue_id, &"gate-a".parse().unwrap())
            .unwrap()
            .unwrap();
        let last_b = executor
            .gates()
            .get_last_gate_run(&issue_id, &"gate-b".parse().unwrap())
            .unwrap()
            .unwrap();

        assert_eq!(last_a.gate_key, "gate-a".parse().unwrap());
        assert_eq!(last_b.gate_key, "gate-b".parse().unwrap());
        assert_eq!(last_b.run_id, b_run.run_id);
        assert_ne!(last_a.run_id, last_b.run_id);
    }

    #[test]
    fn test_get_last_gate_run_shows_failure_details() {
        let executor = setup();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "fail-gate".parse().unwrap(),
            make_auto_gate(&"fail-gate".parse().unwrap(), "echo 'oops' && exit 1"),
        );
        seed_gate_registry(executor.storage(), &registry);

        let issue = crate::test_utils::fixture_issue("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"fail-gate".parse().unwrap())
            .unwrap();

        executor
            .gates()
            .check_gate(&issue_id, &"fail-gate".parse().unwrap())
            .unwrap();

        let last = executor
            .gates()
            .get_last_gate_run(&issue_id, &"fail-gate".parse().unwrap())
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
