//! Classification of propagated failures into the machine-readable envelope.
//!
//! [`classify`] is the one library classifier shared by every consumer: it maps
//! a typed failure to its registered [`ErrorCode`], whose
//! [`ErrorCode::exit_code`] is the process status, and to the details,
//! suggestions and warnings its type determines.

use super::{ErrorCode, ExitCode, JsonError};

/// Classify a propagated failure by its typed cause.
///
/// The envelope carries the registered code, the error's message, the
/// details and suggestions its type determines, and the warnings it carries;
/// its [`JsonError::exit_code`] is that code's [`ErrorCode::exit_code`], so the
/// reported code and the process status cannot drift.
pub fn classify(error: &anyhow::Error) -> JsonError {
    let json =
        derive_guidance(error).apply(JsonError::new(classify_code(error), error.to_string()));
    match error.downcast_ref::<crate::commands::GatePassFailed>() {
        Some(failure) => json.with_warnings(failure.warnings.clone()),
        None => json,
    }
}

/// The details and suggestions a failure's type determines.
#[derive(Default)]
struct Guidance {
    details: Option<serde_json::Value>,
    suggestions: Vec<String>,
}

impl Guidance {
    fn of(json: JsonError) -> Self {
        Self {
            details: json.error.details,
            suggestions: json.error.suggestions,
        }
    }

    fn apply(self, json: JsonError) -> JsonError {
        let json = match self.details {
            Some(details) => json.with_details(details),
            None => json,
        };
        json.with_suggestions(self.suggestions)
    }
}

/// The guidance of the one typed failure `error` is; empty for an untyped or
/// self-explanatory one.
fn derive_guidance(error: &anyhow::Error) -> Guidance {
    use crate::commands::{
        GateNotRequiredError, GatePassFailed, GateStatusArgumentsTransposedError,
        ManualGateAttestationRequiredError, ProfileApplyError, ProfileNotFoundError,
        ProfileResolutionError,
    };
    use crate::errors::{
        DependencyBatchRejectedError, InvalidLabelPatternError, LinkedCheckoutWriteRefusedError,
        TransitionBlockedError,
    };
    use crate::profile::ProfileGraphError;
    use crate::repository_state::RepositoryStateError;

    if let Some(failure) = error.downcast_ref::<GatePassFailed>() {
        let verdict = match failure.status {
            crate::domain::GateRunStatus::Error => "error",
            _ => "fail",
        };
        let (issue, key) = (&failure.issue_id, &failure.gate_key);
        return Guidance {
            details: Some(serde_json::json!({
                "issue_id": issue,
                "key": key,
                "status": "failed",
                "verdict": verdict,
                "checker_result": super::GateRunSummary::full(&failure.result),
            })),
            suggestions: vec![
                format!("Inspect the checker result with: jit gate status {issue} {key}"),
                format!("View run history with: jit gate status {issue} {key} --all"),
                format!("Fix the failing gate and rerun: jit gate evaluate {issue} {key}"),
            ],
        };
    }
    if let Some(failed) = error.downcast_ref::<crate::commands::serve::ServeActionFailed>() {
        return derive_guidance(&failed.cause);
    }
    if let Some(exited) = error.downcast_ref::<crate::commands::serve::ForegroundServerExited>() {
        return Guidance {
            details: Some(serde_json::json!({
                "port": exited.port,
                "exit_code": exited.status.code(),
            })),
            suggestions: Vec::new(),
        };
    }
    if let Some(not_required) = error.downcast_ref::<GateNotRequiredError>() {
        let (issue, key) = (&not_required.issue_id, &not_required.gate_key);
        return Guidance {
            details: Some(serde_json::json!({ "issue_id": issue, "key": key })),
            suggestions: vec![format!("Add the gate first: jit gate add {issue} {key}")],
        };
    }
    if let Some(needs_attestor) = error.downcast_ref::<ManualGateAttestationRequiredError>() {
        let (issue, key) = (&needs_attestor.issue_id, &needs_attestor.gate_key);
        return Guidance {
            details: Some(serde_json::json!({ "issue_id": issue, "key": key })),
            suggestions: vec![format!(
                "Record the pass with: jit gate evaluate {issue} {key} --by <attestor>"
            )],
        };
    }
    if let Some(transposed) = error.downcast_ref::<GateStatusArgumentsTransposedError>() {
        return Guidance {
            details: Some(serde_json::json!({
                "issue_id": transposed.issue,
                "key": transposed.gate_key,
                "transposed": true,
            })),
            suggestions: vec![format!("Did you mean: {}", transposed.canonical_command())],
        };
    }
    if let Some(not_found) = error.downcast_ref::<crate::storage::IssueNotFoundError>() {
        return Guidance::of(JsonError::issue_not_found(not_found.id()));
    }
    if let Some(blocked) = error.downcast_ref::<TransitionBlockedError>() {
        return Guidance::of(JsonError::transition_blocked(blocked));
    }
    if error.is::<InvalidLabelPatternError>() {
        return Guidance {
            details: None,
            suggestions: vec![INVALID_LABEL_PATTERN_SUGGESTION.to_owned()],
        };
    }
    if let Some(refused) = error.downcast_ref::<crate::errors::DeletionNotConfirmedError>() {
        let id = refused.issue_id();
        return Guidance {
            details: Some(serde_json::json!({ "id": id })),
            suggestions: vec![format!(
                "Set JIT_ALLOW_DELETION=1 environment variable to proceed: \
                 JIT_ALLOW_DELETION=1 jit issue delete {id}"
            )],
        };
    }
    if let Some(refusal) = error.downcast_ref::<LinkedCheckoutWriteRefusedError>() {
        // The checkout, the stance that refused it, and how to permit the
        // operation (jit:f52567ed REQ-02).
        return Guidance {
            details: Some(serde_json::json!({
                "checkout": refusal.checkout(),
                "stance": refusal.stance(),
            })),
            suggestions: LinkedCheckoutWriteRefusedError::permit_guidance().to_vec(),
        };
    }
    if let Some(batch) = error.downcast_ref::<DependencyBatchRejectedError>() {
        return Guidance::of(dependency_batch_json_error(batch));
    }
    if let Some(id_error) = id_resolution_error(error) {
        return Guidance::of(id_error);
    }
    if let Some(ProfileApplyError::UncommittedTargets { paths }) =
        error.downcast_ref::<ProfileApplyError>()
    {
        return Guidance {
            details: Some(serde_json::json!({
                "paths": paths,
                "resolution": crate::commands::UNCOMMITTED_TARGETS_RESOLUTION,
            })),
            suggestions: Vec::new(),
        };
    }
    if error.is::<ProfileNotFoundError>()
        || matches!(
            error.downcast_ref::<ProfileResolutionError>(),
            Some(ProfileResolutionError::UnresolvableDependency { .. })
        )
        || matches!(
            error.downcast_ref::<ProfileGraphError>(),
            Some(ProfileGraphError::MissingDependency { .. })
        )
    {
        return Guidance {
            details: None,
            suggestions: vec![MISSING_PROFILE_SUGGESTION.to_owned()],
        };
    }
    if let Some(RepositoryStateError::ProfileTargetConflicts(conflicts)) =
        error.downcast_ref::<RepositoryStateError>()
    {
        return Guidance {
            details: serde_json::to_value(conflicts).ok(),
            suggestions: Vec::new(),
        };
    }
    if let Some(search_error) = error.downcast_ref::<crate::search::SearchError>() {
        return Guidance {
            details: None,
            suggestions: match search_error {
                crate::search::SearchError::RipgrepNotInstalled => {
                    vec![RIPGREP_INSTALL_SUGGESTION.to_owned()]
                }
                crate::search::SearchError::Failed(_) => Vec::new(),
            },
        };
    }
    Guidance::default()
}

const INVALID_LABEL_PATTERN_SUGGESTION: &str =
    "Use 'namespace:value' for an exact match or 'namespace:*' for a wildcard";
const MISSING_PROFILE_SUGGESTION: &str = "Place the package it depends on beside it";
const RIPGREP_INSTALL_SUGGESTION: &str =
    "Install ripgrep from https://github.com/BurntSushi/ripgrep";

