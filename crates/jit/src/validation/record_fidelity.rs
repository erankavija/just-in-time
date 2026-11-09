//! Whether stored records decode and reproduce through their writer's serializer.
//!
//! A record that decodes but does not reproduce loses data at its next rewrite.
//! The event log is judged from its whole bytes: its parser ignores an
//! unterminated final line, while each committed line must parse and reproduce.
//! An unparseable log is one finding naming the line; a parseable one reports
//! every line that does not reproduce and each event id on more than one line.

use crate::domain::artifact_execution::ArchiveEvidenceRecord;
use crate::domain::event_log::parse_known_events;
use crate::domain::identity::{GateRunId, IssueId};
use crate::domain::{Event, GateRunResult, GateVerdictRecord, Issue};
use crate::repository_state::{
    canonical_json, captured_gate_run_ids, captured_gate_verdict_records, serialize_event,
    serialize_gate_run, serialize_gate_verdict, serialize_issue, RepositoryImage, RepositoryIndex,
    VirtualPath,
};
use crate::validation::CapturedValidation;
use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;

/// One message per stored record that does not decode or reproduce, over
/// `captured`'s parsed index and issue records, `records`' runs, verdict
/// records and archive evidence, and the whole `events`.
pub(crate) fn record_drift(
    captured: &CapturedValidation<'_>,
    records: &RepositoryImage,
    events: Option<&[u8]>,
) -> Result<Vec<String>> {
    let report = |path: &VirtualPath, check: Result<Option<String>>| {
        check
            .unwrap_or_else(|error| Some(error.to_string()))
            .map(|reason| format!("{}: {reason}", path.repository_relative()))
    };
    let index = match &captured.index {
        Ok(index) => captured
            .image
            .file_bytes(&VirtualPath::INDEX)?
            .and_then(|bytes| report(&VirtualPath::INDEX, index_drift(index, bytes))),
        Err(_) => None,
    };
    let issues = captured
        .issues
        .as_deref()
        .unwrap_or_default()
        .iter()
        .zip(captured.values.iter())
        .map(|(issue, value)| {
            report(
                &VirtualPath::issue_record(&issue.id),
                issue_drift(issue, value),
            )
        });
    let runs = captured_gate_run_ids(records)
        .iter()
        .map(|(issue, run)| {
            let path = VirtualPath::gate_run_record(issue, run);
            let check = match records.file_bytes(&path)? {
                Some(bytes) => gate_run_drift(issue, run, bytes),
                None => Ok(Some("missing".to_string())),
            };
            Ok(report(&path, check))
        })
        .collect::<Result<Vec<_>>>()?;
    let verdicts = captured_gate_verdict_records(records)
        .iter()
        .map(|path| {
            Ok(records
                .file_bytes(path)?
                .and_then(|bytes| report(path, verdict_drift(bytes))))
        })
        .collect::<Result<Vec<_>>>()?;
    let evidence = records
        .entries()
        .keys()
        .filter(|path| {
            path.root_class() == VirtualPath::ARCHIVE_EVIDENCE.root_class()
                && path
                    .relative()
                    .as_path()
                    .starts_with(VirtualPath::ARCHIVE_EVIDENCE.relative().as_path())
                && path
                    .relative()
                    .as_path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            let check = match records.file_bytes(path) {
                Ok(Some(bytes)) => archive_evidence_drift(path, bytes),
                Ok(None) => Ok(Some("missing".to_string())),
                Err(error) => Err(error.into()),
            };
            report(path, check)
        });
    let events = events.map_or_else(Vec::new, event_log_drift);
    Ok(issues
        .into_iter()
        .chain(runs)
        .chain(verdicts)
        .chain(evidence)
        .chain([index])
        .flatten()
        .chain(
            events
                .into_iter()
                .map(|reason| format!("{}: {reason}", VirtualPath::EVENTS.repository_relative())),
        )
        .collect())
}

