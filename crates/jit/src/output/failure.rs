//! Classification of propagated failures into the machine-readable envelope.
//!
//! [`classify`] is the one library classifier shared by every consumer: it maps
//! a typed failure to its registered [`ErrorCode`], whose
//! [`ErrorCode::exit_code`] is the process status.

use super::{ErrorCode, ExitCode, JsonError};

/// Classify a propagated failure by its typed cause.
///
/// The envelope carries the registered code and the error's message; its
/// [`JsonError::exit_code`] is that code's [`ErrorCode::exit_code`], so the
/// reported code and the process status cannot drift.
pub fn classify(error: &anyhow::Error) -> JsonError {
    JsonError::new(classify_code(error), error.to_string())
}

/// The registered code of a failure. Human message text is deliberately absent
/// from this cascade, and its order is semantic.
fn classify_code(error: &anyhow::Error) -> ErrorCode {
    // A rejected `jit dep add` batch (jit:c8518f2a) wraps every rejected edge's
    // own typed error; classify by the batch's dominant edge rather than this
    // wrapper itself, so a mixed batch still maps to the right exit code.
    if let Some(batch) = error.downcast_ref::<crate::errors::DependencyBatchRejectedError>() {
        return dependency_batch_error_code(batch);
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
    if error
        .downcast_ref::<crate::commands::GateNotRequiredError>()
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
    if error
        .downcast_ref::<crate::errors::TransitionBlockedError>()
        .is_some()
    {
        return ErrorCode::Blocked;
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
    if error
        .downcast_ref::<crate::profile::ProfileClaimError>()
        .is_some()
        || error
            .downcast_ref::<crate::commands::ProfileApplyError>()
            .is_some()
    {
        return ErrorCode::ProfileConflict;
    }
    // Which package bytes a profile command reads, or which packages an
    // application applies, could not be settled. Both are classified here,
    // ahead of the source-chain searches below, because a location that is
    // simply gone carries a NotFound I/O cause: taking that classification
    // would report a repository whose record outlived its package as a missing
    // repository or an absent profile, which is the reading the record exists
    // to rule out.
    if error
        .downcast_ref::<crate::commands::ProfileResolutionError>()
        .is_some()
        || error
            .downcast_ref::<crate::commands::ProfileDependencyError>()
            .is_some()
    {
        return ErrorCode::ProfileError;
    }

    // The repository-state derivation error is classified by an exhaustive match
    // over every current variant — no fallback arm — so a newly added variant forces
    // a deliberate classification here rather than silently taking the generic code.
    // Projection, managed-document, profile-target-conflict, ambiguous-ownership,
    // gate-registry-edit, and archive-execution failures are validation failures (4);
    // a layout rejection is an argument error (2); capture/parse producer failures and
    // the structural delta/overlay/initialization failures keep the
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
            | RepositoryStateError::Overlay(_) => ErrorCode::GenericError,
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
    // Claim/lease commands require git; running them outside a git repository is
    // an external-dependency failure (exit 10).
    if error
        .downcast_ref::<crate::errors::ClaimRequiresGitError>()
        .is_some()
    {
        return ErrorCode::ClaimRequiresGit;
    }

    // A repository whose on-disk format is newer than this binary supports is an
    // external-dependency failure (exit 10): the binary, not the repository, is
    // out of date, and the fix is upgrading jit — the same family as the git and
    // filesystem dependency failures, and deliberately NOT NotFound (3), which
    // would misread as "repository/resource missing".
    if error
        .downcast_ref::<crate::storage::RepositoryFormatTooNewError>()
        .is_some()
    {
        return ErrorCode::RepositoryFormatTooNew;
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
            RepositoryIndexError::UnsupportedVersion { .. } => ErrorCode::RepositoryFormatTooNew,
            RepositoryIndexError::DuplicateActive(_)
            | RepositoryIndexError::DuplicateDeleted(_)
            | RepositoryIndexError::ActiveDeletedOverlap(_) => ErrorCode::ValidationFailed,
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
    if let Some(graph_error) = error.downcast_ref::<crate::graph::GraphError>() {
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
            | crate::storage::PathReadError::CommitNotFound(_) => ErrorCode::RepositoryNotFound,
            crate::storage::PathReadError::InvalidPath(_) => ErrorCode::InvalidArgument,
            crate::storage::PathReadError::OutsideRepoRoot(_) => ErrorCode::GenericError,
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

/// Build the JSON error envelope for a failed claim/lease command.
///
/// A specific typed class always wins over the command fallback. This keeps
/// permission, external-I/O, no-Git, parsing, and lookup failures aligned with
/// the human path, which uses [`classify`]. Only a genuinely unclassified
/// failure retains the command-specific registered code.
pub fn claim_json_error(error: &anyhow::Error, fallback_code: ErrorCode) -> JsonError {
    let classified = classify_code(error);
    let code = if classified == ErrorCode::GenericError {
        fallback_code
    } else {
        classified
    };
    JsonError::new(code, error.to_string())
}

/// Preserve a `validate --fix` failure's complete cause chain and its exact
/// typed classification. The envelope's registered code owns the process
/// status; callers must not retain a parallel exit-code projection.
pub fn validate_fix_json_error(error: &anyhow::Error) -> JsonError {
    JsonError::new(classify_code(error), format!("{error:#}"))
}

/// Build the JSON error envelope for a failed profile command, attaching the
/// details and suggestions a profile failure carries.
pub fn profile_json_error(error: &anyhow::Error) -> anyhow::Result<JsonError> {
    use crate::repository_state::RepositoryStateError;

    if error
        .downcast_ref::<crate::errors::NotFoundError>()
        .is_some()
    {
        return Ok(command_json_error(error, ErrorCode::ProfileNotFound)
            .with_suggestion("Select a package with --profile DIR"));
    }
    if error
        .downcast_ref::<crate::profile::ProfileClaimError>()
        .is_some()
        || error
            .downcast_ref::<crate::commands::ProfileApplyError>()
            .is_some()
    {
        return Ok(command_json_error(error, ErrorCode::ProfileConflict));
    }
    // Stated rather than left to the fallback below, so this envelope and
    // [`classify`]'s classification of the same failure are read from
    // one decision.
    if error
        .downcast_ref::<crate::commands::ProfileResolutionError>()
        .is_some()
    {
        return Ok(command_json_error(error, ErrorCode::ProfileError));
    }
    if error
        .downcast_ref::<crate::commands::ProfileDependencyError>()
        .is_some()
    {
        return Ok(command_json_error(error, ErrorCode::ProfileError)
            .with_suggestion("Place the package it depends on beside it"));
    }
    // The single sanctioned downcast of the anyhow CLI transport to the typed
    // repository-state error: every refused profile decision — a target or a
    // declaration, raised directly or through initialization — is a conflict; an
    // unreadable or unparseable profile registry is a generic profile error. Every
    // other variant keeps the generic profile-error code.
    if let Some(state_error) = error.downcast_ref::<RepositoryStateError>() {
        if let RepositoryStateError::ProfileTargetConflicts(conflicts) = state_error {
            return Ok(command_json_error(error, ErrorCode::ProfileConflict)
                .with_details(serde_json::to_value(conflicts)?));
        }
        return Ok(command_json_error(error, ErrorCode::ProfileError));
    }
    Ok(command_json_error(error, ErrorCode::ProfileError))
}

#[cfg(test)]
mod exit_code_projection_tests {
    //! REQ-02: verify the projected command exit-code mappings against the
    //! runtime classifier they document. Each representative typed error is run
    //! through [`classify`] (the exact classifier the CLI dispatch
    //! uses) and the resulting code is confirmed to (a) equal the expected code
    //! and (b) be a code the schema's `command_exit_codes` projection documents.
    //! If the classifier ever reclassifies one of these conditions, or the
    //! projection stops documenting a code the classifier still emits, this test
    //! fails — so the projection cannot silently drift from runtime behavior.

    use super::{claim_json_error, classify, validate_fix_json_error};
    use crate::declarations::GateStage;
    use crate::domain::{GateRunResult, GateRunStatus};
    use crate::output::ErrorCode;
    use crate::schema::CommandSchema;

    /// Build a `gate evaluate` checker failure carrying `status`, so the
    /// `gate evaluate` / `gate evaluate-all` exception rows (checker verdict `4`,
    /// runner error `10`) are pinned to the real classifier: `classify`
    /// splits `GatePassFailed` on this status.
    fn gate_pass_failed(status: GateRunStatus) -> anyhow::Error {
        let result = GateRunResult {
            schema_version: 1,
            run_id: "run-1".to_string(),
            gate_key: "tests".to_string(),
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
            gate_key: "tests".to_string(),
            status,
            exit_code: Some(1),
            result,
            warnings: Vec::new(),
        }
        .into()
    }

    #[test]
    fn test_claim_json_error_keeps_registered_fallback_for_unclassified_failure() {
        let error = anyhow::anyhow!("claim-specific failure without a typed cause");
        let json = claim_json_error(&error, ErrorCode::ClaimRenewError);

        assert_eq!(json.error.code, ErrorCode::ClaimRenewError);
        assert_eq!(json.exit_code(), ErrorCode::ClaimRenewError.exit_code());
    }

    /// Representative typed errors, each paired with the exit code the classifier
    /// must produce for it and the `command_exit_codes` **row** that pair pins
    /// (`"*"` for the universal rows, which any command reaches through the shared
    /// classifier).
    ///
    /// Naming the row — not just the code — is what binds each documented
    /// command-family mapping to runtime behavior: a row whose condition is raised
    /// by a typed error is verified here against the classifier that command's
    /// dispatch actually runs. The rows whose conditions are raised by
    /// crate-private errors (`TransitionBlockedError`) or by direct
    /// `std::process::exit` sites are bound end-to-end instead, by the subprocess
    /// tests in `tests/command_exit_code_projection_tests.rs`; the guard there
    /// asserts every row is covered by one binding or the other.
    fn classifier_cases() -> Vec<(anyhow::Error, i32, &'static str)> {
        use std::io::{Error as IoError, ErrorKind};
        vec![
            // Gate-evaluation exception rows route through `classify`:
            // a checker that failed (`Failed`) is `4`; one that could not run
            // (`Error`) is `10`.
            (
                gate_pass_failed(GateRunStatus::Failed),
                4,
                "gate evaluate, gate evaluate-all, gate evaluate-many",
            ),
            (
                gate_pass_failed(GateRunStatus::Error),
                10,
                "gate evaluate, gate evaluate-all, gate evaluate-many",
            ),
            // Universal rows: raised from shared paths, reachable from any command.
            (anyhow::anyhow!("untyped failure"), 1, "*"),
            (
                crate::errors::InvalidArgumentError::new("bad arg").into(),
                2,
                "*",
            ),
            (
                crate::storage::AmbiguousIdError::issue(
                    "aaaa",
                    ["aaaa1111".to_string(), "aaaa2222".to_string()],
                )
                .into(),
                2,
                "*",
            ),
            (
                crate::domain::identity::InvalidIdPrefixError::new("ab").into(),
                2,
                "*",
            ),
            (
                crate::commands::GateNotRequiredError {
                    issue_id: crate::test_utils::issue_id(85),
                    gate_key: "tests".to_string(),
                }
                .into(),
                2,
                "*",
            ),
            (
                crate::storage::IssueNotFoundError::new("abc123").into(),
                3,
                "*",
            ),
            (IoError::new(ErrorKind::NotFound, "missing").into(), 3, "*"),
            // `validate_for_write` is the shared write-validation path, so every
            // command that writes an issue — and only those — can reject a
            // blocking rule finding with this type.
            (
                crate::errors::ValidationFailedError::new("bad").into(),
                4,
                "any command that writes an issue",
            ),
            (
                IoError::new(ErrorKind::PermissionDenied, "denied").into(),
                5,
                "*",
            ),
            (
                anyhow::Error::new(
                    serde_json::from_str::<serde_json::Value>("{")
                        .expect_err("fixture JSON is malformed"),
                )
                .context("outer command context"),
                1,
                "*",
            ),
            (
                crate::storage::RepositoryFormatTooNewError::new(9999, 1).into(),
                10,
                "*",
            ),
            // Command-family rows, each pinned by the error that command raises.
            (crate::graph::GraphError::CycleDetected.into(), 4, "dep add"),
            (
                crate::errors::RedundantDependencyError::new(
                    ("aaaa1111".to_string(), "bbbb2222".to_string()),
                    vec![("aaaa1111".to_string(), "cccc3333".to_string())],
                )
                .into(),
                4,
                "dep add",
            ),
            (
                crate::storage::GateAlreadyExistsError::new("tests").into(),
                6,
                "gate define",
            ),
            (
                crate::errors::AlreadyExistsError::new("Output path already exists: taken").into(),
                6,
                "snapshot export",
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
                "issue batch-create",
            ),
            (
                crate::errors::ClaimRequiresGitError::new(
                    crate::errors::GitRequirementGap::NoRepository,
                )
                .into(),
                10,
                "claim",
            ),
        ]
    }

    #[test]
    fn test_classify_produces_documented_exit_codes() {
        for (error, expected, _row) in classifier_cases() {
            assert_eq!(
                classify(&error).exit_code().code(),
                expected,
                "classifier produced the wrong code for `{error}`"
            );
        }
    }

    #[test]
    fn test_error_code_classifier_is_the_exit_classifier_authority() {
        for (index, (error, _expected, _row)) in classifier_cases().into_iter().enumerate() {
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
    fn test_validate_fix_json_error_uses_typed_code_as_status_authority() {
        use std::io::{Error as IoError, ErrorKind};

        let cases = [
            (
                crate::errors::ValidationFailedError::new("invalid repository").into(),
                ErrorCode::ValidationFailed,
            ),
            (
                crate::errors::InvalidArgumentError::new("bad repair option").into(),
                ErrorCode::InvalidArgument,
            ),
            (
                crate::errors::AlreadyExistsError::new("occupied repair target").into(),
                ErrorCode::AlreadyExists,
            ),
            (
                IoError::new(ErrorKind::PermissionDenied, "denied").into(),
                ErrorCode::PermissionDenied,
            ),
            (
                crate::storage::RepositoryFormatTooNewError::new(9999, 1).into(),
                ErrorCode::RepositoryFormatTooNew,
            ),
            (
                anyhow::anyhow!("untyped repair failure"),
                ErrorCode::GenericError,
            ),
            (
                IoError::new(ErrorKind::BrokenPipe, "closed repair output").into(),
                ErrorCode::IoError,
            ),
        ];

        for (error, expected_code) in cases {
            let envelope = validate_fix_json_error(&error);
            assert_eq!(envelope.error.code, expected_code);
            assert_eq!(envelope.exit_code(), expected_code.exit_code());
            assert_eq!(envelope.exit_code(), classify(&error).exit_code());
        }
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

    /// Every classifier case must land on the exact projected row it documents —
    /// same command family, same code. Code-level membership is not enough: a row
    /// that names the wrong command family, or a command family whose error is
    /// reclassified, must fail here.
    #[test]
    fn test_command_exit_codes_documents_every_classified_row() {
        let schema = CommandSchema::generate();
        for (error, _expected, row) in classifier_cases() {
            let actual = classify(&error).exit_code().code();
            assert!(
                schema
                    .command_exit_codes
                    .iter()
                    .any(|c| c.command == row && c.code == Some(actual)),
                "classifier emits code {actual} for `{error}`, but the \
                 command_exit_codes projection has no `{row}` row documenting it"
            );
        }
    }
}

#[cfg(test)]
mod repository_state_classifier_tests {
    //! REQ-01/REQ-02: pin the exhaustive `RepositoryStateError` exit-code mapping
    //! and the profile `--json` variant classification to their real classifiers
    //! ([`classify`] and [`profile_json_error`]). Each representative
    //! variant is asserted so a reclassification (or a mis-mapped newly added
    //! variant) fails here rather than silently changing a command's exit status.

    use super::{classify, profile_json_error};
    use crate::repository_state::{
        AmbiguousOwnershipError, ArchiveExecutionError, Contribution, ContributionConflictOwner,
        GateRegistryEditError, InitializationError, ManagedDocumentError, ProducerError,
        ProfilePackageId, ProfileTargetConflict, ProfileTargetConflictEntry,
        ProfileTargetConflictsError, ProfileTargetSubject, ProjectionError, RepositoryLayoutError,
        RepositoryStateError, ScalarTarget, VirtualPath,
    };

    fn path() -> VirtualPath {
        VirtualPath::data("issues/abc123.json").unwrap()
    }

    /// A refusal carrying a contested file target and a contested declaration,
    /// which is what a selection whose packages define both differently produces.
    fn target_conflicts() -> ProfileTargetConflictsError {
        let contested = || ProfileTargetConflict::Contested {
            owners: vec![
                ContributionConflictOwner::Package(ProfilePackageId::new("candidate")),
                ContributionConflictOwner::Package(ProfilePackageId::new("occupant")),
            ],
        };
        ProfileTargetConflictsError::new(vec![
            ProfileTargetConflictEntry {
                owner: "candidate".try_into().expect("a canonical profile id"),
                subject: ProfileTargetSubject::File(path()),
                conflict: contested(),
            },
            ProfileTargetConflictEntry {
                owner: "candidate".try_into().expect("a canonical profile id"),
                subject: ProfileTargetSubject::Contribution(
                    Contribution::Scalar {
                        target: ScalarTarget::ValidationDefaultType,
                        value: "task".to_string(),
                    }
                    .semantic_identity(),
                ),
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
    fn test_profile_json_error_classifies_repository_state_variants() {
        // Every refused profile decision — a target or a declaration — is a
        // profile conflict (validation exit 4); an unreadable registry and an
        // unrelated variant keep the generic profile-error code (exit 1).
        let conflict_cases: Vec<RepositoryStateError> = vec![target_conflicts().into()];
        for state_error in conflict_cases {
            let error = anyhow::Error::new(state_error);
            let json = profile_json_error(&error).expect("a conflict renders a JSON error");
            assert_eq!(json.error.code, crate::output::ErrorCode::ProfileConflict);
            assert_eq!(json.exit_code().code(), 4);
        }

        let error_cases: Vec<RepositoryStateError> = vec![
            ProducerError::ProfileRegistryNotFile { target: "t".into() }.into(),
            RepositoryLayoutError::LexicalEscape("p".into()).into(),
        ];
        for state_error in error_cases {
            let error = anyhow::Error::new(state_error);
            let json = profile_json_error(&error).expect("a profile error renders a JSON error");
            assert_eq!(json.error.code, crate::output::ErrorCode::ProfileError);
            assert_eq!(json.exit_code().code(), 1);
        }
    }

    /// REQ-04: the conflict code, exit status, and rendered human message a
    /// refused publication carries are unaffected by attaching structured
    /// details beside them — the message stays exactly the
    /// sentence [`RepositoryStateError`]'s `Display` renders, byte for byte,
    /// carrying every fact its structured form carries so a reader of either
    /// acts on the same evidence.
    #[test]
    fn test_profile_json_error_conflict_keeps_its_code_exit_and_message_stable() {
        let state_error: RepositoryStateError = target_conflicts().into();
        let error = anyhow::Error::new(state_error);
        let rendered = error.to_string();

        let json = profile_json_error(&error).expect("a conflict renders a JSON error");

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
    fn test_profile_json_error_conflict_attaches_structured_details_without_remedies() {
        let state_error: RepositoryStateError = target_conflicts().into();
        let error = anyhow::Error::new(state_error);

        let json = profile_json_error(&error).expect("a conflict renders a JSON error");

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
    fn test_profile_json_error_maps_not_found_before_variant_match() {
        let error = anyhow::Error::new(crate::errors::NotFoundError::new("no such profile"));
        let json = profile_json_error(&error).expect("a not-found error renders a JSON error");
        assert_eq!(json.error.code, crate::output::ErrorCode::ProfileNotFound);
        assert_eq!(json.exit_code().code(), 3);
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