/// The envelope of an ambiguous or too-short id prefix, carrying the offending
/// prefix (and the candidates it matches) in `details`.
pub(super) fn id_resolution_error(error: &anyhow::Error) -> Option<JsonError> {
    if let Some(prefix_error) =
        error.downcast_ref::<crate::domain::identity::InvalidIdPrefixError>()
    {
        return Some(
            JsonError::new(ErrorCode::InvalidIdPrefix, prefix_error.to_string())
                .with_details(serde_json::json!({ "prefix": prefix_error.prefix() })),
        );
    }
    error
        .downcast_ref::<crate::storage::AmbiguousIdError>()
        .map(|ambiguous| {
            JsonError::new(ErrorCode::AmbiguousId, ambiguous.to_string()).with_details(
                serde_json::json!({
                    "prefix": ambiguous.prefix(),
                    "matches": ambiguous.matches(),
                }),
            )
        })
}

/// The registered code of a failure. Human message text is deliberately absent
/// from this cascade, and its order is semantic.
fn classify_code(error: &anyhow::Error) -> ErrorCode {
    if let Some(failed) = error.downcast_ref::<crate::commands::serve::ServeActionFailed>() {
        return classify_code(&failed.cause);
    }
    // A rejected `jit dep add` batch (jit:c8518f2a) wraps every rejected edge's
    // own typed error; classify by the batch's dominant edge rather than this
    // wrapper itself, so a mixed batch still maps to the right exit code.
    if let Some(batch) = error.downcast_ref::<crate::errors::DependencyBatchRejectedError>() {
        return dependency_batch_error_code(batch);
    }
    // A leftover redo record that `jit recover` or a read cannot apply, or
    // recovery's failed repairs, is `recovery_failed`, unless it was contention.
    if let Some(failure) = error.downcast_ref::<crate::errors::RecoveryFailedError>() {
        return if crate::storage::is_lock_timeout(&failure.0) {
            ErrorCode::LockTimeout
        } else {
            ErrorCode::RecoveryFailed
        };
    }
    // A lock wait that expired is contention, not the command's own failure:
    // the command changed nothing and may be retried.
    if crate::storage::is_lock_timeout(error) {
        return ErrorCode::LockTimeout;
    }
    // A `gate evaluate` checker that ran but did not pass: split checker-failure
    // (verdict `fail`, validation error) from runner/infra error (verdict
    // `error`, external error). `Passed` never produces this error.
    if let Some(gate_failure) = error.downcast_ref::<crate::commands::GatePassFailed>() {
        return match gate_failure.status {
            crate::domain::GateRunStatus::Error => ErrorCode::IoError,
            _ => ErrorCode::GateFailed,
        };
    }
    // Targeting a gate the issue does not require is an argument/lookup error,
    // classified before the run path, never reported as a runner error.
    if error.is::<crate::commands::serve::ForegroundServerExited>() {
        return ErrorCode::ServerExited;
    }
    if error
        .downcast_ref::<crate::commands::GateNotRequiredError>()
        .is_some()
        || error
            .downcast_ref::<crate::commands::GateStatusArgumentsTransposedError>()
            .is_some()
    {
        return ErrorCode::InvalidArgument;
    }
    // A manual gate evaluated without --by (jit:1d59070d REQ-03): a usage
    // error, raised before any write, in the same family as gate define's
    // manual+checker-command conflict.
    if error
        .downcast_ref::<crate::commands::ManualGateAttestationRequiredError>()
        .is_some()
    {
        return ErrorCode::InvalidArgument;
    }
    // The linked-checkout write policy refused this invocation at dispatch. Like
    // the deletion refusal it sits beside in the vocabulary, it is a refusal
    // pending an explicit opt-in rather than a repository fault, so it takes the
    // invalid-argument status that class already has.
    if error
        .downcast_ref::<crate::errors::LinkedCheckoutWriteRefusedError>()
        .is_some()
    {
        return ErrorCode::LinkedCheckoutWriteRefused;
    }
    if let Some(blocked) = error.downcast_ref::<crate::errors::TransitionBlockedError>() {
        return blocked.error_code();
    }
    if error
        .downcast_ref::<crate::errors::ValidationFailedError>()
        .is_some()
        || error
            .downcast_ref::<crate::errors::RedundantDependencyError>()
            .is_some()
        || error
            .downcast_ref::<crate::repository_state::ProjectionError>()
            .is_some()
        || error
            .downcast_ref::<crate::repository_state::ManagedDocumentError>()
            .is_some()
    {
        return ErrorCode::ValidationFailed;
    }
    if matches!(
        error.downcast_ref::<crate::commands::ProfileApplyError>(),
        Some(crate::commands::ProfileApplyError::UncommittedTargets { .. })
    ) {
        return ErrorCode::ProfileUncommittedTargets;
    }
    if error
        .downcast_ref::<crate::profile::ProfileClaimError>()
        .is_some()
        || error
            .downcast_ref::<crate::commands::ProfileApplyError>()
            .is_some()
        // The sanctioned downcast of the typed repository-state error: every
        // refused target decision is a profile conflict; every other variant
        // keeps its classification below.
        || matches!(
            error.downcast_ref::<crate::repository_state::RepositoryStateError>(),
            Some(crate::repository_state::RepositoryStateError::ProfileTargetConflicts(_))
        )
    {
        return ErrorCode::ProfileConflict;
    }
    // Which package bytes a profile command reads, or which packages an
    // application applies, could not be settled. Classified ahead of the
    // source-chain searches below, because a location that is simply gone
    // carries a NotFound I/O cause.
    if error
        .downcast_ref::<crate::commands::ProfileResolutionError>()
        .is_some()
        || error
            .downcast_ref::<crate::commands::ProfileNotFoundError>()
            .is_some()
        || error
            .downcast_ref::<crate::profile::ProfileGraphError>()
            .is_some()
    {
        return ErrorCode::ProfileError;
    }
    // A search that could not run, or ran and failed (jit search).
    if let Some(search_error) = error.downcast_ref::<crate::search::SearchError>() {
        return match search_error {
            crate::search::SearchError::RipgrepNotInstalled => ErrorCode::RipgrepNotFound,
            crate::search::SearchError::Failed(_) => ErrorCode::SearchFailed,
        };
    }
    // An addressed item no scope declares.
    if error
        .downcast_ref::<crate::errors::ItemNotFoundError>()
        .is_some()
    {
        return ErrorCode::ItemNotFound;
    }

    // The repository-state derivation error is classified by an exhaustive match
    // over every current variant — no fallback arm — so a newly added variant forces
    // a deliberate classification here rather than silently taking the generic code.
    // Projection, managed-document, ambiguous-ownership, gate-registry-edit, and
    // archive-execution failures are validation failures (4); a profile-target
    // conflict is classified as a profile conflict (4) above and listed here only for
    // exhaustiveness;
    // a layout rejection is an argument error (2); capture/parse producer failures and
    // the structural delta/overlay/initialization/mutation failures keep the
    // generic code (1). The `project render` command additionally unwraps this error
    // to its bare inner `ProjectionError`/`ManagedDocumentError`, which the checks
    // above classify identically.
    if let Some(state_error) = error.downcast_ref::<crate::repository_state::RepositoryStateError>()
    {
        use crate::repository_state::RepositoryStateError;
        return match state_error {
            RepositoryStateError::Projection(_)
            | RepositoryStateError::ManagedDocument(_)
            | RepositoryStateError::ProfileTargetConflicts(_)
            | RepositoryStateError::AmbiguousOwnership(_)
            | RepositoryStateError::GateRegistryEdit(_)
            | RepositoryStateError::ArchiveExecution(_) => ErrorCode::ValidationFailed,
            RepositoryStateError::Layout(_) => ErrorCode::InvalidArgument,
            RepositoryStateError::Producer(_)
            | RepositoryStateError::Initialization(_)
            | RepositoryStateError::Delta(_)
            | RepositoryStateError::Overlay(_)
            | RepositoryStateError::Mutation(_) => ErrorCode::GenericError,
        };
    }
    // A failed batch-create pre-validation is an argument error (exit 2): no
    // writes happened, the file is malformed.
    if error
        .downcast_ref::<crate::commands::BatchValidationError>()
        .is_some()
    {
        return ErrorCode::InvalidArgument;
    }
    if error
        .downcast_ref::<crate::errors::RequiresGitError>()
        .is_some()
    {
        return ErrorCode::RequiresGit;
    }

    // A repository whose on-disk format this binary does not accept is an
    // external-dependency failure (exit 10): the binary and the repository
    // disagree, and the fix is the matching jit — the same family as the git and
    // filesystem dependency failures, and deliberately NOT NotFound (3), which
    // would misread as "repository/resource missing".
    if error
        .downcast_ref::<crate::storage::RepositoryFormatUnsupportedError>()
        .is_some()
    {
        return ErrorCode::RepositoryFormatUnsupported;
    }

    // Stored-record decoding stays typed across the library/binary boundary.
    // Structural membership defects are validation failures; malformed JSON is
    // a parse failure and a future schema remains an external compatibility
    // failure.
    if let Some(index_error) = error.downcast_ref::<crate::repository_state::RepositoryIndexError>()
    {
        use crate::repository_state::RepositoryIndexError;
        return match index_error {
            RepositoryIndexError::Parse(_) => ErrorCode::ParseError,
            RepositoryIndexError::UnsupportedVersion { .. } => {
                ErrorCode::RepositoryFormatUnsupported
            }
            RepositoryIndexError::DuplicateDeleted(_)
            | RepositoryIndexError::IssueFileName(_)
            | RepositoryIndexError::IssuesListingNotCaptured => ErrorCode::ValidationFailed,
        };
    }

    // TOML and JSON decoding are typed parser failures even when command
    // orchestration adds anyhow context around them (configuration, event-log,
    // migration, and server PID readers all retain their parser source).
    if error.chain().any(|cause| {
        cause.downcast_ref::<toml::de::Error>().is_some()
            || cause.downcast_ref::<serde_json::Error>().is_some()
    }) {
        return ErrorCode::ParseError;
    }

    // Check the complete source chain for I/O errors. Session and publication
    // boundaries wrap the original I/O error without changing its exit class.
    if let Some(io_error) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
    {
        return match io_error.kind() {
            std::io::ErrorKind::NotFound => ErrorCode::RepositoryNotFound,
            std::io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
            _ => ErrorCode::IoError,
        };
    }

    // Graph errors are typed: a cycle is a validation failure, a missing node is
    // a not-found condition. Classified by downcast, not by message text.
    if let Some(graph_error) =
        error.downcast_ref::<crate::graph::GraphError<crate::domain::identity::IssueId>>()
    {
        return match graph_error {
            crate::graph::GraphError::CycleDetected => ErrorCode::CycleDetected,
            crate::graph::GraphError::NodeNotFound { .. } => ErrorCode::IssueNotFound,
        };
    }

    // A missing storage-backed resource is a typed not-found condition (exit 3):
    // an issue or gate key (either backend), a gate-run record, a gate preset, the
    // repository itself, or a lease. Classified by downcast, not message text.
    if error
        .downcast_ref::<crate::storage::IssueNotFoundError>()
        .is_some()
        || error
            .downcast_ref::<crate::errors::NotFoundError>()
            .is_some()
    {
        return ErrorCode::IssueNotFound;
    }
    if error
        .downcast_ref::<crate::storage::GateNotFoundError>()
        .is_some()
        || error
            .downcast_ref::<crate::storage::GateRunNotFoundError>()
            .is_some()
    {
        return ErrorCode::GateNotFound;
    }
    if error
        .downcast_ref::<crate::storage::PresetNotFoundError>()
        .is_some()
    {
        return ErrorCode::PresetError;
    }
    if error
        .downcast_ref::<crate::storage::RepositoryNotFoundError>()
        .is_some()
    {
        return ErrorCode::RepositoryNotFound;
    }
    if error
        .downcast_ref::<crate::errors::LeaseNotFoundError>()
        .is_some()
    {
        return ErrorCode::IssueNotFound;
    }

    // An already-exists condition is typed: the gate-registry case plus the shared
    // AlreadyExistsError carrier (e.g. an occupied snapshot output path).
    if error
        .downcast_ref::<crate::storage::GateAlreadyExistsError>()
        .is_some()
        || error
            .downcast_ref::<crate::errors::AlreadyExistsError>()
            .is_some()
    {
        return ErrorCode::AlreadyExists;
    }

    // A malformed query label keeps its established public code rather than
    // collapsing into the generic invalid-argument member. The dedicated type
    // makes that distinction structural while retaining the same exit class.
    if error
        .downcast_ref::<crate::errors::InvalidLabelPatternError>()
        .is_some()
    {
        return ErrorCode::InvalidLabelPattern;
    }

    // Invalid-argument conditions are typed: the shared InvalidArgumentError, the
    // enum parse errors (gate stage/mode), and a UTF-8 decode failure of
    // subprocess/git output (`String::from_utf8` / `str::from_utf8`, possibly
    // behind a `.context(...)` describing which output) all map to exit code 2 by
    // downcast against their concrete types.
    // An ambiguous or too-short id prefix is an argument error (exit 2): the
    // caller gave an id that does not uniquely (or legally) resolve. Classified
    // by downcast against the typed storage errors, never by message text.
    if error
        .downcast_ref::<crate::storage::AmbiguousIdError>()
        .is_some()
        || error
            .downcast_ref::<crate::domain::identity::InvalidIdPrefixError>()
            .is_some()
    {
        return if error
            .downcast_ref::<crate::storage::AmbiguousIdError>()
            .is_some()
        {
            ErrorCode::AmbiguousId
        } else {
            ErrorCode::InvalidIdPrefix
        };
    }
    if error
        .downcast_ref::<crate::errors::InvalidArgumentError>()
        .is_some()
        || error
            .downcast_ref::<crate::domain::identity::InvalidGateKeyError>()
            .is_some()
        || error
            .downcast_ref::<crate::domain::identity::InvalidGateRunIdError>()
            .is_some()
        || error
            .downcast_ref::<crate::domain::AssigneeParseError>()
            .is_some()
        || error
            .downcast_ref::<crate::query_engine::QueryParseError>()
            .is_some_and(|parse_error| {
                matches!(
                    parse_error,
                    crate::query_engine::QueryParseError::InvalidAssignee { .. }
                )
            })
        || error
            .downcast_ref::<crate::domain::vocabulary::UnknownToken>()
            .is_some()
        || error
            .downcast_ref::<crate::document::DocumentScopeParseError>()
            .is_some()
        || error.downcast_ref::<std::string::FromUtf8Error>().is_some()
        || error.downcast_ref::<std::str::Utf8Error>().is_some()
    {
        return ErrorCode::InvalidArgument;
    }

    // A `jit issue delete` refused for missing operator confirmation
    // (jit:0daba57d) is an argument/usage error (exit 2), not the generic
    // fallback: the caller omitted the required `JIT_ALLOW_DELETION=1`
    // confirmation, the same family as a malformed or missing required value.
    if error
        .downcast_ref::<crate::errors::DeletionNotConfirmedError>()
        .is_some()
    {
        return ErrorCode::DeletionNotConfirmed;
    }

    // Path-based storage errors are typed via PathReadError; classify by variant.
    // A wrapped (`Other`) cause is re-classified by recursing on its inner error,
    // so an io::Error or an InvalidArgumentError nested in PathReadError still
    // reaches the right code.
    if let Some(path_error) = error.downcast_ref::<crate::storage::PathReadError>() {
        return match path_error {
            crate::storage::PathReadError::NotFound(_)
            | crate::storage::PathReadError::CommitNotFound(_) => ErrorCode::DocumentNotFound,
            crate::storage::PathReadError::InvalidPath(_)
            | crate::storage::PathReadError::OutsideRepoRoot(_) => ErrorCode::InvalidArgument,
            crate::storage::PathReadError::Other(inner) => classify_code(inner),
        };
    }

    // A template whose internal depends_on edges form a cycle is a validation
    // failure (4); other template-config errors are generic.
    if let Some(template_error) = error.downcast_ref::<crate::templates::TemplateConfigError>() {
        return match template_error {
            crate::templates::TemplateConfigError::CyclicDependsOn { .. } => {
                ErrorCode::ValidationFailed
            }
            _ => ErrorCode::GenericError,
        };
    }

    // A mutation session that kept losing its capture/apply race is reported
    // through the typed MutationSessionExhausted terminal error (jit:4b2005fe).
    // It keeps the generic exit code the bespoke "did not converge" bails
    // carried before the shared retry contract, made explicit here as a typed
    // classification rather than a message-text fallthrough.
    if error
        .downcast_ref::<crate::commands::MutationSessionExhausted>()
        .is_some()
    {
        return ErrorCode::GenericError;
    }

    // No typed classifier matched: a genuinely-unknown error keeps the historical
    // default exit code. Every condition the CLI deliberately distinguishes is
    // classified by a typed downcast above; classification is never driven by
    // matching against a human-readable error string.
    ErrorCode::GenericError
}

