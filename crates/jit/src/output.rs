//! Structured output formatting for CLI commands.
//!
//! This module provides consistent JSON output formatting for both success
//! and error cases, ensuring machine-readable output that works well with
//! AI agents and automation tools.

use crate::domain::identity::GateKey;
use crate::domain::Assignee;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Display;
use std::io::{self, Write};
use thiserror::Error;

mod apply;
mod archive;
mod claim;
pub mod config;
mod dep;
mod emit;
mod events;
pub mod failure;
pub mod hooks;
mod init;
mod invariant;
mod item;
mod project;
mod search;
mod serve;
mod validate;
mod version;

pub use validate::*;

pub use archive::ArchiveTargetResponse;
pub use claim::{
    ClaimAcquireResponse, ClaimForceEvictResponse, ClaimHeartbeatResponse, ClaimReleaseResponse,
    ClaimRenewResponse, LeaseListResponse, LeasesKey, RecoverResponse,
};
pub use dep::{DependencyAddResponse, DependencyRemoveResponse};
pub use emit::{emit_outcome, HumanRender, Report, Response};
pub use search::{SearchHit, SearchKey, SearchQuery, SearchResponse};
pub use serve::{ServeResponse, WebUiSource};

use crate::declarations::{GateMode, GateStage};
use crate::domain::gate_findings::GateFindings;
use crate::domain::identity::IssueId;
use crate::domain::list_envelope::{CollectionKey, ListEnvelope};
use crate::domain::queries::BlockingReason;
use crate::domain::store_divergence::StoreDivergence;
use crate::domain::vocabulary::wire_vocabulary;
use crate::domain::{
    GateRunResult, GateRunStatus, GateState, GateStatus, Issue, MinimalBlockedIssue, MinimalIssue,
    Priority, State,
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
    render_diagnostics(&mut rendered, "Warnings", plan.warnings());

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
        render_diagnostics(&mut rendered, "    warnings", artifact.warnings());
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
    for warning in &result.warnings {
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

/// One row of a human issue list: the issue and its optional reason lines.
pub type IssueListRow<'a> = (&'a Issue, Vec<String>);

/// Rows with no reason lines.
pub fn plain_issue_rows(issues: &[Issue]) -> impl Iterator<Item = IssueListRow<'_>> {
    issues.iter().map(|issue| (issue, Vec::new()))
}

/// Render a human issue list: `header`, two-space-indented rows (`id | title`
/// plus `columns`, then one `    - reason` line per reason), and a blank-line
/// separated `Total: N` footer. `quiet` drops the header and footer.
pub fn render_issue_list<'a>(
    header: &str,
    rows: impl IntoIterator<Item = IssueListRow<'a>>,
    columns: &[IssueColumn],
    quiet: bool,
) -> String {
    use std::fmt::Write;

    let mut lines = String::new();
    let mut total = 0usize;
    for (issue, reasons) in rows {
        total += 1;
        let _ = write!(lines, "  {} | {}", issue.id, issue.title);
        for column in columns {
            let _ = match column {
                IssueColumn::State => write!(lines, " | {}", issue.state),
                IssueColumn::Priority => write!(lines, " | {}", issue.priority),
            };
        }
        lines.push('\n');
        for reason in reasons {
            let _ = writeln!(lines, "    - {reason}");
        }
    }
    if quiet {
        lines.truncate(lines.trim_end_matches('\n').len());
        lines
    } else {
        format!("{header}\n{lines}\nTotal: {total}")
    }
}

