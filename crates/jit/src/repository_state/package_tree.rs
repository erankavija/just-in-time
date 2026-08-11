//! Pure finalization of one whole-tree republication at a repository target.
//!
//! A capture hands over the complete content of a tree and the destination it
//! belongs at; this module turns that into one exact delta. The republication
//! is whole: every declared file is written, and every file the captured image
//! shows beneath the destination that the content does not declare is deleted
//! in the same delta, so content the caller stopped declaring cannot survive
//! into the next publication. The delta is published through the shared
//! recoverable transaction like any other, so a failure leaves the destination
//! as it was rather than losing what was there.
//!
//! The tree's content is opaque here: this module knows paths, bytes, and
//! modes. What decides that content — a package manifest and the repository
//! files it declares — belongs to the caller.

use super::{
    CaptureBudget, CaptureError, CaptureSpec, ExpectedPreimage, FileMode, MaterializationIntent,
    MaterializationPlan, PlanHashError, RepositoryAction, RepositoryDelta, RepositoryEntry,
    RepositoryImage, RepositoryLayoutError, RepositoryRootClass, RepositorySeed,
    RepositorySeedKind, RootRelativePath, SeedError, VirtualPath,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const CAPTURE_OWNER: &str = "package-capture";

/// One file a whole-tree republication publishes, addressed relative to the
/// tree root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedTreeFile {
    /// Tree-relative path of the file.
    pub relative: RootRelativePath,
    /// Exact bytes to publish.
    pub bytes: Vec<u8>,
    /// Mode to publish the file with.
    pub mode: FileMode,
}

/// The complete content one republication publishes at `destination`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageTreeCapture {
    destination: VirtualPath,
    files: Vec<CapturedTreeFile>,
}

/// What a republication does to one path beneath its destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeFileDisposition {
    /// Already exactly what the tree declares, so nothing is published.
    Unchanged,
    /// Absent, so the tree's content is created.
    Create,
    /// Present with other content or mode, so it is replaced.
    Update,
    /// Present and not declared, so it is removed.
    Remove,
}

/// One decision a republication reached about a path beneath its destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeFileOutcome {
    /// The path the decision is about.
    pub path: VirtualPath,
    /// What the delta does to it.
    pub disposition: TreeFileDisposition,
    /// Mode the path carries after publication; a removal keeps the mode it had.
    pub mode: FileMode,
}

