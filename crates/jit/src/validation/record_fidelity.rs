//! Whether stored records decode and reproduce through their writer's serializer.
//!
//! A record that decodes but does not reproduce loses data at its next rewrite.
//! The event log is judged from its whole bytes: an unparseable log is one
//! finding naming the line, and a parseable one reports each line that does not
//! reproduce and each event id on more than one line.

use crate::domain::event_log::parse_known_events;
use crate::domain::identity::GateRunId;
use crate::domain::{Event, GateRunResult, GateVerdictRecord, Issue};
use crate::repository_state::{
    canonical_json, captured_gate_run_ids, captured_gate_verdict_records, captured_issue_ids,
    serialize_event, serialize_gate_run, serialize_gate_verdict, serialize_issue, RepositoryImage,
    RepositoryIndex, VirtualPath,
};
use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;

/// One message per stored record that does not decode or reproduce, over
/// `image`'s records, `gate_runs`' runs and verdict records, and the event
/// log's whole `events`.
pub(crate) fn record_drift(
    image: &RepositoryImage,
    gate_runs: &RepositoryImage,
    events: Option<&[u8]>,
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
    let verdicts = captured_gate_verdict_records(gate_runs)
        .iter()
        .map(|path| {
            Ok(gate_runs
                .file_bytes(path)?
                .and_then(|bytes| report(path, verdict_drift(bytes))))
        })
        .collect::<Result<Vec<_>>>()?;
    let events = events.map_or_else(Vec::new, event_log_drift);
    Ok(issues
        .into_iter()
        .chain(runs)
        .chain(verdicts)
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
    let written = RepositoryIndex::parse(bytes)?.to_pretty_bytes()?;
    Ok((written != bytes).then(|| "stored bytes are not canonical".to_string()))
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

fn verdict_drift(bytes: &[u8]) -> Result<Option<String>> {
    let record: GateVerdictRecord = serde_json::from_slice(bytes)?;
    Ok((serialize_gate_verdict(&record)? != bytes)
        .then(|| "stored bytes are not canonical".to_string()))
}

/// Every committed line parses and reproduces its bytes, and every event id
/// names one line.
fn event_log_drift(bytes: &[u8]) -> Vec<String> {
    let events = match parse_known_events(bytes) {
        Ok(events) => events,
        Err(error) => return vec![error.to_string()],
    };
    let ids = events
        .iter()
        .map(Event::id)
        .fold(BTreeMap::<&str, usize>::new(), |mut ids, id| {
            *ids.entry(id).or_default() += 1;
            ids
        });
    bytes
        .split_inclusive(|byte| *byte == b'\n')
        .filter_map(|line| {
            let event = parse_known_events(line).ok()?.into_iter().next()?;
            let record = line.strip_suffix(b"\n")?;
            (serialize_event(&event).ok()?.as_slice() != record).then(|| {
                format!(
                    "line does not reproduce: {}",
                    String::from_utf8_lossy(record)
                )
            })
        })
        .chain(
            ids.into_iter()
                .filter(|(_, lines)| *lines > 1)
                .map(|(id, lines)| format!("event id '{id}' appears on {lines} lines")),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::GateStage;
    use crate::domain::event_catalog::EventTag;
    use crate::domain::identity::GateKey;
    use crate::domain::repository_inputs::InputsDigest;
    use crate::domain::GateStatus;
    use crate::repository_state::{DirectoryListing, EntryIdentity, FileMode, RepositoryEntry};
    use crate::test_harness::image::ImageBuilder;

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

    /// A catalog sample's line, under an event id only this tag's lines use.
    fn event_line(tag: EventTag) -> String {
        let position = EventTag::ALL.iter().position(|each| *each == tag).unwrap();
        let mut value: Value =
            serde_json::from_slice(&serialize_event(&tag.sample()).unwrap()).unwrap();
        value["id"] = format!("00000000-0000-0000-0000-{position:012x}").into();
        value.to_string()
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

    fn directory() -> RepositoryEntry {
        RepositoryEntry::Directory {
            identity: EntryIdentity::for_bytes("dir", b"dir").unwrap(),
            mode: FileMode::Regular,
        }
    }

    fn drift_over(gate_runs: ImageBuilder) -> Vec<String> {
        let image = ImageBuilder::new()
            .entry(VirtualPath::INDEX, RepositoryEntry::Absent)
            .build();
        record_drift(&image, &gate_runs.build(), None).unwrap()
    }

    #[test]
    fn test_record_drift_reports_a_verdict_record_that_does_not_reproduce() {
        let key: GateKey = "tests".parse().unwrap();
        let digest = |fill: &str| InputsDigest::try_from(fill.repeat(64)).unwrap();
        let record = GateVerdictRecord {
            issue_id: crate::test_utils::issue_id(7),
            run_id: crate::test_utils::gate_run_id(1),
        };
        let canonical = VirtualPath::gate_verdict_record(&key, &digest("a"));
        let compact = VirtualPath::gate_verdict_record(&key, &digest("b"));
        let directory_names = |names: &[&str]| {
            DirectoryListing::new(names.iter().map(|name| name.to_string())).unwrap()
        };

        let drift = drift_over(
            ImageBuilder::new()
                .entry(VirtualPath::GATE_VERDICTS, directory())
                .listing(VirtualPath::GATE_VERDICTS, directory_names(&["tests"]))
                .entry(VirtualPath::gate_verdict_directory(&key), directory())
                .listing(
                    VirtualPath::gate_verdict_directory(&key),
                    directory_names(&[
                        canonical.file_name().unwrap(),
                        compact.file_name().unwrap(),
                    ]),
                )
                .file(canonical, serialize_gate_verdict(&record).unwrap())
                .file(compact.clone(), serde_json::to_vec(&record).unwrap()),
        );

        assert_eq!(drift.len(), 1, "{drift:?}");
        assert!(
            drift[0].starts_with(&compact.repository_relative()),
            "{drift:?}"
        );
    }

    #[test]
    fn test_record_drift_enumerates_no_gate_run_under_the_verdicts_directory() {
        let verdicts = VirtualPath::GATE_VERDICTS.file_name().unwrap().to_string();

        let drift = drift_over(
            ImageBuilder::new()
                .entry(VirtualPath::GATE_RUNS, directory())
                .listing(
                    VirtualPath::GATE_RUNS,
                    DirectoryListing::new([verdicts]).unwrap(),
                )
                .entry(VirtualPath::GATE_VERDICTS, directory()),
        );

        assert_eq!(drift, Vec::<String>::new());
    }

    #[test]
    fn test_event_log_drift_is_empty_over_every_cataloged_tag() {
        let log = EventTag::ALL
            .iter()
            .map(|tag| event_line(*tag))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(event_log_drift(log.as_bytes()), Vec::<String>::new());
    }

    #[test]
    fn test_event_log_drift_names_an_id_on_more_than_one_line_whether_or_not_they_match() {
        let first = event_line(EventTag::ALL[0]);
        let id = serde_json::from_str::<Value>(&first).unwrap()["id"].clone();
        let mut other = serde_json::from_str::<Value>(&event_line(EventTag::ALL[1])).unwrap();
        other["id"] = id.clone();
        let log = format!("{first}\n{first}\n{other}\n");

        let drift = event_log_drift(log.as_bytes());

        assert_eq!(drift.len(), 1, "{drift:?}");
        assert!(drift[0].contains(id.as_str().unwrap()), "{drift:?}");
    }

    #[test]
    fn test_event_log_drift_ignores_an_unterminated_final_line() {
        let line = event_line(EventTag::ALL[0]);
        let log = format!("{line}\n{}", line.replacen(':', ": ", 1));

        assert_eq!(event_log_drift(log.as_bytes()), Vec::<String>::new());
    }

    #[test]
    fn test_event_log_drift_reports_an_unparseable_log_as_one_finding_naming_its_line() {
        let line = event_line(EventTag::ALL[0]);
        let log = format!("{line}\nnot json\n{line}\n");

        let drift = event_log_drift(log.as_bytes());

        assert_eq!(drift.len(), 1, "{drift:?}");
        assert!(drift[0].contains("line 2"), "{drift:?}");
    }

    #[test]
    fn test_event_log_drift_reports_a_line_that_does_not_reproduce() {
        let line = event_line(EventTag::ALL[0]);
        let spaced = event_line(EventTag::ALL[1]).replacen(':', ": ", 1);
        let log = format!("{line}\n{spaced}\n");

        let drift = event_log_drift(log.as_bytes());

        assert_eq!(drift.len(), 1, "{drift:?}");
        assert!(drift[0].contains(&spaced));
    }
}