/// Print [`render_issue_list`]; quiet mode prints the rows alone and an empty
/// quiet list prints nothing.
pub fn print_issue_list<'a>(
    ctx: &OutputContext,
    header: &str,
    rows: impl IntoIterator<Item = IssueListRow<'a>>,
    columns: &[IssueColumn],
) -> io::Result<()> {
    let rendered = render_issue_list(header, rows, columns, ctx.quiet);
    if ctx.json || (ctx.quiet && rendered.is_empty()) {
        return Ok(());
    }
    writeln_safe(&rendered)
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

impl RenderedFindings {
    pub fn print(&self) {
        print!("{}", self.stdout);
        eprint!("{}", self.stderr);
    }
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
                "Header:\n  {} | First | {state} | {priority}\n    - waits on x\n  {} | Second | {state} | {priority}\n\nTotal: 2", issues[0].id, issues[1].id
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
            format!("H\n  {} | First\n\nTotal: 1", issues[0].id)
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
            format!("  {} | First | {}", issues[0].id, issues[0].priority)
        );
    }

    #[test]
    fn test_render_issue_list_empty_prints_header_and_zero_total() {
        assert_eq!(render_issue_list("H", [], &[], false), "H\n\nTotal: 0");
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
        .with_warnings(vec![PlanWarning::new(
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

/// Context for controlling output verbosity
pub struct OutputContext {
    quiet: bool,
    json: bool,
}

impl OutputContext {
    /// Create a new output context
    pub fn new(quiet: bool, json: bool) -> Self {
        Self { quiet, json }
    }

    /// Print essential output (always shown unless --json)
    pub fn print_data(&self, msg: impl Display) -> io::Result<()> {
        if !self.json {
            writeln_safe(&format!("{}", msg))
        } else {
            Ok(())
        }
    }

    /// Print informational message (suppressed by --quiet or --json)
    pub fn print_info(&self, msg: impl Display) -> io::Result<()> {
        if !self.quiet && !self.json {
            writeln_safe(&format!("{}", msg))
        } else {
            Ok(())
        }
    }

    /// Print success message (suppressed by --quiet or --json)
    pub fn print_success(&self, msg: impl Display) -> io::Result<()> {
        if !self.quiet && !self.json {
            writeln_safe(&format!("{}", msg))
        } else {
            Ok(())
        }
    }

    /// Print warning (suppressed by --quiet or --json)
    pub fn print_warning(&self, msg: impl Display) -> io::Result<()> {
        if !self.quiet && !self.json {
            writeln_safe_stderr(&format!("Warning: {}", msg))
        } else {
            Ok(())
        }
    }
}

/// Safe println that handles broken pipes gracefully
fn writeln_safe(msg: &str) -> io::Result<()> {
    match writeln!(io::stdout(), "{}", msg) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
            // Quiet exit on broken pipe (expected when piping to head, etc.)
            std::process::exit(ExitCode::BrokenPipe.code());
        }
        Err(e) => Err(e),
    }
}

/// Safe eprintln that handles broken pipes gracefully
fn writeln_safe_stderr(msg: &str) -> io::Result<()> {
    match writeln!(io::stderr(), "{}", msg) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
            // Quiet exit on broken pipe
            std::process::exit(ExitCode::BrokenPipe.code());
        }
        Err(e) => Err(e),
    }
}

/// True when `message` is the panic text std's `print!`/`println!`/`eprint!`/
/// `eprintln!` machinery raises when a stream write fails
/// (`library/std/src/io/stdio.rs`: `panic!("failed printing to {label}: {e}")`,
/// where `label` is `"stdout"` or `"stderr"`), and the underlying failure is
/// specifically a closed pipe. Detected by message text because a panic
/// payload is an opaque `dyn Any`, not a typed [`io::Error`] — this matches
/// std's exact, long-stable wording rather than approximating the general
/// panic path. Used by `main`'s top-level panic hook to suppress the panic
/// banner for the hundreds of raw `println!`/`print!` call sites that cannot
/// individually check an `io::Result` (jit:6f881a85).
pub fn is_broken_pipe_write_panic(message: &str) -> bool {
    message.starts_with("failed printing to std") && message.contains("Broken pipe")
}

// ============================================================================
// JSON Output Types
// ============================================================================

/// Wrapper for successful command output (now returns raw data without envelope)
#[derive(Debug)]
pub struct JsonOutput<T: Serialize> {
    pub data: T,
    pub message: Option<String>,
}

impl<T: Serialize> JsonOutput<T> {
    /// Create a new successful output with the given data
    pub fn success(data: T) -> Self {
        Self {
            data,
            message: None,
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

    /// Serialize to JSON string with pretty formatting (returns raw data, no envelope)
    /// If a message is set, it is injected into the top-level object.
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        let mut value = serde_json::to_value(&self.data)?;
        if let Some(ref msg) = self.message {
            if let Value::Object(ref mut map) = value {
                map.insert("message".to_string(), Value::String(msg.clone()));
            }
        }
        serde_json::to_string_pretty(&value)
    }
}

/// Wrapper for error output with suggestions (simplified, no envelope)
#[derive(Debug, Serialize)]
pub struct JsonError {
    pub error: ErrorDetail,
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
        }
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

    /// A downstream reader closed the pipe while jit was writing (141)
    ///
    /// `128 + SIGPIPE (13) = 141`: the exit status a shell reports for a
    /// process terminated by SIGPIPE, adopted here as a direct
    /// `std::process::exit` code rather than by altering this process's own
    /// signal disposition (resetting SIGPIPE disposition needs an unsafe
    /// FFI call, and this crate forbids unsafe code).
    BrokenPipe = 141,
}

impl ExitCode {
    /// Every exit code, in documentation order.
    pub const ALL: [ExitCode; 10] = [
        ExitCode::Success,
        ExitCode::GenericError,
        ExitCode::InvalidArgument,
        ExitCode::NotFound,
        ExitCode::ValidationFailed,
        ExitCode::PermissionDenied,
        ExitCode::AlreadyExists,
        ExitCode::ExternalError,
        ExitCode::LockTimeout,
        ExitCode::BrokenPipe,
    ];