/// Why a whole-tree republication could not be declared or finalized.
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
    /// The complete plan could not be hashed.
    #[error(transparent)]
    PlanHash(#[from] PlanHashError),
    /// The closed semantic seed was invalid.
    #[error(transparent)]
    Seed(#[from] SeedError),
    /// The destination names a repository root rather than a directory in it.
    #[error("package tree destination {0:?} is a repository root")]
    RootDestination(VirtualPath),
    /// The destination is not worktree content.
    #[error("package tree destination '{0}' is not inside the repository worktree")]
    DestinationOutsideWorktree(String),
    /// A path the tree needs as a directory is occupied by something else.
    #[error("package tree path '{0}' is occupied by an entry that is not a directory")]
    OccupiedDirectory(String),
    /// The destination holds an entry a republication may neither publish over
    /// nor remove.
    #[error("package tree destination holds '{0}', which is not an ordinary file or directory")]
    UnsafeOccupant(String),
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

    /// Where the tree is published.
    pub fn destination(&self) -> &VirtualPath {
        &self.destination
    }

    /// The bounded capture declaration this republication starts from.
    ///
    /// It reads the destination, every directory above it, every path the tree
    /// writes together with the directories those need, and the destination's
    /// own listing. What already sits deeper inside the destination is
    /// discovered from that listing by
    /// [`expand_destination_closure`](Self::expand_destination_closure), which
    /// is what makes the republication whole rather than additive.
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
        // Every path here is worktree content, which phase one never reads, so
        // the whole set is discovered rather than fixed.
        let mut spec = CaptureSpec::phase_one(std::iter::empty(), budget)?;
        spec.discover_paths(paths)?;
        spec.discover_listing(self.destination.clone())?;
        Ok(spec)
    }

    /// Extend `spec` with everything `image` shows beneath the destination that
    /// `spec` does not already declare, and report whether it grew.
    ///
    /// A listing names its children; a child the image shows to be a directory
    /// needs a listing of its own. One pass therefore reaches one level deeper,
    /// and a caller repeats until nothing is added, at which point the
    /// declaration covers the whole destination subtree. Every path it adds is
    /// read and revalidated by the same session, so a file that appears beneath
    /// the destination between two attempts is a retryable conflict rather than
    /// content that survives the republication.
    pub fn expand_destination_closure(
        &self,
        image: &RepositoryImage,
        spec: &mut CaptureSpec,
    ) -> Result<bool, PackageTreeCaptureError> {
        let listed = image
            .listing_fingerprints()
            .iter()
            .filter(|(path, _)| *path == &self.destination || self.contains(path))
            .flat_map(|(path, listing)| {
                listing
                    .children()
                    .keys()
                    .map(|name| child_path(path, name))
                    .collect::<Vec<_>>()
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let directories = image
            .entries()
            .iter()
            .filter(|(path, entry)| {
                self.contains(path) && matches!(entry, RepositoryEntry::Directory { .. })
            })
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();

        let before = (spec.paths().count(), spec.listings().len());
        spec.discover_paths(listed)?;
        for directory in directories {
            spec.discover_listing(directory)?;
        }
        Ok((spec.paths().count(), spec.listings().len()) != before)
    }

    /// Whether `path` sits strictly beneath the destination.
    fn contains(&self, path: &VirtualPath) -> bool {
        path.root_class() == self.destination.root_class()
            && path
                .relative()
                .as_path()
                .starts_with(self.destination.relative().as_path())
            && path != &self.destination
    }

    /// The repository identity of one tree-relative path.
    fn tree_path(&self, relative: &RootRelativePath) -> Result<VirtualPath, RepositoryLayoutError> {
        VirtualPath::from_root(
            self.destination.root_class(),
            RootRelativePath::parse(self.destination.relative().as_path().join(relative.as_path()))?,
        )
    }
}

/// Finalize one whole-tree republication as an exact recoverable plan, with the
/// decision it reached about every path beneath the destination.
///
/// The delta creates every directory the tree needs and is absent, publishes
/// every declared file whose content or mode differs from what the image shows,
/// and deletes every ordinary file the image shows beneath the destination that
/// the tree does not declare. A file already exactly as declared is reported
/// [`TreeFileDisposition::Unchanged`] and carries no action, so republishing an
/// unchanged tree publishes nothing. A directory the tree no longer needs is
/// left in place: the repository transaction publishes files, and an empty
/// directory carries no content the tree declares.
///
/// # Errors
///
/// [`PackageTreeCaptureError::OccupiedDirectory`] when a path the tree needs as
/// a directory holds something else; [`PackageTreeCaptureError::UnsafeOccupant`]
/// when the destination holds an entry that is neither an ordinary file nor a
/// directory, which a republication may neither publish over nor remove; the
/// capture, delta, seed, and plan-hash failures of the shared plan boundary.
pub fn finalize_package_tree_capture(
    base: &RepositoryImage,
    capture: &PackageTreeCapture,
) -> Result<(MaterializationPlan, Vec<TreeFileOutcome>), PackageTreeCaptureError> {
    let published = capture
        .files
        .iter()
        .map(|file| Ok((capture.tree_path(&file.relative)?, file)))
        .collect::<Result<BTreeMap<_, _>, RepositoryLayoutError>>()?;

    // Nearest root first, so each parent is created before its child.
    let mut directories = capture.destination.ancestor_directories()?;
    directories.push(capture.destination.clone());
    directories.extend(
        published
            .keys()
            .map(VirtualPath::ancestor_directories)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .filter(|path| capture.contains(path))
            .collect::<BTreeSet<_>>(),
    );

    let created = directories
        .into_iter()
        .map(|path| match base.entry(&path)? {
            RepositoryEntry::Absent => Ok(Some(RepositoryAction::CreateDirectory {
                path,
                owner: CAPTURE_OWNER.to_string(),
                expected: ExpectedPreimage::Absent,
            })),
            RepositoryEntry::Directory { .. } => Ok(None),
            _ => Err(PackageTreeCaptureError::OccupiedDirectory(
                path.repository_relative(),
            )),
        })
        .collect::<Result<Vec<_>, PackageTreeCaptureError>>()?;

    let writes = published
        .iter()
        .map(|(path, file)| {
            let entry = base.entry(path)?;
            let disposition = match entry {
                RepositoryEntry::Absent => TreeFileDisposition::Create,
                RepositoryEntry::File { bytes, mode, .. }
                    if bytes == &file.bytes && mode == &file.mode =>
                {
                    TreeFileDisposition::Unchanged
                }
                RepositoryEntry::File { .. } => TreeFileDisposition::Update,
                RepositoryEntry::Directory { .. } => {
                    return Err(PackageTreeCaptureError::OccupiedDirectory(
                        path.repository_relative(),
                    ))
                }
                _ => {
                    return Err(PackageTreeCaptureError::UnsafeOccupant(
                        path.repository_relative(),
                    ))
                }
            };
            let action = (disposition != TreeFileDisposition::Unchanged).then(|| {
                RepositoryAction::WriteFile {
                    path: (*path).clone(),
                    owner: CAPTURE_OWNER.to_string(),
                    expected: ExpectedPreimage::of(entry),
                    bytes: file.bytes.clone(),
                    mode: file.mode,
                }
            });
            Ok((
                action,
                TreeFileOutcome {
                    path: (*path).clone(),
                    disposition,
                    mode: file.mode,
                },
            ))
        })
        .collect::<Result<Vec<_>, PackageTreeCaptureError>>()?;

    let removals = base
        .entries()
        .iter()
        .filter(|(path, _)| capture.contains(path) && !published.contains_key(*path))
        .map(|(path, entry)| match entry {
            RepositoryEntry::Absent | RepositoryEntry::Directory { .. } => Ok(None),
            RepositoryEntry::File { mode, .. } => Ok(Some((
                RepositoryAction::DeleteFile {
                    path: path.clone(),
                    owner: CAPTURE_OWNER.to_string(),
                    expected: ExpectedPreimage::of(entry),
                },
                TreeFileOutcome {
                    path: path.clone(),
                    disposition: TreeFileDisposition::Remove,
                    mode: *mode,
                },
            ))),
            _ => Err(PackageTreeCaptureError::UnsafeOccupant(
                path.repository_relative(),
            )),
        })
        .collect::<Result<Vec<_>, PackageTreeCaptureError>>()?;

    let (removal_actions, removal_outcomes): (Vec<_>, Vec<_>) =
        removals.into_iter().flatten().unzip();
    let (write_actions, write_outcomes): (Vec<_>, Vec<_>) = writes.into_iter().unzip();
    let outcomes = write_outcomes
        .into_iter()
        .chain(removal_outcomes)
        .collect::<Vec<_>>();
    let actions = created
        .into_iter()
        .flatten()
        .chain(write_actions.into_iter().flatten())
        .chain(removal_actions)
        .collect::<Vec<_>>();
    let delta = RepositoryDelta::new(base.layout(), actions)?;
    let seed = RepositorySeed::new(
        RepositorySeedKind::Command {
            name: "package-tree-capture".to_string(),
        },
        BTreeMap::from([
            (
                "destination".to_string(),
                serde_json::to_string(&capture.destination).map_err(PlanHashError::from)?,
            ),
            ("owner".to_string(), CAPTURE_OWNER.to_string()),
        ]),
        BTreeMap::new(),
    )?;
    Ok((
        MaterializationPlan::new(
            base,
            &seed,
            &MaterializationIntent::CapturePackageTree,
            delta,
        )?,
        outcomes,
    ))
}

/// The repository identity of one named child of a captured directory.
fn child_path(directory: &VirtualPath, name: &str) -> Result<VirtualPath, RepositoryLayoutError> {
    VirtualPath::from_root(
        directory.root_class(),
        RootRelativePath::parse(directory.relative().as_path().join(Path::new(name)))?,
    )
}
