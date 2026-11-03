//! Decoding of captured issue records.

use super::{CaptureError, VirtualPath};
use crate::domain::identity::IssueId;
use crate::domain::Issue;
use thiserror::Error;

/// A captured issue record that does not decode, including a non-canonical id
/// or dependency.
#[derive(Debug, Error)]
#[error("malformed issue record at {path}: {source}")]
pub struct MalformedIssueRecordError {
    path: String,
    #[source]
    source: serde_json::Error,
}

impl MalformedIssueRecordError {
    /// The JSON decode error, without the record path.
    pub(crate) fn into_source(self) -> serde_json::Error {
        self.source
    }
}

/// Decode the issue record captured at `path`.
pub fn decode_issue_record(
    path: &VirtualPath,
    bytes: &[u8],
) -> Result<Issue, MalformedIssueRecordError> {
    serde_json::from_slice(bytes).map_err(|source| MalformedIssueRecordError {
        path: path.relative().as_str().to_string(),
        source,
    })
}

/// A listed issue record that could not be read or parsed.
#[derive(Debug, Error)]
pub enum IssueRecordsError {
    #[error(transparent)]
    Read(#[from] CaptureError),
    #[error("required repository file '{}' is missing", VirtualPath::issue_record(.0).repository_relative())]
    Missing(IssueId),
    #[error("{error}")]
    Malformed {
        id: IssueId,
        error: MalformedIssueRecordError,
    },
    #[error(
        "issue file '{}' contains id '{found}' instead of '{expected}'",
        VirtualPath::issue_record(expected).repository_relative()
    )]
    MismatchedId { expected: IssueId, found: IssueId },
}

/// Parse the record of each listed issue id from `read`, in listing order.
pub fn parse_issue_records<'a>(
    ids: &[IssueId],
    read: impl Fn(&VirtualPath) -> Result<Option<&'a [u8]>, CaptureError>,
) -> Result<Vec<Issue>, IssueRecordsError> {
    ids.iter()
        .map(|id| {
            let path = VirtualPath::issue_record(id);
            let bytes = read(&path)?.ok_or_else(|| IssueRecordsError::Missing(id.clone()))?;
            let issue = decode_issue_record(&path, bytes).map_err(|error| {
                IssueRecordsError::Malformed {
                    id: id.clone(),
                    error,
                }
            })?;
            if &issue.id != id {
                return Err(IssueRecordsError::MismatchedId {
                    expected: id.clone(),
                    found: issue.id,
                });
            }
            Ok(issue)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::issue_id;
    use std::collections::BTreeMap;

    fn record_bytes(id: &IssueId) -> Vec<u8> {
        serde_json::to_vec(&Issue::draft(id.clone(), "t".into(), String::new())).unwrap()
    }

    /// Parse `ids` from `records`, a byte source holding only those paths.
    fn parse_from(
        ids: &[IssueId],
        records: &BTreeMap<VirtualPath, Vec<u8>>,
    ) -> Result<Vec<Issue>, IssueRecordsError> {
        parse_issue_records(ids, |path| Ok(records.get(path).map(Vec::as_slice)))
    }

    #[test]
    fn test_parse_issue_records_returns_well_formed_records_in_listing_order() {
        let ids = [issue_id(2), issue_id(1)];
        let records = ids
            .iter()
            .map(|id| (VirtualPath::issue_record(id), record_bytes(id)))
            .collect();

        let issues = parse_from(&ids, &records).unwrap();

        let parsed: Vec<_> = issues.into_iter().map(|issue| issue.id).collect();
        assert_eq!(parsed, ids);
    }

    #[test]
    fn test_parse_issue_records_reports_a_missing_record_by_id() {
        let error = parse_from(&[issue_id(1)], &BTreeMap::new()).unwrap_err();

        assert!(
            matches!(&error, IssueRecordsError::Missing(id) if *id == issue_id(1)),
            "{error}"
        );
    }

    #[test]
    fn test_parse_issue_records_rejects_a_record_embedding_another_id() {
        let records = BTreeMap::from([(
            VirtualPath::issue_record(&issue_id(1)),
            record_bytes(&issue_id(2)),
        )]);

        let error = parse_from(&[issue_id(1)], &records).unwrap_err();

        assert!(
            matches!(&error, IssueRecordsError::MismatchedId { expected, found }
                if *expected == issue_id(1) && *found == issue_id(2)),
            "{error}"
        );
    }

    #[test]
    fn test_parse_issue_records_rejects_an_unparseable_record() {
        let records = BTreeMap::from([(VirtualPath::issue_record(&issue_id(1)), b"{".to_vec())]);

        let error = parse_from(&[issue_id(1)], &records).unwrap_err();

        assert!(
            matches!(&error, IssueRecordsError::Malformed { id, .. } if *id == issue_id(1)),
            "{error}"
        );
    }

    #[test]
    fn test_decode_issue_record_rejects_a_non_canonical_dependency_naming_the_path() {
        let path = VirtualPath::issue_record(&issue_id(1));
        let mut record =
            serde_json::to_value(Issue::draft(issue_id(1), "t".into(), String::new())).unwrap();
        record["dependencies"] = serde_json::json!(["not-canonical"]);

        let error = decode_issue_record(&path, &serde_json::to_vec(&record).unwrap()).unwrap_err();

        let message = error.to_string();
        assert!(
            message.contains(path.relative().as_str()) && message.contains("not-canonical"),
            "{message}"
        );
    }
}
