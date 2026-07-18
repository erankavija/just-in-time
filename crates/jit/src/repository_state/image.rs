//! Closed repository images, bounded capture declarations, and exact deltas.

use super::{RepositoryLayout, RepositoryLayoutError, VirtualPath};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Platform-neutral file mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileMode {
    /// Non-executable ordinary file.
    Regular,
    /// Executable ordinary file.
    Executable,
}

/// Exact identity of captured content or an occupant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntryIdentity {
    /// Boundary-acquired no-follow object identity.
    pub object: String,
    /// SHA-256 of exact bytes or stable occupant metadata.
    pub sha256: String,
    /// Exact byte count where applicable.
    pub byte_size: u64,
}

impl EntryIdentity {
    /// Derive a content identity from boundary-provided object identity and bytes.
    pub fn for_bytes(object: impl Into<String>, bytes: &[u8]) -> Self {
        Self {
            object: object.into(),
            sha256: format!("{:x}", Sha256::digest(bytes)),
            byte_size: bytes.len() as u64,
        }
    }
}

/// One exact captured path state, including typed absence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepositoryEntry {
    /// The captured path was absent.
    Absent,
    /// An ordinary file and its exact bytes.
    File {
        /// Exact entry identity.
        identity: EntryIdentity,
        /// Exact bytes.
        bytes: Vec<u8>,
        /// Normalized mode.
        mode: FileMode,
    },
    /// A directory occupant.
    Directory {
        /// Exact directory identity.
        identity: EntryIdentity,
    },
    /// A symbolic link, never followed.
    Symlink {
        /// Exact link identity.
        identity: EntryIdentity,
        /// Exact link payload.
        target: Vec<u8>,
    },
    /// An occupant unsafe for semantic mutation.
    Unsupported {
        /// Exact occupant identity.
        identity: EntryIdentity,
        /// Stable diagnostic.
        reason: String,
    },
}

impl RepositoryEntry {
    /// Exact identity when an occupant exists.
    pub fn identity(&self) -> Option<&EntryIdentity> {
        match self {
            Self::Absent => None,
            Self::File { identity, .. }
            | Self::Directory { identity }
            | Self::Symlink { identity, .. }
            | Self::Unsupported { identity, .. } => Some(identity),
        }
    }
}

/// Fingerprint of one complete non-recursive directory listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingFingerprint {
    /// Sorted child name to exact child identity; `None` is not representable.
    pub children: BTreeMap<String, EntryIdentity>,
    /// SHA-256 of the boundary's canonical listing serialization.
    pub sha256: String,
}

impl ListingFingerprint {
    /// Construct a deterministic fingerprint.
    pub fn new(children: BTreeMap<String, EntryIdentity>) -> Self {
        let mut hasher = Sha256::new();
        for (name, identity) in &children {
            hash_field(&mut hasher, name.as_bytes());
            hash_field(&mut hasher, identity.object.as_bytes());
            hash_field(&mut hasher, identity.sha256.as_bytes());
            hash_field(&mut hasher, &identity.byte_size.to_be_bytes());
        }
        Self {
            children,
            sha256: format!("{:x}", hasher.finalize()),
        }
    }
}

/// Origin class for immutable pinned-document evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinnedSourceClass {
    /// A Git object database entry.
    GitObject,
    /// Git was unavailable at the boundary.
    GitUnavailable,
}

/// Boundary-acquired pinned document or asset evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedDocumentEvidence {
    /// Requested symbolic or object revision.
    pub requested_revision: String,
    /// Requested repository-relative path.
    pub requested_path: String,
    /// Evidence source class.
    pub source: PinnedSourceClass,
    /// Canonical commit OID when available.
    pub commit_oid: Option<String>,
    /// Blob OID when present.
    pub blob_oid: Option<String>,
    /// Exact object/content identity, including SHA-256 and byte size.
    pub identity: Option<EntryIdentity>,
    /// Exact bytes when present.
    pub bytes: Option<Vec<u8>>,
    /// Stable absence/unavailable reason.
    pub unavailable_reason: Option<String>,
}

/// Source class for linked-worktree fallback evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkedWorktreeSourceClass {
    /// Selected data root.
    LocalData,
    /// Immutable Git HEAD blob.
    GitHead,
    /// Main worktree no-follow file.
    MainWorktree,
    /// No candidate existed.
    Absent,
}

