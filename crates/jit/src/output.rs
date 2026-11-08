//! Structured output formatting for CLI commands.
//!
//! This module provides consistent JSON output formatting for both success
//! and error cases, ensuring machine-readable output that works well with
//! AI agents and automation tools.

use crate::commands::{
    ArtifactConformanceResponse, ArtifactDirectoryResponse, ContainerProgressResponse,
    DivergenceResponse, DivergenceView, GateStatusEntry, GraphDepsTreeResponse, GraphRootsResponse,
    GraphTreeResponse, IssueChildrenResponse, IssueShowResponse, IssueStatusResponse, StateRollup,
    UpstreamMoveReport,
};
use crate::domain::identity::GateKey;
use crate::domain::Assignee;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Display;
use thiserror::Error;

mod apply;
mod archive;
mod claim;
pub mod config;
mod dep;
mod doc;
mod emit;
mod events;
pub mod failure;
pub mod gate;
mod graph;
pub mod hooks;
mod init;
mod invariant;
mod issue;
mod item;
mod profile;
mod project;
mod projection;
mod search;
mod serve;
pub mod snapshot;
mod validate;
mod version;
mod worktree;

pub use validate::*;
pub use worktree::WorktreeInfoOutput;

pub use archive::ArchiveTargetResponse;
pub use claim::{
    ClaimAcquireResponse, ClaimForceEvictResponse, ClaimHeartbeatResponse, ClaimReleaseResponse,
    ClaimRenewResponse, LeaseListResponse, LeasesKey, RecoverResponse,
};
pub use dep::{DependencyAddResponse, DependencyRemoveResponse};
pub(crate) use emit::status_after;
pub use emit::{emit_failure, emit_outcome, HumanRender, Report, Response};
pub use issue::{
    BatchCreateResponse, BlockedListing, IssueBatchCreateOutput, IssueDeleteResponse,
    IssueLifecycleResponse, IssueListing, IssueSearchResults, IssueShowOutput, IssueStatusOutput,
    IssueUpdateOutput, QuerySubset, SubsetListing,
};
pub use projection::FieldProjection;
pub use search::{SearchHit, SearchKey, SearchQuery, SearchResponse};
pub use serve::{ServeResponse, WebUiSource};

use crate::declarations::{GateMode, GateStage};
use crate::domain::gate_findings::GateFindings;
use crate::domain::identity::IssueId;
use crate::domain::list_envelope::{collection_keys, CollectionKey, ListEnvelope};
use crate::domain::queries::BlockingReason;
use crate::domain::store_divergence::StoreDivergence;
use crate::domain::vocabulary::wire_vocabulary;
use crate::domain::{
    GateRunResult, GateRunStatus, GateStatus, Issue, MinimalBlockedIssue, MinimalIssue, Priority,
    State,
};
use crate::errors::{TransitionBlockedError, TransitionBlocker};

/// Render the complete schema-v1 archive plan without introducing a second
/// decision model. Every action, fact, edge, relink, deletion, warning, and
/// blocker comes directly from the serialized plan object.
pub fn render_archive_plan(plan: &crate::domain::artifact_plan::ArtifactPlan) -> String {
    use crate::domain::artifact_plan::PlanTarget;
    use std::fmt::Write;

    let mut rendered = String::new();
    let target = match plan.target() {
        PlanTarget::Container { id } => format!("container {}", id.short_id()),
        PlanTarget::Document { path } => format!("document {path}"),
    };
    let counts = plan.action_counts();
    let _ = writeln!(rendered, "Archive preview: {target}");
    let _ = writeln!(rendered, "  Schema version: {}", plan.schema_version());
    let _ = writeln!(rendered, "  Policy: {}", wire_name(&plan.policy_status()));
    let _ = writeln!(rendered, "  Eligible for execution: {}", plan.eligible());
    let _ = writeln!(
        rendered,
        "  Destination root: {}",
        display_optional(plan.destination_root())
    );
    let _ = writeln!(
        rendered,
        "  Actions: move={} copy={} retain={} block={} already-archived={} pending-deletions={}",
        counts.r#move,
        counts.copy,
        counts.retain,
        counts.block,
        counts.already_archived,
        counts.pending_deletions
    );
    render_diagnostics(&mut rendered, "Blockers", plan.blockers());
    render_diagnostics(&mut rendered, "Warnings", plan.plan_warnings());

    let _ = writeln!(rendered, "Artifacts ({}):", plan.count());
    for artifact in plan.artifacts() {
        let _ = writeln!(
            rendered,
            "  - [{}] {} @ {}",
            wire_name(&artifact.action()),
            artifact.source(),
            artifact.version().as_str()
        );
        let _ = writeln!(
            rendered,
            "    destination: {}",
            artifact.destination().unwrap_or("-")
        );
        let _ = writeln!(
            rendered,
            "    format: {}",
            artifact.format().unwrap_or("opaque")
        );
        let _ = writeln!(
            rendered,
            "    already archived: {}",
            artifact.already_archived()
        );
        if let Some(identity) = artifact.content_identity() {
            let _ = writeln!(
                rendered,
                "    content identity: sha256={} bytes={}",
                identity.sha256(),
                identity.byte_size()
            );
        }
        let _ = writeln!(
            rendered,
            "    provenance: {}",
            joined_wire_names(artifact.provenance())
        );
        let _ = writeln!(
            rendered,
            "    evidence: {}",
            joined_wire_names(artifact.evidence())
        );
        if !artifact.owners().is_empty() {
            let _ = writeln!(rendered, "    owners:");
            for owner in artifact.owners() {
                let _ = writeln!(
                    rendered,
                    "      - issue={} document-index={} state={} inside-subtree={} pinned={} selected-for-relink={}",
                    owner.issue.short_id(),
                    owner.document_index,
                    wire_name(&owner.state),
                    owner.inside_subtree,
                    owner.pinned,
                    owner.selected_for_relink
                );
            }
        }
        if !artifact.edges().is_empty() {
            let _ = writeln!(rendered, "    edges:");
            for edge in artifact.edges() {
                let _ = writeln!(
                    rendered,
                    "      - {} {} reference={} target={}",
                    wire_name(&edge.kind),
                    wire_name(&edge.resolution_mode),
                    edge.reference,
                    edge.target.as_deref().unwrap_or("-")
                );
            }
        }
        if !artifact.reference_changes().is_empty() {
            let _ = writeln!(rendered, "    reference changes:");
            for change in artifact.reference_changes() {
                let _ = writeln!(
                    rendered,
                    "      - issue={} document-index={} {} -> {}",
                    change.issue.short_id(),
                    change.document_index,
                    change.from_path,
                    change.to_path
                );
            }
        }
        if !artifact.pending_deletions().is_empty() {
            let _ = writeln!(rendered, "    pending deletions:");
            for deletion in artifact.pending_deletions() {
                let _ = writeln!(
                    rendered,
                    "      - {} sha256={} bytes={}",
                    deletion.source,
                    deletion.content_identity.sha256(),
                    deletion.content_identity.byte_size()
                );
            }
        }
        render_diagnostics(&mut rendered, "    blockers", artifact.blockers());
        render_diagnostics(&mut rendered, "    warnings", artifact.plan_warnings());
    }
    if !plan.eligible() {
        let _ = writeln!(
            rendered,
            "Archival execution is disabled until every blocker is resolved."
        );
    }
    rendered
}

/// Render exactly the fully evaluated plans returned by `archive candidates`.
pub fn render_archive_candidates(
    report: &crate::domain::artifact_plan::ArchiveCandidates,
) -> String {
    use std::fmt::Write;

    let mut rendered = String::new();
    let _ = writeln!(rendered, "Archive candidates ({}):", report.count());
    if report.candidates().is_empty() {
        let _ = writeln!(rendered, "  No terminal configured non-leaf containers.");
        return rendered;
    }
    for (index, plan) in report.candidates().iter().enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        rendered.push_str(&render_archive_plan(plan));
    }
    rendered + &render_archive_advisory(report.advisory())
}

/// Render an archive advisory section: each cited artifact, then one
/// `cited at` line per citation location. An empty section renders nothing.
pub fn render_archive_advisory(
    advisory: &crate::domain::archive_advisory::ArchiveAdvisory,
) -> String {
    use std::fmt::Write;

    if advisory.citations.is_empty() {
        return String::new();
    }
    let mut rendered = String::from("Advisory citations (live files, not part of the plan):\n");
    for citations in &advisory.citations {
        let _ = writeln!(rendered, "  - {}", citations.artifact);
        for location in &citations.locations {
            let _ = writeln!(rendered, "    cited at: {location}");
        }
    }
    rendered
}

/// Render the human summary of an executed archive: the completion counts,
/// then one `warning: {code} ({path})` line per warning.
pub fn render_archive_execution(
    result: &crate::domain::artifact_execution::ArchiveExecutionResult,
) -> String {
    use std::fmt::Write;

    let mut rendered = String::new();
    let _ = writeln!(
        rendered,
        "Archive execution complete: {} publication(s), {} reference change(s), {} source deletion(s)",
        result.publications.len(),
        result.reference_changes.len(),
        result.deleted_sources.len()
    );
    for warning in &result.plan_warnings {
        let _ = writeln!(
            rendered,
            "warning: {}{}",
            warning.code.as_str(),
            warning
                .path
                .as_ref()
                .map(|path| format!(" ({path})"))
                .unwrap_or_default()
        );
    }
    rendered
}

/// A column printed after `{id} | {title}` in a human issue-list row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueColumn {
    State,
    Priority,
}

/// The fields a human issue-list row shows.
pub trait ListedIssue {
    fn id(&self) -> &IssueId;
    fn title(&self) -> &str;
    fn state(&self) -> State;
    fn priority(&self) -> Priority;
}

macro_rules! listed_issue {
    ($($ty:ty),+) => {$(
        impl ListedIssue for $ty {
            fn id(&self) -> &IssueId {
                &self.id
            }
            fn title(&self) -> &str {
                &self.title
            }
            fn state(&self) -> State {
                self.state
            }
            fn priority(&self) -> Priority {
                self.priority
            }
        }
    )+};
}

listed_issue!(Issue, MinimalIssue);

/// One row of a human issue list: the issue and its optional reason lines.
pub type IssueListRow<'a, I = Issue> = (&'a I, Vec<String>);

/// Rows with no reason lines.
pub fn plain_issue_rows<I: ListedIssue>(issues: &[I]) -> impl Iterator<Item = IssueListRow<'_, I>> {
    issues.iter().map(|issue| (issue, Vec::new()))
}

/// Render a human issue list: `header`, two-space-indented rows (`id | title`
/// plus `columns`, then one `    - reason` line per reason), and a blank-line
/// separated `Total: N` footer. `quiet` drops the header and footer.
pub fn render_issue_list<'a, I: ListedIssue + 'a>(
    header: &str,
    rows: impl IntoIterator<Item = IssueListRow<'a, I>>,
    columns: &[IssueColumn],
    quiet: bool,
) -> String {
    use std::fmt::Write;

    let mut lines = String::new();
    let mut total = 0usize;
    for (issue, reasons) in rows {
        total += 1;
        let _ = write!(lines, "  {} | {}", issue.id(), issue.title());
        for column in columns {
            let _ = match column {
                IssueColumn::State => write!(lines, " | {}", issue.state()),
                IssueColumn::Priority => write!(lines, " | {}", issue.priority()),
            };
        }
        lines.push('\n');
        for reason in reasons {
            let _ = writeln!(lines, "    - {reason}");
        }
    }
    if quiet {
        lines.truncate(lines.trim_end_matches('\n').len());
        if !lines.is_empty() {
            lines.push('\n');
        }
        lines
    } else {
        format!("{header}\n{lines}\nTotal: {total}\n")
    }
}

/// Which run-status line [`render_gate_run_status`] renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateRunLine {
    NeverRun,
    Passed,
}

/// The human status line for a gate's run, preserving the requested text.
pub fn render_gate_run_status(
    line: GateRunLine,
    gate_key: &GateKey,
    requested: impl Display,
) -> String {
    match line {
        GateRunLine::NeverRun => format!(
            "Gate '{gate_key}' has not been run yet for issue {requested}. Use 'jit gate evaluate' to run it."
        ),
        GateRunLine::Passed => format!("Passed gate '{gate_key}' for issue {requested}"),
    }
}

/// Closing lines of a findings block: `pass` when there are no findings,
/// `"{failure} with N error(s)"` on stderr when any finding is an error.
#[derive(Debug, Clone, Copy)]
pub struct FindingsSummary<'a> {
    pub pass: &'a str,
    pub failure: &'a str,
}

/// The human findings block, split by destination stream.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderedFindings {
    pub stdout: String,
    pub stderr: String,
}

/// One `{severity} [{rule}] {message}` line per finding, then the summary.
/// Warnings-only findings get no pass line; `summary: None` renders the lines alone
/// for commands that frame the block themselves.
pub fn render_validation_findings(
    findings: &[crate::validation::report::ReportedFinding],
    summary: Option<FindingsSummary<'_>>,
) -> RenderedFindings {
    use std::fmt::Write;
    let mut rendered = RenderedFindings::default();
    for finding in findings {
        let severity = if finding.is_error() {
            "error"
        } else {
            "warning"
        };
        let _ = writeln!(
            rendered.stdout,
            "{severity} [{}] {}",
            finding.rule, finding.message
        );
    }
    let errors = findings.iter().filter(|f| f.is_error()).count();
    if let Some(summary) = summary {
        if errors > 0 {
            let _ = writeln!(
                rendered.stderr,
                "{} with {errors} error(s)",
                summary.failure
            );
        } else if findings.is_empty() {
            let _ = writeln!(rendered.stdout, "{}", summary.pass);
        }
    }
    rendered
}

fn render_diagnostics<T: Serialize>(rendered: &mut String, heading: &str, diagnostics: &[T]) {
    use std::fmt::Write;
    if diagnostics.is_empty() {
        return;
    }
    let _ = writeln!(rendered, "{heading}:");
    for diagnostic in diagnostics {
        let value = serde_json::to_value(diagnostic).unwrap_or(Value::Null);
        let code = value
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let path = value
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("target");
        let _ = writeln!(rendered, "  - {code}: {path}");
        // A state-caused blocker carries a `guidance` field (REQ-05): show the
        // permitted next action right under the blocker so the operator does not
        // have to look the code up.
        if let Some(guidance) = value.get("guidance").and_then(Value::as_str) {
            let _ = writeln!(rendered, "      next action: {guidance}");
        }
    }
}

