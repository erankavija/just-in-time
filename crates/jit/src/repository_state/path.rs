//! Canonical repository-root and virtual-path identities.

use crate::domain::relative_path::{enters, RootRelativePath, RootRelativePathError};
use serde::{Deserialize, Serialize, Serializer};
use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};

/// The data root's scratch directory, which holds a mutation's commit point.
///
/// Reserved from every semantic identity: what sits there is engine state
/// mid-publication, never repository content. The data root's ignore file names
/// it so version control never carries a commit point a checkout never made.
pub const RECORD_SCRATCH_DIR: &str = "tmp";

/// The worktree's bootstrap lock file, which serializes every session sharing
/// one worktree over the init publisher's sibling stage.
pub const BOOTSTRAP_LOCK_FILE: &str = ".jit-bootstrap.lock";

/// The data root's repository write lock, the outermost lock a mutation holds.
pub const REPO_WRITE_LOCK_FILE: &str = ".repo-write.lock";

/// The data root's index lock.
pub const INDEX_LOCK_FILE: &str = ".index.lock";

/// The data root's event-log lock.
pub const EVENTS_LOCK_FILE: &str = ".events.lock";

/// Whether `path` names the worktree bootstrap lock, or something below it.
///
/// No mutation may target it. Publication writes a file by staging it and
/// renaming it into place, so such an action replaces the inode every current
/// holder — the mutating session included — holds its `flock` on, and the next
/// acquirer locks the replacement freely while the mutation is still running.
///
/// Unlike [`RECORD_SCRATCH_DIR`] the lock is not reserved from the identity
/// itself: it sits among worktree content, so capture must still be able to name
/// it as an entry it observed.
pub(crate) fn is_bootstrap_lock(path: &VirtualPath) -> bool {
    matches!(path.root_class(), RepositoryRootClass::Worktree)
        && enters(path.relative().as_str(), BOOTSTRAP_LOCK_FILE)
}

/// Selected repository root class for a canonical virtual path.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryRootClass {
    /// Authored worktree root.
    Worktree,
    /// Selected repository data root.
    Data,
}

pub use self::identity::VirtualPath;

/// Whether a path relative to the selected data root changes graph inputs.
pub fn is_graph_relevant_data_path(path: &Path) -> bool {
    if path.is_absolute()
        || path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".tmp"))
    {
        return false;
    }

    path.starts_with(VirtualPath::GATE_RUNS.relative().as_path())
        || (path.parent() == Some(VirtualPath::ISSUES.relative().as_path())
            && path.extension().is_some_and(|ext| ext == "json"))
        || [VirtualPath::INDEX, VirtualPath::GATES, VirtualPath::CONFIG]
            .iter()
            .any(|known| path == known.relative().as_path())
}

/// Sole defining scope of the [`VirtualPath`] identity and of its well-known
/// const paths.
///
/// The struct's fields are declared here and nowhere else, so a `Cow::Borrowed`
/// const identity cannot be spelled outside this module: everywhere else in the
/// crate, including the rest of this file, a struct literal is a privacy error.
/// What the module hands its parent is [`from_parts`](VirtualPath::from_parts),
/// the record constructors, and the two field accessors. `from_parts` returns a
/// `Result`; [`issue_record`](VirtualPath::issue_record),
/// [`gate_run_issue_directory`](VirtualPath::gate_run_issue_directory), and
/// [`gate_run_record`](VirtualPath::gate_run_record) are the infallible derived
/// paths, whose canonical UUID components make their descendants safe by
/// construction. [`archive_evidence_record`](VirtualPath::archive_evidence_record)
/// uses a canonical UUID or a hexadecimal digest, and the gate-verdict paths a
/// lowercase-kebab gate key and a lowercase-hex digest.
///
/// Only this module can write the raw struct literal needed for const identities
/// and the well-formed record descendants. It is therefore kept to the
/// identity's accessors, these constructors, the reserved-namespace guard, the
/// const-construction primitive, and the declaration list that emits every const
/// together with its inventory.
mod identity {
    use super::{RepositoryLayoutError, RepositoryRootClass, RootRelativePath};
    use crate::domain::artifact_plan::{normalize_artifact_path, PlanTarget};
    use crate::domain::identity::{GateKey, GateRunId, IssueId};
    use crate::domain::repository_inputs::InputsDigest;
    use serde::Serialize;
    use sha2::{Digest, Sha256};
    use std::borrow::Cow;

    /// A canonical semantic path qualified by its selected root.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
    pub struct VirtualPath {
        root: RepositoryRootClass,
        relative: RootRelativePath,
    }

    impl VirtualPath {
        /// Address evidence by container id or the digest of a normalized document path.
        pub fn archive_evidence_record(target: &PlanTarget) -> Self {
            let key = match target {
                PlanTarget::Container { id } => format!("container/{id}"),
                PlanTarget::Document { path } => format!(
                    "document/{:x}",
                    Sha256::digest(normalize_artifact_path(path).as_bytes())
                ),
            };
            Self {
                root: RepositoryRootClass::Data,
                relative: Self::ARCHIVE_EVIDENCE
                    .relative()
                    .join(&format!("{key}.json")),
            }
        }

        /// Construct the data-root directory holding one issue's gate runs.
        pub fn gate_run_issue_directory(issue: &IssueId) -> Self {
            Self {
                root: RepositoryRootClass::Data,
                relative: Self::GATE_RUNS.relative().join(issue.as_str()),
            }
        }

        /// Construct the data-root path of the record of `issue`'s run `run`.
        pub fn gate_run_record(issue: &IssueId, run: &GateRunId) -> Self {
            Self {
                root: RepositoryRootClass::Data,
                relative: Self::gate_run_issue_directory(issue)
                    .relative
                    .join(&[run.as_str(), ".json"].concat()),
            }
        }

