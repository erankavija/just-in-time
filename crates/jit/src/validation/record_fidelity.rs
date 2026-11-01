//! Whether stored records decode and reproduce through their writer's serializer.
//!
//! A record that decodes but does not reproduce loses data at its next rewrite.

use crate::domain::event_log::parse_known_events;
use crate::domain::identity::GateRunId;
use crate::domain::{GateRunResult, Issue};
use crate::repository_state::{
    canonical_json, captured_gate_run_ids, captured_issue_ids, serialize_event, serialize_gate_run,
    serialize_issue, RepositoryImage, RepositoryIndex, VirtualPath, SUPPORTED_INDEX_SCHEMA_VERSION,
};
use anyhow::Result;
use serde_json::Value;

/// Tags earlier versions wrote, which the log keeps and readers skip.
const RETIRED_EVENT_TAGS: [&str; 3] = [
    "document_archived",
    "lifecycle_timestamps_backfilled",
    "profile_applied",
];

/// One message per stored record that does not reproduce. What fails to parse
/// or is absent from `image` is the records pass's finding.
pub(crate) fn record_drift(
    image: &RepositoryImage,
    gate_runs: &RepositoryImage,
) -> Result<Vec<String>> {
    let report = |path: &VirtualPath, check: Result<Option<String>>| {
        check
            .unwrap_or_else(|error| Some(error.to_string()))
            .map(|reason| format!("{}: {reason}", path.repository_relative()))
    };
    let index = image.file_bytes(&VirtualPath::INDEX)?;
    let issues = captured_issue_ids(image)
        .unwrap_or_default()
        .iter()
        .map(|id| {
            let path = VirtualPath::issue_record(id);
            Ok(image
                .file_bytes(&path)?
                .and_then(|bytes| report(&path, issue_drift(bytes))))
        })
        .collect::<Result<Vec<_>>>()?;
    let runs = captured_gate_run_ids(gate_runs)
        .unwrap_or_default()
        .iter()
        .map(|id| {
            let path = VirtualPath::gate_run_record(id);
            let check = match gate_runs.file_bytes(&path)? {
                Some(bytes) => gate_run_drift(id, bytes),
                None => Ok(Some("missing".to_string())),
            };
            Ok(report(&path, check))
        })
        .collect::<Result<Vec<_>>>()?;
    let events = image
        .file_bytes(&VirtualPath::EVENTS)?
        .map_or_else(Vec::new, event_log_drift);
    Ok(issues
        .into_iter()
        .chain(runs)
        .chain([index.and_then(|bytes| report(&VirtualPath::INDEX, index_drift(bytes)))])
        .flatten()
        .chain(
            events
                .into_iter()
                .map(|reason| format!("{}: {reason}", VirtualPath::EVENTS.repository_relative())),
        )
        .collect())
}

fn index_drift(bytes: &[u8]) -> Result<Option<String>> {
    let index = RepositoryIndex::parse(bytes)?;
    // An older format is read as-is until its explicit rewrite.
    let current = index.schema_version == SUPPORTED_INDEX_SCHEMA_VERSION;
    let written = index.to_pretty_bytes()?;
    Ok((current && written != bytes).then(|| "stored bytes are not canonical".to_string()))
}

/// Canonical rather than byte-exact: key order is not part of an issue record.
fn issue_drift(bytes: &[u8]) -> Result<Option<String>> {
    let raw: Value = serde_json::from_slice(bytes)?;
    let issue: Issue = serde_json::from_value(raw.clone())?;
    Ok((serialize_issue(&issue)? != canonical_json(&raw, true)?)
        .then(|| "re-serializing changes the record".to_string()))
}

/// A record carrying the current key set reproduces byte for byte; one written
/// under an earlier schema keeps its identity fields.
fn gate_run_drift(id: &GateRunId, bytes: &[u8]) -> Result<Option<String>> {
    let run: GateRunResult = serde_json::from_slice(bytes)?;
    if run.run_id != *id {
        return Ok(Some(format!("holds run id '{}'", run.run_id)));
    }
    let written = serialize_gate_run(&run)?;
    let raw: Value = serde_json::from_slice(bytes)?;
    let decoded: Value = serde_json::from_slice(&written)?;
    let keys = |value: &Value| {
        value
            .as_object()
            .map(|object| object.keys().cloned().collect::<Vec<_>>())
    };
    let lost = ["issue_id", "run_id", "gate_key"]
        .into_iter()
        .chain(raw["origin"].is_object().then_some("origin"))
        .find(|field| decoded[field] != raw[field]);
    Ok(match lost {
        Some(field) => Some(format!("re-serializing changes '{field}'")),
        None => (keys(&decoded) == keys(&raw) && written != bytes)
            .then(|| "stored bytes are not canonical".to_string()),
    })
}

