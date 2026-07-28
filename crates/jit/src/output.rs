//! Structured output formatting for CLI commands.
//!
//! This module provides consistent JSON output formatting for both success
//! and error cases, ensuring machine-readable output that works well with
//! AI agents and automation tools.

use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use std::fmt::Display;
use std::io::{self, Write};
use thiserror::Error;

use crate::declarations::{GateMode, GateStage};
use crate::domain::{
    GateFindings, GateRunResult, GateRunStatus, GateState, GateStatus, Issue, MinimalBlockedIssue,
    MinimalIssue, Priority, State,
};
use crate::errors::{
    gate_status_name, short_id, state_name, TransitionBlockedError, TransitionBlocker,
};

/// Render the complete schema-v1 archive plan without introducing a second
/// decision model. Every action, fact, edge, relink, deletion, warning, and
/// blocker comes directly from the serialized plan object.
pub fn render_archive_plan(plan: &crate::domain::artifact_plan::ArtifactPlan) -> String {
    use crate::domain::artifact_plan::PlanTarget;
    use std::fmt::Write;

    let mut rendered = String::new();
    let target = match plan.target() {
        PlanTarget::Container { id } => format!("container {}", short_id(id)),
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
                    short_id(&owner.issue),
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
                    short_id(&change.issue),
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
        let artifact = ArtifactPlanEntry::new(
            "dev/active/page.md",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Block,
        )
        .with_provenance(vec![ArtifactProvenance::Explicit])
        .with_format("markdown")
        .with_owners(vec![ArtifactOwner {
            issue: "12345678-1234-1234-1234-123456789abc".into(),
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
        for expected in [
            "Schema version: 1",
            "Policy: configured",
            "Eligible for execution: false",
            "[block] dev/active/page.md @ working-tree",
            "format: markdown",
            "provenance: explicit",
            "evidence: permanent-path",
            "issue=12345678 document-index=2 state=done",
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

    /// Print error (always shown to stderr)
    pub fn print_error(&self, msg: impl Display) -> io::Result<()> {
        writeln_safe_stderr(&format!("Error: {}", msg))
    }

    /// Check if quiet mode is enabled
    pub fn is_quiet(&self) -> bool {
        self.quiet
    }

    /// Check if JSON mode is enabled
    pub fn is_json(&self) -> bool {
        self.json
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
#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub struct JsonError {
    pub error: ErrorDetail,
    #[serde(skip)]
    exit_code: ExitCode,
}

#[allow(dead_code)]
impl JsonError {
    /// Create an error whose code belongs to the registered vocabulary.
    ///
    /// The status comes from [`ErrorCode::exit_code`], so registered envelopes
    /// cannot carry a status that disagrees with their code.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            error: ErrorDetail {
                code: code.as_str().to_string(),
                message: message.into(),
                details: None,
                suggestions: Vec::new(),
            },
            exit_code: code.exit_code(),
        }
    }

    /// Create an envelope from a textual code and an explicit fallback status.
    ///
    /// Registered text is resolved through [`ErrorCode`] and therefore uses the
    /// member's declared status. Unknown text preserves the caller-supplied
    /// historical status rather than silently acquiring a default class. New
    /// typed call sites should use [`JsonError::new`].
    pub fn legacy_unregistered(
        code: impl Into<String>,
        exit_code: ExitCode,
        message: impl Into<String>,
    ) -> Self {
        let code = code.into();
        let exit_code = code
            .parse::<ErrorCode>()
            .map_or(exit_code, ErrorCode::exit_code);
        Self {
            error: ErrorDetail {
                code,
                message: message.into(),
                details: None,
                suggestions: Vec::new(),
            },
            exit_code,
        }
    }

    /// Resolve a textual code and construct its registered error envelope.
    ///
    /// Unknown text is returned to the caller and is never assigned a fallback
    /// process status.
    pub fn from_code_text(
        code: &str,
        message: impl Into<String>,
    ) -> Result<Self, UnknownErrorCode> {
        code.parse::<ErrorCode>()
            .map(|code| Self::new(code, message))
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

    /// Return the explicitly selected process exit status.
    pub const fn exit_code(&self) -> ExitCode {
        self.exit_code
    }
}

/// Error details including code, message, and suggestions
#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    /// Error code (e.g., "ISSUE_NOT_FOUND", "CYCLE_DETECTED")
    pub code: String,
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
/// for automation and scripting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
#[allow(dead_code)] // Part of public API
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

    /// A downstream reader closed the pipe while jit was writing (141)
    ///
    /// `128 + SIGPIPE (13) = 141`: the exit status a shell reports for a
    /// process terminated by SIGPIPE, adopted here as a direct
    /// `std::process::exit` code rather than by altering this process's own
    /// signal disposition (resetting SIGPIPE disposition needs an unsafe
    /// FFI call, and this crate forbids unsafe code).
    BrokenPipe = 141,
}

#[allow(dead_code)] // Part of public API
impl ExitCode {
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
            ExitCode::BrokenPipe => {
                "Downstream reader closed the pipe while jit was writing (128 + SIGPIPE)"
            }
        }
    }

    /// Get all exit codes as a formatted string for documentation
    pub fn all_codes_documentation() -> String {
        format!(
            "Exit Codes:\n\
             {}  - {}\n\
             {}  - {}\n\
             {}  - {}\n\
             {}  - {}\n\
             {}  - {}\n\
             {}  - {}\n\
             {}  - {}\n\
             {} - {}\n\
             {} - {}",
            ExitCode::Success.code(),
            ExitCode::Success.description(),
            ExitCode::GenericError.code(),
            ExitCode::GenericError.description(),
            ExitCode::InvalidArgument.code(),
            ExitCode::InvalidArgument.description(),
            ExitCode::NotFound.code(),
            ExitCode::NotFound.description(),
            ExitCode::ValidationFailed.code(),
            ExitCode::ValidationFailed.description(),
            ExitCode::PermissionDenied.code(),
            ExitCode::PermissionDenied.description(),
            ExitCode::AlreadyExists.code(),
            ExitCode::AlreadyExists.description(),
            ExitCode::ExternalError.code(),
            ExitCode::ExternalError.description(),
            ExitCode::BrokenPipe.code(),
            ExitCode::BrokenPipe.description(),
        )
    }
}

// ============================================================================
// Error Codes (JSON responses)
// ============================================================================

/// Standard error-code vocabulary for JIT's machine-readable responses.
///
/// The serialized names are the strings carried in JSON error envelopes.
/// [`ErrorCode::ALL`] is checked against this enum's derived schema, keeping
/// enumeration consumers in lockstep with the type's variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// The requested issue does not exist.
    IssueNotFound,
    /// The requested gate does not exist.
    GateNotFound,
    /// The requested dependency would make the graph cyclic.
    CycleDetected,
    /// An argument or invocation is invalid.
    InvalidArgument,
    /// Repository or domain validation failed.
    ValidationFailed,
    /// The requested resource already exists.
    AlreadyExists,
    /// A requested lifecycle state or transition is invalid.
    InvalidState,
    /// Unfinished dependencies block the requested operation.
    Blocked,
    /// A quality-gate checker completed without passing.
    GateFailed,
    /// An input/output or external-system operation failed.
    IoError,
    /// Input data could not be parsed.
    ParseError,
    /// A claim or lease command requires a Git repository.
    ClaimRequiresGit,
    /// An ID prefix matched more than one candidate.
    AmbiguousId,
    /// An ID prefix was shorter than the accepted minimum.
    InvalidIdPrefix,
    /// No `.jit` repository exists at the resolved data directory.
    RepositoryNotFound,
    /// The repository format is newer than this binary supports.
    RepositoryFormatTooNew,
    /// A gate checker was refused because the binary predates the repository.
    StaleBinary,
    /// Issue deletion was refused because operator confirmation was absent.
    DeletionNotConfirmed,
    /// The requested embedded profile does not exist.
    ProfileNotFound,
    /// Profile planning or final-state validation rejected the operation.
    ProfileConflict,
    /// A dependency command failed.
    DependencyError,
    /// A gate command failed.
    GateError,
    /// A gate-status check failed.
    GateCheckError,
    /// A gate-preset command failed.
    PresetError,
    /// An issue item-address lookup failed before resolving an item.
    ItemNotFound,
    /// An item command failed without a more specific public classification.
    ItemCommandFailed,
    /// An invariant command failed without a more specific public classification.
    InvariantCommandFailed,
    /// A project command failed without a more specific public classification.
    ProjectCommandFailed,
    /// A profile command failed without a more specific public classification.
    ProfileError,
    /// The search backend failed while executing a query.
    SearchFailed,
    /// The external ripgrep search tool could not be found.
    RipgrepNotFound,
    /// Worktree identity inspection failed.
    WorktreeInfoError,
    /// Worktree enumeration failed.
    WorktreeListError,
    /// Repository hook installation failed.
    HooksInstallError,
    /// A command failed without a more specific public classification.
    GenericError,
    /// Repository recovery failed.
    #[serde(rename = "recovery_failed")]
    RecoveryFailed,
    /// Claim acquisition failed.
    ClaimAcquireError,
    /// Claim release failed.
    ClaimReleaseError,
    /// Claim renewal failed.
    ClaimRenewError,
    /// Claim heartbeat failed.
    ClaimHeartbeatError,
    /// Claim status inspection failed without a more specific classification.
    ClaimStatusError,
    /// Claim enumeration failed without a more specific classification.
    ClaimListError,
    /// Forced claim eviction failed.
    ClaimForceEvictError,
}