fn joined_wire_names<T: Serialize>(values: &[T]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.iter().map(wire_name).collect::<Vec<_>>().join(", ")
    }
}

fn wire_name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

fn display_optional(value: &str) -> &str {
    if value.is_empty() {
        "(not configured)"
    } else {
        value
    }
}

#[cfg(test)]
mod issue_list_render_tests {
    use super::*;

    fn issue(id: IssueId, title: &str) -> Issue {
        Issue::draft(id, title.into(), String::new())
    }

    #[test]
    fn test_render_issue_list_indents_rows_and_columns_reasons_and_total_footer() {
        let issues = [
            issue(
                crate::test_utils::issue_id(0x4000_8000_0000_0000_0001_aaaa_aaaa),
                "First",
            ),
            issue(
                crate::test_utils::issue_id(0x4000_8000_0000_0000_0002_bbbb_bbbb),
                "Second",
            ),
        ];
        let rows = vec![
            (&issues[0], vec!["waits on x".to_string()]),
            (&issues[1], Vec::new()),
        ];

        let rendered = render_issue_list(
            "Header:",
            rows,
            &[IssueColumn::State, IssueColumn::Priority],
            false,
        );

        let (state, priority) = (issues[0].state, issues[0].priority);
        assert_eq!(
            rendered,
            format!(
                "Header:\n  {} | First | {state} | {priority}\n    - waits on x\n  {} | Second | {state} | {priority}\n\nTotal: 2\n", issues[0].id, issues[1].id
            )
        );
    }

    #[test]
    fn test_render_issue_list_without_columns_prints_id_and_title_only() {
        let issues = [issue(
            crate::test_utils::issue_id(0x4000_8000_0000_0000_0001_aaaa_aaaa),
            "First",
        )];

        let rendered = render_issue_list("H", plain_issue_rows(&issues), &[], false);

        assert_eq!(
            rendered,
            format!("H\n  {} | First\n\nTotal: 1\n", issues[0].id)
        );
    }

    #[test]
    fn test_render_issue_list_quiet_keeps_rows_and_drops_header_and_footer() {
        let issues = [issue(
            crate::test_utils::issue_id(0x4000_8000_0000_0000_0001_aaaa_aaaa),
            "First",
        )];

        let rendered = render_issue_list(
            "H",
            plain_issue_rows(&issues),
            &[IssueColumn::Priority],
            true,
        );

        assert_eq!(
            rendered,
            format!("  {} | First | {}\n", issues[0].id, issues[0].priority)
        );
    }

    #[test]
    fn test_render_issue_list_empty_prints_header_and_zero_total() {
        assert_eq!(
            render_issue_list::<Issue>("H", [], &[], false),
            "H\n\nTotal: 0\n"
        );
    }
}

#[cfg(test)]
mod archive_render_tests {
    use super::*;
    use crate::domain::artifact_plan::{
        ArtifactAction, ArtifactEdge, ArtifactOwner, ArtifactPlan, ArtifactPlanEntry,
        ArtifactProvenance, ArtifactVersion, BlockerCode, EdgeKind, EdgeResolutionMode,
        EvidenceCode, PlanBlocker, PlanTarget, PlanWarning, PolicyStatus, WarningCode,
    };
    use crate::domain::State;

    #[test]
    fn test_human_archive_preview_projects_every_plan_decision_and_diagnostic() {
        let owner_id = crate::test_utils::issue_id(1);
        let artifact = ArtifactPlanEntry::new(
            "dev/active/page.md",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Block,
        )
        .with_provenance(vec![ArtifactProvenance::Explicit])
        .with_format("markdown")
        .with_owners(vec![ArtifactOwner {
            issue: owner_id.clone(),
            document_index: 2,
            state: State::Done,
            archived_from: None,
            inside_subtree: true,
            pinned: false,
            selected_for_relink: false,
        }])
        .with_edges(vec![ArtifactEdge {
            reference: "missing.png".into(),
            target: Some("dev/active/missing.png".into()),
            kind: EdgeKind::Supported,
            resolution_mode: EdgeResolutionMode::Relative,
        }])
        .with_evidence(vec![EvidenceCode::PermanentPath])
        .with_blockers(vec![PlanBlocker::new(
            BlockerCode::DestinationConflict,
            Some("dev/archive/page.md"),
        )])
        .with_plan_warnings(vec![PlanWarning::new(
            WarningCode::MissingEdgeTarget,
            Some("dev/active/missing.png"),
        )]);
        let plan = ArtifactPlan::new(
            PlanTarget::Document {
                path: "dev/active/page.md".into(),
            },
            "dev/archive",
            PolicyStatus::Configured,
            vec![artifact],
            vec![],
            vec![],
        )
        .unwrap();

        let human = render_archive_plan(&plan);
        assert!(human.contains(&format!(
            "issue={} document-index=2 state=done",
            owner_id.short_id()
        )));
        for expected in [
            "Schema version: 1",
            "Policy: configured",
            "Eligible for execution: false",
            "[block] dev/active/page.md @ working-tree",
            "format: markdown",
            "provenance: explicit",
            "evidence: permanent-path",
            "supported relative reference=missing.png target=dev/active/missing.png",
            "destination-conflict: dev/archive/page.md",
            "missing-edge-target: dev/active/missing.png",
            "Archival execution is disabled",
        ] {
            assert!(
                human.contains(expected),
                "human preview omitted {expected:?}\n{human}"
            );
        }
    }
}

// ============================================================================
// Output Context for Quiet Mode
// ============================================================================

/// The global output flags of one invocation, built once from the parsed
/// command line; the only source of `--json` for dispatch and for rendering a
/// failure that escapes it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutputMode {
    pub json: bool,
    pub quiet: bool,
}

/// True when a panic `payload` is the one std's `print!`/`println!`/`eprint!`/
/// `eprintln!` machinery raises when a stream write fails on a closed pipe
/// (`library/std/src/io/stdio.rs`: `panic!("failed printing to {label}: {e}")`).
/// A payload is an opaque `dyn Any` holding a `String` or `&str`, so detection
/// matches std's long-stable wording. `main`'s panic hook uses it to turn the
/// remaining raw print sites' broken-pipe panic into [`ExitCode::BrokenPipe`].
pub fn is_broken_pipe_write_panic(payload: &dyn std::any::Any) -> bool {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .is_some_and(|message| {
            message.starts_with("failed printing to std") && message.contains("Broken pipe")
        })
}

// ============================================================================
// JSON Output Types
// ============================================================================

/// The top-level success-envelope property carrying a success outcome's
/// warnings; the schema generator declares it for every success schema.
pub const WARNINGS_FIELD: &str = "warnings";

/// Wrapper for successful command output (now returns raw data without envelope)
#[derive(Debug)]
pub struct JsonOutput<T: Serialize> {
    pub data: T,
    pub message: Option<String>,
    pub warnings: Vec<String>,
}

impl<T: Serialize> JsonOutput<T> {
    /// Create a new successful output with the given data
    pub fn success(data: T) -> Self {
        Self {
            data,
            message: None,
            warnings: Vec::new(),
        }
    }

    /// Add a human-readable message to the JSON output.
    ///
    /// The message is injected as a top-level `"message"` field in the
    /// serialized JSON object. If the data serializes to a non-object
    /// (e.g. an array), the message is silently dropped.
    pub fn with_message(mut self, msg: impl Into<String>) -> Self {
        self.message = Some(msg.into());
        self
    }

    /// Add the warnings listed as the top-level `warnings` array.
    pub fn with_warnings(mut self, warnings: Vec<String>) -> Self {
        self.warnings = warnings;
        self
    }

    /// Serialize to JSON string with pretty formatting (returns raw data, no envelope).
    /// A set message is injected as `message`, and non-empty warnings as the
    /// `warnings` array, into the top-level object.
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        let mut value = serde_json::to_value(&self.data)?;
        if let Value::Object(ref mut map) = value {
            if let Some(ref msg) = self.message {
                map.insert("message".to_string(), Value::String(msg.clone()));
            }
            if !self.warnings.is_empty() {
                map.insert(WARNINGS_FIELD.to_string(), serde_json::json!(self.warnings));
            }
        }
        serde_json::to_string_pretty(&value)
    }
}

/// Wrapper for error output with suggestions (simplified, no envelope)
#[derive(Debug, Serialize)]
pub struct JsonError {
    pub error: ErrorDetail,
    /// Serialized as the top-level `warnings` array only when non-empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl JsonError {
    /// Create an error whose code belongs to the registered vocabulary.
    ///
    /// The status comes from [`ErrorCode::exit_code`], so registered envelopes
    /// cannot carry a status that disagrees with their code.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            error: ErrorDetail {
                code,
                message: message.into(),
                details: None,
                suggestions: Vec::new(),
            },
            warnings: Vec::new(),
        }
    }

    /// Add the warnings listed beside `error`.
    pub fn with_warnings(mut self, warnings: Vec<String>) -> Self {
        self.warnings = warnings;
        self
    }

    /// Add details to the error
    pub fn with_details(mut self, details: Value) -> Self {
        self.error.details = Some(details);
        self
    }

    /// Add a suggestion to the error
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.error.suggestions.push(suggestion.into());
        self
    }

    /// Add multiple suggestions to the error
    pub fn with_suggestions(mut self, suggestions: Vec<String>) -> Self {
        self.error.suggestions.extend(suggestions);
        self
    }

    /// Serialize to JSON string with pretty formatting
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Return the process exit status selected by the envelope's error code.
    pub const fn exit_code(&self) -> ExitCode {
        self.error.code.exit_code()
    }
}

/// Error details including code, message, and suggestions
#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    /// Typed error code serialized using its registered wire spelling.
    pub code: ErrorCode,
    /// Human-readable error message
    pub message: String,
    /// Optional additional error details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    /// Suggested actions to resolve the error
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
}

// ============================================================================
// Exit Codes
// ============================================================================

/// Standardized exit codes for the JIT CLI
///
/// These codes follow Unix conventions and provide consistent error reporting
/// for automation and scripting. [`ExitCode::ALL`] is checked against this
/// enum's test-only derived schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(JsonSchema))]
#[repr(i32)]
pub enum ExitCode {
    /// Command succeeded (0)
    Success = 0,

    /// Generic error (1)
    GenericError = 1,

    /// Invalid arguments or usage error (2)
    InvalidArgument = 2,

    /// Resource not found - issue, gate, etc. (3)
    NotFound = 3,

    /// Validation failed - cycle detected, broken refs, etc. (4)
    ValidationFailed = 4,

    /// Permission denied (5)
    PermissionDenied = 5,

    /// Resource already exists (6)
    AlreadyExists = 6,

    /// External dependency failed - git, file system, etc. (10)
    ExternalError = 10,

    /// A lock wait expired while another jit process held the lock (11)
    LockTimeout = 11,

    /// A repository read or mutation conflicted and may be retried (12).
    RetryableConflict = 12,

    /// A downstream reader closed the pipe while jit was writing (141)
    ///
    /// `128 + SIGPIPE (13) = 141`: the exit status a shell reports for a
    /// process terminated by SIGPIPE, returned as a status rather than by
    /// altering this process's own
    /// signal disposition (resetting SIGPIPE disposition needs an unsafe
    /// FFI call, and this crate forbids unsafe code).
    BrokenPipe = 141,
}

impl ExitCode {
    /// Every exit code, in documentation order.
    pub const ALL: [ExitCode; 11] = [
        ExitCode::Success,
        ExitCode::GenericError,
        ExitCode::InvalidArgument,
        ExitCode::NotFound,
        ExitCode::ValidationFailed,
        ExitCode::PermissionDenied,
        ExitCode::AlreadyExists,
        ExitCode::ExternalError,
        ExitCode::LockTimeout,
        ExitCode::RetryableConflict,
        ExitCode::BrokenPipe,
    ];

    /// The process exit status.
    pub fn code(self) -> i32 {
        self as i32
    }

    /// Get a description of what this exit code means
    pub fn description(self) -> &'static str {
        match self {
            ExitCode::Success => "Command succeeded",
            ExitCode::GenericError => "Generic error occurred",
            ExitCode::InvalidArgument => "Invalid arguments or usage error",
            ExitCode::NotFound => "Resource not found (issue, gate, etc.)",
            ExitCode::ValidationFailed => {
                "Validation failed (cycle detected, broken references, etc.)"
            }
            ExitCode::PermissionDenied => "Permission denied",
            ExitCode::AlreadyExists => "Resource already exists",
            ExitCode::ExternalError => "External dependency failed (git, file system, etc.)",
            ExitCode::LockTimeout => {
                "A lock wait expired while another jit process held the lock; retrying may succeed"
            }
            ExitCode::RetryableConflict => "The repository changed or is busy; retry",
            ExitCode::BrokenPipe => {
                "Downstream reader closed the pipe while jit was writing (128 + SIGPIPE)"
            }
        }
    }
}

/// The status a run exits with: a taxonomy code, or a status another program
/// decided (the `worktree exec` child, or clap rejecting the command line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Code(ExitCode),
    Passthrough(i32),
}

impl From<ExitCode> for RunStatus {
    fn from(code: ExitCode) -> Self {
        Self::Code(code)
    }
}

impl RunStatus {
    /// The process exit status.
    pub fn code(self) -> i32 {
        match self {
            Self::Code(code) => code.code(),
            Self::Passthrough(status) => status,
        }
    }
}

// ============================================================================
// Error Codes (JSON responses)
// ============================================================================