/// Pick the dominant exit code across every rejected edge of a `jit dep add`
/// batch (jit:c8518f2a). Resolution failures (bad/ambiguous id prefix, not
/// found — exit 2/3) always win over graph-validation failures (cycle, or a
/// rejected redundant edge — exit 4): resolution runs before graph
/// validation, so a resolution failure blocks that edge from ever reaching
/// graph validation and takes precedence when a batch mixes both classes.
/// Ties within the winning class are broken by request order (the first
/// offending edge wins). Each rejected edge is classified by recursing
/// through [`classify_code`], so this stays in lockstep with every other
/// typed classification above.
fn dependency_batch_error_code(batch: &crate::errors::DependencyBatchRejectedError) -> ErrorCode {
    batch
        .rejected()
        .iter()
        .map(|(_, error)| classify_code(error))
        .find(|code| code.exit_code() != ExitCode::ValidationFailed)
        .or_else(|| {
            batch
                .rejected()
                .first()
                .map(|(_, error)| classify_code(error))
        })
        .unwrap_or(ErrorCode::ValidationFailed)
}

/// Build a command handler's machine-readable error under `code`, except that a
/// lock wait that expired reports [`ErrorCode::LockTimeout`], so contention is
/// distinguishable from the command's own failure on every command.
pub fn command_json_error(error: &anyhow::Error, code: ErrorCode) -> JsonError {
    let code = if crate::storage::is_lock_timeout(error) {
        ErrorCode::LockTimeout
    } else {
        code
    };
    JsonError::new(code, error.to_string())
}

