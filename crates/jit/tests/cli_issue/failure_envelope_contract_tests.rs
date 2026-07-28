//! Canonical stable external contract for machine-readable failure envelopes.
//!
//! This is the one suite that inventories the envelope fields literally. Every
//! post-dispatch failure probe recorded in `failure_lever_registry.toml` must
//! write exactly this contract to its payload stream: a top-level `error`
//! object carrying a registered code and a non-empty message, with optional
//! object-valued details and an optional array of non-empty string suggestions.
//! The code, rather than copied diagnostic prose, determines the process exit
//! status. Command-specific tests may assert their own semantics, but they do
//! not duplicate this stable external field inventory.

use super::failure_lever_registry::{failure_lever_registry, FailureLever};
use super::failure_probe_fixture::{drive_failure_lever, ForcedFailure, RecordedFailure};
use jit::output::ErrorCode;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::str::FromStr;

fn field_set(object: &Map<String, Value>) -> BTreeSet<&str> {
    object.keys().map(String::as_str).collect()
}

fn failure_context(failure: &ForcedFailure) -> String {
    format!(
        "{} {:?}: status={:?}, stdout={}, stderr={}",
        failure.path,
        failure.argv,
        failure.status.code(),
        String::from_utf8_lossy(&failure.stdout),
        String::from_utf8_lossy(&failure.stderr),
    )
}

fn assert_canonical_failure_envelope(failure: &ForcedFailure) {
    let context = failure_context(failure);
    assert!(
        !failure.status.success(),
        "a recorded failure must exit non-zero: {context}"
    );

    let envelope: Value = serde_json::from_slice(&failure.stdout).unwrap_or_else(|error| {
        panic!("failure payload must be a JSON envelope: {error}; {context}")
    });
    let envelope = envelope
        .as_object()
        .unwrap_or_else(|| panic!("failure payload must be a JSON object: {context}"));
    assert_eq!(
        field_set(envelope),
        BTreeSet::from(["error"]),
        "failure payload must contain only the stable top-level envelope field: {context}"
    );

    let error = envelope["error"].as_object().unwrap_or_else(|| {
        panic!("failure payload must carry an object-valued `error`: {context}")
    });
    let required = BTreeSet::from(["code", "message"]);
    let allowed = BTreeSet::from(["code", "message", "details", "suggestions"]);
    let fields = field_set(error);
    assert!(
        required.is_subset(&fields),
        "error object is missing a required stable field: {context}"
    );
    assert!(
        fields.is_subset(&allowed),
        "error object contains a field outside the stable external contract: {context}"
    );

    let code_text = error["code"]
        .as_str()
        .unwrap_or_else(|| panic!("error.code must be a string: {context}"));
    let code = ErrorCode::from_str(code_text)
        .unwrap_or_else(|error| panic!("error.code must be registered: {error}; {context}"));
    assert_eq!(
        failure.status.code(),
        Some(code.exit_code().code()),
        "the process status must be determined by the registered error classification: {context}"
    );
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| !message.trim().is_empty()),
        "error.message must be a non-empty string: {context}"
    );

    if let Some(details) = error.get("details") {
        assert!(
            details.is_object(),
            "error.details must be an object when present: {context}"
        );
    }
    if let Some(suggestions) = error.get("suggestions") {
        let suggestions = suggestions.as_array().unwrap_or_else(|| {
            panic!("error.suggestions must be an array when present: {context}")
        });
        assert!(
            suggestions.iter().all(|suggestion| suggestion
                .as_str()
                .is_some_and(|suggestion| !suggestion.trim().is_empty())),
            "error.suggestions must contain only non-empty strings: {context}"
        );
    }
}

#[test]
fn test_recorded_failure_arms_emit_the_canonical_error_envelope() {
    let mut probed_arms = 0;

    failure_lever_registry()
        .arms
        .iter()
        .for_each(|lever| match lever {
            FailureLever::Invocation(_) => match drive_failure_lever(lever) {
                RecordedFailure::Invoked(failure) => {
                    probed_arms += 1;
                    assert_canonical_failure_envelope(&failure);
                }
                RecordedFailure::Exempt { .. } => {
                    panic!("an invoked registry arm must remain a runtime probe")
                }
            },
            FailureLever::Exemption(exemption) => assert!(
                !exemption.exemption_reason.trim().is_empty(),
                "an unprobed arm must retain its declared exemption reason: {}",
                exemption.path
            ),
        });

    assert!(
        probed_arms > 0,
        "the canonical failure-envelope suite must exercise recorded command arms"
    );
}