/// Boundary-acquired linked-worktree fallback evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedWorktreeEvidence {
    /// Logical Data path requested.
    pub path: VirtualPath,
    /// Selected fallback source.
    pub source: LinkedWorktreeSourceClass,
    /// Exact identity when present.
    pub identity: Option<EntryIdentity>,
    /// Exact bytes when present.
    pub bytes: Option<Vec<u8>>,
}

/// Explicit bounds for phase-two capture closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureBudget {
    /// Maximum exact paths.
    pub max_paths: usize,
    /// Maximum complete listings.
    pub max_listings: usize,
    /// Maximum combined captured bytes.
    pub max_bytes: u64,
    /// Maximum root-relative depth.
    pub max_depth: usize,
}

/// Normalized two-phase capture request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureSpec {
    fixed: BTreeSet<VirtualPath>,
    discovered: BTreeSet<VirtualPath>,
    listings: BTreeSet<VirtualPath>,
    pinned: BTreeSet<(String, String)>,
    budget: CaptureBudget,
}

impl CaptureSpec {
    /// Begin phase one with fixed declaration paths and explicit bounds.
    pub fn phase_one(
        fixed: impl IntoIterator<Item = VirtualPath>,
        budget: CaptureBudget,
    ) -> Result<Self, CaptureError> {
        let mut spec = Self {
            fixed: fixed.into_iter().collect(),
            discovered: BTreeSet::new(),
            listings: BTreeSet::new(),
            pinned: BTreeSet::new(),
            budget,
        };
        spec.validate()?;
        Ok(spec)
    }

    /// Add paths discovered by parsing already captured bytes.
    pub fn discover_paths(
        &mut self,
        paths: impl IntoIterator<Item = VirtualPath>,
    ) -> Result<(), CaptureError> {
        self.discovered.extend(paths);
        self.validate()
    }

    /// Require a complete non-recursive listing.
    pub fn discover_listing(&mut self, path: VirtualPath) -> Result<(), CaptureError> {
        self.listings.insert(path);
        self.validate()
    }

    /// Require immutable pinned evidence.
    pub fn discover_pinned(
        &mut self,
        revision: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<(), CaptureError> {
        self.pinned.insert((revision.into(), path.into()));
        self.validate()
    }

    /// Every exact path in deterministic work-queue order.
    pub fn paths(&self) -> impl Iterator<Item = &VirtualPath> {
        self.fixed.iter().chain(self.discovered.iter())
    }

    /// Every complete-listing request.
    pub fn listings(&self) -> &BTreeSet<VirtualPath> {
        &self.listings
    }

    /// Capture bounds.
    pub fn budget(&self) -> CaptureBudget {
        self.budget
    }

    fn validate(&mut self) -> Result<(), CaptureError> {
        self.discovered.retain(|path| !self.fixed.contains(path));
        let path_count = self.fixed.len() + self.discovered.len() + self.pinned.len();
        if path_count > self.budget.max_paths {
            return Err(CaptureError::PathBudgetExceeded {
                actual: path_count,
                maximum: self.budget.max_paths,
            });
        }
        if self.listings.len() > self.budget.max_listings {
            return Err(CaptureError::ListingBudgetExceeded {
                actual: self.listings.len(),
                maximum: self.budget.max_listings,
            });
        }
        if let Some(path) = self
            .paths()
            .chain(self.listings.iter())
            .find(|path| path.relative().depth() > self.budget.max_depth)
        {
            return Err(CaptureError::DepthBudgetExceeded(path.clone()));
        }
        Ok(())
    }
}

/// Complete immutable repository image consumed by pure producers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryImage {
    layout: RepositoryLayout,
    spec: CaptureSpec,
    entries: BTreeMap<VirtualPath, RepositoryEntry>,
    listings: BTreeMap<VirtualPath, ListingFingerprint>,
    pinned: BTreeMap<(String, String), PinnedDocumentEvidence>,
    linked_worktree: BTreeMap<VirtualPath, LinkedWorktreeEvidence>,
}

