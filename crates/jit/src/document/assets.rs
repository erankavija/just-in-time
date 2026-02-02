//! Asset discovery and management
//!
//! Scans documents for asset references, classifies them as per-doc vs shared,
//! resolves paths, and computes metadata (MIME types, hashes).

use crate::document::AdapterRegistry;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Asset reference discovered in a document
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Asset {
    /// Original path as written in the document
    pub original_path: String,
    /// Resolved absolute path from repository root
    pub resolved_path: Option<PathBuf>,
    /// Asset type classification
    pub asset_type: AssetType,
    /// MIME type based on file extension
    pub mime_type: Option<String>,
    /// SHA256 hash of file content (if file exists)
    pub content_hash: Option<String>,
    /// Whether this asset is shared across multiple documents
    pub is_shared: bool,
}

/// Type of asset reference
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    /// Local file asset (relative or root-relative path)
    Local,
    /// External URL (http://, https://)
    External,
    /// File not found at resolved path
    Missing,
}

/// Asset scanner for documents
///
/// Scans documents using format adapters to discover asset references,
/// resolves paths, and computes metadata.
///
/// # Example
///
/// ```
/// use jit::document::{AdapterRegistry, AssetScanner};
/// use std::path::Path;
///
/// let registry = AdapterRegistry::with_builtins();
/// let scanner = AssetScanner::new(registry, Path::new("/repo"));
///
/// let assets = scanner
///     .scan_document(Path::new("docs/guide.md"), "# Guide\n\n![Logo](./logo.png)")
///     .unwrap();
///
/// assert_eq!(assets.len(), 1);
/// assert_eq!(assets[0].original_path, "./logo.png");
/// ```
pub struct AssetScanner {
    registry: AdapterRegistry,
    repo_root: PathBuf,
}

impl AssetScanner {
    /// Create a new asset scanner
    ///
    /// # Arguments
    ///
    /// * `registry` - Format adapter registry for document scanning
    /// * `repo_root` - Repository root path for resolving relative paths
    pub fn new(registry: AdapterRegistry, repo_root: &Path) -> Self {
        Self {
            registry,
            repo_root: repo_root.to_path_buf(),
        }
    }

    /// Scan a document for asset references
    ///
    /// Returns a list of assets discovered in the document.
    /// Path resolution is relative to the document's directory.
    ///
    /// # Arguments
    ///
    /// * `doc_path` - Path to the document (relative to repo root)
    /// * `content` - Document content
    pub fn scan_document(&self, doc_path: &Path, content: &str) -> Result<Vec<Asset>, String> {
        let doc_path_str = doc_path.to_string_lossy().to_string();

        // Resolve adapter for this document
        let adapter = self
            .registry
            .resolve(&doc_path_str, content)
            .ok_or_else(|| format!("No adapter found for document: {}", doc_path_str))?;

        // Scan for asset references
        let asset_paths = adapter.scan_assets(content);

        // Resolve and classify each asset
        let mut assets = Vec::new();
        for asset_path in asset_paths {
            let asset = self.resolve_asset(&asset_path, doc_path)?;
            assets.push(asset);
        }

        Ok(assets)
    }

    /// Resolve an asset reference to absolute path and metadata
    fn resolve_asset(&self, asset_path: &str, doc_path: &Path) -> Result<Asset, String> {
        // Check for external URLs
        if asset_path.starts_with("http://") || asset_path.starts_with("https://") {
            return Ok(Asset {
                original_path: asset_path.to_string(),
                resolved_path: None,
                asset_type: AssetType::External,
                mime_type: None,
                content_hash: None,
                is_shared: false,
            });
        }

        // Resolve path
        let resolved = self.resolve_path(asset_path, doc_path)?;

        // Check if file exists
        let full_path = self.repo_root.join(&resolved);
        let (asset_type, mime_type, content_hash) = if full_path.exists() && full_path.is_file() {
            let mime = detect_mime_type(&resolved);
            let hash = compute_file_hash(&full_path).ok();
            (AssetType::Local, mime, hash)
        } else {
            (AssetType::Missing, None, None)
        };

        Ok(Asset {
            original_path: asset_path.to_string(),
            resolved_path: Some(resolved),
            asset_type,
            mime_type,
            content_hash,
            is_shared: false, // Will be updated by classify_assets
        })
    }

    /// Resolve relative path to absolute path from repo root
    ///
    /// Security: Validates that resolved path doesn't escape repo root
    fn resolve_path(&self, asset_path: &str, doc_path: &Path) -> Result<PathBuf, String> {
        let base_path = if let Some(stripped) = asset_path.strip_prefix('/') {
            // Root-relative path: strip leading slash
            PathBuf::from(stripped)
        } else {
            // Relative path: resolve from document's directory
            let doc_dir = doc_path.parent().unwrap_or(Path::new(""));
            doc_dir.join(asset_path)
        };

        // Security check: count parent dir references before normalization
        let parent_count = asset_path.matches("..").count();
        let doc_depth = doc_path
            .parent()
            .unwrap_or(Path::new(""))
            .components()
            .count();

        if !asset_path.starts_with('/') && parent_count > doc_depth {
            return Err(format!(
                "Path escape detected: {} would escape repository root (going up {} levels from depth {})",
                asset_path, parent_count, doc_depth
            ));
        }

        // Normalize path (resolve .. and .)
        let normalized = normalize_path(&base_path);

        Ok(normalized)
    }