/// Build the `jit dep add` envelope for a rejected batch (jit:c8518f2a).
///
/// Every rejected edge is named under `details.rejected`, each carrying its own
/// classified `code`. The top-level `code`, `message`, and `suggestions` mirror
/// the batch's dominant edge, the one `classify_code` picks, so the JSON and
/// non-JSON paths agree on the exit code. That edge's kind-specific details are
/// kept and the batch-wide listing merged beside them.
pub fn dependency_batch_json_error(
    batch: &crate::errors::DependencyBatchRejectedError,
) -> JsonError {
    let from_id = batch.from_id();
    let edge_code = |err: &anyhow::Error| match classify_code(err) {
        ErrorCode::GenericError => ErrorCode::DependencyError,
        code => code,
    };

    let rejected = batch
        .rejected()
        .iter()
        .map(|(to, err)| {
            let code = edge_code(err);
            let message = if code == ErrorCode::CycleDetected {
                format!("Adding dependency would create a cycle: {from_id} -> {to}")
            } else {
                err.to_string()
            };
            serde_json::json!({
                "from": from_id,
                "to": to,
                "code": code.as_str(),
                "message": message,
            })
        })
        .collect::<Vec<_>>();

    let (to, err) = &batch.rejected()[batch
        .rejected()
        .iter()
        .position(|(_, err)| classify_code(err).exit_code() != ExitCode::ValidationFailed)
        .unwrap_or(0)];
    let mut json_error = match edge_code(err) {
        ErrorCode::CycleDetected => JsonError::cycle_detected(from_id, to),
        ErrorCode::IssueNotFound => JsonError::issue_not_found(to),
        ErrorCode::ValidationFailed => command_json_error(err, ErrorCode::ValidationFailed),
        _ => super::refine_id_error(err, command_json_error(err, ErrorCode::DependencyError)),
    };

    let mut details = json_error
        .error
        .details
        .take()
        .unwrap_or_else(|| serde_json::json!({}));
    if let serde_json::Value::Object(map) = &mut details {
        map.insert("from_id".to_string(), serde_json::json!(from_id));
        map.insert("rejected".to_string(), serde_json::Value::Array(rejected));
    }
    json_error.error.details = Some(details);
    json_error
}

#[cfg(test)]
mod exit_code_projection_tests {
    //! Pin the exit status [`classify`] gives representative typed errors.

    use super::classify;
    use crate::declarations::GateStage;
    use crate::domain::{GateRunResult, GateRunStatus};
    use crate::output::ErrorCode;

    /// Build a `gate evaluate` checker failure carrying `status`, which
    /// `classify` splits into checker verdict `4` and runner error `10`.
    pub(super) fn gate_pass_failed(status: GateRunStatus) -> anyhow::Error {
        let result = GateRunResult {
            schema_version: 1,
            run_id: crate::test_utils::gate_run_id(1),
            gate_key: "tests".parse().unwrap(),
            stage: GateStage::Precheck,
            issue_id: crate::test_utils::issue_id(85),
            commit: None,
            branch: None,
            status,
            started_at: chrono::Utc::now(),
            completed_at: None,
            duration_ms: None,
            exit_code: Some(1),
            stdout: String::new(),
            stderr: String::new(),
            command: "false".to_string(),
            by: None,
            message: None,
            findings: None,
            inputs_digest: None,
            origin: crate::domain::GateVerdictOrigin::Executed,
            evaluation_stability: crate::domain::GateEvaluationStability::Stable,
        };
        crate::commands::GatePassFailed {
            issue_id: crate::test_utils::issue_id(85),
            gate_key: "tests".parse().unwrap(),
            status,
            exit_code: Some(1),
            result,
            warnings: vec!["lease warning".to_owned()],
        }
        .into()
    }

    /// Representative typed errors, each paired with its expected exit status.
    fn classifier_cases() -> Vec<(anyhow::Error, i32)> {
        use std::io::{Error as IoError, ErrorKind};
        vec![
            (gate_pass_failed(GateRunStatus::Failed), 4),
            (gate_pass_failed(GateRunStatus::Error), 10),
            (anyhow::anyhow!("untyped failure"), 1),
            (
                crate::errors::InvalidArgumentError::new("bad arg").into(),
                2,
            ),
            (
                crate::storage::AmbiguousIdError::issue(
                    "aaaa",
                    ["aaaa1111".to_string(), "aaaa2222".to_string()],
                )
                .into(),
                2,
            ),
            (
                "Bad_Key"
                    .parse::<crate::domain::identity::GateKey>()
                    .unwrap_err()
                    .into(),
                2,
            ),
            (
                crate::domain::identity::InvalidIdPrefixError::new("ab").into(),
                2,
            ),
            (
                crate::commands::GateNotRequiredError {
                    issue_id: crate::test_utils::issue_id(85),
                    gate_key: "tests".parse().unwrap(),
                }
                .into(),
                2,
            ),
            (
                crate::storage::IssueNotFoundError::new(crate::test_utils::issue_id(86)).into(),
                3,
            ),
            (IoError::new(ErrorKind::NotFound, "missing").into(), 3),
            (crate::errors::ValidationFailedError::new("bad").into(), 4),
            (
                IoError::new(ErrorKind::PermissionDenied, "denied").into(),
                5,
            ),
            (
                anyhow::Error::new(
                    serde_json::from_str::<serde_json::Value>("{")
                        .expect_err("fixture JSON is malformed"),
                )
                .context("outer command context"),
                1,
            ),
            (
                crate::storage::RepositoryFormatUnsupportedError::new(9999, 1).into(),
                10,
            ),
            (
                crate::graph::GraphError::<crate::domain::identity::IssueId>::CycleDetected.into(),
                4,
            ),
            (
                crate::errors::RedundantDependencyError::new(
                    (
                        crate::test_utils::issue_id(1),
                        crate::test_utils::issue_id(2),
                    ),
                    vec![(
                        crate::test_utils::issue_id(1),
                        crate::test_utils::issue_id(3),
                    )],
                )
                .into(),
                4,
            ),
            (
                crate::storage::GateAlreadyExistsError::new(&"tests".parse().unwrap()).into(),
                6,
            ),
            (
                crate::errors::AlreadyExistsError::new("Output path already exists: taken").into(),
                6,
            ),
            (
                crate::commands::BatchValidationError {
                    problems: vec![crate::commands::BatchValidationProblem::UnknownDependency {
                        key: "a".to_string(),
                        missing: "ghost".to_string(),
                    }],
                }
                .into(),
                2,
            ),
            (
                crate::errors::RequiresGitError::new(
                    crate::errors::GitRequirementGap::NoRepository,
                )
                .into(),
                10,
            ),
        ]
    }