impl RepositoryImage {
    /// Close a capture only when every requested path/listing/evidence is present.
    pub fn close(
        layout: RepositoryLayout,
        spec: CaptureSpec,
        entries: BTreeMap<VirtualPath, RepositoryEntry>,
        listings: BTreeMap<VirtualPath, ListingFingerprint>,
        pinned: BTreeMap<(String, String), PinnedDocumentEvidence>,
        linked_worktree: BTreeMap<VirtualPath, LinkedWorktreeEvidence>,
    ) -> Result<Self, CaptureError> {
        for path in spec.paths() {
            layout.ensure_canonical(path)?;
            if !entries.contains_key(path) {
                return Err(CaptureError::IncompleteCapture(path.clone()));
            }
        }
        for path in spec.listings() {
            layout.ensure_canonical(path)?;
            if !listings.contains_key(path) {
                return Err(CaptureError::IncompleteListing(path.clone()));
            }
        }
        for request in &spec.pinned {
            if !pinned.contains_key(request) {
                return Err(CaptureError::IncompletePinnedEvidence(request.clone()));
            }
        }
        let byte_count = entries
            .values()
            .filter_map(|entry| match entry {
                RepositoryEntry::File { bytes, .. } => Some(bytes.len() as u64),
                _ => None,
            })
            .sum::<u64>();
        if byte_count > spec.budget.max_bytes {
            return Err(CaptureError::ByteBudgetExceeded {
                actual: byte_count,
                maximum: spec.budget.max_bytes,
            });
        }
        Ok(Self {
            layout,
            spec,
            entries,
            listings,
            pinned,
            linked_worktree,
        })
    }

    /// Canonical layout captured by this image.
    pub fn layout(&self) -> &RepositoryLayout {
        &self.layout
    }

    /// Read an exactly captured entry; uncaptured reads are always errors.
    pub fn entry(&self, path: &VirtualPath) -> Result<&RepositoryEntry, CaptureError> {
        self.entries
            .get(path)
            .ok_or_else(|| CaptureError::UndiscoveredRepositoryPath(path.clone()))
    }

    /// Read an ordinary file's exact bytes.
    pub fn file_bytes(&self, path: &VirtualPath) -> Result<Option<&[u8]>, CaptureError> {
        match self.entry(path)? {
            RepositoryEntry::File { bytes, .. } => Ok(Some(bytes)),
            RepositoryEntry::Absent | RepositoryEntry::Directory { .. } => Ok(None),
            RepositoryEntry::Symlink { .. } | RepositoryEntry::Unsupported { .. } => {
                Err(CaptureError::UnsafeCapturedOccupant(path.clone()))
            }
        }
    }
}

/// Typed command/profile input included in a semantic plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositorySeed {
    /// Closed seed source and semantic operation identity.
    pub kind: RepositorySeedKind,
    /// Sorted typed scalar facts.
    pub facts: BTreeMap<String, String>,
    /// Sorted exact immutable payloads.
    pub payloads: BTreeMap<String, Vec<u8>>,
}

/// Closed source class for semantic seed data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositorySeedKind {
    /// An ordinary command mutation.
    Command {
        /// Stable command semantic name.
        name: String,
    },
    /// A fresh repository initialization.
    Initialization,
    /// An embedded profile operation.
    Profile {
        /// Profile identity.
        name: String,
        /// Profile declaration version.
        version: String,
        /// Exact package identity.
        package_hash: String,
    },
}

/// Constrained complete materialization operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterializationIntent {
    /// Full derived-state rebuild after a semantic mutation.
    SemanticMutation,
    /// Render all configured projections selected by declarations.
    RenderConfiguredProjections,
    /// Repair every explicitly owned derived-state drift finding.
    RepairDerivedState,
}

/// Stable ownership identity for a target claim.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TargetClaim {
    /// Canonical target.
    pub target: VirtualPath,
    /// Stable producer/owner identity.
    pub owner: String,
}

/// Exact expected preimage for a delta action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpectedPreimage {
    /// Path must remain absent.
    Absent,
    /// Exact captured occupant identity.
    Present(EntryIdentity),
}