    /// Classify assets as shared or per-doc based on reference count
    ///
    /// Assets referenced by multiple documents are marked as shared.
    /// Per-doc assets follow the pattern: `<doc>_assets/...`
    pub fn classify_assets(
        &self,
        assets_by_doc: &HashMap<PathBuf, Vec<Asset>>,
    ) -> HashMap<PathBuf, Vec<Asset>> {
        // Count references to each asset
        let mut reference_counts: HashMap<PathBuf, usize> = HashMap::new();
        for assets in assets_by_doc.values() {
            for asset in assets {
                if let (AssetType::Local, Some(ref path)) =
                    (&asset.asset_type, &asset.resolved_path)
                {
                    *reference_counts.entry(path.clone()).or_insert(0) += 1;
                }
            }
        }

        // Update is_shared flag based on reference counts and path patterns
        let mut classified = HashMap::new();
        for (doc_path, assets) in assets_by_doc {
            let updated_assets = assets
                .iter()
                .map(|asset| {
                    let mut updated = asset.clone();
                    if let (AssetType::Local, Some(ref path)) =
                        (&asset.asset_type, &asset.resolved_path)
                    {
                        let ref_count = reference_counts.get(path).copied().unwrap_or(0);

                        // Shared if referenced by multiple docs
                        let multi_ref = ref_count > 1;

                        // Check if it's in a per-doc assets folder
                        let in_per_doc_folder = path.to_string_lossy().contains("_assets/");

                        // Shared if multi-referenced OR not in per-doc folder
                        updated.is_shared = multi_ref || !in_per_doc_folder;
                    }
                    updated
                })
                .collect();
            classified.insert(doc_path.clone(), updated_assets);
        }

        classified
    }
}

/// Normalize a path by resolving . and .. components
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                components.pop();
            }
            c => components.push(c),
        }
    }
    components.iter().collect()
}

/// Detect MIME type from file extension
fn detect_mime_type(path: &Path) -> Option<String> {
    path.extension().and_then(|ext| {
        let ext_str = ext.to_string_lossy().to_lowercase();
        match ext_str.as_str() {
            "png" => Some("image/png"),
            "jpg" | "jpeg" => Some("image/jpeg"),
            "gif" => Some("image/gif"),
            "svg" => Some("image/svg+xml"),
            "pdf" => Some("application/pdf"),
            "md" | "markdown" => Some("text/markdown"),
            "txt" => Some("text/plain"),
            _ => None,
        }
        .map(String::from)
    })
}