wire_vocabulary! {
    /// Standard error-code vocabulary for JIT's machine-readable responses.
    ///
    /// The declared names are the strings carried in JSON error envelopes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
    pub enum ErrorCode {
        /// The requested issue does not exist.
        IssueNotFound = "ISSUE_NOT_FOUND",
        /// The requested gate does not exist.
        GateNotFound = "GATE_NOT_FOUND",
        /// The requested dependency would make the graph cyclic.
        CycleDetected = "CYCLE_DETECTED",
        /// An argument or invocation is invalid.
        InvalidArgument = "INVALID_ARGUMENT",
        /// A label query filter is not in the accepted namespace/value form.
        InvalidLabelPattern = "INVALID_LABEL_PATTERN",
        /// Repository or domain validation failed.
        ValidationFailed = "VALIDATION_FAILED",
        /// The requested resource already exists.
        AlreadyExists = "ALREADY_EXISTS",
        /// Unfinished dependencies block the requested operation.
        Blocked = "BLOCKED",
        /// A quality-gate checker completed without passing.
        GateFailed = "GATE_FAILED",
        /// An input/output or external-system operation failed.
        IoError = "IO_ERROR",
        /// An operation was denied by filesystem or operating-system permissions.
        PermissionDenied = "PERMISSION_DENIED",
        /// Input data could not be parsed.
        ParseError = "PARSE_ERROR",
        /// The command requires a Git repository.
        RequiresGit = "REQUIRES_GIT",
        /// An ID prefix matched more than one candidate.
        AmbiguousId = "AMBIGUOUS_ID",
        /// An ID prefix was shorter than the accepted minimum.
        InvalidIdPrefix = "INVALID_ID_PREFIX",
        /// No `.jit` repository exists at the resolved data directory.
        RepositoryNotFound = "REPOSITORY_NOT_FOUND",
        /// A document or file addressed by path or commit does not exist.
        DocumentNotFound = "DOCUMENT_NOT_FOUND",
        /// The repository format is not the one this binary supports.
        RepositoryFormatUnsupported = "REPOSITORY_FORMAT_UNSUPPORTED",
        /// Issue deletion was refused because operator confirmation was absent.
        DeletionNotConfirmed = "DELETION_NOT_CONFIRMED",
        /// A state-mutating command was refused because it ran inside a linked
        /// non-primary checkout whose write stance refuses one.
        LinkedCheckoutWriteRefused = "LINKED_CHECKOUT_WRITE_REFUSED",
        /// No resolution route found the requested profile.
        /// Profile planning or final-state validation rejected the operation.
        ProfileConflict = "PROFILE_CONFLICT",
        /// A profile application was refused because a path it writes differs
        /// from `HEAD`.
        ProfileUncommittedTargets = "PROFILE_UNCOMMITTED_TARGETS",
        /// A gate-preset command failed.
        PresetError = "PRESET_ERROR",
        /// An issue item-address lookup failed before resolving an item.
        ItemNotFound = "ITEM_NOT_FOUND",
        /// A profile command failed without a more specific public classification.
        ProfileError = "PROFILE_ERROR",
        /// The search backend failed while executing a query.
        SearchFailed = "SEARCH_FAILED",
        /// The external ripgrep search tool could not be found.
        RipgrepNotFound = "RIPGREP_NOT_FOUND",
        /// A command failed without a more specific public classification.
        GenericError = "GENERIC_ERROR",
        /// Repository recovery failed.
        RecoveryFailed = "recovery_failed",
        /// The foreground server process exited with a failure status.
        ServerExited = "SERVER_EXITED",
        /// A lock wait expired while another jit process held the lock.
        LockTimeout = "LOCK_TIMEOUT",
        /// A repository read or mutation conflicted and may be retried.
        RetryableConflict = "RETRYABLE_CONFLICT",
    }
}

impl ErrorCode {
    /// Return the process exit status associated with this error code.
    ///
    /// The match is exhaustive so adding a member requires choosing its status.
    pub const fn exit_code(self) -> ExitCode {
        match self {
            ErrorCode::IssueNotFound
            | ErrorCode::GateNotFound
            | ErrorCode::RepositoryNotFound
            | ErrorCode::DocumentNotFound
            | ErrorCode::PresetError => ExitCode::NotFound,
            ErrorCode::CycleDetected
            | ErrorCode::ValidationFailed
            | ErrorCode::Blocked
            | ErrorCode::GateFailed
            | ErrorCode::ProfileConflict
            | ErrorCode::ProfileUncommittedTargets => ExitCode::ValidationFailed,
            ErrorCode::InvalidArgument
            | ErrorCode::InvalidLabelPattern
            | ErrorCode::AmbiguousId
            | ErrorCode::InvalidIdPrefix
            | ErrorCode::DeletionNotConfirmed
            | ErrorCode::LinkedCheckoutWriteRefused => ExitCode::InvalidArgument,
            ErrorCode::AlreadyExists => ExitCode::AlreadyExists,
            ErrorCode::IoError
            | ErrorCode::RequiresGit
            | ErrorCode::RepositoryFormatUnsupported => ExitCode::ExternalError,
            ErrorCode::PermissionDenied => ExitCode::PermissionDenied,
            ErrorCode::LockTimeout => ExitCode::LockTimeout,
            ErrorCode::RetryableConflict => ExitCode::RetryableConflict,
            ErrorCode::ParseError
            | ErrorCode::ItemNotFound
            | ErrorCode::ProfileError
            | ErrorCode::SearchFailed
            | ErrorCode::RipgrepNotFound
            | ErrorCode::GenericError
            | ErrorCode::RecoveryFailed
            | ErrorCode::ServerExited => ExitCode::GenericError,
        }
    }

    /// Return a concise description of the failure this member names.
    ///
    /// The match is exhaustive so adding a member requires describing it.
    pub const fn description(self) -> &'static str {
        match self {
            ErrorCode::IssueNotFound => "The requested issue does not exist.",
            ErrorCode::GateNotFound => "The requested gate does not exist.",
            ErrorCode::CycleDetected => "The dependency would create a cycle.",
            ErrorCode::InvalidArgument => "An argument or invocation is invalid.",
            ErrorCode::InvalidLabelPattern => {
                "A label query filter is not in the accepted namespace/value form."
            }
            ErrorCode::ValidationFailed => "Repository or domain validation failed.",
            ErrorCode::AlreadyExists => "The requested resource already exists.",
            ErrorCode::Blocked => "Unfinished dependencies block the operation.",
            ErrorCode::GateFailed => "A quality-gate checker did not pass.",
            ErrorCode::IoError => "An input/output or external-system operation failed.",
            ErrorCode::PermissionDenied => {
                "The operation was denied by filesystem or operating-system permissions."
            }
            ErrorCode::ParseError => "Input data could not be parsed.",
            ErrorCode::RequiresGit => "The command requires Git.",
            ErrorCode::AmbiguousId => "The ID prefix matches more than one candidate.",
            ErrorCode::InvalidIdPrefix => "The ID prefix is shorter than the accepted minimum.",
            ErrorCode::RepositoryNotFound => "No JIT repository exists at the resolved path.",
            ErrorCode::DocumentNotFound => "The requested document or commit does not exist.",
            ErrorCode::RepositoryFormatUnsupported => {
                "The repository format is not the one this binary supports."
            }
            ErrorCode::DeletionNotConfirmed => {
                "Issue deletion lacks the required operator confirmation."
            }
            ErrorCode::LinkedCheckoutWriteRefused => {
                "The linked checkout's write stance refuses state-mutating commands."
            }
            ErrorCode::ProfileConflict => {
                "Profile planning found packages being applied that define one target or \
                 declaration differently."
            }
            ErrorCode::ProfileUncommittedTargets => {
                "A profile application would overwrite paths that differ from HEAD; committing \
                 or discarding those changes lets it proceed."
            }
            ErrorCode::PresetError => "A gate-preset command failed.",
            ErrorCode::ItemNotFound => "An issue item-address lookup did not resolve an item.",
            ErrorCode::ProfileError => {
                "A profile command failed without a more specific classification."
            }
            ErrorCode::SearchFailed => "The search backend failed while executing a query.",
            ErrorCode::RipgrepNotFound => "The external ripgrep search tool was not found.",
            ErrorCode::GenericError => {
                "A command failed without a more specific public classification."
            }
            ErrorCode::RecoveryFailed => {
                "Applying a leftover redo record, or a recovery routine, failed."
            }
            ErrorCode::ServerExited => {
                "The foreground server process exited with a failure status."
            }
            ErrorCode::LockTimeout => {
                "A lock wait expired while another jit process held the lock; nothing was changed."
            }
            ErrorCode::RetryableConflict => "The repository changed or is busy; retry. If a pending redo record persists, run `jit recover`.",
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_support {
    /// Repo-relative path of the committed adopter reference for [`super::ErrorCode`].
    pub const ERROR_CODE_REFERENCE_PATH: &str = "docs/reference/error-codes.md";

    /// The command that renders [`ERROR_CODE_REFERENCE_PATH`] from [`super::ErrorCode`],
    /// named in the conformance test's message so a stale reference carries its own
    /// repair.
    pub const ERROR_CODE_REFERENCE_GENERATOR: &str = "./scripts/generate-error-code-reference.sh";
}

/// Render the complete machine-readable error-code vocabulary as adopter
/// documentation.
///
/// Each row reads its wire spelling, meaning, and numeric exit status from
/// [`ErrorCode::as_str`], [`ErrorCode::description`], and
/// [`ErrorCode::exit_code`] respectively. Iterating [`ErrorCode::ALL`] keeps the
/// page coupled to the runtime vocabulary declaration.
#[cfg(any(test, feature = "test-support"))]
pub fn render_error_code_reference() -> String {
    let rows = ErrorCode::ALL
        .iter()
        .map(|code| {
            format!(
                "| `{}` | {} | `{}` |",
                code.as_str().replace('|', "\\|"),
                code.description().replace('|', "\\|"),
                code.exit_code().code(),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "<!-- Generated from `ErrorCode::ALL`, `as_str`, `description`, and `exit_code` in \
         `jit::output` — do not edit by hand. -->\n\
         \n\
         # Machine-readable Error Codes\n\
         \n\
         > **Diátaxis Type:** Reference\n\
         \n\
         These are the values written to `error.code` in a machine-readable failure envelope.\n\
         Each code determines the process exit status shown in the final column. For the exit\n\
         status taxonomy and command-specific exceptions, see [Exit Codes](exit-codes.md).\n\
         \n\
         | Error code | Meaning | Exit status |\n\
         | --- | --- | --- |\n\
         {rows}\n"
    )
}

impl From<ErrorCode> for String {
    fn from(code: ErrorCode) -> Self {
        code.as_str().to_string()
    }
}

/// Helper to create common error responses
impl JsonError {
    /// Preserve unresolved request text in the lookup diagnostic.
    pub fn issue_not_found(requested: &str) -> Self {
        Self::new(
            ErrorCode::IssueNotFound,
            format!("Issue not found: {}", requested),
        )
        .with_details(serde_json::json!({"issue_id": requested}))
        .with_suggestion("Run 'jit query all' to see available issues")
        .with_suggestion("Check if the issue ID is correct")
    }

    /// Preserve the rejected target request text alongside the resolved source.
    pub fn cycle_detected(from: &IssueId, requested: &str) -> Self {
        Self::new(
            ErrorCode::CycleDetected,
            format!(
                "Adding dependency would create a cycle: {} -> {}",
                from, requested
            ),
        )
        .with_details(serde_json::json!({"from": from, "to": requested}))
        .with_suggestion("Remove existing dependencies that create the cycle")
        .with_suggestion("Use 'jit graph show' to visualize the dependency graph")
    }

    /// Build a JSON error for a blocked state transition.
    ///
    /// This keeps machine-readable error shaping in the output layer while the
    /// command layer returns typed blocker data.
    pub fn transition_blocked(blocked: &TransitionBlockedError) -> Self {
        Self::new(blocked.error_code(), blocked.summary())
            .with_details(serde_json::json!({
                "issue_id": blocked.issue_id(),
                "requested_state": blocked.requested_state().as_str(),
                "actual_state": blocked.actual_state().as_str(),
                "blockers": blocked.blockers().iter().map(transition_blocker_json).collect::<Vec<_>>(),
                "warnings": blocked.warnings(),
                "remediation": blocked.remediation_commands(),
            }))
            .with_suggestions(blocked.remediation_commands())
    }
}

fn transition_blocker_json(blocker: &TransitionBlocker) -> serde_json::Value {
    match blocker {
        TransitionBlocker::Dependency {
            issue_id,
            title,
            state,
        } => serde_json::json!({
            "type": "dependency",
            "issue_id": issue_id,
            "short_id": issue_id.short_id(),
            "title": title,
            "state": state.as_str(),
        }),
        TransitionBlocker::MissingDependency { issue_id } => serde_json::json!({
            "type": "dependency",
            "issue_id": issue_id,
            "short_id": issue_id.short_id(),
            "title": "(missing issue)",
            "state": "missing",
        }),
        TransitionBlocker::Gate {
            gate_key,
            status,
            mode,
        } => serde_json::json!({
            "type": "gate",
            "key": gate_key,
            "status": status.as_str(),
            "mode": mode.as_str(),
        }),
        TransitionBlocker::GraphRule { rule, message } => serde_json::json!({
            "type": "graph_rule",
            "rule": rule,
            "message": message,
        }),
        TransitionBlocker::ArchivedRevive { origin } => serde_json::json!({
            "type": "archived_revive",
            "origin": origin.as_str(),
            "message": format!(
                "an archived issue only revives to its pre-archive state '{}'",
                origin.as_str()
            ),
        }),
    }
}

// ============================================================================
// Query Response Types
// ============================================================================

/// Machine-readable result of `jit init`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct InitResponse {
    /// Repository directory initialization ran against.
    pub repository_root: String,
    /// Selected repository data directory.
    pub data_dir: String,
    /// Machine-local worktree identity, when inside a Git worktree.
    pub repository_id: Option<String>,
    /// Exact disposition of the worktree Git-attributes claim.
    pub gitattributes_status: crate::repository_state::GitattributesStatus,
    /// Files absent before this run and created by it.
    pub created_paths: Vec<String>,
    /// Existing files modified by this run.
    pub modified_paths: Vec<String>,
    /// Applied profile results when initialization included a profile, one per
    /// package of its dependency closure.
    pub profile: Option<crate::profile::ProfileComposedApplyResult>,
}

/// Response for `status` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusResponse {
    pub backlog: usize,
    pub ready: usize,
    pub in_progress: usize,
    pub gated: usize,
    pub done: usize,
    pub rejected: usize,
    pub blocked: usize,
    pub total: usize,
}

impl From<crate::commands::StatusSummary> for StatusResponse {
    fn from(summary: crate::commands::StatusSummary) -> Self {
        let crate::commands::StatusSummary {
            backlog,
            ready,
            in_progress,
            gated,
            done,
            rejected,
            blocked,
            total,
        } = summary;
        Self {
            backlog,
            ready,
            in_progress,
            gated,
            done,
            rejected,
            blocked,
            total,
        }
    }
}

impl HumanRender for StatusResponse {
    fn render_human(&self, _: OutputMode) -> String {
        format!(
            "Status:\n  backlog: {}\n  ready: {}\n  in_progress: {}\n  done: {}\n  rejected: {}\n  blocked: {}\n",
            self.backlog, self.ready, self.in_progress, self.done, self.rejected, self.blocked
        )
    }
}

/// The query an issue search echoes beside its matches.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueSearchQuery {
    pub query: Option<String>,
}

collection_keys! {
    /// Collection key of [`GraphDownstreamResponse`].
    DependentsKey = "dependents" as "GraphDownstreamResponse";
    IssuesKey = "issues";
    /// Collection key of [`IssueListResponse`].
    ListedIssuesKey = "issues" as "IssueListResponse";
    /// Collection key of [`BlockedListResponse`].
    BlockedIssuesKey = "issues" as "BlockedListResponse";
    /// Collection key of [`IssueSearchResponse`].
    SearchedIssuesKey = "issues" as "IssueSearchResponse";
    DivergencesKey = "divergences";
    GatesKey = "gates";
    /// Collection key of [`GateCheckAllResponse`].
    GateReadinessKey = "gates" as "GateCheckAllResponse";
    ResultsKey = "results";
    /// Collection key of [`GateEvaluateManyResponse`].
    GateEvaluationsKey = "results" as "GateEvaluateManyResponse";
    /// Collection key of [`NamespacesResponse`].
    NamespacesKey = "namespaces" as "NamespacesResponse";
    /// Collection key of [`LabelValuesResponse`].
    ValuesKey = "values" as "LabelValuesResponse";
    /// Collection key of [`WorktreeListResponse`].
    WorktreesKey = "worktrees" as "WorktreeListResponse";
    /// Collection key of [`GatePresetListResponse`].
    PresetsKey = "presets" as "GatePresetListResponse";
}

/// Generic response for issue list queries (available, all, ready, etc.)
/// Uses MinimalIssue for efficiency - contains only id, title, state, priority
pub type IssueListResponse = ListEnvelope<ListedIssuesKey, MinimalIssue>;

/// Full-record response for `jit query … --full --json`.
///
/// Where the default query shape ([`IssueListResponse`]) emits lean
/// [`MinimalIssue`] entries with no gate fields, `--full` hands back complete
/// stored [`Issue`] records, so the gate list appears under the storage names
/// `gates_required` / `gates_status` — the same shape `.jit/issues/<id>.json`
/// carries. The two shapes are declared side by side in `jit --schema`
/// (@/inv/single-source-prose: the full arm is derived from the [`Issue`]
/// struct, not a hand-written mirror).
pub type IssueListFullResponse = ListEnvelope<IssuesKey, Issue>;

/// Summary response for `jit issue search … --json` (no `--full`).
///
/// The [`IssueListResponse`] shape plus the echoed `query`: lean
/// [`MinimalIssue`] entries with no gate fields.
pub type IssueSearchResponse = ListEnvelope<SearchedIssuesKey, MinimalIssue, IssueSearchQuery>;

/// Full-record response for `jit issue search … --full --json`.
///
/// The search counterpart of [`IssueListFullResponse`]: complete stored
/// [`Issue`] records (gate list under the storage names `gates_required` /
/// `gates_status`) plus the echoed `query`. Declared alongside
/// [`IssueSearchResponse`] in `jit --schema`.
pub type IssueSearchFullResponse = ListEnvelope<IssuesKey, Issue, IssueSearchQuery>;

/// Response for `jit apply --json`.
///
/// `created_issues` maps each template role to the complete stored [`Issue`]
/// record created for it, so each carries the gate list under the storage names
/// `gates_required` / `gates_status` (the raw-record shape). Declared in
/// `jit --schema`; deriving `created_issues` from [`Issue`] keeps the gate field
/// names in lockstep with the serialized record.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TemplateApplyResponse {
    pub template: String,
    pub container: IssueId,
    pub anchor_bindings: std::collections::BTreeMap<String, IssueId>,
    pub created_node_ids_by_role: std::collections::BTreeMap<String, IssueId>,
    pub anchor_dependency_snapshots: std::collections::BTreeMap<String, Vec<IssueId>>,
    pub created_issues: std::collections::BTreeMap<String, Issue>,
    /// Container dependencies a transform moved onto a created node; absent
    /// when nothing moved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_move: Option<UpstreamMoveReport>,
}

