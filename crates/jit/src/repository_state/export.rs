//! Typed classification and pure finalization of repository export files.

use super::{
    CaptureBudget, CaptureError, CaptureSpec, ExpectedPreimage, FileMode, MaterializationIntent,
    MaterializationPlan, PlanHashError, RepositoryAction, RepositoryDelta, RepositoryEntry,
    RepositoryImage, RepositoryLayout, RepositoryLayoutError, RepositoryRootClass, RepositorySeed,
    RepositorySeedKind, RootRelativePath, SeedError, VirtualPath,
};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

const EXPORT_OWNER: &str = "repository-export";

/// A lexically normalized output path classified outside both repository roots.
///
/// This type has no public constructor. External persistence boundaries accept it
/// instead of an arbitrary [`Path`], preventing repository-owned exports from
/// accidentally bypassing [`crate::storage::RepositoryStateStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternalExportPath(PathBuf);

impl ExternalExportPath {
    /// Return the normalized external filesystem path.
    pub(crate) fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Classification of one requested export destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepositoryExportDestination {
    /// A canonical Data- or Worktree-owned target.
    Repository(VirtualPath),
    /// A normalized target outside both repository roots.
    External(ExternalExportPath),
}

/// Exact typed input to repository export finalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryExportIntent {
    target: VirtualPath,
    bytes: Vec<u8>,
}

impl RepositoryExportIntent {
    /// Select one canonical repository target and its complete output bytes.
    pub(crate) fn new(target: VirtualPath, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            target,
            bytes: bytes.into(),
        }
    }

    /// Build the bounded capture declaration for this export.
    ///
    /// The exact target, its immediate parent, and the parent's complete
    /// non-recursive listing are revalidated before publication.
    pub(crate) fn capture_spec(
        &self,
        budget: CaptureBudget,
    ) -> Result<CaptureSpec, RepositoryExportError> {
        let parent = immediate_parent(&self.target)?;
        let paths = [self.target.clone(), parent.clone()];
        let fixed = paths
            .iter()
            .filter(|path| path.root_class() == RepositoryRootClass::Data)
            .cloned();
        let mut spec = CaptureSpec::phase_one(fixed, budget)?;
        spec.discover_paths(
            paths
                .into_iter()
                .filter(|path| path.root_class() == RepositoryRootClass::Worktree),
        )?;
        spec.discover_listing(parent)?;
        Ok(spec)
    }
}