/// Compute SHA256 hash of file content
fn compute_file_hash(path: &Path) -> Result<String, std::io::Error> {
    let content = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&content);
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::AdapterRegistry;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_asset_scanner_scan_document_basic() {
        let temp = TempDir::new().unwrap();
        let registry = AdapterRegistry::with_builtins();
        let scanner = AssetScanner::new(registry, temp.path());

        let content = r#"
# Document

![Logo](./logo.png)
[Guide](../docs/guide.md)
        "#;

        let assets = scanner
            .scan_document(Path::new("dev/active/doc.md"), content)
            .unwrap();
        assert_eq!(assets.len(), 2);

        // Find assets by original path (order may vary)
        let logo = assets
            .iter()
            .find(|a| a.original_path == "./logo.png")
            .unwrap();
        let guide = assets
            .iter()
            .find(|a| a.original_path == "../docs/guide.md")
            .unwrap();

        assert_eq!(logo.original_path, "./logo.png");
        assert_eq!(guide.original_path, "../docs/guide.md");
    }

    #[test]
    fn test_asset_scanner_external_urls() {
        let temp = TempDir::new().unwrap();
        let registry = AdapterRegistry::with_builtins();
        let scanner = AssetScanner::new(registry, temp.path());

        let content = r#"![External](https://example.com/image.png)"#;

        let assets = scanner.scan_document(Path::new("doc.md"), content).unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_type, AssetType::External);
        assert!(assets[0].resolved_path.is_none());
    }

    #[test]
    fn test_asset_scanner_missing_files() {
        let temp = TempDir::new().unwrap();
        let registry = AdapterRegistry::with_builtins();
        let scanner = AssetScanner::new(registry, temp.path());

        let content = r#"![Missing](./missing.png)"#;

        let assets = scanner.scan_document(Path::new("doc.md"), content).unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_type, AssetType::Missing);
        assert!(assets[0].content_hash.is_none());
    }

    #[test]
    fn test_asset_scanner_existing_file() {
        let temp = TempDir::new().unwrap();
        let registry = AdapterRegistry::with_builtins();
        let scanner = AssetScanner::new(registry, temp.path());

        // Create a test file
        let asset_path = temp.path().join("logo.png");
        fs::write(&asset_path, b"fake png content").unwrap();

        let content = r#"![Logo](./logo.png)"#;

        let assets = scanner.scan_document(Path::new("doc.md"), content).unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].asset_type, AssetType::Local);
        assert!(assets[0].content_hash.is_some());
        assert_eq!(assets[0].mime_type, Some("image/png".to_string()));
    }

    #[test]
    fn test_resolve_path_relative() {
        let temp = TempDir::new().unwrap();
        let registry = AdapterRegistry::with_builtins();
        let scanner = AssetScanner::new(registry, temp.path());

        let resolved = scanner
            .resolve_path("./logo.png", Path::new("dev/active/doc.md"))
            .unwrap();
        assert_eq!(resolved, PathBuf::from("dev/active/logo.png"));

        let resolved = scanner
            .resolve_path("../guide.md", Path::new("dev/active/doc.md"))
            .unwrap();
        assert_eq!(resolved, PathBuf::from("dev/guide.md"));
    }

    #[test]
    fn test_resolve_path_root_relative() {
        let temp = TempDir::new().unwrap();
        let registry = AdapterRegistry::with_builtins();
        let scanner = AssetScanner::new(registry, temp.path());

        let resolved = scanner
            .resolve_path("/assets/logo.png", Path::new("dev/doc.md"))
            .unwrap();
        assert_eq!(resolved, PathBuf::from("assets/logo.png"));
    }

    #[test]
    fn test_resolve_path_escape_detection() {
        let temp = TempDir::new().unwrap();
        let registry = AdapterRegistry::with_builtins();
        let scanner = AssetScanner::new(registry, temp.path());

        // Going up 3 levels from dev/active/ (depth 2) would escape
        let result = scanner.resolve_path("../../../etc/passwd", Path::new("dev/active/doc.md"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("escape"));

        // Going up 2 levels from dev/active/ reaches repo root - this is OK
        let result = scanner.resolve_path("../../file.md", Path::new("dev/active/doc.md"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("file.md"));
    }

    #[test]
    fn test_normalize_path() {
        assert_eq!(
            normalize_path(Path::new("./file.md")),
            PathBuf::from("file.md")
        );
        assert_eq!(normalize_path(Path::new("a/./b")), PathBuf::from("a/b"));
        assert_eq!(normalize_path(Path::new("a/b/../c")), PathBuf::from("a/c"));
    }

    #[test]
    fn test_detect_mime_type() {
        assert_eq!(
            detect_mime_type(Path::new("image.png")),
            Some("image/png".to_string())
        );
        assert_eq!(
            detect_mime_type(Path::new("photo.jpg")),
            Some("image/jpeg".to_string())
        );
        assert_eq!(
            detect_mime_type(Path::new("doc.pdf")),
            Some("application/pdf".to_string())
        );
        assert_eq!(detect_mime_type(Path::new("file.unknown")), None);
    }

    #[test]
    fn test_classify_assets_single_reference() {
        let temp = TempDir::new().unwrap();
        let registry = AdapterRegistry::with_builtins();
        let scanner = AssetScanner::new(registry, temp.path());

        let mut assets_by_doc = HashMap::new();
        assets_by_doc.insert(
            PathBuf::from("doc.md"),
            vec![Asset {
                original_path: "./doc_assets/image.png".to_string(),
                resolved_path: Some(PathBuf::from("doc_assets/image.png")),
                asset_type: AssetType::Local,
                mime_type: Some("image/png".to_string()),
                content_hash: None,
                is_shared: false,
            }],
        );

        let classified = scanner.classify_assets(&assets_by_doc);
        let assets = classified.get(&PathBuf::from("doc.md")).unwrap();
        assert!(!assets[0].is_shared);
    }

    #[test]
    fn test_classify_assets_multiple_references() {
        let temp = TempDir::new().unwrap();
        let registry = AdapterRegistry::with_builtins();
        let scanner = AssetScanner::new(registry, temp.path());

        let shared_asset = PathBuf::from("shared/logo.png");
        let mut assets_by_doc = HashMap::new();

        assets_by_doc.insert(
            PathBuf::from("doc1.md"),
            vec![Asset {
                original_path: "/shared/logo.png".to_string(),
                resolved_path: Some(shared_asset.clone()),
                asset_type: AssetType::Local,
                mime_type: Some("image/png".to_string()),
                content_hash: None,
                is_shared: false,
            }],
        );

        assets_by_doc.insert(
            PathBuf::from("doc2.md"),
            vec![Asset {
                original_path: "/shared/logo.png".to_string(),
                resolved_path: Some(shared_asset.clone()),
                asset_type: AssetType::Local,
                mime_type: Some("image/png".to_string()),
                content_hash: None,
                is_shared: false,
            }],
        );

        let classified = scanner.classify_assets(&assets_by_doc);
        let assets1 = classified.get(&PathBuf::from("doc1.md")).unwrap();
        let assets2 = classified.get(&PathBuf::from("doc2.md")).unwrap();

        assert!(assets1[0].is_shared);
        assert!(assets2[0].is_shared);
    }

    #[test]
    fn test_compute_file_hash() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.txt");
        fs::write(&file_path, b"test content").unwrap();

        let hash = compute_file_hash(&file_path).unwrap();
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64); // SHA256 hex string length
    }
}
