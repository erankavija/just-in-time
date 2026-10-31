//! Decoding of captured issue records.

use super::VirtualPath;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::issue_id;

    #[test]
    fn test_decode_issue_record_rejects_a_non_canonical_dependency_naming_the_path() {
        let path = VirtualPath::data(format!("issues/{}.json", issue_id(1))).unwrap();
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