    /// Convert exit code to i32 for `std::process::exit`
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
            ExitCode::BrokenPipe => {
                "Downstream reader closed the pipe while jit was writing (128 + SIGPIPE)"
            }
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
        /// A requested lifecycle state or transition is invalid.
        InvalidState = "INVALID_STATE",
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
        /// A claim or lease command requires a Git repository.
        ClaimRequiresGit = "CLAIM_REQUIRES_GIT",
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
        /// A dependency command failed.
        DependencyError = "DEPENDENCY_ERROR",
        /// A gate command failed.
        GateError = "GATE_ERROR",
        /// A gate-status check failed.
        GateCheckError = "GATE_CHECK_ERROR",
        /// A gate-preset command failed.
        PresetError = "PRESET_ERROR",
        /// An issue item-address lookup failed before resolving an item.
        ItemNotFound = "ITEM_NOT_FOUND",
        /// An item command failed without a more specific public classification.
        ItemCommandFailed = "ITEM_COMMAND_FAILED",
        /// An invariant command failed without a more specific public classification.
        InvariantCommandFailed = "INVARIANT_COMMAND_FAILED",
        /// A profile command failed without a more specific public classification.
        ProfileError = "PROFILE_ERROR",
        /// The search backend failed while executing a query.
        SearchFailed = "SEARCH_FAILED",
        /// The external ripgrep search tool could not be found.
        RipgrepNotFound = "RIPGREP_NOT_FOUND",
        /// Worktree identity inspection failed.
        WorktreeInfoError = "WORKTREE_INFO_ERROR",
        /// Worktree enumeration failed.
        WorktreeListError = "WORKTREE_LIST_ERROR",
        /// Cross-checkout store comparison failed.
        WorktreeStoreDivergenceError = "WORKTREE_STORE_DIVERGENCE_ERROR",
        /// A command failed without a more specific public classification.
        GenericError = "GENERIC_ERROR",
        /// Repository recovery failed.
        RecoveryFailed = "recovery_failed",
        /// The foreground server process exited with a failure status.
        ServerExited = "SERVER_EXITED",
        /// A lock wait expired while another jit process held the lock.
        LockTimeout = "LOCK_TIMEOUT",
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
            | ErrorCode::DependencyError
            | ErrorCode::GateCheckError
            | ErrorCode::PresetError => ExitCode::NotFound,
            ErrorCode::CycleDetected
            | ErrorCode::ValidationFailed
            | ErrorCode::Blocked
            | ErrorCode::GateFailed
            | ErrorCode::ProfileConflict
            | ErrorCode::ProfileUncommittedTargets => ExitCode::ValidationFailed,
            ErrorCode::InvalidArgument
            | ErrorCode::InvalidLabelPattern
            | ErrorCode::InvalidState
            | ErrorCode::AmbiguousId
            | ErrorCode::InvalidIdPrefix
            | ErrorCode::DeletionNotConfirmed
            | ErrorCode::LinkedCheckoutWriteRefused => ExitCode::InvalidArgument,
            ErrorCode::AlreadyExists | ErrorCode::GateError => ExitCode::AlreadyExists,
            ErrorCode::IoError
            | ErrorCode::ClaimRequiresGit
            | ErrorCode::RepositoryFormatUnsupported => ExitCode::ExternalError,
            ErrorCode::PermissionDenied => ExitCode::PermissionDenied,
            ErrorCode::LockTimeout => ExitCode::LockTimeout,
            ErrorCode::ParseError
            | ErrorCode::ItemNotFound
            | ErrorCode::ItemCommandFailed
            | ErrorCode::InvariantCommandFailed
            | ErrorCode::ProfileError
            | ErrorCode::SearchFailed
            | ErrorCode::RipgrepNotFound
            | ErrorCode::WorktreeInfoError
            | ErrorCode::WorktreeListError
            | ErrorCode::WorktreeStoreDivergenceError
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
            ErrorCode::InvalidState => "A lifecycle state or transition is invalid.",
            ErrorCode::Blocked => "Unfinished dependencies block the operation.",
            ErrorCode::GateFailed => "A quality-gate checker did not pass.",
            ErrorCode::IoError => "An input/output or external-system operation failed.",
            ErrorCode::PermissionDenied => {
                "The operation was denied by filesystem or operating-system permissions."
            }
            ErrorCode::ParseError => "Input data could not be parsed.",
            ErrorCode::ClaimRequiresGit => "The claim or lease operation requires Git.",
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
            ErrorCode::DependencyError => "A dependency command failed.",
            ErrorCode::GateError => "A gate command failed.",
            ErrorCode::GateCheckError => "A gate-status check failed.",
            ErrorCode::PresetError => "A gate-preset command failed.",
            ErrorCode::ItemNotFound => "An issue item-address lookup did not resolve an item.",
            ErrorCode::ItemCommandFailed => {
                "An item command failed without a more specific classification."
            }
            ErrorCode::InvariantCommandFailed => {
                "An invariant command failed without a more specific classification."
            }
            ErrorCode::ProfileError => {
                "A profile command failed without a more specific classification."
            }
            ErrorCode::SearchFailed => "The search backend failed while executing a query.",
            ErrorCode::RipgrepNotFound => "The external ripgrep search tool was not found.",
            ErrorCode::WorktreeInfoError => "Worktree identity inspection failed.",
            ErrorCode::WorktreeListError => "Worktree enumeration failed.",
            ErrorCode::WorktreeStoreDivergenceError => "Cross-checkout store comparison failed.",
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

/// Refine a fallback JSON error when the underlying failure is a typed
/// id-resolution error.
///
/// The generic `--json` error path at each command hands in a call-site fallback
/// (e.g. `ISSUE_NOT_FOUND`). When the actual failure is an ambiguous-prefix or
/// too-short-prefix id lookup, that fallback misreports both the code and the
/// exit class, so this replaces it with the matching argument error
/// ([`ErrorCode::AmbiguousId`] or [`ErrorCode::InvalidIdPrefix`], exit code 2)
/// carrying the offending prefix in `details`. Any other error keeps its `fallback` unchanged, so this
/// is a no-op for the vast majority of call sites.
pub fn refine_id_error(error: &anyhow::Error, fallback: JsonError) -> JsonError {
    failure::id_resolution_error(error).unwrap_or(fallback)
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

    pub fn gate_not_found(gate_key: &GateKey) -> Self {
        Self::new(
            ErrorCode::GateNotFound,
            format!("Gate not found: {}", gate_key),
        )
        .with_details(serde_json::json!({"key": gate_key}))
        .with_suggestion("Run 'jit gate list' to see available gates")
        .with_suggestion("Add the gate to the registry first with 'jit gate define'")
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

/// The query an issue search echoes beside its matches.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueSearchQuery {
    pub query: Option<String>,
}

/// Declares the [`CollectionKey`] marker for each list-envelope collection key.
macro_rules! collection_keys {
    ($($(#[$meta:meta])* $marker:ident = $key:literal;)+) => {$(
        $(#[$meta])*
        pub enum $marker {}

        impl CollectionKey for $marker {
            const KEY: &'static str = $key;
        }
    )+};
}

collection_keys! {
    IssuesKey = "issues";
    DependentsKey = "dependents";
    RootsKey = "roots";
    NodesKey = "nodes";
    DivergencesKey = "divergences";
    ByStateKey = "by_state";
    GatesKey = "gates";
    ResultsKey = "results";
    ArtifactsKey = "artifacts";
    WorktreesKey = "worktrees";
    PresetsKey = "presets";
}

pub enum NamespacesKey {}

impl CollectionKey for NamespacesKey {
    const KEY: &'static str = "namespaces";
    const SCHEMA_NAME: Option<&'static str> = Some("NamespacesResponse");
}

pub enum ValuesKey {}

impl CollectionKey for ValuesKey {
    const KEY: &'static str = "values";
    const SCHEMA_NAME: Option<&'static str> = Some("LabelValuesResponse");
}

/// Generic response for issue list queries (available, all, ready, etc.)
/// Uses MinimalIssue for efficiency - contains only id, title, state, priority
pub type IssueListResponse = ListEnvelope<IssuesKey, MinimalIssue>;

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
pub type IssueSearchResponse = ListEnvelope<IssuesKey, MinimalIssue, IssueSearchQuery>;

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
}

impl TemplateApplyResponse {
    /// The `--json` envelope's `message`.
    pub fn message(&self) -> String {
        format!("Applied template '{}' to {}", self.template, self.container)
    }
}

/// Response for blocked query with reasons (minimal issue + reasons)
pub type BlockedListResponse = ListEnvelope<IssuesKey, MinimalBlockedIssue>;

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

/// Tree node for hierarchical dependency display
#[derive(Debug, Serialize, JsonSchema, Clone)]
pub struct DependencyTreeNode {
    /// Issue ID
    pub id: IssueId,
    /// Short ID (first 8 chars)
    pub short_id: String,
    /// Issue title
    pub title: String,
    /// Current state
    pub state: State,
    /// Priority
    pub priority: Priority,
    /// Depth level in tree (1 = immediate child, 2 = grandchild, etc.)
    pub level: u32,
    /// Whether this node appears multiple times in the tree (shared dependency)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared: Option<bool>,
    /// Pre-archive origin state, present exactly for an `Archived` node.
    /// Carried so dependency consumers can compute effective
    /// terminality (`jit:45a140ae`): an `archived` node with a terminal origin
    /// still satisfies its dependents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_from: Option<State>,
    /// Child dependencies
    pub children: Vec<DependencyTreeNode>,
}

impl DependencyTreeNode {
    /// Create from MinimalIssue
    pub fn from_minimal(issue: &MinimalIssue, level: u32) -> Self {
        Self {
            short_id: issue.short_id(),
            id: issue.id.clone(),
            title: issue.title.clone(),
            state: issue.state,
            priority: issue.priority,
            level,
            shared: None,
            archived_from: issue.archived_from,
            children: Vec::new(),
        }
    }
}

/// Response for `graph deps` with tree structure.
///
/// `count` is the number of top-level entries in `nodes` (the immediate
/// dependency nodes; each may carry nested `children`). This differs from
/// `summary.total`, which counts every unique dependency across the whole tree.
pub type GraphDepsTreeResponse = ListEnvelope<NodesKey, DependencyTreeNode, DependencyTreeScope>;

/// Fields of [`GraphDepsTreeResponse`] beside its `nodes`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DependencyTreeScope {
    pub issue_id: IssueId,
    /// Depth of traversal (1 = immediate, 0 = unlimited)
    pub depth: u32,
    /// Summary statistics
    pub summary: DependencySummary,
}

/// Summary statistics for dependencies
#[derive(Debug, Serialize, JsonSchema)]
pub struct DependencySummary {
    /// Total number of unique dependencies
    pub total: usize,
    /// Count by state (keys are canonical snake_case state names)
    pub by_state: std::collections::HashMap<State, usize>,
}

/// Response for `graph roots` command
pub type GraphRootsResponse = ListEnvelope<RootsKey, MinimalIssue>;

/// One node's resolved hierarchy facts, as rendered by `graph tree`.
///
/// The `type` key is the value of the node's `type:` label (absent when it has
/// none). The `parent`, `children`, `cluster`, and `rank` keys are flattened
/// from [`NodeHierarchy`](crate::graph::hierarchy::NodeHierarchy), the
/// DAG-authoritative resolution produced by
/// [`resolve_hierarchy`](crate::graph::hierarchy::resolve_hierarchy).
///
/// # Examples
///
/// ```
/// use jit::graph::hierarchy::NodeHierarchy;
/// use jit::output::HierarchyNodeView;
///
/// let epic_id = jit::test_utils::issue_id(0x4000_8000_0000_0000_0001_1234_5678);
/// let task_id = jit::test_utils::issue_id(0x4000_8000_0000_0000_0002_8765_4321);
/// let view = HierarchyNodeView {
///     id: epic_id.clone(),
///     short_id: epic_id.short_id().to_string(),
///     title: "Auth epic".into(),
///     type_name: Some("epic".into()),
///     hierarchy: NodeHierarchy {
///         parent: None,
///         children: vec![task_id.clone()],
///         cluster: Some(epic_id.clone()),
///         rank: 1,
///     },
/// };
/// let json = serde_json::to_value(&view).unwrap();
/// // `type_name` serializes under the `type` key.
/// assert_eq!(json["type"], "epic");
/// // The resolution fields sit flat on the node.
/// assert_eq!(json["children"][0], task_id.as_str());
/// assert_eq!(json["cluster"], epic_id.as_str());
/// assert_eq!(json["rank"], 1);
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct HierarchyNodeView {
    /// Full issue id.
    pub id: IssueId,
    /// Short id (first 8 chars).
    pub short_id: String,
    /// Issue title.
    pub title: String,
    /// The node's `type:` label value, if any.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    /// DAG-resolved parent, children, cluster, and rank, flattened onto the node.
    #[serde(flatten)]
    pub hierarchy: crate::graph::hierarchy::NodeHierarchy,
}

/// Response for `graph tree` command.
///
/// `count` is the number of entries in `nodes`. `root` echoes the optional root
/// id the view was scoped to (`null` for the whole repository). Each node
/// carries its DAG-resolved parent, children, cluster, and rank.
///
/// # Examples
///
/// ```
/// use jit::graph::hierarchy::NodeHierarchy;
/// use jit::output::{GraphTreeResponse, HierarchyNodeView, TreeRoot};
///
/// let epic_id = jit::test_utils::issue_id(0x4000_8000_0000_0000_0001_1234_5678);
/// let task_id = jit::test_utils::issue_id(0x4000_8000_0000_0000_0002_8765_4321);
/// let response = GraphTreeResponse::with_extras(
///     vec![HierarchyNodeView {
///         id: task_id.clone(),
///         short_id: task_id.short_id().to_string(),
///         title: "A task".into(),
///         type_name: Some("task".into()),
///         hierarchy: NodeHierarchy {
///             parent: Some(epic_id.clone()),
///             children: vec![],
///             cluster: Some(epic_id.clone()),
///             rank: 0,
///         },
///     }],
///     TreeRoot { root: None },
/// );
/// let json = serde_json::to_value(&response).unwrap();
/// assert_eq!(json["count"], 1);
/// assert_eq!(json["root"], serde_json::Value::Null);
/// assert_eq!(json["nodes"][0]["parent"], epic_id.as_str());
/// ```
pub type GraphTreeResponse = ListEnvelope<NodesKey, HierarchyNodeView, TreeRoot>;

/// The scope of a [`GraphTreeResponse`]. `nodes` are ordered by ascending short id.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TreeRoot {
    /// The root id the tree was scoped to, or `null` for the whole repository.
    pub root: Option<IssueId>,
}

/// One reported membership-vs-DAG divergence, as rendered by `query divergence`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DivergenceView {
    /// Full id of the issue carrying the unsupported membership label.
    pub id: IssueId,
    /// Short id (first 8 chars).
    pub short_id: String,
    /// Issue title.
    pub title: String,
    /// The full `namespace:value` membership label.
    pub label: String,
    /// The label namespace.
    pub namespace: String,
    /// The label value.
    pub value: String,
}

