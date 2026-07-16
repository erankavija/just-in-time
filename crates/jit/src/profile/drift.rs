use super::{PackageProjection, ProjectedFileMode};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Kind of mismatch between a dedicated projection tree and expected output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DriftKind {
    /// Expected target is absent.
    Missing,
    /// Target exists but bytes or executable mode differ.
    Stale,
    /// Tree contains a file not present in the expected projection.
    Extra,
}

/// One deterministic projection drift finding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DriftFinding {
    /// Relative path in the compared tree.
    pub path: String,
    /// Mismatch classification.
    pub kind: DriftKind,
}

/// Errors reading a projection tree for comparison.
#[derive(Debug, thiserror::Error)]
pub enum ProjectionDriftError {
    /// Tree traversal or file read failed.
    #[error("failed to inspect projection tree at '{path}': {source}")]
    Inspect {
        /// Path being inspected.
        path: PathBuf,
        /// Filesystem error.
        source: std::io::Error,
    },
    /// Projection trees must contain ordinary files and directories only.
    #[error("projection tree contains unsupported non-file entry '{0}'")]
    UnsupportedEntry(String),
}

/// Compare a dedicated rendered tree byte-for-byte against a projection.
///
/// Findings are sorted by relative path then kind. The dedicated-tree contract
/// intentionally reports every unprojected file as extra; callers comparing a
/// live repository should first render/copy only the managed target set into a
/// temporary tree.
pub fn compare_projection_tree(
    root: &Path,
    expected: &PackageProjection,
) -> Result<Vec<DriftFinding>, ProjectionDriftError> {
    let actual_paths = collect_files(root)?;
    let expected_paths = expected.files().keys().cloned().collect::<BTreeSet<_>>();
    let mut findings = Vec::new();

    for path in expected_paths.difference(&actual_paths) {
        findings.push(DriftFinding {
            path: path.clone(),
            kind: DriftKind::Missing,
        });
    }
    for path in actual_paths.difference(&expected_paths) {
        findings.push(DriftFinding {
            path: path.clone(),
            kind: DriftKind::Extra,
        });
    }
    for path in actual_paths.intersection(&expected_paths) {
        let expected_file = &expected.files()[path];
        let absolute = root.join(path);
        let bytes = fs::read(&absolute).map_err(|source| ProjectionDriftError::Inspect {
            path: absolute.clone(),
            source,
        })?;
        let stale = bytes != expected_file.bytes || mode_differs(&absolute, expected_file.mode)?;
        if stale {
            findings.push(DriftFinding {
                path: path.clone(),
                kind: DriftKind::Stale,
            });
        }
    }

    findings.sort();
    Ok(findings)
}

fn collect_files(root: &Path) -> Result<BTreeSet<String>, ProjectionDriftError> {
    fn visit(
        root: &Path,
        current: &Path,
        files: &mut BTreeSet<String>,
    ) -> Result<(), ProjectionDriftError> {
        if !current.exists() {
            return Ok(());
        }
        let entries = fs::read_dir(current).map_err(|source| ProjectionDriftError::Inspect {
            path: current.to_path_buf(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| ProjectionDriftError::Inspect {
                path: current.to_path_buf(),
                source,
            })?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|source| ProjectionDriftError::Inspect {
                    path: path.clone(),
                    source,
                })?;
            if kind.is_dir() {
                visit(root, &path, files)?;
            } else if kind.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .expect("visited entries stay beneath root")
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                files.insert(relative);
            } else {
                return Err(ProjectionDriftError::UnsupportedEntry(
                    path.display().to_string(),
                ));
            }
        }
        Ok(())
    }

    let mut files = BTreeSet::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

#[cfg(unix)]
fn mode_differs(path: &Path, expected: ProjectedFileMode) -> Result<bool, ProjectionDriftError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(path)
        .map_err(|source| ProjectionDriftError::Inspect {
            path: path.to_path_buf(),
            source,
        })?
        .permissions()
        .mode();
    let actual = mode & 0o777;
    let expected = match expected {
        ProjectedFileMode::Regular => 0o644,
        ProjectedFileMode::Executable => 0o755,
    };
    Ok(actual != expected)
}

#[cfg(not(unix))]
fn mode_differs(_path: &Path, _expected: ProjectedFileMode) -> Result<bool, ProjectionDriftError> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{ProjectedFile, ProjectedFileMode};
    use std::collections::BTreeMap;

    fn projection() -> PackageProjection {
        PackageProjection::from_files(BTreeMap::from([
            (
                "a.txt".to_string(),
                ProjectedFile {
                    target: "a.txt".to_string(),
                    bytes: b"a".to_vec(),
                    mode: ProjectedFileMode::Regular,
                },
            ),
            (
                "nested/b.sh".to_string(),
                ProjectedFile {
                    target: "nested/b.sh".to_string(),
                    bytes: b"b".to_vec(),
                    mode: ProjectedFileMode::Executable,
                },
            ),
        ]))
    }

    #[test]
    fn test_compare_projection_tree_reports_sorted_missing_stale_and_extra() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a.txt"), b"stale").unwrap();
        fs::write(temp.path().join("extra.txt"), b"extra").unwrap();

        let findings = compare_projection_tree(temp.path(), &projection()).unwrap();
        assert_eq!(
            findings,
            vec![
                DriftFinding {
                    path: "a.txt".to_string(),
                    kind: DriftKind::Stale,
                },
                DriftFinding {
                    path: "extra.txt".to_string(),
                    kind: DriftKind::Extra,
                },
                DriftFinding {
                    path: "nested/b.sh".to_string(),
                    kind: DriftKind::Missing,
                },
            ]
        );
    }

    #[test]
    fn test_write_and_compare_projection_tree_preserves_bytes_and_modes() {
        let temp = tempfile::tempdir().unwrap();
        let projection = projection();

        crate::profile::write_projection_tree(temp.path(), &projection).unwrap();

        assert_eq!(fs::read(temp.path().join("a.txt")).unwrap(), b"a");
        assert_eq!(fs::read(temp.path().join("nested/b.sh")).unwrap(), b"b");
        assert!(compare_projection_tree(temp.path(), &projection)
            .unwrap()
            .is_empty());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let executable_mode = fs::metadata(temp.path().join("nested/b.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(executable_mode & 0o777, 0o755);

            fs::set_permissions(temp.path().join("a.txt"), fs::Permissions::from_mode(0o600))
                .unwrap();
            assert_eq!(
                compare_projection_tree(temp.path(), &projection).unwrap(),
                vec![DriftFinding {
                    path: "a.txt".to_string(),
                    kind: DriftKind::Stale,
                }]
            );
        }
    }
}
