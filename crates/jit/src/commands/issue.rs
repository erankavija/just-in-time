//! Issue CRUD operations and lifecycle management

use super::*;
use crate::domain::identity::{GateKey, IssueIdPrefix};
use crate::domain::list_envelope::{collection_keys, ListEnvelope};
use crate::domain::{GateRunResult, MinimalIssue};
use crate::errors::{DeletionNotConfirmedError, TransitionBlocker};
use schemars::JsonSchema;
use serde::Serialize;

collection_keys! {
    /// Collection key of [`IssueChildrenResponse`].
    ChildrenKey = "issues" as "IssueChildrenResponse";
}

/// Per-gate view emitted in `issue show --json`.
///
/// One entry per gate in the issue's `gates_required`. `status` reflects the
/// gate's [`GateState`] (or `pending` when none exists). `last_run_at` and
/// `exit_code` come from the gate's latest [`GateRunResult`] and are both
/// `null` when no run has been recorded; a manual gate's attestation is a run
/// whose `exit_code` is `null`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateView {
    /// Gate key.
    pub key: GateKey,
    /// Gate status.
    pub status: crate::domain::GateStatus,
    /// Timestamp of the gate's latest run, or `null` when never run.
    pub last_run_at: Option<String>,
    /// Exit code of the gate's latest run, or `null` when never run.
    pub exit_code: Option<i32>,
}

impl GateView {
    /// Build the per-gate view for a single required gate.
    ///
    /// `latest` is the run named by `state`, loaded by the latest-run accessor.
    fn build(
        key: &GateKey,
        state: Option<&crate::domain::GateState>,
        latest: Option<&GateRunResult>,
    ) -> Self {
        let status = state
            .map(|s| s.status)
            .unwrap_or(crate::domain::GateStatus::Pending);

        GateView {
            key: key.clone(),
            status,
            last_run_at: latest.map(|r| r.started_at.to_rfc3339()),
            exit_code: latest.and_then(|r| r.exit_code),
        }
    }
}

/// A single unmet dependency, as projected into `issue show --json` and the
/// compact `issue status` view.
///
/// A dependency is *unmet* when it is not effectively terminal (`Done`,
/// `Rejected`, or `Archived` retired from one of those —
/// [`Issue::is_effectively_terminal`]) — the exact same readiness test as
/// [`Issue::is_blocked`] and
/// [`query_ready`](crate::domain::queries::query_ready): an effectively terminal
/// dependency unblocks its dependents, so it is never listed here. The shape is a subset of
/// the enriched `dependencies` entries (`id`, `short_id`, `title`, `state`),
/// carrying only what an orchestrator needs to see what is still blocking work.
#[derive(Debug, Serialize, JsonSchema)]
pub struct UnmetDependency {
    pub id: IssueId,
    /// Short ID (first 8 chars of the full UUID), for human-readable references.
    pub short_id: String,
    pub title: String,
    pub state: State,
}

impl From<&MinimalIssue> for UnmetDependency {
    fn from(dep: &MinimalIssue) -> Self {
        Self {
            id: dep.id.clone(),
            short_id: dep.short_id(),
            title: dep.title.clone(),
            state: dep.state,
        }
    }
}

/// Response for `issue show` command with enriched dependencies
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueShowResponse {
    pub id: IssueId,
    /// Short ID (first 8 chars of the full UUID), for human-readable references.
    pub short_id: String,
    pub title: String,
    pub description: String,
    pub state: State,
    pub priority: Priority,
    pub assignee: Option<String>,
    /// Enriched dependency list with full metadata
    pub dependencies: Vec<MinimalIssue>,
    /// Dependency ids from the issue's stored `dependencies` array that did
    /// NOT resolve to an existing issue (e.g. a stale reference left by raw
    /// storage mutation or legacy data predating the delete-cascade fix in
    /// `IssueOps::delete_issue`, jit:f847df3f). Deleting an issue
    /// through the CLI strips its own id from every dependent's
    /// `dependencies`, so this is empty in a repository touched only through
    /// the CLI; when non-empty, it means a dangling id exists and — unlike
    /// `dependencies`, which silently omits ids it cannot resolve — is not
    /// hidden.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dangling_dependency_ids: Vec<IssueId>,
    /// The subset of `dependencies` that are not yet met (not effectively terminal),
    /// consistent with readiness — see [`UnmetDependency`]. Always an array,
    /// empty when every dependency is effectively terminal (`Done`, `Rejected`,
    /// or `Archived` from one of those) or there are none.
    pub unmet_dependencies: Vec<UnmetDependency>,
    /// Per-gate view, one entry per required gate, enriched from each gate's
    /// latest run.
    pub gates: Vec<GateView>,
    pub context: std::collections::HashMap<String, String>,
    pub documents: Vec<crate::domain::DocumentReference>,
    pub labels: Vec<String>,
    /// Per-issue content format override; absent means the repo default applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_format: Option<crate::domain::ContentFormat>,
    pub created_at: String,
    pub updated_at: String,
    /// When the issue FIRST entered `ready` (RFC 3339); absent when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_ready_at: Option<String>,
    /// When the issue was FIRST claimed/assigned (RFC 3339); absent when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<String>,
    /// When the issue FIRST reached `done` (RFC 3339); absent when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done_at: Option<String>,
}

impl IssueShowResponse {
    /// Create from Issue with enriched dependencies and the issue's gate runs.
    ///
    /// `gate_runs` contains one latest run per gate, supplied by
    /// [`GateOps::get_last_gate_runs_for_issue`](crate::commands::GateOps::get_last_gate_runs_for_issue).
    /// Pass an empty slice when no gate state names a run.
    ///
    /// `enriched_deps` normally holds one [`MinimalIssue`] per entry in
    /// `issue.dependencies`, but an id that no longer resolves to a stored
    /// issue is left out of it by
    /// [`get_dependencies_enriched`](crate::commands::IssueOps::get_dependencies_enriched).
    /// This constructor recovers any such id by diffing `enriched_deps`
    /// against `issue.dependencies` and reports it via
    /// `dangling_dependency_ids` instead of dropping it silently
    /// (jit:f847df3f).
    pub fn from_issue(
        issue: crate::domain::Issue,
        enriched_deps: Vec<MinimalIssue>,
        gate_runs: &[GateRunResult],
    ) -> Self {
        let latest_runs: std::collections::HashMap<_, _> =
            gate_runs.iter().map(|run| (&run.gate_key, run)).collect();
        let gates = issue
            .gates_required
            .iter()
            .map(|key| {
                GateView::build(
                    key,
                    issue.gates_status.get(key),
                    latest_runs.get(key).copied(),
                )
            })
            .collect();

        // See the doc comment above: recover ids `enriched_deps` couldn't
        // resolve so they surface instead of vanishing from the response.
        let resolved_ids: std::collections::HashSet<&IssueId> =
            enriched_deps.iter().map(|dep| &dep.id).collect();
        let dangling_dependency_ids: Vec<IssueId> = issue
            .dependencies
            .iter()
            .filter(|dep_id| !resolved_ids.contains(dep_id))
            .cloned()
            .collect();

        // Unmet by the one predicate `Issue::is_blocked` and the transition
        // blockers apply. Dangling ids are reported separately above rather than
        // projected here.
        let unmet_dependencies: Vec<UnmetDependency> = enriched_deps
            .iter()
            .filter(|dep| !crate::domain::is_dependency_met(dep.state, dep.archived_from))
            .map(UnmetDependency::from)
            .collect();

        Self {
            short_id: issue.short_id(),
            id: issue.id,
            title: issue.title,
            description: issue.description,
            state: issue.state,
            priority: issue.priority,
            assignee: issue
                .assignee
                .as_ref()
                .map(crate::domain::Assignee::to_string),
            dependencies: enriched_deps,
            dangling_dependency_ids,
            unmet_dependencies,
            gates,
            context: issue.context,
            documents: issue.documents,
            labels: issue.labels,
            content_format: issue.content_format,
            created_at: issue.created_at.to_rfc3339(),
            updated_at: issue.updated_at.to_rfc3339(),
            first_ready_at: issue.first_ready_at.map(|t| t.to_rfc3339()),
            claimed_at: issue.claimed_at.map(|t| t.to_rfc3339()),
            done_at: issue.done_at.map(|t| t.to_rfc3339()),
        }
    }
}

/// Compact orchestration status for one issue: where it stands in one glance.
///
/// This is the small shape behind `jit issue status` (text one-liner and
/// `--json` object). It projects an [`IssueShowResponse`] down to the fields an
/// agent reconstructs by hand when it only needs "state, per-gate status, and
/// what is still blocking": `state`, a `{key, status}` entry per required gate
/// (b1586c0d convention), and the `short_id`s of the [unmet
/// dependencies](UnmetDependency). It carries no run history, description, or
/// enriched dependency metadata.
///
/// # Examples
///
/// ```
/// use jit::domain::identity::GateKey;
/// use jit::domain::{GateStatus, Issue, State};
/// use jit::commands::{IssueShowResponse, IssueStatusResponse};
///
/// let id = jit::test_utils::issue_id(1);
/// let mut issue = Issue::draft(id, "Build parser".into(), "Body".into());
/// issue.state = State::Ready;
/// let gate: GateKey = "tests".parse().unwrap();
/// issue.gates_required = vec![gate.clone()];
/// let show = IssueShowResponse::from_issue(issue, vec![], &[]);
///
/// let status = IssueStatusResponse::from_show(&show);
/// assert_eq!(status.state, State::Ready);
/// assert_eq!(status.gates.len(), 1);
/// assert_eq!(status.gates[0].key, gate);
/// assert_eq!(status.gates[0].status, GateStatus::Pending);
/// assert!(status.unmet_dependencies.is_empty());
///
/// // The one-line render is stable and greppable; empty sections read `none`.
/// let line = status.to_line();
/// assert!(line.contains("[ready]"));
/// assert!(line.contains("gates: tests=pending"));
/// assert!(line.contains("unmet: none"));
/// assert!(line.ends_with("title: Build parser"));
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueStatusResponse {
    /// Short ID (first 8 chars of the full UUID).
    pub short_id: String,
    pub state: State,
    /// One entry per required gate, `{key, status}` only.
    pub gates: Vec<GateStatusEntry>,
    /// Short ids of the dependencies that are not yet met (not effectively terminal),
    /// consistent with readiness. Empty when nothing is blocking.
    pub unmet_dependencies: Vec<String>,
    pub title: String,
}

impl IssueStatusResponse {
    /// Project a full [`IssueShowResponse`] down to the compact status shape.
    pub fn from_show(show: &IssueShowResponse) -> Self {
        Self {
            short_id: show.short_id.clone(),
            state: show.state,
            gates: show
                .gates
                .iter()
                .map(|g| GateStatusEntry {
                    key: g.key.clone(),
                    status: g.status,
                })
                .collect(),
            unmet_dependencies: show
                .unmet_dependencies
                .iter()
                .map(|d| d.short_id.clone())
                .collect(),
            title: show.title.clone(),
        }
    }
}

/// Compact container header — `{short_id, title, state}` — carried at the top of
/// `jit issue children` and `jit issue progress` JSON so a consumer sees which
/// container the child listing or rollup is for without a second lookup.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ContainerHeader {
    /// Short ID (first 8 chars of the full UUID).
    pub short_id: String,
    /// The container's title.
    pub title: String,
    /// The container's own lifecycle state.
    pub state: State,
}

impl From<&Issue> for ContainerHeader {
    fn from(issue: &Issue) -> Self {
        Self {
            short_id: issue.short_id(),
            title: issue.title.clone(),
            state: issue.state,
        }
    }
}

/// Response for `jit issue children`: a container header plus one compact
/// [`IssueStatusResponse`] per resolvable direct child (its immediate
/// dependencies, depth 1), and the ids of any child edges that no longer
/// resolve.
///
/// `issues` and `count` form the standard `{count, issues}` list envelope, with
/// `container` added on top. `dangling` follows
/// the [`IssueShowResponse::dangling_dependency_ids`] precedent: a dependency id
/// that points at no stored issue is surfaced here rather than silently dropped,
/// and is omitted from the JSON when empty. Children are ordered by ascending
/// short id (stored dependency order is set-derived and not meaningful).
///
/// `issues` holds one compact status projection per resolvable direct child,
/// ascending short id.
pub type IssueChildrenResponse = ListEnvelope<ChildrenKey, IssueStatusResponse, ChildrenScope>;

/// Fields of [`IssueChildrenResponse`] beside its `issues`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ChildrenScope {
    /// `{short_id, title, state}` of the queried container.
    pub container: ContainerHeader,
    /// Dependency ids that resolve to no stored issue (dangling edges), omitted
    /// when empty. `count`/`issues` cover resolvable children only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dangling: Vec<IssueId>,
}

/// Response for `jit issue progress`: a container header, the counts-by-state
/// [`StateRollup`] over its resolvable direct children, and any dangling child
/// edges.
///
/// The [`StateRollup`] fields are flattened to the top level, so the JSON is the `StateRollup`
/// shape plus `container` (and `dangling` when non-empty). `total` and every
/// count are over **resolvable** children only; a dependency id resolving to no
/// stored issue is surfaced in `dangling` (the [`IssueShowResponse`] precedent)
/// rather than counted or dropped.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ContainerProgressResponse {
    /// `{short_id, title, state}` of the queried container.
    pub container: ContainerHeader,
    /// The counts-by-state rollup over resolvable direct children, flattened to
    /// the top level.
    #[serde(flatten)]
    pub rollup: StateRollup,
    /// Dependency ids that resolve to no stored issue (dangling edges), omitted
    /// when empty. The rollup counts resolvable children only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dangling: Vec<IssueId>,
}

/// Per-required-gate readiness entry in `gate status-all`.
///
/// Reports one required gate's authoritative per-issue [`GateStatus`] so a
/// `--json` consumer can distinguish a gate that
/// ran and failed from one that has never run/attested, both of which map to
/// the single nonzero exit code of `gate status-all`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateStatusEntry {
    pub key: GateKey,
    pub status: GateStatus,
}

impl GateStatusEntry {
    /// Build the `{key, status}` entry for one required gate: its recorded
    /// [`GateState`] status, or `pending` when no run or attestation exists.
    ///
    /// Centralizes the pending-default so the compact `gates` projection shared
    /// by `issue status` and `issue show --summary` matches the status the full
    /// [`GateView`] reports for the same gate.
    pub fn for_required(key: &GateKey, state: Option<&GateState>) -> Self {
        Self {
            key: key.clone(),
            status: state.map(|s| s.status).unwrap_or(GateStatus::Pending),
        }
    }
}

/// How `update_issue` should apply a new description value.
///
/// `Replace` overwrites the existing description entirely (`--description` /
/// `--description-file`). `Append` adds text to the end of whatever
/// description already exists (`--append-description` /
/// `--append-description-file`), separated by exactly one blank line; when
/// the existing description is empty the result is just the new text, with
/// no leading blank line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescriptionUpdate {
    Replace(String),
    Append(String),
}

