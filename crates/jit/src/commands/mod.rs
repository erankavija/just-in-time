//! Command execution logic for all CLI operations.
//!
//! The `CommandExecutor` handles all business logic for issue management,
//! dependency manipulation, gate operations, and event logging.
//!
//! This module is organized into submodules by functional area:
//! - `issue`: Issue CRUD operations and lifecycle management
//! - `dependency`: Dependency graph operations  
//! - `breakdown`: Bracket-aware breakdown operations
//! - `template`: Graph-template apply engine (`jit apply <template> <container>`):
//!   the plan-before-fan-out scaffold (planning node `P` + breakdown node `B`),
//!   validating, snapshotting anchors, and committing the expanded delta
//! - `template_expand`: pure expansion of a graph template into the delta the
//!   apply engine commits (issues to create, edges to add/remove, anchor gates)
//! - `config`: `jit config set` write (target-file resolution, typed value
//!   validation incl. `project.name`, atomic write) and `jit config get`'s
//!   generic dotted-path accessor over the whole configuration surface
//! - `gate`: Quality gate operations
//! - `graph`: Graph visualization and traversal
//! - `query`: Issue query operations
//! - `validate`: Validation and status operations
//! - `plan_doc`: Plan-document location resolver (boundary; loads inline/external
//!   plan content and feeds the pure projection engine)
//! - `labels`: Label operations
//! - `document`: Document reference operations
//! - `events`: Event log operations
//! - `search`: Issue search operations
//! - `item`: Addressable-item queries (`jit item list/show/search`) over the pure
//!   item model

mod archive;
pub mod batch_create;
mod breakdown;
pub mod bulk_update;
pub mod claim;
mod config;
mod dependency;
mod document;
mod events;
mod gate;
mod gate_check;
mod gate_cli_tests;
pub mod graph;
pub mod hooks;
mod init;
pub mod invariant;
mod issue;
pub mod item;
mod labels;
pub mod migrate;
pub mod plan_doc;
mod profile;
pub mod project;
mod query;
mod search;
pub mod serve;
pub mod snapshot;
mod template;
pub mod template_expand;
mod validate;
pub mod worktree;

#[cfg(test)]
pub mod test_helpers;

pub use archive::ArchiveExecutionHooks;
pub use batch_create::{
    BatchCreateOutcome, BatchIssueDef, BatchValidationError, BatchValidationProblem,
};
pub use breakdown::{BracketBreakdownResult, BracketChild};
pub use bulk_update::{BulkUpdatePreview, BulkUpdateResult, UpdateOperations};
pub use config::{resolve_dotted_key, ConfigGetOutcome, ConfigKeyError, ConfigSetOutcome};
pub use gate::{
    FieldEdit, GateNotRequiredError, GatePassAllEntry, GatePassFailed, GatePassOutcome, GateUpdate,
    ManualGateAttestationRequiredError, PassAllOutcome,
};
pub use graph::{BatchExport, BoundaryEdge, GraphExportFormat};
pub use init::FreshInitResult;
pub use invariant::InvariantCheckResult;
pub use issue::DescriptionUpdate;
pub use item::{ItemListResult, ItemShowResult};
pub use migrate::LifecycleBackfillResult;
pub use profile::ProfileApplyError;
pub use project::{ProjectRenderResult, ProjectionRenderReport};
pub use template::TemplateApplyResult;
pub use template_expand::{
    expand_template, validate_delta_acyclic, AnchorGates, DeltaEdge, DeltaEndpoint, PlannedNode,
    TemplateDelta,
};
pub(crate) use validate::{find_planning_node, planning_node_plan_path, validate_claims_index_at};
pub use validate::{DANGLING_LINK_RULE, ENFORCEMENT_DRIFT_RULE, REVIEW_PLACEHOLDER_RULE};

// Re-export WorktreeIdentity for init return type
pub use crate::storage::worktree_identity::WorktreeIdentity;

// Common imports used across modules
use crate::config::JitConfig;
use crate::config_manager::ConfigManager;
use crate::declarations::rules::{RuleConfigError, RuleSet};
use crate::declarations::{GateDefinition, GateMode};
use crate::domain::{
    is_dependency_met, Event, GateState, GateStatus, Issue, LabelNamespaces, Priority, State,
};
use crate::graph::DependencyGraph;
use crate::labels as label_utils;
use crate::storage::IssueStore;
// Type hierarchy validation (currently only validates type labels)
use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::sync::OnceLock;

/// Finalizer-assigned record identities returned after one semantic publication.
struct MutationPublication {
    created_issue_ids: Vec<String>,
    gate_run_ids: Vec<String>,
}

/// Closed issue-local operations whose final record is derived from the issue
/// captured by the mutation session. Ordinary commands use this instead of
/// constructing a full-record `UpdateIssue` from an earlier storage read.
#[derive(Clone)]
enum CapturedIssueMutation {
    Assign {
        issue_id: String,
        assignee: crate::domain::Assignee,
    },
    Unassign {
        issue_id: String,
    },
    AddGates {
        issue_id: String,
        gate_keys: Vec<String>,
    },
    RemoveGates {
        issue_id: String,
        gate_keys: Vec<String>,
    },
    SetManualGateStatus {
        issue_id: String,
        gate_key: String,
        status: GateStatus,
        by: Option<crate::domain::Assignee>,
    },
}

impl CapturedIssueMutation {
    fn issue_id(&self) -> &str {
        match self {
            Self::Assign { issue_id, .. }
            | Self::Unassign { issue_id }
            | Self::AddGates { issue_id, .. }
            | Self::RemoveGates { issue_id, .. }
            | Self::SetManualGateStatus { issue_id, .. } => issue_id,
        }
    }

    fn captures_gate_registry(&self) -> bool {
        matches!(
            self,
            Self::AddGates { .. } | Self::SetManualGateStatus { .. }
        )
    }
}

enum CapturedIssueMutationOutcome {
    Changed,
    Unchanged,
    GatesAdded {
        added: Vec<String>,
        already_exist: Vec<String>,
    },
    GatesRemoved {
        removed: Vec<String>,
        not_found: Vec<String>,
    },
}

struct DerivedCapturedIssueMutation {
    outcome: CapturedIssueMutationOutcome,
    intents: Vec<crate::repository_state::MutationIntent>,
}

/// Pure result of deriving one lifecycle transition from a closed repository
/// image. Graph-rule refusals carry their audit append so the coordinator can
/// commit the attempted transition before returning the typed error.
enum DerivedStateTransition {
    Applied {
        issue: Box<Issue>,
        warnings: Vec<String>,
        events: PhasedEvents,
        changed: bool,
    },
    GraphBlocked {
        error: crate::errors::TransitionBlockedError,
        events: PhasedEvents,
    },
}

struct CapturedTransitionEvidence<'a> {
    issues: &'a [Issue],
    declarations: &'a ImageDeclarations,
    config: &'a JitConfig,
    plan_content: &'a std::collections::HashMap<String, String>,
    context: &'a crate::repository_state::MutationContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrecheckEvidence {
    inputs: Vec<(
        crate::repository_state::VirtualPath,
        Option<crate::repository_state::EntryIdentity>,
    )>,
    validation_view: Option<PrecheckValidationView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrecheckValidationView {
    listings: std::collections::BTreeMap<
        crate::repository_state::VirtualPath,
        crate::repository_state::ListingFingerprint,
    >,
    pinned: std::collections::BTreeMap<
        (String, String),
        crate::repository_state::PinnedDocumentEvidence,
    >,
    linked: std::collections::BTreeMap<
        crate::repository_state::VirtualPath,
        crate::repository_state::LinkedWorktreeEvidence,
    >,
}