impl ErrorCode {
    /// Every registered error code, in declaration order.
    ///
    /// A conformance test compares this list with the variants schemars derives
    /// from [`ErrorCode`], so omitting a newly added member fails the suite.
    pub const ALL: [ErrorCode; 43] = [
        ErrorCode::IssueNotFound,
        ErrorCode::GateNotFound,
        ErrorCode::CycleDetected,
        ErrorCode::InvalidArgument,
        ErrorCode::ValidationFailed,
        ErrorCode::AlreadyExists,
        ErrorCode::InvalidState,
        ErrorCode::Blocked,
        ErrorCode::GateFailed,
        ErrorCode::IoError,
        ErrorCode::ParseError,
        ErrorCode::ClaimRequiresGit,
        ErrorCode::AmbiguousId,
        ErrorCode::InvalidIdPrefix,
        ErrorCode::RepositoryNotFound,
        ErrorCode::RepositoryFormatTooNew,
        ErrorCode::StaleBinary,
        ErrorCode::DeletionNotConfirmed,
        ErrorCode::ProfileNotFound,
        ErrorCode::ProfileConflict,
        ErrorCode::DependencyError,
        ErrorCode::GateError,
        ErrorCode::GateCheckError,
        ErrorCode::PresetError,
        ErrorCode::ItemNotFound,
        ErrorCode::ItemCommandFailed,
        ErrorCode::InvariantCommandFailed,
        ErrorCode::ProjectCommandFailed,
        ErrorCode::ProfileError,
        ErrorCode::SearchFailed,
        ErrorCode::RipgrepNotFound,
        ErrorCode::WorktreeInfoError,
        ErrorCode::WorktreeListError,
        ErrorCode::HooksInstallError,
        ErrorCode::GenericError,
        ErrorCode::RecoveryFailed,
        ErrorCode::ClaimAcquireError,
        ErrorCode::ClaimReleaseError,
        ErrorCode::ClaimRenewError,
        ErrorCode::ClaimHeartbeatError,
        ErrorCode::ClaimStatusError,
        ErrorCode::ClaimListError,
        ErrorCode::ClaimForceEvictError,
    ];

    /// Return the byte-exact code written to a JSON error envelope.
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorCode::IssueNotFound => "ISSUE_NOT_FOUND",
            ErrorCode::GateNotFound => "GATE_NOT_FOUND",
            ErrorCode::CycleDetected => "CYCLE_DETECTED",
            ErrorCode::InvalidArgument => "INVALID_ARGUMENT",
            ErrorCode::ValidationFailed => "VALIDATION_FAILED",
            ErrorCode::AlreadyExists => "ALREADY_EXISTS",
            ErrorCode::InvalidState => "INVALID_STATE",
            ErrorCode::Blocked => "BLOCKED",
            ErrorCode::GateFailed => "GATE_FAILED",
            ErrorCode::IoError => "IO_ERROR",
            ErrorCode::ParseError => "PARSE_ERROR",
            ErrorCode::ClaimRequiresGit => "CLAIM_REQUIRES_GIT",
            ErrorCode::AmbiguousId => "AMBIGUOUS_ID",
            ErrorCode::InvalidIdPrefix => "INVALID_ID_PREFIX",
            ErrorCode::RepositoryNotFound => "REPOSITORY_NOT_FOUND",
            ErrorCode::RepositoryFormatTooNew => "REPOSITORY_FORMAT_TOO_NEW",
            ErrorCode::StaleBinary => "STALE_BINARY",
            ErrorCode::DeletionNotConfirmed => "DELETION_NOT_CONFIRMED",
            ErrorCode::ProfileNotFound => "PROFILE_NOT_FOUND",
            ErrorCode::ProfileConflict => "PROFILE_CONFLICT",
            ErrorCode::DependencyError => "DEPENDENCY_ERROR",
            ErrorCode::GateError => "GATE_ERROR",
            ErrorCode::GateCheckError => "GATE_CHECK_ERROR",
            ErrorCode::PresetError => "PRESET_ERROR",
            ErrorCode::ItemNotFound => "ITEM_NOT_FOUND",
            ErrorCode::ItemCommandFailed => "ITEM_COMMAND_FAILED",
            ErrorCode::InvariantCommandFailed => "INVARIANT_COMMAND_FAILED",
            ErrorCode::ProjectCommandFailed => "PROJECT_COMMAND_FAILED",
            ErrorCode::ProfileError => "PROFILE_ERROR",
            ErrorCode::SearchFailed => "SEARCH_FAILED",
            ErrorCode::RipgrepNotFound => "RIPGREP_NOT_FOUND",
            ErrorCode::WorktreeInfoError => "WORKTREE_INFO_ERROR",
            ErrorCode::WorktreeListError => "WORKTREE_LIST_ERROR",
            ErrorCode::HooksInstallError => "HOOKS_INSTALL_ERROR",
            ErrorCode::GenericError => "GENERIC_ERROR",
            ErrorCode::RecoveryFailed => "recovery_failed",
            ErrorCode::ClaimAcquireError => "CLAIM_ACQUIRE_ERROR",
            ErrorCode::ClaimReleaseError => "CLAIM_RELEASE_ERROR",
            ErrorCode::ClaimRenewError => "CLAIM_RENEW_ERROR",
            ErrorCode::ClaimHeartbeatError => "CLAIM_HEARTBEAT_ERROR",
            ErrorCode::ClaimStatusError => "CLAIM_STATUS_ERROR",
            ErrorCode::ClaimListError => "CLAIM_LIST_ERROR",
            ErrorCode::ClaimForceEvictError => "CLAIM_FORCE_EVICT_ERROR",
        }
    }

    /// Return the process exit status associated with this error code.
    ///
    /// The match is exhaustive so adding a member requires choosing its status.
    pub const fn exit_code(self) -> ExitCode {
        match self {
            ErrorCode::IssueNotFound
            | ErrorCode::GateNotFound
            | ErrorCode::ProfileNotFound
            | ErrorCode::RepositoryNotFound
            | ErrorCode::DependencyError
            | ErrorCode::GateCheckError
            | ErrorCode::PresetError
            | ErrorCode::ClaimAcquireError
            | ErrorCode::ClaimReleaseError
            | ErrorCode::ClaimRenewError
            | ErrorCode::ClaimHeartbeatError
            | ErrorCode::ClaimForceEvictError => ExitCode::NotFound,
            ErrorCode::CycleDetected
            | ErrorCode::ValidationFailed
            | ErrorCode::Blocked
            | ErrorCode::GateFailed
            | ErrorCode::ProfileConflict => ExitCode::ValidationFailed,
            ErrorCode::InvalidArgument
            | ErrorCode::InvalidState
            | ErrorCode::AmbiguousId
            | ErrorCode::InvalidIdPrefix
            | ErrorCode::DeletionNotConfirmed => ExitCode::InvalidArgument,
            ErrorCode::AlreadyExists | ErrorCode::GateError => ExitCode::AlreadyExists,
            ErrorCode::IoError
            | ErrorCode::ClaimRequiresGit
            | ErrorCode::RepositoryFormatTooNew
            | ErrorCode::StaleBinary => ExitCode::ExternalError,
            ErrorCode::ParseError
            | ErrorCode::ItemNotFound
            | ErrorCode::ItemCommandFailed
            | ErrorCode::InvariantCommandFailed
            | ErrorCode::ProjectCommandFailed
            | ErrorCode::ProfileError
            | ErrorCode::SearchFailed
            | ErrorCode::RipgrepNotFound
            | ErrorCode::WorktreeInfoError
            | ErrorCode::WorktreeListError
            | ErrorCode::HooksInstallError
            | ErrorCode::GenericError
            | ErrorCode::RecoveryFailed
            | ErrorCode::ClaimStatusError
            | ErrorCode::ClaimListError => ExitCode::GenericError,
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
            ErrorCode::ValidationFailed => "Repository or domain validation failed.",
            ErrorCode::AlreadyExists => "The requested resource already exists.",
            ErrorCode::InvalidState => "A lifecycle state or transition is invalid.",
            ErrorCode::Blocked => "Unfinished dependencies block the operation.",
            ErrorCode::GateFailed => "A quality-gate checker did not pass.",
            ErrorCode::IoError => "An input/output or external-system operation failed.",
            ErrorCode::ParseError => "Input data could not be parsed.",
            ErrorCode::ClaimRequiresGit => "The claim or lease operation requires Git.",
            ErrorCode::AmbiguousId => "The ID prefix matches more than one candidate.",
            ErrorCode::InvalidIdPrefix => "The ID prefix is shorter than the accepted minimum.",
            ErrorCode::RepositoryNotFound => "No JIT repository exists at the resolved path.",
            ErrorCode::RepositoryFormatTooNew => {
                "The repository format is newer than this binary supports."
            }
            ErrorCode::StaleBinary => "The running binary predates the repository under review.",
            ErrorCode::DeletionNotConfirmed => {
                "Issue deletion lacks the required operator confirmation."
            }
            ErrorCode::ProfileNotFound => "The requested embedded profile does not exist.",
            ErrorCode::ProfileConflict => "Profile planning or validation found a conflict.",
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
            ErrorCode::ProjectCommandFailed => {
                "A project command failed without a more specific classification."
            }
            ErrorCode::ProfileError => {
                "A profile command failed without a more specific classification."
            }
            ErrorCode::SearchFailed => "The search backend failed while executing a query.",
            ErrorCode::RipgrepNotFound => "The external ripgrep search tool was not found.",
            ErrorCode::WorktreeInfoError => "Worktree identity inspection failed.",
            ErrorCode::WorktreeListError => "Worktree enumeration failed.",
            ErrorCode::HooksInstallError => "Repository hook installation failed.",
            ErrorCode::GenericError => {
                "A command failed without a more specific public classification."
            }
            ErrorCode::RecoveryFailed => "Repository recovery failed.",
            ErrorCode::ClaimAcquireError => "Claim acquisition failed.",
            ErrorCode::ClaimReleaseError => "Claim release failed.",
            ErrorCode::ClaimRenewError => "Claim renewal failed.",
            ErrorCode::ClaimHeartbeatError => "Claim heartbeat failed.",
            ErrorCode::ClaimStatusError => {
                "Claim status inspection failed without a more specific classification."
            }
            ErrorCode::ClaimListError => {
                "Claim enumeration failed without a more specific classification."
            }
            ErrorCode::ClaimForceEvictError => "Forced claim eviction failed.",
        }
    }
}

