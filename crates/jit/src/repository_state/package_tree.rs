//! Pure finalization of one package tree's publication at an absent destination.
//!
//! A capture hands over the complete content of a tree and the destination it
//! belongs at; this module turns that into one exact delta, published through
//! one redo record like any other, so the destination ends either absent or
//! holding the whole tree.
//!
//! The tree's content is opaque here: this module knows paths, bytes, and
//! modes. What decides that content — a package manifest and the repository
//! files it declares — belongs to the caller.

use super::{
    CaptureBudget, CaptureError, CaptureSpec, ExpectedPreimage, FileMode, MaterializationPlan,
    MutationContext, RepositoryAction, RepositoryActionOwner, RepositoryDelta, RepositoryEntry,
    RepositoryImage, RepositoryLayoutError, RepositoryRootClass, VirtualPath,
};
use crate::domain::relative_path::RootRelativePath;
use std::collections::BTreeSet;

/// One file a tree publication writes, addressed relative to the tree root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedTreeFile {
    /// Tree-relative path of the file.
    pub relative: RootRelativePath,
    /// Exact bytes to publish.
    pub bytes: Vec<u8>,
    /// Mode to publish the file with.
    pub mode: FileMode,
}

/// The complete content one publication writes at `destination`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageTreeCapture {
    destination: VirtualPath,
    files: Vec<CapturedTreeFile>,
}

/// Why a tree publication could not be declared or finalized.
#[derive(Debug, thiserror::Error)]
pub enum PackageTreeCaptureError {
    /// Path construction or canonicality failed.
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    /// The bounded capture declaration was refused.
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// Delta normalization refused the produced actions.
    #[error(transparent)]
    Delta(#[from] super::DeltaError),
    /// The destination names a repository root rather than a directory in it.
    #[error("package tree destination {0:?} is a repository root")]
    RootDestination(VirtualPath),
    /// The destination is not worktree content.
    #[error("package tree destination '{0}' is not inside the repository worktree")]
    DestinationOutsideWorktree(String),
    /// A path the tree needs as a directory is occupied by something else.
    #[error("package tree path '{0}' is occupied by an entry that is not a directory")]
    OccupiedDirectory(String),
    /// Composing the publication's event append failed.
    #[error(transparent)]
    Mutation(#[from] super::MutationError),
}

impl PackageTreeCapture {
    /// Declare the complete content to publish at `destination`.
    ///
    /// The destination must be worktree content: a package tree is repository
    /// material an adopter reads and commits, not engine state, so the separate
    /// data root and anything outside the repository are refused here rather
    /// than at publication.
    ///
    /// # Errors
    ///
    /// [`PackageTreeCaptureError::DestinationOutsideWorktree`] when the
    /// destination is not worktree content;
    /// [`PackageTreeCaptureError::RootDestination`] when it names the worktree
    /// root itself, which has no parent to publish into.
    pub fn new(
        destination: VirtualPath,
        files: Vec<CapturedTreeFile>,
    ) -> Result<Self, PackageTreeCaptureError> {
        if destination.root_class() != RepositoryRootClass::Worktree {
            return Err(PackageTreeCaptureError::DestinationOutsideWorktree(
                destination.repository_relative(),
            ));
        }
        if destination.relative().is_root() {
            return Err(PackageTreeCaptureError::RootDestination(destination));
        }
        Ok(Self { destination, files })
    }

    /// The bounded capture declaration this publication reads: the destination,
    /// every directory above it, every path the tree writes together with the
    /// directories those need, and the event log's tail its record appends to.
    pub fn capture_spec(
        &self,
        budget: CaptureBudget,
    ) -> Result<CaptureSpec, PackageTreeCaptureError> {
        let mut paths = BTreeSet::from([self.destination.clone()]);
        paths.extend(self.destination.ancestor_directories()?);
        for file in &self.files {
            let path = self.tree_path(&file.relative)?;
            paths.extend(path.ancestor_directories()?);
            paths.insert(path);
        }
        let mut spec = CaptureSpec::new(paths, budget)?;
        spec.discover_tail(VirtualPath::EVENTS)?;
        Ok(spec)
    }

    /// The repository identity of one tree-relative path.
    fn tree_path(&self, relative: &RootRelativePath) -> Result<VirtualPath, RepositoryLayoutError> {
        VirtualPath::from_root(
            self.destination.root_class(),
            RootRelativePath::parse(
                self.destination
                    .relative()
                    .as_path()
                    .join(relative.as_path()),
            )?,
        )
    }
}

/// Finalize one tree publication as an exact recoverable plan.
///
/// `base` shows the destination absent. The delta creates every directory the
/// tree needs and writes every declared file.
///
/// # Errors
///
/// [`PackageTreeCaptureError::OccupiedDirectory`] when a directory above the
/// destination holds something else; the capture and delta failures of the
/// shared plan boundary.
pub fn finalize_package_tree_capture(
    base: &RepositoryImage,
    context: &MutationContext,
    capture: &PackageTreeCapture,
) -> Result<MaterializationPlan, PackageTreeCaptureError> {
    let written = capture
        .files
        .iter()
        .map(|file| Ok((capture.tree_path(&file.relative)?, file)))
        .collect::<Result<Vec<_>, RepositoryLayoutError>>()?;

    let directories = written
        .iter()
        .map(|(path, _)| path.ancestor_directories())
        .chain([capture.destination.ancestor_directories()])
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .chain([capture.destination.clone()])
        .collect::<BTreeSet<_>>();

    let created = directories
        .into_iter()
        .map(|path| match base.entry(&path)? {
            RepositoryEntry::Absent => Ok(Some(RepositoryAction::CreateDirectory {
                path,
                owner: RepositoryActionOwner::PackageCapture,
                expected: ExpectedPreimage::Absent,
            })),
            RepositoryEntry::Directory { .. } => Ok(None),
            _ => Err(PackageTreeCaptureError::OccupiedDirectory(
                path.repository_relative(),
            )),
        })
        .collect::<Result<Vec<_>, PackageTreeCaptureError>>()?;

    let actions = created
        .into_iter()
        .flatten()
        .chain(
            written
                .into_iter()
                .map(|(path, file)| RepositoryAction::WriteFile {
                    path,
                    owner: RepositoryActionOwner::PackageCapture,
                    expected: ExpectedPreimage::Absent,
                    bytes: file.bytes.clone(),
                    mode: file.mode,
                }),
        )
        .collect::<Vec<_>>();
    let delta = RepositoryDelta::new(base.layout(), actions)?;
    Ok(MaterializationPlan::new(base, delta, context)?)
}