        /// Construct the data-root directory holding one gate's verdict records.
        pub fn gate_verdict_directory(gate_key: &GateKey) -> Self {
            Self {
                root: RepositoryRootClass::Data,
                relative: Self::GATE_VERDICTS.relative().join(gate_key.as_str()),
            }
        }

        /// Construct the data-root path of the verdict record for `gate_key`
        /// over inputs digesting to `digest`.
        pub fn gate_verdict_record(gate_key: &GateKey, digest: &InputsDigest) -> Self {
            Self {
                root: RepositoryRootClass::Data,
                relative: Self::gate_verdict_directory(gate_key)
                    .relative
                    .join(&[digest.as_str(), ".json"].concat()),
            }
        }

        /// Construct the data-root record path from one canonical UUID component.
        pub fn issue_record(id: &IssueId) -> Self {
            Self {
                root: RepositoryRootClass::Data,
                relative: Self::ISSUES
                    .relative()
                    .clone()
                    .join(&[id.as_str(), ".json"].concat()),
            }
        }

        /// Route caller-supplied parts through canonical and reserved-path checks.
        pub(super) fn from_parts(
            root: RepositoryRootClass,
            relative: RootRelativePath,
        ) -> Result<Self, RepositoryLayoutError> {
            Self { root, relative }.checked()
        }

        /// Selected root class.
        pub fn root_class(&self) -> RepositoryRootClass {
            self.root
        }

        /// Root-relative portion of this identity.
        pub fn relative(&self) -> &RootRelativePath {
            &self.relative
        }

        /// Final path component, when this identity is below its selected root.
        pub fn file_name(&self) -> Option<&str> {
            self.relative().as_path().file_name()?.to_str()
        }

        /// The data root's scratch directory is the one namespace no semantic
        /// identity may name: what sits there is engine state mid-publication,
        /// which no repository path addresses. The worktree has no such
        /// namespace; its one engine-control file sits among repository content
        /// and is refused where mutation targets are validated, not here
        /// ([`is_bootstrap_lock`](super::is_bootstrap_lock)).
        fn checked(self) -> Result<Self, RepositoryLayoutError> {
            let reserved = matches!(self.root, RepositoryRootClass::Data)
                && super::enters(self.relative.as_str(), super::RECORD_SCRATCH_DIR);
            if reserved {
                Err(RepositoryLayoutError::ReservedControlPath(self))
            } else {
                Ok(self)
            }
        }
    }

    /// Const-construct a well-known identity, bypassing the `checked` guard.
    ///
    /// Callers must supply a canonical, non-reserved path. The declared entries do,
    /// which the round-trip test over the emitted inventory proves.
    const fn borrowed_identity(root: RepositoryRootClass, relative: &'static str) -> VirtualPath {
        VirtualPath {
            root,
            relative: RootRelativePath::Descendant(Cow::Borrowed(relative)),
        }
    }