/// An input string that is not a member of the registered error-code vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown error code `{code}`")]
pub struct UnknownErrorCode {
    code: String,
}

impl UnknownErrorCode {
    /// Return the unresolved input string.
    pub fn as_str(&self) -> &str {
        &self.code
    }
}

impl std::str::FromStr for ErrorCode {
    type Err = UnknownErrorCode;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        match code {
            "ISSUE_NOT_FOUND" => Ok(ErrorCode::IssueNotFound),
            "GATE_NOT_FOUND" => Ok(ErrorCode::GateNotFound),
            "CYCLE_DETECTED" => Ok(ErrorCode::CycleDetected),
            "INVALID_ARGUMENT" => Ok(ErrorCode::InvalidArgument),
            "VALIDATION_FAILED" => Ok(ErrorCode::ValidationFailed),
            "ALREADY_EXISTS" => Ok(ErrorCode::AlreadyExists),
            "INVALID_STATE" => Ok(ErrorCode::InvalidState),
            "BLOCKED" => Ok(ErrorCode::Blocked),
            "GATE_FAILED" => Ok(ErrorCode::GateFailed),
            "IO_ERROR" => Ok(ErrorCode::IoError),
            "PARSE_ERROR" => Ok(ErrorCode::ParseError),
            "CLAIM_REQUIRES_GIT" => Ok(ErrorCode::ClaimRequiresGit),
            "AMBIGUOUS_ID" => Ok(ErrorCode::AmbiguousId),
            "INVALID_ID_PREFIX" => Ok(ErrorCode::InvalidIdPrefix),
            "REPOSITORY_NOT_FOUND" => Ok(ErrorCode::RepositoryNotFound),
            "REPOSITORY_FORMAT_TOO_NEW" => Ok(ErrorCode::RepositoryFormatTooNew),
            "STALE_BINARY" => Ok(ErrorCode::StaleBinary),
            "DELETION_NOT_CONFIRMED" => Ok(ErrorCode::DeletionNotConfirmed),
            "PROFILE_NOT_FOUND" => Ok(ErrorCode::ProfileNotFound),
            "PROFILE_CONFLICT" => Ok(ErrorCode::ProfileConflict),
            "DEPENDENCY_ERROR" => Ok(ErrorCode::DependencyError),
            "GATE_ERROR" => Ok(ErrorCode::GateError),
            "GATE_CHECK_ERROR" => Ok(ErrorCode::GateCheckError),
            "PRESET_ERROR" => Ok(ErrorCode::PresetError),
            "ITEM_NOT_FOUND" => Ok(ErrorCode::ItemNotFound),
            "ITEM_COMMAND_FAILED" => Ok(ErrorCode::ItemCommandFailed),
            "INVARIANT_COMMAND_FAILED" => Ok(ErrorCode::InvariantCommandFailed),
            "PROJECT_COMMAND_FAILED" => Ok(ErrorCode::ProjectCommandFailed),
            "PROFILE_ERROR" => Ok(ErrorCode::ProfileError),
            "SEARCH_FAILED" => Ok(ErrorCode::SearchFailed),
            "RIPGREP_NOT_FOUND" => Ok(ErrorCode::RipgrepNotFound),
            "WORKTREE_INFO_ERROR" => Ok(ErrorCode::WorktreeInfoError),
            "WORKTREE_LIST_ERROR" => Ok(ErrorCode::WorktreeListError),
            "HOOKS_INSTALL_ERROR" => Ok(ErrorCode::HooksInstallError),
            "GENERIC_ERROR" => Ok(ErrorCode::GenericError),
            "recovery_failed" => Ok(ErrorCode::RecoveryFailed),
            "CLAIM_ACQUIRE_ERROR" => Ok(ErrorCode::ClaimAcquireError),
            "CLAIM_RELEASE_ERROR" => Ok(ErrorCode::ClaimReleaseError),
            "CLAIM_RENEW_ERROR" => Ok(ErrorCode::ClaimRenewError),
            "CLAIM_HEARTBEAT_ERROR" => Ok(ErrorCode::ClaimHeartbeatError),
            "CLAIM_STATUS_ERROR" => Ok(ErrorCode::ClaimStatusError),
            "CLAIM_LIST_ERROR" => Ok(ErrorCode::ClaimListError),
            "CLAIM_FORCE_EVICT_ERROR" => Ok(ErrorCode::ClaimForceEvictError),
            code => Err(UnknownErrorCode {
                code: code.to_string(),
            }),
        }
    }
}

impl Display for ErrorCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
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
/// (`AMBIGUOUS_ID` / `INVALID_ID_PREFIX`, exit code 2) carrying the offending
/// prefix in `details`. Any other error keeps its `fallback` unchanged, so this
/// is a no-op for the vast majority of call sites.
pub fn refine_id_error(error: &anyhow::Error, fallback: JsonError) -> JsonError {
    if let Some(prefix_error) = error.downcast_ref::<crate::storage::InvalidIdPrefixError>() {
        return JsonError::new(ErrorCode::InvalidIdPrefix, prefix_error.to_string())
            .with_details(serde_json::json!({ "prefix": prefix_error.prefix() }));
    }
    if let Some(ambiguous) = error.downcast_ref::<crate::storage::AmbiguousIdError>() {
        return JsonError::new(ErrorCode::AmbiguousId, ambiguous.to_string()).with_details(
            serde_json::json!({
                "prefix": ambiguous.prefix(),
                "matches": ambiguous.matches(),
            }),
        );
    }
    fallback
}

/// Helper to create common error responses
#[allow(dead_code)]
impl JsonError {
    pub fn issue_not_found(issue_id: &str) -> Self {
        Self::new(
            ErrorCode::IssueNotFound,
            format!("Issue not found: {}", issue_id),
        )
        .with_details(serde_json::json!({"issue_id": issue_id}))
        .with_suggestion("Run 'jit query all' to see available issues")
        .with_suggestion("Check if the issue ID is correct")
    }

    pub fn gate_not_found(gate_key: &str) -> Self {
        Self::new(
            ErrorCode::GateNotFound,
            format!("Gate not found: {}", gate_key),
        )
        .with_details(serde_json::json!({"key": gate_key}))
        .with_suggestion("Run 'jit gate list' to see available gates")
        .with_suggestion("Add the gate to the registry first with 'jit gate define'")
    }

    pub fn cycle_detected(from: &str, to: &str) -> Self {
        Self::new(
            ErrorCode::CycleDetected,
            format!("Adding dependency would create a cycle: {} -> {}", from, to),
        )
        .with_details(serde_json::json!({"from": from, "to": to}))
        .with_suggestion("Remove existing dependencies that create the cycle")
        .with_suggestion("Use 'jit graph show' to visualize the dependency graph")
    }

    pub fn invalid_state(state: &str) -> Self {
        Self::new(ErrorCode::InvalidState, format!("Invalid state: {}", state))
            .with_details(serde_json::json!({"invalid_state": state}))
            .with_suggestion("Valid states are: open, ready, in_progress, done")
    }

    pub fn invalid_priority(priority: &str) -> Self {
        Self::new(
            ErrorCode::InvalidArgument,
            format!("Invalid priority: {}", priority),
        )
        .with_details(serde_json::json!({"invalid_priority": priority}))
        .with_suggestion("Valid priorities are: low, normal, high, critical")
    }

    pub fn gate_validation_failed(unpassed_gates: &[String], issue_id: &str) -> Self {
        Self::new(
            ErrorCode::ValidationFailed,
            format!(
                "Cannot transition to 'done' - {} gate(s) not passed: {}",
                unpassed_gates.len(),
                unpassed_gates.join(", ")
            ),
        )
        .with_details(serde_json::json!({
            "issue_id": issue_id,
            "requested_state": "done",
            "actual_state": "gated",
            "unpassed_gates": unpassed_gates
        }))
        .with_suggestion("Issue automatically transitioned to 'gated' (awaiting gate approval)")
        .with_suggestion("The issue will auto-transition to 'done' when all gates pass")
        .with_suggestion(format!(
            "To complete: jit gate evaluate {} <gate_key>",
            issue_id
        ))
    }