impl TemplateApplyResponse {
    /// The `--json` envelope's `message`.
    pub fn message(&self) -> String {
        format!("Applied template '{}' to {}", self.template, self.container)
    }
}

/// Response for blocked query with reasons (minimal issue + reasons)
pub type BlockedListResponse = ListEnvelope<BlockedIssuesKey, MinimalBlockedIssue>;

/// One [`BlockedListResponse`] reason string. A gate reason keeps the status
/// variant name (`gate:<key> (Pending)`) the JSON contract carries; human output
/// renders the wire token through [`BlockingReason`]'s `Display`.
pub fn minimal_blocked_reason(reason: &BlockingReason) -> String {
    match reason {
        BlockingReason::Gate { key, status } => format!("gate:{key} ({status:?})"),
        BlockingReason::Dependency { .. } => reason.to_string(),
    }
}

/// Multi-id envelope for `issue show <id> <id> …` — full show projections in
/// argument order.
pub type IssueShowListResponse = ListEnvelope<IssuesKey, IssueShowResponse>;

/// Multi-id envelope for `issue status <id> <id> …` — status projections in
/// argument order.
pub type IssueStatusListResponse = ListEnvelope<IssuesKey, IssueStatusResponse>;

/// Response for `query blocked --full`: reason-enriched entries. Neither
/// blocked shape carries the issue's gate-list fields — a blocking gate
/// appears only as a structured reason — so this command belongs to neither
/// gate-field rule in the storage reference.
pub type BlockedFullListResponse = ListEnvelope<IssuesKey, BlockedIssue>;

/// Issue with blocking reasons (minimal version for lists)
#[derive(Debug, Serialize, JsonSchema)]
pub struct BlockedIssue {
    #[serde(flatten)]
    pub issue: MinimalIssue,
    pub blocked_reasons: Vec<BlockedReason>,
}

/// Reason why an issue is blocked
#[derive(Debug, Serialize, JsonSchema)]
pub struct BlockedReason {
    #[serde(rename = "type")]
    pub reason_type: BlockedReasonType,
    pub detail: String,
}

impl From<&BlockingReason> for BlockedReason {
    fn from(reason: &BlockingReason) -> Self {
        match reason {
            BlockingReason::Dependency { id, title, state } => Self {
                reason_type: BlockedReasonType::Dependency,
                detail: format!("{id} ({title}:{state})"),
            },
            BlockingReason::Gate { key, status } => Self {
                reason_type: BlockedReasonType::Gate,
                detail: format!("{key} ({status})"),
            },
        }
    }
}

/// Type of blocking reason
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BlockedReasonType {
    Dependency,
    Gate,
}

// ============================================================================
// Graph Response Types
// ============================================================================

/// Response for `graph downstream` command
pub type GraphDownstreamResponse = ListEnvelope<DependentsKey, MinimalIssue, IssueScope>;

/// The issue a graph traversal starts from.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueScope {
    pub issue_id: IssueId,
}

impl HumanRender for DivergenceResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        let rows = self
            .items
            .iter()
            .map(|d| format!("  {} | {} | {}\n", d.short_id, d.label, d.title))
            .collect::<String>();
        let header = match self.items.is_empty() {
            true => "No membership/DAG divergences".to_owned(),
            false => format!(
                "{} membership label(s) not backed by the DAG:",
                self.count()
            ),
        };
        if mode.quiet {
            rows
        } else {
            format!("{header}\n{rows}")
        }
    }
}

// ============================================================================
// Issue Show Response
// ============================================================================

/// Response for `issue create`: the created issue's view and its findings.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueCreateResponse {
    #[serde(flatten)]
    pub issue: IssueShowResponse,
    pub findings: Vec<crate::validation::ReportedFinding>,
}

/// Render a single top-level field of a serialized issue value as plain text.
///
/// `value` must be the JSON object produced by serializing an
/// [`IssueShowResponse`]. The field is looked up by its serialized key (e.g.
/// `state`, `title`, `labels`). String fields render as their raw contents and
/// scalar fields (number/bool/null) as their textual value; array and object
/// fields fall back to compact JSON, since a plain-text rendering of those is
/// ambiguous. An unknown key returns `None` so callers can signal a usage error.
pub fn project_field(value: &Value, name: &str) -> Option<String> {
    value.get(name).map(|field| match field {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    })
}

/// Error returned when a requested projection field is not a known top-level
/// field of the serialized response. Carries the offending field name.
#[derive(Debug, Error, PartialEq)]
#[error("unknown field '{0}'")]
pub struct UnknownFieldError(pub String);

/// Render the named top-level fields of a serialized response as a single
/// compact JSON object, preserving the requested order.
///
/// `value` must be a serialized JSON object. Each name is looked up by its key. The
/// first unknown key is returned as `Err(UnknownFieldError(name))` so callers
/// can signal a usage error; otherwise the result is a compact object
/// `{"a":...,"b":...}` whose keys keep the requested order (so the output is
/// stable regardless of the `serde_json` map-ordering feature).
pub fn project_fields(value: &Value, names: &[String]) -> Result<String, UnknownFieldError> {
    let pairs = names
        .iter()
        .map(|name| {
            value
                .get(name)
                .map(|field| (name.as_str(), field))
                .ok_or_else(|| UnknownFieldError(name.clone()))
        })
        .collect::<Result<Vec<_>, UnknownFieldError>>()?;

    // Build the object by hand so the requested key order is preserved
    // regardless of whether `serde_json`'s `preserve_order` feature is enabled.
    // `Value::to_string` yields compact JSON for both the key and the value.
    let body = pairs
        .iter()
        .map(|(name, field)| format!("{}:{}", Value::String((*name).to_string()), field))
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!("{{{body}}}"))
}

/// Render a resolved `jit config get` value as plain text.
///
/// A scalar (string/number/bool/null) renders bare, mirroring
/// [`project_field`]'s convention (a string's raw contents, unquoted, rather
/// than its `Display` which would include the JSON quotes). An array or
/// object — returned for an intermediate key, e.g. `jit config get
/// documentation` — has no sensible bare rendering, so it falls back to
/// PRETTY-printed JSON (unlike [`project_field`]'s compact fallback): a
/// config section is read by a human at a terminal far more often than
/// parsed by a script, and scripts should pass `--json` anyway.
pub fn render_config_get_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

/// Render an effective-configuration snapshot (the `jit config get` surface)
/// as `[section]` blocks of `key = value` lines; a nested value renders as
/// compact JSON.
pub fn render_config_snapshot(snapshot: &Value) -> String {
    let leaf = |v: &Value| match v {
        Value::Object(_) | Value::Array(_) => v.to_string(),
        scalar => render_config_get_value(scalar),
    };
    match snapshot {
        Value::Object(sections) => sections
            .iter()
            .map(|(name, section)| match section {
                Value::Object(fields) => fields
                    .iter()
                    .fold(format!("[{name}]\n"), |out, (key, value)| {
                        format!("{out}  {key} = {}\n", leaf(value))
                    }),
                other => format!("{name} = {}\n", leaf(other)),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        other => format!("{}\n", leaf(other)),
    }
}

// ============================================================================
// Lean Issue Update / Show Summary Responses
// ============================================================================

/// Lightweight confirmation returned by `jit issue update --json`.
///
/// Mutating an issue does not need to echo the full body back; agents that
/// need it can call `jit issue show`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueUpdateResponse {
    pub id: IssueId,
    pub short_id: String,
    pub state: State,
    pub updated_at: String,
}

impl From<&Issue> for IssueUpdateResponse {
    fn from(issue: &Issue) -> Self {
        Self {
            id: issue.id.clone(),
            short_id: issue.short_id(),
            state: issue.state,
            updated_at: issue.updated_at.to_rfc3339(),
        }
    }
}

/// Compact response returned by `jit issue show --summary --json`.
///
/// Carries the `MinimalIssue` fields plus the issue's `gates` list, but omits
/// the description and enriched dependency list. `gates` is the same field name
/// the full `issue show --json` ([`IssueShowResponse`]) exposes, projected to
/// `{key, status}` per gate: the summary drops the `last_run_at`/`exit_code`
/// run enrichment that the full view's [`crate::commands::GateView`] carries, so a consumer that
/// only needs per-gate status reads it identically on both shapes.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueShowSummaryResponse {
    pub id: IssueId,
    pub short_id: String,
    pub title: String,
    pub state: State,
    pub priority: Priority,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// One `{key, status}` entry per required gate — the summary projection of
    /// the full view's `gates`, without run enrichment.
    pub gates: Vec<GateStatusEntry>,
}

impl From<&Issue> for IssueShowSummaryResponse {
    fn from(issue: &Issue) -> Self {
        Self {
            id: issue.id.clone(),
            short_id: issue.short_id(),
            title: issue.title.clone(),
            state: issue.state,
            priority: issue.priority,
            assignee: issue
                .assignee
                .as_ref()
                .map(crate::domain::Assignee::to_string),
            labels: issue.labels.clone(),
            gates: issue
                .gates_required
                .iter()
                .map(|key| GateStatusEntry::for_required(key, issue.gates_status.get(key)))
                .collect(),
        }
    }
}

// ============================================================================
// Lean Gate Run Summary
// ============================================================================