/// One exact repository-state action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepositoryAction {
    /// Create an absent directory.
    CreateDirectory {
        path: VirtualPath,
        owner: String,
        expected: ExpectedPreimage,
    },
    /// Create or replace an ordinary file.
    WriteFile {
        path: VirtualPath,
        owner: String,
        expected: ExpectedPreimage,
        bytes: Vec<u8>,
        mode: FileMode,
    },
    /// Change only an ordinary file's mode.
    SetMode {
        path: VirtualPath,
        owner: String,
        expected: ExpectedPreimage,
        mode: FileMode,
    },
    /// Delete one ordinary file.
    DeleteFile {
        path: VirtualPath,
        owner: String,
        expected: ExpectedPreimage,
    },
}

impl RepositoryAction {
    fn path(&self) -> &VirtualPath {
        match self {
            Self::CreateDirectory { path, .. }
            | Self::WriteFile { path, .. }
            | Self::SetMode { path, .. }
            | Self::DeleteFile { path, .. } => path,
        }
    }
}

/// Sorted, duplicate-free exact repository delta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryDelta {
    actions: Vec<RepositoryAction>,
}

impl RepositoryDelta {
    /// Normalize actions and reject duplicate canonical targets.
    pub fn new(
        layout: &RepositoryLayout,
        mut actions: Vec<RepositoryAction>,
    ) -> Result<Self, DeltaError> {
        for action in &actions {
            layout.ensure_canonical(action.path())?;
        }
        actions.sort_by(|left, right| left.path().cmp(right.path()));
        if let Some(pair) = actions
            .windows(2)
            .find(|pair| pair[0].path() == pair[1].path())
        {
            return Err(DeltaError::DuplicateTarget(pair[0].path().clone()));
        }
        Ok(Self { actions })
    }

    /// Exact normalized actions.
    pub fn actions(&self) -> &[RepositoryAction] {
        &self.actions
    }
}

/// Deterministic plan hash over layout, capture, evidence, seed, intent, and delta.
pub fn plan_hash(
    image: &RepositoryImage,
    seed: &RepositorySeed,
    intent: &MaterializationIntent,
    delta: &RepositoryDelta,
) -> Result<String, PlanHashError> {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, b"jit-repository-plan-v1");
    hash_serialized(&mut hasher, &image.layout)?;
    hash_serialized(&mut hasher, &image.spec)?;
    hash_serialized(&mut hasher, &image.entries.iter().collect::<Vec<_>>())?;
    hash_serialized(&mut hasher, &image.listings.iter().collect::<Vec<_>>())?;
    hash_serialized(&mut hasher, &image.pinned.iter().collect::<Vec<_>>())?;
    hash_serialized(
        &mut hasher,
        &image.linked_worktree.iter().collect::<Vec<_>>(),
    )?;
    hash_serialized(&mut hasher, seed)?;
    hash_serialized(&mut hasher, intent)?;
    hash_serialized(&mut hasher, delta)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn hash_field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn hash_serialized(hasher: &mut Sha256, value: &impl Serialize) -> Result<(), PlanHashError> {
    let bytes = serde_json::to_vec(value)?;
    hash_field(hasher, &bytes);
    Ok(())
}

/// Capture closure failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    #[error("capture requested {actual} paths but the maximum is {maximum}")]
    PathBudgetExceeded { actual: usize, maximum: usize },
    #[error("capture requested {actual} listings but the maximum is {maximum}")]
    ListingBudgetExceeded { actual: usize, maximum: usize },
    #[error("captured {actual} bytes but the maximum is {maximum}")]
    ByteBudgetExceeded { actual: u64, maximum: u64 },
    #[error("capture path {0:?} exceeds the depth budget")]
    DepthBudgetExceeded(VirtualPath),
    #[error("capture did not provide requested path {0:?}")]
    IncompleteCapture(VirtualPath),
    #[error("capture did not provide requested listing {0:?}")]
    IncompleteListing(VirtualPath),
    #[error("capture did not provide pinned evidence {0:?}")]
    IncompletePinnedEvidence((String, String)),
    #[error("pure producer requested undiscovered path {0:?}")]
    UndiscoveredRepositoryPath(VirtualPath),
    #[error("captured path {0:?} has a symlink or unsupported occupant")]
    UnsafeCapturedOccupant(VirtualPath),
}

/// Invalid exact delta.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeltaError {
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    #[error("delta contains duplicate canonical target {0:?}")]
    DuplicateTarget(VirtualPath),
}