/// Response for `query divergence` command.
///
/// Each entry in `divergences` is a membership label whose claim the dependency
/// DAG does not back (the issue carries the label but is not in the closure of
/// the container that owns it), ordered by `(issue_id, label)`.
pub type DivergenceResponse = ListEnvelope<DivergencesKey, DivergenceView>;

// ============================================================================
// Issue Show Response
// ============================================================================

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

/// Response for `issue create`: the created issue's view and its findings.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueCreateResponse {
    #[serde(flatten)]
    pub issue: IssueShowResponse,
    pub findings: Vec<crate::validation::ReportedFinding>,
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
/// use jit::output::{IssueShowResponse, IssueStatusResponse};
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

/// One `(state, count)` bucket in a [`StateRollup`].
///
/// Every [`State`] variant is represented, so `count` is `0` for a state with
/// no issues rather than the entry being omitted (see [`StateRollup`]).
#[derive(Debug, Serialize, JsonSchema)]
pub struct StateCount {
    /// The lifecycle state this bucket counts.
    pub state: State,
    /// Number of issues in that state (`0` when the state is unpopulated).
    pub count: usize,
}

/// Counts-by-state aggregation with a terminal-state rollup over a set of
/// issues, shared by `jit issue progress` (over a container's direct children)
/// and `jit query count --by state` (over a label bucket or the whole repo).
///
/// `by_state` has one [`StateCount`] per [`State`] variant in [`State::ALL`]
/// order, zero-count states included, so the shape is stable and complete
/// (@/inv/domain-agnostic — the state list is enumerated, never hardcoded).
/// `count` is `by_state.len()` (one entry per state).
///
/// Effective-terminal semantics ([`Issue::effective_terminal_state`],
/// `Done`/`Rejected`): the two terminal tallies are reported separately because
/// a rejected issue is terminal but not delivered. `Archived` is
/// terminality-preserving, so an issue archived from `Done` counts toward `done`
/// and one archived from `Rejected` toward `rejected`; an `Archived` issue with a
/// non-terminal origin is not effectively terminal and counts as `open`. `open` is every issue that is not effectively terminal
/// (`total − done − rejected`). The `done`/`total` ratio and `percent` (rounded,
/// `0` when `total` is `0`) measure delivery, i.e. `done` against `total`.
pub type StateRollup = ListEnvelope<ByStateKey, StateCount, RollupTotals>;

