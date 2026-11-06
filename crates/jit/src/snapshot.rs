//! Snapshot export functionality for creating portable, verifiable snapshots of JIT repository state.
//!
//! Snapshots capture complete issue state with all linked documents and assets at a specific point
//! in time. They are critical for handoffs, audits, compliance, and reproducibility.

use crate::domain::list_envelope::{CollectionKey, ListEnvelope};
use crate::errors::InvalidArgumentError;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Snapshot manifest containing complete provenance and verification data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotManifest {
    /// Manifest version (currently "1")
    pub version: String,
    /// ISO 8601 timestamp when snapshot was created
    pub created_at: String,
    /// Tool and version that created this snapshot
    pub created_by: String,
    /// Repository information
    pub repo: RepoInfo,
    /// Scope of the snapshot
    pub scope: String,
    /// Information about included issues
    pub issues: IssuesInfo,
    /// Information about included documents
    pub documents: DocumentsInfo,
    /// Metadata about snapshot policies
    pub metadata: MetadataInfo,
    /// Verification information
    pub verification: VerificationInfo,
}

/// Repository information in the snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoInfo {
    /// Absolute path to the repository
    pub path: String,
    /// Remote URL (if git repository)
    pub remote: Option<String>,
    /// Commit SHA (null if working-tree mode)
    pub commit: Option<String>,
    /// Current branch name (null if working-tree or detached)
    pub branch: Option<String>,
    /// Whether working tree has uncommitted changes
    pub dirty: bool,
    /// Source mode: "git" or "working-tree"
    pub source: String,
}

/// Information about issues in the snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssuesInfo {
    /// Issue file paths and their state counts.
    #[serde(flatten)]
    pub files: ListEnvelope<SnapshotIssueFiles, String, IssueStates>,
}

/// Information about documents in the snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentsInfo {
    /// Document snapshots, with a count derived from the collection.
    #[serde(flatten)]
    pub items: ListEnvelope<SnapshotDocumentItems, DocumentSnapshot>,
}

pub enum SnapshotIssueFiles {}

impl CollectionKey for SnapshotIssueFiles {
    const KEY: &'static str = "files";
}

pub enum SnapshotDocumentItems {}

impl CollectionKey for SnapshotDocumentItems {
    const KEY: &'static str = "items";
}

/// Counts of snapshot issues by canonical state name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueStates {
    /// Count of issues by state.
    pub states: std::collections::HashMap<crate::domain::State, usize>,
}

/// Snapshot of a single document with verification data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSnapshot {
    /// Path to document relative to repository root
    pub path: String,
    /// Document format (e.g., "markdown")
    pub format: String,
    /// Size in bytes
    pub size_bytes: usize,
    /// Source information
    pub source: SourceInfo,
    /// SHA256 hash of document content
    pub hash_sha256: String,
    /// Associated assets
    pub assets: Vec<AssetSnapshot>,
}

/// Source information for a file in the snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    /// Source type: "git" or "filesystem"
    #[serde(rename = "type")]
    pub type_: String,
    /// Commit SHA (null if filesystem)
    pub commit: Option<String>,
    /// Git blob SHA (null if filesystem)
    pub blob_sha: Option<String>,
}

/// Snapshot of an asset file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetSnapshot {
    /// Path to asset relative to repository root
    pub path: String,
    /// Source information
    pub source: SourceInfo,
    /// SHA256 hash of asset content
    pub hash_sha256: String,
    /// MIME type
    pub mime: String,
    /// Size in bytes
    pub size_bytes: usize,
    /// Whether this asset is shared by multiple documents
    pub shared: bool,
}

/// Metadata about snapshot policies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataInfo {
    /// How links are handled: "preserve" | "inline" | "convert"
    pub link_policy: String,
    /// How external assets are handled: "include" | "exclude"
    pub external_assets_policy: String,
    /// How Git LFS is handled: "allow-pointers" | "download"
    pub lfs_policy: String,
}

/// Verification information for the snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationInfo {
    /// Total number of files in snapshot
    pub total_files: usize,
    /// Total size in bytes
    pub total_bytes: usize,
    /// Instructions for verifying snapshot integrity
    pub instructions: String,
}

/// Output format for snapshot
#[derive(Debug, Clone)]
pub enum SnapshotFormat {
    /// Directory structure
    Directory,
    /// Tar archive
    Tar,
}

impl SnapshotFormat {
    /// Parse format from string: "dir" | "tar"
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "dir" => Ok(SnapshotFormat::Directory),
            "tar" => Ok(SnapshotFormat::Tar),
            _ => Err(InvalidArgumentError::new(format!(
                "Invalid format: '{s}'. Expected 'dir' or 'tar'"
            ))
            .into()),
        }
    }
}

/// Source mode for reading files
#[derive(Debug, Clone)]
pub enum SourceMode {
    /// Read from git at specific commit
    Git { commit: String },
    /// Read from working tree filesystem
    WorkingTree,
}

/// Compute SHA256 hash of data
pub fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_sha256() {
        let data = b"hello world";
        let hash = compute_sha256(data);
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_compute_sha256_empty() {
        let data = b"";
        let hash = compute_sha256(data);
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_format_parse_dir() {
        let format = SnapshotFormat::parse("dir").unwrap();
        matches!(format, SnapshotFormat::Directory);
    }

    #[test]
    fn test_format_parse_tar() {
        let format = SnapshotFormat::parse("tar").unwrap();
        matches!(format, SnapshotFormat::Tar);
    }

    #[test]
    fn test_format_parse_invalid() {
        let result = SnapshotFormat::parse("zip");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid format"));
    }

    #[test]
    fn test_issues_info_states_json_uses_snake_case_keys() {
        // Regression guard: the state keys in IssuesInfo.states must serialize as
        // canonical snake_case ("in_progress"), never as the Debug repr ("inprogress").
        use crate::domain::State;
        use std::collections::HashMap;

        let mut states: HashMap<State, usize> = HashMap::new();
        states.insert(State::InProgress, 2);
        states.insert(State::Done, 1);
        let info = IssuesInfo {
            files: ListEnvelope::with_extras(vec![], IssueStates { states }),
        };

        let json = serde_json::to_string(&info).expect("IssuesInfo serializes");

        assert!(
            json.contains("\"in_progress\""),
            "expected snake_case key \"in_progress\" in JSON, got: {}",
            json
        );
        assert!(
            !json.contains("\"inprogress\""),
            "must not contain Debug-format key \"inprogress\", got: {}",
            json
        );
        assert!(
            json.contains("\"done\""),
            "expected key \"done\" in JSON, got: {}",
            json
        );
    }
}