/// Failure to serialize the closed typed plan vocabulary.
#[derive(Debug, thiserror::Error)]
#[error("failed to serialize repository plan identity: {0}")]
pub struct PlanHashError(#[from] serde_json::Error);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::{RepositoryRootEvidence, RootRelativePath};

    fn layout() -> RepositoryLayout {
        RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "wt", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap()
    }

    fn image(seed_byte: u8, listing_byte: u8) -> RepositoryImage {
        let path = VirtualPath::data("index.json").unwrap();
        let dir = VirtualPath::Data(RootRelativePath::Descendant("issues".into()));
        let spec = {
            let mut spec = CaptureSpec::phase_one(
                [path.clone()],
                CaptureBudget {
                    max_paths: 8,
                    max_listings: 2,
                    max_bytes: 64,
                    max_depth: 4,
                },
            )
            .unwrap();
            spec.discover_listing(dir.clone()).unwrap();
            spec
        };
        let bytes = vec![seed_byte];
        RepositoryImage::close(
            layout(),
            spec,
            BTreeMap::from([(
                path,
                RepositoryEntry::File {
                    identity: EntryIdentity::for_bytes("index", &bytes),
                    bytes,
                    mode: FileMode::Regular,
                },
            )]),
            BTreeMap::from([(
                dir,
                ListingFingerprint::new(BTreeMap::from([(
                    "one.json".into(),
                    EntryIdentity::for_bytes("issue", &[listing_byte]),
                )])),
            )]),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap()
    }

    #[test]
    fn test_capture_rejects_undiscovered_repository_read() {
        let captured = image(1, 2);
        assert!(matches!(
            captured.entry(&VirtualPath::data("config.toml").unwrap()),
            Err(CaptureError::UndiscoveredRepositoryPath(_))
        ));
    }

    #[test]
    fn test_capture_spec_phase_two_closes_discovered_paths() {
        let fixed = VirtualPath::data("config.toml").unwrap();
        let discovered = VirtualPath::worktree("docs/plan.md").unwrap();
        let mut spec = CaptureSpec::phase_one(
            [fixed.clone()],
            CaptureBudget {
                max_paths: 4,
                max_listings: 1,
                max_bytes: 64,
                max_depth: 4,
            },
        )
        .unwrap();
        spec.discover_paths([discovered.clone()]).unwrap();
        let bytes = b"plan".to_vec();
        let captured = RepositoryImage::close(
            layout(),
            spec,
            BTreeMap::from([
                (fixed, RepositoryEntry::Absent),
                (
                    discovered.clone(),
                    RepositoryEntry::File {
                        identity: EntryIdentity::for_bytes("plan", &bytes),
                        bytes: bytes.clone(),
                        mode: FileMode::Regular,
                    },
                ),
            ]),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(captured.file_bytes(&discovered).unwrap(), Some(&bytes[..]));
    }

    #[test]
    fn test_plan_hash_is_deterministic_and_covers_entry_listing_and_seed() {
        let captured = image(1, 2);
        let seed = RepositorySeed {
            kind: RepositorySeedKind::Command {
                name: "test".into(),
            },
            facts: BTreeMap::from([("key".into(), "value".into())]),
            payloads: BTreeMap::new(),
        };
        let delta = RepositoryDelta::new(&layout(), Vec::new()).unwrap();
        let hash = plan_hash(
            &captured,
            &seed,
            &MaterializationIntent::SemanticMutation,
            &delta,
        )
        .unwrap();
        assert_eq!(
            hash,
            plan_hash(
                &captured,
                &seed,
                &MaterializationIntent::SemanticMutation,
                &delta
            )
            .unwrap()
        );
        assert_ne!(
            hash,
            plan_hash(
                &image(9, 2),
                &seed,
                &MaterializationIntent::SemanticMutation,
                &delta
            )
            .unwrap()
        );
        assert_ne!(
            hash,
            plan_hash(
                &image(1, 9),
                &seed,
                &MaterializationIntent::SemanticMutation,
                &delta
            )
            .unwrap()
        );
        let changed_seed = RepositorySeed {
            kind: RepositorySeedKind::Command {
                name: "changed".into(),
            },
            ..seed.clone()
        };
        assert_ne!(
            hash,
            plan_hash(
                &captured,
                &changed_seed,
                &MaterializationIntent::SemanticMutation,
                &delta
            )
            .unwrap()
        );
    }
}
