use super::{EmbeddedProfilePackage, RegionDeclaration};
use crate::storage::atomic_write::write_file_atomic_bytes_with_permissions;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Platform-neutral file-mode intent carried by a package projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectedFileMode {
    /// Ordinary non-executable file.
    Regular,
    /// Executable file where the host supports Unix executable bits.
    Executable,
}

/// Exact final bytes and mode for one repository-relative target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedFile {
    /// Repository-relative target path.
    pub target: String,
    /// Exact bytes to materialize.
    pub bytes: Vec<u8>,
    /// Platform-neutral mode intent.
    pub mode: ProjectedFileMode,
}

/// Deterministically ordered package file projection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageProjection {
    files: BTreeMap<String, ProjectedFile>,
}

impl PackageProjection {
    pub(crate) fn from_files(files: BTreeMap<String, ProjectedFile>) -> Self {
        Self { files }
    }

    /// Files keyed by repository-relative target path.
    pub fn files(&self) -> &BTreeMap<String, ProjectedFile> {
        &self.files
    }

    /// One projected file by target path.
    pub fn get(&self, target: &str) -> Option<&ProjectedFile> {
        self.files.get(target)
    }

    /// Number of projected targets.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether no targets are projected.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Errors deriving or materializing a package projection.
#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    /// A declared embedded source disappeared after package validation.
    #[error("declared embedded source '{0}' is unavailable")]
    MissingSource(String),
    /// Matching managed-region markers are malformed or ambiguous.
    #[error("managed region '{region_id}' in '{target}' is malformed: {reason}")]
    MalformedRegion {
        /// Target containing the malformed markers.
        target: String,
        /// Stable region identity.
        region_id: String,
        /// Specific marker defect.
        reason: String,
    },
    /// A projected path could not be materialized.
    #[error("failed to materialize projected target '{target}': {source}")]
    Materialize {
        /// Repository-relative projected target.
        target: String,
        /// Filesystem error.
        source: anyhow::Error,
    },
}

/// Render every ordinary asset and managed region in a validated package.
///
/// `existing` contains the current bytes for region targets. Assets are exact
/// package-owned files and therefore do not consult existing bytes. The result
/// is sorted by target path regardless of manifest or map insertion order.
pub fn project_package(
    package: &EmbeddedProfilePackage<'_>,
    existing: &BTreeMap<String, Vec<u8>>,
) -> Result<PackageProjection, ProjectionError> {
    let assets = package.manifest().assets.iter().map(|asset| {
        package
            .source_bytes(&asset.source)
            .ok_or_else(|| ProjectionError::MissingSource(asset.source.clone()))
            .map(|bytes| ProjectedFile {
                target: asset.target.clone(),
                bytes: bytes.to_vec(),
                mode: if asset.executable {
                    ProjectedFileMode::Executable
                } else {
                    ProjectedFileMode::Regular
                },
            })
    });

    let regions = package.manifest().regions.iter().map(|region| {
        let content = package
            .source_bytes(&region.source)
            .ok_or_else(|| ProjectionError::MissingSource(region.source.clone()))?;
        let current = existing.get(&region.target).map(Vec::as_slice);
        render_managed_region(region, current, content).map(|bytes| ProjectedFile {
            target: region.target.clone(),
            bytes,
            mode: ProjectedFileMode::Regular,
        })
    });

    let files = assets
        .chain(regions)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|file| (file.target.clone(), file))
        .collect();
    Ok(PackageProjection::from_files(files))
}