/// Export classification or pure finalization failure.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RepositoryExportError {
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error(transparent)]
    Delta(#[from] super::DeltaError),
    #[error(transparent)]
    PlanHash(#[from] PlanHashError),
    #[error(transparent)]
    Seed(#[from] SeedError),
    #[error("repository export target must be below a selected root: {0:?}")]
    RootTarget(VirtualPath),
    #[error("repository export parent is not an ordinary directory: {0:?}")]
    UnsafeParent(VirtualPath),
    #[error("repository export target has an unsafe occupant: {0:?}")]
    UnsafeTarget(VirtualPath),
}

/// Lexically normalize and classify a requested output path.
///
/// Relative paths are resolved against the explicit invocation directory. Data
/// classification is authoritative when the selected data root is nested under
/// the worktree; only `OutsideRepositoryRoots` becomes an external destination.
pub(crate) fn classify_repository_export(
    layout: &RepositoryLayout,
    invocation_dir: &Path,
    requested: &Path,
) -> Result<RepositoryExportDestination, RepositoryExportError> {
    let physical = normalize_absolute(if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        invocation_dir.join(requested)
    })?;
    match layout.classify_and_canonicalize(&physical) {
        Ok(path) => Ok(RepositoryExportDestination::Repository(path)),
        Err(RepositoryLayoutError::OutsideRepositoryRoots(_)) => Ok(
            RepositoryExportDestination::External(ExternalExportPath(physical)),
        ),
        Err(error) => Err(error.into()),
    }
}

/// Finalize one repository-contained export as an exact recoverable plan.
pub(crate) fn finalize_repository_export(
    base: &RepositoryImage,
    intent: &RepositoryExportIntent,
) -> Result<MaterializationPlan, RepositoryExportError> {
    base.layout().ensure_canonical(&intent.target)?;
    let parent = immediate_parent(&intent.target)?;
    if !base.listing_fingerprints().contains_key(&parent) {
        return Err(CaptureError::UndiscoveredRepositoryPath(parent).into());
    }
    if !matches!(base.entry(&parent)?, RepositoryEntry::Directory { .. }) {
        return Err(RepositoryExportError::UnsafeParent(parent));
    }
    let target_entry = base.entry(&intent.target)?;
    let mode = match target_entry {
        RepositoryEntry::Absent => FileMode::Regular,
        RepositoryEntry::File { mode, .. } => *mode,
        _ => return Err(RepositoryExportError::UnsafeTarget(intent.target.clone())),
    };
    let delta = RepositoryDelta::new(
        base.layout(),
        vec![RepositoryAction::WriteFile {
            path: intent.target.clone(),
            owner: EXPORT_OWNER.to_string(),
            expected: ExpectedPreimage::of(target_entry),
            bytes: intent.bytes.clone(),
            mode,
        }],
    )?;
    let seed = RepositorySeed::new(
        RepositorySeedKind::Command {
            name: "repository-export".to_string(),
        },
        BTreeMap::from([
            (
                "target".to_string(),
                serde_json::to_string(&intent.target).map_err(PlanHashError::from)?,
            ),
            ("owner".to_string(), EXPORT_OWNER.to_string()),
        ]),
        BTreeMap::new(),
    )?;
    Ok(MaterializationPlan::new(
        base,
        &seed,
        &MaterializationIntent::RepositoryExport,
        delta,
    )?)
}

fn immediate_parent(path: &VirtualPath) -> Result<VirtualPath, RepositoryExportError> {
    let relative = path.relative().as_path();
    if relative.as_os_str().is_empty() {
        return Err(RepositoryExportError::RootTarget(path.clone()));
    }
    Ok(VirtualPath::from_root(
        path.root_class(),
        RootRelativePath::parse(relative.parent().unwrap_or_else(|| Path::new("")))?,
    )?)
}

fn normalize_absolute(path: PathBuf) -> Result<PathBuf, RepositoryLayoutError> {
    if !path.is_absolute() {
        return Err(RepositoryLayoutError::RelativeRoot(path));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::Normal(value) => {
                if value.to_string_lossy().chars().any(char::is_control) {
                    return Err(RepositoryLayoutError::LexicalEscape(
                        path.to_string_lossy().into_owned(),
                    ));
                }
                normalized.push(value);
            }
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(RepositoryLayoutError::LexicalEscape(
                        path.to_string_lossy().into_owned(),
                    ));
                }
            }
            Component::Prefix(_) => {
                return Err(RepositoryLayoutError::LexicalEscape(
                    path.to_string_lossy().into_owned(),
                ));
            }
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::{EntryIdentity, ListingFingerprint, RepositoryRootEvidence};

    fn layout() -> RepositoryLayout {
        RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "worktree", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap()
    }

    fn identity(name: &str, bytes: &[u8]) -> EntryIdentity {
        EntryIdentity::for_bytes(name, bytes).unwrap()
    }

    fn image(target: VirtualPath, target_entry: RepositoryEntry) -> RepositoryImage {
        let layout = layout();
        let intent = RepositoryExportIntent::new(target.clone(), b"new".to_vec());
        let spec = intent
            .capture_spec(CaptureBudget {
                max_paths: 4,
                max_listings: 1,
                max_bytes: 1024,
                max_depth: 8,
            })
            .unwrap();
        let parent = immediate_parent(&target).unwrap();
        let parent_identity = identity("parent", b"directory");
        RepositoryImage::close(
            layout,
            spec,
            BTreeMap::from([
                (target, target_entry),
                (
                    parent.clone(),
                    RepositoryEntry::Directory {
                        identity: parent_identity.clone(),
                        mode: FileMode::Regular,
                    },
                ),
            ]),
            BTreeMap::from([(
                parent,
                ListingFingerprint::for_directory(parent_identity, BTreeMap::new()).unwrap(),
            )]),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap()
    }

    #[test]
    fn test_classify_repository_export_prefers_data_then_worktree_then_external() {
        let layout = layout();
        assert!(matches!(
            classify_repository_export(&layout, Path::new("/repo"), Path::new(".jit/out.json"))
                .unwrap(),
            RepositoryExportDestination::Repository(path)
                if path == VirtualPath::data("out.json").unwrap()
        ));
        assert!(matches!(
            classify_repository_export(&layout, Path::new("/repo/sub"), Path::new("../out.json"))
                .unwrap(),
            RepositoryExportDestination::Repository(path)
                if path == VirtualPath::worktree("out.json").unwrap()
        ));
        assert!(matches!(
            classify_repository_export(&layout, Path::new("/repo"), Path::new("../out.json"))
                .unwrap(),
            RepositoryExportDestination::External(_)
        ));
    }

    #[test]
    fn test_finalize_repository_export_preserves_existing_mode_and_preimage() {
        let target = VirtualPath::worktree("out.json").unwrap();
        let old = b"old";
        let plan = finalize_repository_export(
            &image(
                target.clone(),
                RepositoryEntry::File {
                    identity: identity("target", old),
                    bytes: old.to_vec(),
                    mode: FileMode::Executable,
                },
            ),
            &RepositoryExportIntent::new(target, b"new".to_vec()),
        )
        .unwrap();
        assert!(matches!(
            &plan.delta().actions()[0],
            RepositoryAction::WriteFile {
                owner,
                expected: ExpectedPreimage::File { .. },
                bytes,
                mode: FileMode::Executable,
                ..
            } if owner == EXPORT_OWNER && bytes == b"new"
        ));
    }

    #[test]
    fn test_finalize_repository_export_rejects_wrong_kind_target() {
        let target = VirtualPath::worktree("out.json").unwrap();
        let result = finalize_repository_export(
            &image(
                target.clone(),
                RepositoryEntry::Symlink {
                    identity: identity("target", b"elsewhere"),
                    target: b"elsewhere".to_vec(),
                    mode: FileMode::Regular,
                },
            ),
            &RepositoryExportIntent::new(target.clone(), b"new".to_vec()),
        );
        assert!(matches!(
            result,
            Err(RepositoryExportError::UnsafeTarget(path)) if path == target
        ));
    }
}