/// Every current-vocabulary line reproduces its bytes; every other line carries
/// a retired tag. A line alone that fails to parse is the log's certified torn
/// tail, since the whole log parsed.
fn event_log_drift(bytes: &[u8]) -> Vec<String> {
    let Some(log) = std::str::from_utf8(bytes)
        .ok()
        .filter(|log| parse_known_events(log).is_ok())
    else {
        return Vec::new();
    };
    log.lines()
        .filter_map(|line| match parse_known_events(line).ok()?.first() {
            Some(event) => (serialize_event(event).ok()?.as_slice() != line.as_bytes())
                .then(|| format!("line does not reproduce: {line}")),
            None if line.trim().is_empty() => None,
            None => {
                let value: Value = serde_json::from_str(line).ok()?;
                let tag = value["type"].as_str()?;
                (!RETIRED_EVENT_TAGS.contains(&tag)).then(|| format!("unknown event tag '{tag}'"))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::GateStage;
    use crate::domain::event_catalog::EventTag;
    use crate::domain::GateStatus;

    fn run_bytes(id: &GateRunId) -> Vec<u8> {
        serialize_gate_run(&GateRunResult::attestation(
            id,
            &crate::test_utils::issue_id(7),
            &"tests".parse().unwrap(),
            GateStage::Postcheck,
            GateStatus::Passed,
            None,
        ))
        .unwrap()
    }

    fn event_line(tag: EventTag) -> String {
        String::from_utf8(serialize_event(&tag.sample()).unwrap()).unwrap()
    }

    #[test]
    fn test_index_drift_reports_only_a_non_canonical_form() {
        let canonical = crate::repository_state::fresh_index_bytes().unwrap();
        assert_eq!(index_drift(&canonical).unwrap(), None);

        let spaced = [canonical.as_slice(), b"\n"].concat();
        assert!(index_drift(&spaced).unwrap().is_some());
    }

    #[test]
    fn test_issue_drift_ignores_key_order_and_reports_changed_content() {
        let issue = crate::test_utils::fixture_issue("Title".into(), "Body".into());
        let mut raw: Value = serde_json::from_slice(&serialize_issue(&issue).unwrap()).unwrap();
        let reversed = raw
            .as_object()
            .unwrap()
            .iter()
            .rev()
            .map(|(key, value)| format!("{key:?}:{value}"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            issue_drift(format!("{{{reversed}}}").as_bytes()).unwrap(),
            None
        );

        raw["field_no_reader_knows"] = Value::Bool(true);
        assert!(issue_drift(&serde_json::to_vec(&raw).unwrap())
            .unwrap()
            .is_some());
    }

    #[test]
    fn test_gate_run_drift_reports_a_record_stored_under_another_run_id() {
        let holding = crate::test_utils::gate_run_id(2);
        let reason = gate_run_drift(&crate::test_utils::gate_run_id(1), &run_bytes(&holding));
        assert!(reason.unwrap().unwrap().contains(holding.as_str()));
    }

    #[test]
    fn test_gate_run_drift_exempts_an_earlier_schema_from_bytes_but_not_identity() {
        let id = crate::test_utils::gate_run_id(1);
        let mut raw: Value = serde_json::from_slice(&run_bytes(&id)).unwrap();
        raw["retired_key"] = Value::Bool(true);
        assert_eq!(
            gate_run_drift(&id, &serde_json::to_vec(&raw).unwrap()).unwrap(),
            None
        );

        raw["origin"] = serde_json::json!({ "kind": "not-a-recorded-origin" });
        let lost = gate_run_drift(&id, &serde_json::to_vec(&raw).unwrap());
        assert!(!matches!(lost, Ok(None)));
    }

    #[test]
    fn test_gate_run_drift_holds_current_keys_to_canonical_bytes() {
        let id = crate::test_utils::gate_run_id(1);
        let canonical = run_bytes(&id);
        assert_eq!(gate_run_drift(&id, &canonical).unwrap(), None);

        let raw: Value = serde_json::from_slice(&canonical).unwrap();
        assert!(gate_run_drift(&id, &serde_json::to_vec(&raw).unwrap())
            .unwrap()
            .is_some());
    }

    #[test]
    fn test_event_log_drift_is_empty_over_every_cataloged_tag_and_retired_tags() {
        let log = EventTag::ALL
            .iter()
            .map(|tag| event_line(*tag))
            .chain(
                RETIRED_EVENT_TAGS
                    .iter()
                    .map(|tag| format!("{{\"type\":\"{tag}\"}}")),
            )
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(event_log_drift(log.as_bytes()), Vec::<String>::new());
    }

    #[test]
    fn test_event_log_drift_is_empty_over_a_profile_applied_line() {
        let line = r#"{"id":"39dc3e16-4985-440d-a1f3-ca63416ec1cd","profile_id":"jit-default","timestamp":"2026-08-05T12:01:27.128206959Z","type":"profile_applied"}"#;
        assert_eq!(event_log_drift(line.as_bytes()), Vec::<String>::new());
    }

    #[test]
    fn test_event_log_drift_reports_an_unknown_tag_and_a_line_that_does_not_reproduce() {
        let line = event_line(EventTag::ALL[0]);
        let spaced = line.replacen(':', ": ", 1);
        let log = format!("{line}\n{spaced}\n{{\"type\":\"never_written\"}}\n");

        let drift = event_log_drift(log.as_bytes());

        assert_eq!(drift.len(), 2, "{drift:?}");
        assert!(drift[0].contains(&spaced) && drift[1].contains("never_written"));
    }
}