/// Delivery tallies of a [`StateRollup`] beside its `by_state` buckets.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RollupTotals {
    /// Total issues aggregated.
    pub total: usize,
    /// Issues in `Done`.
    pub done: usize,
    /// Issues in `Rejected` (terminal but not delivered).
    pub rejected: usize,
    /// Non-terminal issues (`total − done − rejected`).
    pub open: usize,
    /// `round(100 * done / total)`, or `0` when `total` is `0`.
    pub percent: u32,
}

impl ListEnvelope<ByStateKey, StateCount, RollupTotals> {
    /// Aggregate a slice of issues into the counts-by-state rollup.
    ///
    /// `by_state` is [`count_by_state`](crate::domain::queries::count_by_state)
    /// (every variant, zero-count states kept); the [`RollupTotals`] follow the
    /// effective-terminal semantics documented on the type, folding an
    /// `Archived` issue into its pre-archive terminal origin
    /// ([`Issue::effective_terminal_state`]).
    pub fn from_issues(issues: &[Issue]) -> Self {
        let by_state: Vec<StateCount> = crate::domain::queries::count_by_state(issues)
            .into_iter()
            .map(|(state, count)| StateCount { state, count })
            .collect();

        let total = issues.len();
        // Delivery accounting folds by effective terminal state: an issue archived
        // from Done counts as delivered, one archived from Rejected as rejected,
        // so a fully-delivered-then-archived container still reports 100%.
        let done = issues
            .iter()
            .filter(|i| i.effective_terminal_state() == Some(State::Done))
            .count();
        let rejected = issues
            .iter()
            .filter(|i| i.effective_terminal_state() == Some(State::Rejected))
            .count();
        let open = total - done - rejected;
        let percent = if total == 0 {
            0
        } else {
            ((done as f64 / total as f64) * 100.0).round() as u32
        };

        Self::with_extras(
            by_state,
            RollupTotals {
                total,
                done,
                rejected,
                open,
                percent,
            },
        )
    }

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
pub type IssueChildrenResponse = ListEnvelope<IssuesKey, IssueStatusResponse, ChildrenScope>;

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
/// field of the serialized issue. Carries the offending field name.
#[derive(Debug, Error, PartialEq)]
#[error("unknown field '{0}'")]
pub struct UnknownFieldError(pub String);

/// Render the named top-level fields of a serialized issue value as a single
/// compact JSON object, preserving the requested order.
///
/// `value` must be the JSON object produced by serializing an
/// [`IssueShowResponse`]. Each name is looked up by its serialized key. The
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
/// run enrichment that the full view's [`GateView`] carries, so a consumer that
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

/// Per-required-gate readiness entry in a [`GateCheckAllResponse`].
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
pub type GateCheckAllResponse = ListEnvelope<GatesKey, GateStatusEntry, GateCheckSummary>;

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
    /// [`GateStatus::Passed`] token when a verdict was recorded as passing,
    /// otherwise the [`GateStatus::Failed`] token.
    pub status: String,
    /// `pass` or `fail`/`error` when a checker produced a verdict. Pre-verdict
    /// lookup and argument errors leave this absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    /// Advisory warnings from the gate evaluation.
    pub warnings: Vec<String>,
    /// The single-issue gate error envelope's `error` object, when evaluation failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

/// JSON payload of `jit gate evaluate-many --json`.
///
/// `results` holds one entry per requested issue, in argument order.
pub type GateEvaluateManyResponse = ListEnvelope<ResultsKey, GateEvaluateManyResult>;

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
/// block ([`GateFindings`](crate::domain::gate_findings::GateFindings)); they are present only
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

/// Response for `doc dir` command.
///
/// One resolved value rather than a collection, so the shape is a flat named
/// object: the issue whose directory this is, the area the caller named, and
/// the repository-relative directory
/// ([`resolve_artifact_directory`](crate::domain::artifact_directory::resolve_artifact_directory)
/// derives the name inside the area). `area` echoes the caller's spelling
/// rather than a normalized one, so a response is readable against the request
/// that produced it, while `directory` is normalized.
///
/// The directory is a name: it is reported whether or not anything has been
/// written there, and producing this response creates nothing.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ArtifactDirectoryResponse {
    /// Full identifier of the issue the directory belongs to.
    pub issue_id: IssueId,
    /// Short identifier the directory name is built from.
    pub short_id: String,
    /// Issue-scoped area the caller named.
    pub area: String,
    /// Repository-relative directory the issue owns in that area.
    pub directory: String,
}