    /// Build a JSON error for a blocked state transition.
    ///
    /// This keeps machine-readable error shaping in the output layer while the
    /// command layer returns typed blocker data.
    pub fn transition_blocked(blocked: &TransitionBlockedError) -> Self {
        Self::new(blocked.error_code(), blocked.summary())
            .with_details(serde_json::json!({
                "issue_id": blocked.issue_id(),
                "requested_state": state_name(blocked.requested_state()),
                "actual_state": state_name(blocked.actual_state()),
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
            "short_id": short_id(issue_id),
            "title": title,
            "state": state_name(*state),
        }),
        TransitionBlocker::MissingDependency { issue_id } => serde_json::json!({
            "type": "dependency",
            "issue_id": issue_id,
            "short_id": short_id(issue_id),
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
            "status": gate_status_name(*status),
            "mode": mode.as_str(),
        }),
        TransitionBlocker::GraphRule { rule, message } => serde_json::json!({
            "type": "graph_rule",
            "rule": rule,
            "message": message,
        }),
        TransitionBlocker::ArchivedRevive { origin } => serde_json::json!({
            "type": "archived_revive",
            "origin": state_name(*origin),
            "message": format!(
                "an archived issue only revives to its pre-archive state '{}'",
                state_name(*origin)
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
    /// Applied hierarchy template name.
    pub hierarchy_template: String,
    /// Exact disposition of the worktree Git-attributes claim.
    pub gitattributes_status: crate::repository_state::GitattributesStatus,
    /// Files absent before this run and created by it.
    pub created_paths: Vec<String>,
    /// Existing files modified by this run.
    pub modified_paths: Vec<String>,
    /// Applied profile result when initialization included one.
    pub profile: Option<crate::profile::ProfileApplyResult>,
}

/// Response for `status` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusResponse {
    pub open: usize,
    pub ready: usize,
    pub in_progress: usize,
    pub gated: usize,
    pub done: usize,
    pub rejected: usize,
    pub blocked: usize,
    pub total: usize,
}

/// Generic response for issue list queries (available, all, ready, etc.)
/// Uses MinimalIssue for efficiency - contains only id, title, state, priority
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueListResponse {
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Full-record response for `jit query … --full --json`.
///
/// Where the default query shape ([`IssueListResponse`]) emits lean
/// [`MinimalIssue`] entries with no gate fields, `--full` hands back complete
/// stored [`Issue`] records, so the gate list appears under the storage names
/// `gates_required` / `gates_status` — the same shape `.jit/issues/<id>.json`
/// carries. The two shapes are declared side by side in `jit --schema`
/// (@/inv/single-source-prose: the full arm is derived from the [`Issue`]
/// struct, not a hand-written mirror).
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueListFullResponse {
    pub issues: Vec<Issue>,
    pub count: usize,
}

/// Summary response for `jit issue search … --json` (no `--full`).
///
/// The [`IssueListResponse`] shape plus the echoed `query`: lean
/// [`MinimalIssue`] entries with no gate fields.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueSearchResponse {
    pub query: Option<String>,
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Full-record response for `jit issue search … --full --json`.
///
/// The search counterpart of [`IssueListFullResponse`]: complete stored
/// [`Issue`] records (gate list under the storage names `gates_required` /
/// `gates_status`) plus the echoed `query`. Declared alongside
/// [`IssueSearchResponse`] in `jit --schema`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueSearchFullResponse {
    pub query: Option<String>,
    pub issues: Vec<Issue>,
    pub count: usize,
}

/// Response for `jit issue claim` and `jit issue claim-next --json`.
///
/// A record echo: the complete stored [`Issue`] just claimed — so the gate list
/// appears under the storage names `gates_required` / `gates_status`, exactly as
/// the on-disk record carries it — flattened to the top level, plus the advisory
/// `warnings` array the claim produced. Declared as a record dump in
/// `jit --schema`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ClaimResponse {
    #[serde(flatten)]
    pub issue: Issue,
    pub warnings: Vec<crate::storage::StorageWarning>,
}

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
    pub container: String,
    pub anchor_bindings: std::collections::BTreeMap<String, String>,
    pub created_node_ids_by_role: std::collections::BTreeMap<String, String>,
    pub anchor_dependency_snapshots: std::collections::BTreeMap<String, Vec<String>>,
    pub created_issues: std::collections::BTreeMap<String, Issue>,
}

/// Response for blocked query with reasons (minimal issue + reasons)
#[derive(Debug, Serialize, JsonSchema)]
pub struct BlockedListResponse {
    pub issues: Vec<MinimalBlockedIssue>,
    pub count: usize,
}

/// Multi-id envelope for `issue show <id> <id> …` — full show projections in
/// argument order.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueShowListResponse {
    pub issues: Vec<IssueShowResponse>,
    pub count: usize,
}

/// Multi-id envelope for `issue status <id> <id> …` — status projections in
/// argument order.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueStatusListResponse {
    pub issues: Vec<IssueStatusResponse>,
    pub count: usize,
}