    /// Emit every well-known path const, and the inventory of them, from one
    /// declaration list of `NAME: RootClass = "relative path";` entries.
    macro_rules! declare_well_known_paths {
        ($(
            $(#[$doc:meta])*
            $name:ident: $root:ident = $relative:literal;
        )+) => {
            impl VirtualPath {
                $(
                    $(#[$doc])*
                    pub const $name: Self =
                        borrowed_identity(RepositoryRootClass::$root, $relative);
                )+

                /// Every well-known associated-const path, in declaration order.
                ///
                /// The inventory and the consts are emitted from one declaration,
                /// so the inventory enumerates all of them by construction: a const
                /// absent from it cannot be declared. One test iterates the
                /// inventory and proves each entry canonical and non-reserved by
                /// round-tripping it through the fallible constructor for its root
                /// class, which is what licenses these consts to skip that
                /// constructor.
                pub const ALL_KNOWN: &'static [Self] = &[$(Self::$name,)+];
            }
        };
    }

    declare_well_known_paths! {
        /// The format marker and tombstones under the data root (`.jit/index.json`).
        INDEX: Data = "index.json";
        /// Per-target archive execution evidence.
        ARCHIVE_EVIDENCE: Data = "archive-evidence";
        /// Repository configuration under the data root (`.jit/config.toml`).
        CONFIG: Data = "config.toml";
        /// Gate registry definitions under the data root (`.jit/gates.toml`).
        GATES: Data = "gates.toml";
        /// Graph template registry under the data root (`.jit/templates.toml`).
        TEMPLATES: Data = "templates.toml";
        /// Validation rules under the data root (`.jit/rules.toml`).
        RULES: Data = "rules.toml";
        /// Invariants registry under the data root (`.jit/invariants.toml`).
        INVARIANTS: Data = "invariants.toml";
        /// Append-only event log under the data root (`.jit/events.jsonl`).
        EVENTS: Data = "events.jsonl";
        /// Machine-local worktree identity record of a data root inside a checkout;
        /// a data root outside every checkout holds none.
        WORKTREE_IDENTITY: Data = "worktree.json";
        /// Server process record under the data root.
        SERVER_PID: Data = "server.pid.json";
        /// Server output log under the data root.
        SERVER_LOG: Data = "server.log";
        /// Per-issue record directory under the data root (`.jit/issues`).
        ISSUES: Data = "issues";
        /// Recorded gate-run directory under the data root (`.jit/gate-runs`).
        GATE_RUNS: Data = "gate-runs";
        /// Per-key gate verdict records under the data root
        /// (`.jit/gate-runs/verdicts`).
        GATE_VERDICTS: Data = "gate-runs/verdicts";
        /// JSON Schema directory under the data root (`.jit/schemas`).
        SCHEMAS: Data = "schemas";
        /// Nested configuration directory under the data root (`.jit/config`).
        CONFIG_DIR: Data = "config";
        /// Project-defined gate-preset directory under the data root
        /// (`.jit/config/gate-presets`).
        GATE_PRESETS: Data = "config/gate-presets";
        /// Version-control ignore file under the data root (`.jit/.gitignore`),
        /// naming what version control must not carry from the data root.
        DATA_IGNORE: Data = ".gitignore";
        /// Managed gitattributes file at the worktree root (`.gitattributes`).
        GITATTRIBUTES: Worktree = ".gitattributes";
    }
}

/// Constructors and derived queries over the identity, written without field
/// access so the defining module above stays the only const-capable scope.
impl VirtualPath {
    /// The data root's records: issue records, the event log, gate-run
    /// records, archive evidence, and the index holding the tombstones.
    pub const RECORDS: [Self; 5] = [
        Self::ISSUES,
        Self::EVENTS,
        Self::GATE_RUNS,
        Self::ARCHIVE_EVIDENCE,
        Self::INDEX,
    ];

    /// The declaration files of the declaration set: configuration, gates,
    /// templates, rules, and invariants.
    pub const DECLARATION_FILES: [Self; 5] = [
        Self::CONFIG,
        Self::GATES,
        Self::TEMPLATES,
        Self::RULES,
        Self::INVARIANTS,
    ];

    /// The declaration directories of the declaration set, each declaring
    /// through the `*.json` files its listing names.
    pub const DECLARATION_DIRECTORIES: [Self; 1] = [Self::GATE_PRESETS];

    /// Data-root paths holding history: the event log, the gate-run records, and
    /// the redo scratch directory, which can hold appended event bytes.
    pub fn history_paths() -> [String; 3] {
        [
            Self::EVENTS.relative().as_str().to_owned(),
            Self::GATE_RUNS.relative().as_str().to_owned(),
            RECORD_SCRATCH_DIR.to_owned(),
        ]
    }

    /// Whether profile application must refuse this target as application-owned state.
    pub(crate) fn is_reserved_application_target(&self) -> bool {
        self == &Self::EVENTS
            || (self.root_class() == RepositoryRootClass::Worktree
                && self
                    .relative()
                    .as_str()
                    .split('/')
                    .any(|component| component == ".git"))
    }

    /// Construct a worktree-relative semantic identity.
    pub fn worktree(path: impl AsRef<Path>) -> Result<Self, RepositoryLayoutError> {
        Self::from_parts(
            RepositoryRootClass::Worktree,
            RootRelativePath::parse(path)?,
        )
    }

    /// Construct a data-root-relative semantic identity.
    pub fn data(path: impl AsRef<Path>) -> Result<Self, RepositoryLayoutError> {
        Self::from_parts(RepositoryRootClass::Data, RootRelativePath::parse(path)?)
    }

    /// Construct a schema file path from one validated file-name component.
    pub fn schema(name: impl AsRef<Path>) -> Result<Self, RepositoryLayoutError> {
        let name = name.as_ref();
        let parsed = RootRelativePath::parse(name)?;
        if parsed.depth() != 1 {
            return Err(RepositoryLayoutError::LexicalEscape(
                name.to_string_lossy().into_owned(),
            ));
        }
        Self::from_parts(
            RepositoryRootClass::Data,
            Self::SCHEMAS.relative().clone().join(parsed.as_str()),
        )
    }

    /// Parse a rule schema reference into its validated data-root path.
    pub fn schema_reference(reference: &str) -> Result<Self, RepositoryLayoutError> {
        let reference_path = RootRelativePath::parse(reference)?;
        let Some(name) = reference_path
            .as_str()
            .strip_prefix(Self::SCHEMAS.relative().as_str())
            .and_then(|suffix| suffix.strip_prefix('/'))
        else {
            return Err(RepositoryLayoutError::LexicalEscape(reference.to_owned()));
        };
        if !name.ends_with(".json") {
            return Err(RepositoryLayoutError::LexicalEscape(reference.to_owned()));
        }
        Self::schema(name)
    }

    /// Construct a virtual identity from an explicit selected-root class.
    ///
    /// Every fallible constructor funnels through the same canonicality and
    /// reserved-namespace guard, so live mutation planning, redo-record
    /// decoding, and layout classification all re-apply it instead of accepting a
    /// string path and inferring its root.
    pub(crate) fn from_root(
        root: RepositoryRootClass,
        relative: RootRelativePath,
    ) -> Result<Self, RepositoryLayoutError> {
        Self::from_parts(root, relative)
    }

    /// Canonical descendant directories above this path, nearest root first.
    pub(crate) fn ancestor_directories(&self) -> Result<Vec<Self>, RepositoryLayoutError> {
        let RootRelativePath::Descendant(relative) = self.relative() else {
            return Ok(Vec::new());
        };
        let components = relative.split('/').collect::<Vec<_>>();
        (1..components.len())
            .map(|length| {
                let relative = components[..length].join("/");
                match self.root_class() {
                    RepositoryRootClass::Worktree => Self::worktree(relative),
                    RepositoryRootClass::Data => Self::data(relative),
                }
            })
            .collect()
    }

    /// Re-apply the reserved-namespace guard to an existing identity.
    pub(crate) fn ensure_semantic(&self) -> Result<(), RepositoryLayoutError> {
        Self::from_parts(self.root_class(), self.relative().clone()).map(drop)
    }

    /// The adopter-facing repository-relative spelling of this identity.
    ///
    /// The inverse of [`RepositoryLayout::classify_repository_relative`]:
    /// a data-root identity renders under the logical `.jit/` prefix and every
    /// other identity renders worktree-relative. Both directions live beside the
    /// identity they convert so no consumer maintains its own `.jit` prefix
    /// adapter — profile packages address targets and messages name paths in
    /// exactly this spelling.
    pub fn repository_relative(&self) -> String {
        repository_relative_path(self.root_class(), self.relative())
    }

    /// Field serializer naming this identity by its repository-relative spelling.
    pub(crate) fn serialize_repository_relative<S: Serializer>(
        &self,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.repository_relative())
    }
}

/// The adopter-facing repository-relative spelling of a path below `root`.
///
/// [`VirtualPath::repository_relative`] is this rule applied to a canonical
/// identity. A persisted target carries the same two components without having
/// been re-guarded as an identity, so a consumer naming one spells it through
/// this function rather than through a second `.jit/` prefix adapter.
pub fn repository_relative_path(root: RepositoryRootClass, relative: &RootRelativePath) -> String {
    let relative = relative.as_str();
    match root {
        RepositoryRootClass::Data => format!(".jit/{relative}"),
        RepositoryRootClass::Worktree => relative.to_string(),
    }
}

/// Boundary-acquired, no-follow evidence for one root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepositoryRootEvidence {
    path: PathBuf,
    identity: String,
    symlink_free: bool,
}