fn archive_evidence_drift(path: &VirtualPath, bytes: &[u8]) -> Result<Option<String>> {
    let evidence: ArchiveEvidenceRecord = serde_json::from_slice(bytes)?;
    if VirtualPath::archive_evidence_record(&evidence.target) != *path
        || evidence
            .executions
            .iter()
            .any(|execution| execution.target != evidence.target)
    {
        return Ok(Some("archive evidence contains another target".to_string()));
    }
    let raw: Value = serde_json::from_slice(bytes)?;
    Ok(
        (canonical_json(&evidence, true)? != canonical_json(&raw, true)?)
            .then(|| "re-serializing changes the record".to_string()),
    )
}

fn index_drift(index: &RepositoryIndex, bytes: &[u8]) -> Result<Option<String>> {
    Ok((index.to_pretty_bytes()? != bytes).then(|| "stored bytes are not canonical".to_string()))
}

/// Canonical rather than byte-exact: key order is not part of an issue record.
fn issue_drift(issue: &Issue, value: &Value) -> Result<Option<String>> {
    Ok((serialize_issue(issue)? != canonical_json(value, true)?)
        .then(|| "re-serializing changes the record".to_string()))
}

/// A record names the issue and run its path names. One carrying the current
/// key set reproduces byte for byte; one written under an earlier schema keeps
/// its identity fields.
fn gate_run_drift(issue: &IssueId, id: &GateRunId, bytes: &[u8]) -> Result<Option<String>> {
    let run: GateRunResult = serde_json::from_slice(bytes)?;
    if run.issue_id != *issue {
        return Ok(Some(format!("holds issue id '{}'", run.issue_id)));
    }
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
            &run_issue(),
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
    fn test_record_drift_reports_archive_evidence_that_loses_data() {
        use crate::test_harness::image::ImageBuilder;
        let target = crate::domain::artifact_plan::PlanTarget::Document {
            path: "doc.md".into(),
        };
        let path = VirtualPath::archive_evidence_record(&target);
        let mut raw = serde_json::to_value(ArchiveEvidenceRecord {
            executions: vec![crate::test_utils::archive_execution(
                target.clone(),
                "archive",
                &[],
            )],
            target,
        })
        .unwrap();
        let image = ImageBuilder::new()
            .file(
                VirtualPath::INDEX,
                crate::repository_state::fresh_index_bytes().unwrap(),
            )
            .build();
        let records = ImageBuilder::new()
            .file(path.clone(), raw.to_string())
            .build();
        assert!(
            record_drift(&CapturedValidation::from_image(&image), &records, None)
                .unwrap()
                .is_empty()
        );
        raw["executions"][0]["field_no_reader_knows"] = true.into();
        let records = ImageBuilder::new()
            .file(path.clone(), raw.to_string())
            .build();
        let drift = record_drift(&CapturedValidation::from_image(&image), &records, None).unwrap();
        assert_eq!(drift.len(), 1, "{drift:?}");
        assert!(drift[0].contains(&path.repository_relative()));
    }

    #[test]
    fn test_index_drift_reports_only_a_non_canonical_form() {
        let canonical = crate::repository_state::fresh_index_bytes().unwrap();
        let index = RepositoryIndex::parse(&canonical).unwrap();
        assert_eq!(index_drift(&index, &canonical).unwrap(), None);

        let spaced = [canonical.as_slice(), b"\n"].concat();
        assert!(index_drift(&index, &spaced).unwrap().is_some());
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
            issue_drift(
                &issue,
                &serde_json::from_str(&format!("{{{reversed}}}")).unwrap()
            )
            .unwrap(),
            None
        );

        raw["field_no_reader_knows"] = Value::Bool(true);
        assert!(issue_drift(&issue, &raw).unwrap().is_some());
    }

    fn run_issue() -> IssueId {
        crate::test_utils::issue_id(7)
    }

    #[test]
    fn test_gate_run_drift_reports_a_record_stored_under_another_run_id() {
        let holding = crate::test_utils::gate_run_id(2);
        let reason = gate_run_drift(
            &run_issue(),
            &crate::test_utils::gate_run_id(1),
            &run_bytes(&holding),
        );
        assert!(reason.unwrap().unwrap().contains(holding.as_str()));
    }

    #[test]
    fn test_record_drift_reports_a_record_stored_under_another_issue_directory() {
        let (directory, run) = (
            crate::test_utils::issue_id(8),
            crate::test_utils::gate_run_id(1),
        );
        let record = VirtualPath::gate_run_record(&directory, &run);

        let drift = drift_over(
            ImageBuilder::new()
                .entry(VirtualPath::GATE_RUNS, directory_entry())
                .listing(
                    VirtualPath::GATE_RUNS,
                    DirectoryListing::new([directory.to_string()]).unwrap(),
                )
                .entry(
                    VirtualPath::gate_run_issue_directory(&directory),
                    directory_entry(),
                )
                .listing(
                    VirtualPath::gate_run_issue_directory(&directory),
                    DirectoryListing::new([record.file_name().unwrap().to_string()]).unwrap(),
                )
                .file(record.clone(), run_bytes(&run)),
        );

        assert_eq!(drift.len(), 1, "{drift:?}");
        assert!(
            drift[0].starts_with(&record.repository_relative()),
            "{drift:?}"
        );
        assert!(drift[0].contains(run_issue().as_str()), "{drift:?}");
    }

    #[test]
    fn test_gate_run_drift_exempts_an_earlier_schema_from_bytes_but_not_identity() {
        let id = crate::test_utils::gate_run_id(1);
        let mut raw: Value = serde_json::from_slice(&run_bytes(&id)).unwrap();
        raw["retired_key"] = Value::Bool(true);
        assert_eq!(
            gate_run_drift(&run_issue(), &id, &serde_json::to_vec(&raw).unwrap()).unwrap(),
            None
        );

        raw["origin"] = serde_json::json!({ "kind": "not-a-recorded-origin" });
        let lost = gate_run_drift(&run_issue(), &id, &serde_json::to_vec(&raw).unwrap());
        assert!(!matches!(lost, Ok(None)));
    }

    #[test]
    fn test_gate_run_drift_holds_current_keys_to_canonical_bytes() {
        let id = crate::test_utils::gate_run_id(1);
        let canonical = run_bytes(&id);
        assert_eq!(gate_run_drift(&run_issue(), &id, &canonical).unwrap(), None);

        let raw: Value = serde_json::from_slice(&canonical).unwrap();
        assert!(
            gate_run_drift(&run_issue(), &id, &serde_json::to_vec(&raw).unwrap())
                .unwrap()
                .is_some()
        );
    }

    fn directory_entry() -> RepositoryEntry {
        RepositoryEntry::Directory {
            identity: EntryIdentity::for_bytes("dir", b"dir").unwrap(),
            mode: FileMode::Regular,
        }
    }

    fn drift_over(gate_runs: ImageBuilder) -> Vec<String> {
        let image = ImageBuilder::new()
            .entry(VirtualPath::INDEX, RepositoryEntry::Absent)
            .build();
        record_drift(
            &CapturedValidation::from_image(&image),
            &gate_runs.build(),
            None,
        )
        .unwrap()
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
                .entry(VirtualPath::GATE_VERDICTS, directory_entry())
                .listing(VirtualPath::GATE_VERDICTS, directory_names(&["tests"]))
                .entry(VirtualPath::gate_verdict_directory(&key), directory_entry())
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
                .entry(VirtualPath::GATE_RUNS, directory_entry())
                .listing(
                    VirtualPath::GATE_RUNS,
                    DirectoryListing::new([verdicts]).unwrap(),
                )
                .entry(VirtualPath::GATE_VERDICTS, directory_entry()),
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