/// Ready-issue list envelope built on [`query_ready`](crate::domain::queries::query_ready)
/// (no dedicated CLI subcommand emits it; `query available` layers the
/// unassigned filter on the same readiness test).
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReadyQueryResponse {
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for `query blocked` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct BlockedQueryResponse {
    pub issues: Vec<MinimalBlockedIssue>,
    pub count: usize,
}

/// Response for `query blocked --full`: reason-enriched entries. Neither
/// blocked shape carries the issue's gate-list fields — a blocking gate
/// appears only as a structured reason — so this command belongs to neither
/// gate-field rule in the storage reference.
#[derive(Debug, Serialize, JsonSchema)]
pub struct BlockedFullListResponse {
    pub issues: Vec<BlockedIssue>,
    pub count: usize,
}

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

/// Response for `query assignee` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct AssigneeQueryResponse {
    pub assignee: String,
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for `query state` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct StateQueryResponse {
    pub state: State,
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for `query priority` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct PriorityQueryResponse {
    pub priority: Priority,
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for `query label` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct LabelQueryResponse {
    pub pattern: String,
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for `query strategic` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct StrategicQueryResponse {
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

/// Response for `query closed` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct ClosedQueryResponse {
    pub issues: Vec<MinimalIssue>,
    pub count: usize,
}

// ============================================================================
// Graph Response Types
// ============================================================================

/// Response for `graph downstream` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphDownstreamResponse {
    pub issue_id: String,
    pub dependents: Vec<MinimalIssue>,
    pub count: usize,
}

/// Tree node for hierarchical dependency display
#[derive(Debug, Serialize, JsonSchema, Clone)]
pub struct DependencyTreeNode {
    /// Issue ID
    pub id: String,
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
    /// Pre-archive origin state, present only for an `Archived` node that
    /// recorded one. Carried so dependency consumers can compute effective
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

    /// Get state symbol for display
    ///
    /// `✓` for effectively terminal nodes — `done`, `rejected`, or `archived`
    /// from one of those (`jit:45a140ae`) — `○` otherwise.
    pub fn state_symbol(&self) -> &str {
        if crate::domain::is_effectively_terminal(self.state, self.archived_from) {
            "✓"
        } else {
            "○"
        }
    }
}

/// Response for `graph deps` with tree structure.
///
/// List envelope: `count` is the number of top-level entries in `nodes` (the
/// immediate dependency nodes; each may carry nested `children`). This differs
/// from `summary.total`, which counts every unique dependency across the whole
/// tree. The node collection was renamed from `tree` to `nodes` when the
/// envelope landed.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphDepsTreeResponse {
    pub issue_id: String,
    /// Depth of traversal (1 = immediate, 0 = unlimited)
    pub depth: u32,
    /// Number of top-level entries in `nodes` (list envelope `count`).
    pub count: usize,
    /// Dependency nodes, each of which may carry nested `children`.
    pub nodes: Vec<DependencyTreeNode>,
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
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphRootsResponse {
    pub roots: Vec<MinimalIssue>,
    pub count: usize,
}

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
/// let view = HierarchyNodeView {
///     id: "epic-1234".into(),
///     short_id: "epic-123".into(),
///     title: "Auth epic".into(),
///     type_name: Some("epic".into()),
///     hierarchy: NodeHierarchy {
///         parent: None,
///         children: vec!["task-a".into()],
///         cluster: Some("epic-1234".into()),
///         rank: 1,
///     },
/// };
/// let json = serde_json::to_value(&view).unwrap();
/// // `type_name` serializes under the `type` key.
/// assert_eq!(json["type"], "epic");
/// // The resolution fields sit flat on the node.
/// assert_eq!(json["children"][0], "task-a");
/// assert_eq!(json["cluster"], "epic-1234");
/// assert_eq!(json["rank"], 1);
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct HierarchyNodeView {
    /// Full issue id.
    pub id: String,
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
/// List envelope: `count` is the number of entries in `nodes`. `root` echoes the
/// optional root id the view was scoped to (`null` for the whole repository).
/// Each node carries its DAG-resolved parent, children, cluster, and rank.
///
/// # Examples
///
/// ```
/// use jit::graph::hierarchy::NodeHierarchy;
/// use jit::output::{GraphTreeResponse, HierarchyNodeView};
///
/// let response = GraphTreeResponse {
///     root: None,
///     count: 1,
///     nodes: vec![HierarchyNodeView {
///         id: "task-1234".into(),
///         short_id: "task-123".into(),
///         title: "A task".into(),
///         type_name: Some("task".into()),
///         hierarchy: NodeHierarchy {
///             parent: Some("epic-1".into()),
///             children: vec![],
///             cluster: Some("epic-1".into()),
///             rank: 0,
///         },
///     }],
/// };
/// let json = serde_json::to_value(&response).unwrap();
/// assert_eq!(json["count"], 1);
/// assert_eq!(json["root"], serde_json::Value::Null);
/// assert_eq!(json["nodes"][0]["parent"], "epic-1");
/// ```
#[derive(Debug, Serialize, JsonSchema)]
pub struct GraphTreeResponse {
    /// The root id the tree was scoped to, or `null` for the whole repository.
    pub root: Option<String>,
    /// Number of entries in `nodes`.
    pub count: usize,
    /// Resolved hierarchy per node, ordered by ascending short id.
    pub nodes: Vec<HierarchyNodeView>,
}

/// One reported membership-vs-DAG divergence, as rendered by `query divergence`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DivergenceView {
    /// Full id of the issue carrying the unsupported membership label.
    pub id: String,
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
/// List envelope: `count` is the number of entries in `divergences`. Each entry
/// is a membership label whose claim the dependency DAG does not back (the issue
/// carries the label but is not in the closure of the container that owns it).
#[derive(Debug, Serialize, JsonSchema)]
pub struct DivergenceResponse {
    /// Number of entries in `divergences`.
    pub count: usize,
    /// The reported divergences, ordered by `(issue_id, label)`.
    pub divergences: Vec<DivergenceView>,
}

// ============================================================================
// Issue Show Response
// ============================================================================

/// Per-gate view emitted in `issue show --json`.
///
/// One entry per gate in the issue's `gates_required`. `status` reflects the
/// gate's [`GateState`] (or `pending` when none exists). `last_run_at` and
/// `exit_code` come from the gate's latest [`GateRunResult`] and are both
/// `null` when no run has been recorded (required-but-never-run, or a manual
/// gate attested without a run).
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateView {
    /// Gate key.
    pub key: String,
    /// Gate status: `pending`, `passed`, or `failed`.
    pub status: crate::domain::GateStatus,
    /// Timestamp of the gate's latest run, or `null` when never run.
    pub last_run_at: Option<String>,
    /// Exit code of the gate's latest run, or `null` when never run.
    pub exit_code: Option<i32>,
}

impl GateView {
    /// Build the per-gate view for a single required gate.
    ///
    /// `state` is the issue's recorded [`GateState`] for this gate (if any) and
    /// `runs` are all recorded runs for the issue; the latest run matching
    /// `key` supplies `last_run_at`/`exit_code`.
    fn build(key: &str, state: Option<&crate::domain::GateState>, runs: &[GateRunResult]) -> Self {
        let status = state
            .map(|s| s.status)
            .unwrap_or(crate::domain::GateStatus::Pending);

        let latest = runs
            .iter()
            .filter(|r| r.gate_key == key)
            .max_by_key(|r| r.started_at);

        GateView {
            key: key.to_string(),
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
    pub id: String,
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
    pub id: String,
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
    /// `CommandExecutor::delete_issue`, jit:f847df3f). Deleting an issue
    /// through the CLI strips its own id from every dependent's
    /// `dependencies`, so this is empty in a repository touched only through
    /// the CLI; when non-empty, it means a dangling id exists and — unlike
    /// `dependencies`, which silently omits ids it cannot resolve — is not
    /// hidden.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dangling_dependency_ids: Vec<String>,
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
    /// `gate_runs` are all recorded [`GateRunResult`]s for the issue; the
    /// latest run per required gate enriches that gate's `last_run_at` and
    /// `exit_code`. Pass an empty slice when no runs exist.
    ///
    /// `enriched_deps` normally holds one [`MinimalIssue`] per entry in
    /// `issue.dependencies`, but an id that no longer resolves to a stored
    /// issue is left out of it by
    /// [`get_dependencies_enriched`](crate::commands::CommandExecutor::get_dependencies_enriched).
    /// This constructor recovers any such id by diffing `enriched_deps`
    /// against `issue.dependencies` and reports it via
    /// `dangling_dependency_ids` instead of dropping it silently
    /// (jit:f847df3f).
    pub fn from_issue(
        issue: crate::domain::Issue,
        enriched_deps: Vec<MinimalIssue>,
        gate_runs: &[GateRunResult],
    ) -> Self {
        let gates = issue
            .gates_required
            .iter()
            .map(|key| GateView::build(key, issue.gates_status.get(key), gate_runs))
            .collect();

        // See the doc comment above: recover ids `enriched_deps` couldn't
        // resolve so they surface instead of vanishing from the response.
        let resolved_ids: std::collections::HashSet<&str> =
            enriched_deps.iter().map(|dep| dep.id.as_str()).collect();
        let dangling_dependency_ids: Vec<String> = issue
            .dependencies
            .iter()
            .filter(|dep_id| !resolved_ids.contains(dep_id.as_str()))
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
/// use jit::domain::{GateStatus, Issue, State};
/// use jit::output::{IssueShowResponse, IssueStatusResponse};
///
/// let mut issue = Issue::draft("Build parser".into(), "Body".into());
/// issue.state = State::Ready;
/// issue.gates_required = vec!["tests".into()];
/// let show = IssueShowResponse::from_issue(issue, vec![], &[]);
///
/// let status = IssueStatusResponse::from_show(&show);
/// assert_eq!(status.state, State::Ready);
/// assert_eq!(status.gates.len(), 1);
/// assert_eq!(status.gates[0].key, "tests");
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
/// `by_state` has one [`StateCount`] per [`State`] variant in [`State::all`]
/// order, zero-count states included, so the shape is stable and complete
/// (@/inv/domain-agnostic — the state list is enumerated, never hardcoded).
/// `count` is `by_state.len()` (the list-envelope count, one entry per state).
///
/// Effective-terminal semantics ([`Issue::effective_terminal_state`],
/// `Done`/`Rejected`): `done` and `rejected` are reported separately because a
/// rejected issue is terminal but not delivered. `Archived` is
/// terminality-preserving, so an issue archived from `Done` counts toward `done`
/// and one archived from `Rejected` toward `rejected`; an `Archived` issue with a
/// non-terminal (or unrecorded, legacy) origin is not effectively terminal and
/// counts as `open`. `open` is every issue that is not effectively terminal
/// (`total − done − rejected`). The `done`/`total` ratio and `percent` (rounded,
/// `0` when `total` is `0`) measure delivery, i.e. `done` against `total`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StateRollup {
    /// Length of `by_state` (list-envelope count); equals the number of
    /// [`State`] variants.
    pub count: usize,
    /// One entry per state, zero-count states included, in [`State::all`] order.
    pub by_state: Vec<StateCount>,
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

impl StateRollup {
    /// Aggregate a slice of issues into the counts-by-state rollup.
    ///
    /// `by_state` is [`count_by_state`](crate::domain::queries::count_by_state)
    /// (every variant, zero-count states kept); `done`/`rejected`/`open` and
    /// `percent` follow the effective-terminal semantics documented on the type,
    /// folding an `Archived` issue into `done`/`rejected` by its pre-archive
    /// origin ([`Issue::effective_terminal_state`]).
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

        Self {
            count: by_state.len(),
            by_state,
            total,
            done,
            rejected,
            open,
            percent,
        }
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
            .by_state
            .iter()
            .map(|c| format!("{}={}", c.state.as_str(), c.count))
            .collect::<Vec<_>>()
            .join(" ");
        vec![
            format!("by state: {by_state}"),
            format!(
                "done {}/{} ({}%)  open {}  rejected {}",
                self.done, self.total, self.percent, self.open, self.rejected
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
/// `issues` and `count` form the standard `{count, issues}` list envelope
/// (`count == issues.len()`), with `container` added on top. `dangling` follows
/// the [`IssueShowResponse::dangling_dependency_ids`] precedent: a dependency id
/// that points at no stored issue is surfaced here rather than silently dropped,
/// and is omitted from the JSON when empty. Children are ordered by ascending
/// short id (stored dependency order is set-derived and not meaningful).
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueChildrenResponse {
    /// `{short_id, title, state}` of the queried container.
    pub container: ContainerHeader,
    /// Number of resolvable direct children (equals `issues.len()`).
    pub count: usize,
    /// One compact status projection per resolvable direct child, ascending
    /// short id.
    pub issues: Vec<IssueStatusResponse>,
    /// Dependency ids that resolve to no stored issue (dangling edges), omitted
    /// when empty. `count`/`issues` cover resolvable children only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dangling: Vec<String>,
}

/// Response for `jit issue progress`: a container header, the counts-by-state
/// [`StateRollup`] over its resolvable direct children, and any dangling child
/// edges.
///
/// The rollup fields (`count`, `by_state`, `total`, `done`, `rejected`, `open`,
/// `percent`) are flattened to the top level, so the JSON is the `StateRollup`
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
    pub dangling: Vec<String>,
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

// ============================================================================
// Lean Issue Update / Show Summary Responses
// ============================================================================

/// Lightweight confirmation returned by `jit issue update --json`.
///
/// Mutating an issue does not need to echo the full body back; agents that
/// need it can call `jit issue show`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IssueUpdateResponse {
    pub id: String,
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
    pub id: String,
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
/// use jit::domain::{GateRunResult, GateRunStatus};
/// use jit::output::GateRunSummary;
///
/// let run = GateRunResult {
///     schema_version: 1,
///     run_id: "r1".into(),
///     gate_key: "tests".into(),
///     stage: GateStage::Postcheck,
///     issue_id: "i1".into(),
///     commit: None,
///     branch: None,
///     tree_dirty: None,
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
    pub run_id: String,
    pub key: String,
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
    /// Whether the working tree differed from [`commit`](Self::commit) when the
    /// checker started: `true` dirty, `false` clean, omitted when there was no
    /// commit to compare against. Carries
    /// [`GateRunResult::tree_dirty`](crate::domain::GateRunResult) so a machine
    /// reader can tell a pass evidencing the commit from one taken on a modified
    /// tree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree_dirty: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
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
            tree_dirty: r.tree_dirty,
            by: r.by.clone(),
            message: r.message.clone(),
            stdout: include_output.then(|| r.stdout.clone()),
            stderr: include_output.then(|| r.stderr.clone()),
            findings: r.findings.clone(),
        }
    }
}

/// Per-required-gate readiness entry in a [`GateCheckAllResponse`].
///
/// Reports one required gate's authoritative per-issue status (`passed`,
/// `failed`, or `pending`) so a `--json` consumer can distinguish a gate that
/// ran and failed from one that has never run/attested, both of which map to
/// the single nonzero exit code of `gate status-all`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateStatusEntry {
    pub key: String,
    pub status: GateStatus,
}

impl GateStatusEntry {
    /// Build the `{key, status}` entry for one required gate: its recorded
    /// [`GateState`] status, or `pending` when no run or attestation exists.
    ///
    /// Centralizes the pending-default so the compact `gates` projection shared
    /// by `issue status` and `issue show --summary` matches the status the full
    /// [`GateView`] reports for the same gate.
    pub fn for_required(key: &str, state: Option<&GateState>) -> Self {
        Self {
            key: key.to_string(),
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
/// List envelope: `count` is the length of the `gates` collection (one
/// entry per required gate). It equals `total` whenever every required gate has
/// a status entry; `total`/`passed` remain the readiness tallies.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateCheckAllResponse {
    /// Length of `gates` (list envelope `count`).
    pub count: usize,
    pub results: Vec<GateRunSummary>,
    pub passed: usize,
    pub total: usize,
    pub not_run: Vec<String>,
    pub gates: Vec<GateStatusEntry>,
    pub all_passed: bool,
}

/// JSON payload of `jit gate status --all` / `--limit` (the history view).
///
/// `results` holds one [`GateRunSummary`] per matching run, newest-first, after
/// any `--gate` / `--status` filtering and `--limit` capping. Each summary is the
/// full form (stdout/stderr included) so history inspection never loses report
/// text. `count` is `results.len()` (the number returned, after filtering).
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateRunHistoryResponse {
    pub results: Vec<GateRunSummary>,
    pub count: usize,
}

/// JSON payload of `jit gate status`'s flat report-text view
/// (`--stdout` / `--stderr` / `--tail`).
///
/// Carries the latest run's stored report text for the selected stream(s),
/// already tail-trimmed when `--tail <N>` was supplied. `stdout` / `stderr` are
/// present only for the streams the caller asked for, so the JSON mirrors the
/// plain verbatim output exactly.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateFlatReportResponse {
    pub key: String,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

/// JSON payload of `jit gate status <id> <gate> --findings`.
///
/// The structured-findings view of a single gate's latest run. `verdict`,
/// `summary`, and `findings` come straight from the checker's machine-readable
/// block ([`GateFindings`](crate::domain::GateFindings)); they are present only
/// when that run carried a block. `has_findings` is `false` for a plain-text
/// checker (no block emitted) so a consumer can distinguish "ran, no structured
/// findings" from "no findings recorded", without inspecting the optional
/// fields.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateFindingsResponse {
    pub key: String,
    pub run_id: String,
    /// Whether the latest run carried a machine-readable findings block.
    pub has_findings: bool,
    /// Checker-declared verdict; `None` when no block was emitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    /// One-line run summary; `None` when no block was emitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Individual findings; empty when no block was emitted.
    pub findings: Vec<crate::domain::GateFinding>,
}

// ============================================================================
// Gate Registry Response Types
// ============================================================================

/// Response for `gate list` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateListResponse {
    pub gates: Vec<GateDefinition>,
    pub count: usize,
}

/// Gate definition structure (for registry responses)
///
/// Serialized as snake_case JSON; `stage` and `mode` are typed enums so the
/// serialized form is always well-formed and identical to the registry storage
/// format (`"precheck"` / `"postcheck"` / `"manual"` / `"auto"`).
#[derive(Debug, Serialize, JsonSchema)]
pub struct GateDefinition {
    pub key: String,
    pub title: String,
    pub description: String,
    pub auto: bool,
    pub example_integration: Option<String>,
    /// Execution stage; serializes as `"precheck"` or `"postcheck"`.
    pub stage: GateStage,
    /// Execution mode; serializes as `"manual"` or `"auto"`.
    pub mode: GateMode,
    /// Checker configuration for automated gates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checker: Option<crate::declarations::GateChecker>,
}

impl From<crate::declarations::GateDefinition> for GateDefinition {
    fn from(gate: crate::declarations::GateDefinition) -> Self {
        Self {
            key: gate.key,
            title: gate.title,
            description: gate.description,
            auto: gate.auto,
            example_integration: gate.example_integration,
            stage: gate.stage,
            mode: gate.mode,
            checker: gate.checker,
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
    pub issue_id: String,
    /// Short identifier the directory name is built from.
    pub short_id: String,
    /// Issue-scoped area the caller named.
    pub area: String,
    /// Repository-relative directory the issue owns in that area.
    pub directory: String,
}

/// Response for `doc conformance`, the advisory artifact-location report.
///
/// A list envelope: `count` is the length of `artifacts`. `areas` states the
/// registry that was walked, which is exactly
/// [`DocumentationConfig::issue_scoped_areas`](crate::config::DocumentationConfig::issue_scoped_areas),
/// so a machine caller reads the scanned set instead of assuming one.
///
/// The report is advice (`@/issue/8e071e18/decision/D-7`): an entry states
/// where an artifact sits and where its owner's directory is, and blocks
/// nothing.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ArtifactConformanceResponse {
    /// Declared issue-scoped areas the report walked.
    pub areas: Vec<String>,
    /// Reported artifacts, ordered by area and then by path.
    pub artifacts: Vec<ArtifactConformanceEntry>,
    /// Number of reported artifacts.
    pub count: usize,
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
    pub issue_id: Option<String>,
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

/// Response for `label namespaces` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct NamespacesResponse {
    pub namespaces: Vec<String>,
    pub count: usize,
}

/// Response for top-level `search` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<crate::search::SearchResult>,
    pub count: usize,
}

/// Response for `worktree list` command
#[derive(Debug, Serialize, JsonSchema)]
pub struct WorktreeListResponse {
    pub worktrees: Vec<crate::commands::worktree::WorktreeListEntry>,
    pub count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::schema_for;
    use serde_json::json;
    use std::collections::BTreeSet;

    /// jit:45a140ae REQ-02: a dependency-tree node archived from a terminal
    /// state renders and serializes as effectively terminal; a legacy Archived
    /// node (no recorded origin) stays non-terminal.
    #[test]
    fn test_dependency_tree_node_effective_terminality() {
        let mut issue = crate::domain::MinimalIssue {
            id: "a".repeat(36),
            title: "Archived dep".to_string(),
            state: State::Archived,
            priority: Priority::Normal,
            assignee: None,
            labels: Vec::new(),
            archived_from: Some(State::Done),
        };
        let node = DependencyTreeNode::from_minimal(&issue, 1);
        assert_eq!(node.archived_from, Some(State::Done));
        assert_eq!(node.state_symbol(), "✓");
        let serialized = serde_json::to_value(&node).unwrap();
        assert_eq!(serialized["archived_from"], json!("done"));

        issue.archived_from = None;
        let legacy = DependencyTreeNode::from_minimal(&issue, 1);
        assert_eq!(legacy.state_symbol(), "○");
        let serialized = serde_json::to_value(&legacy).unwrap();
        assert!(serialized.get("archived_from").is_none());
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
            key: "anything".to_string(),
            title: "Coverage".to_string(),
            description: "Configured indirection".to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: Some(crate::declarations::GateChecker::LabelTargetValidation {
                label_namespace: "owner".to_string(),
            }),
            priority: 100,
            reserved: Default::default(),
            auto: true,
            example_integration: None,
        };

        let encoded = serde_json::to_value(GateDefinition::from(gate)).unwrap();
        assert_eq!(encoded["key"], "anything");
        assert_eq!(encoded["checker"]["type"], "label_target_validation");
        assert_eq!(encoded["checker"]["label_namespace"], "owner");
    }

    #[test]
    fn test_show_response_root_shape_short_id_and_arrays() {
        // short_id is id[0..8]; labels and dependencies always serialize as arrays.
        let issue = crate::domain::types::fixture_issue("T".to_string(), "B".to_string());
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
        let mut issue = crate::domain::types::fixture_issue("T".to_string(), "B".to_string());
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

        let mut issue =
            crate::domain::types::fixture_issue("Parent".to_string(), "Body".to_string());
        issue.dependencies = vec!["resolved-id".to_string(), "dangling-id".to_string()];

        // Simulate what `get_dependencies_enriched` would produce: only the
        // resolvable dependency's MinimalIssue, the dangling one absent.
        let mut resolved_dep =
            crate::domain::types::fixture_issue("Dep".to_string(), "".to_string());
        resolved_dep.id = "resolved-id".to_string();
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
            serde_json::json!(["dangling-id"]),
            "the dangling id must be reported, not silently dropped"
        );
    }

    #[test]
    fn test_show_response_omits_rejected_dependency_from_unmet() {
        use crate::domain::State;

        let mut issue =
            crate::domain::types::fixture_issue("Parent".to_string(), "Body".to_string());
        issue.dependencies = vec!["rejected-id".to_string(), "pending-id".to_string()];

        let mut rejected =
            crate::domain::types::fixture_issue("Abandoned".to_string(), String::new());
        rejected.id = "rejected-id".to_string();
        rejected.state = State::Rejected;
        let mut pending =
            crate::domain::types::fixture_issue("Upstream".to_string(), String::new());
        pending.id = "pending-id".to_string();
        pending.state = State::InProgress;
        let enriched_deps = vec![MinimalIssue::from(&rejected), MinimalIssue::from(&pending)];

        let resp = IssueShowResponse::from_issue(issue, enriched_deps, &[]);

        let unmet: Vec<&str> = resp
            .unmet_dependencies
            .iter()
            .map(|dep| dep.id.as_str())
            .collect();
        assert_eq!(
            unmet,
            vec!["pending-id"],
            "a Rejected dependency is met and must not render as unmet"
        );
    }

    #[test]
    fn test_show_response_exposes_content_format() {
        use crate::domain::ContentFormat;
        // Set -> appears in `jit issue show --json` (create/show parity).
        let mut issue = crate::domain::types::fixture_issue("T".to_string(), "B".to_string());
        issue.content_format = Some(ContentFormat::Html);
        let resp = IssueShowResponse::from_issue(issue, vec![], &[]);
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["content_format"], "html");

        // Absent -> omitted (existing issues without the field stay clean).
        let issue2 = crate::domain::types::fixture_issue("T".to_string(), "B".to_string());
        let resp2 = IssueShowResponse::from_issue(issue2, vec![], &[]);
        let v2 = serde_json::to_value(&resp2).unwrap();
        assert!(v2.get("content_format").is_none());
    }

    #[test]
    fn test_show_response_gates_array_replaces_split_fields() {
        // `gates` replaces `gates_required`/`gates_status`; neither legacy field
        // appears in the `issue show --json` shape.
        let mut issue = crate::domain::types::fixture_issue("T".to_string(), "B".to_string());
        issue.gates_required = vec!["tests".to_string()];
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
        let mut issue = crate::domain::types::fixture_issue("T".to_string(), "B".to_string());
        issue.gates_required = vec!["tests".to_string()];
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
        // the latest matching GateRunResult.
        let mut issue = crate::domain::types::fixture_issue("T".to_string(), "B".to_string());
        issue.gates_required = vec!["tests".to_string()];
        let run_at = Utc::now();
        issue.gates_status.insert(
            "tests".to_string(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("ci:test".parse().unwrap()),
                updated_at: run_at,
            },
        );

        let run = GateRunResult {
            schema_version: 1,
            run_id: "run-1".to_string(),
            gate_key: "tests".to_string(),
            stage: GateStage::Postcheck,
            issue_id: issue.id.clone(),
            commit: None,
            branch: None,
            tree_dirty: None,
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
        let data = json!({"id": "123", "title": "Test"});
        let output = JsonOutput::success(data);

        // success field removed
        assert_eq!(output.data["id"], "123");
        // metadata removed
    }

    #[test]
    fn test_json_output_serialization() {
        let data = json!({"id": "123", "title": "test"});
        let output = JsonOutput::success(data);

        let json_str = output.to_json_string().unwrap();
        // Should contain raw data without envelope
        assert!(json_str.contains("\"id\": \"123\""));
        assert!(json_str.contains("\"title\": \"test\""));
        // Should NOT contain envelope fields
        assert!(!json_str.contains("\"success\""));
        assert!(!json_str.contains("\"data\":"));
    }

    #[test]
    fn test_gate_findings_response_preserves_populated_references_and_omits_empty() {
        use crate::domain::GateFinding;

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
            key: "review".to_string(),
            run_id: "run-1".to_string(),
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
        let data = json!({"id": "abc12345", "title": "Test issue"});
        let output = JsonOutput::success(data).with_message("Created issue abc12345 - Test issue");

        let json_str = output.to_json_string().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["message"], "Created issue abc12345 - Test issue");
        assert_eq!(parsed["id"], "abc12345");
        assert_eq!(parsed["title"], "Test issue");
    }

    #[test]
    fn test_json_output_without_message() {
        let data = json!({"id": "123"});
        let output = JsonOutput::success(data);

        let json_str = output.to_json_string().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.get("message").is_none());
    }

    #[test]
    fn test_json_output_with_message_array_payload() {
        // When data serializes to a JSON array (not object), message cannot be injected.
        // Verify the output is still valid JSON (the array), just without message.
        let data = json!([{"id": "a"}, {"id": "b"}]);
        let output = JsonOutput::success(data).with_message("Should not appear");

        let json_str = output.to_json_string().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.is_array(), "Array payload should serialize as array");
        assert_eq!(parsed.as_array().unwrap().len(), 2);
        // message is silently dropped for non-object payloads
    }

    #[test]
    fn test_json_error_basic() {
        let error = JsonError::legacy_unregistered(
            "TEST_ERROR",
            ExitCode::GenericError,
            "This is a test error",
        );

        assert_eq!(error.error.code, "TEST_ERROR");
        assert_eq!(error.error.message, "This is a test error");
        assert!(error.error.details.is_none());
        assert!(error.error.suggestions.is_empty());
    }

    #[test]
    fn test_json_error_with_details() {
        let error = JsonError::legacy_unregistered(
            "NOT_FOUND",
            ExitCode::GenericError,
            "Resource not found",
        )
        .with_details(json!({"requested_id": "abc123"}));

        assert_eq!(error.error.details, Some(json!({"requested_id": "abc123"})));
    }

    #[test]
    fn test_json_error_with_suggestions() {
        let error =
            JsonError::legacy_unregistered("NOT_FOUND", ExitCode::GenericError, "Issue not found")
                .with_suggestion("Run 'jit issue list' to see available issues")
                .with_suggestion("Check if the issue ID is correct");

        assert_eq!(error.error.suggestions.len(), 2);
        assert!(error.error.suggestions[0].contains("jit issue list"));
    }

    #[test]
    fn test_json_error_serialization() {
        let error =
            JsonError::legacy_unregistered("TEST_ERROR", ExitCode::ValidationFailed, "Test")
                .with_details(json!({"key": "value"}))
                .with_suggestion("Try something");

        let json_str = error.to_json_string().unwrap();
        // Should have error object without envelope
        assert!(json_str.contains("\"code\": \"TEST_ERROR\""));
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
    fn test_json_error_text_constructor_rejects_unknown_code() {
        let unresolved = JsonError::from_code_text("UNREGISTERED_ERROR", "unregistered failure")
            .expect_err("an unregistered code must remain unresolved");
        assert_eq!(unresolved.as_str(), "UNREGISTERED_ERROR");
    }

    #[test]
    fn test_json_error_legacy_constructor_preserves_wire_code_and_explicit_status() {
        let error = JsonError::legacy_unregistered(
            "LEGACY_UNREGISTERED",
            ExitCode::ExternalError,
            "legacy failure",
        );

        assert_eq!(error.error.code, "LEGACY_UNREGISTERED");
        assert_eq!(error.exit_code(), ExitCode::ExternalError);
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
    fn test_error_code_all_lists_every_derived_variant() {
        let schema = serde_json::to_value(schema_for!(ErrorCode))
            .expect("ErrorCode schema should serialize");
        let derived = schema_accepted_strings(&schema);
        assert!(!derived.is_empty(), "derived schema should list variants");

        let listed: BTreeSet<String> = ErrorCode::ALL
            .iter()
            .map(|code| code.as_str().to_string())
            .collect();

        assert_eq!(listed, derived, "ErrorCode::ALL must list every variant");
    }

    #[test]
    fn test_error_code_wire_strings_and_exit_statuses_match_contract() {
        let expected = [
            (
                ErrorCode::IssueNotFound,
                "ISSUE_NOT_FOUND",
                ExitCode::NotFound,
            ),
            (
                ErrorCode::GateNotFound,
                "GATE_NOT_FOUND",
                ExitCode::NotFound,
            ),
            (
                ErrorCode::CycleDetected,
                "CYCLE_DETECTED",
                ExitCode::ValidationFailed,
            ),
            (
                ErrorCode::InvalidArgument,
                "INVALID_ARGUMENT",
                ExitCode::InvalidArgument,
            ),
            (
                ErrorCode::ValidationFailed,
                "VALIDATION_FAILED",
                ExitCode::ValidationFailed,
            ),
            (
                ErrorCode::AlreadyExists,
                "ALREADY_EXISTS",
                ExitCode::AlreadyExists,
            ),
            (
                ErrorCode::InvalidState,
                "INVALID_STATE",
                ExitCode::InvalidArgument,
            ),
            (ErrorCode::Blocked, "BLOCKED", ExitCode::ValidationFailed),
            (
                ErrorCode::GateFailed,
                "GATE_FAILED",
                ExitCode::ValidationFailed,
            ),
            (ErrorCode::IoError, "IO_ERROR", ExitCode::ExternalError),
            (ErrorCode::ParseError, "PARSE_ERROR", ExitCode::GenericError),
            (
                ErrorCode::ClaimRequiresGit,
                "CLAIM_REQUIRES_GIT",
                ExitCode::ExternalError,
            ),
            (
                ErrorCode::AmbiguousId,
                "AMBIGUOUS_ID",
                ExitCode::InvalidArgument,
            ),
            (
                ErrorCode::InvalidIdPrefix,
                "INVALID_ID_PREFIX",
                ExitCode::InvalidArgument,
            ),
            (
                ErrorCode::RepositoryNotFound,
                "REPOSITORY_NOT_FOUND",
                ExitCode::NotFound,
            ),
            (
                ErrorCode::RepositoryFormatTooNew,
                "REPOSITORY_FORMAT_TOO_NEW",
                ExitCode::ExternalError,
            ),
            (
                ErrorCode::StaleBinary,
                "STALE_BINARY",
                ExitCode::ExternalError,
            ),
            (
                ErrorCode::DeletionNotConfirmed,
                "DELETION_NOT_CONFIRMED",
                ExitCode::InvalidArgument,
            ),
            (
                ErrorCode::ProfileNotFound,
                "PROFILE_NOT_FOUND",
                ExitCode::NotFound,
            ),
            (
                ErrorCode::ProfileConflict,
                "PROFILE_CONFLICT",
                ExitCode::ValidationFailed,
            ),
            (
                ErrorCode::DependencyError,
                "DEPENDENCY_ERROR",
                ExitCode::NotFound,
            ),
            (ErrorCode::GateError, "GATE_ERROR", ExitCode::AlreadyExists),
            (
                ErrorCode::GateCheckError,
                "GATE_CHECK_ERROR",
                ExitCode::NotFound,
            ),
            (ErrorCode::PresetError, "PRESET_ERROR", ExitCode::NotFound),
            (
                ErrorCode::ItemNotFound,
                "ITEM_NOT_FOUND",
                ExitCode::GenericError,
            ),
            (
                ErrorCode::ItemCommandFailed,
                "ITEM_COMMAND_FAILED",
                ExitCode::GenericError,
            ),
            (
                ErrorCode::InvariantCommandFailed,
                "INVARIANT_COMMAND_FAILED",
                ExitCode::GenericError,
            ),
            (
                ErrorCode::ProjectCommandFailed,
                "PROJECT_COMMAND_FAILED",
                ExitCode::GenericError,
            ),
            (
                ErrorCode::ProfileError,
                "PROFILE_ERROR",
                ExitCode::GenericError,
            ),
            (
                ErrorCode::SearchFailed,
                "SEARCH_FAILED",
                ExitCode::GenericError,
            ),
            (
                ErrorCode::RipgrepNotFound,
                "RIPGREP_NOT_FOUND",
                ExitCode::GenericError,
            ),
            (
                ErrorCode::WorktreeInfoError,
                "WORKTREE_INFO_ERROR",
                ExitCode::GenericError,
            ),
            (
                ErrorCode::WorktreeListError,
                "WORKTREE_LIST_ERROR",
                ExitCode::GenericError,
            ),
            (
                ErrorCode::HooksInstallError,
                "HOOKS_INSTALL_ERROR",
                ExitCode::GenericError,
            ),
            (
                ErrorCode::GenericError,
                "GENERIC_ERROR",
                ExitCode::GenericError,
            ),
            (
                ErrorCode::RecoveryFailed,
                "recovery_failed",
                ExitCode::GenericError,
            ),
            (
                ErrorCode::ClaimAcquireError,
                "CLAIM_ACQUIRE_ERROR",
                ExitCode::NotFound,
            ),
            (
                ErrorCode::ClaimReleaseError,
                "CLAIM_RELEASE_ERROR",
                ExitCode::NotFound,
            ),
            (
                ErrorCode::ClaimRenewError,
                "CLAIM_RENEW_ERROR",
                ExitCode::NotFound,
            ),
            (
                ErrorCode::ClaimHeartbeatError,
                "CLAIM_HEARTBEAT_ERROR",
                ExitCode::NotFound,
            ),
            (
                ErrorCode::ClaimStatusError,
                "CLAIM_STATUS_ERROR",
                ExitCode::GenericError,
            ),
            (
                ErrorCode::ClaimListError,
                "CLAIM_LIST_ERROR",
                ExitCode::GenericError,
            ),
            (
                ErrorCode::ClaimForceEvictError,
                "CLAIM_FORCE_EVICT_ERROR",
                ExitCode::NotFound,
            ),
        ];

        assert_eq!(expected.len(), ErrorCode::ALL.len());
        for ((code, wire, status), listed) in expected.into_iter().zip(ErrorCode::ALL) {
            assert_eq!(code, listed);
            assert_eq!(code.as_str(), wire);
            assert_eq!(code.exit_code(), status);
            let error = JsonError::new(code, "failure");
            assert_eq!(error.exit_code(), status);
            assert_eq!(
                serde_json::to_value(&error).expect("registered error should serialize")["error"]
                    ["code"],
                wire
            );
        }
    }

    #[test]
    fn test_error_code_strings_round_trip_and_unknown_is_unresolved() {
        for code in ErrorCode::ALL {
            assert_eq!(code.as_str().parse::<ErrorCode>(), Ok(code));
            let error = JsonError::from_code_text(code.as_str(), "failure")
                .expect("every registered text code should construct an envelope");
            assert_eq!(error.error.code, code.as_str());
            assert_eq!(error.exit_code(), code.exit_code());
        }

        let unresolved = "UNREGISTERED_ERROR"
            .parse::<ErrorCode>()
            .expect_err("unknown input should stay unresolved");
        assert_eq!(unresolved.as_str(), "UNREGISTERED_ERROR");
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

    #[test]
    fn test_emitted_text_error_codes_resolve_without_changing_spelling() {
        let emitted = emitted_text_error_codes();
        assert!(
            !emitted.is_empty(),
            "the source-derived emitted-code set must not be vacuous"
        );

        for text in emitted {
            let code = text
                .parse::<ErrorCode>()
                .unwrap_or_else(|_| panic!("emitted code {text} must be registered"));
            assert_eq!(code.as_str(), text);

            let error =
                JsonError::legacy_unregistered(text, ExitCode::Success, "classification probe");
            assert_eq!(error.error.code, text);
            assert_eq!(error.exit_code(), code.exit_code());
        }
    }

    #[test]
    fn test_new_error_code_members_match_plain_failure_classes() {
        for code in [
            ErrorCode::DependencyError,
            ErrorCode::GateCheckError,
            ErrorCode::PresetError,
            ErrorCode::ClaimAcquireError,
            ErrorCode::ClaimReleaseError,
            ErrorCode::ClaimRenewError,
            ErrorCode::ClaimHeartbeatError,
            ErrorCode::ClaimForceEvictError,
        ] {
            assert_eq!(code.exit_code(), ExitCode::NotFound, "{code}");
        }

        assert_eq!(ErrorCode::GateError.exit_code(), ExitCode::AlreadyExists);

        for code in [
            ErrorCode::ItemNotFound,
            ErrorCode::ItemCommandFailed,
            ErrorCode::InvariantCommandFailed,
            ErrorCode::ProjectCommandFailed,
            ErrorCode::ProfileError,
            ErrorCode::SearchFailed,
            ErrorCode::RipgrepNotFound,
            ErrorCode::WorktreeInfoError,
            ErrorCode::WorktreeListError,
            ErrorCode::HooksInstallError,
            ErrorCode::GenericError,
            ErrorCode::RecoveryFailed,
            ErrorCode::ClaimStatusError,
            ErrorCode::ClaimListError,
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

    /// Create a test MinimalIssue
    fn test_minimal_issue() -> MinimalIssue {
        MinimalIssue {
            id: "test-id".to_string(),
            title: "Issue 1".to_string(),
            state: State::Ready,
            priority: Priority::Normal,
            assignee: None,
            labels: Vec::new(),
            archived_from: None,
        }
    }

    #[test]
    fn test_ready_query_response_serialization() {
        let issues = vec![test_minimal_issue()];
        let response = ReadyQueryResponse { issues, count: 1 };

        let json_output = JsonOutput::success(response);
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"count\": 1"));
        // Envelope fields removed - raw data only
        assert!(!serialized.contains("\"success\""));
        assert!(!serialized.contains("\"metadata\""));
        assert!(!serialized.contains("\"command\""));
    }

    #[test]
    fn test_blocked_query_response_serialization() {
        let blocked_issue = MinimalBlockedIssue {
            id: "test-id".to_string(),
            title: "Issue 1".to_string(),
            state: State::Ready,
            priority: Priority::Normal,
            assignee: None,
            labels: vec![],
            blocked_reasons: vec!["dep:abc123".to_string()],
        };

        let response = BlockedQueryResponse {
            issues: vec![blocked_issue],
            count: 1,
        };

        let json_output = JsonOutput::success(response);
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"blocked_reasons\""));
        // Envelope fields removed - raw data only
        assert!(!serialized.contains("\"success\""));
        assert!(!serialized.contains("\"command\""));
    }

    #[test]
    fn test_assignee_query_response() {
        let issues = vec![test_minimal_issue()];

        let response = AssigneeQueryResponse {
            assignee: "copilot:session-1".to_string(),
            issues,
            count: 1,
        };

        let json_output = JsonOutput::success(response);
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"assignee\""));
        assert!(serialized.contains("copilot:session-1"));
        // Envelope fields removed - raw data only
        assert!(!serialized.contains("\"command\""));
    }

    #[test]
    fn test_state_query_response() {
        let issues = vec![test_minimal_issue()];

        let response = StateQueryResponse {
            state: State::Ready,
            issues,
            count: 1,
        };

        let json_output = JsonOutput::success(response);
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"state\""));
        assert!(serialized.contains("\"ready\""));
        // Envelope fields removed - raw data only
        assert!(!serialized.contains("\"command\""));
    }

    #[test]
    fn test_priority_query_response() {
        let issues = vec![test_minimal_issue()];

        let response = PriorityQueryResponse {
            priority: Priority::High,
            issues,
            count: 1,
        };

        let json_output = JsonOutput::success(response);
        let serialized = json_output.to_json_string().unwrap();

        assert!(serialized.contains("\"priority\""));
        assert!(serialized.contains("\"high\""));
        // Envelope fields removed - raw data only
        assert!(!serialized.contains("\"command\""));
    }

    #[test]
    fn test_blocked_reason_types() {
        let dep = BlockedReason {
            reason_type: BlockedReasonType::Dependency,
            detail: "ABC".to_string(),
        };
        let gate = BlockedReason {
            reason_type: BlockedReasonType::Gate,
            detail: "test-gate".to_string(),
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
            key: "ci".to_string(),
            title: "CI".to_string(),
            description: "Continuous integration".to_string(),
            auto: true,
            example_integration: None,
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: None,
        };
        let v = serde_json::to_value(&def).unwrap();
        assert_eq!(v["stage"], "postcheck", "stage must be snake_case");
        assert_eq!(v["mode"], "auto", "mode must be snake_case");

        let def_pre = GateDefinition {
            key: "lint".to_string(),
            title: "Lint".to_string(),
            description: "Linting".to_string(),
            auto: false,
            example_integration: None,
            stage: GateStage::Precheck,
            mode: GateMode::Manual,
            checker: None,
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
            key: "tests".to_string(),
            title: "Tests".to_string(),
            description: "Run tests".to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: None,
            priority: 100,
            reserved: std::collections::HashMap::new(),
            auto: true,
            example_integration: None,
        };
        let def = GateDefinition::from(gate);
        assert_eq!(def.stage, GateStage::Postcheck);
        assert_eq!(def.mode, GateMode::Auto);
        let v = serde_json::to_value(&def).unwrap();
        assert_eq!(v["stage"], "postcheck");
        assert_eq!(v["mode"], "auto");
    }
}