impl PrecheckEvidence {
    fn matches(&self, image: &crate::repository_state::RepositoryImage) -> Result<bool> {
        let inputs = self
            .inputs
            .iter()
            .map(|(path, _)| {
                image
                    .entry(path)
                    .map(|entry| (path.clone(), entry.identity().cloned()))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let validation_view = self
            .validation_view
            .as_ref()
            .map(|_| PrecheckValidationView {
                listings: image.listing_fingerprints().clone(),
                pinned: image.pinned_evidence().clone(),
                linked: image.linked_worktree_evidence().clone(),
            });
        Ok(inputs == self.inputs && validation_view == self.validation_view)
    }
}

struct CapturedPrecheckPlan {
    evidence: PrecheckEvidence,
    prompts: std::collections::HashMap<String, String>,
}

struct CachedPrecheckExecution {
    target_id: String,
    evidence: PrecheckEvidence,
    execution: gate_check::PrecheckExecution,
}

fn captured_precheck_plan(
    image: &crate::repository_state::RepositoryImage,
    issue: &Issue,
    registry: &crate::declarations::GateRegistry,
) -> Result<CapturedPrecheckPlan> {
    use crate::declarations::{GateChecker, GateStage};
    use crate::repository_state::VirtualPath;
    use std::collections::{BTreeSet, HashMap};

    let gates = issue
        .gates_required
        .iter()
        .filter_map(|key| registry.gates.get(key).map(|gate| (key, gate)))
        .filter(|(_, gate)| gate.stage == GateStage::Precheck)
        .collect::<Vec<_>>();
    let broad_builtin = gates.iter().any(|(_, gate)| {
        gate.mode == GateMode::Auto
            && !matches!(gate.checker, Some(GateChecker::Exec { .. }))
            && !matches!(gate.checker, Some(GateChecker::ReviewPlaceholder))
    });
    let mut paths = if broad_builtin {
        image.entries().keys().cloned().collect::<BTreeSet<_>>()
    } else {
        BTreeSet::from([
            VirtualPath::data(format!("issues/{}.json", issue.id))?,
            VirtualPath::data("gates.toml")?,
        ])
    };
    if !broad_builtin {
        paths.extend(
            issue
                .dependencies
                .iter()
                .map(|id| VirtualPath::data(format!("issues/{id}.json")))
                .collect::<std::result::Result<Vec<_>, _>>()?,
        );
        for (path, entry) in image.entries() {
            let relative = path.relative().as_str();
            if relative.starts_with("gate-runs/") && relative.ends_with("/result.json") {
                if let crate::repository_state::RepositoryEntry::File { bytes, .. } = entry {
                    let run: crate::domain::GateRunResult = serde_json::from_slice(bytes)?;
                    if run.issue_id == issue.id {
                        paths.insert(path.clone());
                    }
                }
            }
        }
    }
    let mut prompts = HashMap::new();
    for (key, gate) in gates {
        if gate.mode != GateMode::Auto {
            continue;
        }
        if let Some(GateChecker::Exec {
            pass_context: true,
            prompt_file: Some(path),
            ..
        }) = &gate.checker
        {
            let path = repo_rel_virtual_path(path)?;
            let bytes = image
                .file_bytes(&path)?
                .ok_or_else(|| anyhow!("captured precheck prompt file '{:?}' is missing", path))?;
            prompts.insert(key.clone(), String::from_utf8(bytes.to_vec())?);
            paths.insert(path);
        }
    }
    let inputs = paths
        .into_iter()
        .map(|path| {
            image
                .entry(&path)
                .map(|entry| (path, entry.identity().cloned()))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let validation_view = broad_builtin.then(|| PrecheckValidationView {
        listings: image.listing_fingerprints().clone(),
        pinned: image.pinned_evidence().clone(),
        linked: image.linked_worktree_evidence().clone(),
    });
    Ok(CapturedPrecheckPlan {
        evidence: PrecheckEvidence {
            inputs,
            validation_view,
        },
        prompts,
    })
}

fn claims_mutation_guard(
    layout: &crate::repository_state::RepositoryLayout,
) -> Result<Option<crate::storage::claim_coordinator::ClaimsMutationGuard>> {
    use crate::storage::worktree_paths::WorktreePaths;
    use crate::storage::{ClaimCoordinator, FileLocker};
    use std::time::Duration;

    let in_git = std::process::Command::new("git")
        .arg("-C")
        .arg(layout.worktree_root())
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .is_ok_and(|output| output.status.success());
    if !in_git {
        return Ok(None);
    }
    let paths = WorktreePaths::detect_from(layout.worktree_root())?;
    let agent = crate::agent_config::resolve_agent_id(None)
        .unwrap_or_else(|_| "system:lease-check".to_string());
    let worktree_id = crate::storage::worktree_identity::read_worktree_id(layout.worktree_root())?
        .unwrap_or_else(|| layout.worktree_root().display().to_string());
    let coordinator = ClaimCoordinator::new(
        paths,
        FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        )),
        worktree_id,
        agent,
    );
    coordinator.lock_claims_for_repository_mutation().map(Some)
}

fn captured_lease_warnings(
    mode: crate::config::EnforcementMode,
    targets: &[String],
    issues: &[Issue],
    guard: Option<&crate::storage::claim_coordinator::ClaimsMutationGuard>,
) -> Result<Vec<String>> {
    use crate::agent_config::resolve_agent_id;
    use crate::config::EnforcementMode;

    if mode == EnforcementMode::Off {
        return Ok(Vec::new());
    }
    let agent = resolve_agent_id(None).ok();
    let mut warnings = Vec::new();
    for id in targets {
        let active = guard
            .map(|guard| {
                guard.has_active_lease(id, agent.as_deref(), |raw| {
                    resolve_issue_from_capture(issues, raw)
                })
            })
            .transpose()?
            .unwrap_or(false);
        if active {
            continue;
        }
        let message =
            format!("No active lease for issue {id}.\nAcquire lease with: jit claim acquire {id}");
        match mode {
            EnforcementMode::Warn => warnings.push(message),
            EnforcementMode::Strict => return Err(anyhow!(message)),
            EnforcementMode::Off => unreachable!(),
        }
    }
    Ok(warnings)
}

fn ensure_claim_available(
    guard: Option<&crate::storage::claim_coordinator::ClaimsMutationGuard>,
    target: &str,
    assignee: &crate::domain::Assignee,
    issues: &[Issue],
) -> Result<()> {
    let Some(lease) = guard
        .map(|guard| {
            guard.conflicting_lease(target, Some(&assignee.to_string()), |raw| {
                resolve_issue_from_capture(issues, raw)
            })
        })
        .transpose()?
        .flatten()
    else {
        return Ok(());
    };
    let expires = lease
        .expires_at
        .map(|time| time.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "indefinitely".to_string());
    Err(anyhow!(
        "Issue {} is currently leased by {} {}.\nUse 'jit claim acquire' to properly coordinate work.",
        target,
        lease.agent_id,
        expires
    ))
}

#[derive(Clone)]
enum CapturedLifecycleMutation {
    State {
        issue_id: String,
        target: State,
        force: bool,
        divert_unpassed_gates: bool,
        enforce_lease: bool,
    },
    Claim {
        issue_id: String,
        assignee: crate::domain::Assignee,
    },
    Release {
        issue_id: String,
        reason: String,
    },
    AutoReady {
        issue_id: String,
    },
    AutoDone {
        issue_id: String,
    },
}

impl CapturedLifecycleMutation {
    fn issue_id(&self) -> &str {
        match self {
            Self::State { issue_id, .. }
            | Self::Claim { issue_id, .. }
            | Self::Release { issue_id, .. }
            | Self::AutoReady { issue_id }
            | Self::AutoDone { issue_id } => issue_id,
        }
    }
}

fn lifecycle_requires_prechecks(request: &CapturedLifecycleMutation, issue: &Issue) -> bool {
    issue.state == State::Ready
        && matches!(
            request,
            CapturedLifecycleMutation::State {
                target: State::InProgress,
                ..
            } | CapturedLifecycleMutation::Claim { .. }
        )
}

struct CapturedLifecycleOutcome {
    target_id: String,
    changed: bool,
    warnings: Vec<String>,
    storage_warnings: Vec<crate::storage::StorageWarning>,
}

#[derive(Clone)]
struct CapturedFieldUpdate {
    issue_id: String,
    title: Option<String>,
    description: Option<DescriptionUpdate>,
    priority: Option<Priority>,
    state: Option<State>,
    add_labels: Vec<String>,
    remove_labels: Vec<String>,
    content_format: Option<Option<crate::domain::ContentFormat>>,
    issue_type: Option<String>,
    add_gates: Vec<String>,
    remove_gates: Vec<String>,
    assignee: Option<crate::domain::Assignee>,
    unassign: bool,
    bulk: bool,
    force: bool,
    enforce_lease: bool,
}

struct DerivedFieldUpdate {
    changed: bool,
    warnings: Vec<String>,
    intents: Vec<crate::repository_state::MutationIntent>,
    error_after_apply: Option<crate::errors::TransitionBlockedError>,
}

struct CapturedFieldUpdateOutcome {
    changed: bool,
    warnings: Vec<String>,
}

fn derive_write_validation(
    issue: &Issue,
    declarations: &ImageDeclarations,
    config: &JitConfig,
    force: bool,
) -> Result<WriteValidation> {
    let repo_format = config
        .validation
        .as_ref()
        .map(crate::config::ValidationConfig::content_format)
        .transpose()?
        .unwrap_or(crate::domain::ContentFormat::Markdown);
    let strictness = config
        .validation
        .as_ref()
        .map(crate::config::ValidationConfig::strictness)
        .transpose()?
        .unwrap_or_default();
    let evaluation = crate::validation::evaluate_local(issue, &declarations.rules, repo_format)
        .map_err(|error| anyhow!("rule evaluation failed: {error}"))?
        .with_strictness(strictness);
    let blocking = evaluation.blocking_rules();
    if !blocking.is_empty() && !force {
        return Err(crate::errors::ValidationFailedError::new(
            evaluation
                .rejection_message()
                .unwrap_or_else(|| "blocked by validation rule(s)".to_string()),
        )
        .into());
    }
    Ok(WriteValidation {
        warnings: evaluation.warnings(),
        bypassed_rules: blocking,
    })
}

/// Derive one state transition without storage access.
fn derive_state_transition(
    mut issue: Issue,
    target: State,
    force: bool,
    evidence: CapturedTransitionEvidence<'_>,
) -> Result<DerivedStateTransition> {
    use crate::declarations::rules::{RuleScope, Severity};
    use crate::validation::graph::evaluate_graph;
    use std::collections::HashSet;

    let old_state = issue.state;
    if old_state == target {
        return Ok(DerivedStateTransition::Applied {
            issue: Box::new(issue),
            warnings: Vec::new(),
            events: Vec::new(),
            changed: false,
        });
    }

    let mut warnings = Vec::new();
    if old_state == State::Archived {
        match issue.archived_from {
            Some(origin) if target != origin => {
                return Err(crate::errors::TransitionBlockedError::archived_revive(
                    issue.id.clone(),
                    target,
                    origin,
                )
                .into());
            }
            None => warnings.push(format!(
                "issue {} was archived before its pre-archive state was recorded; reviving to \
                 '{}' without a verified origin",
                issue.short_id(),
                target.as_str()
            )),
            Some(_) => {}
        }
    }

    if matches!(target, State::Ready | State::Done) {
        let blockers = issue::blocking_dependencies(
            &issue,
            &crate::domain::queries::build_issue_map(evidence.issues),
        );
        if !blockers.is_empty() {
            return Err(crate::errors::TransitionBlockedError::dependencies(
                issue.id.clone(),
                target,
                old_state,
                blockers,
            )
            .into());
        }
        if target == State::Done && issue.has_unpassed_gates() {
            return Err(crate::errors::TransitionBlockedError::gates(
                issue.id.clone(),
                target,
                old_state,
                unpassed_gate_blockers(&issue, &evidence.declarations.gates),
            )
            .into());
        }
    }

    let mut bypass_events = Vec::new();
    if !matches!(target, State::Rejected | State::Archived) {
        let mut projected = issue.clone();
        projected.state = target;
        let rules = evidence
            .declarations
            .rules
            .rules
            .iter()
            .filter(|rule| rule.scope == RuleScope::Graph && rule.severity != Severity::Off)
            .filter(|rule| !rule.assert.is_repo_wide_at_transition())
            .filter(|rule| rule.when.matches(&projected))
            .collect::<Vec<_>>();
        if !rules.is_empty() {
            let refs = evidence.issues.iter().collect::<Vec<_>>();
            let graph = DependencyGraph::new(&refs);
            let mut ids = HashSet::from([projected.id.clone()]);
            ids.extend(
                graph
                    .get_transitive_dependents(&projected.id)
                    .into_iter()
                    .map(|candidate| candidate.id.clone()),
            );
            ids.extend(
                graph
                    .get_transitive_dependencies(&projected.id)
                    .into_iter()
                    .map(|candidate| candidate.id.clone()),
            );
            let slice = evidence
                .issues
                .iter()
                .filter(|candidate| ids.contains(&candidate.id))
                .map(|candidate| {
                    if candidate.id == projected.id {
                        projected.clone()
                    } else {
                        candidate.clone()
                    }
                })
                .collect::<Vec<_>>();
            let namespaces = crate::config_manager::namespaces_from_config(evidence.config);
            let hierarchy = crate::repository_state::hierarchy_config(&namespaces);
            let repo_format = evidence
                .config
                .validation
                .as_ref()
                .map(crate::config::ValidationConfig::content_format)
                .transpose()?
                .unwrap_or(crate::domain::ContentFormat::Markdown);
            let strictness = evidence
                .config
                .validation
                .as_ref()
                .map(crate::config::ValidationConfig::strictness)
                .transpose()?
                .unwrap_or_default();
            let enforcing = rules
                .iter()
                .filter(|rule| rule.enforce)
                .map(|rule| rule.name.as_str())
                .collect::<HashSet<_>>();
            let findings = evaluate_graph(
                &rules,
                &slice,
                &hierarchy,
                repo_format,
                evidence.context.timestamp(),
                evidence.plan_content,
            );
            let mut blocking = Vec::new();
            for finding in findings {
                let config_error = finding.is_config_error();
                let pertains =
                    config_error || finding.issue_id.as_deref() == Some(projected.id.as_str());
                if pertains
                    && strictness.blocks(
                        enforcing.contains(finding.finding.rule.as_str()),
                        finding.finding.severity,
                    )
                {
                    let message = if config_error {
                        format!(
                            "rule '{}' is misconfigured: {}; fix the rule or use --force",
                            finding.finding.rule, finding.finding.message
                        )
                    } else {
                        finding.finding.message.clone()
                    };
                    blocking.push((finding.finding.rule, message));
                } else {
                    warnings.push(format!(
                        "[{}] {}",
                        finding.finding.rule, finding.finding.message
                    ));
                }
            }
            if !blocking.is_empty() {
                let events = blocking
                    .iter()
                    .map(|(rule, _)| {
                        (
                            1,
                            if force {
                                Event::draft_graph_rule_bypassed(
                                    issue.id.clone(),
                                    target,
                                    rule.clone(),
                                )
                            } else {
                                Event::draft_transition_blocked(
                                    issue.id.clone(),
                                    target,
                                    rule.clone(),
                                )
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                if force {
                    bypass_events = events;
                } else {
                    return Ok(DerivedStateTransition::GraphBlocked {
                        error: crate::errors::TransitionBlockedError::graph_rules(
                            issue.id.clone(),
                            target,
                            old_state,
                            blocking,
                        ),
                        events,
                    });
                }
            }
        }
    }

    issue.state = target;
    if target == State::Archived {
        issue.archived_from = Some(old_state);
    } else if old_state == State::Archived {
        issue.archived_from = None;
    }
    bypass_events.push((
        2,
        Event::draft_issue_state_changed(issue.id.clone(), old_state, target),
    ));
    if target == State::Done {
        bypass_events.push((3, Event::draft_issue_completed(issue.id.clone())));
    }
    Ok(DerivedStateTransition::Applied {
        issue: Box::new(issue),
        warnings,
        events: bypass_events,
        changed: true,
    })
}

fn derive_field_update(
    original: Issue,
    request: &CapturedFieldUpdate,
    evidence: CapturedTransitionEvidence<'_>,
) -> Result<DerivedFieldUpdate> {
    use crate::repository_state::MutationIntent;

    let mut issue = original.clone();
    if let Some(title) = &request.title {
        issue.title = title.clone();
    }
    if let Some(description) = &request.description {
        issue.description = description.clone().apply(&issue.description);
    }
    if let Some(priority) = request.priority {
        issue.priority = priority;
    }
    if let Some(content_format) = request.content_format {
        issue.content_format = content_format;
    }
    for label in &request.add_labels {
        if !issue.labels.contains(label) {
            issue.labels.push(label.clone());
        }
    }
    for label in &request.remove_labels {
        issue.labels.retain(|candidate| candidate != label);
    }
    let missing_gates = request
        .add_gates
        .iter()
        .filter(|key| !evidence.declarations.gates.gates.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    if !missing_gates.is_empty() {
        return Err(crate::storage::GateNotFoundError::new(missing_gates).into());
    }
    for gate in &request.add_gates {
        if !issue.gates_required.contains(gate) {
            issue.gates_required.push(gate.clone());
        }
    }
    for gate in &request.remove_gates {
        issue.gates_required.retain(|candidate| candidate != gate);
        issue.gates_status.remove(gate);
    }
    if let Some(assignee) = &request.assignee {
        issue.assignee = Some(assignee.clone());
    } else if request.unassign {
        issue.assignee = None;
    }
    if let Some(kind) = &request.issue_type {
        issue
            .labels
            .retain(|label| !label_utils::is_type_label(label));
        issue.labels.push(label_utils::type_label(kind));
    }

    let mut changed_fields = Vec::new();
    if issue.title != original.title {
        changed_fields.push("title".to_string());
    }
    if issue.description != original.description {
        changed_fields.push("description".to_string());
    }
    if issue.priority != original.priority {
        changed_fields.push("priority".to_string());
    }
    if issue.labels != original.labels {
        changed_fields.push("labels".to_string());
    }
    if issue.content_format != original.content_format {
        changed_fields.push("content_format".to_string());
    }
    if issue.gates_required != original.gates_required {
        changed_fields.push("gates".to_string());
    }
    if issue.assignee != original.assignee {
        changed_fields.push("assignee".to_string());
    }

    let mut gate_error = None;
    let target = match request.state {
        Some(State::Done) if issue.has_unpassed_gates() => {
            let blockers = issue::blocking_dependencies(
                &issue,
                &crate::domain::queries::build_issue_map(evidence.issues),
            );
            if !blockers.is_empty() {
                return Err(crate::errors::TransitionBlockedError::dependencies(
                    issue.id.clone(),
                    State::Done,
                    original.state,
                    blockers,
                )
                .into());
            }
            gate_error = Some(crate::errors::TransitionBlockedError::gates(
                issue.id.clone(),
                State::Done,
                State::Gated,
                unpassed_gate_blockers(&issue, &evidence.declarations.gates),
            ));
            Some(State::Gated)
        }
        target => target,
    };

    let mut projected = issue.clone();
    if let Some(target) = target {
        projected.state = target;
    }
    let validation = derive_write_validation(
        &projected,
        evidence.declarations,
        evidence.config,
        request.force,
    )?;
    if request.issue_type.is_some() {
        let repo_format = evidence
            .config
            .validation
            .as_ref()
            .map(crate::config::ValidationConfig::content_format)
            .transpose()?
            .unwrap_or(crate::domain::ContentFormat::Markdown);
        let explicit_type = crate::validation::evaluate_local(
            &projected,
            &evidence.declarations.rules,
            repo_format,
        )
        .map_err(|error| anyhow!("rule evaluation failed: {error}"))?;
        if let Some(finding) = explicit_type
            .findings()
            .into_iter()
            .find(|finding| finding.rule == "type-hierarchy-known")
        {
            return Err(crate::errors::ValidationFailedError::new(finding.message.clone()).into());
        }
    }

    let mut warnings = validation.warnings;
    let mut events = Vec::new();
    if let Some(target) = target {
        match derive_state_transition(
            issue,
            target,
            request.force,
            CapturedTransitionEvidence { ..evidence },
        )? {
            DerivedStateTransition::Applied {
                issue: transitioned,
                warnings: transition_warnings,
                events: transition_events,
                ..
            } => {
                issue = *transitioned;
                warnings.extend(transition_warnings);
                events.extend(transition_events);
            }
            DerivedStateTransition::GraphBlocked {
                error,
                events: blocked_events,
            } => {
                return Ok(DerivedFieldUpdate {
                    changed: false,
                    warnings,
                    intents: blocked_events
                        .into_iter()
                        .map(|(phase, event)| MutationIntent::RecordEvent {
                            phase,
                            event: Box::new(event),
                        })
                        .collect(),
                    error_after_apply: Some(error),
                });
            }
        }
    }

    let changed = !changed_fields.is_empty() || issue.state != original.state;
    if issue.assignee != original.assignee {
        if let Some(assignee) = issue.assignee.clone() {
            events.push((4, Event::draft_issue_claimed(issue.id.clone(), assignee)));
        }
    }
    if !changed_fields.is_empty() {
        events.push((
            5,
            Event::draft_issue_updated(
                issue.id.clone(),
                if request.bulk {
                    "bulk-update".to_string()
                } else {
                    "issue-update".to_string()
                },
                changed_fields,
            ),
        ));
    }
    events.extend(
        validation
            .bypassed_rules
            .into_iter()
            .map(|rule| (9, Event::draft_local_rule_bypassed(issue.id.clone(), rule))),
    );
    let intents = changed
        .then(|| MutationIntent::UpdateIssue {
            issue: Box::new(issue),
        })
        .into_iter()
        .chain(
            events
                .into_iter()
                .map(|(phase, event)| MutationIntent::RecordEvent {
                    phase,
                    event: Box::new(event),
                }),
        )
        .collect();
    Ok(DerivedFieldUpdate {
        changed,
        warnings,
        intents,
        error_after_apply: gate_error,
    })
}

/// Derive the full-record low-level intent and its audit events from one captured
/// issue. Keeping this pure makes the retry contract directly testable: every
/// attempt receives the new image's record and cannot retain an ambient preimage.
fn derive_captured_issue_mutation(
    mut issue: Issue,
    registry: &crate::declarations::GateRegistry,
    request: &CapturedIssueMutation,
) -> Result<DerivedCapturedIssueMutation> {
    use crate::repository_state::MutationIntent;

    let (outcome, events, changed) = match request {
        CapturedIssueMutation::Assign { assignee, .. } => {
            if issue.assignee.as_ref() == Some(assignee) {
                (CapturedIssueMutationOutcome::Unchanged, Vec::new(), false)
            } else {
                issue.assignee = Some(assignee.clone());
                let event = Event::draft_issue_claimed(issue.id.clone(), assignee.clone());
                (
                    CapturedIssueMutationOutcome::Changed,
                    vec![(1, event)],
                    true,
                )
            }
        }
        CapturedIssueMutation::Unassign { .. } => {
            if issue.assignee.is_none() {
                (CapturedIssueMutationOutcome::Unchanged, Vec::new(), false)
            } else {
                issue.assignee = None;
                let event = Event::draft_issue_updated(
                    issue.id.clone(),
                    "issue-unassign".to_string(),
                    vec!["assignee".to_string()],
                );
                (
                    CapturedIssueMutationOutcome::Changed,
                    vec![(1, event)],
                    true,
                )
            }
        }
        CapturedIssueMutation::AddGates { gate_keys, .. } => {
            let missing = gate_keys
                .iter()
                .filter(|key| !registry.gates.contains_key(*key))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Err(crate::storage::GateNotFoundError::new(missing).into());
            }
            let mut added = Vec::new();
            let mut already_exist = Vec::new();
            for key in gate_keys {
                if issue.gates_required.contains(key) {
                    already_exist.push(key.clone());
                    continue;
                }
                issue.gates_required.push(key.clone());
                issue.gates_status.entry(key.clone()).or_insert(GateState {
                    status: GateStatus::Pending,
                    updated_by: None,
                    updated_at: chrono::DateTime::default(),
                });
                added.push(key.clone());
            }
            let events = added
                .iter()
                .map(|key| (1, Event::draft_gate_added(issue.id.clone(), key.clone())))
                .collect();
            let changed = !added.is_empty();
            (
                CapturedIssueMutationOutcome::GatesAdded {
                    added,
                    already_exist,
                },
                events,
                changed,
            )
        }
        CapturedIssueMutation::RemoveGates { gate_keys, .. } => {
            let mut removed = Vec::new();
            let mut not_found = Vec::new();
            for key in gate_keys {
                if !issue.gates_required.contains(key) {
                    not_found.push(key.clone());
                    continue;
                }
                issue.gates_required.retain(|required| required != key);
                issue.gates_status.remove(key);
                removed.push(key.clone());
            }
            let events = removed
                .iter()
                .map(|key| (1, Event::draft_gate_removed(issue.id.clone(), key.clone())))
                .collect();
            let changed = !removed.is_empty();
            (
                CapturedIssueMutationOutcome::GatesRemoved { removed, not_found },
                events,
                changed,
            )
        }
        CapturedIssueMutation::SetManualGateStatus {
            gate_key,
            status,
            by,
            ..
        } => {
            if !matches!(status, GateStatus::Passed | GateStatus::Failed) {
                return Err(anyhow!("manual gate status must be passed or failed"));
            }
            if !issue.gates_required.contains(gate_key) {
                return Err(gate::GateNotRequiredError {
                    issue_id: issue.id.clone(),
                    gate_key: gate_key.clone(),
                }
                .into());
            }
            let gate = registry
                .gates
                .get(gate_key)
                .ok_or_else(|| crate::storage::GateNotFoundError::single(gate_key))?;
            if gate.mode == GateMode::Auto {
                return Err(anyhow!(
                    "Gate '{}' is automated and cannot be manually changed. Use 'jit gate evaluate {} {}' to run the checker.",
                    gate_key,
                    issue.id,
                    gate_key
                ));
            }
            let replacement = GateState {
                status: *status,
                updated_by: by.clone(),
                updated_at: chrono::DateTime::default(),
            };
            issue.gates_status.insert(gate_key.clone(), replacement);
            let event = match status {
                GateStatus::Passed => {
                    Event::draft_gate_passed(issue.id.clone(), gate_key.clone(), by.clone())
                }
                GateStatus::Failed => {
                    Event::draft_gate_failed(issue.id.clone(), gate_key.clone(), by.clone())
                }
                GateStatus::Pending => unreachable!("validated manual gate evidence status"),
            };
            (
                CapturedIssueMutationOutcome::Changed,
                vec![(1, event)],
                true,
            )
        }
    };
    let intents = if changed {
        std::iter::once(MutationIntent::UpdateIssue {
            issue: Box::new(issue),
        })
        .chain(
            events
                .into_iter()
                .map(|(phase, event)| MutationIntent::RecordEvent {
                    phase,
                    event: Box::new(event),
                }),
        )
        .collect()
    } else {
        Vec::new()
    };
    Ok(DerivedCapturedIssueMutation { outcome, intents })
}

/// The unpassed gates of `issue` paired with their current status and registry
/// mode, in the order [`Issue::get_unpassed_gates`] reports them.
///
/// A required gate with no recorded run counts as [`GateStatus::Pending`]. A
/// gate key missing from `registry` (should not normally happen) defaults to
/// [`GateMode::Manual`], matching [`CommandExecutor::pass_gate`]'s own
/// fallback: an unregistered gate never matches the `Auto` branch there
/// either, so it is treated as requiring `--by` just the same. Shared by the
/// transition guard and the gate-diversion path so both describe a
/// gate-blocked completion with the same blockers, and so the remediation
/// hint can name the `--by <attestor>` form for a manual gate (jit:1d59070d
/// REQ-03).
fn unpassed_gate_blockers(
    issue: &Issue,
    registry: &crate::declarations::GateRegistry,
) -> Vec<(String, GateStatus, GateMode)> {
    issue
        .get_unpassed_gates()
        .into_iter()
        .map(|gate_key| {
            let status = issue
                .gates_status
                .get(&gate_key)
                .map(|gate| gate.status)
                .unwrap_or(GateStatus::Pending);
            let mode = registry
                .gates
                .get(&gate_key)
                .map(|gate| gate.mode)
                .unwrap_or(GateMode::Manual);
            (gate_key, status, mode)
        })
        .collect()
}

/// Information about a git commit
#[derive(Debug, Clone, Serialize)]
pub struct CommitInfo {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub message: String,
}

/// Status summary for all issues
#[derive(Debug, Serialize)]
pub struct StatusSummary {
    pub open: usize, // Backlog count (kept as 'open' for compatibility)
    pub ready: usize,
    pub in_progress: usize,
    pub gated: usize,
    pub done: usize,
    pub rejected: usize, // New: count of rejected issues
    pub blocked: usize,
    pub total: usize,
}

/// Result of listing document references for an issue
#[derive(Debug, Serialize)]
pub struct DocumentListResult {
    pub issue_id: String,
    pub documents: Vec<crate::domain::DocumentReference>,
    pub count: usize,
}

/// Git history for a document
#[derive(Debug, Serialize)]
pub struct DocumentHistory {
    pub path: String,
    pub commits: Vec<CommitInfo>,
}

/// Result of listing assets for a document
#[derive(Debug, Serialize)]
pub struct AssetListResult {
    pub issue_id: String,
    pub document_path: String,
    /// Number of assets in `assets` (list envelope `count`; equals `assets.len()`).
    pub count: usize,
    pub assets: Vec<crate::document::Asset>,
    pub summary: AssetSummary,
    pub warnings: Vec<String>,
}

/// Document content display result
#[derive(Debug, Serialize)]
pub struct DocumentContentResult {
    pub path: String,
    pub label: Option<String>,
    pub commit: String,
    pub doc_type: Option<String>,
    pub content: String,
}

/// Document diff result
#[derive(Debug, Serialize)]
pub struct DocumentDiffResult {
    pub path: String,
    pub from_commit: String,
    pub to_commit: String,
    pub diff: String,
}

/// Result of adding a document reference
#[derive(Debug, Serialize)]
pub struct DocumentAddResult {
    pub issue_id: String,
    pub document: crate::domain::DocumentReference,
    /// `true` when `path` was already linked to the issue and this call
    /// refreshed that entry in place; `false` when it appended a new one.
    pub updated: bool,
}

/// Result of removing a document reference
#[derive(Debug, Serialize)]
pub struct DocumentRemoveResult {
    pub issue_id: String,
    pub path: String,
}

/// Summary of asset counts by category
#[derive(Debug, Serialize)]
pub struct AssetSummary {
    pub total: usize,
    pub per_doc: usize,
    pub shared: usize,
    pub external: usize,
    pub missing: usize,
}

/// Result of exporting a snapshot
#[derive(Debug, Serialize)]
pub struct SnapshotExportResult {
    pub path: String,
    pub issue_count: usize,
    pub document_count: usize,
    pub format: String,
    pub size_bytes: Option<u64>,
}

/// Result of checking document links
#[derive(Debug, Serialize)]
pub struct LinkCheckResult {
    pub valid: bool,
    pub errors: Vec<serde_json::Value>,
    pub warnings: Vec<serde_json::Value>,
    #[serde(skip)]
    pub exit_code: i32,
    #[serde(skip)]
    pub scope: String,
    pub summary: LinkCheckSummary,
}

/// Summary of link check results
#[derive(Debug, Serialize)]
pub struct LinkCheckSummary {
    pub total_documents: usize,
    pub valid: usize,
    pub errors: usize,
    pub warnings: usize,
}

/// Result of adding a dependency
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyAddResult {
    /// Dependency was added
    Added,
    /// Dependency was skipped because it's transitive (redundant)
    Skipped { reason: String },
    /// Dependency already existed
    AlreadyExists,
}

/// How `jit dep add` treats an edge that would break transitive reduction.
///
/// Cycle detection is a write-time guard (@/inv/dag-acyclic); this policy makes the
/// transitive-reduction property a write-time guard too, closing the asymmetry
/// where a redundant edge was written silently and only rejected at a later
/// `jit validate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RedundancyPolicy {
    /// Reject the add, naming the offending edge pair (nonzero exit). Default.
    #[default]
    Reject,
    /// Add the edge and drop the now-redundant edge(s) in the same operation,
    /// leaving the graph transitively reduced.
    Reduce,
}

/// Outcome of the unified write-time validation pass.
///
/// Produced by the executor's `validate_for_write` entry point BEFORE an issue
/// is persisted. It carries the non-blocking warnings to surface to the caller
/// and the list of `enforce` rules that a `--force` write is bypassing. The
/// bypass events are intentionally NOT emitted during validation: the caller
/// emits them through the captured mutation only after validation succeeds, so a save
/// that fails never leaves a false "bypass happened" entry in the audit log.
#[derive(Debug, Clone, Default)]
pub struct WriteValidation {
    /// Non-blocking warnings (legacy validator + local `warn`/non-enforce
    /// findings) to surface to the caller.
    pub warnings: Vec<String>,
    /// Names of `enforce` rules whose blocking findings were overridden by
    /// `--force`. One [`Event::LocalRuleBypassed`] must be logged per entry,
    /// AFTER the write commits.
    pub bypassed_rules: Vec<String>,
}

type PhasedEvents = Vec<(u8, Event)>;

/// Outcome of [`CommandExecutor::sync_default_rule_membership`]: the
/// `namespace-unique-*` default-rule NAMES appended to `.jit/rules.toml` and
/// those dropped from it. Both empty means the file already matched the
/// `[namespaces]` registry (a no-op write).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleMembershipSync {
    /// Names of the `origin = "default"` `namespace-unique-<ns>` rows
    /// appended, in [`default_ruleset`](crate::repository_state::default_ruleset)'s
    /// emission order.
    pub added: Vec<String>,
    /// Names of the `origin = "default"` `namespace-unique-<ns>` rows dropped.
    pub dropped: Vec<String>,
}

/// Owned repository declarations assembled from a captured [`RepositoryImage`].
///
/// The declaration bundle a session-driven producer graph consumes — the parsed
/// configuration, the authored gate registry, and the EFFECTIVE rule set (the
/// authored `rules.toml` with its `origin = "default"` family reconciled against
/// the configuration, or the in-memory defaults when no `rules.toml` was
/// captured) — every part read from the captured image bytes, never the live
/// filesystem.
pub(crate) struct ImageDeclarations {
    configuration: crate::declarations::ConfigurationDeclarations,
    gates: crate::declarations::GateRegistry,
    rules: RuleSet,
}

impl ImageDeclarations {
    /// Borrow these owned declarations as the bundle the pure producers take.
    pub(crate) fn borrowed(&self) -> crate::repository_state::RepositoryDeclarations<'_> {
        crate::repository_state::RepositoryDeclarations {
            configuration: &self.configuration,
            gates: &self.gates,
            rules: &self.rules,
        }
    }
}

/// Convert repo-relative planned changes into a canonical overlay-override map.
///
/// Each `(repo-relative path, Some(bytes) | None)` becomes a
/// `(VirtualPath, Some|None)` entry (`.jit/...` is `Data`, everything else
/// `Worktree`) suitable for
/// [`apply_overlay`](crate::repository_state::apply_overlay) and the overlaid
/// validation capture. Production init/profile now compose typed deltas directly;
/// this remains as a proposed-overlay test helper for validation/gate-check suites
/// that need a compact `.jit`-prefix path adapter.
#[cfg(test)]
pub(crate) fn overrides_from_repo_changes(
    changes: impl IntoIterator<Item = (std::path::PathBuf, Option<Vec<u8>>)>,
) -> Result<std::collections::BTreeMap<crate::repository_state::VirtualPath, Option<Vec<u8>>>> {
    use crate::repository_state::VirtualPath;
    changes
        .into_iter()
        .map(|(path, value)| {
            let repo_rel = path.to_string_lossy();
            let vpath = match repo_rel.strip_prefix(".jit/") {
                Some(rest) => VirtualPath::data(rest),
                None => VirtualPath::worktree(repo_rel.as_ref()),
            }?;
            Ok((vpath, value))
        })
        .collect()
}

/// Read a repo-relative path's bytes from the captured image.
///
/// A `.jit/`-prefixed path is a `Data(...)` entry and every other repo-relative
/// path a `Worktree(...)` entry, matching the materialization producers' mapping.
/// `Ok(None)` when the captured entry is absent or not a file.
fn image_repo_bytes(
    image: &crate::repository_state::RepositoryImage,
    repo_rel: &str,
) -> Result<Option<Vec<u8>>> {
    use crate::repository_state::VirtualPath;
    let vpath = match repo_rel.strip_prefix(".jit/") {
        Some(rest) => VirtualPath::data(rest),
        None => VirtualPath::worktree(repo_rel),
    }?;
    Ok(image.file_bytes(&vpath)?.map(<[u8]>::to_vec))
}

/// Project a finalized delta to the file-overlay validation reads: each written
/// file's bytes and each deleted file's absence, ignoring directory and mode
/// actions. This is the exact proposed repository state — only what the delta
/// writes — used by init/profile to validate their proposed state under one
/// coherent captured image.
pub(crate) fn validation_overlay(
    delta: &crate::repository_state::RepositoryDelta,
) -> std::collections::BTreeMap<crate::repository_state::VirtualPath, Option<Vec<u8>>> {
    use crate::repository_state::RepositoryAction;
    delta
        .actions()
        .iter()
        .filter_map(|action| match action {
            RepositoryAction::WriteFile { path, bytes, .. } => {
                Some((path.clone(), Some(bytes.clone())))
            }
            RepositoryAction::DeleteFile { path, .. } => Some((path.clone(), None)),
            RepositoryAction::CreateDirectory { .. } | RepositoryAction::SetMode { .. } => None,
        })
        .collect()
}

/// Map a repository-relative path to its canonical virtual path (`.jit/...` is
/// `Data`, everything else `Worktree`).
pub(crate) fn repo_rel_virtual_path(path: &str) -> Result<crate::repository_state::VirtualPath> {
    use crate::repository_state::VirtualPath;
    match path.strip_prefix(".jit/") {
        Some(rest) => Ok(VirtualPath::data(rest)?),
        None => Ok(VirtualPath::worktree(path)?),
    }
}

/// Assemble the configuration, effective rule set, and gate registry from a
/// captured image, ready to drive the session-based materialization producers.
///
/// The effective rule set reconciles the authored `rules.toml`'s default family
/// against the configuration exactly as the query-path loader does, but reads the
/// rules bytes and every referenced schema from the captured image rather than the
/// filesystem — so a mutation derives from the same closed evidence it will
/// revalidate before journaling. A missing `.jit/config.toml` in the closure is a
/// capture error.
pub(crate) fn declarations_from_image(
    image: &crate::repository_state::RepositoryImage,
) -> Result<ImageDeclarations> {
    use crate::declarations::{parse_configuration, parse_gate_registry, GateRegistry};
    use crate::repository_state::{
        assemble_config, default_ruleset, reconcile_default_rules_with_config, VirtualPath,
    };

    let config_bytes = image_repo_bytes(image, ".jit/config.toml")?
        .ok_or_else(|| anyhow!("captured image has no .jit/config.toml"))?;
    let configuration = parse_configuration(&config_bytes)?;
    let jit_config = assemble_config(image)?;
    let namespaces = crate::config_manager::namespaces_from_config(&jit_config);

    let gates = match image_repo_bytes(image, ".jit/gates.toml")? {
        Some(bytes) => parse_gate_registry(&bytes)?,
        None => GateRegistry::default(),
    };

    let rules = match image_repo_bytes(image, ".jit/rules.toml")? {
        Some(bytes) => {
            let content = String::from_utf8(bytes)?;
            // Schema references are data-root relative (`schemas/...`); the render
            // closure captures them, so `file_bytes` resolves without a filesystem
            // read. A captured-absent schema simply contributes no validator bytes.
            let schemas = RuleSet::schema_requests(&content)?
                .into_iter()
                .filter_map(|request| {
                    let vpath = VirtualPath::data(&request.reference).ok()?;
                    let bytes = image.file_bytes(&vpath).ok().flatten()?.to_vec();
                    Some((request.reference, bytes))
                })
                .collect::<Vec<_>>();
            let user = RuleSet::parse(&content, Some(&jit_config), schemas)?;
            reconcile_default_rules_with_config(user, &namespaces)
        }
        None => default_ruleset(&namespaces),
    };

    Ok(ImageDeclarations {
        configuration,
        gates,
        rules,
    })
}

/// Parse the active issue set from the captured index and issue records.
fn captured_active_issues(image: &crate::repository_state::RepositoryImage) -> Result<Vec<Issue>> {
    use crate::repository_state::{RepositoryEntry, VirtualPath};

    let index_path = VirtualPath::data("index.json")?;
    let index_bytes = image
        .file_bytes(&index_path)?
        .ok_or_else(|| anyhow!("captured image has no .jit/index.json"))?;
    let index = crate::storage::json::parse_repository_index(index_bytes)?;
    let mut issues = index
        .all_ids
        .iter()
        .map(|id| {
            let path = VirtualPath::data(format!("issues/{id}.json"))?;
            let bytes = match image.entry(&path)? {
                RepositoryEntry::File { bytes, .. } => bytes,
                RepositoryEntry::Absent => {
                    return Err(crate::storage::IssueNotFoundError::new(id).into())
                }
                _ => return Err(anyhow!("indexed issue {id} is not an ordinary file")),
            };
            let issue: Issue = serde_json::from_slice(bytes)
                .with_context(|| format!("failed to parse captured issue {id}"))?;
            if issue.id != *id {
                return Err(anyhow!(
                    "indexed issue {id} contains mismatched embedded id {}",
                    issue.id
                ));
            }
            Ok(issue)
        })
        .collect::<Result<Vec<_>>>()?;
    issues.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(issues)
}

fn resolve_issue_from_capture(issues: &[Issue], requested: &str) -> Result<String> {
    if let Some(issue) = issues.iter().find(|issue| issue.id == requested) {
        return Ok(issue.id.clone());
    }
    let normalized = requested.to_lowercase().replace('-', "");
    if normalized.len() < crate::storage::MIN_ID_PREFIX_LENGTH {
        return Err(crate::storage::InvalidIdPrefixError::new(requested).into());
    }
    let matches = issues
        .iter()
        .filter(|issue| {
            issue
                .id
                .to_lowercase()
                .replace('-', "")
                .starts_with(&normalized)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(crate::storage::IssueNotFoundError::new(requested).into()),
        [issue] => Ok(issue.id.clone()),
        _ => Err(crate::storage::AmbiguousIdError::issue(
            requested,
            matches
                .iter()
                .map(|issue| format!("{} | {}", issue.short_id(), issue.title)),
        )
        .into()),
    }
}

/// Executes CLI commands with business logic and validation.
///
/// Generic over storage backend to support different implementations
/// (JSON files, SQLite, in-memory, etc.).
pub struct CommandExecutor<S: IssueStore> {
    storage: S,
    pub config_manager: ConfigManager,
    /// Lazily-parsed `.jit/rules.toml`, cached for the lifetime of the
    /// executor so the validation ruleset is read at most once per process
    /// rather than re-parsed on every write. The cached value retains any
    /// load/parse error so callers can surface a misconfigured rules file
    /// instead of silently treating it as "no rules".
    rules: OnceLock<Result<RuleSet, RuleConfigError>>,
    /// Lazily-built EFFECTIVE rule set. When `.jit/rules.toml` is present it
    /// supplies the rules, with the `origin = "default"` family reconciled against
    /// this repo's `config.toml` registry at load
    /// ([`reconcile_default_rules_with_config`](crate::repository_state::reconcile_default_rules_with_config));
    /// when absent, the built-in
    /// [`default_ruleset`](crate::repository_state::default_ruleset) is built
    /// IN MEMORY. The former hard-coded checks (a0f0f342 migration) now live as
    /// default rules here. A load/parse error from either source is retained as an
    /// `Err` so a misconfigured repo surfaces the problem rather than silently
    /// disabling enforcement.
    effective_rules: OnceLock<Result<RuleSet, String>>,
    /// Lazily-loaded `.jit/config.toml`, cached so the unified write-time
    /// validation entry point does not re-read and re-parse `config.toml` on
    /// every write. The parsed config seeds the built-in default rules. A
    /// malformed config is retained as an `Err` so it is surfaced rather than
    /// swallowed.
    config: OnceLock<Result<JitConfig, String>>,
    /// Lazily-built label namespace registry, derived from the cached config and
    /// cached alongside it for the same reason.
    namespaces: OnceLock<Result<LabelNamespaces, String>>,
    /// The canonical repository layout every session this executor opens mutates
    /// through.
    ///
    /// This is the PERMANENT construction seam of the executor boundary, not an
    /// interim shim: the executor is constructed with its canonical layout at
    /// startup (from the CLI's Git-optional worktree root and selected data root,
    /// or a fixture layout in tests), and every session-opening command opens
    /// `open_mutation_session` for exactly this layout. The layout always arrives
    /// from the construction boundary and is never inferred from the storage
    /// parent (the layout authority forbids that). Deepening this seam to a fully
    /// retained recovered session — reentrant only for this same canonical layout
    /// (`@/inv` layout coherence) — changes what the executor holds internally, not
    /// these call sites. `None` only when no layout was supplied; a session-opening
    /// command then reports a wiring error rather than proceeding.
    layout: Option<crate::repository_state::RepositoryLayout>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Create a new command executor with the given storage
    pub fn new(storage: S) -> Self {
        let config_manager = ConfigManager::new(storage.root());
        Self {
            storage,
            config_manager,
            rules: OnceLock::new(),
            effective_rules: OnceLock::new(),
            config: OnceLock::new(),
            namespaces: OnceLock::new(),
            layout: None,
        }
    }

    /// Construct this executor over its canonical [`RepositoryLayout`].
    ///
    /// The permanent construction surface for the layout every session-opening
    /// command mutates through (see the [`layout`](Self::layout) field). The CLI
    /// supplies it at startup from the Git-optional worktree root plus the selected
    /// data root; tests supply a fixture layout over a temporary worktree.
    pub fn with_layout(mut self, layout: crate::repository_state::RepositoryLayout) -> Self {
        self.layout = Some(layout);
        self
    }

    /// The canonical mutation layout, or a typed error when none was supplied.
    ///
    /// A session-opening command needs an explicit worktree/data-root layout; a
    /// missing one is a construction-wiring error, reported rather than inferred
    /// from the storage parent.
    pub(crate) fn require_layout(&self) -> Result<crate::repository_state::RepositoryLayout> {
        self.layout
            .clone()
            .ok_or_else(|| anyhow!("no repository layout configured for this command"))
    }
    /// Publish one closed set of repository-owned issue/gate-run/audit intents.
    ///
    /// Capture paths are derived solely from the typed intents, so command callers
    /// cannot submit a partial snapshot. Each retry opens a fresh session while
    /// reusing the operation's sole identity/time authority.
    fn publish_repository_mutation(
        &self,
        intents: Vec<crate::repository_state::MutationIntent>,
    ) -> Result<MutationPublication>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.publish_repository_mutation_with(|_| Ok(intents.clone()))
    }

    fn publish_repository_mutation_with<F>(&self, build_intents: F) -> Result<MutationPublication>
    where
        S: crate::storage::RepositoryStateStore,
        F: Fn(
            &crate::repository_state::MutationContext,
        ) -> Result<Vec<crate::repository_state::MutationIntent>>,
    {
        use crate::repository_state::{
            finalize, CaptureBudget, CaptureSpec, MutationIntent, VirtualPath,
        };
        use crate::storage::RepositoryStateStoreError;
        use std::collections::BTreeSet;

        let layout = self.require_layout()?;
        // Operation-scoped: each retry gets a fresh recovered session while
        // identifiers and mutation time remain stable.
        let context = crate::repository_state::MutationContext::production();
        for _ in 0..8 {
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let intents = build_intents(&context)?;
            let create_count = intents
                .iter()
                .map(|intent| match intent {
                    MutationIntent::CreateIssue { .. } => 1,
                    MutationIntent::CreateIssueBatch { drafts, .. } => drafts.len(),
                    _ => 0,
                })
                .sum();
            let created_issue_ids = (0..create_count)
                .map(|index| context.identifier_at(index as u64))
                .collect::<Vec<_>>();
            let mut gate_keys = intents
                .iter()
                .filter_map(|intent| match intent {
                    MutationIntent::RecordGateRun { draft } => {
                        Some((draft.issue_id.clone(), draft.gate_key.clone()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            gate_keys.sort();
            let gate_run_ids = (0..gate_keys.len())
                .map(|index| context.identifier_at((create_count + index) as u64))
                .collect::<Vec<_>>();

            let mut paths = BTreeSet::new();
            for intent in &intents {
                match intent {
                    MutationIntent::CreateIssue { .. }
                    | MutationIntent::CreateIssueBatch { .. } => {
                        paths.insert(VirtualPath::data("issues")?);
                        paths.insert(VirtualPath::data("index.json")?);
                        paths.insert(VirtualPath::data("events.jsonl")?);
                    }
                    MutationIntent::ClaimIssue { issue_id, .. } => {
                        paths.insert(VirtualPath::data(format!("issues/{issue_id}.json"))?);
                        paths.insert(VirtualPath::data("events.jsonl")?);
                    }
                    MutationIntent::UpdateIssue { issue }
                    | MutationIntent::RepairIssueLifecycle { issue } => {
                        paths.insert(VirtualPath::data(format!("issues/{}.json", issue.id))?);
                    }
                    MutationIntent::DeleteIssue { issue_id } => {
                        paths.insert(VirtualPath::data(format!("issues/{issue_id}.json"))?);
                        paths.insert(VirtualPath::data("index.json")?);
                    }
                    MutationIntent::EditGateRegistry { .. } => {
                        paths.insert(VirtualPath::data("gates.toml")?);
                    }
                    MutationIntent::RecordGateRun { .. } => {}
                    MutationIntent::RecordEvent { .. } => {
                        paths.insert(VirtualPath::data("events.jsonl")?);
                    }
                }
            }
            for id in &created_issue_ids {
                paths.insert(VirtualPath::data(format!("issues/{id}.json"))?);
            }
            for id in &gate_run_ids {
                paths.insert(VirtualPath::data("gate-runs")?);
                paths.insert(VirtualPath::data(format!("gate-runs/{id}"))?);
                paths.insert(VirtualPath::data(format!("gate-runs/{id}/result.json"))?);
            }
            let budget = CaptureBudget {
                max_paths: paths.len().saturating_add(16),
                max_listings: 0,
                max_bytes: 64 * 1024 * 1024,
                max_depth: 8,
            };
            let spec = CaptureSpec::phase_one(paths, budget)?;
            let image = match session.capture(spec) {
                Ok(image) => image,
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            };
            let plan = finalize(&layout, &image, &context, &intents)?;
            match session.apply(&plan) {
                Ok(_) => {
                    return Ok(MutationPublication {
                        created_issue_ids,
                        gate_run_ids,
                    })
                }
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(anyhow!(
            "repository mutation did not converge after repeated capture conflicts"
        ))
    }

    /// Rebase one closed issue-local operation on a freshly captured record.
    ///
    /// The request determines its complete capture set. The caller cannot pass a
    /// record, capture closure, or arbitrary patch. A conflict reopens a session
    /// and derives again while retaining the operation's identity/time context.
    fn publish_captured_issue_mutation(
        &self,
        request: CapturedIssueMutation,
    ) -> Result<CapturedIssueMutationOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{
            finalize, CaptureBudget, CaptureSpec, MutationContext, RepositoryEntry, VirtualPath,
        };
        use crate::storage::RepositoryStateStoreError;
        use std::collections::BTreeSet;

        let layout = self.require_layout()?;
        let context = MutationContext::production();
        for _ in 0..8 {
            let mut paths = BTreeSet::from([
                VirtualPath::data(format!("issues/{}.json", request.issue_id()))?,
                VirtualPath::data("events.jsonl")?,
            ]);
            if request.captures_gate_registry() {
                paths.insert(VirtualPath::data("gates.toml")?);
            }
            let spec = CaptureSpec::phase_one(
                paths.clone(),
                CaptureBudget {
                    max_paths: paths.len(),
                    max_listings: 0,
                    max_bytes: 64 * 1024 * 1024,
                    max_depth: 4,
                },
            )?;
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let image = match session.capture(spec) {
                Ok(image) => image,
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            };
            let issue_path = VirtualPath::data(format!("issues/{}.json", request.issue_id()))?;
            let issue: Issue = match image.entry(&issue_path)? {
                RepositoryEntry::File { bytes, .. } => {
                    serde_json::from_slice(bytes).with_context(|| {
                        format!("failed to parse captured issue {}", request.issue_id())
                    })?
                }
                RepositoryEntry::Absent => {
                    return Err(crate::storage::IssueNotFoundError::new(request.issue_id()).into())
                }
                _ => return Err(anyhow!("captured issue path is not an ordinary file")),
            };
            if issue.id != request.issue_id() {
                return Err(anyhow!(
                    "captured issue identity mismatch: requested {}, found {}",
                    request.issue_id(),
                    issue.id
                ));
            }
            let registry = if request.captures_gate_registry() {
                let path = VirtualPath::data("gates.toml")?;
                match image.entry(&path)? {
                    RepositoryEntry::File { bytes, .. } => {
                        crate::declarations::parse_gate_registry(bytes)
                            .context("failed to parse captured gate registry")?
                    }
                    RepositoryEntry::Absent => crate::declarations::GateRegistry::default(),
                    _ => return Err(anyhow!("captured gate registry is not an ordinary file")),
                }
            } else {
                crate::declarations::GateRegistry::default()
            };

            let derived = derive_captured_issue_mutation(issue, &registry, &request)?;
            if derived.intents.is_empty() {
                return Ok(derived.outcome);
            }
            let plan = finalize(&layout, &image, &context, &derived.intents)?;
            match session.apply(&plan) {
                Ok(_) => return Ok(derived.outcome),
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(anyhow!(
            "captured issue mutation did not converge after repeated conflicts"
        ))
    }

    /// Transitional boundary for callers that still publish records loaded before
    /// session capture. New callers must use a closed semantic request instead.
    fn publish_ambient_issue_mutation(
        &self,
        updates: Vec<Issue>,
        events: Vec<(u8, Event)>,
    ) -> Result<()>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::MutationIntent;
        let intents = updates
            .into_iter()
            .map(|issue| MutationIntent::UpdateIssue {
                issue: Box::new(issue),
            })
            .chain(
                events
                    .into_iter()
                    .map(|(phase, event)| MutationIntent::RecordEvent {
                        phase,
                        event: Box::new(event),
                    }),
            )
            .collect();
        self.publish_repository_mutation(intents).map(|_| ())
    }

    fn publish_issue_creation(&self, draft: Issue, bypassed_rules: &[String]) -> Result<String>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::MutationIntent;
        let publication = self.publish_repository_mutation_with(|context| {
            let issue_id = context.identifier_at(0);
            Ok(std::iter::once(MutationIntent::CreateIssue {
                draft: Box::new(draft.clone()),
            })
            .chain(
                bypassed_rules
                    .iter()
                    .map(|rule| MutationIntent::RecordEvent {
                        phase: 9,
                        event: Box::new(Event::draft_local_rule_bypassed(
                            issue_id.clone(),
                            rule.clone(),
                        )),
                    }),
            )
            .collect())
        })?;
        publication
            .created_issue_ids
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("issue finalizer returned no created issue identity"))
    }

    fn publish_gate_evaluation(
        &self,
        mut result: crate::domain::GateRunResult,
        issue: Issue,
        event: Event,
    ) -> Result<crate::domain::GateRunResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::MutationIntent;
        let intents = vec![
            MutationIntent::UpdateIssue {
                issue: Box::new(issue),
            },
            MutationIntent::RecordGateRun {
                draft: Box::new(result.clone()),
            },
            MutationIntent::RecordEvent {
                phase: 1,
                event: Box::new(event),
            },
        ];
        let publication = self.publish_repository_mutation(intents)?;
        result.run_id = publication
            .gate_run_ids
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("gate-run finalizer returned no record identity"))?;
        Ok(result)
    }

    /// Get reference to the storage backend
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Return the parsed validation ruleset, loading `.jit/rules.toml` on first
    /// access and caching the result for subsequent calls.
    ///
    /// A MISSING `.jit/rules.toml` is not an error: it yields `Ok(`an empty
    /// [`RuleSet`]`)`. A genuine parse or load failure (malformed TOML, an
    /// invalid `assert` table, an unsafe schema reference, etc.) is returned as
    /// `Err` rather than being swallowed, so a misconfigured repository cannot
    /// silently disable all rule enforcement. The load is performed at most once
    /// and the outcome (success or failure) is cached.
    pub fn rules(&self) -> Result<&RuleSet, &RuleConfigError> {
        self.rules
            .get_or_init(|| {
                let config =
                    self.config_manager
                        .load()
                        .map_err(|error| RuleConfigError::Configuration {
                            message: error.to_string(),
                        })?;
                crate::storage::ruleset_store::load_ruleset(self.storage.root(), &config)
            })
            .as_ref()
    }

    /// Return the EFFECTIVE rule set, with `.jit/rules.toml` as the operative
    /// ruleset and its `origin = "default"` rules reconciled against `config.toml`
    /// at load (DR §8.2/§8.4).
    ///
    /// Semantics depend on whether the file EXISTS, not whether it is empty:
    ///
    /// - **File present (even with zero rules):** the parsed file supplies the
    ///   operative rule set — which rules exist and, for the built-in rules `jit
    ///   init` scaffolds (marked `origin = "default"`), their editable policy
    ///   fields (severity, enforce, selector). The default family is reconciled
    ///   against the declared `[namespaces]` / `[type_hierarchy]` registry IN
    ///   MEMORY at load
    ///   ([`reconcile_default_rules_with_config`](crate::repository_state::reconcile_default_rules_with_config)):
    ///   assertions are re-derived, a rule is added for a newly-declared namespace
    ///   and dropped for a removed one, so a hand edit of the registry cannot
    ///   desync validation against a stale `schemas/default-*.json` projection.
    ///   Custom rules (any other `origin`)
    ///   are used verbatim, reading their own declared schema files. An
    ///   intentionally-emptied file yields an empty set.
    /// - **File ABSENT (pre-init repo or deleted file):** build the FIXED
    ///   [`default_ruleset`](crate::repository_state::default_ruleset) from the
    ///   repo's namespace registry IN MEMORY (MF4). This is read-only — NO disk
    ///   write, NO warning, NO error — so gates, the server, read-only checkouts,
    ///   and multiple worktrees stay safe. Only `jit init` materializes the file
    ///   to disk (under the write lock).
    ///
    /// The result is built at most once and cached. A malformed `config.toml` or
    /// `.jit/rules.toml` is surfaced as an `Err` rather than silently dropping
    /// enforcement.
    pub fn effective_rules(&self) -> Result<&RuleSet> {
        self.effective_rules
            .get_or_init(|| {
                let rules_path = self.storage.root().join("rules.toml");
                if rules_path.exists() {
                    // File present (even empty) supplies the operative rule set.
                    // Its default-origin rules are a config PROJECTION: reconcile
                    // the default family against the declared registry IN MEMORY at
                    // load — re-derive assertions, add a rule for a newly-declared
                    // namespace, drop one whose namespace is gone — so the baked
                    // `schemas/default-*.json` files are never the authority. Custom
                    // rules and the default rules' editable policy fields
                    // (severity/enforce/selector) are taken from the file unchanged.
                    let user = self.rules().map_err(|e| e.to_string())?.clone();
                    let namespaces = self.cached_namespaces().map_err(|e| e.to_string())?;
                    Ok(
                        crate::repository_state::reconcile_default_rules_with_config(
                            user, namespaces,
                        ),
                    )
                } else {
                    // File absent: build the fixed defaults IN MEMORY (no write,
                    // no warning) from the repo's namespace registry. Materialized
                    // to disk only by `jit init`.
                    let namespaces = self.cached_namespaces().map_err(|e| e.to_string())?;
                    Ok(crate::repository_state::default_ruleset(namespaces))
                }
            })
            .as_ref()
            .map_err(|err| anyhow!("{err}"))
    }

    /// Return the cached parsed `.jit/config.toml`, loading it on first access.
    ///
    /// The config is read at most once per executor (cached in a `OnceLock`) so
    /// the unified write-validation path does not re-parse `config.toml` on every
    /// write. A malformed config is surfaced as an error rather than swallowed.
    fn cached_config(&self) -> Result<&JitConfig> {
        self.config
            .get_or_init(|| self.config_manager.load().map_err(|err| err.to_string()))
            .as_ref()
            .map_err(|err| {
                crate::errors::InvalidArgumentError::new(format!("invalid .jit/config.toml: {err}"))
                    .into()
            })
    }

    /// Return the cached label namespace registry, building it on first access.
    ///
    /// Cached alongside [`cached_config`](Self::cached_config) so the registry is
    /// derived at most once per executor.
    fn cached_namespaces(&self) -> Result<&LabelNamespaces> {
        self.namespaces
            .get_or_init(|| {
                // Derive namespaces from the already-cached config so a single
                // write command reads `config.toml` at most once (the prior
                // `get_namespaces()` call re-loaded it from disk).
                match self.cached_config() {
                    Ok(config) => Ok(self.config_manager.namespaces_from_config(config)),
                    Err(err) => Err(format!("{err}")),
                }
            })
            .as_ref()
            .map_err(|err| {
                crate::errors::InvalidArgumentError::new(format!(
                    "invalid namespace configuration: {err}"
                ))
                .into()
            })
    }

    /// Resolve the repo-level default content format from `[validation]
    /// .content_format` (defaulting to Markdown when unset/absent), used as the
    /// fallback for issues that carry no per-issue `content_format` when selecting
    /// the [`ContentParser`](crate::document::ContentParser). A malformed value is
    /// surfaced as an error rather than silently picking the wrong parser.
    fn repo_content_format(&self) -> Result<crate::domain::ContentFormat> {
        match self.cached_config()?.validation.as_ref() {
            Some(validation) => validation.content_format(),
            None => Ok(crate::domain::ContentFormat::Markdown),
        }
    }

    /// Resolve the repository-wide validation strictness from
    /// `[validation].strictness`, defaulting to [`Strictness::Loose`] when the
    /// key (or the whole `[validation]` section) is absent. A malformed value is
    /// surfaced as an error rather than silently defaulting, so a misconfigured
    /// `config.toml` cannot quietly disable or widen enforcement.
    ///
    /// [`Strictness::Loose`]: crate::validation::Strictness::Loose
    fn validation_strictness(&self) -> Result<crate::validation::Strictness> {
        match self.cached_config()?.validation.as_ref() {
            Some(validation) => validation.strictness(),
            None => Ok(crate::validation::Strictness::Loose),
        }
    }

    /// The single write-time validation entry point shared by issue create,
    /// update, and the batch path (DR §7.5).
    ///
    /// `issue` MUST be the FINAL persisted shape — i.e. all field and state
    /// mutations (create's auto-promotion to `Ready`, update's requested state
    /// transition, bulk's projected after-update shape) already applied — so that
    /// rules keyed on the final `state` are evaluated correctly.
    ///
    /// It evaluates the EFFECTIVE local rules (built-in defaults + user
    /// `.jit/rules.toml`) via
    /// [`evaluate_local`](crate::validation::evaluate_local), then applies the
    /// repository's `[validation].strictness`
    /// ([`Strictness`](crate::validation::Strictness)) to the block/allow
    /// decision. Under the default [`Loose`](crate::validation::Strictness::Loose)
    /// level the blocking semantics are:
    ///
    /// - An `error` finding from an `enforce = true` rule REJECTS the write
    ///   unless `force` is set (DR §7.2).
    /// - With `force`, the write is allowed and the bypassed rule names are
    ///   returned in [`WriteValidation::bypassed_rules`] for the CALLER to log
    ///   AFTER the write commits (DR §7.6) — they are NOT logged here, so a
    ///   failed save cannot leave a false bypass entry in the audit log.
    /// - `warn`/non-`enforce` findings never block; their messages are
    ///   returned as warnings.
    ///
    /// [`Strict`](crate::validation::Strictness::Strict) widens the block set to
    /// EVERY violation (any warning or error blocks);
    /// [`Permissive`](crate::validation::Strictness::Permissive) empties it (no
    /// violation blocks — all findings become warnings). Strictness modulates only
    /// this decision; it never changes a rule's severity or `enforce` flag.
    ///
    /// The former hard-coded `IssueValidator` checks are now default rules inside
    /// the effective rule set, so they run through this same path. A genuinely
    /// misconfigured `.jit/rules.toml` or `config.toml` (parse/load error) is
    /// surfaced as an error rather than silently disabling enforcement.
    fn validate_for_write(&self, issue: &Issue, force: bool) -> Result<WriteValidation> {
        let rules = self.effective_rules()?;
        let repo_format = self.repo_content_format()?;
        let strictness = self.validation_strictness()?;
        let evaluation = crate::validation::evaluate_local(issue, rules, repo_format)
            .map_err(|err| anyhow!("rule evaluation failed: {err}"))?
            .with_strictness(strictness);

        let blocking = evaluation.blocking_rules();
        if !blocking.is_empty() && !force {
            // Ordinary rejection: NOT logged (only --force bypasses are). Typed as a
            // validation failure (exit 4) carrying the message verbatim, so the
            // top-level handler classifies it by downcast rather than message text.
            return Err(crate::errors::ValidationFailedError::new(
                evaluation
                    .rejection_message()
                    .unwrap_or_else(|| "blocked by validation rule(s)".to_string()),
            )
            .into());
        }

        let mut warnings = Vec::new();
        warnings.extend(evaluation.warnings());

        // On a forced write `blocking` names the enforce rules being overridden;
        // the caller logs them AFTER the write succeeds. When nothing blocks (or
        // not forced), `blocking` is empty so no events are deferred.
        Ok(WriteValidation {
            warnings,
            bypassed_rules: blocking,
        })
    }

    fn publish_captured_field_update(
        &self,
        request: CapturedFieldUpdate,
    ) -> Result<CapturedFieldUpdateOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, MutationContext};
        use crate::storage::RepositoryStateStoreError;
        use std::collections::BTreeMap;

        let layout = self.require_layout()?;
        let context = MutationContext::production();
        for _ in 0..8 {
            let (expected_target, expected_lease_mode) = {
                let mut preflight = self.storage.open_mutation_session(layout.clone())?;
                let Some(image) =
                    self.capture_proposed_base(preflight.as_mut(), &BTreeMap::new(), &[], None)?
                else {
                    continue;
                };
                let issues = captured_active_issues(&image)?;
                let target = resolve_issue_from_capture(&issues, &request.issue_id)?;
                let config = crate::repository_state::assemble_config(&image)?;
                (
                    target,
                    self.config_manager.enforcement_mode_from_config(&config)?,
                )
            };
            let claims_guard = request
                .enforce_lease
                .then(|| claims_mutation_guard(&layout))
                .transpose()?
                .flatten();
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some(image) =
                self.capture_proposed_base(session.as_mut(), &BTreeMap::new(), &[], None)?
            else {
                continue;
            };
            let issues = captured_active_issues(&image)?;
            if resolve_issue_from_capture(&issues, &request.issue_id)? != expected_target {
                continue;
            }
            let issue = issues
                .iter()
                .find(|issue| issue.id == expected_target)
                .cloned()
                .ok_or_else(|| crate::storage::IssueNotFoundError::new(&expected_target))?;
            let declarations = declarations_from_image(&image)?;
            let config = crate::repository_state::assemble_config(&image)?;
            if request.enforce_lease
                && self.config_manager.enforcement_mode_from_config(&config)? != expected_lease_mode
            {
                continue;
            }
            let lease_warnings = if request.enforce_lease {
                captured_lease_warnings(
                    expected_lease_mode,
                    std::slice::from_ref(&expected_target),
                    &issues,
                    claims_guard.as_ref(),
                )?
            } else {
                Vec::new()
            };
            let plan_content = validate::plan_content_from_image(&image, &issues)?;
            let mut effective_request = request.clone();
            effective_request.issue_id = expected_target;
            let mut derived = derive_field_update(
                issue,
                &effective_request,
                CapturedTransitionEvidence {
                    issues: &issues,
                    declarations: &declarations,
                    config: &config,
                    plan_content: &plan_content,
                    context: &context,
                },
            )?;
            let mut lease_warnings = lease_warnings;
            lease_warnings.append(&mut derived.warnings);
            derived.warnings = lease_warnings;
            if derived.intents.is_empty() {
                return match derived.error_after_apply {
                    Some(error) => Err(error.with_warnings(derived.warnings).into()),
                    None => Ok(CapturedFieldUpdateOutcome {
                        changed: derived.changed,
                        warnings: derived.warnings,
                    }),
                };
            }
            let plan = finalize(&layout, &image, &context, &derived.intents)?;
            match session.apply(&plan) {
                Ok(_) => {
                    return match derived.error_after_apply {
                        Some(error) => Err(error.with_warnings(derived.warnings).into()),
                        None => Ok(CapturedFieldUpdateOutcome {
                            changed: derived.changed,
                            warnings: derived.warnings,
                        }),
                    }
                }
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(anyhow!(
            "issue update did not converge after repeated capture conflicts"
        ))
    }

    fn publish_captured_bulk_update(
        &self,
        issue_id: String,
        operations: &UpdateOperations,
        force: bool,
    ) -> Result<(bool, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let outcome = self.publish_captured_field_update(CapturedFieldUpdate {
            issue_id,
            title: None,
            description: None,
            priority: operations.priority,
            state: operations.state,
            add_labels: operations.add_labels.clone(),
            remove_labels: operations.remove_labels.clone(),
            content_format: None,
            issue_type: None,
            add_gates: operations.add_gates.clone(),
            remove_gates: operations.remove_gates.clone(),
            assignee: operations.assignee.as_deref().map(str::parse).transpose()?,
            unassign: operations.unassign,
            bulk: true,
            force,
            enforce_lease: false,
        })?;
        Ok((outcome.changed, outcome.warnings))
    }

    fn publish_captured_state_transition(
        &self,
        issue_id: &str,
        target: State,
        force: bool,
        divert_unpassed_gates: bool,
        enforce_lease: bool,
    ) -> Result<CapturedLifecycleOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.publish_captured_lifecycle_mutation(CapturedLifecycleMutation::State {
            issue_id: issue_id.to_string(),
            target,
            force,
            divert_unpassed_gates,
            enforce_lease,
        })
    }

    fn publish_captured_claim(
        &self,
        issue_id: String,
        assignee: crate::domain::Assignee,
    ) -> Result<Vec<crate::storage::StorageWarning>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        Ok(self
            .publish_captured_lifecycle_mutation(CapturedLifecycleMutation::Claim {
                issue_id,
                assignee,
            })?
            .storage_warnings)
    }

    fn publish_captured_release(&self, issue_id: String, reason: String) -> Result<()>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.publish_captured_lifecycle_mutation(CapturedLifecycleMutation::Release {
            issue_id,
            reason,
        })?;
        Ok(())
    }

    fn publish_captured_auto_transition(&self, issue_id: String, target: State) -> Result<bool>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let request = match target {
            State::Ready => CapturedLifecycleMutation::AutoReady { issue_id },
            State::Done => CapturedLifecycleMutation::AutoDone { issue_id },
            _ => {
                return Err(anyhow!(
                    "unsupported automatic transition target '{}'",
                    target.as_str()
                ))
            }
        };
        Ok(self.publish_captured_lifecycle_mutation(request)?.changed)
    }

    fn publish_captured_lifecycle_mutation(
        &self,
        request: CapturedLifecycleMutation,
    ) -> Result<CapturedLifecycleOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, MutationContext, MutationIntent};
        use crate::storage::RepositoryStateStoreError;
        use std::collections::BTreeMap;

        let layout = self.require_layout()?;
        let context = MutationContext::production();
        let mut cached_precheck = None::<CachedPrecheckExecution>;
        for _ in 0..8 {
            let (
                expected_target,
                expected_precheck_required,
                expected_precheck,
                precheck_image,
                precheck_issue,
                precheck_registry,
                expected_lease_mode,
                enforce_lease,
            ) = {
                let mut preflight = self.storage.open_mutation_session(layout.clone())?;
                let Some(image) = self.capture_proposed_base(
                    preflight.as_mut(),
                    &BTreeMap::new(),
                    &[],
                    Some(request.issue_id()),
                )?
                else {
                    continue;
                };
                let issues = captured_active_issues(&image)?;
                let target_id = resolve_issue_from_capture(&issues, request.issue_id())?;
                let issue = issues
                    .iter()
                    .find(|issue| issue.id == target_id)
                    .ok_or_else(|| crate::storage::IssueNotFoundError::new(&target_id))?;
                let should_precheck = lifecycle_requires_prechecks(&request, issue);
                let enforce_lease = matches!(
                    &request,
                    CapturedLifecycleMutation::State {
                        enforce_lease: true,
                        ..
                    }
                );
                let config = crate::repository_state::assemble_config(&image)?;
                let registry = declarations_from_image(&image)?.gates;
                let plan = should_precheck
                    .then(|| captured_precheck_plan(&image, issue, &registry))
                    .transpose()?;
                (
                    target_id,
                    should_precheck,
                    plan,
                    image,
                    issue.clone(),
                    registry,
                    self.config_manager.enforcement_mode_from_config(&config)?,
                    enforce_lease,
                )
            };
            let coordinate_claims =
                enforce_lease || matches!(request, CapturedLifecycleMutation::Claim { .. });
            if coordinate_claims {
                let preflight_issues = captured_active_issues(&precheck_image)?;
                let preliminary_guard = claims_mutation_guard(&layout)?;
                if enforce_lease {
                    captured_lease_warnings(
                        expected_lease_mode,
                        std::slice::from_ref(&expected_target),
                        &preflight_issues,
                        preliminary_guard.as_ref(),
                    )?;
                }
                if let CapturedLifecycleMutation::Claim { assignee, .. } = &request {
                    ensure_claim_available(
                        preliminary_guard.as_ref(),
                        &expected_target,
                        assignee,
                        &preflight_issues,
                    )?;
                }
            }
            if let Some(plan) = &expected_precheck {
                let reuse = cached_precheck.as_ref().is_some_and(|cached| {
                    cached.target_id == expected_target && cached.evidence == plan.evidence
                });
                if !reuse {
                    cached_precheck = Some(CachedPrecheckExecution {
                        target_id: expected_target.clone(),
                        evidence: plan.evidence.clone(),
                        execution: self.execute_captured_prechecks(
                            &precheck_image,
                            &precheck_issue,
                            &precheck_registry,
                            &plan.prompts,
                        )?,
                    });
                }
            }
            let precheck_runs = cached_precheck
                .as_ref()
                .filter(|_| expected_precheck.is_some())
                .map(|cached| cached.execution.runs.clone())
                .unwrap_or_default();
            let precheck_has_error = cached_precheck
                .as_ref()
                .filter(|_| expected_precheck.is_some())
                .is_some_and(|cached| cached.execution.error.is_some());
            let precheck_run_paths = (0..precheck_runs.len())
                .flat_map(|index| {
                    let id = context.identifier_at(index as u64);
                    [
                        format!("gate-runs/{id}"),
                        format!("gate-runs/{id}/result.json"),
                    ]
                })
                .map(crate::repository_state::VirtualPath::data)
                .collect::<std::result::Result<Vec<_>, _>>()?;

            let claims_guard = coordinate_claims
                .then(|| claims_mutation_guard(&layout))
                .transpose()?
                .flatten();

            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some(image) = self.capture_proposed_base(
                session.as_mut(),
                &BTreeMap::new(),
                &precheck_run_paths,
                Some(request.issue_id()),
            )?
            else {
                continue;
            };
            let issues = captured_active_issues(&image)?;
            if resolve_issue_from_capture(&issues, request.issue_id())? != expected_target {
                continue;
            }
            let mut issue = issues
                .iter()
                .find(|issue| issue.id == expected_target)
                .cloned()
                .ok_or_else(|| crate::storage::IssueNotFoundError::new(&expected_target))?;
            let config = crate::repository_state::assemble_config(&image)?;
            if enforce_lease
                && self.config_manager.enforcement_mode_from_config(&config)? != expected_lease_mode
            {
                continue;
            }
            let current_precheck_required = lifecycle_requires_prechecks(&request, &issue);
            if current_precheck_required != expected_precheck_required {
                continue;
            }
            if let Some(expected) = &expected_precheck {
                let evidence_matches = if expected.evidence.validation_view.is_some() {
                    expected.evidence.matches(&image)?
                } else {
                    let current_registry = declarations_from_image(&image)?.gates;
                    captured_precheck_plan(&image, &issue, &current_registry)?.evidence
                        == expected.evidence
                };
                if !evidence_matches {
                    continue;
                }
            }
            let mut external_warnings = if enforce_lease {
                captured_lease_warnings(
                    expected_lease_mode,
                    std::slice::from_ref(&expected_target),
                    &issues,
                    claims_guard.as_ref(),
                )?
            } else {
                Vec::new()
            };
            let storage_warnings = claims_guard
                .as_ref()
                .map_or_else(Vec::new, |guard| guard.warnings());
            if let CapturedLifecycleMutation::Claim { assignee, .. } = &request {
                ensure_claim_available(claims_guard.as_ref(), &expected_target, assignee, &issues)?;
            }
            let mut precheck_events = Vec::new();
            if expected_precheck.is_some() {
                for result in &precheck_runs {
                    let (state, by) = gate_check::gate_state_from_run(result)?;
                    let status = state.status;
                    issue.gates_status.insert(result.gate_key.clone(), state);
                    precheck_events.push((
                        1,
                        if status == GateStatus::Passed {
                            Event::draft_gate_passed(issue.id.clone(), result.gate_key.clone(), by)
                        } else {
                            Event::draft_gate_failed(issue.id.clone(), result.gate_key.clone(), by)
                        },
                    ));
                }
            }
            if precheck_has_error {
                if precheck_runs.is_empty() {
                    let error = cached_precheck
                        .take()
                        .and_then(|cached| cached.execution.error)
                        .ok_or_else(|| anyhow!("precheck execution lost its recorded error"))?;
                    return Err(error);
                }
                let intents = std::iter::once(MutationIntent::UpdateIssue {
                    issue: Box::new(issue),
                })
                .chain(
                    precheck_runs
                        .into_iter()
                        .map(|draft| MutationIntent::RecordGateRun {
                            draft: Box::new(draft),
                        }),
                )
                .chain(precheck_events.into_iter().map(|(phase, event)| {
                    MutationIntent::RecordEvent {
                        phase,
                        event: Box::new(event),
                    }
                }))
                .collect::<Vec<_>>();
                let plan = finalize(&layout, &image, &context, &intents)?;
                match session.apply(&plan) {
                    Ok(_) => {
                        let error = cached_precheck
                            .take()
                            .and_then(|cached| cached.execution.error)
                            .ok_or_else(|| anyhow!("precheck execution lost its recorded error"))?;
                        return Err(error);
                    }
                    Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                    Err(apply_error) => return Err(apply_error.into()),
                }
            }
            let prechecks_changed = !precheck_runs.is_empty();
            let (target, force, divert) = match &request {
                CapturedLifecycleMutation::State {
                    target,
                    force,
                    divert_unpassed_gates,
                    ..
                } => (*target, *force, *divert_unpassed_gates),
                CapturedLifecycleMutation::Claim { assignee, .. } => {
                    if let Some(existing) = &issue.assignee {
                        if existing != assignee {
                            return Err(anyhow!(
                                "Issue {} is already assigned to {existing}; refusing to claim as \
                                 {assignee} (re-claiming as {existing} succeeds and promotes it to in_progress)",
                                issue.id,
                            ));
                        }
                        if issue.state == State::InProgress {
                            return Ok(CapturedLifecycleOutcome {
                                target_id: expected_target.clone(),
                                changed: false,
                                warnings: Vec::new(),
                                storage_warnings,
                            });
                        }
                    }
                    if issue.state == State::Backlog {
                        let blockers = issue::blocking_dependencies(
                            &issue,
                            &crate::domain::queries::build_issue_map(&issues),
                        );
                        if !blockers.is_empty() {
                            return Err(crate::errors::TransitionBlockedError::dependencies(
                                issue.id.clone(),
                                State::InProgress,
                                issue.state,
                                blockers,
                            )
                            .into());
                        }
                    }
                    (
                        if issue.state == State::Ready {
                            State::InProgress
                        } else {
                            issue.state
                        },
                        false,
                        false,
                    )
                }
                CapturedLifecycleMutation::Release { .. } => (
                    if issue.state == State::InProgress {
                        State::Ready
                    } else {
                        issue.state
                    },
                    false,
                    false,
                ),
                CapturedLifecycleMutation::AutoReady { .. } => {
                    let resolved = crate::domain::queries::build_issue_map(&issues);
                    if !issue.should_auto_transition_to_ready(&resolved) {
                        return Ok(CapturedLifecycleOutcome {
                            target_id: expected_target.clone(),
                            changed: false,
                            warnings: Vec::new(),
                            storage_warnings,
                        });
                    }
                    (State::Ready, false, false)
                }
                CapturedLifecycleMutation::AutoDone { .. } => {
                    if !issue.should_auto_transition_to_done() {
                        return Ok(CapturedLifecycleOutcome {
                            target_id: expected_target.clone(),
                            changed: false,
                            warnings: Vec::new(),
                            storage_warnings,
                        });
                    }
                    (State::Done, false, false)
                }
            };
            let declarations = declarations_from_image(&image)?;
            let mut after_apply_error = None;
            let effective_target = if divert && target == State::Done && issue.has_unpassed_gates()
            {
                let blockers = issue::blocking_dependencies(
                    &issue,
                    &crate::domain::queries::build_issue_map(&issues),
                );
                if !blockers.is_empty() {
                    return Err(crate::errors::TransitionBlockedError::dependencies(
                        issue.id.clone(),
                        target,
                        issue.state,
                        blockers,
                    )
                    .into());
                }
                after_apply_error = Some(crate::errors::TransitionBlockedError::gates(
                    issue.id.clone(),
                    State::Done,
                    State::Gated,
                    unpassed_gate_blockers(&issue, &declarations.gates),
                ));
                State::Gated
            } else {
                target
            };
            let plan_content = validate::plan_content_from_image(&image, &issues)?;
            let (mut warnings, mut events, mut update, blocked) = if issue.state == effective_target
            {
                (Vec::new(), Vec::new(), None, None)
            } else {
                match derive_state_transition(
                    issue.clone(),
                    effective_target,
                    force,
                    CapturedTransitionEvidence {
                        issues: &issues,
                        declarations: &declarations,
                        config: &config,
                        plan_content: &plan_content,
                        context: &context,
                    },
                )? {
                    DerivedStateTransition::Applied {
                        issue,
                        warnings,
                        events,
                        changed,
                    } => (warnings, events, changed.then_some(*issue), None),
                    DerivedStateTransition::GraphBlocked { error, events } => {
                        (Vec::new(), events, None, Some(error))
                    }
                }
            };
            events.extend(precheck_events);
            if prechecks_changed && update.is_none() {
                update = Some(issue.clone());
            }
            warnings.append(&mut external_warnings);
            if blocked.is_none() {
                let record = update.as_mut().unwrap_or(&mut issue);
                match &request {
                    CapturedLifecycleMutation::Claim { assignee, .. } => {
                        record.assignee = Some(assignee.clone());
                        events.push((
                            4,
                            Event::draft_issue_claimed(record.id.clone(), assignee.clone()),
                        ));
                        if update.is_none() {
                            update = Some(issue);
                        }
                    }
                    CapturedLifecycleMutation::Release { reason, .. } => {
                        if let Some(assignee) = record.assignee.take() {
                            events.push((
                                4,
                                Event::draft_issue_released(
                                    record.id.clone(),
                                    assignee,
                                    reason.clone(),
                                ),
                            ));
                            if update.is_none() {
                                update = Some(issue);
                            }
                        }
                    }
                    _ => {}
                }
            }
            let changed = update.is_some();
            let intents = update
                .into_iter()
                .map(|issue| MutationIntent::UpdateIssue {
                    issue: Box::new(issue),
                })
                .chain(
                    precheck_runs
                        .into_iter()
                        .map(|draft| MutationIntent::RecordGateRun {
                            draft: Box::new(draft),
                        }),
                )
                .chain(
                    events
                        .into_iter()
                        .map(|(phase, event)| MutationIntent::RecordEvent {
                            phase,
                            event: Box::new(event),
                        }),
                )
                .collect::<Vec<_>>();
            if intents.is_empty() {
                return match after_apply_error {
                    Some(error) => Err(error.into()),
                    None => Ok(CapturedLifecycleOutcome {
                        target_id: expected_target.clone(),
                        changed,
                        warnings,
                        storage_warnings,
                    }),
                };
            }
            let plan = finalize(&layout, &image, &context, &intents)?;
            match session.apply(&plan) {
                Ok(_) => {
                    if let Some(error) = blocked.or(after_apply_error) {
                        return Err(error.with_warnings(std::mem::take(&mut warnings)).into());
                    }
                    return Ok(CapturedLifecycleOutcome {
                        target_id: expected_target.clone(),
                        changed,
                        warnings,
                        storage_warnings,
                    });
                }
                Err(RepositoryStateStoreError::RetryableConflict { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(anyhow!(
            "lifecycle mutation did not converge after repeated capture conflicts"
        ))
    }

    /// The dependency and gate guards a transition must clear, evaluated against
    /// `issue` in its CURRENT state.
    ///
    /// Used by the read-only bulk preview path.
    /// entering [`State::Ready`] or [`State::Done`] requires every dependency to
    /// have reached a terminal state, and entering [`State::Done`] additionally
    /// requires every required gate to have passed (`@/inv/gate-semantics`).
    /// Dependencies are checked before gates, so an issue that is both
    /// dependency-blocked and gate-blocked reports its dependencies.
    ///
    /// Every other target (including [`State::Rejected`], which abandons an issue)
    /// is unguarded. `force` has no bearing here: it overrides validation rules,
    /// never the graph's own semantics.
    fn transition_blockers(&self, issue: &Issue, target: State) -> Result<()> {
        if !matches!(target, State::Ready | State::Done) {
            return Ok(());
        }

        let issues = self.storage.list_issues()?;
        let resolved = crate::domain::queries::build_issue_map(&issues);
        let blockers = issue::blocking_dependencies(issue, &resolved);
        if !blockers.is_empty() {
            return Err(crate::errors::TransitionBlockedError::dependencies(
                issue.id.clone(),
                target,
                issue.state,
                blockers,
            )
            .into());
        }

        if target == State::Done && issue.has_unpassed_gates() {
            let registry = self.storage.load_gate_registry()?;
            return Err(crate::errors::TransitionBlockedError::gates(
                issue.id.clone(),
                State::Done,
                issue.state,
                unpassed_gate_blockers(issue, &registry),
            )
            .into());
        }

        Ok(())
    }

    /// Initialize a new jit repository in the current directory.
    ///
    /// Returns the worktree identity if in a git repository (`None` otherwise),
    /// paired with any non-fatal [`StorageWarning`](crate::storage::StorageWarning)s
    /// observed while loading the identity (e.g. a relocation). This method never
    /// writes to stderr; the calling command surfaces the warnings.
    pub fn init(
        &self,
    ) -> Result<(
        Option<WorktreeIdentity>,
        Vec<crate::storage::StorageWarning>,
    )> {
        self.storage.init()?;
        self.initialize_worktree_identity()
    }

    /// Create or refresh machine-local worktree identity after repository
    /// scaffold publication.
    ///
    /// Kept separate from [`Self::init`] so the fresh profiled path can publish
    /// all repository bytes transactionally before performing optional Git host
    /// integration.
    pub fn initialize_worktree_identity(
        &self,
    ) -> Result<(
        Option<WorktreeIdentity>,
        Vec<crate::storage::StorageWarning>,
    )> {
        use crate::storage::worktree_identity::load_or_create_worktree_identity_with_warnings;
        use crate::storage::worktree_paths::WorktreePaths;

        // Check if we're actually in a git repository
        let in_git_repo = std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !in_git_repo {
            return Ok((None, Vec::new()));
        }

        // Create worktree identity
        let paths = WorktreePaths::detect()?;

        // Get git branch name
        let branch = std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(&paths.worktree_root)
            .output()
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    String::from_utf8(output.stdout).ok()
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "main".to_string())
            .trim()
            .to_string();

        // Create or update worktree identity
        // This handles copied files in git worktrees automatically
        let (identity, warnings) = load_or_create_worktree_identity_with_warnings(
            &paths.local_jit,
            &paths.worktree_root,
            &branch,
        )?;

        Ok((Some(identity), warnings))
    }

    /// Scaffold the FIXED default `.jit/rules.toml` (+ `.jit/schemas/*.json`) for a
    /// new repo (DR §8.4). Called from the `jit init` path AFTER `config.toml` has
    /// been written/exists, so the default rules derive from the repo's real
    /// namespace registry + type hierarchy.
    ///
    /// Idempotent: a no-op when `.jit/rules.toml` already exists (the present file
    /// is left untouched, so re-init never clobbers user edits) beyond republishing
    /// its projections (the header comment and `schemas/default-*.json`) and
    /// write-through syncing `namespace-unique-*` MEMBERSHIP
    /// ([`sync_default_rule_membership`](Self::sync_default_rule_membership)).
    /// Returns `true` when it wrote a fresh file, `false` when it was a
    /// projection-and-membership-only refresh.
    pub fn scaffold_default_rules(&self) -> Result<bool> {
        let jit_root = self.storage.root();

        // Hold the repo write lock across the existence check AND the scaffold
        // writes, so two concurrent `jit init` runs cannot both pass the
        // absent-file check and then race on the fixed temp paths for rules.toml
        // and the schema files (MF4: materialize under the write lock).
        let _guard = self.control_plane_write_lock("rules.lock")?;

        let config = self.config_manager.load()?;
        let namespaces = self.config_manager.namespaces_from_config(&config);

        if crate::storage::ruleset_store::has_validation_ruleset(jit_root) {
            // `rules.toml` is never clobbered when present. Its default-origin
            // rules validate against the config registry derived in memory, but the
            // `schemas/default-*.json` files and the file's header comment are
            // projections; republish both from the current registry / contract so a
            // re-init refreshes them. The `namespace-unique-*` MEMBERSHIP is
            // additionally write-through synced into the file itself (not just
            // projected) so `@/rule/<name>` addressability tracks the registry too.
            // The header rewrite preserves every rule body (including custom-rule
            // comments). Idempotent and atomic; a no-op when already current.
            self.refresh_default_schema_projections()?;
            self.sync_default_rule_membership()?;
            crate::storage::ruleset_store::rewrite_rules_header(
                jit_root,
                crate::repository_state::rules_file_header(),
            )?;
            return Ok(false);
        }

        // Validation produces the content; storage performs the atomic writes.
        let serialized = crate::repository_state::serialize_ruleset(
            &crate::repository_state::default_ruleset(&namespaces),
        );
        let schema_files: Vec<(String, String)> = serialized
            .schema_files
            .into_iter()
            .map(|file| (file.name, file.content))
            .collect();
        crate::storage::ruleset_store::write_validation_ruleset(
            jit_root,
            &serialized.rules_toml,
            &schema_files,
        )?;
        Ok(true)
    }

    /// Refresh every default-origin `schemas/default-*.json` projection from the
    /// current `[namespaces]` / `[type_hierarchy]` config, returning the names of
    /// the files (re)written.
    ///
    /// The default rules validate against the registry reconciled in memory at
    /// load
    /// ([`reconcile_default_rules_with_config`](crate::repository_state::reconcile_default_rules_with_config)),
    /// so these files are write-through projections for external consumers, never
    /// the validation authority. jit republishes them whenever it writes
    /// `config.toml` or `rules.toml` (init/re-init, `config set`) so a projection
    /// a tool reads stays current after a jit-driven registry change.
    ///
    /// Idempotent (rewriting current content yields identical bytes) and atomic
    /// (temp + rename per file). A no-op for a repo with no materialized `schemas/`
    /// layout — the read path builds the default schemas in memory — so it is safe
    /// to call unconditionally.
    pub fn refresh_default_schema_projections(&self) -> Result<Vec<String>> {
        let jit_root = self.storage.root();
        let config = self.config_manager.load()?;
        let namespaces = self.config_manager.namespaces_from_config(&config);
        // Validation builds the schema content (the SAME files `jit init`
        // scaffolds); storage performs the atomic per-file write.
        let serialized = crate::repository_state::serialize_ruleset(
            &crate::repository_state::default_ruleset(&namespaces),
        );
        let mut written = Vec::new();
        for file in serialized.schema_files {
            if crate::storage::ruleset_store::write_baked_schema(
                jit_root,
                &file.name,
                &file.content,
            )? {
                written.push(file.name);
            }
        }
        Ok(written)
    }

    /// Write-through the `namespace-unique-*` DEFAULT-rule file MEMBERSHIP into
    /// `.jit/rules.toml` itself (not just its `schemas/*.json` projections), so
    /// the registry-first `rule` item kind — which resolves `@/rule/<name>`
    /// straight from the file, not the in-memory-reconciled ruleset (`jit item
    /// show`/`list`, docs-mechanical citation checking) — never dangles behind
    /// [`reconcile_default_rules_with_config`](crate::repository_state::reconcile_default_rules_with_config)'s
    /// load-time-only reconciliation.
    ///
    /// Computes [`default_rule_membership_diff`](crate::repository_state::default_rule_membership_diff)
    /// between the CURRENT on-disk `rules.toml` and the CURRENT `[namespaces]`
    /// registry, then appends the row for each newly-unique namespace and drops
    /// the row for each namespace no longer unique or no longer declared —
    /// `origin = "default"` rows ONLY. All other content (custom rules,
    /// hand-edited policy fields on surviving default rules, comments,
    /// formatting) survives: an append-only sync preserves it byte-exact, while
    /// a sync that drops a row re-serializes the document and may canonicalize
    /// exotic-but-valid TOML syntax spellings elsewhere in the file —
    /// semantically lossless (REQ-01 as amended, jit:d74a9ed1).
    ///
    /// Called from the same jit-driven-write triggers as
    /// [`refresh_default_schema_projections`](Self::refresh_default_schema_projections)
    /// (init/re-init, `config set`), so a registry edit that changes derived
    /// membership propagates to the file on the next jit write, not only in
    /// memory. A no-op when `rules.toml` is absent or the diff is empty.
    ///
    /// In-memory reconciliation stays the validation authority (out of scope
    /// for this write-through) — this exists only so the file cannot lag it
    /// for addressability.
    pub fn sync_default_rule_membership(&self) -> Result<RuleMembershipSync> {
        let jit_root = self.storage.root();
        if !jit_root.join("rules.toml").exists() {
            return Ok(RuleMembershipSync::default());
        }

        // Identity-only read (never full RuleSet validation): a custom rule
        // whose assertion fails to load must not strand this sync after
        // config.toml was already saved (jit:d74a9ed1 review F1).
        let identities = crate::storage::ruleset_store::read_rule_identities(jit_root)?;
        let config = self.config_manager.load()?;
        let namespaces = self.config_manager.namespaces_from_config(&config);
        let diff = crate::repository_state::default_rule_membership_diff_from_identities(
            &identities,
            &namespaces,
        );
        if diff.is_empty() {
            return Ok(RuleMembershipSync::default());
        }

        let to_add_blocks: Vec<String> = diff
            .to_add
            .iter()
            .map(crate::repository_state::render_rule_block)
            .collect();
        crate::storage::ruleset_store::sync_namespace_unique_rules(
            jit_root,
            &to_add_blocks,
            &diff.to_drop,
        )?;

        Ok(RuleMembershipSync {
            added: diff.to_add.into_iter().map(|r| r.name).collect(),
            dropped: diff.to_drop,
        })
    }

    /// Acquire the control-plane lock named `lock_file`, held until the returned
    /// guard drops. Returns `None` when this storage root's working tree is not a
    /// git repository of its own: it then has no control plane, so the caller
    /// proceeds lockless.
    ///
    /// The lock lives in the repo's git control plane (`.git/jit/locks/`), the
    /// same plane claims coordination uses, derived from the repository that OWNS
    /// this storage root rather than the process's ambient cwd (see
    /// [`repo_control_plane_dir`](Self::repo_control_plane_dir)).
    ///
    /// It guards control-plane state, NOT the issue store: no `IssueStore` write
    /// path takes it, and it is absent outside git. A sequence that mutates issues,
    /// gates, or events must instead hold
    /// [`IssueStore::acquire_repo_write_lock`](crate::storage::IssueStore::acquire_repo_write_lock),
    /// the lock every ordinary writer takes, which lives in the storage root and
    /// exists with or without git (`@/charter/D-4`).
    fn control_plane_write_lock(
        &self,
        lock_file: &str,
    ) -> Result<Option<crate::storage::lock::LockGuard>> {
        use crate::storage::FileLocker;
        use std::time::Duration;

        let Some(control_plane) = self.repo_control_plane_dir() else {
            return Ok(None);
        };
        let locks_dir = control_plane.join("locks");
        std::fs::create_dir_all(&locks_dir)
            .context("Failed to create control-plane locks directory")?;
        FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ))
        .lock_exclusive(&locks_dir.join(lock_file))
        .map(Some)
    }

    /// Resolve the git control-plane dir (`<git-common-dir>/jit`) for the
    /// repository that OWNS this storage root's working tree, or `None` when that
    /// working tree is not a git repository of its own.
    ///
    /// Determined from the storage root's parent (the working dir) rather than the
    /// process cwd, so a `.jit` nested under an unrelated ancestor repo is treated
    /// as non-git instead of borrowing the ancestor's control plane. Returns the
    /// shared common dir for linked worktrees (where `.git` is a gitdir pointer
    /// file) so siblings serialize on the same lock.
    fn repo_control_plane_dir(&self) -> Option<std::path::PathBuf> {
        let work_dir = self.storage.root().parent()?;
        let dot_git = work_dir.join(".git");
        if dot_git.is_dir() {
            // Main worktree: the common dir IS `<work_dir>/.git`.
            Some(dot_git.join("jit"))
        } else if dot_git.is_file() {
            // Linked worktree: `.git` is a gitdir pointer; ask git (with cwd at
            // this working tree, so it resolves THIS repo) for the shared common
            // dir that all linked worktrees share.
            let out = std::process::Command::new("git")
                .args(["rev-parse", "--git-common-dir"])
                .current_dir(work_dir)
                .output()
                .ok()?;
            if !out.status.success() {
                return None;
            }
            let raw = String::from_utf8(out.stdout).ok()?;
            let common = std::path::PathBuf::from(raw.trim());
            let common = if common.is_absolute() {
                common
            } else {
                work_dir.join(common)
            };
            Some(common.join("jit"))
        } else {
            None
        }
    }

    /// Check if an active lease exists for the given issue by the current agent.
    ///
    /// Returns true if the current agent has an active lease (not expired or stale).
    /// Returns false if no lease exists, lease is stale, or belongs to another agent.
    ///
    /// In tests: If JIT_AGENT_ID is not set, any valid lease counts (single-user mode).
    fn check_active_lease(&self, issue_id: &str) -> Result<bool> {
        use crate::agent_config::resolve_agent_id;
        use crate::storage::claim_coordinator::ClaimsIndex;
        use crate::storage::worktree_paths::WorktreePaths;

        // Get worktree paths to access shared control plane
        let paths = match WorktreePaths::detect() {
            Ok(p) => p,
            Err(_) => {
                // Not in a git repository - no claims possible
                return Ok(false);
            }
        };

        // Load the active-lease index through storage (an absent index yields an
        // empty one, i.e. no active leases).
        let claims_index = ClaimsIndex::load(&paths)?;

        // Resolve current agent identity (or None for single-user mode)
        let current_agent = resolve_agent_id(None).ok();

        // Check if active lease exists for this issue
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let now = chrono::Utc::now();

        let has_active_lease = claims_index.leases.iter().any(|lease| {
            // Must match issue ID
            if lease.issue_id != full_id {
                return false;
            }

            // Must not be expired
            if let Some(expires) = lease.expires_at {
                if expires <= now {
                    return false;
                }
            }

            // Must not be stale
            if claims_index.is_stale(lease) {
                return false;
            }

            // Agent verification:
            // - If current_agent is Some, lease must belong to this agent
            // - If current_agent is None (single-user mode), any valid lease counts
            match &current_agent {
                Some(agent_id) => lease.agent_id == *agent_id,
                None => true, // Single-user mode: any valid lease
            }
        });

        Ok(has_active_lease)
    }

    /// Require an active lease for the given issue, respecting enforcement mode.
    ///
    /// Checks the configured enforcement mode and either blocks, warns, or bypasses
    /// the lease requirement. Used before structural operations that modify issues.
    ///
    /// # Errors
    ///
    /// Returns an error in `strict` mode when no active lease exists.
    /// In `warn` mode, returns Ok with a warning message.
    /// In `off` mode, always returns Ok with None.
    ///
    /// Returns: Result<Option<String>> where Some(warning) should be printed by the CLI layer.
    pub fn require_active_lease(&self, issue_id: &str) -> Result<Option<String>> {
        use crate::config::EnforcementMode;

        // Derive the mode from the cached config so a write command that also runs
        // `validate_for_write` parses `config.toml` at most once (DR §6.1).
        let mode = self
            .config_manager
            .enforcement_mode_from_config(self.cached_config()?)?;

        match mode {
            EnforcementMode::Off => Ok(None),
            EnforcementMode::Warn | EnforcementMode::Strict => {
                let has_lease = self.check_active_lease(issue_id)?;

                if !has_lease {
                    let msg = format!(
                        "No active lease for issue {}.\nAcquire lease with: jit claim acquire {}",
                        issue_id, issue_id
                    );

                    match mode {
                        EnforcementMode::Warn => Ok(Some(msg)),
                        EnforcementMode::Strict => {
                            anyhow::bail!("{}", msg)
                        }
                        _ => unreachable!(),
                    }
                } else {
                    Ok(None)
                }
            }
        }
    }
}

// Helper functions for parsing command-line arguments
#[cfg(test)]
mod tests {
    use super::*;