impl RepositoryRootEvidence {
    /// Record evidence obtained by a filesystem boundary.
    pub fn new(path: impl Into<PathBuf>, identity: impl Into<String>, symlink_free: bool) -> Self {
        Self {
            path: path.into(),
            identity: identity.into(),
            symlink_free,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), RepositoryLayoutError> {
        validate_root(self)
    }

    /// Lexically normalized absolute root path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Opaque no-follow root identity.
    pub fn identity(&self) -> &str {
        &self.identity
    }
}

/// Proof included in capture and plan identities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct InjectivityProof {
    worktree_identity: String,
    data_identity: String,
    nested_data_relative: Option<RootRelativePath>,
}

/// Canonical relationship between worktree and selected data roots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepositoryLayout {
    worktree: RepositoryRootEvidence,
    data: RepositoryRootEvidence,
    nested_data_relative: Option<RootRelativePath>,
    proof: InjectivityProof,
}

impl RepositoryLayout {
    /// Construct a pure layout from no-follow boundary evidence.
    pub fn new(
        worktree: RepositoryRootEvidence,
        data: RepositoryRootEvidence,
    ) -> Result<Self, RepositoryLayoutError> {
        validate_root(&worktree)?;
        validate_root(&data)?;
        if worktree.path == data.path || worktree.path.starts_with(&data.path) {
            return Err(RepositoryLayoutError::OverlappingRepositoryRoots {
                worktree: worktree.path,
                data: data.path,
            });
        }
        if worktree.identity == data.identity {
            return Err(RepositoryLayoutError::AliasedRepositoryRoots {
                worktree: worktree.path,
                data: data.path,
                identity: worktree.identity,
            });
        }
        let nested_data_relative = data
            .path
            .strip_prefix(&worktree.path)
            .ok()
            .map(RootRelativePath::parse)
            .transpose()?;
        let proof = InjectivityProof {
            worktree_identity: worktree.identity.clone(),
            data_identity: data.identity.clone(),
            nested_data_relative: nested_data_relative.clone(),
        };
        Ok(Self {
            worktree,
            data,
            nested_data_relative,
            proof,
        })
    }

    /// Worktree root.
    pub fn worktree_root(&self) -> &Path {
        self.worktree.path()
    }

    /// Selected data root.
    pub fn data_root(&self) -> &Path {
        self.data.path()
    }

    /// Worktree-relative paths jit writes during a mutation, which no gate
    /// input digest may contain.
    ///
    /// The data-root entries (issue records, the event log, gate-run records,
    /// the index, the fixed lock set, the record scratch directory, and the
    /// worktree identity) appear at the data root's worktree offset, and not at
    /// all when the data root lies outside the worktree. Configuration under
    /// the data root is not in the set, so its edits reach a digest.
    pub(crate) fn tracker_written_paths(&self) -> Vec<RootRelativePath> {
        let records = VirtualPath::RECORDS;
        let worktree_identity = VirtualPath::WORKTREE_IDENTITY;
        let data_entries = records
            .iter()
            .map(|record| record.relative().as_str())
            .chain([
                REPO_WRITE_LOCK_FILE,
                INDEX_LOCK_FILE,
                EVENTS_LOCK_FILE,
                RECORD_SCRATCH_DIR,
                worktree_identity.relative().as_str(),
            ]);
        std::iter::once(RootRelativePath::Descendant(Cow::Borrowed(
            BOOTSTRAP_LOCK_FILE,
        )))
        .chain(
            self.nested_data_relative
                .iter()
                .flat_map(|offset| data_entries.clone().map(|entry| offset.join(entry))),
        )
        .collect()
    }

    /// Boundary identity captured for the worktree root.
    pub(crate) fn worktree_identity(&self) -> &str {
        self.worktree.identity()
    }

    /// Boundary identity captured for the selected data root.
    pub(crate) fn data_identity(&self) -> &str {
        self.data.identity()
    }

    /// Resolve an already-canonical virtual identity.
    pub fn resolve(&self, path: &VirtualPath) -> Result<PathBuf, RepositoryLayoutError> {
        self.ensure_canonical(path)?;
        let root = match path.root_class() {
            RepositoryRootClass::Worktree => &self.worktree.path,
            RepositoryRootClass::Data => &self.data.path,
        };
        Ok(root.join(path.relative().as_path()))
    }

    /// Classify a normalized absolute path, applying Data precedence.
    pub fn classify_and_canonicalize(
        &self,
        physical: impl AsRef<Path>,
    ) -> Result<VirtualPath, RepositoryLayoutError> {
        let physical = lexical_absolute(physical.as_ref())?;
        if let Ok(relative) = physical.strip_prefix(&self.data.path) {
            VirtualPath::data(relative)
        } else if let Ok(relative) = physical.strip_prefix(&self.worktree.path) {
            VirtualPath::worktree(relative)
        } else {
            Err(RepositoryLayoutError::OutsideRepositoryRoots(physical))
        }
    }