    #[test]
    fn test_classify_produces_documented_exit_codes() {
        for (error, expected) in classifier_cases() {
            assert_eq!(
                classify(&error).exit_code().code(),
                expected,
                "classifier produced the wrong code for `{error}`"
            );
        }
    }

    #[test]
    fn test_classify_assignee_parse_failures_as_invalid_arguments() {
        let assignee_error =
            anyhow::Error::new("invalid".parse::<crate::domain::Assignee>().unwrap_err());
        assert_eq!(
            classify(&assignee_error).error.code,
            ErrorCode::InvalidArgument
        );

        let filter_error =
            anyhow::Error::new(crate::query_engine::QueryParseError::InvalidAssignee {
                value: "invalid".to_string(),
                reason: "invalid".parse::<crate::domain::Assignee>().unwrap_err(),
            });
        assert_eq!(
            classify(&filter_error).error.code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn test_error_code_classifier_is_the_exit_classifier_authority() {
        for (index, (error, _expected)) in classifier_cases().into_iter().enumerate() {
            let error_code = classify(&error).error.code;
            assert_eq!(
                classify(&error).exit_code(),
                error_code.exit_code(),
                "reported code and process status diverged for `{error}`"
            );
            if index != 2 {
                assert_ne!(
                    error_code,
                    ErrorCode::GenericError,
                    "typed failure fell through to the generic code: `{error}`"
                );
            }
        }
    }

    #[test]
    fn test_error_code_classifier_recognizes_context_wrapped_json_parse_error() {
        let error = anyhow::Error::new(
            serde_json::from_str::<serde_json::Value>("{").expect_err("fixture JSON is malformed"),
        )
        .context("outer command context");

        assert_eq!(classify(&error).error.code, ErrorCode::ParseError);
        assert_eq!(
            classify(&error).exit_code(),
            ErrorCode::ParseError.exit_code()
        );
    }

    #[test]
    fn test_invalid_label_pattern_keeps_specific_registered_code() {
        let error = anyhow::Error::new(crate::errors::InvalidLabelPatternError::new(
            "bad label pattern",
        ));

        assert_eq!(classify(&error).error.code, ErrorCode::InvalidLabelPattern);
        assert_eq!(
            classify(&error).exit_code(),
            ErrorCode::InvalidLabelPattern.exit_code()
        );
    }
}

#[cfg(test)]
mod repository_state_classifier_tests {
    //! REQ-01/REQ-02: pin the exhaustive `RepositoryStateError` exit-code mapping
    //! to the real classifier ([`classify`]). Each representative
    //! variant is asserted so a reclassification (or a mis-mapped newly added
    //! variant) fails here rather than silently changing a command's exit status.

    use super::classify;
    use crate::domain::identity::ProfileId;
    use crate::repository_state::{
        AmbiguousOwnershipError, ArchiveExecutionError, Contribution, GateRegistryEditError,
        InitializationError, ManagedDocumentError, ProducerError, ProfileTargetConflict,
        ProfileTargetConflictEntry, ProfileTargetConflictsError, ProfileTargetSubject,
        ProjectionError, RepositoryLayoutError, RepositoryStateError, ScalarTarget, VirtualPath,
    };

    fn path() -> VirtualPath {
        VirtualPath::data("issues/abc123.json").unwrap()
    }

    /// A refusal carrying a contested file target and a contested declaration,
    /// which is what a selection whose packages define both differently produces.
    fn target_conflicts() -> ProfileTargetConflictsError {
        let contested = || ProfileTargetConflict::Contested {
            owners: vec![
                ProfileId::try_from("candidate").unwrap(),
                ProfileId::try_from("occupant").unwrap(),
            ],
        };
        ProfileTargetConflictsError::new(vec![
            ProfileTargetConflictEntry {
                owner: "candidate".try_into().expect("a canonical profile id"),
                subject: ProfileTargetSubject::File { path: path() },
                conflict: contested(),
            },
            ProfileTargetConflictEntry {
                owner: "candidate".try_into().expect("a canonical profile id"),
                subject: ProfileTargetSubject::Contribution {
                    identity: Contribution::Scalar {
                        target: ScalarTarget::ValidationDefaultType,
                        value: "task".to_string(),
                    }
                    .semantic_identity(),
                },
                conflict: contested(),
            },
        ])
    }

    /// Each repository-state variant paired with the exit code the exhaustive match
    /// must produce: validation failures (4) for the semantic-boundary variants, an
    /// argument error (2) for a layout rejection, and the generic code (1) for the
    /// capture/parse producer and structural initialization families.
    fn state_error_cases() -> Vec<(RepositoryStateError, i32)> {
        vec![
            (ProjectionError::UnknownKind { kind: "k".into() }.into(), 4),
            (
                ManagedDocumentError::DuplicateRegionIdentity("r".into()).into(),
                4,
            ),
            (target_conflicts().into(), 4),
            (
                AmbiguousOwnershipError::DuplicateRuleName("dup".into()).into(),
                4,
            ),
            (GateRegistryEditError::MissingEdit.into(), 4),
            (
                ArchiveExecutionError::MissingDestination("d".into()).into(),
                4,
            ),
            (RepositoryLayoutError::LexicalEscape("p".into()).into(), 2),
            (ProducerError::UnknownProjection("p".into()).into(), 1),
            (
                RepositoryStateError::Initialization(InitializationError::UnexpectedOccupant {
                    path: path(),
                }),
                1,
            ),
        ]
    }

    #[test]
    fn test_classify_maps_every_repository_state_class() {
        for (state_error, expected) in state_error_cases() {
            let error = anyhow::Error::new(state_error);
            assert_eq!(
                classify(&error).exit_code().code(),
                expected,
                "classifier produced the wrong code for `{error}`"
            );
        }
    }

    #[test]
    fn test_classify_profile_repository_state_failures_report_their_classified_code() {
        // A refused profile decision is a profile conflict (exit 4); an
        // unreadable registry keeps the generic code (exit 1); a layout rejection
        // is an argument error. None is substituted with `PROFILE_ERROR`.
        let cases: Vec<(RepositoryStateError, crate::output::ErrorCode, i32)> = vec![
            (
                target_conflicts().into(),
                crate::output::ErrorCode::ProfileConflict,
                4,
            ),
            (
                ProducerError::ProfileRegistryNotFile { target: "t".into() }.into(),
                crate::output::ErrorCode::GenericError,
                1,
            ),
            (
                RepositoryLayoutError::LexicalEscape("p".into()).into(),
                crate::output::ErrorCode::InvalidArgument,
                2,
            ),
        ];
        for (state_error, code, exit) in cases {
            let error = anyhow::Error::new(state_error);
            let json = classify(&error);
            assert_eq!(json.error.code, code, "{error}");
            assert_ne!(json.error.code, crate::output::ErrorCode::ProfileError);
            assert_eq!(json.exit_code().code(), exit, "{error}");
        }
    }

    /// REQ-04: the conflict code, exit status, and rendered human message a
    /// refused publication carries are unaffected by attaching structured
    /// details beside them — the message stays exactly the
    /// sentence [`RepositoryStateError`]'s `Display` renders, byte for byte,
    /// carrying every fact its structured form carries so a reader of either
    /// acts on the same evidence.
    #[test]
    fn test_classify_profile_conflict_keeps_its_code_exit_and_message_stable() {
        let state_error: RepositoryStateError = target_conflicts().into();
        let error = anyhow::Error::new(state_error);
        let rendered = error.to_string();

        let json = classify(&error);

        assert_eq!(json.error.code, crate::output::ErrorCode::ProfileConflict);
        assert_eq!(json.exit_code().code(), 4);
        assert_eq!(json.error.message, rendered);
        for expected in [
            "2 profile decision(s) cannot be published",
            ".jit/issues/abc123.json",
            "scalar:validation-default-type",
            "package occupant",
        ] {
            assert!(
                rendered.contains(expected),
                "the conflict message omits '{expected}': {rendered}"
            );
        }
    }

    /// A refused publication's JSON envelope names every conflicting target
    /// beside its conflict class and the packages defining it differently, and
    /// suggests no resolution.
    #[test]
    fn test_classify_profile_conflict_attaches_structured_details_without_remedies() {
        let state_error: RepositoryStateError = target_conflicts().into();
        let error = anyhow::Error::new(state_error);

        let json = classify(&error);

        let details = json.error.details.expect("a conflict attaches details");
        let conflicts = details["conflicts"]
            .as_array()
            .expect("details carry the conflict list");
        assert_eq!(conflicts.len(), 2);
        let file = &conflicts[0];
        assert_eq!(file["owner"], "candidate");
        assert_eq!(file["subject"]["kind"], "file");
        assert_eq!(file["subject"]["path"], ".jit/issues/abc123.json");
        assert_eq!(file["conflict"]["kind"], "contested");
        assert_eq!(conflicts[1]["subject"]["kind"], "contribution");
        assert!(
            conflicts
                .iter()
                .all(|conflict| conflict["conflict"].get("remedy").is_none()),
            "no conflict carries a remedy: {conflicts:?}"
        );
        assert!(
            json.error.suggestions.is_empty(),
            "{:?}",
            json.error.suggestions
        );
    }

    #[test]
    fn test_classify_profile_conflict_details_conform_to_the_published_schema() {
        let error = anyhow::Error::new(RepositoryStateError::from(target_conflicts()));
        let details = classify(&error)
            .error
            .details
            .expect("a conflict attaches details");

        let schema = crate::schema::CommandSchema::generate();
        let validator = jsonschema::validator_for(&schema.types["ProfileTargetConflictsError"])
            .expect("the published conflict schema compiles");

        assert!(validator.is_valid(&details), "{details}");
        assert!(
            !validator.is_valid(&serde_json::json!({ "conflicts": [{}] })),
            "the schema constrains each conflict entry"
        );
    }

    #[test]
    fn test_every_profile_graph_error_variant_classifies_as_profile_error_with_its_exit_status() {
        use crate::profile::ProfileGraphError as Graph;

        let text = |value: &str| value.to_string();
        let variants = vec![
            Graph::SelectedPackageMissing { id: text("a") },
            Graph::IncompatibleEngine {
                package: text("a"),
                required: text("x"),
                actual: text("y"),
            },
            Graph::MissingDependency {
                package: text("a"),
                dependency: text("b"),
            },
            Graph::DependencyVersionMismatch {
                package: text("a"),
                dependency: text("b"),
                required: text("x"),
                found: text("y"),
            },
            Graph::IncompatiblePackages {
                package: text("a"),
                other: text("b"),
                requirement: text("x"),
                other_version: text("y"),
            },
            Graph::DependencyCycle {
                cycle: vec![text("a"), text("a")],
            },
            Graph::ConflictingPackageIdentity {
                id: text("a"),
                first_hash: text("x"),
                second_hash: text("y"),
            },
        ];

        for variant in variants {
            let error = anyhow::Error::new(variant);
            let rendered = classify(&error);
            assert_eq!(
                rendered.error.code,
                crate::output::ErrorCode::ProfileError,
                "{error}"
            );
            assert_eq!(
                rendered.exit_code(),
                crate::output::ExitCode::GenericError,
                "{error}"
            );
        }
    }
}

#[cfg(test)]
mod lock_timeout_classification_tests {
    //! A lock wait that expired is reported as contention wherever it surfaces:
    //! through the shared classifier and through a handler's own JSON error,
    //! including when a mutation session wraps it and callers add context.

    use super::{classify, command_json_error};
    use crate::output::{ErrorCode, ExitCode};
    use crate::storage::{LockMode, LockTimeout, RepositoryStateStoreError};

    fn session_lock_timeout() -> anyhow::Error {
        let timeout = LockTimeout {
            mode: LockMode::Exclusive,
            path: ".jit-bootstrap.lock".into(),
            waited: std::time::Duration::from_secs(1),
        };
        anyhow::Error::new(RepositoryStateStoreError::Other(timeout.into()))
            .context("issue update could not open its mutation session")
    }

    #[test]
    fn test_classify_reports_wrapped_session_lock_timeout() {
        assert_eq!(
            classify(&session_lock_timeout()).error.code,
            ErrorCode::LockTimeout
        );
    }

    #[test]
    fn test_classify_recovery_failed_error_is_recovery_failed_unless_contention() {
        use crate::errors::RecoveryFailedError;
        let failed = RecoveryFailedError(anyhow::anyhow!("redo record"));
        let contended = RecoveryFailedError(session_lock_timeout());

        assert_eq!(
            classify(&failed.into()).error.code,
            ErrorCode::RecoveryFailed
        );
        assert_eq!(
            classify(&contended.into()).error.code,
            ErrorCode::LockTimeout
        );
    }

    #[test]
    fn test_command_json_error_reports_lock_timeout_over_handler_code() {
        [
            ErrorCode::GenericError,
            ErrorCode::GateError,
            ErrorCode::PresetError,
        ]
        .into_iter()
        .for_each(|handler_code| {
            assert_eq!(
                command_json_error(&session_lock_timeout(), handler_code).exit_code(),
                ExitCode::LockTimeout
            );
        });
    }

    #[test]
    fn test_command_json_error_keeps_handler_code_for_other_failures() {
        let error = anyhow::anyhow!("gate registry is malformed");
        assert_eq!(
            command_json_error(&error, ErrorCode::GateError).exit_code(),
            ErrorCode::GateError.exit_code()
        );
    }
}

#[cfg(test)]
mod derived_guidance_tests {
    //! The details and suggestions [`classify`] derives from a failure's type
    //! alone, so a call site never has to build them. Each test owns one
    //! derivation and asserts what the guidance says about the failure.

    use super::exit_code_projection_tests::gate_pass_failed;
    use super::{classify, dependency_batch_json_error};
    use crate::commands::{
        GateNotRequiredError, GateStatusArgumentsTransposedError,
        ManualGateAttestationRequiredError, ProfileApplyError, ProfileNotFoundError,
        ProfileResolutionError,
    };
    use crate::domain::identity::{InvalidIdPrefixError, ProfileId};
    use crate::domain::{GateRunStatus, GateStatus, State};
    use crate::errors::{
        DependencyBatchRejectedError, InvalidLabelPatternError, ItemNotFoundError,
        LinkedCheckoutWriteRefusedError, TransitionBlockedError, TransitionBlocker,
    };
    use crate::output::{ErrorCode, JsonError};
    use crate::storage::{AmbiguousIdError, IssueNotFoundError};
    use crate::test_utils::issue_id;

    fn gate_key() -> crate::domain::identity::GateKey {
        "tests".parse().unwrap()
    }

    fn details_of(json: &JsonError) -> &serde_json::Value {
        json.error
            .details
            .as_ref()
            .expect("the failure carries details")
    }

    /// Assert the envelope carries exactly one suggestion per marker group, in
    /// order, each naming every marker of its group (its action and subject).
    fn assert_suggestions(json: &JsonError, expected: &[&[&str]]) {
        let actual = &json.error.suggestions;
        assert_eq!(actual.len(), expected.len(), "{actual:?}");
        for (suggestion, markers) in actual.iter().zip(expected) {
            for marker in *markers {
                assert!(
                    suggestion.contains(marker),
                    "suggestion lacks `{marker}`: {suggestion}"
                );
            }
        }
    }

    #[test]
    fn test_classify_gate_pass_failed_carries_verdict_checker_result_and_gate_commands() {
        for (status, verdict, code) in [
            (GateRunStatus::Failed, "fail", ErrorCode::GateFailed),
            (GateRunStatus::Error, "error", ErrorCode::IoError),
        ] {
            let json = classify(&gate_pass_failed(status));

            assert_eq!(json.error.code, code);
            let details = details_of(&json);
            assert_eq!(details["verdict"], verdict);
            assert_eq!(details["issue_id"], issue_id(85).to_string());
            assert_eq!(details["key"], "tests");
            let issue = issue_id(85).to_string();
            let checker = &details["checker_result"];
            assert_eq!(checker["key"], "tests");
            assert_eq!(checker["status"], status.as_str());
            assert_eq!(checker["exit_code"], 1);
            assert_eq!(checker["command"], "false");
            assert_eq!(details["status"], "failed");
            assert_eq!(json.warnings, ["lease warning"]);
            assert_suggestions(
                &json,
                &[
                    &["Inspect", "gate status", &issue, "tests"],
                    &["history", "gate status", "--all", &issue, "tests"],
                    &["rerun", "gate evaluate", &issue, "tests"],
                ],
            );
        }
    }

    #[test]
    fn test_classify_gate_not_required_names_the_gate_and_how_to_add_it() {
        let error: anyhow::Error = GateNotRequiredError {
            issue_id: issue_id(1),
            gate_key: gate_key(),
        }
        .into();

        let json = classify(&error);

        assert_eq!(json.error.code, ErrorCode::InvalidArgument);
        let details = details_of(&json);
        assert_eq!(details["issue_id"], issue_id(1).to_string());
        assert_eq!(details["key"], "tests");
        assert!(details.get("verdict").is_none(), "no checker ran");
        let issue = issue_id(1).to_string();
        assert_suggestions(&json, &[&["gate add", &issue, "tests"]]);
    }

    #[test]
    fn test_classify_manual_gate_without_attestor_suggests_supplying_one() {
        let error: anyhow::Error = ManualGateAttestationRequiredError {
            issue_id: issue_id(1),
            gate_key: gate_key(),
        }
        .into();

        let json = classify(&error);

        assert_eq!(json.error.code, ErrorCode::InvalidArgument);
        assert_eq!(details_of(&json)["key"], "tests");
        assert!(details_of(&json).get("verdict").is_none());
        let issue = issue_id(1).to_string();
        assert_suggestions(
            &json,
            &[&["gate evaluate", "--by", issue.as_str(), "tests"]],
        );
    }

    #[test]
    fn test_classify_transposed_gate_status_arguments_suggests_the_canonical_order() {
        let transposed = GateStatusArgumentsTransposedError {
            issue: "abc12345".to_owned(),
            gate_key: "tests".to_owned(),
        };
        let error: anyhow::Error = transposed.into();

        let json = classify(&error);

        assert_eq!(json.error.code, ErrorCode::InvalidArgument);
        let details = details_of(&json);
        assert_eq!(details["transposed"], true);
        assert_eq!(details["issue_id"], "abc12345");
        assert_eq!(details["key"], "tests");
        assert_suggestions(
            &json,
            &[&["Did you mean", "gate status", "abc12345", "tests"]],
        );
        let suggestion = &json.error.suggestions[0];
        assert!(
            suggestion.find("abc12345") < suggestion.find("tests"),
            "the issue comes before the gate: {suggestion}"
        );
        assert!(error.to_string().contains("transposed"));
    }

    #[test]
    fn test_classify_issue_not_found_names_the_missing_id_in_details() {
        let error: anyhow::Error = IssueNotFoundError::new(issue_id(7)).into();

        let json = classify(&error);

        assert_eq!(json.error.code, ErrorCode::IssueNotFound);
        assert_eq!(details_of(&json)["issue_id"], issue_id(7).to_string());
        assert_suggestions(&json, &[&["query all"], &["Check", "issue ID"]]);
        assert_eq!(json.error.message, error.to_string());
    }

    #[test]
    fn test_classify_transition_blocked_carries_blockers_and_remediation() {
        let blocked = TransitionBlockedError::dependencies(
            issue_id(1),
            State::Ready,
            State::Backlog,
            vec![TransitionBlocker::MissingDependency {
                issue_id: issue_id(2),
            }],
        );
        let remediation = blocked.remediation_commands();
        let error: anyhow::Error = blocked.into();

        let json = classify(&error);

        assert_eq!(json.error.code, ErrorCode::Blocked);
        let details = details_of(&json);
        assert_eq!(details["issue_id"], issue_id(1).to_string());
        assert_eq!(details["blockers"].as_array().map(Vec::len), Some(1));
        assert_eq!(json.error.suggestions, remediation);
        let (issue, missing) = (issue_id(1).to_string(), issue_id(2).to_string());
        assert_suggestions(
            &json,
            &[
                &["graph deps", &issue],
                &["validate", "missing dependency", &missing],
                &["issue assign", &issue],
            ],
        );
    }

    #[test]
    fn test_classify_transition_blocked_by_gate_is_a_validation_failure() {
        let error: anyhow::Error = TransitionBlockedError::gates(
            issue_id(1),
            State::Done,
            State::Gated,
            vec![(
                gate_key(),
                GateStatus::Pending,
                crate::declarations::GateMode::Manual,
            )],
        )
        .into();

        let json = classify(&error);

        assert_eq!(json.error.code, ErrorCode::ValidationFailed);
        let issue = issue_id(1).to_string();
        assert_suggestions(
            &json,
            &[
                &["gate status-all", &issue],
                &["gate evaluate", "--by", &issue, "tests"],
                &["gate status", "--all", &issue, "tests"],
            ],
        );
    }

    #[test]
    fn test_classify_invalid_label_pattern_suggests_the_accepted_forms() {
        let error: anyhow::Error = InvalidLabelPatternError::new("bad").into();

        let json = classify(&error);

        assert_eq!(json.error.code, ErrorCode::InvalidLabelPattern);
        assert_suggestions(&json, &[&["namespace:value", "namespace:*"]]);
    }

    #[test]
    fn test_classify_linked_checkout_refusal_names_checkout_stance_and_permit_routes() {
        let refusal = LinkedCheckoutWriteRefusedError::new(
            "/work/linked",
            crate::domain::LinkedCheckoutWriteStance::Refuse,
        );
        let stance = refusal.stance();
        let error: anyhow::Error = refusal.into();

        let json = classify(&error);

        assert_eq!(json.error.code, ErrorCode::LinkedCheckoutWriteRefused);
        let details = details_of(&json);
        assert_eq!(details["checkout"], "/work/linked");
        assert_eq!(details["stance"], serde_json::to_value(stance).unwrap());
        assert_suggestions(
            &json,
            &[
                &["write_policy", "allow", ".jit/config.toml"],
                &[
                    crate::config_manager::LINKED_CHECKOUT_WRITE_STANCE_ENV,
                    "allow",
                ],
            ],
        );
    }

    #[test]
    fn test_classify_ambiguous_id_lists_the_prefix_and_its_candidates() {
        let error: anyhow::Error =
            AmbiguousIdError::issue("aaaa", ["aaaa1111".to_string(), "aaaa2222".to_string()])
                .into();

        let json = classify(&error);

        assert_eq!(json.error.code, ErrorCode::AmbiguousId);
        let details = details_of(&json);
        assert_eq!(details["prefix"], "aaaa");
        assert_eq!(
            details["matches"],
            serde_json::json!(["aaaa1111", "aaaa2222"])
        );
        assert!(json.error.suggestions.is_empty());
    }

    #[test]
    fn test_classify_invalid_id_prefix_names_the_prefix() {
        let error: anyhow::Error = InvalidIdPrefixError::new("ab").into();

        let json = classify(&error);

        assert_eq!(json.error.code, ErrorCode::InvalidIdPrefix);
        assert_eq!(details_of(&json)["prefix"], "ab");
    }

    fn batch(rejected: Vec<(&str, anyhow::Error)>) -> anyhow::Error {
        DependencyBatchRejectedError::new(
            issue_id(1),
            rejected
                .into_iter()
                .map(|(to, error)| (to.to_owned(), error))
                .collect(),
        )
        .into()
    }

    #[test]
    fn test_classify_dependency_batch_names_every_rejected_edge_beside_the_dominant_edge() {
        let error = batch(vec![
            (
                "cycle-target",
                crate::graph::GraphError::<crate::domain::identity::IssueId>::CycleDetected.into(),
            ),
            ("ghost", IssueNotFoundError::new(issue_id(9)).into()),
        ]);

        let json = classify(&error);

        assert_eq!(json.error.code, ErrorCode::IssueNotFound, "resolution wins");
        let details = details_of(&json);
        assert_eq!(details["from_id"], issue_id(1).to_string());
        let rejected = details["rejected"].as_array().expect("edge listing");
        assert_eq!(rejected.len(), 2);
        assert_eq!(rejected[0]["code"], ErrorCode::CycleDetected.as_str());
        assert_eq!(rejected[0]["to"], "cycle-target");
        assert_eq!(rejected[1]["code"], ErrorCode::IssueNotFound.as_str());
        assert_eq!(
            details["issue_id"], "ghost",
            "the dominant edge's own details survive"
        );
        assert_suggestions(&json, &[&["query all"], &["Check", "issue ID"]]);
    }

    #[test]
    fn test_classify_dependency_batch_agrees_with_its_dedicated_envelope() {
        let error = batch(vec![(
            "loop",
            crate::graph::GraphError::<crate::domain::identity::IssueId>::CycleDetected.into(),
        )]);
        let batch_error = error
            .downcast_ref::<DependencyBatchRejectedError>()
            .expect("a batch");

        let classified = classify(&error);
        let dedicated = dependency_batch_json_error(batch_error);

        assert_eq!(classified.error.code, dedicated.error.code);
        assert_eq!(classified.exit_code(), dedicated.exit_code());
        assert_eq!(classified.error.details, dedicated.error.details);
        assert_eq!(classified.error.suggestions, dedicated.error.suggestions);
        assert_suggestions(&classified, &[&["Remove", "cycle"], &["graph show"]]);
        assert_eq!(details_of(&classified)["from"], issue_id(1).to_string());
        assert_eq!(details_of(&classified)["to"], "loop");
    }

    #[test]
    fn test_classify_uncommitted_profile_targets_names_paths_and_resolution() {
        let error: anyhow::Error = ProfileApplyError::UncommittedTargets {
            paths: vec!["docs/a.md".to_owned()],
        }
        .into();

        let json = classify(&error);

        assert_eq!(json.error.code, ErrorCode::ProfileUncommittedTargets);
        let details = details_of(&json);
        assert_eq!(details["paths"][0], "docs/a.md");
        assert_eq!(
            details["resolution"],
            crate::commands::UNCOMMITTED_TARGETS_RESOLUTION
        );
        assert!(json.error.suggestions.is_empty());
    }

    #[test]
    fn test_classify_profile_target_conflicts_attach_the_conflict_list() {
        let error: anyhow::Error = crate::repository_state::RepositoryStateError::from(
            crate::repository_state::ProfileTargetConflictsError::new(vec![
                crate::repository_state::ProfileTargetConflictEntry {
                    owner: "candidate".try_into().expect("a canonical profile id"),
                    subject: crate::repository_state::ProfileTargetSubject::File {
                        path: crate::repository_state::VirtualPath::data("issues/a.json").unwrap(),
                    },
                    conflict: crate::repository_state::ProfileTargetConflict::Contested {
                        owners: vec![ProfileId::try_from("candidate").unwrap()],
                    },
                },
            ]),
        )
        .into();

        let json = classify(&error);

        assert_eq!(json.error.code, ErrorCode::ProfileConflict);
        assert_eq!(
            details_of(&json)["conflicts"].as_array().map(Vec::len),
            Some(1)
        );
        let conflict = &details_of(&json)["conflicts"][0];
        assert_eq!(conflict["owner"], "candidate");
        assert_eq!(conflict["subject"]["path"], ".jit/issues/a.json");
        assert_eq!(conflict["conflict"]["kind"], "contested");
        assert!(json.error.suggestions.is_empty());
    }

    #[test]
    fn test_classify_missing_profile_dependency_suggests_placing_the_package() {
        let missing: Vec<anyhow::Error> = vec![
            ProfileNotFoundError {
                profile: ProfileId::try_from("base").unwrap(),
            }
            .into(),
            ProfileResolutionError::UnresolvableDependency {
                package: "workflow".to_owned(),
                dependency: "base".to_owned(),
                cause: anyhow::anyhow!("absent"),
            }
            .into(),
            crate::profile::ProfileGraphError::MissingDependency {
                package: "workflow".to_owned(),
                dependency: "base".to_owned(),
            }
            .into(),
        ];

        for error in missing {
            let json = classify(&error);

            assert_eq!(json.error.code, ErrorCode::ProfileError, "{error}");
            assert_suggestions(&json, &[&["Place", "package", "beside"]]);
        }
    }

    #[test]
    fn test_classify_search_error_picks_search_code_and_installs_hint_only_for_missing_ripgrep() {
        use crate::search::SearchError;

        let missing = classify(&SearchError::RipgrepNotInstalled.into());
        let failed = classify(&SearchError::Failed("boom".to_owned()).into());

        assert_eq!(missing.error.code, ErrorCode::RipgrepNotFound);
        assert_suggestions(&missing, &[&["Install", "ripgrep"]]);
        assert_eq!(failed.error.code, ErrorCode::SearchFailed);
        assert!(failed.error.suggestions.is_empty());
    }

    #[test]
    fn test_classify_item_not_found_has_its_own_code_without_guidance() {
        let error: anyhow::Error = ItemNotFoundError::new("no such item").into();

        let json = classify(&error);

        assert_eq!(json.error.code, ErrorCode::ItemNotFound);
        assert!(json.error.details.is_none());
        assert!(json.error.suggestions.is_empty());
    }
}

#[cfg(test)]
mod path_read_classification_tests {
    //! Each [`PathReadError`] variant classifies by what it describes: absent
    //! content is a not-found, an unusable path is an invalid argument, and a
    //! wrapped cause classifies as itself.

    use super::classify;
    use crate::output::{ErrorCode, ExitCode};
    use crate::storage::PathReadError;

    fn code_of(error: PathReadError) -> ErrorCode {
        classify(&error.into()).error.code
    }

    #[test]
    fn test_classify_path_read_not_found_is_document_not_found_in_not_found_class() {
        let code = code_of(PathReadError::NotFound("docs/a.md".into()));
        assert_eq!(code, ErrorCode::DocumentNotFound);
        assert_eq!(code.exit_code(), ExitCode::NotFound);
    }

    #[test]
    fn test_classify_path_read_commit_not_found_is_document_not_found_in_not_found_class() {
        let code = code_of(PathReadError::CommitNotFound("deadbeef".into()));
        assert_eq!(code, ErrorCode::DocumentNotFound);
        assert_eq!(code.exit_code(), ExitCode::NotFound);
    }

    #[test]
    fn test_classify_path_read_invalid_path_is_invalid_argument() {
        assert_eq!(
            code_of(PathReadError::InvalidPath("../x".into())),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn test_classify_path_read_outside_repo_root_is_invalid_argument() {
        assert_eq!(
            code_of(PathReadError::OutsideRepoRoot("link".into())),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn test_classify_path_read_other_classifies_by_inner_error() {
        let inner = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let expected = classify(&anyhow::Error::from(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        )))
        .error
        .code;
        assert_eq!(code_of(PathReadError::Other(inner.into())), expected);
        assert_eq!(expected, ErrorCode::PermissionDenied);
    }
}

#[cfg(test)]
mod malformed_identity_classification_tests {
    //! A malformed gate key or gate-run id is an invalid argument.

    use super::classify;
    use crate::domain::identity::{GateKey, GateRunId};
    use crate::output::ErrorCode;

    #[test]
    fn test_classify_malformed_gate_run_id_is_invalid_argument() {
        let error = "not-a-run-id".parse::<GateRunId>().unwrap_err();
        assert_eq!(
            classify(&error.into()).error.code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn test_classify_malformed_gate_key_is_invalid_argument() {
        let error = "Bad_Key".parse::<GateKey>().unwrap_err();
        assert_eq!(
            classify(&error.into()).error.code,
            ErrorCode::InvalidArgument
        );
    }
    #[test]
    fn test_with_read_session_reports_an_unappliable_leftover_record_as_recovery_failed() {
        use crate::commands::with_read_session;
        use crate::storage::repository_state_store::test_fixtures::{
            leftover_record, leftover_target, InterruptAtMoment, MemorySessionBackend,
            SessionRecordBackend,
        };
        use crate::storage::{MutationPoint, RedoPoint};

        let backend = MemorySessionBackend::seeded();
        backend.commit(&leftover_record());
        let interrupted = backend
            .0
            .with_mutation_interruption_view(InterruptAtMoment::new(MutationPoint::Redo(
                RedoPoint::BeforeApply,
            )));

        let error = with_read_session(&interrupted, &backend.layout(), "blocked", |session| {
            let image = session.capture(crate::repository_state::CaptureSpec::new(
                [leftover_target()],
                crate::repository_state::CaptureBudget {
                    max_listings: 0,
                    max_bytes: 1024 * 1024,
                    max_depth: 8,
                },
            )?)?;
            Ok(Some(
                image.file_bytes(&leftover_target())?.map(<[u8]>::to_vec),
            ))
        })
        .unwrap_err();

        assert!(error
            .downcast_ref::<crate::errors::RecoveryFailedError>()
            .is_some());
        assert!(error.to_string().contains("jit recover"), "{error}");
        assert_eq!(
            crate::output::failure::classify(&error).error.code,
            crate::output::ErrorCode::RecoveryFailed
        );
    }
}