/// Render one exact append-placed managed region.
///
/// An absent target becomes marker/content/marker plus a final newline. An
/// existing unmarked target remains an exact prefix, followed by a deterministic
/// blank-line boundary and the region. Exactly one valid matching marker pair is
/// replaced in place; partial, duplicate, reversed, or nested matching markers
/// are rejected without altering foreign regions.
pub fn render_managed_region(
    region: &RegionDeclaration,
    existing: Option<&[u8]>,
    content: &[u8],
) -> Result<Vec<u8>, ProjectionError> {
    let begin = format!("<!-- jit:{}:begin -->", region.region_id);
    let end = format!("<!-- jit:{}:end -->", region.region_id);
    if !find_all(content, begin.as_bytes()).is_empty()
        || !find_all(content, end.as_bytes()).is_empty()
    {
        return Err(malformed(
            region,
            "managed content contains matching marker",
        ));
    }
    let existing = existing.unwrap_or_default();
    let begin_hits = find_all(existing, begin.as_bytes());
    let end_hits = find_all(existing, end.as_bytes());

    match (begin_hits.as_slice(), end_hits.as_slice()) {
        ([], []) => {
            let mut rendered = existing.to_vec();
            append_region_boundary(&mut rendered);
            append_region(&mut rendered, &begin, content, &end);
            Ok(rendered)
        }
        ([begin_at], [end_at]) if begin_at < end_at => {
            let content_start = begin_at + begin.len();
            let mut rendered = existing[..content_start].to_vec();
            rendered.push(b'\n');
            rendered.extend_from_slice(content);
            if !content.ends_with(b"\n") {
                rendered.push(b'\n');
            }
            rendered.extend_from_slice(&existing[*end_at..]);
            Ok(rendered)
        }
        ([begin_at], [end_at]) if end_at < begin_at => {
            Err(malformed(region, "end marker precedes begin marker"))
        }
        ([], _) => Err(malformed(region, "end marker exists without begin marker")),
        (_, []) => Err(malformed(region, "begin marker exists without end marker")),
        _ => Err(malformed(region, "duplicate matching markers")),
    }
}

/// Materialize a projection into a dedicated output tree.
///
/// The tree is intended for deterministic rendering and byte comparison, not
/// live repository publication. Existing projected targets are replaced.
pub fn write_projection_tree(
    root: &Path,
    projection: &PackageProjection,
) -> Result<(), ProjectionError> {
    for file in projection.files.values() {
        let target = root.join(&file.target);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|source| ProjectionError::Materialize {
                target: file.target.clone(),
                source: source.into(),
            })?;
        }
        write_file_atomic_bytes_with_permissions(
            &target,
            &file.bytes,
            projected_permissions(file.mode),
        )
        .map_err(|source| ProjectionError::Materialize {
            target: file.target.clone(),
            source,
        })?;
    }
    Ok(())
}

fn append_region_boundary(bytes: &mut Vec<u8>) {
    if bytes.is_empty() {
        return;
    }
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    if !bytes.ends_with(b"\n\n") {
        bytes.push(b'\n');
    }
}

fn append_region(bytes: &mut Vec<u8>, begin: &str, content: &[u8], end: &str) {
    bytes.extend_from_slice(begin.as_bytes());
    bytes.push(b'\n');
    bytes.extend_from_slice(content);
    if !content.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(end.as_bytes());
    bytes.push(b'\n');
}

fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return Vec::new();
    }
    haystack
        .windows(needle.len())
        .enumerate()
        .filter_map(|(index, window)| (window == needle).then_some(index))
        .collect()
}

fn malformed(region: &RegionDeclaration, reason: &str) -> ProjectionError {
    ProjectionError::MalformedRegion {
        target: region.target.clone(),
        region_id: region.region_id.clone(),
        reason: reason.to_string(),
    }
}

#[cfg(unix)]
fn projected_permissions(mode: ProjectedFileMode) -> Option<fs::Permissions> {
    use std::os::unix::fs::PermissionsExt;

    Some(fs::Permissions::from_mode(match mode {
        ProjectedFileMode::Regular => 0o644,
        ProjectedFileMode::Executable => 0o755,
    }))
}