    fn precheck_evidence_fixture() -> (
        crate::storage::InMemoryStorage,
        CommandExecutor<crate::storage::InMemoryStorage>,
        String,
    ) {
        use crate::declarations::{GateChecker, GateDefinition, GateRegistry, GateStage};
        use crate::storage::IssueStore;
        use std::collections::HashMap;

        let storage = crate::storage::InMemoryStorage::new();
        storage.init().unwrap();
        storage.add_repo_file(".jit/config.toml", "");
        storage.add_repo_file("start-prompt.md", "captured prompt");
        let mut registry = GateRegistry::default();
        registry.gates.insert(
            "auto-start".to_string(),
            GateDefinition {
                version: 1,
                key: "auto-start".to_string(),
                title: "Auto start".to_string(),
                description: String::new(),
                stage: GateStage::Precheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "true".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: None,
                    prompt_file: Some("start-prompt.md".to_string()),
                }),
                priority: 1,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
            },
        );
        registry.gates.insert(
            "manual-start".to_string(),
            GateDefinition {
                version: 1,
                key: "manual-start".to_string(),
                title: "Manual start".to_string(),
                description: String::new(),
                stage: GateStage::Precheck,
                mode: GateMode::Manual,
                checker: None,
                priority: 2,
                reserved: HashMap::new(),
                auto: false,
                example_integration: None,
            },
        );
        storage.save_gate_registry(&registry).unwrap();
        let mut issue = crate::domain::types::fixture_issue("checked".to_string(), String::new());
        issue.state = State::Ready;
        issue.gates_required = vec!["auto-start".to_string(), "manual-start".to_string()];
        issue.gates_status.insert(
            "manual-start".to_string(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("human:reviewer".parse().unwrap()),
                updated_at: chrono::Utc::now(),
            },
        );
        let issue_id = issue.id.clone();
        storage.save_issue(issue).unwrap();
        let executor =
            CommandExecutor::new(storage.clone()).with_layout(storage.repository_layout());
        (storage, executor, issue_id)
    }

    fn capture_precheck_plan(
        executor: &CommandExecutor<crate::storage::InMemoryStorage>,
        issue_id: &str,
    ) -> CapturedPrecheckPlan {
        use crate::storage::RepositoryStateStore;
        let mut session = executor
            .storage
            .open_mutation_session(executor.require_layout().unwrap())
            .unwrap();
        let image = executor
            .capture_proposed_base(
                session.as_mut(),
                &std::collections::BTreeMap::new(),
                &[],
                Some(issue_id),
            )
            .unwrap()
            .unwrap();
        let issue = captured_active_issues(&image)
            .unwrap()
            .into_iter()
            .find(|issue| issue.id == issue_id)
            .unwrap();
        let registry = declarations_from_image(&image).unwrap().gates;
        captured_precheck_plan(&image, &issue, &registry).unwrap()
    }

    fn prior_precheck_run(issue_id: &str) -> crate::domain::GateRunResult {
        crate::domain::GateRunResult {
            schema_version: crate::domain::GATE_RUN_SCHEMA_VERSION,
            run_id: "prior-run".to_string(),
            gate_key: "auto-start".to_string(),
            stage: crate::declarations::GateStage::Precheck,
            issue_id: issue_id.to_string(),
            commit: None,
            branch: None,
            tree_dirty: None,
            status: crate::domain::GateRunStatus::Passed,
            started_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
            duration_ms: Some(1),
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            command: "true".to_string(),
            by: None,
            message: None,
            findings: None,
        }
    }

    fn captured_issue_spec(id: &str) -> crate::repository_state::CaptureSpec {
        use crate::repository_state::{CaptureBudget, CaptureSpec, VirtualPath};
        let paths = std::collections::BTreeSet::from([
            VirtualPath::data(format!("issues/{id}.json")).unwrap(),
            VirtualPath::data("events.jsonl").unwrap(),
        ]);
        CaptureSpec::phase_one(
            paths,
            CaptureBudget {
                max_paths: 2,
                max_listings: 0,
                max_bytes: 1024 * 1024,
                max_depth: 4,
            },
        )
        .unwrap()
    }

    fn captured_gate_issue_spec(id: &str) -> crate::repository_state::CaptureSpec {
        use crate::repository_state::{CaptureBudget, CaptureSpec, VirtualPath};
        let paths = std::collections::BTreeSet::from([
            VirtualPath::data(format!("issues/{id}.json")).unwrap(),
            VirtualPath::data("events.jsonl").unwrap(),
            VirtualPath::data("gates.toml").unwrap(),
        ]);
        CaptureSpec::phase_one(
            paths,
            CaptureBudget {
                max_paths: 3,
                max_listings: 0,
                max_bytes: 1024 * 1024,
                max_depth: 4,
            },
        )
        .unwrap()
    }

    fn issue_from_image(image: &crate::repository_state::RepositoryImage, id: &str) -> Issue {
        let path = crate::repository_state::VirtualPath::data(format!("issues/{id}.json")).unwrap();
        serde_json::from_slice(image.file_bytes(&path).unwrap().unwrap()).unwrap()
    }

    fn event_bytes(plan: &crate::repository_state::MaterializationPlan) -> Vec<u8> {
        plan.delta()
            .actions()
            .iter()
            .find_map(|action| match action {
                crate::repository_state::RepositoryAction::WriteFile { path, bytes, .. }
                    if path
                        == &crate::repository_state::VirtualPath::data("events.jsonl").unwrap() =>
                {
                    Some(bytes.clone())
                }
                _ => None,
            })
            .unwrap()
    }

    fn manual_gate_registry(key: &str) -> crate::declarations::GateRegistry {
        use crate::declarations::{GateDefinition, GateMode, GateRegistry, GateStage};
        let definition = GateDefinition {
            version: 1,
            key: key.to_string(),
            title: key.to_string(),
            description: String::new(),
            stage: GateStage::Postcheck,
            mode: GateMode::Manual,
            checker: None,
            priority: 100,
            reserved: std::collections::HashMap::new(),
            auto: false,
            example_integration: None,
        };
        GateRegistry {
            gates: std::collections::HashMap::from([(key.to_string(), definition)]),
        }
    }

    fn gate_registry_from_image(
        image: &crate::repository_state::RepositoryImage,
    ) -> crate::declarations::GateRegistry {
        let path = crate::repository_state::VirtualPath::data("gates.toml").unwrap();
        crate::declarations::parse_gate_registry(image.file_bytes(&path).unwrap().unwrap()).unwrap()
    }

    #[test]
    fn test_captured_issue_retry_preserves_unrelated_change_and_context_identity() {
        use crate::repository_state::{finalize, MutationContext};
        use crate::storage::{InMemoryStorage, RepositoryStateStore, RepositoryStateStoreError};

        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        let issue = crate::domain::types::fixture_issue(
            "captured-retry-issue".to_string(),
            "Captured retry".to_string(),
        );
        let id = issue.id.clone();
        storage.save_issue(issue).unwrap();
        let layout = storage.repository_layout();
        let request = CapturedIssueMutation::Assign {
            issue_id: id.clone(),
            assignee: "agent:retry".parse().unwrap(),
        };
        let context = MutationContext::deterministic(
            [7; 32],
            chrono::DateTime::parse_from_rfc3339("2026-07-20T10:11:12Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );

        let mut first_session = storage.open_mutation_session(layout.clone()).unwrap();
        let first_image = first_session.capture(captured_issue_spec(&id)).unwrap();
        let first = derive_captured_issue_mutation(
            issue_from_image(&first_image, &id),
            &crate::declarations::GateRegistry::default(),
            &request,
        )
        .unwrap();
        let first_plan = finalize(&layout, &first_image, &context, &first.intents).unwrap();

        // Model an unrelated writer between capture and apply. The stale plan
        // conflicts instead of replacing that writer's label.
        let mut concurrent = storage.load_issue(&id).unwrap();
        concurrent.labels.push("owner:concurrent".to_string());
        storage.save_issue(concurrent).unwrap();
        assert!(matches!(
            first_session.apply(&first_plan),
            Err(RepositoryStateStoreError::RetryableConflict { .. })
        ));
        drop(first_session);

        let mut retry_session = storage.open_mutation_session(layout.clone()).unwrap();
        let retry_image = retry_session.capture(captured_issue_spec(&id)).unwrap();
        let retry = derive_captured_issue_mutation(
            issue_from_image(&retry_image, &id),
            &crate::declarations::GateRegistry::default(),
            &request,
        )
        .unwrap();
        let retry_plan = finalize(&layout, &retry_image, &context, &retry.intents).unwrap();
        assert_eq!(
            event_bytes(&first_plan),
            event_bytes(&retry_plan),
            "one operation context must keep event identity and time stable"
        );
        retry_session.apply(&retry_plan).unwrap();

        let updated = storage.load_issue(&id).unwrap();
        assert!(updated
            .labels
            .iter()
            .any(|label| label == "owner:concurrent"));
        assert_eq!(updated.assignee.unwrap().to_string(), "agent:retry");
    }

    #[test]
    fn test_captured_issue_idempotent_retry_derives_true_noop() {
        let mut issue = crate::domain::types::fixture_issue(
            "captured-noop-issue".to_string(),
            "Captured no-op".to_string(),
        );
        issue.assignee = Some("agent:same".parse().unwrap());
        let request = CapturedIssueMutation::Assign {
            issue_id: issue.id.clone(),
            assignee: "agent:same".parse().unwrap(),
        };
        let derived = derive_captured_issue_mutation(
            issue,
            &crate::declarations::GateRegistry::default(),
            &request,
        )
        .unwrap();
        assert!(
            derived.intents.is_empty(),
            "rebased no-op must write nothing"
        );
        assert!(matches!(
            derived.outcome,
            CapturedIssueMutationOutcome::Unchanged
        ));
    }

    #[test]
    fn test_manual_gate_evidence_repeats_but_operation_retry_is_stable() {
        use crate::repository_state::{finalize, MutationContext};
        use crate::storage::{InMemoryStorage, RepositoryStateStore, RepositoryStateStoreError};

        for status in [GateStatus::Passed, GateStatus::Failed] {
            let storage = InMemoryStorage::new();
            storage.init().unwrap();
            storage
                .save_gate_registry(&manual_gate_registry("review"))
                .unwrap();
            let mut issue = crate::domain::types::fixture_issue(
                format!("repeat-{status:?}"),
                "Repeated evidence".to_string(),
            );
            let id = issue.id.clone();
            let actor: crate::domain::Assignee = "agent:reviewer".parse().unwrap();
            issue.gates_required.push("review".to_string());
            issue.gates_status.insert(
                "review".to_string(),
                GateState {
                    status,
                    updated_by: Some(actor.clone()),
                    updated_at: chrono::DateTime::UNIX_EPOCH,
                },
            );
            storage.save_issue(issue).unwrap();
            let layout = storage.repository_layout();
            let request = CapturedIssueMutation::SetManualGateStatus {
                issue_id: id.clone(),
                gate_key: "review".to_string(),
                status,
                by: Some(actor.clone()),
            };
            let first_time = chrono::DateTime::parse_from_rfc3339("2026-07-20T10:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc);
            let first_context = MutationContext::deterministic([31; 32], first_time);
            let mut first_session = storage.open_mutation_session(layout.clone()).unwrap();
            let first_image = first_session
                .capture(captured_gate_issue_spec(&id))
                .unwrap();
            let first = derive_captured_issue_mutation(
                issue_from_image(&first_image, &id),
                &gate_registry_from_image(&first_image),
                &request,
            )
            .unwrap();
            let first_plan =
                finalize(&layout, &first_image, &first_context, &first.intents).unwrap();

            // Force a genuine captured-image retry. The operation context must
            // retain its event identity and timestamp when the issue preimage
            // changes between attempts.
            let mut concurrent = storage.load_issue(&id).unwrap();
            concurrent.labels.push("owner:concurrent".to_string());
            storage.save_issue(concurrent).unwrap();
            assert!(matches!(
                first_session.apply(&first_plan),
                Err(RepositoryStateStoreError::RetryableConflict { .. })
            ));
            drop(first_session);

            let mut retry_session = storage.open_mutation_session(layout.clone()).unwrap();
            let retry_image = retry_session
                .capture(captured_gate_issue_spec(&id))
                .unwrap();
            let retry = derive_captured_issue_mutation(
                issue_from_image(&retry_image, &id),
                &gate_registry_from_image(&retry_image),
                &request,
            )
            .unwrap();
            let retry_plan =
                finalize(&layout, &retry_image, &first_context, &retry.intents).unwrap();
            assert_eq!(
                event_bytes(&first_plan),
                event_bytes(&retry_plan),
                "one operation context must keep manual evidence identity and time stable"
            );
            retry_session.apply(&retry_plan).unwrap();
            assert_eq!(
                storage.load_issue(&id).unwrap().gates_status["review"].updated_at,
                first_time
            );

            let second_time = chrono::DateTime::parse_from_rfc3339("2026-07-20T11:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc);
            let second_context = MutationContext::deterministic([32; 32], second_time);
            let mut second_session = storage.open_mutation_session(layout.clone()).unwrap();
            let second_image = second_session
                .capture(captured_gate_issue_spec(&id))
                .unwrap();
            let second = derive_captured_issue_mutation(
                issue_from_image(&second_image, &id),
                &gate_registry_from_image(&second_image),
                &request,
            )
            .unwrap();
            let second_plan =
                finalize(&layout, &second_image, &second_context, &second.intents).unwrap();
            assert_ne!(event_bytes(&first_plan), event_bytes(&second_plan));
            second_session.apply(&second_plan).unwrap();

            assert_eq!(
                storage.load_issue(&id).unwrap().gates_status["review"].updated_at,
                second_time
            );
            let events = storage.read_events().unwrap();
            assert_eq!(events.len(), 2);
            let evidence = events
                .iter()
                .map(|event| match (status, event) {
                    (
                        GateStatus::Passed,
                        Event::GatePassed {
                            id,
                            timestamp,
                            updated_by,
                            ..
                        },
                    )
                    | (
                        GateStatus::Failed,
                        Event::GateFailed {
                            id,
                            timestamp,
                            updated_by,
                            ..
                        },
                    ) if updated_by.as_ref() == Some(&actor) => (id, timestamp),
                    _ => panic!("unexpected manual gate evidence: {event:?}"),
                })
                .collect::<Vec<_>>();
            assert_eq!(*evidence[0].1, first_time);
            assert_eq!(*evidence[1].1, second_time);
            assert_ne!(evidence[0].0, evidence[1].0);
        }
    }

    #[test]
    fn test_manual_gate_pending_status_is_rejected_before_other_validation() {
        let issue = crate::domain::types::fixture_issue(
            "pending-precedence".to_string(),
            "Pending precedence".to_string(),
        );
        let request = CapturedIssueMutation::SetManualGateStatus {
            issue_id: issue.id.clone(),
            gate_key: "missing-and-not-required".to_string(),
            status: GateStatus::Pending,
            by: None,
        };

        let error = match derive_captured_issue_mutation(
            issue,
            &crate::declarations::GateRegistry::default(),
            &request,
        ) {
            Ok(_) => panic!("pending status unexpectedly accepted"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "manual gate status must be passed or failed"
        );
    }

    #[test]
    fn test_gate_add_retry_rejects_removed_registry_declaration() {
        use crate::repository_state::{finalize, MutationContext};
        use crate::storage::{
            GateNotFoundError, InMemoryStorage, RepositoryStateStore, RepositoryStateStoreError,
        };

        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        storage
            .save_gate_registry(&manual_gate_registry("review"))
            .unwrap();
        let issue = crate::domain::types::fixture_issue(
            "registry-race".to_string(),
            "Registry race".to_string(),
        );
        let id = issue.id.clone();
        storage.save_issue(issue).unwrap();
        let layout = storage.repository_layout();
        let request = CapturedIssueMutation::AddGates {
            issue_id: id.clone(),
            gate_keys: vec!["review".to_string()],
        };
        let context = MutationContext::deterministic(
            [41; 32],
            chrono::DateTime::parse_from_rfc3339("2026-07-20T12:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );

        let mut first_session = storage.open_mutation_session(layout.clone()).unwrap();
        let first_image = first_session
            .capture(captured_gate_issue_spec(&id))
            .unwrap();
        let first = derive_captured_issue_mutation(
            issue_from_image(&first_image, &id),
            &gate_registry_from_image(&first_image),
            &request,
        )
        .unwrap();
        let stale_plan = finalize(&layout, &first_image, &context, &first.intents).unwrap();

        storage
            .save_gate_registry(&crate::declarations::GateRegistry::default())
            .unwrap();
        assert!(matches!(
            first_session.apply(&stale_plan),
            Err(RepositoryStateStoreError::RetryableConflict { .. })
        ));
        drop(first_session);

        let mut retry_session = storage.open_mutation_session(layout).unwrap();
        let retry_image = retry_session
            .capture(captured_gate_issue_spec(&id))
            .unwrap();
        let error = match derive_captured_issue_mutation(
            issue_from_image(&retry_image, &id),
            &gate_registry_from_image(&retry_image),
            &request,
        ) {
            Ok(_) => panic!("retry must revalidate against the changed registry"),
            Err(error) => error,
        };
        assert_eq!(
            error.downcast_ref::<GateNotFoundError>(),
            Some(&GateNotFoundError::Batch(vec!["review".to_string()]))
        );
        assert!(storage.load_issue(&id).unwrap().gates_required.is_empty());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_rules_are_parsed_once_and_cached() {
        use crate::declarations::rules::Assertion;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();

        let rules_path = storage.root().join("rules.toml");
        std::fs::write(
            &rules_path,
            r#"
[[rules]]
name = "first"
assert = { require-section = { heading = "Goals" } }
"#,
        )
        .unwrap();

        let executor = CommandExecutor::new(storage);

        // First access parses and caches the file.
        let first = executor.rules().expect("valid rules");
        assert_eq!(first.rules.len(), 1);
        assert_eq!(first.rules[0].name, "first");
        let ptr_first = std::ptr::from_ref(first);

        // Mutate the file on disk AFTER the first parse.
        std::fs::write(
            &rules_path,
            r#"
[[rules]]
name = "second"
assert = { require-section = { heading = "Other" } }

[[rules]]
name = "third"
assert = { require-doc-type = { doc-type = "design" } }
"#,
        )
        .unwrap();

        // Second access must return the cached (original) parse, NOT re-read.
        let second = executor.rules().expect("valid rules");
        assert_eq!(second.rules.len(), 1, "ruleset was re-read from disk");
        assert_eq!(second.rules[0].name, "first");
        assert!(matches!(
            second.rules[0].assert,
            Assertion::RequireSection { .. }
        ));
        // Same cached instance is returned each time.
        assert_eq!(ptr_first, std::ptr::from_ref(second));
    }

    #[test]
    fn test_namespaces_are_parsed_once_and_cached() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();

        let config_path = storage.root().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
[namespaces.req]
description = "Requirement tags"
unique = false
"#,
        )
        .unwrap();

        let executor = CommandExecutor::new(storage);

        // First access parses config.toml and caches the namespace registry.
        let first = executor.cached_namespaces().expect("valid namespaces");
        let ptr_first = std::ptr::from_ref(first);

        // Mutate config.toml on disk AFTER the first parse. A re-read would pick
        // up the new namespace; a cached registry must not.
        std::fs::write(
            &config_path,
            r#"
[namespaces.req]
description = "Requirement tags"
unique = false

[namespaces.owner]
description = "Ownership tags"
unique = false
"#,
        )
        .unwrap();

        // Second access must return the SAME cached instance, proving the
        // namespace config is parsed once per executor and not re-read per write.
        let second = executor.cached_namespaces().expect("valid namespaces");
        assert_eq!(
            ptr_first,
            std::ptr::from_ref(second),
            "namespace registry was re-read from disk instead of cached"
        );
    }

    #[test]
    fn test_rules_missing_file_yields_empty_ok() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();
        // No rules.toml written.

        let executor = CommandExecutor::new(storage);
        let rules = executor.rules().expect("missing file is not an error");
        assert!(rules.rules.is_empty());
    }

    #[test]
    fn test_rules_malformed_file_yields_err() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();

        // A rule whose assert table has no kind is a genuine config error and
        // must NOT be downgraded to an empty (no-rules) set.
        std::fs::write(
            storage.root().join("rules.toml"),
            r#"
[[rules]]
name = "broken"
assert = {}
"#,
        )
        .unwrap();

        let executor = CommandExecutor::new(storage);
        assert!(
            executor.rules().is_err(),
            "malformed rules.toml must surface an error, not an empty set"
        );
    }

    // Enforcement tests
    #[test]
    fn test_require_active_lease_off_mode() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        // Create a test issue
        let issue =
            crate::domain::types::fixture_issue("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(issue).unwrap();

        // Create the root directory and config with enforcement off
        std::fs::create_dir_all(storage.root()).unwrap();
        let config_toml = r#"
[worktree]
enforce_leases = "off"
"#;
        std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

        let executor = CommandExecutor::new(storage);

        // Should always succeed in off mode, even without lease, and return None
        let result = executor.require_active_lease(&issue_id);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn test_check_active_lease_no_claims_index() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        // Create a test issue
        let issue =
            crate::domain::types::fixture_issue("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(issue).unwrap();

        let executor = CommandExecutor::new(storage);

        // No claims index - should return false
        let result = executor.check_active_lease(&issue_id);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_require_active_lease_strict_mode_no_lease() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        // Create a test issue
        let issue =
            crate::domain::types::fixture_issue("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(issue).unwrap();

        // Create the root directory and config with enforcement strict
        std::fs::create_dir_all(storage.root()).unwrap();
        let config_toml = r#"
[worktree]
enforce_leases = "strict"
"#;
        std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

        let executor = CommandExecutor::new(storage);

        // Should fail in strict mode without lease
        let result = executor.require_active_lease(&issue_id);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("No active lease"));
        assert!(err_msg.contains("jit claim acquire"));
    }

    #[test]
    fn test_require_active_lease_off_mode_default() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        // Create a test issue
        let issue =
            crate::domain::types::fixture_issue("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(issue).unwrap();

        // No config file - should default to off mode (single-agent development)
        let executor = CommandExecutor::new(storage);

        // Should succeed in off mode (default) without lease
        let result = executor.require_active_lease(&issue_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_precheck_evidence_detects_manual_status_mutation() {
        use crate::storage::IssueStore;
        let (storage, executor, issue_id) = precheck_evidence_fixture();
        let before = capture_precheck_plan(&executor, &issue_id).evidence;

        let mut issue = storage.load_issue(&issue_id).unwrap();
        issue.gates_status.get_mut("manual-start").unwrap().status = GateStatus::Failed;
        storage.save_issue(issue).unwrap();

        assert_ne!(capture_precheck_plan(&executor, &issue_id).evidence, before);
    }

    #[test]
    fn test_precheck_evidence_detects_run_history_mutation() {
        let (storage, executor, issue_id) = precheck_evidence_fixture();
        let before = capture_precheck_plan(&executor, &issue_id).evidence;
        let run = prior_precheck_run(&issue_id);
        storage.add_repo_file(
            ".jit/gate-runs/prior-run/result.json",
            &serde_json::to_string(&run).unwrap(),
        );

        assert_ne!(capture_precheck_plan(&executor, &issue_id).evidence, before);
    }

    #[test]
    fn test_precheck_evidence_detects_prompt_mutation_and_preserves_captured_bytes() {
        let (storage, executor, issue_id) = precheck_evidence_fixture();
        let before = capture_precheck_plan(&executor, &issue_id);
        assert_eq!(before.prompts["auto-start"], "captured prompt");

        storage.add_repo_file("start-prompt.md", "mutated prompt");
        let after = capture_precheck_plan(&executor, &issue_id);

        assert_eq!(after.prompts["auto-start"], "mutated prompt");
        assert_ne!(after.evidence, before.evidence);
    }

    #[test]
    fn test_claims_guard_uses_explicit_layout_not_process_cwd() {
        use crate::storage::worktree_paths::WorktreePaths;
        use crate::storage::{ClaimCoordinator, FileLocker};
        use std::time::Duration;

        let repo = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(repo.path().join(".jit")).unwrap();
        assert!(std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(repo.path())
            .status()
            .unwrap()
            .success());
        let paths = WorktreePaths::detect_from(repo.path()).unwrap();
        let coordinator = ClaimCoordinator::new(
            paths,
            FileLocker::new(Duration::from_secs(1)),
            "wt:test".to_string(),
            "agent:test".to_string(),
        );
        coordinator.init().unwrap();
        let issue_id = "abcd1111111111111111111111111111";
        coordinator.acquire_claim(issue_id, 600).unwrap();
        let layout =
            crate::storage::discover_repository_layout(repo.path(), repo.path().join(".jit"))
                .unwrap();

        let guard = claims_mutation_guard(&layout).unwrap().unwrap();

        assert!(guard
            .has_active_lease(issue_id, None, |raw| Ok(raw.to_string()))
            .unwrap());
    }
}