/// Response for `doc conformance`, the advisory artifact-location report.
///
/// `count` is the length of `artifacts`, which are ordered by area and then by
/// path. `areas` states the registry that was walked, which is exactly
/// [`DocumentationConfig::issue_scoped_areas`](crate::config::DocumentationConfig::issue_scoped_areas),
/// so a machine caller reads the scanned set instead of assuming one.
///
/// The report is advice (`@/issue/8e071e18/decision/D-7`): an entry states
/// where an artifact sits and where its owner's directory is, and blocks
/// nothing.
pub type ArtifactConformanceResponse =
    ListEnvelope<ArtifactsKey, ArtifactConformanceEntry, ConformanceScope>;

/// Fields of [`ArtifactConformanceResponse`] beside its `artifacts`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ConformanceScope {
    /// Declared issue-scoped areas the report walked.
    pub areas: Vec<String>,
}

/// One artifact `doc conformance` names.
///
/// `status` is `nonconforming` when the owning issue is known and the artifact
/// sits outside the directory that issue owns, and `unattributed` when the
/// report's inputs settle no single owner. `issue_id` and
/// `canonical_directory` are carried by a `nonconforming` entry and absent from
/// an `unattributed` one, which claims no owner and no destination.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ArtifactConformanceEntry {
    /// Repository-relative path of the artifact, which is a directory when a
    /// whole directory sits outside its owner's.
    pub path: String,
    /// Declared issue-scoped area the artifact was found under.
    pub area: String,
    /// Short identifier the artifact's own name opens with, empty when the name
    /// carries none and a document reference resolved the owner instead.
    pub short_id: String,
    /// `nonconforming` or `unattributed`.
    pub status: String,
    /// Full identifier of the owning issue, for a `nonconforming` entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<IssueId>,
    /// Repository-relative directory the owner owns in that area, for a
    /// `nonconforming` entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_directory: Option<String>,
}