/// Summary of a single gate run.
///
/// `stdout` and `stderr` are optional so the same shape can carry either a
/// lean (passing) record or a full (failing or `--full`) record. Build via
/// [`GateRunSummary::lean`] or [`GateRunSummary::full`].
///
/// # Examples
///
/// ```
/// use chrono::Utc;
/// use jit::declarations::GateStage;
/// use jit::domain::{GateEvaluationStability, GateRunResult, GateRunStatus, GateVerdictOrigin};
/// use jit::output::GateRunSummary;
///
/// let run = GateRunResult {
///     schema_version: 1,
///     run_id: jit::test_utils::gate_run_id(1),
///     gate_key: "tests".parse().unwrap(),
///     stage: GateStage::Postcheck,
///     issue_id: jit::test_utils::issue_id(0x4000_8000_0000_0000_0001_1234_5678),
///     commit: None,
///     branch: None,
///     status: GateRunStatus::Passed,
///     started_at: Utc::now(),
///     completed_at: None,
///     duration_ms: None,
///     exit_code: Some(0),
///     stdout: "lots of output".into(),
///     stderr: String::new(),
///     command: "cargo test".into(),
///     by: None,
///     message: None,
///     findings: None,
///     inputs_digest: None,
///     origin: GateVerdictOrigin::Executed,
///     evaluation_stability: GateEvaluationStability::Stable,
/// };
/// // Lean form drops stdout/stderr for passing runs.
/// let lean = GateRunSummary::lean(&run);
/// assert!(lean.stdout.is_none());
/// // Full form keeps everything.
/// let full = GateRunSummary::full(&run);
/// assert_eq!(full.stdout.as_deref(), Some("lots of output"));
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateRunSummary {
    pub run_id: crate::domain::identity::GateRunId,
    pub key: GateKey,
    pub stage: GateStage,
    pub status: GateRunStatus,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<Assignee>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    /// Structured findings parsed from the checker's machine-readable block, if
    /// one was emitted. Kept even in the lean form (which drops the raw
    /// stdout/stderr blob), so rework loops read verdict + findings as data
    /// without re-grepping a report that a passing lean summary omits entirely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub findings: Option<GateFindings>,
    /// Digest of the repository files the gate declares its checker reads.
    /// Omitted when the gate declares none. Two summaries carrying the same
    /// value describe evaluations over identical content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs_digest: Option<crate::domain::repository_inputs::InputsDigest>,
    /// Whether this run executed the checker (`{"derivation": "executed"}`),
    /// carried an earlier run's verdict
    /// (`{"derivation": "reused", "source_issue": "<issue-id>", "source_run": "<run-id>"}`), or records a
    /// manual gate's attestation (`{"derivation": "attested"}`). A reader
    /// tallying how many times a gate was actually verified counts the executed
    /// and attested ones and follows the named run for the report text behind a
    /// reused verdict.
    pub origin: crate::domain::GateVerdictOrigin,
    /// Whether the evaluation inputs still matched the repository when the run
    /// was recorded. `inputs_moved` means the verdict remains the checker's
    /// verdict over the pre-run image, while publication used a later image.
    pub evaluation_stability: crate::domain::GateEvaluationStability,
}

impl GateRunSummary {
    /// Build a summary that drops stdout/stderr for passing runs but keeps
    /// them for failed/error runs so diagnostics survive. Structured findings
    /// are retained regardless of status.
    pub fn lean(r: &GateRunResult) -> Self {
        let include_output = !matches!(r.status, GateRunStatus::Passed);
        Self::build(r, include_output)
    }

    /// Build a summary that always includes stdout/stderr.
    pub fn full(r: &GateRunResult) -> Self {
        Self::build(r, true)
    }

    fn build(r: &GateRunResult, include_output: bool) -> Self {
        Self {
            run_id: r.run_id.clone(),
            key: r.gate_key.clone(),
            stage: r.stage,
            status: r.status,
            started_at: r.started_at.to_rfc3339(),
            completed_at: r.completed_at.map(|t| t.to_rfc3339()),
            duration_ms: r.duration_ms,
            exit_code: r.exit_code,
            command: r.command.clone(),
            commit: r.commit.clone(),
            branch: r.branch.clone(),
            by: r.by.clone(),
            message: r.message.clone(),
            stdout: include_output.then(|| r.stdout.clone()),
            stderr: include_output.then(|| r.stderr.clone()),
            findings: r.findings.clone(),
            inputs_digest: r.inputs_digest.clone(),
            origin: r.origin.clone(),
            evaluation_stability: r.evaluation_stability,
        }
    }
}

/// JSON payload of `jit gate status-all --json`.
///
/// `results` contains one [`GateRunSummary`] per recorded AUTOMATED run;
/// `gates` covers EVERY required gate (automated and manual) with its
/// readiness status, so manual gates are represented too. `total`, `passed`,
/// and `not_run` are computed over all required gates: `total` is their count,
/// `passed` how many are green, and `not_run` the keys still pending.
/// `all_passed` mirrors the strict exit contract (0 iff true, else 4).
///
/// `count` is the length of the `gates` collection (one entry per required
/// gate). It equals `total` whenever every required gate has a status entry;
/// `total`/`passed` remain the readiness tallies.
pub type GateCheckAllResponse = ListEnvelope<GateReadinessKey, GateStatusEntry, GateCheckSummary>;

/// Fields of [`GateCheckAllResponse`] beside its `gates`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateCheckSummary {
    pub results: Vec<GateRunSummary>,
    pub passed: usize,
    pub total: usize,
    pub not_run: Vec<GateKey>,
    pub all_passed: bool,
}

/// A resolved issue id or the request text that failed lookup, serialized as a string.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum BatchTarget {
    Resolved(IssueId),
    Unresolved(String),
}

impl BatchTarget {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Resolved(id) => id.as_str(),
            Self::Unresolved(requested) => requested.as_str(),
        }
    }
}

/// One per-issue result from `jit gate evaluate-many`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateEvaluateManyResult {
    #[serde(rename = "issue_id")]
    pub target: BatchTarget,
    /// Gate key evaluated for this issue.
    pub key: GateKey,
    /// `passed` when the gate passed, otherwise `failed`.
    pub status: GateStatus,
    /// `pass` or `fail`/`error` when a checker produced a verdict. Pre-verdict
    /// lookup and argument errors leave this absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    /// The classified `error` object of the entry's failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

impl GateEvaluateManyResult {
    pub fn passed(target: BatchTarget, key: GateKey) -> Self {
        Self {
            target,
            key,
            status: GateStatus::Passed,
            verdict: Some("pass".to_owned()),
            error: None,
        }
    }

    /// A failed entry carrying `failure`, the classified envelope of its error.
    pub fn failed(
        target: BatchTarget,
        key: GateKey,
        failure: &JsonError,
    ) -> serde_json::Result<Self> {
        let verdict = failure
            .error
            .details
            .as_ref()
            .and_then(|details| details.get("verdict"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(Self {
            target,
            key,
            status: GateStatus::Failed,
            verdict,
            error: Some(serde_json::to_value(&failure.error)?),
        })
    }
}

/// Report of `jit gate evaluate-many`: the success payload, or `error.details`
/// when an entry failed.
///
/// `results` holds one entry per requested issue, in argument order.
pub type GateEvaluateManyResponse = ListEnvelope<GateEvaluationsKey, GateEvaluateManyResult>;

/// JSON payload of `jit gate status --all` / `--limit` (the history view).
///
/// `results` holds one [`GateRunSummary`] per matching run, newest-first, after
/// any `--gate` / `--status` filtering and `--limit` capping. Each summary is the
/// full form (stdout/stderr included) so history inspection never loses report
/// text. `count` is `results.len()` (the number returned, after filtering).
pub type GateRunHistoryResponse = ListEnvelope<ResultsKey, GateRunSummary>;

/// JSON payload of `jit gate status`'s flat report-text view
/// (`--stdout` / `--stderr` / `--tail`).
///
/// Carries the latest run's stored report text for the selected stream(s),
/// already tail-trimmed when `--tail <N>` was supplied. `stdout` / `stderr` are
/// present only for the streams the caller asked for, so the JSON mirrors the
/// plain verbatim output exactly.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateFlatReportResponse {
    pub key: GateKey,
    pub run_id: crate::domain::identity::GateRunId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

/// JSON payload of `jit gate status <id> <gate> --findings`.
///
/// The structured-findings view of a single gate's latest run. `verdict`,
/// `summary`, and `findings` come straight from the checker's machine-readable
/// block ([`GateFindings`]); they are present only
/// when that run carried a block. `has_findings` is `false` for a plain-text
/// checker (no block emitted) so a consumer can distinguish "ran, no structured
/// findings" from "no findings recorded", without inspecting the optional
/// fields.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateFindingsResponse {
    pub key: GateKey,
    pub run_id: crate::domain::identity::GateRunId,
    /// Whether the latest run carried a machine-readable findings block.
    pub has_findings: bool,
    /// Checker-declared verdict; `None` when no block was emitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    /// One-line run summary; `None` when no block was emitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Individual findings; empty when no block was emitted.
    pub findings: Vec<crate::domain::gate_findings::GateFinding>,
}

impl GateFindingsResponse {
    /// The findings view of `run`.
    pub fn of_run(run: &GateRunResult) -> Self {
        Self {
            key: run.gate_key.clone(),
            run_id: run.run_id.clone(),
            has_findings: run.findings.is_some(),
            verdict: run.findings.as_ref().map(|f| f.verdict.clone()),
            summary: run.findings.as_ref().map(|f| f.summary.clone()),
            findings: run
                .findings
                .as_ref()
                .map_or_else(Vec::new, |f| f.findings.clone()),
        }
    }
}

// ============================================================================
// Gate Registry Response Types
// ============================================================================

/// Response for `gate list` command
pub type GateListResponse = ListEnvelope<GatesKey, GateDefinition>;

/// Gate definition structure (for registry responses)
///
/// Serialized as snake_case JSON; `stage` and `mode` are typed enums so the
/// serialized form is always well-formed and identical to the registry storage
/// format.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateDefinition {
    pub key: GateKey,
    pub title: String,
    pub description: String,
    /// Execution stage; serializes as its [`GateStage`] token.
    pub stage: GateStage,
    /// Execution mode; serializes as its [`GateMode`] token.
    pub mode: GateMode,
    /// Checker configuration for automated gates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checker: Option<crate::declarations::GateChecker>,
    /// Repository files this gate's checker reads. Omitted when the gate
    /// declares none, which is what keeps its checker executing on every
    /// evaluation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs: Option<crate::domain::repository_inputs::RepositoryInputs>,
}

impl From<crate::declarations::GateDefinition> for GateDefinition {
    fn from(gate: crate::declarations::GateDefinition) -> Self {
        Self {
            key: gate.key,
            title: gate.title,
            description: gate.description,
            stage: gate.stage,
            mode: gate.mode,
            checker: gate.checker,
            inputs: gate.inputs,
        }
    }
}

// ============================================================================
// Document Response Types
// ============================================================================

// ============================================================================
// Label Response Types
// ============================================================================

/// Response for `label namespaces` command.
pub type NamespacesResponse = ListEnvelope<NamespacesKey, String, NamespacesHumanDetails>;

/// Configuration kept only for rendering the human namespace list.
#[derive(Debug, Serialize, JsonSchema)]
pub struct NamespacesHumanDetails {
    #[serde(skip)]
    #[schemars(skip)]
    pub namespaces: Vec<crate::domain::LabelNamespace>,
}

impl HumanRender for NamespacesResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        let header = if mode.quiet {
            String::new()
        } else {
            "Label Namespaces:\n\n".to_owned()
        };
        let rows = self
            .items
            .iter()
            .zip(&self.extras.namespaces)
            .map(|(name, namespace)| {
                format!(
                    "  {name}\n    Description: {}\n    Unique: {}\n\n",
                    namespace.description, namespace.unique
                )
            })
            .collect::<String>();
        format!("{header}{rows}")
    }
}

/// Response for `gate preset list` command
pub type GatePresetListResponse = ListEnvelope<PresetsKey, crate::gate_presets::PresetInfo>;

/// Collection key of [`EventListResponse`]. Its schema keeps the response's
/// name, since no other response carries the `events` key.
pub enum EventsKey {}

impl CollectionKey for EventsKey {
    const KEY: &'static str = "events";
    const SCHEMA_NAME: Option<&'static str> = Some("EventListResponse");
}

/// Response for `events tail` and `events query`
pub type EventListResponse = ListEnvelope<EventsKey, crate::domain::Event>;

/// Response for `label values` command
pub type LabelValuesResponse = ListEnvelope<ValuesKey, String, LabelValuesScope>;

/// The namespace a [`LabelValuesResponse`] lists values of.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LabelValuesScope {
    pub namespace: String,
}

impl HumanRender for LabelValuesResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        let header = if mode.quiet {
            String::new()
        } else {
            format!("Values in namespace '{}':\n\n", self.extras.namespace)
        };
        let values = self
            .items
            .iter()
            .map(|value| format!("  {value}\n"))
            .collect::<String>();
        let total = if mode.quiet {
            String::new()
        } else {
            format!("\nTotal: {}\n", self.count())
        };
        format!("{header}{values}{total}")
    }
}

/// Response for `worktree list` command
pub type WorktreeListResponse =
    ListEnvelope<WorktreesKey, crate::commands::worktree::WorktreeListEntry>;

/// `worktree exec` writes no response: its child owns stdout and stderr, and
/// the child's exit status is the run's.
#[derive(Debug, Serialize, JsonSchema)]
pub enum WorktreeExecResponse {}

/// Response for `worktree store-divergence`: every record the two stores
/// disagree about, beside the stores that were compared.
pub type WorktreeStoreDivergenceResponse =
    ListEnvelope<DivergencesKey, StoreDivergence, StoreComparison>;

