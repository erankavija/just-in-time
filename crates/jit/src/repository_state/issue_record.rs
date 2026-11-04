//! Decoding of captured issue records.

use super::{CaptureError, VirtualPath};
use crate::domain::identity::IssueId;
use crate::domain::Issue;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_json::{Map, Value};
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
    decode_issue_value(path, bytes).map(|(issue, _)| issue)
}

/// Decode the record at `path` to its JSON value, and the issue from that value.
///
/// Record fidelity compares the value with the issue's serialization, so the
/// value is the one decode. It rejects a repeated key as the typed decode
/// does; a typed decode of the bytes runs only to locate a failed issue decode.
fn decode_issue_value(
    path: &VirtualPath,
    bytes: &[u8],
) -> Result<(Issue, Value), MalformedIssueRecordError> {
    let malformed = |source| MalformedIssueRecordError {
        path: path.relative().as_str().to_string(),
        source,
    };
    let UniqueKeys(value) = serde_json::from_slice(bytes).map_err(malformed)?;
    let issue = Issue::deserialize(&value).map_err(|error| {
        malformed(
            serde_json::from_slice::<Issue>(bytes)
                .err()
                .unwrap_or(error),
        )
    })?;
    Ok((issue, value))
}

/// A JSON value whose objects reject a repeated key at its position.
struct UniqueKeys(Value);

impl<'de> Deserialize<'de> for UniqueKeys {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer
            .deserialize_any(UniqueKeysVisitor)
            .map(UniqueKeys)
    }
}

struct UniqueKeysVisitor;

impl<'de> Visitor<'de> for UniqueKeysVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_string<E>(self, value: String) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        std::iter::from_fn(|| seq.next_element::<UniqueKeys>().transpose())
            .map(|element| element.map(|UniqueKeys(value)| value))
            .collect::<Result<_, _>>()
            .map(Value::Array)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut object = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(de::Error::custom(format_args!("duplicate key `{key}`")));
            }
            let UniqueKeys(value) = map.next_value()?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
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

/// Parse the record of each listed issue id from `read`, in listing order,
/// each beside the JSON value it decoded from.
pub fn parse_issue_records<'a>(
    ids: &[IssueId],
    read: impl Fn(&VirtualPath) -> Result<Option<&'a [u8]>, CaptureError>,
) -> Result<Vec<(Issue, Value)>, IssueRecordsError> {
    ids.iter()
        .map(|id| {
            let path = VirtualPath::issue_record(id);
            let bytes = read(&path)?.ok_or_else(|| IssueRecordsError::Missing(id.clone()))?;
            let (issue, value) =
                decode_issue_value(&path, bytes).map_err(|error| IssueRecordsError::Malformed {
                    id: id.clone(),
                    error,
                })?;
            if &issue.id != id {
                return Err(IssueRecordsError::MismatchedId {
                    expected: id.clone(),
                    found: issue.id,
                });
            }
            Ok((issue, value))
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
    ) -> Result<Vec<(Issue, Value)>, IssueRecordsError> {
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

        let parsed: Vec<_> = issues.into_iter().map(|(issue, _)| issue.id).collect();
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
    fn test_decode_issue_record_rejects_a_duplicated_key_at_its_position() {
        let path = VirtualPath::issue_record(&issue_id(1));
        let record = String::from_utf8(record_bytes(&issue_id(1))).unwrap();
        let duplicated = record.replacen('{', "{\"title\":\"a\",\n\"title\":\"b\",", 1);

        let error = decode_issue_record(&path, duplicated.as_bytes())
            .unwrap_err()
            .into_source();

        assert!(error.to_string().contains("title"), "{error}");
        assert_eq!((error.line(), error.column()), (2, 7), "{error}");
    }

    #[test]
    fn test_decode_issue_record_locates_a_mistyped_field() {
        let path = VirtualPath::issue_record(&issue_id(1));
        let mut record =
            serde_json::to_value(Issue::draft(issue_id(1), "t".into(), String::new())).unwrap();
        record["title"] = serde_json::json!(7);

        let error = decode_issue_record(&path, &serde_json::to_vec(&record).unwrap())
            .unwrap_err()
            .into_source();

        assert!(error.line() > 0, "{error}");
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