impl From<crate::domain::artifact_conformance::ReportedArtifact> for ArtifactConformanceEntry {
    fn from(artifact: crate::domain::artifact_conformance::ReportedArtifact) -> Self {
        use crate::domain::artifact_conformance::ArtifactDisposition;

        let status = artifact.disposition.as_str().to_string();
        let (issue_id, canonical_directory) = match artifact.disposition {
            ArtifactDisposition::Nonconforming {
                issue_id,
                canonical_directory,
            } => (Some(issue_id), Some(canonical_directory)),
            ArtifactDisposition::Unattributed => (None, None),
        };
        Self {
            path: artifact.path,
            area: artifact.area,
            short_id: artifact.short_id,
            status,
            issue_id,
            canonical_directory,
        }
    }
}

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
    use super::*;
    use schemars::schema_for;
    use serde_json::json;
    use std::collections::BTreeSet;

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
    fn test_render_archive_execution_lists_counts_and_warnings_or_none() {
        use crate::domain::artifact_execution::{ArchiveExecutionResult, ArchivePublication};
        use crate::domain::artifact_plan::{ContentIdentity, PlanTarget, PlanWarning, WarningCode};
        let result = |warnings| ArchiveExecutionResult {
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
            warnings,
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
    fn test_is_broken_pipe_write_panic_matches_stdout_and_stderr_messages() {
        assert!(is_broken_pipe_write_panic(
            "failed printing to stdout: Broken pipe (os error 32)"
        ));
        assert!(is_broken_pipe_write_panic(
            "failed printing to stderr: Broken pipe (os error 32)"
        ));
    }

    #[test]
    fn test_is_broken_pipe_write_panic_rejects_unrelated_panics() {
        assert!(!is_broken_pipe_write_panic("index out of bounds"));
        assert!(!is_broken_pipe_write_panic(
            "failed printing to stdout: Permission denied (os error 13)"
        ));
        assert!(!is_broken_pipe_write_panic(
            "some other message mentions Broken pipe in passing"
        ));
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
            (ErrorCode::InvalidState, ExitCode::InvalidArgument),
            (ErrorCode::Blocked, ExitCode::ValidationFailed),
            (ErrorCode::GateFailed, ExitCode::ValidationFailed),
            (ErrorCode::IoError, ExitCode::ExternalError),
            (ErrorCode::PermissionDenied, ExitCode::PermissionDenied),
            (ErrorCode::ParseError, ExitCode::GenericError),
            (ErrorCode::ClaimRequiresGit, ExitCode::ExternalError),
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
            (ErrorCode::DependencyError, ExitCode::NotFound),
            (ErrorCode::GateError, ExitCode::AlreadyExists),
            (ErrorCode::GateCheckError, ExitCode::NotFound),
            (ErrorCode::PresetError, ExitCode::NotFound),
            (ErrorCode::ItemNotFound, ExitCode::GenericError),
            (ErrorCode::ItemCommandFailed, ExitCode::GenericError),
            (ErrorCode::InvariantCommandFailed, ExitCode::GenericError),
            (ErrorCode::ProfileError, ExitCode::GenericError),
            (ErrorCode::SearchFailed, ExitCode::GenericError),
            (ErrorCode::RipgrepNotFound, ExitCode::GenericError),
            (ErrorCode::WorktreeInfoError, ExitCode::GenericError),
            (ErrorCode::WorktreeListError, ExitCode::GenericError),
            (
                ErrorCode::WorktreeStoreDivergenceError,
                ExitCode::GenericError,
            ),
            (ErrorCode::GenericError, ExitCode::GenericError),
            (ErrorCode::RecoveryFailed, ExitCode::GenericError),
            (ErrorCode::ServerExited, ExitCode::GenericError),
            (ErrorCode::LockTimeout, ExitCode::LockTimeout),
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
    fn test_new_error_code_members_match_plain_failure_classes() {
        for code in [
            ErrorCode::DependencyError,
            ErrorCode::GateCheckError,
            ErrorCode::PresetError,
        ] {
            assert_eq!(code.exit_code(), ExitCode::NotFound, "{code}");
        }

        assert_eq!(ErrorCode::GateError.exit_code(), ExitCode::AlreadyExists);

        for code in [
            ErrorCode::ItemNotFound,
            ErrorCode::ItemCommandFailed,
            ErrorCode::InvariantCommandFailed,
            ErrorCode::ProfileError,
            ErrorCode::SearchFailed,
            ErrorCode::RipgrepNotFound,
            ErrorCode::WorktreeInfoError,
            ErrorCode::WorktreeListError,
            ErrorCode::WorktreeStoreDivergenceError,
            ErrorCode::GenericError,
            ErrorCode::RecoveryFailed,
            ErrorCode::ServerExited,
        ] {
            assert_eq!(code.exit_code(), ExitCode::GenericError, "{code}");
        }
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