/// The stores a [`WorktreeStoreDivergenceResponse`] compared.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StoreComparison {
    /// The store the inspected checkout owns.
    pub checkout_store: String,
    /// The store it was compared against, absent when the checkout has none.
    pub reference_store: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::commands::DependencyTreeNode;
    /// REQ-05: the machine-readable status of a reused verdict is
    /// distinguishable from a derived one and names the run behind it.
    #[test]
    fn test_gate_run_summary_distinguishes_a_reused_verdict_from_a_derived_one() {
        use crate::commands::test_helpers::gate_reuse::ReuseFixture;

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
        let reused = fixture.evaluate(&fixture.issue("Test", "Test"));

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
            reused_json["origin"]
                .to_string()
                .contains(executed.run_id.as_str()),
            "the reused summary must name the run it was taken from: {reused_json}"
        );
        assert_eq!(
            executed_json["inputs_digest"], reused_json["inputs_digest"],
            "both summaries describe the same declared input content"
        );
    }

    use super::*;
    use schemars::schema_for;
    use serde_json::json;
    use std::collections::BTreeSet;

    fn divergence_view(label: &str) -> DivergenceView {
        let id = crate::test_utils::issue_id(7);
        DivergenceView {
            short_id: id.short_id().to_string(),
            id,
            title: "Orphan".to_owned(),
            label: label.to_owned(),
            namespace: "epic".to_owned(),
            value: "x".to_owned(),
        }
    }

    #[test]
    fn test_divergence_response_human_header_names_the_count_and_quiet_keeps_rows_only() {
        let report = DivergenceResponse::new(vec![divergence_view("epic:x")]);

        let loud = report.render_human(OutputMode::default());
        let quiet = report.render_human(OutputMode {
            json: false,
            quiet: true,
        });

        assert!(loud.starts_with("1 membership label(s)"));
        assert!(loud.ends_with(&quiet));
        assert!(quiet.contains("epic:x") && quiet.lines().count() == 1);
    }

    #[test]
    fn test_divergence_response_human_empty_reports_none_unless_quiet() {
        let report = DivergenceResponse::new(Vec::new());

        assert_eq!(
            report.render_human(OutputMode::default()),
            "No membership/DAG divergences\n"
        );
        assert_eq!(
            report.render_human(OutputMode {
                json: false,
                quiet: true
            }),
            ""
        );
    }

    #[test]
    fn test_state_rollup_human_renders_its_greppable_lines() {
        let rollup = StateRollup::from_issues(&[]);

        let human = rollup.render_human(OutputMode::default());

        assert_eq!(
            human.lines().collect::<Vec<_>>(),
            rollup
                .to_lines()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_batch_target_serializes_resolved_and_unresolved_as_plain_strings() {
        let id = crate::test_utils::issue_id(0x4000_8000_0000_0000_0001_1234_5678);
        for target in [
            BatchTarget::Resolved(id),
            BatchTarget::Unresolved("missing-prefix".into()),
        ] {
            let expected = serde_json::to_string(target.as_str()).unwrap();
            assert_eq!(serde_json::to_string(&target).unwrap(), expected);
        }
    }

    #[test]
    fn test_render_gate_run_status_never_run_and_passed_name_gate_and_issue() {
        let id = crate::test_utils::issue_id(1);
        let never = render_gate_run_status(GateRunLine::NeverRun, &"tests".parse().unwrap(), &id);
        let passed = render_gate_run_status(GateRunLine::Passed, &"tests".parse().unwrap(), &id);
        for line in [&never, &passed] {
            assert!(line.contains("'tests'") && line.contains(id.as_str()));
        }
        assert!(never.contains("has not been run yet") && never.contains("jit gate evaluate"));
        assert_eq!(passed, format!("Passed gate 'tests' for issue {id}"));
    }

    #[test]
    fn test_render_archive_advisory_lists_each_citing_file_under_its_artifact_or_nothing() {
        use crate::domain::archive_advisory::{
            ArchiveAdvisory, ArtifactCitations, CitationLocation,
        };

        assert_eq!(render_archive_advisory(&ArchiveAdvisory::default()), "");
        let locations = ["notes/x.md", "notes/y.md"].map(|citing| CitationLocation {
            citing: citing.into(),
            line: 2,
            column: 5,
        });
        let rendered = render_archive_advisory(&ArchiveAdvisory {
            citations: vec![ArtifactCitations {
                artifact: "docs/a.md".into(),
                locations: locations.to_vec(),
            }],
        });
        let lines = rendered.lines().collect::<Vec<_>>();
        assert!(lines[0].starts_with("Advisory"));
        assert!(lines[1].contains("docs/a.md"));
        for (line, location) in lines[2..].iter().zip(&locations) {
            assert!(line.contains(&location.to_string()), "{line}");
        }
        assert_eq!(lines.len(), 2 + locations.len());
    }

    #[test]
    fn test_render_archive_execution_lists_counts_and_warnings_or_none() {
        use crate::domain::artifact_execution::{ArchiveExecutionResult, ArchivePublication};
        use crate::domain::artifact_plan::{ContentIdentity, PlanTarget, PlanWarning, WarningCode};
        let result = |plan_warnings| ArchiveExecutionResult {
            schema_version: 1,
            target: PlanTarget::Document {
                path: "docs/a.md".into(),
            },
            destination_root: "archive".into(),
            publications: vec![ArchivePublication {
                source: Some("docs/a.md".into()),
                destination: "archive/a.md".into(),
                content_identity: ContentIdentity::from_bytes(b"a"),
                adopted: false,
            }],
            reference_changes: Vec::new(),
            planned_deletions: Vec::new(),
            deleted_sources: vec!["docs/a.md".into()],
            plan_warnings,
            event_appended: true,
            reconciling: false,
        };
        let summary = "Archive execution complete: 1 publication(s), 0 reference change(s), 1 source deletion(s)";

        assert_eq!(
            render_archive_execution(&result(Vec::new())),
            format!("{summary}\n")
        );

        let warned = render_archive_execution(&result(vec![
            PlanWarning::new(WarningCode::DeletionFailed, Some("docs/a.md")),
            PlanWarning::new(WarningCode::NoOwner, None::<String>),
        ]));
        assert_eq!(
            warned,
            format!(
                "{summary}\nwarning: {} (docs/a.md)\nwarning: {}\n",
                WarningCode::DeletionFailed,
                WarningCode::NoOwner
            )
        );
    }

    #[test]
    fn test_render_validation_findings_formats_lines_and_summary_by_stream() {
        use crate::declarations::rules::Severity;
        use crate::validation::report::ReportedFinding;
        let finding = |rule: &str, severity| ReportedFinding {
            issue_id: None,
            rule: rule.into(),
            severity,
            message: format!("{rule} message"),
        };
        let summary = Some(FindingsSummary {
            pass: "All passed",
            failure: "Checks failed",
        });

        let clean = render_validation_findings(&[], summary);
        assert_eq!(clean.stdout, "All passed\n");
        assert!(clean.stderr.is_empty());

        let warned = render_validation_findings(&[finding("w", Severity::Warn)], summary);
        assert_eq!(warned.stdout, "warning [w] w message\n");
        assert!(warned.stderr.is_empty());

        let failed = render_validation_findings(
            &[finding("w", Severity::Warn), finding("e", Severity::Error)],
            summary,
        );
        assert_eq!(
            failed.stdout,
            "warning [w] w message\nerror [e] e message\n"
        );
        assert_eq!(failed.stderr, "Checks failed with 1 error(s)\n");

        let bare = render_validation_findings(&[], None);
        assert_eq!(bare, RenderedFindings::default());
    }

    fn schema_property_names<T: JsonSchema>() -> BTreeSet<String> {
        schema_for!(T)
            .schema
            .object
            .expect("a response schema is an object")
            .properties
            .into_keys()
            .collect()
    }

    fn serialized_keys(value: &impl Serialize) -> BTreeSet<String> {
        serde_json::to_value(value)
            .expect("a response serializes")
            .as_object()
            .expect("a response serializes as an object")
            .keys()
            .cloned()
            .collect()
    }

    #[test]
    fn test_envelope_responses_schema_names_the_serialized_keys() {
        let search = IssueSearchResponse::with_extras(Vec::new(), IssueSearchQuery { query: None });
        assert_eq!(
            schema_property_names::<IssueSearchResponse>(),
            serialized_keys(&search)
        );

        let comparison = WorktreeStoreDivergenceResponse::with_extras(
            Vec::new(),
            StoreComparison {
                checkout_store: "a".to_string(),
                reference_store: None,
            },
        );
        assert_eq!(
            schema_property_names::<WorktreeStoreDivergenceResponse>(),
            serialized_keys(&comparison)
        );

        let rollup = StateRollup::from_issues(&[]);
        assert_eq!(
            schema_property_names::<StateRollup>(),
            serialized_keys(&rollup)
        );
    }

    /// An empty container reports percent 0 and still enumerates every state.
    #[test]
    fn test_state_rollup_from_issues_empty_counts_zero_in_every_state() {
        let rollup = StateRollup::from_issues(&[]);
        assert_eq!(rollup.extras.total, 0);
        assert_eq!(rollup.extras.percent, 0);
        assert_eq!(rollup.items.len(), State::ALL.len());
        assert!(rollup.items.iter().all(|c| c.count == 0));
    }

    /// jit:45a140ae REQ-02: a dependency-tree node archived from a terminal
    /// state renders and serializes as effectively terminal; one archived from a
    /// non-terminal state stays non-terminal.
    #[test]
    fn test_dependency_tree_node_effective_terminality() {
        let issue = crate::domain::MinimalIssue {
            id: crate::test_utils::issue_id(0x4000_8000_0000_0000_0001_aaaa_aaaa),
            title: "Archived dep".to_string(),
            state: State::Archived,
            priority: Priority::Normal,
            assignee: None,
            labels: Vec::new(),
            archived_from: Some(State::Done),
        };
        let node = DependencyTreeNode::from_minimal(&issue, 1);
        assert_eq!(node.archived_from, Some(State::Done));
        let serialized = serde_json::to_value(&node).unwrap();
        assert_eq!(serialized["archived_from"], json!("done"));
    }

    /// Status markers (check marks, crosses, warning and info signs, circles,
    /// status-colored squares). Type icons are a separate, allowed mechanism.
    const STATUS_MARKERS: &[char] = &[
        '\u{2705}',
        '\u{274C}',
        '\u{26A0}',
        '\u{2713}',
        '\u{2714}',
        '\u{2717}',
        '\u{2718}',
        '\u{2716}',
        '\u{2139}',
        '\u{25CB}',
        '\u{25CF}',
        '\u{1F310}',
        '\u{2757}',
        '\u{26D4}',
        '\u{1F6AB}',
        '\u{1F534}',
        '\u{1F7E1}',
        '\u{1F7E2}',
        '\u{23F3}',
        '\u{2B55}',
    ];

    fn rust_sources(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .flat_map(|entry| {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    rust_sources(&path)
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    vec![path]
                } else {
                    Vec::new()
                }
            })
            .collect()
    }

    #[test]
    fn test_cli_sources_contain_no_pictographic_status_marker() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let offenders: Vec<String> = rust_sources(&src)
            .iter()
            .flat_map(|path| {
                std::fs::read_to_string(path)
                    .unwrap()
                    .lines()
                    .enumerate()
                    .filter(|(_, line)| !line.trim_start().starts_with("//"))
                    .filter(|(_, line)| line.contains(STATUS_MARKERS))
                    .map(|(n, line)| format!("{}:{}: {}", path.display(), n + 1, line.trim()))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(
            offenders.is_empty(),
            "status markers must be words:\n{}",
            offenders.join("\n")
        );
    }

    #[test]
    fn test_is_broken_pipe_write_panic_matches_stdout_and_stderr_payloads_of_either_shape() {
        assert!(is_broken_pipe_write_panic(
            &"failed printing to stdout: Broken pipe (os error 32)"
        ));
        assert!(is_broken_pipe_write_panic(
            &"failed printing to stderr: Broken pipe (os error 32)".to_owned()
        ));
    }

    #[test]
    fn test_is_broken_pipe_write_panic_rejects_unrelated_panics() {
        assert!(!is_broken_pipe_write_panic(&"index out of bounds"));
        assert!(!is_broken_pipe_write_panic(
            &"failed printing to stdout: Permission denied (os error 13)"
        ));
        assert!(!is_broken_pipe_write_panic(
            &"some other message mentions Broken pipe in passing"
        ));
        assert!(!is_broken_pipe_write_panic(&32_i32));
    }

    #[test]
    fn test_exit_code_broken_pipe_is_128_plus_sigpipe() {
        // 128 + SIGPIPE(13) = 141: the exit status a shell reports for a
        // process terminated by SIGPIPE, adopted here without altering this
        // process's own signal disposition.
        assert_eq!(ExitCode::BrokenPipe.code(), 141);
    }

    #[test]
    fn test_gate_definition_json_includes_builtin_checker_configuration() {
        let gate = crate::declarations::GateDefinition {
            version: 1,
            key: "anything".parse().unwrap(),
            title: "Coverage".to_string(),
            description: "Configured indirection".to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: Some(crate::declarations::GateChecker::LabelTargetValidation {
                label_namespace: "owner".to_string(),
            }),
            priority: 100,
            reserved: Default::default(),
            inputs: None,
        };

        let encoded = serde_json::to_value(GateDefinition::from(gate.clone())).unwrap();
        assert_eq!(encoded["key"], "anything");
        assert_eq!(encoded["checker"]["type"], "label_target_validation");
        assert_eq!(encoded["checker"]["label_namespace"], "owner");

        let mut selected_rule = gate;
        selected_rule.checker = Some(crate::declarations::GateChecker::RuleValidation {
            rule: "coverage-preview".to_string(),
        });
        let encoded = serde_json::to_value(GateDefinition::from(selected_rule)).unwrap();
        assert_eq!(encoded["checker"]["type"], "rule_validation");
        assert_eq!(encoded["checker"]["rule"], "coverage-preview");
    }

    #[test]
    fn test_show_response_root_shape_short_id_and_arrays() {
        // Labels and dependencies always serialize as arrays.
        let issue = crate::test_utils::fixture_issue("T".to_string(), "B".to_string());
        let expected_short = issue.short_id();
        let resp = IssueShowResponse::from_issue(issue, vec![], &[]);
        let v = serde_json::to_value(&resp).unwrap();

        assert_eq!(v["short_id"], serde_json::Value::String(expected_short));
        assert!(v["labels"].is_array(), "labels must serialize as an array");
        assert!(
            v["dependencies"].is_array(),
            "dependencies must serialize as an array"
        );
        assert_eq!(v["labels"].as_array().unwrap().len(), 0);
        assert_eq!(v["dependencies"].as_array().unwrap().len(), 0);
        // A clean issue has no dangling deps, so the field is omitted from
        // the JSON entirely (`skip_serializing_if = "Vec::is_empty"`).
        assert!(v.get("dangling_dependency_ids").is_none());
        // Unset lifecycle timestamps are omitted from the show shape.
        assert!(v.get("first_ready_at").is_none());
        assert!(v.get("claimed_at").is_none());
        assert!(v.get("done_at").is_none());
    }

    #[test]
    fn test_show_response_surfaces_lifecycle_timestamps() {
        let mut issue = crate::test_utils::fixture_issue("T".to_string(), "B".to_string());
        let at = chrono::Utc::now();
        issue.mark_first_ready(at);
        issue.mark_claimed(at);
        issue.mark_done(at);

        let resp = IssueShowResponse::from_issue(issue, vec![], &[]);
        let v = serde_json::to_value(&resp).unwrap();

        assert_eq!(
            v["first_ready_at"],
            serde_json::Value::String(at.to_rfc3339())
        );
        assert_eq!(v["claimed_at"], serde_json::Value::String(at.to_rfc3339()));
        assert_eq!(v["done_at"], serde_json::Value::String(at.to_rfc3339()));
    }

    #[test]
    fn test_show_response_surfaces_dangling_dependency_ids_not_silently() {
        // jit:f847df3f: a dependency id that never resolved to a MinimalIssue
        // (constructed directly here via storage-level state, standing in for
        // a dangling reference left by raw storage mutation or legacy data)
        // must surface in `dangling_dependency_ids` rather than vanish when
        // `enriched_deps` only contains the resolvable ones.

        let mut issue = crate::test_utils::fixture_issue("Parent".to_string(), "Body".to_string());
        let resolved_dep = crate::test_utils::fixture_issue("Dep".to_string(), String::new());
        let dangling_id = crate::test_utils::issue_id(0x4000_8000_0000_0000_0002_2222_2222);
        issue.dependencies = vec![resolved_dep.id.clone(), dangling_id.clone()];
        let enriched_deps = vec![MinimalIssue::from(&resolved_dep)];

        let resp = IssueShowResponse::from_issue(issue, enriched_deps, &[]);
        let v = serde_json::to_value(&resp).unwrap();

        assert_eq!(
            v["dependencies"].as_array().unwrap().len(),
            1,
            "only the resolvable dependency appears in the enriched list"
        );
        assert_eq!(
            v["dangling_dependency_ids"],
            serde_json::json!([dangling_id]),
            "the dangling id must be reported, not silently dropped"
        );
    }

    #[test]
    fn test_show_response_omits_rejected_dependency_from_unmet() {
        use crate::domain::State;

        let mut issue = crate::test_utils::fixture_issue("Parent".to_string(), "Body".to_string());
        let mut rejected = crate::test_utils::fixture_issue("Abandoned".to_string(), String::new());
        rejected.state = State::Rejected;
        let mut pending = crate::test_utils::fixture_issue("Upstream".to_string(), String::new());
        pending.state = State::InProgress;
        issue.dependencies = vec![rejected.id.clone(), pending.id.clone()];
        let enriched_deps = vec![MinimalIssue::from(&rejected), MinimalIssue::from(&pending)];

        let resp = IssueShowResponse::from_issue(issue, enriched_deps, &[]);

        let unmet: Vec<&str> = resp
            .unmet_dependencies
            .iter()
            .map(|dep| dep.id.as_str())
            .collect();
        assert_eq!(
            unmet,
            vec![pending.id.as_str()],
            "a Rejected dependency is met and must not render as unmet"
        );
    }

    #[test]
    fn test_show_response_exposes_content_format() {
        use crate::domain::ContentFormat;
        // Set -> appears in `jit issue show --json` (create/show parity).
        let mut issue = crate::test_utils::fixture_issue("T".to_string(), "B".to_string());
        issue.content_format = Some(ContentFormat::Html);
        let resp = IssueShowResponse::from_issue(issue, vec![], &[]);
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["content_format"], "html");

        // Absent -> omitted (existing issues without the field stay clean).
        let issue2 = crate::test_utils::fixture_issue("T".to_string(), "B".to_string());
        let resp2 = IssueShowResponse::from_issue(issue2, vec![], &[]);
        let v2 = serde_json::to_value(&resp2).unwrap();
        assert!(v2.get("content_format").is_none());
    }

    #[test]
    fn test_show_response_gates_array_replaces_split_fields() {
        // `gates` replaces `gates_required`/`gates_status`; neither legacy field
        // appears in the `issue show --json` shape.
        let mut issue = crate::test_utils::fixture_issue("T".to_string(), "B".to_string());
        issue.gates_required = vec!["tests".parse().unwrap()];
        let resp = IssueShowResponse::from_issue(issue, vec![], &[]);
        let v = serde_json::to_value(&resp).unwrap();

        assert!(v["gates"].is_array(), "gates must serialize as an array");
        assert!(
            v.get("gates_required").is_none(),
            "gates_required must be gone"
        );
        assert!(v.get("gates_status").is_none(), "gates_status must be gone");
    }

    #[test]
    fn test_show_response_gate_never_run_is_null() {
        // Required-but-never-run gate: status pending, last_run_at/exit_code null.
        let mut issue = crate::test_utils::fixture_issue("T".to_string(), "B".to_string());
        issue.gates_required = vec!["tests".parse().unwrap()];
        let resp = IssueShowResponse::from_issue(issue, vec![], &[]);
        let v = serde_json::to_value(&resp).unwrap();

        let gate = &v["gates"][0];
        assert_eq!(gate["key"], "tests");
        assert_eq!(gate["status"], "pending");
        assert!(gate["last_run_at"].is_null(), "last_run_at must be null");
        assert!(gate["exit_code"].is_null(), "exit_code must be null");
    }

    #[test]
    fn test_show_response_gate_with_run_is_enriched() {
        use crate::declarations::GateStage;
        use crate::domain::{GateRunResult, GateRunStatus, GateState, GateStatus};
        use chrono::Utc;

        // A gate that has run: status from GateState, last_run_at/exit_code from
        // the GateRunResult the state names.
        let mut issue = crate::test_utils::fixture_issue("T".to_string(), "B".to_string());
        issue.gates_required = vec!["tests".parse().unwrap()];
        let run_at = Utc::now();
        let run_id = crate::test_utils::gate_run_id(1);
        issue.gates_status.insert(
            "tests".parse().unwrap(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("ci:test".parse().unwrap()),
                updated_at: run_at,
                run_id: Some(run_id.clone()),
            },
        );

        let run = GateRunResult {
            schema_version: 1,
            run_id,
            gate_key: "tests".parse().unwrap(),
            stage: GateStage::Postcheck,
            issue_id: issue.id.clone(),
            commit: None,
            branch: None,
            status: GateRunStatus::Passed,
            started_at: run_at,
            completed_at: Some(run_at),
            duration_ms: Some(42),
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            command: "cargo test".to_string(),
            by: None,
            message: None,
            findings: None,
            inputs_digest: None,
            origin: crate::domain::GateVerdictOrigin::Executed,
            evaluation_stability: crate::domain::GateEvaluationStability::Stable,
        };

        let resp = IssueShowResponse::from_issue(issue, vec![], std::slice::from_ref(&run));
        let v = serde_json::to_value(&resp).unwrap();

        let gate = &v["gates"][0];
        assert_eq!(gate["key"], "tests");
        assert_eq!(gate["status"], "passed");
        assert_eq!(gate["exit_code"], 0);
        assert_eq!(gate["last_run_at"], run_at.to_rfc3339());
    }

    #[test]
    fn test_json_output_success() {
        let id = crate::test_utils::issue_id(1);
        let data = json!({"id": id, "title": "Test"});
        let output = JsonOutput::success(data);

        // success field removed
        assert_eq!(output.data["id"], id.as_str());
        // metadata removed
    }

    #[test]
    fn test_json_output_serialization() {
        let id = crate::test_utils::issue_id(1);
        let data = json!({"id": id, "title": "test"});
        let output = JsonOutput::success(data);

        let json_str = output.to_json_string().unwrap();
        // Should contain raw data without envelope
        assert!(json_str.contains(&format!("\"id\": \"{id}\"")));
        assert!(json_str.contains("\"title\": \"test\""));
        // Should NOT contain envelope fields
        assert!(!json_str.contains("\"success\""));
        assert!(!json_str.contains("\"data\":"));
    }

    #[test]
    fn test_gate_findings_response_preserves_populated_references_and_omits_empty() {
        use crate::domain::gate_findings::GateFinding;

        let finding = |id: &str, references: Vec<String>| GateFinding {
            id: id.to_string(),
            severity: "high".to_string(),
            disposition: Some("blocking".to_string()),
            origin: Some("issue-impact".to_string()),
            summary: "defect".to_string(),
            file: None,
            line: None,
            references,
        };
        let response = GateFindingsResponse {
            key: "review".parse().unwrap(),
            run_id: crate::test_utils::gate_run_id(1),
            has_findings: true,
            verdict: Some("fail".to_string()),
            summary: Some("two defects".to_string()),
            findings: vec![
                finding(
                    "F1",
                    vec![
                        "@/inv/atomic-writes".to_string(),
                        "checker:opaque value".to_string(),
                    ],
                ),
                finding("F2", Vec::new()),
            ],
        };

        let encoded = serde_json::to_value(response).unwrap();

        assert_eq!(
            encoded["findings"][0]["references"],
            serde_json::json!(["@/inv/atomic-writes", "checker:opaque value"])
        );
        assert!(encoded["findings"][1].get("references").is_none());
    }

    #[test]
    fn test_json_output_with_message() {
        let id = crate::test_utils::issue_id(1);
        let data = json!({"id": id, "title": "Test issue"});
        let message = format!("Created issue {id} - Test issue");
        let output = JsonOutput::success(data).with_message(&message);

        let json_str = output.to_json_string().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["message"], message);
        assert_eq!(parsed["id"], id.as_str());
        assert_eq!(parsed["title"], "Test issue");
    }

    #[test]
    fn test_json_output_without_message() {
        let data = json!({"id": crate::test_utils::issue_id(1)});
        let output = JsonOutput::success(data);

        let json_str = output.to_json_string().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.get("message").is_none());
    }

    #[test]
    fn test_json_output_with_message_array_payload() {
        // When data serializes to a JSON array (not object), message cannot be injected.
        // Verify the output is still valid JSON (the array), just without message.
        let data =
            json!([{"id": crate::test_utils::issue_id(1)}, {"id": crate::test_utils::issue_id(2)}]);
        let output = JsonOutput::success(data).with_message("Should not appear");

        let json_str = output.to_json_string().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.is_array(), "Array payload should serialize as array");
        assert_eq!(parsed.as_array().unwrap().len(), 2);
        // message is silently dropped for non-object payloads
    }

    #[test]
    fn test_json_error_basic() {
        let error = JsonError::new(ErrorCode::GenericError, "This is a test error");

        assert_eq!(error.error.code, ErrorCode::GenericError);
        assert_eq!(error.error.message, "This is a test error");
        assert!(error.error.details.is_none());
        assert!(error.error.suggestions.is_empty());
    }

    #[test]
    fn test_json_error_with_details() {
        let error = JsonError::new(ErrorCode::IssueNotFound, "Resource not found")
            .with_details(json!({"requested_id": "abc123"}));

        assert_eq!(error.error.details, Some(json!({"requested_id": "abc123"})));
    }

    #[test]
    fn test_json_error_with_suggestions() {
        let error = JsonError::new(ErrorCode::IssueNotFound, "Issue not found")
            .with_suggestion("Run 'jit issue list' to see available issues")
            .with_suggestion("Check if the issue ID is correct");

        assert_eq!(error.error.suggestions.len(), 2);
        assert!(error.error.suggestions[0].contains("jit issue list"));
    }

    #[test]
    fn test_json_error_serialization() {
        let error = JsonError::new(ErrorCode::ValidationFailed, "Test")
            .with_details(json!({"key": "value"}))
            .with_suggestion("Try something");

        let json_str = error.to_json_string().unwrap();
        // Should have error object without envelope
        assert!(json_str.contains("\"code\": \"VALIDATION_FAILED\""));
        assert!(json_str.contains("\"message\": \"Test\""));
        assert!(json_str.contains("\"details\""));
        assert!(json_str.contains("\"suggestions\""));
        // Should NOT have envelope fields
        assert!(!json_str.contains("\"success\""));
        assert!(!json_str.contains("\"metadata\""));
        assert!(!json_str.contains("exit_code"));
        assert_eq!(error.exit_code(), ExitCode::ValidationFailed);
    }

    #[test]
    fn test_gate_failed_error_code_is_validation_failure() {
        assert_eq!(
            ErrorCode::GateFailed.exit_code(),
            ExitCode::ValidationFailed
        );
    }

    fn schema_accepted_strings(schema: &serde_json::Value) -> BTreeSet<String> {
        match schema {
            serde_json::Value::Object(map) => map
                .iter()
                .flat_map(|(key, value)| match (key.as_str(), value) {
                    ("enum", serde_json::Value::Array(values)) => values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect(),
                    _ => schema_accepted_strings(value),
                })
                .collect(),
            serde_json::Value::Array(values) => {
                values.iter().flat_map(schema_accepted_strings).collect()
            }
            _ => BTreeSet::new(),
        }
    }

    #[test]
    fn test_error_code_wire_strings_and_exit_statuses_match_contract() {
        let expected = [
            (ErrorCode::IssueNotFound, ExitCode::NotFound),
            (ErrorCode::GateNotFound, ExitCode::NotFound),
            (ErrorCode::CycleDetected, ExitCode::ValidationFailed),
            (ErrorCode::InvalidArgument, ExitCode::InvalidArgument),
            (ErrorCode::InvalidLabelPattern, ExitCode::InvalidArgument),
            (ErrorCode::ValidationFailed, ExitCode::ValidationFailed),
            (ErrorCode::AlreadyExists, ExitCode::AlreadyExists),
            (ErrorCode::Blocked, ExitCode::ValidationFailed),
            (ErrorCode::GateFailed, ExitCode::ValidationFailed),
            (ErrorCode::IoError, ExitCode::ExternalError),
            (ErrorCode::PermissionDenied, ExitCode::PermissionDenied),
            (ErrorCode::ParseError, ExitCode::GenericError),
            (ErrorCode::RequiresGit, ExitCode::ExternalError),
            (ErrorCode::AmbiguousId, ExitCode::InvalidArgument),
            (ErrorCode::InvalidIdPrefix, ExitCode::InvalidArgument),
            (ErrorCode::RepositoryNotFound, ExitCode::NotFound),
            (ErrorCode::DocumentNotFound, ExitCode::NotFound),
            (
                ErrorCode::RepositoryFormatUnsupported,
                ExitCode::ExternalError,
            ),
            (ErrorCode::DeletionNotConfirmed, ExitCode::InvalidArgument),
            (
                ErrorCode::LinkedCheckoutWriteRefused,
                ExitCode::InvalidArgument,
            ),
            (ErrorCode::ProfileConflict, ExitCode::ValidationFailed),
            (
                ErrorCode::ProfileUncommittedTargets,
                ExitCode::ValidationFailed,
            ),
            (ErrorCode::PresetError, ExitCode::NotFound),
            (ErrorCode::ItemNotFound, ExitCode::GenericError),
            (ErrorCode::ProfileError, ExitCode::GenericError),
            (ErrorCode::SearchFailed, ExitCode::GenericError),
            (ErrorCode::RipgrepNotFound, ExitCode::GenericError),
            (ErrorCode::GenericError, ExitCode::GenericError),
            (ErrorCode::RecoveryFailed, ExitCode::GenericError),
            (ErrorCode::ServerExited, ExitCode::GenericError),
            (ErrorCode::LockTimeout, ExitCode::LockTimeout),
            (ErrorCode::RetryableConflict, ExitCode::RetryableConflict),
        ];

        assert_eq!(expected.len(), ErrorCode::ALL.len());
        for ((code, status), listed) in expected.into_iter().zip(ErrorCode::ALL) {
            assert_eq!(code, listed);
            assert_eq!(code.exit_code(), status);
            let error = JsonError::new(code, "failure");
            assert_eq!(error.exit_code(), status);
            assert_eq!(
                serde_json::to_value(&error).expect("registered error should serialize")["error"]
                    ["code"],
                code.as_str()
            );
        }
    }

    #[test]
    fn test_exit_code_all_lists_every_derived_variant() {
        let schema =
            serde_json::to_value(schema_for!(ExitCode)).expect("ExitCode schema should serialize");
        let derived = schema_accepted_strings(&schema);
        assert!(!derived.is_empty(), "derived schema should list variants");

        let listed: BTreeSet<String> = ExitCode::ALL
            .iter()
            .map(|code| format!("{code:?}"))
            .collect();

        assert_eq!(listed, derived, "ExitCode::ALL must list every variant");
    }

    #[test]
    fn test_exit_code_all_members_have_distinct_described_statuses() {
        let statuses: BTreeSet<i32> = ExitCode::ALL.iter().map(|c| c.code()).collect();
        assert_eq!(statuses.len(), ExitCode::ALL.len());
        assert!(ExitCode::ALL.iter().all(|c| !c.description().is_empty()));
    }

    #[test]
    fn test_error_code_members_have_descriptions() {
        for code in ErrorCode::ALL {
            assert!(
                !code.description().is_empty(),
                "{} should have a description",
                code.as_str()
            );
        }
    }

    fn error_code_reference_path() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(test_support::ERROR_CODE_REFERENCE_PATH)
    }

    /// Every generated row is composed from the vocabulary accessors used by
    /// runtime envelopes, rather than from a second description or status map.
    #[test]
    fn test_error_code_reference_lists_runtime_vocabulary() {
        let page = render_error_code_reference();

        for code in ErrorCode::ALL {
            let row = format!(
                "| `{}` | {} | `{}` |",
                code.as_str(),
                code.description(),
                code.exit_code().code()
            );
            assert!(
                page.contains(&row),
                "generated reference is missing runtime-derived row {row:?}"
            );
        }
    }

    /// The wire-contract test above pins the declaration order; a new member also
    /// changes this projection until the committed page is regenerated.
    #[test]
    fn test_committed_error_code_reference_matches_projection() {
        let committed = std::fs::read_to_string(error_code_reference_path())
            .expect("committed error-code reference should exist");
        assert_eq!(
            committed,
            render_error_code_reference(),
            "{} is stale — regenerate it from `jit::output::ErrorCode` (run: {})",
            test_support::ERROR_CODE_REFERENCE_PATH,
            test_support::ERROR_CODE_REFERENCE_GENERATOR,
        );
    }

    fn emitted_text_error_codes() -> BTreeSet<&'static str> {
        regex::Regex::new(r#""([A-Za-z_]+)""#)
            .expect("error-code literal regex should compile")
            .captures_iter(include_str!("main.rs"))
            .filter_map(|captures| captures.get(1).map(|capture| capture.as_str()))
            .filter(|text| {
                *text == "recovery_failed"
                    || (text
                        .bytes()
                        .all(|byte| byte == b'_' || byte.is_ascii_uppercase())
                        && ["ERROR", "FAILED", "NOT_FOUND"]
                            .iter()
                            .any(|suffix| text.ends_with(suffix)))
            })
            .collect()
    }

    /// Every error code `main.rs` emits comes from `classify` or a typed `ErrorCode`, never a literal.
    #[test]
    fn test_main_emits_no_literal_error_codes() {
        assert_eq!(emitted_text_error_codes(), BTreeSet::new());
    }

    /// The process status is a semantic projection of the typed classification.
    ///
    /// Assigning an [`ErrorCode`] directly to the envelope makes a textual code
    /// representation fail at compile time. Changing between classifications
    /// with different exit classes then proves that no constructor-selected
    /// status can override the current typed code.
    #[test]
    fn test_json_error_exit_status_follows_the_current_typed_code() {
        let mut envelope = JsonError::new(ErrorCode::GenericError, "failure");
        assert_eq!(envelope.exit_code(), ExitCode::GenericError);

        envelope.error.code = ErrorCode::ValidationFailed;
        assert_eq!(envelope.exit_code(), ExitCode::ValidationFailed);
    }

    #[test]
    fn test_parse_error_has_explicit_generic_status() {
        assert_eq!(ErrorCode::ParseError.exit_code(), ExitCode::GenericError);
    }

    // ========================================================================
    // Query Response Tests
    // ========================================================================

    #[test]
    fn test_blocked_reason_types() {
        let dep = BlockedReason {
            reason_type: BlockedReasonType::Dependency,
            detail: "ABC".to_string(),
        };
        let gate = BlockedReason {
            reason_type: BlockedReasonType::Gate,
            detail: format!(
                "{} ({})",
                "test-gate".parse::<GateKey>().unwrap(),
                GateStatus::Pending
            ),
        };

        let dep_json = serde_json::to_value(&dep).unwrap();
        let gate_json = serde_json::to_value(&gate).unwrap();

        assert_eq!(dep_json["type"], "dependency");
        assert_eq!(gate_json["type"], "gate");
    }

    #[test]
    fn test_gate_definition_serializes_stage_and_mode_as_snake_case() {
        use crate::declarations::{GateMode, GateStage};
        // --json gate output must remain snake_case regardless of enum Debug repr.
        let def = GateDefinition {
            key: "ci".parse().unwrap(),
            title: "CI".to_string(),
            description: "Continuous integration".to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: None,
            inputs: None,
        };
        let v = serde_json::to_value(&def).unwrap();
        assert_eq!(v["stage"], "postcheck", "stage must be snake_case");
        assert_eq!(v["mode"], "auto", "mode must be snake_case");

        let def_pre = GateDefinition {
            key: "lint".parse().unwrap(),
            title: "Lint".to_string(),
            description: "Linting".to_string(),
            stage: GateStage::Precheck,
            mode: GateMode::Manual,
            checker: None,
            inputs: None,
        };
        let v2 = serde_json::to_value(&def_pre).unwrap();
        assert_eq!(v2["stage"], "precheck");
        assert_eq!(v2["mode"], "manual");
    }

    #[test]
    fn test_gate_definition_from_gate_round_trips_stage_mode() {
        use crate::declarations::{GateMode, GateStage};
        let gate = crate::declarations::GateDefinition {
            version: 1,
            key: "tests".parse().unwrap(),
            title: "Tests".to_string(),
            description: "Run tests".to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: None,
            priority: 100,
            reserved: std::collections::HashMap::new(),
            inputs: None,
        };
        let def = GateDefinition::from(gate);
        assert_eq!(def.stage, GateStage::Postcheck);
        assert_eq!(def.mode, GateMode::Auto);
        let v = serde_json::to_value(&def).unwrap();
        assert_eq!(v["stage"], "postcheck");
        assert_eq!(v["mode"], "auto");
    }

    /// `count` is owned by `ListEnvelope`; each distinct response shape owns
    /// only its collection key and the extras beside it.
    fn assert_envelope_value(value: &serde_json::Value, key: &str, len: usize) {
        assert_eq!(value[key].as_array().map(Vec::len), Some(len));
    }

    #[test]
    fn test_gate_preset_list_response_serializes_presets_under_collection_key() {
        let info = crate::gate_presets::PresetInfo {
            name: "rust-tdd".into(),
            description: "TDD gates".into(),
            gate_count: 3,
        };
        let value = serde_json::to_value(GatePresetListResponse::new(vec![info])).unwrap();
        assert_envelope_value(&value, "presets", 1);
        assert_eq!(value["presets"][0]["gate_count"], 3);
    }

    #[test]
    fn test_event_list_response_serializes_events_under_collection_key() {
        let issue = crate::test_utils::fixture_issue("title".to_string(), String::new());
        let event = crate::test_utils::stamped_event(
            crate::domain::EventBody::draft_issue_created(&issue),
            "e",
            chrono::Utc::now(),
        );
        let value =
            serde_json::to_value(EventListResponse::new(vec![event.clone(), event])).unwrap();
        assert_envelope_value(&value, "events", 2);
        assert!(value["events"][0]["type"].is_string());
    }

    #[test]
    fn test_label_values_response_serializes_namespace_beside_values() {
        let response = LabelValuesResponse::with_extras(
            vec!["v1".into(), "v2".into()],
            LabelValuesScope {
                namespace: "epic".into(),
            },
        );
        assert_eq!(
            response.render_human(OutputMode::default()),
            "Values in namespace 'epic':\n\n  v1\n  v2\n\nTotal: 2\n"
        );
        let value = serde_json::to_value(response).unwrap();
        assert_envelope_value(&value, "values", 2);
        assert_eq!(value["namespace"], "epic");
    }

    #[test]
    fn test_label_namespaces_response_renders_configuration_without_serializing_it() {
        let response = NamespacesResponse::with_extras(
            vec!["type".into()],
            NamespacesHumanDetails {
                namespaces: vec![crate::domain::LabelNamespace::new("Issue type", true)],
            },
        );

        assert_eq!(
            response.render_human(OutputMode::default()),
            "Label Namespaces:\n\n  type\n    Description: Issue type\n    Unique: true\n\n"
        );
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["namespaces"], serde_json::json!(["type"]));
        assert!(value.get("description").is_none());

        let quiet = NamespacesResponse::with_extras(
            vec!["type".into()],
            NamespacesHumanDetails {
                namespaces: vec![crate::domain::LabelNamespace::new("Issue type", true)],
            },
        );
        assert_eq!(
            quiet.render_human(OutputMode {
                quiet: true,
                ..OutputMode::default()
            }),
            "  type\n    Description: Issue type\n    Unique: true\n\n"
        );
    }

    #[test]
    fn test_label_values_response_honors_quiet_human_rendering() {
        let response = LabelValuesResponse::with_extras(
            vec!["task".into()],
            LabelValuesScope {
                namespace: "type".into(),
            },
        );

        assert_eq!(
            response.render_human(OutputMode {
                quiet: true,
                ..OutputMode::default()
            }),
            "  task\n"
        );
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["namespace"], "type");
        assert!(value.get("quiet").is_none());
    }

    #[test]
    fn test_lease_list_response_serializes_null_options_and_empty_warnings() {
        let now = chrono::Utc::now();
        let lease = crate::storage::claim_coordinator::Lease {
            lease_id: "00000000-0000-4000-8000-000000000001".parse().unwrap(),
            issue_id: crate::test_utils::issue_id(0x4000_8000_0000_0000_0001_aaaa_aaaa),
            agent_id: "agent:a".parse().unwrap(),
            worktree_id: "w".parse().unwrap(),
            branch: None,
            ttl_secs: 0,
            acquired_at: now,
            expires_at: None,
            last_beat: now,
        };
        let listed = crate::commands::claim::ListedLease {
            lease,
            stale: false,
        };
        let value = serde_json::to_value(LeaseListResponse::listing(vec![listed], true)).unwrap();
        assert_envelope_value(&value, "leases", 1);
        assert!(value["leases"][0]["expires_at"].is_null());
        assert!(value["leases"][0]["branch"].is_null());
    }
}