    /// Classify an adopter-facing repository-relative spelling.
    ///
    /// Logical `.jit/...` paths always address the selected data root, including
    /// when `JIT_DATA_DIR` selects a disjoint or differently named directory.
    /// Every other spelling addresses the worktree root. Keeping this conversion
    /// on the layout prevents consumers from maintaining competing `.jit` prefix
    /// adapters.
    pub fn classify_repository_relative(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<VirtualPath, RepositoryLayoutError> {
        let original = path.as_ref();
        let original_text = original.to_str().ok_or_else(|| {
            RepositoryLayoutError::LexicalEscape(original.to_string_lossy().into())
        })?;
        let mut normalized = original_text;
        while let Some(rest) = normalized.strip_prefix("./") {
            normalized = rest;
        }
        if normalized.is_empty() {
            return Err(RepositoryLayoutError::LexicalEscape(
                original.display().to_string(),
            ));
        }
        let path = Path::new(normalized);
        let data_prefix = Path::new(".jit");
        let virtual_path = match path.strip_prefix(data_prefix) {
            Ok(relative) if !relative.as_os_str().is_empty() => VirtualPath::data(relative)?,
            Ok(_) => {
                return Err(RepositoryLayoutError::LexicalEscape(
                    original.display().to_string(),
                ));
            }
            Err(_) => VirtualPath::worktree(path)?,
        };
        self.ensure_canonical(&virtual_path)?;
        Ok(virtual_path)
    }

    /// Reject a Worktree spelling beneath a nested data root.
    pub fn ensure_canonical(&self, path: &VirtualPath) -> Result<(), RepositoryLayoutError> {
        path.ensure_semantic()?;
        if path.root_class() == RepositoryRootClass::Worktree {
            if let Some(prefix) = &self.nested_data_relative {
                let prefix = prefix.as_str();
                let relative = path.relative().as_str();
                if relative == prefix || relative.starts_with(&format!("{prefix}/")) {
                    return Err(RepositoryLayoutError::DataRootAlias(path.clone()));
                }
            }
        }
        Ok(())
    }
}

/// Invalid root or virtual-path construction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RepositoryLayoutError {
    /// Lexically distinct roots resolve to the same no-follow boundary object.
    #[error(
        "worktree root '{worktree}' and data root '{data}' share physical identity '{identity}'"
    )]
    AliasedRepositoryRoots {
        /// Selected worktree root.
        worktree: PathBuf,
        /// Selected data root.
        data: PathBuf,
        /// Colliding no-follow boundary identity.
        identity: String,
    },
    /// Roots are equal or data contains worktree.
    #[error("worktree root '{worktree}' and data root '{data}' overlap ambiguously")]
    OverlappingRepositoryRoots { worktree: PathBuf, data: PathBuf },
    /// A Worktree spelling aliases nested Data.
    #[error("virtual path {0:?} aliases the selected data root")]
    DataRootAlias(VirtualPath),
    /// A root contains a symlinked component.
    #[error("repository root '{0}' contains a symlinked component")]
    SymlinkedRoot(PathBuf),
    /// A path is not lexically confined.
    #[error("path '{0}' is not lexically confined")]
    LexicalEscape(String),
    /// A root is not absolute.
    #[error("repository root '{0}' is not absolute")]
    RelativeRoot(PathBuf),
    /// A physical path is outside both roots.
    #[error("path '{0}' is outside both repository roots")]
    OutsideRepositoryRoots(PathBuf),
    /// Semantic state entered a machine-local engine-control path.
    #[error("virtual path {0:?} is reserved for engine control")]
    ReservedControlPath(VirtualPath),
    /// Boundary evidence omitted a stable root identity.
    #[error("repository root '{0}' has an invalid boundary identity")]
    InvalidRootIdentity(PathBuf),
}

impl From<RootRelativePathError> for RepositoryLayoutError {
    fn from(error: RootRelativePathError) -> Self {
        Self::LexicalEscape(error.0)
    }
}

fn validate_root(root: &RepositoryRootEvidence) -> Result<(), RepositoryLayoutError> {
    if root.identity.is_empty() || root.identity.chars().any(char::is_control) {
        return Err(RepositoryLayoutError::InvalidRootIdentity(
            root.path.clone(),
        ));
    }
    if !root.symlink_free {
        return Err(RepositoryLayoutError::SymlinkedRoot(root.path.clone()));
    }
    if lexical_absolute(&root.path)? != root.path {
        return Err(RepositoryLayoutError::LexicalEscape(
            root.path.to_string_lossy().into_owned(),
        ));
    }
    Ok(())
}

