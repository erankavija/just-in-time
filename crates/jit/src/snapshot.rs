//! Snapshot export functionality for creating portable, verifiable snapshots of JIT repository state.
//!
//! Snapshots capture complete issue state with all linked documents and assets at a specific point
//! in time. They are critical for handoffs, audits, compliance, and reproducibility.

use anyhow::{anyhow, Result};
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
    /// Total number of issues
    pub count: usize,
    /// Count of issues by state
    pub states: std::collections::HashMap<String, usize>,
    /// List of issue file paths relative to snapshot root
    pub files: Vec<String>,
}

/// Information about documents in the snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentsInfo {
    /// Total number of documents
    pub count: usize,
    /// List of document snapshots
    pub items: Vec<DocumentSnapshot>,
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

/// Scope of snapshot export
#[derive(Debug, Clone)]
pub enum SnapshotScope {
    /// Export all issues
    All,
    /// Export a single issue
    Issue(String),
    /// Export all issues with a specific label
    Label { namespace: String, value: String },
}

impl SnapshotScope {
    /// Parse scope from string format: "all" | "issue:ID" | "label:namespace:value"
    pub fn parse(s: &str) -> Result<Self> {
        if s == "all" {
            Ok(SnapshotScope::All)
        } else if let Some(id) = s.strip_prefix("issue:") {
            Ok(SnapshotScope::Issue(id.to_string()))
        } else if let Some(label_part) = s.strip_prefix("label:") {
            // Parse "namespace:value" from label:namespace:value
            let parts: Vec<&str> = label_part.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(anyhow!(
                    "Invalid label scope format: '{}'. Expected 'label:namespace:value'",
                    s
                ));
            }
            Ok(SnapshotScope::Label {
                namespace: parts[0].to_string(),
                value: parts[1].to_string(),
            })
        } else {
            Err(anyhow!(
                "Invalid scope format: '{}'. Expected 'all', 'issue:ID', or 'label:namespace:value'",
                s
            ))
        }
    }
}

impl std::fmt::Display for SnapshotScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotScope::All => write!(f, "all"),
            SnapshotScope::Issue(id) => write!(f, "issue:{}", id),
            SnapshotScope::Label { namespace, value } => write!(f, "label:{}:{}", namespace, value),
        }
    }
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
            _ => Err(anyhow!("Invalid format: '{}'. Expected 'dir' or 'tar'", s)),
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
    fn test_scope_parse_all() {
        let scope = SnapshotScope::parse("all").unwrap();
        matches!(scope, SnapshotScope::All);
        assert_eq!(scope.to_string(), "all");
    }

    #[test]
    fn test_scope_parse_issue() {
        let scope = SnapshotScope::parse("issue:abc123").unwrap();
        match &scope {
            SnapshotScope::Issue(id) => assert_eq!(id, "abc123"),
            _ => panic!("Expected Issue variant"),
        }
        assert_eq!(scope.to_string(), "issue:abc123");
    }

    #[test]
    fn test_scope_parse_label() {
        let scope = SnapshotScope::parse("label:epic:auth").unwrap();
        match &scope {
            SnapshotScope::Label { namespace, value } => {
                assert_eq!(namespace, "epic");
                assert_eq!(value, "auth");
            }
            _ => panic!("Expected Label variant"),
        }
        assert_eq!(scope.to_string(), "label:epic:auth");
    }

    #[test]
    fn test_scope_parse_label_milestone() {
        let scope = SnapshotScope::parse("label:milestone:v1.0").unwrap();
        match &scope {
            SnapshotScope::Label { namespace, value } => {
                assert_eq!(namespace, "milestone");
                assert_eq!(value, "v1.0");
            }
            _ => panic!("Expected Label variant"),
        }
        assert_eq!(scope.to_string(), "label:milestone:v1.0");
    }

    #[test]
    fn test_scope_parse_label_missing_value() {
        let result = SnapshotScope::parse("label:epic");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Expected 'label:namespace:value'"));
    }

    #[test]
    fn test_scope_parse_invalid() {
        let result = SnapshotScope::parse("invalid:123");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid scope format"));
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
}