impl IssueStatusResponse {
    /// Render the stable, greppable one-line text form:
    ///
    /// ```text
    /// <short_id> [<state>] gates: <key>=<status>,... unmet: <short_id>,... title: <title>
    /// ```
    ///
    /// The `gates:` and `unmet:` sections each render `none` when empty, so the
    /// field order and separators are fixed regardless of content. `state` and
    /// each gate `status` use their canonical snake_case names.
    pub fn to_line(&self) -> String {
        let gates = if self.gates.is_empty() {
            "none".to_string()
        } else {
            self.gates
                .iter()
                .map(|g| format!("{}={}", g.key, g.status.as_str()))
                .collect::<Vec<_>>()
                .join(",")
        };
        let unmet = if self.unmet_dependencies.is_empty() {
            "none".to_string()
        } else {
            self.unmet_dependencies.join(",")
        };
        format!(
            "{} [{}] gates: {} unmet: {} title: {}",
            self.short_id,
            self.state.as_str(),
            gates,
            unmet,
            self.title
        )
    }
}

impl HumanRender for StateRollup {
    fn render_human(&self, _: OutputMode) -> String {
        self.to_lines()
            .into_iter()
            .map(|line| format!("{line}\n"))
            .collect()
    }
}

impl StateRollup {
    /// Render the two-line greppable text form:
    ///
    /// ```text
    /// by state: backlog=0 ready=1 in_progress=1 gated=0 done=2 rejected=1 archived=0
    /// done 2/5 (40%)  open 1  rejected 1
    /// ```
    ///
    /// The `by state:` line lists every state as `<state>=<count>` in canonical
    /// order (states enumerated from the domain, not hardcoded), so its columns
    /// are fixed regardless of which states are populated.
    pub fn to_lines(&self) -> Vec<String> {
        let by_state = self
            .items
            .iter()
            .map(|c| format!("{}={}", c.state.as_str(), c.count))
            .collect::<Vec<_>>()
            .join(" ");
        vec![
            format!("by state: {by_state}"),
            format!(
                "done {}/{} ({}%)  open {}  rejected {}",
                self.extras.done,
                self.extras.total,
                self.extras.percent,
                self.extras.open,
                self.extras.rejected
            ),
        ]
    }
}
