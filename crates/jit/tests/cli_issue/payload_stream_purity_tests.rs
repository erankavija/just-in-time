//! Payload-stream purity for every recorded machine-readable command failure.

use super::failure_lever_registry::failure_lever_registry;
use super::failure_probe_fixture::{drive_failure_lever, RecordedFailure};
use serde_json::Value;

fn parse_single_json_document(payload: &[u8]) -> Result<Value, serde_json::Error> {
    serde_json::from_slice(payload)
}

#[test]
fn test_machine_readable_failures_emit_one_json_document_on_payload_stream() {
    failure_lever_registry()
        .arms
        .iter()
        .for_each(|lever| match drive_failure_lever(lever) {
            RecordedFailure::Invoked(failure) => {
                assert!(
                    !failure.status.success(),
                    "recorded failure {} unexpectedly succeeded: stdout={} stderr={}",
                    failure.path,
                    String::from_utf8_lossy(&failure.stdout),
                    String::from_utf8_lossy(&failure.stderr)
                );
                let envelope = parse_single_json_document(&failure.stdout).unwrap_or_else(|error| {
                    panic!(
                        "recorded failure {} emitted an impure payload stream for {:?}: {error}; stdout={} stderr={}",
                        failure.path,
                        failure.argv,
                        String::from_utf8_lossy(&failure.stdout),
                        String::from_utf8_lossy(&failure.stderr)
                    )
                });
                assert!(
                    envelope.is_object(),
                    "recorded failure {} must emit a JSON envelope object",
                    failure.path
                );
                // The fixture deliberately retains the diagnostic stream. It is
                // not part of the payload assertion: handler-owned failures may
                // choose their own human diagnostic policy, while this guard
                // protects the parser-facing stream from that prose.
                let _diagnostic_stream = &failure.stderr;
            }
            RecordedFailure::Exempt { path, reason } => {
                assert!(
                    !reason.is_empty(),
                    "source-only arm {path} must retain its recorded exemption reason"
                );
            }
        });
}

#[test]
fn test_payload_stream_purity_rejects_non_json_byte() {
    let payload = br#"{"error":{}}!"#;
    assert!(parse_single_json_document(&payload[..payload.len() - 1]).is_ok());
    assert!(parse_single_json_document(payload).is_err());
}

#[test]
fn test_payload_stream_purity_rejects_prefix_before_envelope() {
    assert!(parse_single_json_document(b"progress\n{\"error\":{}}").is_err());
}