#[cfg(not(unix))]
fn projected_permissions(_mode: ProjectedFileMode) -> Option<fs::Permissions> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::RegionPlacement;
    use include_dir::{include_dir, Dir};

    static PACKAGE: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/tests/fixtures/profile-packages/synthetic-valid");

    fn region() -> RegionDeclaration {
        RegionDeclaration {
            source: "regions/agents.md".to_string(),
            target: "AGENTS.md".to_string(),
            region_id: "synthetic-guidance".to_string(),
            placement: RegionPlacement::Append,
        }
    }

    #[test]
    fn test_render_managed_region_absent_and_unmarked_preserve_prefix() {
        let region = region();
        let absent = render_managed_region(&region, None, b"managed").unwrap();
        assert_eq!(
            String::from_utf8(absent).unwrap(),
            "<!-- jit:synthetic-guidance:begin -->\nmanaged\n\
             <!-- jit:synthetic-guidance:end -->\n"
        );

        let original = b"# Guide\n\nHand-authored";
        let rendered = render_managed_region(&region, Some(original), b"managed\n").unwrap();
        assert!(rendered.starts_with(original));
        assert_eq!(
            &rendered[original.len()..],
            b"\n\n<!-- jit:synthetic-guidance:begin -->\nmanaged\n\
              <!-- jit:synthetic-guidance:end -->\n"
        );
    }

    #[test]
    fn test_render_managed_region_replaces_only_matching_region() {
        let region = region();
        let original = b"prefix\n<!-- jit:foreign:begin -->\nkeep\n<!-- jit:foreign:end -->\n\
            <!-- jit:synthetic-guidance:begin -->\nstale\n\
            <!-- jit:synthetic-guidance:end -->\nsuffix\n";
        let rendered = render_managed_region(&region, Some(original), b"fresh").unwrap();
        assert_eq!(
            rendered,
            b"prefix\n<!-- jit:foreign:begin -->\nkeep\n<!-- jit:foreign:end -->\n\
              <!-- jit:synthetic-guidance:begin -->\nfresh\n\
              <!-- jit:synthetic-guidance:end -->\nsuffix\n"
        );
    }

    #[test]
    fn test_render_managed_region_rejects_partial_reversed_and_duplicate_markers() {
        let region = region();
        for malformed in [
            "<!-- jit:synthetic-guidance:begin -->\n",
            "<!-- jit:synthetic-guidance:end -->\n",
            "<!-- jit:synthetic-guidance:end -->\n<!-- jit:synthetic-guidance:begin -->\n",
            "<!-- jit:synthetic-guidance:begin -->\na\n<!-- jit:synthetic-guidance:end -->\n\
             <!-- jit:synthetic-guidance:begin -->\nb\n<!-- jit:synthetic-guidance:end -->\n",
        ] {
            assert!(matches!(
                render_managed_region(&region, Some(malformed.as_bytes()), b"fresh"),
                Err(ProjectionError::MalformedRegion { .. })
            ));
        }
        assert!(matches!(
            render_managed_region(&region, None, b"<!-- jit:synthetic-guidance:begin -->"),
            Err(ProjectionError::MalformedRegion { .. })
        ));
    }

    #[test]
    fn test_project_package_derives_exact_sorted_fixture_targets() {
        let package = EmbeddedProfilePackage::from_dir(&PACKAGE).unwrap();
        let existing = BTreeMap::from([(
            "AGENTS.md".to_string(),
            b"# Existing\n\nKeep this text.\n".to_vec(),
        )]);

        let projection = project_package(&package, &existing).unwrap();

        assert_eq!(
            projection.files().keys().collect::<Vec<_>>(),
            vec![
                &"AGENTS.md".to_string(),
                &"bin/check.sh".to_string(),
                &"docs/workflow.txt".to_string()
            ]
        );
        assert_eq!(
            projection.get("docs/workflow.txt").unwrap().bytes,
            b"Synthetic workflow guidance.\n"
        );
        assert_eq!(
            projection.get("bin/check.sh").unwrap().mode,
            ProjectedFileMode::Executable
        );
        assert_eq!(
            projection.get("AGENTS.md").unwrap().bytes,
            b"# Existing\n\nKeep this text.\n\n<!-- jit:synthetic-guidance:begin -->\n\
              Use the synthetic workflow contract.\n\
              <!-- jit:synthetic-guidance:end -->\n"
        );
    }

    #[test]
    fn test_write_projection_tree_failure_leaves_occupied_target_and_no_temp_file() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("occupied");
        fs::create_dir(&target).unwrap();
        let projection = PackageProjection::from_files(BTreeMap::from([(
            "occupied".to_string(),
            ProjectedFile {
                target: "occupied".to_string(),
                bytes: b"replacement".to_vec(),
                mode: ProjectedFileMode::Regular,
            },
        )]));

        assert!(write_projection_tree(temp.path(), &projection).is_err());

        assert!(target.is_dir());
        assert_eq!(
            fs::read_dir(temp.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>(),
            vec![std::ffi::OsString::from("occupied")]
        );
    }
}