impl DescriptionUpdate {
    /// Apply this operation against an existing description, returning the
    /// resulting description text.
    pub fn apply(self, existing: &str) -> String {
        match self {
            DescriptionUpdate::Replace(text) => text,
            DescriptionUpdate::Append(text) if existing.is_empty() => text,
            DescriptionUpdate::Append(text) => format!("{existing}\n\n{text}"),
        }
    }
}

/// Gate and assignee changes an `issue update` applies, in this order, before
/// its field changes. `unassign` takes precedence over `assignee`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueLocalUpdate {
    pub add_gates: Vec<GateKey>,
    pub remove_gates: Vec<GateKey>,
    pub assignee: Option<crate::domain::Assignee>,
    pub unassign: bool,
}

pub(super) fn blocking_dependencies(
    issue: &Issue,
    resolved_issues: &std::collections::HashMap<IssueId, &Issue>,
) -> Vec<TransitionBlocker> {
    issue
        .dependencies
        .iter()
        .filter_map(|dep_id| match resolved_issues.get(dep_id).copied() {
            Some(dependency) if is_dependency_met(dependency.state, dependency.archived_from) => {
                None
            }
            Some(dependency) => Some(TransitionBlocker::dependency(dependency.clone())),
            None => Some(TransitionBlocker::missing_dependency(dep_id.clone())),
        })
        .collect()
}

struct DerivedIssueDeletion {
    intents: Vec<crate::repository_state::MutationIntent>,
    error_after_apply: Option<crate::errors::TransitionBlockedError>,
}

/// Derive deletion, dependent-edge cleanup, and resulting automatic readiness
/// transitions from one captured active graph.
fn derive_issue_deletion(
    image: &crate::repository_state::RepositoryImage,
    context: &crate::repository_state::MutationContext,
    issues: &[Issue],
    target_id: &IssueId,
) -> Result<DerivedIssueDeletion> {
    use crate::repository_state::MutationIntent;

    let graph = super::CapturedGraphEvidence::capture(image, issues, context)?;
    let mut projected = issues
        .iter()
        .filter(|issue| &issue.id != target_id)
        .cloned()
        .collect::<Vec<_>>();
    let dependents = projected
        .iter_mut()
        .filter(|issue| issue.dependencies.iter().any(|id| id == target_id))
        .map(|issue| {
            issue.dependencies.retain(|id| id != target_id);
            issue.clone()
        })
        .collect::<Vec<_>>();
    let readiness = derive_readiness_intents(issues, &projected, &graph, ReadinessScope::Affected)?;

    let mut intents = std::iter::once(MutationIntent::DeleteIssue {
        issue_id: target_id.clone(),
    })
    .chain(
        dependents
            .iter()
            .filter(|dependent| !readiness.corrects(&dependent.id))
            .map(|dependent| MutationIntent::UpdateIssue {
                issue: Box::new(dependent.clone()),
            }),
    )
    .chain(
        dependents
            .iter()
            .map(|dependent| MutationIntent::RecordEvent {
                phase: 2,
                event: Box::new(EventBody::draft_issue_updated(
                    dependent.id.clone(),
                    "dependency-cascade-delete".to_string(),
                    vec!["dependencies".to_string()],
                )),
            }),
    )
    .chain(std::iter::once(MutationIntent::RecordEvent {
        phase: 1,
        event: Box::new(EventBody::draft_issue_deleted(target_id.clone())),
    }))
    .collect::<Vec<_>>();
    intents.extend(readiness.intents);
    Ok(DerivedIssueDeletion {
        intents,
        error_after_apply: readiness.error_after_apply,
    })
}

/// The issue as persisted by creation, with its advisory findings.
#[derive(Debug, Clone)]
pub struct IssueCreation {
    pub issue: Issue,
    /// Local-rule findings, then graph-rule findings.
    pub findings: Vec<crate::validation::ReportedFinding>,
}

/// The issue as persisted by a mutation, with its advisory warnings.
#[derive(Debug, Clone)]
pub struct IssueMutation<W = String> {
    pub issue: Issue,
    pub warnings: Vec<W>,
}

/// The id a deletion removed, with its advisory warnings.
#[derive(Debug, Clone)]
pub struct IssueDeletion {
    pub id: IssueId,
    pub warnings: Vec<String>,
}

/// Issue lifecycle operations of a [`CommandExecutor`].
pub struct IssueOps<'a, S: IssueStore> {
    pub(super) exec: &'a CommandExecutor<S>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// The issue-lifecycle family.
    pub fn issues(&self) -> IssueOps<'_, S> {
        IssueOps { exec: self }
    }
}