fn lexical_absolute(path: &Path) -> Result<PathBuf, RepositoryLayoutError> {
    if !path.is_absolute() {
        return Err(RepositoryLayoutError::RelativeRoot(path.to_path_buf()));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Normal(_) => normalized.push(component.as_os_str()),
            _ => {
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

    #[test]
    fn test_graph_relevant_data_path_includes_storage_records_and_configuration() {
        let paths = [
            VirtualPath::INDEX.relative().as_path().to_path_buf(),
            VirtualPath::GATES.relative().as_path().to_path_buf(),
            VirtualPath::CONFIG.relative().as_path().to_path_buf(),
            VirtualPath::ISSUES.relative().as_path().join("issue.json"),
            VirtualPath::GATE_RUNS.relative().as_path().join("run.json"),
        ];
        assert!(paths.iter().all(|path| is_graph_relevant_data_path(path)));
    }

    #[test]
    fn test_graph_relevant_data_path_excludes_unrelated_and_temporary_files() {
        let paths = [
            VirtualPath::EVENTS.relative().as_path().to_path_buf(),
            VirtualPath::SERVER_PID.relative().as_path().to_path_buf(),
            VirtualPath::SERVER_LOG.relative().as_path().to_path_buf(),
            VirtualPath::ISSUES.relative().as_path().join("issue.tmp"),
            VirtualPath::ISSUES
                .relative()
                .as_path()
                .join("issue.json.tmp"),
            VirtualPath::ISSUES
                .relative()
                .as_path()
                .join("nested/issue.json"),
            PathBuf::from("other/index.json"),
            PathBuf::from("other/gate-runs/run.json"),
        ];
        assert!(paths.iter().all(|path| !is_graph_relevant_data_path(path)));
    }

    fn root(path: &str) -> RepositoryRootEvidence {
        RepositoryRootEvidence::new(path, format!("id:{path}"), true)
    }

    #[test]
    fn test_layout_rejects_overlap_symlink_and_escape() {
        assert!(matches!(
            RepositoryLayout::new(root("/repo"), root("/repo")),
            Err(RepositoryLayoutError::OverlappingRepositoryRoots { .. })
        ));
        assert!(matches!(
            RepositoryLayout::new(root("/repo/worktree"), root("/repo")),
            Err(RepositoryLayoutError::OverlappingRepositoryRoots { .. })
        ));
        assert!(matches!(
            RepositoryLayout::new(
                RepositoryRootEvidence::new("/repo", "id", false),
                root("/data")
            ),
            Err(RepositoryLayoutError::SymlinkedRoot(_))
        ));
        assert!(matches!(
            RepositoryLayout::new(
                RepositoryRootEvidence::new("/repo", "", true),
                root("/data")
            ),
            Err(RepositoryLayoutError::InvalidRootIdentity(_))
        ));
        assert!(matches!(
            RepositoryLayout::new(root("/repo/../escape"), root("/data")),
            Err(RepositoryLayoutError::LexicalEscape(_))
        ));
    }

    #[test]
    fn test_layout_rejects_lexically_disjoint_roots_with_same_identity() {
        assert!(matches!(
            RepositoryLayout::new(
                RepositoryRootEvidence::new("/repo", "same-object", true),
                RepositoryRootEvidence::new("/external/data", "same-object", true),
            ),
            Err(RepositoryLayoutError::AliasedRepositoryRoots {
                worktree,
                data,
                identity,
            }) if worktree == Path::new("/repo")
                && data == Path::new("/external/data")
                && identity == "same-object"
        ));
    }

    #[test]
    fn test_virtual_path_archive_evidence_record_addresses_normalized_targets() {
        use crate::domain::artifact_plan::PlanTarget;
        use sha2::{Digest, Sha256};
        let id = crate::test_utils::issue_id(1);
        assert_eq!(
            VirtualPath::archive_evidence_record(&PlanTarget::Container { id: id.clone() }),
            VirtualPath::data(format!("archive-evidence/container/{id}.json")).unwrap()
        );
        let canonical = "docs/plan.md";
        let expected = VirtualPath::data(format!(
            "archive-evidence/document/{:x}.json",
            Sha256::digest(canonical.as_bytes())
        ))
        .unwrap();
        for path in [
            canonical,
            "./docs/plan.md",
            "docs/sub/../plan.md",
            r"docs\plan.md",
        ] {
            assert_eq!(
                VirtualPath::archive_evidence_record(&PlanTarget::Document { path: path.into() }),
                expected
            );
        }
        assert_ne!(
            VirtualPath::archive_evidence_record(&PlanTarget::Document {
                path: "other/plan.md".into()
            }),
            expected
        );
    }

    #[test]
    fn test_virtual_path_gate_run_record_names_the_run_under_its_issue_directory() {
        let issue = crate::test_utils::issue_id(7);
        let run = crate::test_utils::gate_run_id(1);
        let path = VirtualPath::gate_run_record(&issue, &run);

        assert_eq!(path.root_class(), RepositoryRootClass::Data);
        assert_eq!(
            path.relative().as_str(),
            format!("gate-runs/{issue}/{run}.json")
        );
        assert_eq!(
            path.relative().as_path().parent(),
            Some(
                VirtualPath::gate_run_issue_directory(&issue)
                    .relative()
                    .as_path()
            )
        );
        assert_eq!(
            VirtualPath::data(path.relative().as_path()).unwrap(),
            path,
            "the derived path is the path parsing yields"
        );
    }

    #[test]
    fn test_virtual_path_gate_verdict_record_nests_under_its_gate_in_the_verdicts_directory() {
        let gate_key: crate::domain::identity::GateKey = "cargo-ci".parse().unwrap();
        let digest =
            crate::domain::repository_inputs::InputsDigest::try_from("a".repeat(64)).unwrap();
        let path = VirtualPath::gate_verdict_record(&gate_key, &digest);

        assert_eq!(
            path.ancestor_directories().unwrap(),
            vec![
                VirtualPath::GATE_RUNS,
                VirtualPath::GATE_VERDICTS,
                VirtualPath::gate_verdict_directory(&gate_key),
            ]
        );
        assert_eq!(
            VirtualPath::gate_verdict_directory(&gate_key).file_name(),
            Some(gate_key.as_str())
        );
        assert_eq!(
            path.file_name().and_then(|name| name.strip_suffix(".json")),
            Some(digest.as_str())
        );
        assert_eq!(
            VirtualPath::data(path.relative().as_path()).unwrap(),
            path,
            "the derived path is the path parsing yields"
        );
    }

    #[test]
    fn test_virtual_path_issue_record_uses_data_root_and_canonical_id_leaf() {
        let id = crate::test_utils::issue_id(1);
        let path = VirtualPath::issue_record(&id);

        assert_eq!(path.root_class(), RepositoryRootClass::Data);
        assert_eq!(
            path.relative().as_path().parent(),
            Some(Path::new("issues"))
        );
        assert_eq!(
            path.file_name().and_then(|name| name.strip_suffix(".json")),
            Some(id.as_str())
        );
    }

    #[test]
    fn test_layout_accepts_distinct_identity_external_and_nested_data_roots() {
        let external = RepositoryLayout::new(root("/repo"), root("/external/data")).unwrap();
        assert_eq!(external.worktree_root(), Path::new("/repo"));
        assert_eq!(external.data_root(), Path::new("/external/data"));

        let nested = RepositoryLayout::new(root("/repo"), root("/repo/.jit")).unwrap();
        assert_eq!(
            nested
                .classify_and_canonicalize("/repo/.jit/index.json")
                .unwrap(),
            VirtualPath::data("index.json").unwrap()
        );
    }

    #[test]
    fn test_layout_applies_data_precedence_and_rejects_alias() {
        let layout = RepositoryLayout::new(root("/repo"), root("/repo/.jit")).unwrap();
        assert_eq!(
            layout.classify_and_canonicalize("/repo/.jit/index.json"),
            Ok(VirtualPath::data("index.json").unwrap())
        );
        assert!(matches!(
            layout.resolve(&VirtualPath::worktree(".jit/index.json").unwrap()),
            Err(RepositoryLayoutError::DataRootAlias(_))
        ));
    }

    /// The set follows the selected data root: every data-root entry sits at
    /// the root's worktree offset, and an external data root contributes none.
    #[test]
    fn test_tracker_written_paths_follow_the_selected_data_root() {
        let entries_under = |data: &str| {
            let layout = RepositoryLayout::new(root("/repo"), root(data)).unwrap();
            layout
                .tracker_written_paths()
                .into_iter()
                .filter(|path| path.as_str() != BOOTSTRAP_LOCK_FILE)
                .collect::<Vec<_>>()
        };

        let conventional = entries_under("/repo/.jit");
        let relocated = entries_under("/repo/state/data");
        assert!(!conventional.is_empty());
        assert_eq!(conventional.len(), relocated.len());
        assert!(relocated
            .iter()
            .all(|path| path.as_str().starts_with("state/data/")));
        assert!(relocated.contains(&RootRelativePath::parse("state/data/tmp").unwrap()));
        assert!(
            relocated.contains(&RootRelativePath::parse("state/data/archive-evidence").unwrap())
        );
        assert!(entries_under("/external/data").is_empty());
    }

    /// The bootstrap lock sits at the worktree root wherever the data root is.
    #[test]
    fn test_tracker_written_paths_keep_the_bootstrap_lock_at_the_worktree_root() {
        let bootstrap = RootRelativePath::parse(BOOTSTRAP_LOCK_FILE).unwrap();
        for data in ["/repo/.jit", "/external/data"] {
            let written = RepositoryLayout::new(root("/repo"), root(data))
                .unwrap()
                .tracker_written_paths();
            assert!(written.contains(&bootstrap), "{data}");
        }
    }

    #[test]
    fn test_classify_repository_relative_maps_logical_data_and_worktree_paths() {
        let layout = RepositoryLayout::new(root("/repo"), root("/external/data")).unwrap();

        assert_eq!(
            layout.classify_repository_relative(".jit/index.json"),
            Ok(VirtualPath::data("index.json").unwrap())
        );
        assert_eq!(
            layout.classify_repository_relative("docs/README.md"),
            Ok(VirtualPath::worktree("docs/README.md").unwrap())
        );
        assert_eq!(
            layout.classify_repository_relative("././.jit/index.json"),
            Ok(VirtualPath::data("index.json").unwrap())
        );
        assert!(layout.classify_repository_relative("./").is_err());
        assert!(layout
            .classify_repository_relative(".jit/../outside")
            .is_err());
    }

    #[test]
    fn test_virtual_path_reserves_the_data_scratch_and_nothing_in_the_worktree() {
        // The data root's scratch directory and its subtree are the one namespace
        // no semantic identity may name.
        assert!(VirtualPath::data(RECORD_SCRATCH_DIR).is_err());
        assert!(VirtualPath::data(format!("{RECORD_SCRATCH_DIR}/record.json")).is_err());
        // A longer name that merely starts with it is ordinary.
        assert!(VirtualPath::data(format!("{RECORD_SCRATCH_DIR}orary")).is_ok());
        // The worktree reserves nothing. Its one engine-control file sits among
        // repository content, is observable there, and is refused as a mutation
        // target rather than as an identity.
        assert!(VirtualPath::worktree(BOOTSTRAP_LOCK_FILE).is_ok());
        assert!(VirtualPath::worktree(format!("{BOOTSTRAP_LOCK_FILE}/below")).is_ok());
    }

    #[test]
    fn test_virtual_path_constructors_reject_noncanonical_spellings() {
        let control = format!("a{}b", '\u{0007}');
        let invalid = [
            "../outside",
            "a//b",
            "a/./b",
            "a\\b",
            "C:/outside",
            "a:b",
            control.as_str(),
        ];
        for spelling in invalid {
            assert!(VirtualPath::worktree(spelling).is_err(), "{spelling:?}");
            assert!(VirtualPath::data(spelling).is_err(), "{spelling:?}");
        }
    }

    #[test]
    fn test_schema_path_constructor_validates_one_file_name_component() {
        let path = VirtualPath::schema("body.json").unwrap();
        assert_eq!(path.root_class(), RepositoryRootClass::Data);
        assert_eq!(path.relative().as_str(), "schemas/body.json");

        for invalid in ["", "..", "a/b", "a\\b", "/absolute.json"] {
            assert!(VirtualPath::schema(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn test_schema_reference_constructor_validates_rule_reference() {
        let path = VirtualPath::schema_reference("schemas/body.json").unwrap();
        assert_eq!(path.root_class(), RepositoryRootClass::Data);
        assert_eq!(path.relative().as_str(), "schemas/body.json");

        for invalid in [
            "body.json",
            "schemas/nested/body.json",
            "schemas//body.json",
            "schemas/../body.json",
            "/schemas/body.json",
        ] {
            assert!(
                VirtualPath::schema_reference(invalid).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn test_virtual_path_reserved_application_target_matches_owned_paths_by_component() {
        let reserved = [
            VirtualPath::data("events.jsonl").unwrap(),
            VirtualPath::worktree(".git").unwrap(),
            VirtualPath::worktree("vendor/.git/config").unwrap(),
        ];
        assert!(reserved
            .iter()
            .all(VirtualPath::is_reserved_application_target));

        let ordinary = [
            VirtualPath::data("events.jsonl.old").unwrap(),
            VirtualPath::data("profiles").unwrap(),
            VirtualPath::data("profiles/installed/package.json").unwrap(),
            VirtualPath::data("events.jsonl/child").unwrap(),
            VirtualPath::worktree(".gitignore").unwrap(),
            VirtualPath::worktree("vendor/git/config").unwrap(),
            VirtualPath::data(".git/config").unwrap(),
        ];
        assert!(ordinary
            .iter()
            .all(|path| !path.is_reserved_application_target()));
    }

    #[test]
    fn test_virtual_path_from_root_rejects_the_reserved_namespace() {
        let relative =
            RootRelativePath::parse(format!("{RECORD_SCRATCH_DIR}/record.json")).unwrap();
        assert!(VirtualPath::from_root(RepositoryRootClass::Data, relative).is_err());
    }

    #[test]
    fn test_layout_ensure_canonical_rejects_worktree_under_data_alias() {
        let layout = RepositoryLayout::new(root("/repo"), root("/repo/.jit")).unwrap();
        let issue_path = VirtualPath::issue_record(&crate::test_utils::issue_id(1));
        let path =
            VirtualPath::worktree(format!(".jit/{}", issue_path.relative().as_str())).unwrap();
        let error = layout.ensure_canonical(&path).unwrap_err();
        assert!(error.to_string().contains("aliases the selected data root"));
    }

    fn hash_of<T: std::hash::Hash>(value: &T) -> u64 {
        use std::hash::Hasher;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn test_all_known_consts_round_trip_through_fallible_constructor() {
        // Each const is a `Cow::Borrowed` identity built without the `checked`
        // guard. Reconstructing it through the fallible constructor for its root
        // class proves it canonical and non-reserved (the constructor errors on
        // either), and the borrowed const must equal its owned reconstruction.
        assert!(
            !VirtualPath::ALL_KNOWN.is_empty(),
            "the well-known set must be enumerated"
        );
        for known in VirtualPath::ALL_KNOWN {
            let relative = known.relative().as_str();
            let rebuilt = match known.root_class() {
                RepositoryRootClass::Data => VirtualPath::data(relative),
                RepositoryRootClass::Worktree => VirtualPath::worktree(relative),
            }
            .unwrap_or_else(|error| {
                panic!("well-known path {relative:?} is not canonical/non-reserved: {error}")
            });
            assert_eq!(&rebuilt, known, "const {relative:?} is not canonical");
        }
    }

    #[test]
    fn test_all_known_paths_are_distinct_borrowed_const_identities() {
        // The inventory is emitted from the same declaration as the consts, whose
        // only construction route borrows a `'static` payload. Every entry must
        // therefore be a borrowed const rather than a runtime parse, and two
        // entries must never name the same repository path.
        for known in VirtualPath::ALL_KNOWN {
            assert!(
                matches!(
                    known.relative(),
                    RootRelativePath::Descendant(Cow::Borrowed(_))
                ),
                "well-known path {known:?} is not a borrowed const identity"
            );
        }
        let distinct = VirtualPath::ALL_KNOWN
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            distinct.len(),
            VirtualPath::ALL_KNOWN.len(),
            "the well-known declaration names one repository path twice"
        );
    }

    #[test]
    fn test_virtual_path_file_name_returns_the_final_component() {
        assert_eq!(VirtualPath::SERVER_LOG.file_name(), Some("server.log"));
        assert_eq!(
            VirtualPath::data("nested/file").unwrap().file_name(),
            Some("file")
        );
        assert_eq!(VirtualPath::worktree("").unwrap().file_name(), None);
    }

    #[test]
    fn test_gate_presets_const_nests_inside_the_config_directory_const() {
        // Gate-preset publication creates parents nearest-root-first from these two
        // separately declared consts, so the containment between them is asserted
        // rather than assumed from their spelling.
        assert_eq!(
            VirtualPath::GATE_PRESETS.ancestor_directories().unwrap(),
            vec![VirtualPath::CONFIG_DIR]
        );
        assert_ne!(VirtualPath::CONFIG_DIR, VirtualPath::CONFIG);
    }

    #[test]
    fn test_borrowed_const_and_owned_parse_unify_as_one_key() {
        use std::collections::{BTreeMap, HashMap};

        // A const is `Cow::Borrowed`; the parsed twin is `Cow::Owned`. Preserved
        // `Eq`/`Ord`/`Hash` semantics must make them one key in both an ordered
        // and a hashed map, so const and parsed keys collide in the image.
        let borrowed = VirtualPath::GATES;
        let owned = VirtualPath::data("gates.toml").unwrap();
        assert!(
            matches!(
                borrowed.relative(),
                RootRelativePath::Descendant(Cow::Borrowed(_))
            ),
            "the const must carry a borrowed payload"
        );
        assert!(
            matches!(
                owned.relative(),
                RootRelativePath::Descendant(Cow::Owned(_))
            ),
            "the parse must carry an owned payload"
        );

        assert_eq!(borrowed, owned);
        assert_eq!(hash_of(&borrowed), hash_of(&owned));

        let mut ordered = BTreeMap::new();
        ordered.insert(borrowed.clone(), 1);
        assert_eq!(ordered.get(&owned), Some(&1));
        ordered.insert(owned.clone(), 2);
        assert_eq!(
            ordered.len(),
            1,
            "owned twin must overwrite, not add a slot"
        );

        let mut hashed = HashMap::new();
        hashed.insert(borrowed, 1);
        assert_eq!(hashed.get(&owned), Some(&1));
        hashed.insert(owned, 2);
        assert_eq!(hashed.len(), 1, "owned twin must overwrite, not add a slot");
    }

    #[test]
    fn test_rejected_parse_converts_to_lexical_escape_with_the_input_text() {
        let error = RootRelativePath::parse("a/../b").unwrap_err();
        let layout = RepositoryLayoutError::from(error.clone());
        assert!(matches!(
            &layout,
            RepositoryLayoutError::LexicalEscape(text) if text == "a/../b"
        ));
        assert_eq!(layout.to_string(), error.to_string());
    }
}