impl<S: IssueStore> IssueOps<'_, S> {
    /// Resolve a full issue ID or unique prefix in this repository.
    pub fn resolve_issue_id(&self, prefix: &IssueIdPrefix) -> Result<IssueId> {
        self.exec.storage.resolve_issue_id(prefix)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_issue(
        &self,
        title: String,
        description: String,
        priority: Priority,
        gates: Vec<GateKey>,
        mut labels: Vec<String>,
        content_format: Option<crate::domain::ContentFormat>,
        issue_type: Option<String>,
        force: bool,
    ) -> Result<IssueCreation>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // REQ-02: an explicit `--type <kind>` derives the canonical `type:<kind>`
        // label, replacing any caller-supplied `type:*` label. The command layer
        // owns this derivation (the CLI only forwards the typed value). Applied
        // BEFORE the default-type fallback so an explicit type suppresses the
        // default. The captured publisher validates kind membership and the final
        // shape against the declarations in its mutation read set.
        let explicit_type = issue_type.is_some();
        if let Some(kind) = issue_type {
            labels.retain(|label| !label_utils::is_type_label(label));
            labels.push(label_utils::type_label(&kind));
        }

        let context = super::production_mutation_context();
        let mut issue = Issue::draft(context.issue_id_at(0), title, description);
        issue.priority = priority;
        issue.gates_required = gates;
        issue.labels = labels;
        issue.content_format = content_format;

        // REQ-02: when `--type` was explicitly provided, hard-reject an undeclared
        // kind through the SAME rule engine the write uses
        // (`type-hierarchy-known`), which only warns on the normal path.
        // Scoped to the explicit-type path so non-`--type` writes keep warn-only
        // behavior; runs before the write so a bad type changes nothing.
        let (issue, findings) =
            self.exec
                .publish_issue_creation(&context, issue, explicit_type, force)?;

        Ok(IssueCreation { issue, findings })
    }

    pub fn list_issues(
        &self,
        state_filter: Option<State>,
        assignee_filter: Option<crate::domain::Assignee>,
        priority_filter: Option<Priority>,
    ) -> Result<Vec<Issue>> {
        let issues = self.exec.storage.list_issues()?;

        let filtered = issues
            .into_iter()
            .filter(|issue| {
                if let Some(ref state) = state_filter {
                    if &issue.state != state {
                        return false;
                    }
                }
                if let Some(ref assignee) = assignee_filter {
                    if !issue.assignee.as_ref().is_some_and(|a| a == assignee) {
                        return false;
                    }
                }
                if let Some(ref priority) = priority_filter {
                    if &issue.priority != priority {
                        return false;
                    }
                }
                true
            })
            .collect();

        Ok(filtered)
    }

    pub fn show_issue(&self, id: &IssueId) -> Result<Issue> {
        let full_id = id.clone();
        self.exec.storage.load_issue(&full_id)
    }

    /// Get enriched dependency information for an issue.
    ///
    /// Only ids that resolve to a stored issue are returned; a dependency id
    /// that no longer resolves (e.g. a dangling reference left by raw storage
    /// mutation or legacy data — normal deletion no longer produces one, see
    /// `delete_issue`) is left out here. That is not a loss of information:
    /// [`IssueShowResponse::from_issue`](crate::commands::IssueShowResponse::from_issue)
    /// diffs its own `enriched_deps` argument against the issue's full
    /// `dependencies` list to recover any id missing here and surfaces it via
    /// `dangling_dependency_ids`, so `jit issue show --json` never silently
    /// hides one (jit:f847df3f).
    pub fn get_dependencies_enriched(&self, issue: &Issue) -> Vec<crate::domain::MinimalIssue> {
        issue
            .dependencies
            .iter()
            .filter_map(|dep_id| {
                self.exec
                    .storage
                    .load_issue(dep_id)
                    .ok()
                    .map(|dep| crate::domain::MinimalIssue::from(&dep))
            })
            .collect()
    }

    /// Build the [`IssueShowResponse`](crate::commands::IssueShowResponse) for one
    /// already-loaded issue: enriched dependencies, gate runs, and the
    /// documents every reader reports ([`DocumentOps::issue_documents`]).
    pub fn show_response(&self, issue: Issue) -> Result<crate::commands::IssueShowResponse> {
        let enriched_deps = self.get_dependencies_enriched(&issue);
        let (gate_runs, _) = self
            .exec
            .gates()
            .get_last_gate_runs_for_issue(&issue.id)
            .with_context(|| format!("Failed to load gate runs for issue {}", issue.id))?;
        let documents = self.exec.documents().issue_documents(&issue)?;
        Ok(crate::commands::IssueShowResponse {
            documents,
            ..crate::commands::IssueShowResponse::from_issue(issue, enriched_deps, &gate_runs)
        })
    }

    /// Build the compact [`IssueStatusResponse`](crate::commands::IssueStatusResponse)
    /// for one already-loaded issue.
    ///
    /// Loads the issue's enriched dependencies and gate runs, builds the full
    /// [`IssueShowResponse`](crate::commands::IssueShowResponse), and projects it
    /// down to the compact status shape — so the projection is byte-for-byte the
    /// same one `jit issue show`/`jit issue status` produce. Shared by
    /// `issue status` and `issue children` (the latter is this projection over a
    /// container's direct dependencies).
    pub fn issue_status_response(
        &self,
        issue: Issue,
    ) -> Result<crate::commands::IssueStatusResponse> {
        let enriched_deps = self.get_dependencies_enriched(&issue);
        let (gate_runs, _) = self.exec.gates().get_last_gate_runs_for_issue(&issue.id)?;
        let show = crate::commands::IssueShowResponse::from_issue(issue, enriched_deps, &gate_runs);
        Ok(crate::commands::IssueStatusResponse::from_show(&show))
    }

    /// Resolve a container's direct dependency edges (its depth-1 children) into
    /// the resolvable child issues and the ids of any dangling edges.
    ///
    /// A dependency id resolving to no stored issue is a *dangling* edge and is
    /// collected separately (following the `issue show`
    /// [`dangling_dependency_ids`](crate::commands::IssueShowResponse::dangling_dependency_ids)
    /// precedent) rather than silently dropped. Only a not-found id is treated
    /// as dangling: any genuine storage error (I/O, deserialization) propagates
    /// via `?` instead of being swallowed. Dangling ids are returned sorted.
    fn resolve_direct_children(&self, container: &Issue) -> Result<(Vec<Issue>, Vec<IssueId>)> {
        let mut children = Vec::new();
        let mut dangling = Vec::new();
        for dep_id in &container.dependencies {
            match self.exec.storage.load_issue_or_not_found(dep_id) {
                Ok(child) => children.push(child),
                Err(crate::storage::PathReadError::NotFound(_)) => dangling.push(dep_id.clone()),
                // A real lookup failure is not a dangling edge — surface it.
                Err(other) => return Err(other.into()),
            }
        }
        dangling.sort();
        Ok((children, dangling))
    }

    /// Build the `jit issue children` response: the container's direct children
    /// (depth-1 dependencies), each projected to the compact `issue status`
    /// shape, plus any dangling edges.
    ///
    /// Containment follows the dependency DAG (membership labels are advisory
    /// and not consulted). The container id is resolved the same way as
    /// `issue show`, so a bad id yields the typed id-resolution error unwrapped.
    /// Children are ordered by ascending short id, since stored dependency
    /// order is set-derived and not meaningful.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::commands::CommandExecutor;
    /// use jit::domain::Priority;
    /// use jit::storage::InMemoryStorage;
    ///
    /// let storage = InMemoryStorage::new();
    /// storage.add_data_file("config.toml", "");
    /// let layout = storage.repository_layout();
    /// let executor = CommandExecutor::new(storage).with_layout(layout).unwrap();
    /// let new = |title: &str| {
    ///     executor
    ///         .issues().create_issue(title.into(), String::new(), Priority::Normal,
    ///             vec![], vec![], None, None, false)
    ///         .unwrap()
    ///         .issue.id
    /// };
    ///
    /// let epic = new("Epic");
    /// let child = new("Child");
    /// executor
    ///     .graph()
    ///     .add_dependencies_with_policy(
    ///         &epic,
    ///         std::slice::from_ref(&child),
    ///         jit::commands::RedundancyPolicy::Reduce,
    ///     )
    ///     .unwrap();
    ///
    /// let response = executor.issues().issue_children(&epic).unwrap();
    /// assert_eq!(response.extras.container.title, "Epic");
    /// assert_eq!(response.items.len(), 1);
    /// assert_eq!(response.items[0].title, "Child");
    /// assert!(response.extras.dangling.is_empty());
    /// ```
    pub fn issue_children(&self, id: &IssueId) -> Result<crate::commands::IssueChildrenResponse> {
        let container = self.show_issue(id)?;
        let (children, dangling) = self.resolve_direct_children(&container)?;

        let mut issues = children
            .into_iter()
            .map(|child| self.issue_status_response(child))
            .collect::<Result<Vec<_>>>()?;
        issues.sort_by(|a, b| a.short_id.cmp(&b.short_id));

        Ok(crate::commands::IssueChildrenResponse::with_extras(
            issues,
            crate::commands::ChildrenScope {
                container: crate::commands::ContainerHeader::from(&container),
                dangling,
            },
        ))
    }

    /// Build the `jit issue progress` response: the counts-by-state rollup over
    /// the container's direct children (depth 1), plus any dangling edges.
    ///
    /// Membership follows the dependency DAG, as for [`Self::issue_children`].
    /// The rollup counts resolvable children only; dangling edges are surfaced
    /// separately rather than counted or dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::commands::CommandExecutor;
    /// use jit::domain::{Priority, State};
    /// use jit::storage::InMemoryStorage;
    ///
    /// let storage = InMemoryStorage::new();
    /// storage.add_data_file("config.toml", "");
    /// let layout = storage.repository_layout();
    /// let executor = CommandExecutor::new(storage).with_layout(layout).unwrap();
    /// let new = |title: &str| {
    ///     executor
    ///         .issues().create_issue(title.into(), String::new(), Priority::Normal,
    ///             vec![], vec![], None, None, false)
    ///         .unwrap()
    ///         .issue.id
    /// };
    ///
    /// let epic = new("Epic");
    /// let done = new("Done child");
    /// let todo = new("Todo child");
    /// executor
    ///     .graph()
    ///     .add_dependencies_with_policy(
    ///         &epic,
    ///         std::slice::from_ref(&done),
    ///         jit::commands::RedundancyPolicy::Reduce,
    ///     )
    ///     .unwrap();
    /// executor
    ///     .graph()
    ///     .add_dependencies_with_policy(
    ///         &epic,
    ///         std::slice::from_ref(&todo),
    ///         jit::commands::RedundancyPolicy::Reduce,
    ///     )
    ///     .unwrap();
    /// executor
    ///     .issues().update_issue(&done, None, None, None, Some(State::Done),
    ///         vec![], vec![], None, None, false)
    ///     .unwrap();
    ///
    /// let progress = executor.issues().issue_progress(&epic).unwrap();
    /// assert_eq!(progress.rollup.extras.total, 2);
    /// assert_eq!(progress.rollup.extras.done, 1);
    /// assert_eq!(progress.rollup.extras.percent, 50);
    /// assert!(progress.dangling.is_empty());
    /// ```
    pub fn issue_progress(
        &self,
        id: &IssueId,
    ) -> Result<crate::commands::ContainerProgressResponse> {
        let container = self.show_issue(id)?;
        let (children, dangling) = self.resolve_direct_children(&container)?;

        Ok(crate::commands::ContainerProgressResponse {
            container: crate::commands::ContainerHeader::from(&container),
            rollup: crate::commands::StateRollup::from_issues(&children),
            dangling,
        })
    }

    /// Update issue fields.
    ///
    /// Note: This function has 9 parameters (exceeds clippy's 7-parameter guideline).
    /// This is intentional because:
    /// - Each parameter corresponds to a distinct CLI flag (--title, --desc, --priority, etc.)
    /// - Grouping into a struct would obscure the 1:1 CLI mapping
    /// - All parameters are optional (except id), making a builder pattern overkill
    /// - The function is only called from CLI parsing, not used as a general API
    #[allow(clippy::too_many_arguments)]
    pub fn update_issue(
        &self,
        id: &IssueId,
        title: Option<String>,
        description: Option<DescriptionUpdate>,
        priority: Option<Priority>,
        state: Option<State>,
        add_labels: Vec<String>,
        remove_labels: Vec<String>,
        // Tri-state: `None` leaves the field unchanged; `Some(None)` clears the
        // override back to repo-default inheritance; `Some(Some(fmt))` sets it.
        content_format: Option<Option<crate::domain::ContentFormat>>,
        issue_type: Option<String>,
        force: bool,
    ) -> Result<Vec<String>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.update_issue_resolved(
            &IssueIdPrefix::from(id),
            title,
            description,
            priority,
            state,
            add_labels,
            remove_labels,
            content_format,
            issue_type,
            IssueLocalUpdate::default(),
            force,
        )
        .map(|result| result.warnings)
    }

    /// [`Self::update_issue`] combined with `local` gate and assignee changes,
    /// published as one mutation: either every requested change applies or
    /// none does. Returns the issue as persisted by the mutation.
    #[allow(clippy::too_many_arguments)]
    pub fn update_issue_resolved(
        &self,
        id: &IssueIdPrefix,
        title: Option<String>,
        description: Option<DescriptionUpdate>,
        priority: Option<Priority>,
        state: Option<State>,
        add_labels: Vec<String>,
        remove_labels: Vec<String>,
        content_format: Option<Option<crate::domain::ContentFormat>>,
        issue_type: Option<String>,
        local: IssueLocalUpdate,
        force: bool,
    ) -> Result<IssueMutation>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let id = self.exec.storage.resolve_issue_id(id)?;
        let assignee = local.assignee;
        let outcome = self
            .exec
            .publish_captured_field_update(CapturedFieldUpdate {
                issue_id: id.clone(),
                title,
                description,
                priority,
                state,
                add_labels,
                remove_labels,
                append_label: None,
                content_format,
                issue_type,
                add_gates: local.add_gates,
                remove_gates: local.remove_gates,
                assignee,
                unassign: local.unassign,
                bulk: false,
                force,
                enforce_lease: true,
                rejection: false,
            })?;

        Ok(IssueMutation {
            issue: outcome.issue,
            warnings: outcome.warnings,
        })
    }

    /// Confirm operator intent for the destructive `jit issue delete` command
    /// (jit:0daba57d).
    ///
    /// Deletion is disabled by default (Phase 3 safety): the caller must pass
    /// `allowed = true`, which the CLI dispatch derives from the process
    /// environment (`JIT_ALLOW_DELETION=1`). Reading that environment variable
    /// is left to the caller — mirroring `resolve_gate_key`'s
    /// already-resolved-input pattern — so this stays a pure decision, testable
    /// without mutating global process state. Returns
    /// [`DeletionNotConfirmedError`] (classified by `output::failure::classify` as
    /// `ExitCode::InvalidArgument`, exit code 2) rather than performing the
    /// delete when `allowed` is `false`; the resolved `id` names the refused deletion.
    pub fn confirm_deletion_allowed(
        &self,
        id: &IssueId,
        allowed: bool,
    ) -> std::result::Result<(), DeletionNotConfirmedError> {
        if allowed {
            Ok(())
        } else {
            Err(DeletionNotConfirmedError::new(id.clone()))
        }
    }

    /// Delete the issue `requested` resolves to, once `allowed` confirms
    /// operator intent ([`Self::confirm_deletion_allowed`]).
    ///
    /// Returns the deleted id and any warnings (e.g., lease warnings).
    ///
    /// Also strips `id` from the `dependencies` array of every issue that
    /// referenced it (jit:f847df3f). Without this cascade a deleted issue left
    /// a dangling edge behind: `jit validate` rejected it, and the normal `jit
    /// dep rm <from> <id>` path could not repair it because removing a
    /// dependency resolved the target id first, and a deleted target no longer
    /// resolves. The cascade is an automatic invariant-maintaining side effect
    /// of deletion, not a user-initiated edit to the dependent — like
    /// dependency addition's Ready-to-Backlog demotion, it intentionally bypasses
    /// the active-lease check on the dependents; only the issue actually being
    /// deleted is lease-checked (above). The dependent rewrites, deletion, tombstone,
    /// and audit events publish through one redo record,
    /// so the repository ends with either the complete old graph or the complete new
    /// graph. `jit dep rm` (which matches a raw stored dependency id without
    /// requiring target resolution) and `jit issue show --json`'s
    /// `dangling_dependency_ids` field remain as defense-in-depth for legacy or
    /// externally edited data.
    pub fn delete_issue(&self, requested: &IssueIdPrefix, allowed: bool) -> Result<IssueDeletion>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::finalize;

        let id = &self.exec.storage.resolve_issue_id(requested)?;
        self.confirm_deletion_allowed(id, allowed)?;
        let layout = self.exec.require_layout()?;
        let context = super::production_mutation_context();
        with_mutation_attempts("issue deletion", || {
            let (expected_target, expected_mode) = {
                let mut preflight = self.exec.storage.open_mutation_session(layout.clone())?;
                let Some(preflight) =
                    self.exec
                        .capture_mutation_preflight(preflight.as_mut(), id, |_| false)?
                else {
                    return Ok(AttemptOutcome::Retry);
                };
                let config = crate::repository_state::assemble_config(&preflight.image)?;
                (preflight.target.id, config.enforcement_mode())
            };
            // Claims coordination is acquired before repository serialization;
            // the target and policy are then re-resolved from the held image.
            let claims_guard = (expected_mode != crate::config::EnforcementMode::Off)
                .then(|| claims_mutation_guard(self.exec.require_worktree_paths()?))
                .transpose()?
                .flatten();
            let mut session = self.exec.storage.open_mutation_session(layout.clone())?;
            let Some(crate::commands::validate::MutationBase { image, issues }) = self
                .exec
                .capture_mutation_base(session.as_mut(), &[], None)?
            else {
                return Ok(AttemptOutcome::Retry);
            };
            let target_id = require_issue_in_capture(&issues, id)?;
            let config = crate::repository_state::assemble_config(&image)?;
            let current_mode = config.enforcement_mode();
            if target_id != expected_target || current_mode != expected_mode {
                return Ok(AttemptOutcome::Retry);
            }
            let warnings = refuse_foreign_leases_held(
                current_mode,
                std::slice::from_ref(&target_id),
                claims_guard.as_ref(),
            )?;
            let derived = derive_issue_deletion(&image, &context, &issues, &target_id)?;
            let plan = finalize(&layout, &image, &context, &derived.intents, None)?;
            if let AttemptOutcome::Retry = classify_apply(session.apply(&plan), ())? {
                return Ok(AttemptOutcome::Retry);
            }
            if let Some(error) = derived.error_after_apply {
                return Err(error.into());
            }
            Ok(AttemptOutcome::Done(IssueDeletion {
                id: target_id,
                warnings,
            }))
        })
    }

    /// Reject an issue, recording `reason` as its `resolution:` label and
    /// promoting each dependent it unblocks, all in one redo record. A `reason` that cannot form a valid label is refused
    /// before anything is written. Rejection bypasses rule enforcement, so
    /// other label-rule findings surface as warnings.
    pub fn reject_issue(&self, id: &IssueIdPrefix, reason: Option<&str>) -> Result<IssueMutation>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let id = self.exec.storage.resolve_issue_id(id)?;
        let resolution = reason.map(crate::labels::resolution_label).transpose()?;
        let outcome = self
            .exec
            .publish_captured_field_update(CapturedFieldUpdate::rejection(id, resolution))?;
        Ok(IssueMutation {
            issue: outcome.issue,
            warnings: outcome.warnings,
        })
    }

    /// Assign an issue to someone.
    ///
    /// Returns the persisted issue and any lease warnings.
    pub fn assign_issue(
        &self,
        id: &IssueIdPrefix,
        assignee: crate::domain::Assignee,
    ) -> Result<IssueMutation>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let full_id = self.exec.storage.resolve_issue_id(id)?;

        // Collect warnings instead of printing
        let warnings = self.exec.refuse_foreign_lease(&full_id)?;

        let (issue, _) =
            self.exec
                .publish_captured_issue_mutation(CapturedIssueMutation::Assign {
                    issue_id: full_id,
                    assignee,
                })?;
        Ok(IssueMutation { issue, warnings })
    }

    /// Claim an issue for `assignee`.
    ///
    /// Returns the persisted issue.
    pub fn claim_issue(
        &self,
        id: &IssueIdPrefix,
        assignee: crate::domain::Assignee,
    ) -> Result<IssueMutation>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // The captured claim owns the precheck capture/checker/recapture loop so
        // checker success and coordinator evidence cannot be applied to a
        // different captured source state.
        let id = self.exec.storage.resolve_issue_id(id)?;
        self.exec
            .publish_captured_claim(&id, assignee)
            .map(|issue| IssueMutation {
                issue,
                warnings: Vec::new(),
            })
    }

    /// Unassign an issue.
    ///
    /// Returns the persisted issue and any lease warnings.
    pub fn unassign_issue(&self, id: &IssueIdPrefix) -> Result<IssueMutation>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let full_id = self.exec.storage.resolve_issue_id(id)?;

        // Collect warnings instead of printing
        let warnings = self.exec.refuse_foreign_lease(&full_id)?;

        let (issue, _) =
            self.exec
                .publish_captured_issue_mutation(CapturedIssueMutation::Unassign {
                    issue_id: full_id,
                })?;
        Ok(IssueMutation { issue, warnings })
    }

    pub fn release_issue(&self, id: &IssueIdPrefix, reason: &str) -> Result<Issue>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let full_id = self.exec.storage.resolve_issue_id(id)?;
        self.exec
            .publish_captured_release(&full_id, reason.to_string())
    }

    /// Claim the highest-priority ready issue matching `filter`, a
    /// [`QueryFilter`](crate::query_engine::QueryFilter) expression.
    ///
    /// The filter is parsed before anything is claimed; a parse failure is an
    /// [`InvalidArgumentError`](crate::errors::InvalidArgumentError).
    pub fn claim_next(
        &self,
        assignee: crate::domain::Assignee,
        filter: Option<String>,
    ) -> Result<IssueMutation>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let filter = filter
            .map(|expr| {
                crate::query_engine::QueryFilter::parse(&expr)
                    .map_err(|e| crate::errors::InvalidArgumentError::new(e.to_string()))
            })
            .transpose()?;
        let issues = self.exec.storage.list_issues()?;
        let context = crate::query_engine::QueryContext::from_issues(&issues);

        let candidates = crate::domain::queries::query_available(
            &issues,
            crate::domain::queries::AvailableFilter::default(),
        );
        if let Some(issue) = candidates
            .iter()
            .find(|i| filter.as_ref().is_none_or(|f| f.matches(i, &context)))
        {
            self.claim_issue(&IssueIdPrefix::from(&issue.id), assignee)
        } else {
            Err(crate::errors::NotFoundError::new("No ready issues available").into())
        }
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Move a `Gated` issue whose gates all passed to `Done`, with the readiness
    /// corrections its dependents need, in one redo record.
    pub(super) fn auto_transition_to_done(&self, issue_id: &IssueId) -> Result<bool>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.publish_captured_lifecycle_mutation(super::CapturedLifecycleMutation::AutoDone {
            issue_id: issue_id.clone(),
        })
        .map(|outcome| outcome.changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{memory_executor_declaring, MemoryDeclarations};
    use crate::declarations::GateDefinition;
    use crate::declarations::{GateMode, GateStage};
    use crate::domain::EventBody;
    use crate::domain::State;
    use crate::storage::{InMemoryStorage, MutationPoint};
    use crate::test_utils::add_reducing_edge;
    use std::collections::HashMap;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct SessionOpenCounter(AtomicUsize);

    impl crate::storage::MutationInterruptionInjector for SessionOpenCounter {
        fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
            if point == &MutationPoint::SessionOpen {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    struct PointFailure(std::sync::Mutex<Option<MutationPoint>>);

    impl crate::storage::MutationInterruptionInjector for PointFailure {
        fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
            let mut selected = self.0.lock().unwrap();
            if selected.as_ref() == Some(point) {
                selected.take();
                return Err(std::io::Error::other("injected mutation failure"));
            }
            Ok(())
        }
    }

    /// Enforcement off, plus the type vocabulary these tests assert on: an
    /// explicit `--type` is accepted or rejected against what this
    /// configuration declares.
    fn scenario_config() -> String {
        format!(
            "[worktree]\nenforce_leases = \"off\"\n\n{}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        )
    }

    fn seed_scenario_gates(storage: &InMemoryStorage) {
        crate::test_harness::seed_memory_gate_keys(
            storage,
            &["tests".parse().unwrap(), "code-review".parse().unwrap()],
        );
    }

    #[test]
    fn test_claim_next_without_ready_issue_returns_typed_not_found_error() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let error = executor
            .issues()
            .claim_next("agent:test".parse().unwrap(), None)
            .expect_err("an empty ready set must reject claim-next");

        assert!(
            error
                .downcast_ref::<crate::errors::NotFoundError>()
                .is_some(),
            "claim-next absence must retain a typed classifier, got: {error:?}"
        );
    }

    /// Seed one issue in `state` that depends on `dependencies`.
    fn seed_in_state(
        storage: &InMemoryStorage,
        title: &str,
        state: State,
        dependencies: &[&IssueId],
    ) -> IssueId {
        let mut issue = crate::test_utils::fixture_issue(title.into(), String::new());
        issue.state = state;
        issue.dependencies = dependencies.iter().map(|id| (*id).clone()).collect();
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(storage, issue);
        id
    }

    /// Progress counts done and rejected children apart and every non-terminal
    /// child as open; children are the direct dependencies in short-id order.
    #[test]
    fn test_issue_progress_and_issue_children_over_children_in_mixed_states() {
        let storage = InMemoryStorage::new();
        let done = seed_in_state(&storage, "Done child", State::Done, &[]);
        let wip = seed_in_state(&storage, "WIP child", State::InProgress, &[]);
        let rejected = seed_in_state(&storage, "Rejected child", State::Rejected, &[]);
        let ready = seed_in_state(&storage, "Ready child", State::Ready, &[]);
        let epic = seed_in_state(
            &storage,
            "Epic",
            State::Backlog,
            &[&done, &wip, &rejected, &ready],
        );
        let executor = crate::commands::test_helpers::memory_executor(storage);

        let progress = executor.issues().issue_progress(&epic).unwrap();
        assert_eq!(progress.container.title, "Epic");
        assert!(progress.dangling.is_empty());
        let rollup = &progress.rollup;
        assert_eq!(rollup.extras.total, 4);
        assert_eq!(rollup.extras.done, 1);
        assert_eq!(rollup.extras.rejected, 1);
        assert_eq!(rollup.extras.open, 2);
        assert_eq!(rollup.extras.percent, 25);
        assert_eq!(rollup.items.len(), State::ALL.len());
        let bucket = |s: State| rollup.items.iter().find(|c| c.state == s).unwrap().count;
        assert_eq!(bucket(State::Done), 1);
        assert_eq!(bucket(State::Gated), 0);

        let children = executor.issues().issue_children(&epic).unwrap();
        assert_eq!(children.items.len(), 4);
        assert!(children.extras.dangling.is_empty());
        let short_ids: Vec<&str> = children.items.iter().map(|c| c.short_id.as_str()).collect();
        assert!(short_ids.is_sorted(), "children sorted by short id");
    }

    /// The status projection carries the issue's per-gate status and its
    /// non-terminal dependencies as unmet.
    #[test]
    fn test_issue_status_response_projects_gates_and_unmet_dependencies() {
        let storage = InMemoryStorage::new();
        let dep = seed_in_state(&storage, "Upstream", State::Ready, &[]);
        let mut child = crate::test_utils::fixture_issue("Gated child".into(), String::new());
        child.dependencies = vec![dep.clone()];
        child.gates_required = vec!["tests".parse().unwrap()];
        crate::commands::test_helpers::seed_issue(&storage, child.clone());
        let executor = crate::commands::test_helpers::memory_executor(storage);

        let status = executor.issues().issue_status_response(child).unwrap();

        assert_eq!(status.title, "Gated child");
        assert_eq!(status.gates.len(), 1);
        assert_eq!(status.gates[0].key, "tests".parse().unwrap());
        assert_eq!(status.unmet_dependencies, [dep.short_id()]);
    }

    fn seed_ready(
        executor: &CommandExecutor<InMemoryStorage>,
        title: &str,
        priority: Priority,
        label: &str,
    ) -> IssueId {
        let mut issue = crate::test_utils::fixture_issue(title.into(), String::new());
        issue.state = State::Ready;
        issue.priority = priority;
        issue.labels = vec![label.to_string()];
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        id
    }

    #[test]
    fn test_claim_next_filter_excluding_top_priority_claims_best_matching_issue() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        seed_ready(&executor, "Alpha", Priority::Critical, "type:task");
        let beta = seed_ready(&executor, "Beta", Priority::Low, "type:epic");
        let gamma = seed_ready(&executor, "Gamma", Priority::High, "type:epic");

        let claimed = executor
            .issues()
            .claim_next(
                "agent:w1".parse().unwrap(),
                Some("label:type:epic".to_string()),
            )
            .unwrap()
            .issue
            .id;

        assert_eq!(claimed, gamma);
        let unclaimed = executor.storage.load_issue(&beta).unwrap();
        assert!(unclaimed.assignee.is_none());
    }

    #[test]
    fn test_claim_next_filter_matching_no_ready_issue_returns_not_found_and_claims_nothing() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let alpha = seed_ready(&executor, "Alpha", Priority::Critical, "type:task");

        let error = executor
            .issues()
            .claim_next(
                "agent:w1".parse().unwrap(),
                Some("label:type:epic".to_string()),
            )
            .expect_err("a filter matching no ready issue must not claim");

        assert!(error
            .downcast_ref::<crate::errors::NotFoundError>()
            .is_some());
        assert!(executor
            .storage
            .load_issue(&alpha)
            .unwrap()
            .assignee
            .is_none());
    }

    #[test]
    fn test_claim_next_unparseable_filter_returns_invalid_argument_and_claims_nothing() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let alpha = seed_ready(&executor, "Alpha", Priority::Critical, "type:task");

        let error = executor
            .issues()
            .claim_next("agent:w1".parse().unwrap(), Some("garbage %%%".to_string()))
            .expect_err("a malformed filter must be rejected");

        assert!(error
            .downcast_ref::<crate::errors::InvalidArgumentError>()
            .is_some());
        assert!(executor
            .storage
            .load_issue(&alpha)
            .unwrap()
            .assignee
            .is_none());
    }

    #[test]
    fn test_claim_issue_enforces_prechecks() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        // Define a manual precheck gate (TDD reminder)
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "tdd-reminder".parse().unwrap(),
            GateDefinition {
                version: 1,
                key: "tdd-reminder".parse().unwrap(),
                title: "TDD Reminder".to_string(),
                description: "Write tests first".to_string(),
                stage: GateStage::Precheck,
                mode: GateMode::Manual,
                checker: None,
                priority: 100,
                reserved: HashMap::new(),
                inputs: None,
            },
        );
        crate::commands::test_helpers::seed_gate_registry(&executor.storage, &registry);

        // Create issue with precheck gate
        let mut issue =
            crate::test_utils::fixture_issue("Test task".to_string(), "Test".to_string());
        issue.state = State::Ready;
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        executor
            .gates()
            .add_gate(&issue_id, &"tdd-reminder".parse().unwrap())
            .unwrap();

        // Try to claim the issue - should fail because precheck hasn't passed
        let result = executor.issues().claim_issue(
            &IssueIdPrefix::from(&issue_id),
            "agent:test".parse().unwrap(),
        );

        // Currently this test FAILS because claim_issue bypasses prechecks
        // After fix, claiming should fail with "Manual precheck 'tdd-reminder' has not been passed"
        assert!(
            result.is_err(),
            "Claiming should fail when precheck gate hasn't passed"
        );
        assert!(result.unwrap_err().to_string().contains("tdd-reminder"));

        // Verify issue is still ready, not in_progress
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue.state,
            State::Ready,
            "Issue should remain Ready when precheck fails"
        );
        assert!(
            issue.assignee.is_none(),
            "Issue should not be assigned when precheck fails"
        );
    }

    #[test]
    fn test_claim_issue_succeeds_when_prechecks_pass() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        // Define a manual precheck gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "tdd-reminder".parse().unwrap(),
            GateDefinition {
                version: 1,
                key: "tdd-reminder".parse().unwrap(),
                title: "TDD Reminder".to_string(),
                description: "Write tests first".to_string(),
                stage: GateStage::Precheck,
                mode: GateMode::Manual,
                checker: None,
                priority: 100,
                reserved: HashMap::new(),
                inputs: None,
            },
        );
        crate::commands::test_helpers::seed_gate_registry(&executor.storage, &registry);

        // Create issue with precheck gate
        let mut issue =
            crate::test_utils::fixture_issue("Test task".to_string(), "Test".to_string());
        issue.state = State::Ready;
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        executor
            .gates()
            .add_gate(&issue_id, &"tdd-reminder".parse().unwrap())
            .unwrap();

        // Pass the precheck manually
        executor
            .gates()
            .pass_gate(
                &issue_id,
                &"tdd-reminder".parse().unwrap(),
                Some("human:dev".parse().unwrap()),
                false,
            )
            .unwrap();

        // Now claiming should succeed
        let result = executor.issues().claim_issue(
            &IssueIdPrefix::from(&issue_id),
            "agent:test".parse().unwrap(),
        );
        assert!(
            result.is_ok(),
            "Claiming should succeed when precheck passes"
        );

        // Verify issue transitioned to in_progress and is assigned
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::InProgress);
        assert_eq!(issue.assignee, Some("agent:test".parse().unwrap()));
    }

    /// REQ-01: an issue assigned to X while its dependency was still open
    /// (leaving the issue Backlog and blocked) must be claimable by that SAME
    /// assignee once the dependency reaches a terminal state, instead of
    /// hard-failing with "already assigned". This is the assign-then-claim
    /// promotion sequence container stewardship produces.
    #[test]
    fn test_claim_promotes_existing_same_assignee_assignment_once_unblocked() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let dependency = crate::test_utils::fixture_issue("Dependency".to_string(), "".to_string());
        let dependency_id = dependency.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, dependency);

        let mut dependent =
            crate::test_utils::fixture_issue("Dependent".to_string(), "".to_string());
        dependent.dependencies.push(dependency_id.clone());
        let dependent_id = dependent.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, dependent);

        // Assign while the dependency is still open. The issue remains Backlog
        // (a full claim here would exit 4 on the blocked in_progress transition).
        executor
            .issues()
            .assign_issue(
                &IssueIdPrefix::from(&dependent_id),
                "agent:test".parse().unwrap(),
            )
            .unwrap();
        let assigned = executor.storage.load_issue(&dependent_id).unwrap();
        assert_eq!(assigned.state, State::Backlog);
        assert_eq!(assigned.assignee, Some("agent:test".parse().unwrap()));

        // Complete the dependency: this auto-promotes the dependent to Ready.
        executor
            .issues()
            .update_issue(
                &dependency_id,
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();
        let promoted = executor.storage.load_issue(&dependent_id).unwrap();
        assert_eq!(
            promoted.state,
            State::Ready,
            "dependent should auto-promote to Ready once its dependency reaches a terminal state"
        );

        // Claim as the SAME assignee: must proceed rather than erroring.
        let result = executor.issues().claim_issue(
            &IssueIdPrefix::from(&dependent_id),
            "agent:test".parse().unwrap(),
        );
        assert!(
            result.is_ok(),
            "same-assignee claim should promote the assignment, got: {:?}",
            result.err()
        );

        let claimed = executor.storage.load_issue(&dependent_id).unwrap();
        assert_eq!(claimed.state, State::InProgress);
        assert_eq!(
            claimed.assignee,
            Some("agent:test".parse().unwrap()),
            "assignee should remain a live claim after the promotion"
        );
    }

    /// REQ-02: claiming an issue already assigned to a DIFFERENT assignee must
    /// still hard-fail exactly as before the idempotency fix. The message must
    /// also name the current holder and state that re-claiming as that same
    /// holder succeeds (jit:30a3b5c1 error message polish).
    #[test]
    fn test_claim_rejects_different_assignee_when_already_assigned() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let mut issue = crate::test_utils::fixture_issue("Task".to_string(), "".to_string());
        issue.state = State::Ready;
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);

        executor
            .issues()
            .claim_issue(
                &IssueIdPrefix::from(&issue_id),
                "agent:first".parse().unwrap(),
            )
            .unwrap();

        let result = executor.issues().claim_issue(
            &IssueIdPrefix::from(&issue_id),
            "agent:second".parse().unwrap(),
        );
        assert!(
            result.is_err(),
            "claim by a different assignee must still be rejected"
        );
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("already assigned"));
        assert!(
            msg.contains("agent:first"),
            "must name the current holder, got: {msg}"
        );
        assert!(
            msg.contains("re-claiming as agent:first succeeds"),
            "must state that re-claiming as the same holder succeeds, got: {msg}"
        );

        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue.state,
            State::InProgress,
            "rejected claim must not disturb the existing in_progress state"
        );
        assert_eq!(
            issue.assignee,
            Some("agent:first".parse().unwrap()),
            "rejected claim must not disturb the original assignee"
        );
    }

    #[test]
    fn test_captured_graph_blocked_claim_is_typed_and_does_not_assign() {
        let storage = InMemoryStorage::new();
        storage.add_data_file("config.toml", "[worktree]\nenforce_leases = \"off\"\n");
        storage.add_data_file(
            "rules.toml",
            r#"
[[rules]]
name = "epic-claim-needs-design"
when = { type = "epic", state = "in_progress" }
severity = "error"
enforce = true
assert = { dependency-shape = { target = { type = "design" }, mode = "must" } }
"#,
        );
        let executor = crate::commands::test_helpers::memory_executor(storage);
        let mut issue = crate::test_utils::fixture_issue("Epic".to_string(), String::new());
        issue.state = State::Ready;
        issue.labels = vec!["type:epic".to_string()];
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);

        let error = executor
            .publish_captured_claim(&issue_id, "agent:test".parse().unwrap())
            .expect_err("the graph rule must block the claim");
        assert!(
            error
                .downcast_ref::<crate::errors::TransitionBlockedError>()
                .is_some(),
            "expected typed transition block, got: {error:#}"
        );
        let after = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(after.state, State::Ready);
        assert_eq!(after.assignee, None);
        let event_types = executor
            .storage
            .read_events()
            .unwrap()
            .into_iter()
            .map(|event| event.get_type().to_string())
            .collect::<Vec<_>>();
        assert_eq!(event_types, vec!["transition_blocked"]);
    }

    #[test]
    fn test_captured_same_assignee_in_progress_reclaim_is_exact_noop() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let mut issue = crate::test_utils::fixture_issue("Task".to_string(), String::new());
        issue.state = State::InProgress;
        issue.assignee = Some("agent:test".parse().unwrap());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        let before = executor.storage.load_issue(&issue_id).unwrap();
        let events_before = executor.storage.read_events().unwrap();

        let outcome = executor
            .publish_captured_lifecycle_mutation(CapturedLifecycleMutation::Claim {
                issue_id: issue_id.clone(),
                assignee: "agent:test".parse().unwrap(),
            })
            .unwrap();
        assert!(!outcome.changed);
        assert_eq!(executor.storage.load_issue(&issue_id).unwrap(), before);
        assert_eq!(executor.storage.read_events().unwrap(), events_before);
    }

    #[test]
    fn test_claim_rechecks_ready_precheck_classification_after_preflight_race() {
        let storage = InMemoryStorage::new();
        storage.add_data_file("config.toml", "[worktree]\nenforce_leases = \"off\"\n");
        let mut registry = storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "manual-start".parse().unwrap(),
            GateDefinition {
                version: 1,
                key: "manual-start".parse().unwrap(),
                title: "Manual start".to_string(),
                description: String::new(),
                stage: GateStage::Precheck,
                mode: GateMode::Manual,
                checker: None,
                priority: 1,
                reserved: HashMap::new(),
                inputs: None,
            },
        );
        crate::commands::test_helpers::seed_gate_registry(&storage, &registry);
        let mut issue = crate::test_utils::fixture_issue("Race".to_string(), String::new());
        issue.gates_required.push("manual-start".parse().unwrap());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue.clone());
        issue.state = State::Ready;
        let storage = crate::commands::test_helpers::with_open_race(
            storage,
            crate::storage::MutationPoint::SessionOpen,
            2,
            crate::commands::test_helpers::SessionRaceAction::Save(Box::new(issue)),
        );
        let executor = crate::commands::test_helpers::memory_executor(storage);

        let error = executor
            .publish_captured_claim(&issue_id, "agent:test".parse().unwrap())
            .expect_err("the newly required manual precheck must block the claim");

        assert!(error
            .downcast_ref::<crate::errors::TransitionBlockedError>()
            .is_some());
        let after = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(after.state, State::Ready);
        assert_eq!(after.assignee, None);
        assert!(executor.storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_rejected_state_bypasses_gates() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        // Create issue with gates
        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);

        // Add gates that haven't been passed
        executor
            .gates()
            .add_gate(&issue_id, &"tests".parse().unwrap())
            .unwrap();
        executor
            .gates()
            .add_gate(&issue_id, &"code-review".parse().unwrap())
            .unwrap();

        // Transition to Rejected should succeed without passing gates
        let result = crate::test_utils::transition_issue(&executor, &issue_id, State::Rejected);
        assert!(
            result.is_ok(),
            "Rejected state should bypass gate validation"
        );

        // Verify state is Rejected
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Rejected);
    }

    #[test]
    fn test_rejected_state_bypasses_dependencies() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        // Create dependency
        let mut dep = crate::test_utils::fixture_issue("Dependency".to_string(), "Dep".to_string());
        dep.state = State::InProgress; // Not done
        let dep_id = dep.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, dep);

        // Create issue that depends on it
        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        issue.dependencies.push(dep_id.clone());
        issue.state = State::Backlog; // Blocked
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);

        // Transition to Rejected should succeed even with unmet dependencies
        let result = crate::test_utils::transition_issue(&executor, &issue_id, State::Rejected);
        assert!(
            result.is_ok(),
            "Rejected state should bypass dependency checks"
        );

        // Verify state is Rejected
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Rejected);
    }

    #[test]
    fn test_done_state_still_enforces_gates() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        // Create issue with unpassed gates
        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        executor
            .gates()
            .add_gate(&issue_id, &"tests".parse().unwrap())
            .unwrap();

        // Transition to Done should fail (gates not passed)
        let result = crate::test_utils::transition_issue(&executor, &issue_id, State::Done);
        assert!(
            result.is_err(),
            "Done state should still enforce gate validation"
        );

        // Verify state transitioned to Gated (not Done)
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(issue.state, State::Gated);
    }

    #[test]
    fn test_retry_done_on_gated_issue_is_event_log_noop() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        // Issue in progress with an unpassed gate.
        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        executor
            .gates()
            .add_gate(&issue_id, &"tests".parse().unwrap())
            .unwrap();

        // First `--state done`: a genuine in_progress -> gated transition, which
        // should persist and emit exactly one state-change event.
        let first = crate::test_utils::transition_issue(&executor, &issue_id, State::Done);
        assert!(first.is_err(), "unpassed gates should block done");
        let gated = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(gated.state, State::Gated);

        let events_after_first = executor.storage.read_events().unwrap().len();
        let updated_after_first = gated.updated_at;

        // Retry `--state done` on the already-gated issue. It must still report
        // the blocking gates, but must NOT append another state-change event or
        // bump updated_at (the no-op gated -> gated audit-log corruption).
        let retry = crate::test_utils::transition_issue(&executor, &issue_id, State::Done);
        assert!(retry.is_err(), "retry should still report blocking gates");

        let after = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(after.state, State::Gated);
        assert_eq!(
            executor.storage.read_events().unwrap().len(),
            events_after_first,
            "retrying done on a gated issue must not append a state-change event"
        );
        assert_eq!(
            after.updated_at, updated_after_first,
            "retrying done on a gated issue must not bump updated_at"
        );
    }

    #[test]
    fn test_update_issue_done_request_on_done_issue_succeeds_as_noop() {
        let executor =
            memory_executor_declaring(InMemoryStorage::new(), MemoryDeclarations::default());
        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::Done;
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);

        executor
            .issues()
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();

        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().state,
            State::Done
        );
        assert!(executor.storage.read_events().unwrap().is_empty());
    }

    /// The CLI path `jit issue update <id> --state done` goes through
    /// `update_issue`. A pure retry (no field edits) on an already-gated issue
    /// must be a no-op for the audit log and timestamp.
    #[test]
    fn test_update_issue_pure_done_retry_on_gated_is_noop() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        executor
            .gates()
            .add_gate(&issue_id, &"tests".parse().unwrap())
            .unwrap();

        // First done attempt: genuine transition to Gated.
        assert!(executor
            .issues()
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false
            )
            .is_err());
        let gated = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(gated.state, State::Gated);
        let events_after_first = executor.storage.read_events().unwrap().len();
        let updated_after_first = gated.updated_at;

        // Pure retry via update_issue (all fields None) must not save or log.
        assert!(executor
            .issues()
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false
            )
            .is_err());
        let after = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(after.state, State::Gated);
        assert_eq!(
            executor.storage.read_events().unwrap().len(),
            events_after_first,
            "pure done retry must not append an event"
        );
        assert_eq!(
            after.updated_at, updated_after_first,
            "pure done retry must not bump updated_at"
        );
    }

    /// `content_format` update is tri-state: `None` leaves it unchanged,
    /// `Some(Some(fmt))` sets the override, and `Some(None)` clears it back to
    /// repo-default inheritance. The CLI maps `--content-format inherit` to the
    /// clear case — this is the only path back to `None` after an override was set.
    #[test]
    fn test_update_issue_content_format_tristate_set_keep_clear() {
        use crate::domain::ContentFormat;
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Body".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);

        // Set the override to Html.
        executor
            .issues()
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                None,
                vec![],
                vec![],
                Some(Some(ContentFormat::Html)),
                None,
                false,
            )
            .unwrap();
        assert_eq!(
            executor
                .storage
                .load_issue(&issue_id)
                .unwrap()
                .content_format,
            Some(ContentFormat::Html)
        );

        // `None` leaves it unchanged (e.g. a title-only edit).
        executor
            .issues()
            .update_issue(
                &issue_id,
                Some("Renamed".to_string()),
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();
        assert_eq!(
            executor
                .storage
                .load_issue(&issue_id)
                .unwrap()
                .content_format,
            Some(ContentFormat::Html),
            "None must not touch content_format"
        );

        // `Some(None)` clears back to inherit.
        executor
            .issues()
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                None,
                vec![],
                vec![],
                Some(None),
                None,
                false,
            )
            .unwrap();
        assert_eq!(
            executor
                .storage
                .load_issue(&issue_id)
                .unwrap()
                .content_format,
            None,
            "Some(None) must clear the override to repo-default inheritance"
        );
    }

    /// A gate-blocked `--state done` that *also* carries field edits must still
    /// persist those edits (they must not be silently dropped), while not
    /// logging a spurious `gated -> gated` state-change event.
    #[test]
    fn test_update_issue_field_edits_persist_when_done_blocked_on_gated() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let mut issue =
            crate::test_utils::fixture_issue("Old Title".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        executor
            .gates()
            .add_gate(&issue_id, &"tests".parse().unwrap())
            .unwrap();

        // Move to Gated via a first blocked done attempt.
        assert!(executor
            .issues()
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false
            )
            .is_err());
        let events_after_first = executor.storage.read_events().unwrap().len();

        // Retry done together with a title edit on the already-gated issue.
        let result = executor.issues().update_issue(
            &issue_id,
            Some("New Title".to_string()),
            None,
            None,
            Some(State::Done),
            vec![],
            vec![],
            None,
            None,
            false,
        );
        assert!(result.is_err(), "gates should still block done");

        let after = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(after.state, State::Gated);
        assert_eq!(
            after.title, "New Title",
            "field edits must persist even when done is gate-blocked"
        );
        // The persisted title edit is event-logged (no spurious gated -> gated
        // state-change event, but the field edit itself must be recorded).
        let events = executor.storage.read_events().unwrap();
        assert_eq!(
            events.len(),
            events_after_first + 1,
            "the persisted field edit must append exactly one issue_updated event"
        );
        let logged = events
            .iter()
            .find(|e| {
                e.get_type() == "issue_updated"
                    && e.issue_id().map(|id| id.as_str()) == Some(issue_id.as_str())
            })
            .expect("a gate-blocked field edit must log an issue_updated event");
        match logged.body() {
            EventBody::IssueUpdated { fields, .. } => assert!(
                fields.iter().any(|f| f == "title"),
                "issue_updated must record the changed field, got: {fields:?}"
            ),
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    /// Providing an edit flag whose value matches the current value (e.g.
    /// `--title <same>`) is a semantic no-op and must not bump `updated_at` or
    /// emit an event on a gate-blocked `--state done` retry.
    #[test]
    fn test_update_issue_idempotent_field_on_gated_retry_is_noop() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let mut issue =
            crate::test_utils::fixture_issue("Same Title".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        executor
            .gates()
            .add_gate(&issue_id, &"tests".parse().unwrap())
            .unwrap();

        // First blocked done attempt: transition to Gated.
        assert!(executor
            .issues()
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false
            )
            .is_err());
        let gated = executor.storage.load_issue(&issue_id).unwrap();
        let events_after_first = executor.storage.read_events().unwrap().len();
        let updated_after_first = gated.updated_at;

        // Retry with the SAME title value: no real change, so it must be a no-op.
        assert!(executor
            .issues()
            .update_issue(
                &issue_id,
                Some("Same Title".to_string()),
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .is_err());
        let after = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            executor.storage.read_events().unwrap().len(),
            events_after_first,
            "idempotent edit must not append an event"
        );
        assert_eq!(
            after.updated_at, updated_after_first,
            "idempotent edit must not bump updated_at"
        );
    }

    #[test]
    fn test_assign_same_assignee_is_noop() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);

        executor
            .issues()
            .assign_issue(&IssueIdPrefix::from(&issue_id), "agent:a".parse().unwrap())
            .unwrap();
        let updated_after_assign = executor.storage.load_issue(&issue_id).unwrap().updated_at;

        // Re-assigning the same assignee is a no-op.
        executor
            .issues()
            .assign_issue(&IssueIdPrefix::from(&issue_id), "agent:a".parse().unwrap())
            .unwrap();
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().updated_at,
            updated_after_assign,
            "re-assigning the same assignee must not bump updated_at"
        );
    }

    #[test]
    fn test_unassign_already_unassigned_is_noop() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        let updated_before = executor.storage.load_issue(&issue_id).unwrap().updated_at;

        // Unassigning an issue that has no assignee is a no-op.
        executor
            .issues()
            .unassign_issue(&IssueIdPrefix::from(&issue_id))
            .unwrap();
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().updated_at,
            updated_before,
            "unassigning an already-unassigned issue must not bump updated_at"
        );
    }

    #[test]
    fn test_update_issue_with_no_changes_is_noop() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::InProgress;
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        let updated_before = executor.storage.load_issue(&issue_id).unwrap().updated_at;
        let events_before = executor.storage.read_events().unwrap().len();

        // No fields and no state change: pure no-op.
        executor
            .issues()
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();
        let after = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            after.updated_at, updated_before,
            "a no-change issue update must not bump updated_at"
        );
        assert_eq!(
            executor.storage.read_events().unwrap().len(),
            events_before,
            "a no-change issue update must not append events"
        );
    }

    #[test]
    fn test_update_issue_description_logs_issue_updated_event() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let issue = crate::test_utils::fixture_issue("Test".to_string(), "old".to_string());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        let events_before = executor.storage.read_events().unwrap().len();

        // A real description edit must be captured in the event log, not only
        // state transitions (mirrors the bulk-update event-logging contract).
        executor
            .issues()
            .update_issue(
                &issue_id,
                None,
                Some(DescriptionUpdate::Replace("new description".to_string())),
                None,
                None,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();

        let events = executor.storage.read_events().unwrap();
        assert_eq!(
            events.len(),
            events_before + 1,
            "a description edit must append exactly one event"
        );
        let logged = events
            .iter()
            .find(|e| {
                e.get_type() == "issue_updated"
                    && e.issue_id().map(|id| id.as_str()) == Some(issue_id.as_str())
            })
            .expect("description edit must log an issue_updated event");
        match logged.body() {
            EventBody::IssueUpdated { fields, .. } => assert!(
                fields.iter().any(|f| f == "description"),
                "issue_updated must record the changed field, got: {fields:?}"
            ),
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    #[test]
    fn test_confirm_deletion_allowed_refuses_when_not_allowed() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let result = executor
            .issues()
            .confirm_deletion_allowed(&crate::test_utils::issue_id(0xabc1_2345), false);

        let err = result.expect_err("must refuse without confirmation");
        assert!(err.message().contains("JIT_ALLOW_DELETION=1"));
        assert!(err.message().contains("abc12345"));
    }

    #[test]
    fn test_confirm_deletion_allowed_ok_when_allowed() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        assert!(executor
            .issues()
            .confirm_deletion_allowed(&crate::test_utils::issue_id(0xabc1_2345), true)
            .is_ok());
    }

    #[test]
    fn test_delete_issue_logs_issue_deleted_event() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let issue = crate::test_utils::fixture_issue("Doomed".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        let events_before = executor.storage.read_events().unwrap().len();

        // Deletion is a state change and must be captured in the event log.
        executor
            .issues()
            .delete_issue(&IssueIdPrefix::from(&issue_id), true)
            .unwrap();

        let events = executor.storage.read_events().unwrap();
        assert_eq!(
            events.len(),
            events_before + 1,
            "a deletion must append exactly one event"
        );
        let logged = events
            .iter()
            .find(|e| {
                e.get_type() == "issue_deleted"
                    && e.issue_id().map(|id| id.as_str()) == Some(issue_id.as_str())
            })
            .expect("deletion must log an issue_deleted event");
        assert!(matches!(logged.body(), EventBody::IssueDeleted { .. }));
    }

    #[test]
    fn test_delete_issue_cascade_preserves_promotion_event_order() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let mut dependency = crate::test_utils::fixture_issue("Dependency".into(), String::new());
        dependency.state = State::Ready;
        let dependency_id = dependency.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, dependency);
        let dependent_ids = [
            crate::test_utils::issue_id(1),
            crate::test_utils::issue_id(2),
        ];
        for id in dependent_ids.iter().rev() {
            let mut dependent = crate::test_utils::fixture_issue("Dependent".into(), String::new());
            dependent.id = id.clone();
            dependent.state = State::Backlog;
            dependent.dependencies = vec![dependency_id.clone()];
            crate::commands::test_helpers::seed_issue(&executor.storage, dependent);
        }
        let mut unrelated = crate::test_utils::fixture_issue("Unrelated".into(), String::new());
        unrelated.state = State::Backlog;
        crate::commands::test_helpers::seed_issue(&executor.storage, unrelated);
        let events_before = executor.storage.read_events().unwrap().len();

        executor
            .issues()
            .delete_issue(&IssueIdPrefix::from(&dependency_id), true)
            .unwrap();

        let events = executor.storage.read_events().unwrap();
        let bodies = events[events_before..]
            .iter()
            .map(|event| event.body().clone())
            .collect::<Vec<_>>();
        let expected = std::iter::once(EventBody::draft_issue_deleted(dependency_id))
            .chain(dependent_ids.iter().map(|id| {
                EventBody::draft_issue_state_changed(id.clone(), State::Backlog, State::Ready)
            }))
            .chain(dependent_ids.iter().map(|id| {
                EventBody::draft_issue_updated(
                    id.clone(),
                    "dependency-cascade-delete".into(),
                    vec!["dependencies".into()],
                )
            }))
            .collect::<Vec<_>>();
        assert_eq!(bodies, expected);
    }

    #[test]
    fn test_delete_issue_cascade_auto_ready_commits_one_timestamp() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let mut dependency =
            crate::test_utils::fixture_issue("Dependency".to_string(), String::new());
        dependency.state = State::Ready;
        let dependency_id = dependency.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, dependency);
        let mut dependent =
            crate::test_utils::fixture_issue("Dependent".to_string(), String::new());
        dependent.state = State::Backlog;
        dependent.dependencies = vec![dependency_id.clone()];
        let dependent_id = dependent.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, dependent);

        executor
            .issues()
            .delete_issue(&IssueIdPrefix::from(&dependency_id), true)
            .unwrap();

        let updated = executor.storage.load_issue(&dependent_id).unwrap();
        assert!(updated.dependencies.is_empty());
        assert_eq!(updated.state, State::Ready);
        assert!(executor.storage.load_issue(&dependency_id).is_err());
        let index: serde_json::Value = serde_json::from_str(
            &executor
                .storage
                .read_repo_file(".jit/index.json")
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(index["deleted_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id.as_str() == Some(dependency_id.as_str())));
        let events = executor.storage.read_events().unwrap();
        let mutation_events = events
            .iter()
            .filter(|event| {
                event.issue_id().map(|id| id.as_str()) == Some(dependency_id.as_str())
                    || event.issue_id().map(|id| id.as_str()) == Some(dependent_id.as_str())
            })
            .collect::<Vec<_>>();
        assert_eq!(mutation_events.len(), 3);
        assert!(mutation_events
            .iter()
            .any(|event| event.get_type() == "issue_deleted"));
        assert!(mutation_events
            .iter()
            .any(|event| event.get_type() == "issue_updated"));
        assert!(mutation_events
            .iter()
            .any(|event| event.get_type() == "issue_state_changed"));
        let timestamps = mutation_events
            .iter()
            .map(|event| serde_json::to_value(event).unwrap()["timestamp"].clone())
            .collect::<Vec<_>>();
        assert!(timestamps.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn test_delete_issue_missing_index_changes_nothing() {
        let storage = InMemoryStorage::new();
        storage.add_data_file("config.toml", "[worktree]\nenforce_leases = \"off\"\n");
        let issue = crate::test_utils::fixture_issue("Doomed".into(), String::new());
        let record = crate::repository_state::VirtualPath::issue_record(&issue.id);
        let issue_relative = record.relative().as_str();
        let issue_path = record.repository_relative();
        let issue_bytes = serde_json::to_string_pretty(&issue).unwrap();
        storage.add_data_file(issue_relative, &issue_bytes);
        let executor = crate::commands::test_helpers::memory_executor(storage.clone());
        storage
            .repository_state()
            .entries
            .remove(&crate::repository_state::VirtualPath::INDEX);

        assert!(executor
            .issues()
            .delete_issue(&IssueIdPrefix::from(&issue.id), true)
            .is_err());
        assert_eq!(
            storage.read_repo_file(&issue_path).unwrap(),
            Some(issue_bytes)
        );
        assert!(storage.read_repo_file(".jit/index.json").unwrap().is_none());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_delete_issue_malformed_index_changes_nothing() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let issue = crate::test_utils::fixture_issue("Doomed".into(), String::new());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        executor
            .storage
            .add_data_file("index.json", "not valid JSON");
        let issue_path =
            crate::repository_state::VirtualPath::issue_record(&issue_id).repository_relative();
        let issue_before = executor.storage.read_repo_file(&issue_path).unwrap();

        assert!(executor
            .issues()
            .delete_issue(&IssueIdPrefix::from(&issue_id), true)
            .is_err());
        assert_eq!(
            executor.storage.read_repo_file(".jit/index.json").unwrap(),
            Some("not valid JSON".to_string())
        );
        assert_eq!(
            executor.storage.read_repo_file(&issue_path).unwrap(),
            issue_before
        );
        assert!(executor.storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_delete_issue_retries_when_lease_mode_changes_after_preflight() {
        let storage = InMemoryStorage::new();
        storage.add_data_file("config.toml", "[worktree]\nenforce_leases = \"off\"\n");
        let issue = crate::test_utils::fixture_issue("Doomed".into(), String::new());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);
        let storage = crate::commands::test_helpers::with_open_race(
            storage,
            crate::storage::MutationPoint::SessionOpen,
            2,
            crate::commands::test_helpers::SessionRaceAction::WriteRepoFile {
                path: ".jit/config.toml".to_string(),
                content: "[worktree]\nenforce_leases = \"warn\"\n".to_string(),
            },
        );
        let executor = crate::commands::test_helpers::memory_executor(storage);

        executor
            .issues()
            .delete_issue(&IssueIdPrefix::from(&issue_id), true)
            .unwrap();

        assert!(executor.storage.load_issue(&issue_id).is_err());
        assert_eq!(
            executor
                .storage
                .read_events()
                .unwrap()
                .iter()
                .filter(|event| event.get_type() == "issue_deleted")
                .count(),
            1
        );
    }

    #[test]
    fn test_delete_issue_keeps_resolved_target_after_a_new_issue_appears() {
        let storage = InMemoryStorage::new();
        storage.add_data_file("config.toml", "[worktree]\nenforce_leases = \"off\"\n");
        let mut target = crate::test_utils::fixture_issue("Target".into(), String::new());
        target.id = crate::test_utils::issue_id(0x1111_1111_1111_1111_1111_1111_2222_1111);
        crate::commands::test_helpers::seed_issue(&storage, target.clone());
        let mut collision = crate::test_utils::fixture_issue("Collision".into(), String::new());
        collision.id = crate::test_utils::issue_id(0x2222_2222_2222_2222_2222_2222_2222_2222);
        let storage = crate::commands::test_helpers::with_open_race(
            storage,
            crate::storage::MutationPoint::SessionOpen,
            2,
            crate::commands::test_helpers::SessionRaceAction::Save(Box::new(collision.clone())),
        );
        let executor = crate::commands::test_helpers::memory_executor(storage);

        executor
            .issues()
            .delete_issue(&IssueIdPrefix::from(&target.id), true)
            .unwrap();

        assert!(executor.storage.load_issue(&target.id).is_err());
        assert!(executor.storage.load_issue(&collision.id).is_ok());
        assert_eq!(executor.storage.read_events().unwrap().len(), 1);
    }

    #[test]
    fn test_delete_issue_atomic_failure_recovers_old_graph() {
        use crate::storage::RepositoryStateStore;

        let failures = std::sync::Arc::new(PointFailure(std::sync::Mutex::new(Some(
            MutationPoint::BeforeCommitCheck,
        ))));
        let storage = InMemoryStorage::with_mutation_interruptions(failures);
        storage.add_data_file("config.toml", "[worktree]\nenforce_leases = \"off\"\n");
        let recovered = storage.without_mutation_interruptions();
        let layout = storage.repository_layout();
        let dependency = crate::test_utils::fixture_issue("Dependency".to_string(), String::new());
        let dependency_id = dependency.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, dependency);
        let mut dependent =
            crate::test_utils::fixture_issue("Dependent".to_string(), String::new());
        dependent.state = State::Backlog;
        dependent.dependencies = vec![dependency_id.clone()];
        let dependent_id = dependent.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, dependent);
        let executor = CommandExecutor::new(storage)
            .with_layout(layout.clone())
            .unwrap();

        assert!(executor
            .issues()
            .delete_issue(&IssueIdPrefix::from(&dependency_id), true)
            .is_err());
        drop(recovered.open_mutation_session(layout.clone()).unwrap());

        let unchanged = recovered.load_issue(&dependent_id).unwrap();
        assert_eq!(unchanged.state, State::Backlog);
        assert_eq!(unchanged.dependencies, vec![dependency_id.clone()]);
        assert!(recovered.load_issue(&dependency_id).is_ok());
        assert!(recovered.read_events().unwrap().is_empty());

        CommandExecutor::new(recovered.clone())
            .with_layout(layout)
            .unwrap()
            .issues()
            .delete_issue(&IssueIdPrefix::from(&dependency_id), true)
            .unwrap();
        let updated = recovered.load_issue(&dependent_id).unwrap();
        assert_eq!(updated.state, State::Ready);
        assert!(updated.dependencies.is_empty());
        assert!(recovered.load_issue(&dependency_id).is_err());
    }

    #[test]
    fn test_add_dependency_logs_issue_updated_event() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let create = |title: &str| {
            executor
                .issues()
                .create_issue(
                    title.to_string(),
                    "Test".to_string(),
                    crate::domain::Priority::Normal,
                    vec![],
                    vec![],
                    None,
                    None,
                    false,
                )
                .unwrap()
                .issue
                .id
        };
        let a_id = create("A");
        let b_id = create("B");

        let events_before = executor.storage.read_events().unwrap().len();
        add_reducing_edge(&executor, &a_id, &b_id).unwrap();

        // Adding a dependency edits `A`; the change must be event-logged.
        let logged = executor
            .storage
            .read_events()
            .unwrap()
            .into_iter()
            .skip(events_before)
            .find(|e| {
                e.get_type() == "issue_updated"
                    && e.issue_id().map(|id| id.as_str()) == Some(a_id.as_str())
            })
            .expect("dependency addition must log an issue_updated event");
        match logged.body() {
            EventBody::IssueUpdated { fields, .. } => assert!(
                fields.iter().any(|f| f == "dependencies"),
                "issue_updated must record the dependencies edit, got: {fields:?}"
            ),
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    #[test]
    fn test_remove_dependency_logs_issue_updated_event() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let create = |title: &str| {
            executor
                .issues()
                .create_issue(
                    title.to_string(),
                    "Test".to_string(),
                    crate::domain::Priority::Normal,
                    vec![],
                    vec![],
                    None,
                    None,
                    false,
                )
                .unwrap()
                .issue
                .id
        };
        let a_id = create("A");
        let b_id = create("B");
        add_reducing_edge(&executor, &a_id, &b_id).unwrap();

        // Snapshot AFTER the add so we isolate the removal's event.
        let events_before = executor.storage.read_events().unwrap().len();
        executor.graph().remove_dependency(&a_id, &b_id).unwrap();

        // Removing a dependency edits `A`; the change must be event-logged.
        let logged = executor
            .storage
            .read_events()
            .unwrap()
            .into_iter()
            .skip(events_before)
            .find(|e| {
                e.get_type() == "issue_updated"
                    && e.issue_id().map(|id| id.as_str()) == Some(a_id.as_str())
            })
            .expect("dependency removal must log an issue_updated event");
        match logged.body() {
            EventBody::IssueUpdated { fields, .. } => assert!(
                fields.iter().any(|f| f == "dependencies"),
                "issue_updated must record the dependencies edit, got: {fields:?}"
            ),
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    #[test]
    fn test_remove_dependencies_logs_issue_updated_event() {
        // The CLI `jit dep rm` path goes through the plural `remove_dependencies`,
        // which must also event-log the edit.
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let a = crate::test_utils::fixture_issue("A".to_string(), "Test".to_string());
        let a_id = a.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, a);
        let b = crate::test_utils::fixture_issue("B".to_string(), "Test".to_string());
        let b_id = b.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, b);
        add_reducing_edge(&executor, &a_id, &b_id).unwrap();

        let events_before = executor.storage.read_events().unwrap().len();
        executor
            .graph()
            .remove_dependencies(&a_id, std::slice::from_ref(&b_id))
            .unwrap();

        let logged = executor
            .storage
            .read_events()
            .unwrap()
            .into_iter()
            .skip(events_before)
            .find(|e| {
                e.get_type() == "issue_updated"
                    && e.issue_id().map(|id| id.as_str()) == Some(a_id.as_str())
            })
            .expect("CLI dependency removal must log an issue_updated event");
        match logged.body() {
            EventBody::IssueUpdated { fields, .. } => assert!(
                fields.iter().any(|f| f == "dependencies"),
                "issue_updated must record the dependencies edit, got: {fields:?}"
            ),
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
    }

    #[test]
    fn test_remove_dependency_noop_does_not_save_or_log() {
        // Removing an edge that is not present must not persist (no `updated_at`
        // bump) and must not append an event, both for the single and bulk paths.
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let a = crate::test_utils::fixture_issue("A".to_string(), "Test".to_string());
        let a_id = a.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, a);
        // B exists but A does NOT depend on it, so removing B from A is a no-op.
        let b = crate::test_utils::fixture_issue("B".to_string(), "Test".to_string());
        let b_id = b.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, b);

        let updated_before = executor.storage.load_issue(&a_id).unwrap().updated_at;
        let events_before = executor.storage.read_events().unwrap().len();

        executor.graph().remove_dependency(&a_id, &b_id).unwrap();
        executor
            .graph()
            .remove_dependencies(&a_id, std::slice::from_ref(&b_id))
            .unwrap();

        assert_eq!(
            executor.storage.load_issue(&a_id).unwrap().updated_at,
            updated_before,
            "a no-op dependency removal must not bump updated_at"
        );
        assert_eq!(
            executor.storage.read_events().unwrap().len(),
            events_before,
            "a no-op dependency removal must not append an event"
        );
    }

    // --- jit:f847df3f: delete must not leave dangling dependency edges -----

    #[test]
    fn test_delete_issue_strips_id_from_dependent_dependencies() {
        // A depends on B. Deleting B must remove B's id from A's stored
        // `dependencies`, not just B's own file — otherwise A is left with a
        // dangling edge that corrupts the graph (jit:f847df3f).
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let a = crate::test_utils::fixture_issue("A".to_string(), "Test".to_string());
        let a_id = a.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, a);
        let b = crate::test_utils::fixture_issue("B".to_string(), "Test".to_string());
        let b_id = b.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, b);
        add_reducing_edge(&executor, &a_id, &b_id).unwrap();
        assert!(executor.storage.load_issue(&a_id).unwrap().dependencies == vec![b_id.clone()]);

        let events_before = executor.storage.read_events().unwrap().len();
        executor
            .issues()
            .delete_issue(&IssueIdPrefix::from(&b_id), true)
            .unwrap();

        let a_after = executor.storage.load_issue(&a_id).unwrap();
        assert!(
            !a_after.dependencies.contains(&b_id),
            "B's id must be gone from A's dependencies after B is deleted, got: {:?}",
            a_after.dependencies
        );

        // The cascade edit is itself event-logged (in addition to the
        // `issue_deleted` event for B), preserving @/inv/event-log.
        let events = executor.storage.read_events().unwrap();
        assert!(
            events.len() > events_before,
            "the delete cascade must append at least the deletion event"
        );
        let cascade_event = events
            .iter()
            .skip(events_before)
            .find(|e| {
                e.get_type() == "issue_updated"
                    && e.issue_id().map(|id| id.as_str()) == Some(a_id.as_str())
            })
            .expect("the cascade cleanup of A's dependencies must be event-logged");
        match cascade_event.body() {
            EventBody::IssueUpdated { fields, .. } => assert!(
                fields.iter().any(|f| f == "dependencies"),
                "cascade event must record the dependencies edit, got: {fields:?}"
            ),
            other => panic!("expected IssueUpdated, got {other:?}"),
        }
        let delete_event = events
            .iter()
            .skip(events_before)
            .find(|e| {
                e.get_type() == "issue_deleted"
                    && e.issue_id().map(|id| id.as_str()) == Some(b_id.as_str())
            })
            .expect("deleting B must still log its own issue_deleted event");
        assert!(matches!(
            delete_event.body(),
            EventBody::IssueDeleted { .. }
        ));
    }

    #[test]
    fn test_delete_issue_then_validate_reports_no_dangling_dependency() {
        // After deleting a dependency through the executor, `jit validate`
        // must not report "depends on ... which does not exist" (jit:f847df3f).
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let a_id = executor
            .issues()
            .create_issue(
                "A".to_string(),
                String::new(),
                crate::domain::Priority::Normal,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap()
            .issue
            .id;
        let b_id = executor
            .issues()
            .create_issue(
                "B".to_string(),
                String::new(),
                crate::domain::Priority::Normal,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap()
            .issue
            .id;
        add_reducing_edge(&executor, &a_id, &b_id).unwrap();
        assert!(
            executor.validation().validate_silent().is_ok(),
            "validation should pass before deletion"
        );

        executor
            .issues()
            .delete_issue(&IssueIdPrefix::from(&b_id), true)
            .unwrap();

        let result = executor.validation().validate_silent();
        assert!(
            result.is_ok(),
            "validate must report no dangling dependency after delete, got: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_remove_dependencies_removes_dangling_edge_by_raw_id() {
        // Reproduces a pre-existing dangling edge the way legacy or externally
        // edited repository data could:
        // A depends on B, then B's file disappears without A's `dependencies`
        // being cleaned up. `jit dep rm` (the plural `remove_dependencies`)
        // must still remove the edge by matching A's raw stored id, even
        // though B can no longer be resolved through the issue files
        // (jit:f847df3f).
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let b_id = crate::test_utils::fixture_issue("B".to_string(), "Test".to_string()).id;
        let mut a = crate::test_utils::fixture_issue("A".to_string(), "Test".to_string());
        a.dependencies.push(b_id.clone());
        let a_id = a.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, a);
        assert!(
            executor.storage.load_issue(&b_id).is_err(),
            "the missing dependency must not resolve, matching the reported bug"
        );

        let result = executor
            .graph()
            .remove_dependencies(&a_id, std::slice::from_ref(&b_id))
            .unwrap();

        assert_eq!(result.removed, vec![b_id.clone()]);
        assert!(
            result.not_found.is_empty(),
            "a dangling edge must be reported as removed, not not_found: {:?}",
            result.not_found
        );
        let issue = executor.storage.load_issue(&a_id).unwrap();
        assert!(
            !issue.dependencies.contains(&b_id),
            "the dangling edge must actually be gone from A's dependencies"
        );
    }

    #[test]
    fn test_get_dependencies_enriched_omits_dangling_id_from_resolved_list() {
        // `get_dependencies_enriched` itself only returns resolvable
        // dependencies (dangling ids are recovered downstream by
        // `IssueShowResponse::from_issue`, see its doc comment and tests in
        // `output.rs`). This pins that contract so the two stay in sync.
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        let dep = crate::test_utils::fixture_issue("Dep".to_string(), "Test".to_string());
        let dep_id = dep.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, dep);

        let mut issue = crate::test_utils::fixture_issue("Parent".to_string(), "Test".to_string());
        issue.dependencies = vec![
            dep_id.clone(),
            crate::test_utils::issue_id(0x7b54_5f2b_8415_68e1_de5a_3bf9_592c_59ad),
        ];

        let resolved = executor.issues().get_dependencies_enriched(&issue);
        assert_eq!(resolved.len(), 1, "only the resolvable dep must appear");
        assert_eq!(resolved[0].id, dep_id);
    }

    /// REQ-02: an explicit `--type` whose kind IS declared in the configured
    /// `[type_hierarchy]` is accepted and the canonical `type:<kind>` label is
    /// written. Acceptance is decided by the rule engine: the
    /// `type-hierarchy-known` rule produces no finding for a declared
    /// kind, the SAME validation layer the write path uses.
    #[test]
    fn test_create_explicit_declared_type_accepted_via_rule_engine() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        // The kind the fixture repository declares at the leaf level, read from
        // the declaration it was configured from.
        let declared = crate::test_taxonomy::test_taxonomy()
            .type_at_level(4)
            .to_string();
        let id = executor
            .issues()
            .create_issue(
                "Declared type".to_string(),
                String::new(),
                crate::domain::Priority::Normal,
                vec![],
                vec![],
                None,
                Some(declared.clone()),
                false,
            )
            .expect("a declared --type kind must be accepted")
            .issue
            .id;
        let issue = executor.storage.load_issue(&id).unwrap();
        assert!(
            issue
                .labels
                .iter()
                .any(|l| l == &format!("type:{declared}")),
            "declared --type must write the canonical label, got: {:?}",
            issue.labels
        );
    }

    /// REQ-02: an explicit `--type` whose kind is NOT declared is hard-rejected
    /// through the existing rule engine (the `type-hierarchy-known`
    /// finding), surfacing as a `ValidationFailedError` (exit 4) and persisting
    /// nothing — NOT via a parallel config containment check.
    #[test]
    fn test_create_explicit_undeclared_type_rejected_via_rule_engine() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let before = executor.storage.list_issues().unwrap().len();
        let err = executor
            .issues()
            .create_issue(
                "Undeclared type".to_string(),
                String::new(),
                crate::domain::Priority::Normal,
                vec![],
                vec![],
                None,
                Some("xyzzy-unknown-type".to_string()),
                false,
            )
            .expect_err("an undeclared --type kind must be rejected");
        assert!(
            err.downcast_ref::<crate::errors::ValidationFailedError>()
                .is_some(),
            "rejection must be a ValidationFailedError, got: {err:?}"
        );
        assert_eq!(
            executor.storage.list_issues().unwrap().len(),
            before,
            "a rejected create must persist nothing"
        );
    }

    /// REQ-02: `issue update --type <declared>` is accepted via the rule engine
    /// and leaves exactly one canonical `type:` label.
    #[test]
    fn test_update_explicit_declared_type_accepted_via_rule_engine() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let declared = crate::test_taxonomy::test_taxonomy()
            .type_at_level(3)
            .to_string();
        let issue = crate::test_utils::fixture_issue("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);

        executor
            .issues()
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                Some(declared.clone()),
                false,
            )
            .expect("a declared --type update must be accepted");
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        let type_labels: Vec<&str> = issue
            .labels
            .iter()
            .filter(|l| l.starts_with("type:"))
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            type_labels,
            vec![format!("type:{declared}")],
            "update --type must leave exactly one canonical type label"
        );
    }

    /// REQ-02: `issue update --type <undeclared>` is hard-rejected through the
    /// rule engine (mirroring create), as a `ValidationFailedError`.
    #[test]
    fn test_update_explicit_undeclared_type_rejected_via_rule_engine() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let issue = crate::test_utils::fixture_issue("T".to_string(), String::new());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);

        let err = executor
            .issues()
            .update_issue(
                &issue_id,
                None,
                None,
                None,
                None,
                vec![],
                vec![],
                None,
                Some("xyzzy-unknown-type".to_string()),
                false,
            )
            .expect_err("update with an undeclared --type must be rejected");
        assert!(
            err.downcast_ref::<crate::errors::ValidationFailedError>()
                .is_some(),
            "rejection must be a ValidationFailedError, got: {err:?}"
        );
    }

    #[test]
    fn test_manual_transition_to_ready_state_actually_changes_state() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );

        // Create dependency issue that is Done
        let mut dep = crate::test_utils::fixture_issue("Dependency".to_string(), "Dep".to_string());
        dep.state = State::Done;
        let dep_id = dep.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, dep);

        // Create issue in Backlog that depends on the Done dependency
        let mut issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        issue.state = State::Backlog;
        issue.dependencies.push(dep_id.clone());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);

        // Manually transition to Ready should succeed (dependency is done)
        let result = executor.issues().update_issue(
            &issue_id,
            None,               // title
            None,               // description
            None,               // priority
            Some(State::Ready), // state
            vec![],             // add_labels
            vec![],             // remove_labels
            None,
            None,
            false, // force
        );

        assert!(
            result.is_ok(),
            "Transition to Ready should succeed when unblocked"
        );

        // BUG: Issue state is NOT updated to Ready, it remains Backlog
        // After fix, this assertion should pass
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue.state,
            State::Ready,
            "Issue state should be Ready after manual transition"
        );
    }

    fn seed_fixture_issue(executor: &CommandExecutor<InMemoryStorage>, issue: Issue) -> IssueId {
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        id
    }

    fn issue_event_types(executor: &CommandExecutor<InMemoryStorage>, id: &IssueId) -> Vec<String> {
        let mut types = executor
            .storage
            .read_events()
            .unwrap()
            .iter()
            .filter(|event| event.issue_id() == Some(id))
            .map(|event| event.get_type().to_string())
            .collect::<Vec<_>>();
        types.sort();
        types
    }

    fn combined_update(
        executor: &CommandExecutor<InMemoryStorage>,
        id: &IssueId,
        local: IssueLocalUpdate,
    ) -> Result<IssueMutation> {
        executor.issues().update_issue_resolved(
            &IssueIdPrefix::from(id),
            None,
            None,
            Some(Priority::High),
            None,
            vec![],
            vec![],
            None,
            None,
            local,
            false,
        )
    }

    #[test]
    fn test_update_issue_resolved_publishes_gate_assignee_and_field_changes_in_one_session() {
        let counter = Arc::new(SessionOpenCounter(AtomicUsize::new(0)));
        let storage = InMemoryStorage::new().with_mutation_interruption_view(counter.clone());
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let id = seed_fixture_issue(
            &executor,
            crate::test_utils::fixture_issue("Combined".into(), String::new()),
        );
        let opened = counter.0.load(Ordering::SeqCst);

        combined_update(
            &executor,
            &id,
            IssueLocalUpdate {
                add_gates: vec!["tests".parse().unwrap()],
                assignee: Some("agent:w1".parse().unwrap()),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(counter.0.load(Ordering::SeqCst) - opened, 1);
        let issue = executor.storage.load_issue(&id).unwrap();
        assert!(issue.gates_required.contains(&"tests".parse().unwrap()));
        assert_eq!(
            issue.assignee.map(|a| a.to_string()),
            Some("agent:w1".into())
        );
        assert_eq!(issue.priority, Priority::High);
    }

    #[test]
    fn test_update_issue_resolved_combined_update_matches_separate_steps() {
        let assign = IssueLocalUpdate {
            add_gates: vec!["tests".parse().unwrap()],
            remove_gates: vec!["code-review".parse().unwrap()],
            assignee: Some("agent:w1".parse().unwrap()),
            unassign: false,
        };
        let unassign = IssueLocalUpdate {
            assignee: None,
            unassign: true,
            ..assign.clone()
        };
        for local in [assign, unassign] {
            let storage = InMemoryStorage::new();
            seed_scenario_gates(&storage);
            let executor = memory_executor_declaring(
                storage,
                MemoryDeclarations {
                    config: Some(&scenario_config()),
                    ..MemoryDeclarations::default()
                },
            );
            let mut template = crate::test_utils::fixture_issue("Twin".into(), String::new());
            template.gates_required = vec!["code-review".parse().unwrap()];
            template.assignee = Some("agent:prior".parse().unwrap());
            let separate = seed_fixture_issue(&executor, template.clone());
            template.id = crate::test_utils::fixture_issue(String::new(), String::new()).id;
            let combined = seed_fixture_issue(&executor, template);

            executor
                .gates()
                .add_gates(&separate, &local.add_gates)
                .unwrap();
            executor
                .gates()
                .remove_gates(&separate, &local.remove_gates)
                .unwrap();
            match &local.assignee {
                Some(assignee) => executor
                    .issues()
                    .assign_issue(&IssueIdPrefix::from(&separate), assignee.clone())
                    .map(drop),
                None => executor
                    .issues()
                    .unassign_issue(&IssueIdPrefix::from(&separate))
                    .map(drop),
            }
            .unwrap();
            executor
                .issues()
                .update_issue(
                    &separate,
                    None,
                    None,
                    Some(Priority::High),
                    None,
                    vec![],
                    vec![],
                    None,
                    None,
                    false,
                )
                .unwrap();
            combined_update(&executor, &combined, local).unwrap();

            let observable = |id: &IssueId| {
                let issue = executor.storage.load_issue(id).unwrap();
                let gates = issue
                    .gates_status
                    .iter()
                    .map(|(key, gate)| (key.clone(), gate.status))
                    .collect::<std::collections::BTreeMap<_, _>>();
                (
                    issue.state,
                    issue.priority,
                    issue.labels,
                    issue.gates_required,
                    gates,
                    issue.assignee,
                )
            };
            assert_eq!(observable(&combined), observable(&separate));
            assert_eq!(
                issue_event_types(&executor, &combined),
                issue_event_types(&executor, &separate)
            );
        }
    }

    #[test]
    fn test_reject_issue_with_reason_publishes_in_one_session() {
        let counter = Arc::new(SessionOpenCounter(AtomicUsize::new(0)));
        let storage = InMemoryStorage::new().with_mutation_interruption_view(counter.clone());
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let id = seed_fixture_issue(
            &executor,
            crate::test_utils::fixture_issue("Reject".into(), String::new()),
        );
        let opened = counter.0.load(Ordering::SeqCst);

        executor
            .issues()
            .reject_issue(&IssueIdPrefix::from(&id), Some("duplicate"))
            .unwrap();

        assert_eq!(counter.0.load(Ordering::SeqCst) - opened, 1);
    }

    #[test]
    fn test_update_issue_restores_archived_done_with_retired_gate() {
        let executor = crate::commands::test_helpers::memory_executor(InMemoryStorage::new());
        let mut issue = crate::test_utils::fixture_issue("Closed".into(), String::new());
        issue.state = State::Archived;
        issue.archived_from = Some(State::Done);
        let key: crate::domain::identity::GateKey = "retired".parse().unwrap();
        issue.gates_required.push(key.clone());
        issue.gates_status.insert(
            key,
            crate::domain::GateState {
                status: crate::domain::GateStatus::Passed,
                updated_by: None,
                updated_at: chrono::Utc::now(),
                run_id: None,
            },
        );
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        executor
            .issues()
            .update_issue(
                &id,
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();
        assert_eq!(executor.storage.load_issue(&id).unwrap().state, State::Done);
    }

    #[test]
    fn test_update_issue_reopening_with_retired_gate_fails_without_mutation() {
        let executor = crate::commands::test_helpers::memory_executor(InMemoryStorage::new());
        let mut issue = crate::test_utils::fixture_issue("Closed".into(), String::new());
        issue.state = State::Done;
        issue.gates_required.push("retired".parse().unwrap());
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, issue);
        let error = executor
            .issues()
            .update_issue(
                &id,
                None,
                None,
                None,
                Some(State::InProgress),
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap_err();
        assert!(error
            .downcast_ref::<crate::storage::GateNotFoundError>()
            .is_some());
        assert_eq!(executor.storage.load_issue(&id).unwrap().state, State::Done);
    }

    /// Move a `Done` blocker back to `InProgress`, optionally with a `Ready`
    /// dependent; returns the session opens the update took and the
    /// dependent's state.
    fn reopen_done_blocker(with_dependent: bool) -> (usize, Option<State>) {
        let counter = crate::commands::test_helpers::SessionOpenCounter::new();
        let executor = crate::commands::test_helpers::memory_executor(
            InMemoryStorage::new().with_mutation_interruption_view(counter.clone()),
        );
        let mut blocker = crate::test_utils::fixture_issue("Blocker".into(), String::new());
        blocker.state = State::Done;
        let blocker_id = blocker.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, blocker);
        let dependent_id = with_dependent.then(|| {
            let mut dependent = crate::test_utils::fixture_issue("Dependent".into(), String::new());
            dependent.state = State::Ready;
            dependent.dependencies.push(blocker_id.clone());
            let id = dependent.id.clone();
            crate::commands::test_helpers::seed_issue(&executor.storage, dependent);
            id
        });
        let opened = counter.count();

        executor
            .issues()
            .update_issue(
                &blocker_id,
                None,
                None,
                None,
                Some(State::InProgress),
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();

        (
            counter.count() - opened,
            dependent_id.map(|id| executor.storage.load_issue(&id).unwrap().state),
        )
    }

    #[test]
    fn test_update_issue_reopening_a_done_blocker_demotes_its_ready_dependent_in_the_same_publication(
    ) {
        let (baseline, _) = reopen_done_blocker(false);
        let (sessions, dependent) = reopen_done_blocker(true);

        assert_eq!(dependent, Some(State::Backlog));
        assert_eq!(
            sessions, baseline,
            "the dependent's demotion must add no session to the state change"
        );
    }

    /// Reject a blocker with `reason`, optionally blocking a `Backlog`
    /// dependent; returns the session opens the rejection took and the
    /// dependent's state.
    fn reject_blocker(reason: Option<&str>, with_dependent: bool) -> (usize, Option<State>) {
        let counter = crate::commands::test_helpers::SessionOpenCounter::new();
        let executor = crate::commands::test_helpers::memory_executor(
            InMemoryStorage::new().with_mutation_interruption_view(counter.clone()),
        );
        let blocker = crate::test_utils::fixture_issue("Blocker".into(), String::new());
        let blocker_id = blocker.id.clone();
        crate::commands::test_helpers::seed_issue(&executor.storage, blocker);
        let dependent_id = with_dependent.then(|| {
            let mut dependent = crate::test_utils::fixture_issue("Dependent".into(), String::new());
            dependent.state = State::Backlog;
            dependent.dependencies.push(blocker_id.clone());
            let id = dependent.id.clone();
            crate::commands::test_helpers::seed_issue(&executor.storage, dependent);
            id
        });
        let opened = counter.count();

        executor
            .issues()
            .reject_issue(&IssueIdPrefix::from(&blocker_id), reason)
            .unwrap();

        (
            counter.count() - opened,
            dependent_id.map(|id| executor.storage.load_issue(&id).unwrap().state),
        )
    }

    #[test]
    fn test_reject_issue_promotes_dependent_whose_last_blocker_it_removes_in_the_same_publication()
    {
        for reason in [None, Some("obsolete")] {
            let (baseline, _) = reject_blocker(reason, false);
            let (sessions, dependent) = reject_blocker(reason, true);

            assert_eq!(dependent, Some(State::Ready), "reason: {reason:?}");
            assert_eq!(
                sessions, baseline,
                "the dependent's promotion must add no session to the rejection (reason: {reason:?})"
            );
        }
    }

    #[test]
    fn test_reject_issue_interruption_leaves_prior_state_or_a_completed_record() {
        use crate::storage::{RedoPoint, RepositoryStateStore};

        for (point, rejected) in [
            (MutationPoint::BeforeCommitCheck, false),
            (MutationPoint::Redo(RedoPoint::BeforePut { index: 1 }), true),
        ] {
            let failures = Arc::new(PointFailure(std::sync::Mutex::new(Some(point))));
            let storage = InMemoryStorage::with_mutation_interruptions(failures);
            storage.add_data_file("config.toml", "[worktree]\nenforce_leases = \"off\"\n");
            let recovered = storage.without_mutation_interruptions();
            let layout = storage.repository_layout();
            let blocker = crate::test_utils::fixture_issue("Blocker".into(), String::new());
            let blocker_id = blocker.id.clone();
            let prior_blocker = blocker.state;
            crate::commands::test_helpers::seed_issue(&storage, blocker);
            let mut dependent = crate::test_utils::fixture_issue("Dependent".into(), String::new());
            dependent.state = State::Backlog;
            dependent.dependencies.push(blocker_id.clone());
            let dependent_id = dependent.id.clone();
            crate::commands::test_helpers::seed_issue(&storage, dependent);
            let executor = CommandExecutor::new(storage)
                .with_layout(layout.clone())
                .unwrap();

            assert!(executor
                .issues()
                .reject_issue(&IssueIdPrefix::from(&blocker_id), None)
                .is_err());
            drop(recovered.open_mutation_session(layout).unwrap());

            let (blocker_state, dependent_state) = if rejected {
                (State::Rejected, State::Ready)
            } else {
                (prior_blocker, State::Backlog)
            };
            assert_eq!(
                recovered.load_issue(&blocker_id).unwrap().state,
                blocker_state
            );
            assert_eq!(
                recovered.load_issue(&dependent_id).unwrap().state,
                dependent_state
            );
        }
    }

    #[test]
    fn test_reject_issue_with_reason_records_state_events_and_resolution_label() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let mut template = crate::test_utils::fixture_issue("Reject".into(), String::new());
        template.state = State::InProgress;
        let mut twin = template.clone();
        twin.id = crate::test_utils::fixture_issue(String::new(), String::new()).id;
        let with_reason = seed_fixture_issue(&executor, template);
        let state_only = seed_fixture_issue(&executor, twin);

        executor
            .issues()
            .reject_issue(&IssueIdPrefix::from(&with_reason), Some("duplicate"))
            .unwrap();
        executor
            .issues()
            .reject_issue(&IssueIdPrefix::from(&state_only), None)
            .unwrap();

        let issue = executor.storage.load_issue(&with_reason).unwrap();
        assert_eq!(issue.state, State::Rejected);
        assert!(issue.labels.contains(&"resolution:duplicate".to_string()));
        assert_eq!(
            issue_event_types(&executor, &with_reason),
            issue_event_types(&executor, &state_only),
            "the resolution label records no event of its own"
        );
    }

    #[test]
    fn test_reject_issue_failed_publication_leaves_prior_state_without_label() {
        let failures = Arc::new(PointFailure(std::sync::Mutex::new(Some(
            MutationPoint::BeforeCommitCheck,
        ))));
        let storage = InMemoryStorage::with_mutation_interruptions(failures);
        let recovered = storage.without_mutation_interruptions();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let issue = crate::test_utils::fixture_issue("Reject".into(), String::new());
        let prior = issue.state;
        let id = seed_fixture_issue(&executor, issue);
        let events_before = recovered.read_events().unwrap().len();

        assert!(executor
            .issues()
            .reject_issue(&IssueIdPrefix::from(&id), Some("duplicate"))
            .is_err());

        let issue = recovered.load_issue(&id).unwrap();
        assert_eq!(issue.state, prior);
        assert!(!issue
            .labels
            .iter()
            .any(|label| label.starts_with("resolution:")));
        assert_eq!(recovered.read_events().unwrap().len(), events_before);
    }

    #[test]
    fn test_reject_issue_recaptures_after_concurrent_issue_change() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage.clone(),
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let id = seed_fixture_issue(
            &executor,
            crate::test_utils::fixture_issue("Reject race".into(), String::new()),
        );
        let mut concurrent = storage.load_issue(&id).unwrap();
        concurrent.labels.push("owner:concurrent".to_string());
        let raced = crate::commands::test_helpers::with_apply_race(
            storage,
            crate::commands::test_helpers::SessionRaceAction::Save(Box::new(concurrent)),
        );
        let executor = crate::commands::test_helpers::memory_executor(raced.clone());

        executor
            .issues()
            .reject_issue(&IssueIdPrefix::from(&id), Some("done"))
            .unwrap();

        let issue = raced.load_issue(&id).unwrap();
        assert_eq!(issue.state, State::Rejected);
        assert!(issue.labels.iter().any(|label| label == "owner:concurrent"));
        assert!(issue.labels.iter().any(|label| label == "resolution:done"));
    }

    #[test]
    fn test_reject_issue_refuses_reason_that_cannot_form_a_label_without_writing() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let issue = crate::test_utils::fixture_issue("Reject prose".into(), String::new());
        let prior = issue.clone();
        let id = seed_fixture_issue(&executor, issue);
        let events_before = executor.storage.read_events().unwrap().len();

        let error = executor
            .issues()
            .reject_issue(&IssueIdPrefix::from(&id), Some("Premise removed. See x"))
            .unwrap_err();

        assert!(error
            .downcast_ref::<crate::errors::InvalidArgumentError>()
            .is_some());
        let issue = executor.storage.load_issue(&id).unwrap();
        assert_eq!(issue.state, prior.state);
        assert_eq!(issue.labels, prior.labels);
        assert_eq!(executor.storage.read_events().unwrap().len(), events_before);
    }

    #[test]
    fn test_update_issue_remove_label_fails_atomically_when_any_label_is_absent() {
        let executor = crate::commands::test_helpers::memory_executor(InMemoryStorage::new());
        let mut issue = crate::test_utils::fixture_issue("Remove".into(), String::new());
        issue.labels = vec!["component:core".to_string()];
        let id = seed_fixture_issue(&executor, issue);

        let result = executor.issues().update_issue(
            &id,
            None,
            None,
            None,
            None,
            vec![],
            vec!["component:core".to_string(), "component:absent".to_string()],
            None,
            None,
            false,
        );

        assert!(result.is_err());
        assert_eq!(
            executor.storage.load_issue(&id).unwrap().labels,
            vec!["component:core".to_string()]
        );
    }

    #[test]
    fn test_update_issue_remove_label_removes_a_malformed_label() {
        let executor = crate::commands::test_helpers::memory_executor(InMemoryStorage::new());
        let mut issue = crate::test_utils::fixture_issue("Remove".into(), String::new());
        issue.labels = vec![
            "component:core".to_string(),
            "resolution:Not valid: prose".to_string(),
        ];
        let id = seed_fixture_issue(&executor, issue);

        executor
            .issues()
            .update_issue(
                &id,
                None,
                None,
                None,
                None,
                vec![],
                vec!["resolution:Not valid: prose".to_string()],
                None,
                None,
                false,
            )
            .unwrap();

        assert_eq!(
            executor.storage.load_issue(&id).unwrap().labels,
            vec!["component:core".to_string()]
        );
    }

    #[test]
    fn test_reject_issue_appends_resolution_label_unchecked() {
        let storage = InMemoryStorage::new();
        seed_scenario_gates(&storage);
        let executor = memory_executor_declaring(
            storage,
            MemoryDeclarations {
                config: Some(&scenario_config()),
                ..MemoryDeclarations::default()
            },
        );
        let id = seed_fixture_issue(
            &executor,
            crate::test_utils::fixture_issue("Reject twice".into(), String::new()),
        );

        executor
            .issues()
            .reject_issue(&IssueIdPrefix::from(&id), Some("done"))
            .unwrap();
        executor
            .issues()
            .reject_issue(&IssueIdPrefix::from(&id), Some("done"))
            .unwrap();

        let labels = executor.storage.load_issue(&id).unwrap().labels;
        assert_eq!(
            labels
                .iter()
                .filter(|label| label.as_str() == "resolution:done")
                .count(),
            2
        );
    }

    #[test]
    fn test_create_issue_returns_the_issue_storage_holds() {
        let executor =
            memory_executor_declaring(InMemoryStorage::new(), MemoryDeclarations::default());

        let created = executor
            .issues()
            .create_issue(
                "Created".to_string(),
                "Body".to_string(),
                crate::domain::Priority::High,
                vec![],
                vec!["type:task".to_string()],
                None,
                None,
                false,
            )
            .unwrap();

        assert_eq!(
            created.issue,
            executor.storage.load_issue(&created.issue.id).unwrap()
        );
    }
}
