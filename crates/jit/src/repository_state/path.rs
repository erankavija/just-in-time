//! Canonical repository-root and virtual-path identities.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};

/// A normalized path relative to one selected repository root.
///
/// [`Root`](RootRelativePath::Root) denotes the selected root itself;
/// [`Descendant`](RootRelativePath::Descendant) holds a normalized, non-empty
/// path strictly below it (no parent, prefix, control, or alternate-separator
/// component). The root is a distinct variant, never an empty-string sentinel,
/// so root-vs-descendant is a type-level distinction rather than a value test.
///
/// The descendant payload is [`Cow<'static, str>`] so well-known paths can be
/// `Cow::Borrowed` const associated items of [`VirtualPath`], while parsing and
/// deserialization mint `Cow::Owned`. `Cow`'s `Eq`/`Ord`/`Hash` compare and
/// hash by `str` content, so a borrowed const and an owned parse of the same
/// path unify as one map key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RootRelativePath {
    /// The selected repository root itself.
    Root,
    /// A normalized, non-empty path strictly below the selected root.
    Descendant(Cow<'static, str>),
}

impl RootRelativePath {
    /// Parse a root-relative path without consulting the filesystem.
    ///
    /// Empty input denotes the root and yields [`RootRelativePath::Root`] through
    /// its own branch; any other input is validated as a normalized
    /// [`Descendant`](RootRelativePath::Descendant). The root is never minted as
    /// an empty descendant.
    pub fn parse(path: impl AsRef<Path>) -> Result<Self, RepositoryLayoutError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Ok(Self::Root);
        }
        let text = path
            .to_str()
            .ok_or_else(|| RepositoryLayoutError::LexicalEscape(path.to_string_lossy().into()))?;
        if path.is_absolute()
            || text.contains('\\')
            || text.contains(':')
            || text.chars().any(char::is_control)
            || text
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == "..")
            || !path
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(RepositoryLayoutError::LexicalEscape(text.to_owned()));
        }
        Ok(Self::Descendant(Cow::Owned(text.to_owned())))
    }

    /// Return this identity as a relative filesystem path (empty for the root).
    pub fn as_path(&self) -> &Path {
        Path::new(self.as_str())
    }

    /// This identity as a string. The root is the empty string, which is also
    /// its stable serialized wire form.
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Root => "",
            Self::Descendant(text) => text,
        }
    }

    /// Number of path components below the selected root (0 for the root).
    pub fn depth(&self) -> usize {
        self.as_path().components().count()
    }

    /// Whether this identity denotes the selected root itself.
    pub fn is_root(&self) -> bool {
        matches!(self, Self::Root)
    }
}

impl Serialize for RootRelativePath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RootRelativePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let path = String::deserialize(deserializer)?;
        Self::parse(path).map_err(serde::de::Error::custom)
    }
}

/// Selected repository root class for a canonical virtual path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryRootClass {
    /// Authored worktree root.
    Worktree,
    /// Selected repository data root.
    Data,
}

pub use self::identity::VirtualPath;

/// Sole defining scope of the [`VirtualPath`] identity and of its well-known
/// const paths.
///
/// The struct's fields are declared here and nowhere else, so a `Cow::Borrowed`
/// const identity — the one shape that reaches a `VirtualPath` without the
/// fallible guard — cannot be spelled outside this module: everywhere else in the
/// crate, including the rest of this file, a struct literal is a privacy error.
/// What the module hands its parent is [`from_parts`](VirtualPath::from_parts)
/// and the two field accessors. `from_parts` returns a `Result`, so it cannot
/// serve a const initializer, and every identity minted elsewhere has passed
/// `checked`.
///
/// One route survives by construction: a raw struct literal written inside this
/// module, which no Rust construct can deny a type's own defining scope. The
/// module is therefore kept to the identity's accessors, its fallible
/// constructor, the reserved-namespace guard, the const-construction primitive,
/// and the declaration list that emits every const together with its inventory.
mod identity {
    use super::{RepositoryLayoutError, RepositoryRootClass, RootRelativePath};
    use serde::Serialize;
    use std::borrow::Cow;

    /// A canonical semantic path qualified by its selected root.
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
    pub struct VirtualPath {
        root: RepositoryRootClass,
        relative: RootRelativePath,
    }

    impl VirtualPath {
        /// Construct a guarded identity from its parts.
        ///
        /// The only route out of this module to a `VirtualPath`, and fallible by
        /// design: a `Result` cannot initialize a const, so no caller can mint an
        /// unguarded identity and no well-known const can be declared outside the
        /// declaration list below.
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

        fn checked(self) -> Result<Self, RepositoryLayoutError> {
            let path = self.relative.as_str();
            let reserved = match self.root {
                RepositoryRootClass::Worktree => {
                    path == ".jit-bootstrap" || path.starts_with(".jit-bootstrap/")
                }
                RepositoryRootClass::Data => path == "tmp" || path.starts_with("tmp/"),
            };
            if reserved {
                Err(RepositoryLayoutError::ReservedTransactionPath(self))
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
        /// The issue index under the data root (`.jit/index.json`).
        INDEX: Data = "index.json";
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
        /// Per-issue record directory under the data root (`.jit/issues`).
        ISSUES: Data = "issues";
        /// Recorded gate-run directory under the data root (`.jit/gate-runs`).
        GATE_RUNS: Data = "gate-runs";
        /// Applied-profile provenance directory under the data root (`.jit/profiles`).
        PROFILES: Data = "profiles";
        /// JSON Schema directory under the data root (`.jit/schemas`).
        SCHEMAS: Data = "schemas";
        /// Nested configuration directory under the data root (`.jit/config`).
        CONFIG_DIR: Data = "config";
        /// Project-defined gate-preset directory under the data root
        /// (`.jit/config/gate-presets`).
        GATE_PRESETS: Data = "config/gate-presets";
        /// Managed gitattributes file at the worktree root (`.gitattributes`).
        GITATTRIBUTES: Worktree = ".gitattributes";
    }
}

/// Constructors and derived queries over the identity, written without field
/// access so the defining module above stays the only const-capable scope.
impl VirtualPath {
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

    /// Construct a virtual identity from an explicit selected-root class.
    ///
    /// Every fallible constructor funnels through the same canonicality and
    /// reserved-namespace guard, so live mutation planning, storage journal
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
        if let Some(relative) = &nested_data_relative {
            let path = relative.as_str();
            if path == ".jit-bootstrap" || path.starts_with(".jit-bootstrap/") {
                return Err(RepositoryLayoutError::ReservedTransactionPath(
                    VirtualPath::worktree(path)?,
                ));
            }
        }
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

    /// Boundary identity captured for the worktree root.
    pub(crate) fn worktree_identity(&self) -> &str {
        self.worktree.identity()
    }

    /// Boundary identity captured for the selected data root.
    pub(crate) fn data_identity(&self) -> &str {
        self.data.identity()
    }

    /// Deserialize a virtual path through this layout's canonicality check.
    ///
    /// `VirtualPath` deliberately has no ambient `Deserialize` implementation:
    /// Worktree-under-Data alias rejection requires the selected layout.
    pub fn deserialize_virtual_path<'de, D>(&self, deserializer: D) -> Result<VirtualPath, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SerializedVirtualPath {
            root: RepositoryRootClass,
            relative: RootRelativePath,
        }

        let path = SerializedVirtualPath::deserialize(deserializer)?;
        let path =
            VirtualPath::from_root(path.root, path.relative).map_err(serde::de::Error::custom)?;
        self.ensure_canonical(&path)
            .map_err(serde::de::Error::custom)?;
        Ok(path)
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
    /// Semantic state entered a transaction-control namespace.
    #[error("virtual path {0:?} is reserved for transaction control")]
    ReservedTransactionPath(VirtualPath),
    /// Boundary evidence omitted a stable root identity.
    #[error("repository root '{0}' has an invalid boundary identity")]
    InvalidRootIdentity(PathBuf),
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

    fn root(path: &str) -> RepositoryRootEvidence {
        RepositoryRootEvidence::new(path, format!("id:{path}"), true)
    }

    fn deserialize_path(
        layout: &RepositoryLayout,
        value: serde_json::Value,
    ) -> Result<VirtualPath, serde_json::Error> {
        let encoded = serde_json::to_string(&value).unwrap();
        let mut deserializer = serde_json::Deserializer::from_str(&encoded);
        layout.deserialize_virtual_path(&mut deserializer)
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
        assert!(matches!(
            RepositoryLayout::new(root("/repo"), root("/repo/.jit-bootstrap/data")),
            Err(RepositoryLayoutError::ReservedTransactionPath(_))
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
    fn test_virtual_path_rejects_control_namespaces() {
        assert!(VirtualPath::worktree(".jit-bootstrap/tx").is_err());
        assert!(VirtualPath::data("tmp/tx").is_err());
        assert!(VirtualPath::worktree("../outside").is_err());
    }

    #[test]
    fn test_path_constructors_and_serde_reject_noncanonical_spellings() {
        let layout = RepositoryLayout::new(root("/repo"), root("/repo/.jit")).unwrap();
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
            assert!(RootRelativePath::parse(spelling).is_err(), "{spelling:?}");
            let encoded = serde_json::to_string(spelling).unwrap();
            assert!(
                serde_json::from_str::<RootRelativePath>(&encoded).is_err(),
                "{spelling:?}"
            );
            assert!(VirtualPath::worktree(spelling).is_err(), "{spelling:?}");
            let encoded = serde_json::json!({
                "root": "worktree",
                "relative": spelling,
            });
            assert!(deserialize_path(&layout, encoded).is_err(), "{spelling:?}");
        }
    }

    #[test]
    fn test_virtual_path_serde_rejects_reserved_namespaces() {
        let layout = RepositoryLayout::new(root("/repo"), root("/repo/.jit")).unwrap();
        for encoded in [
            serde_json::json!({
                "root": "worktree",
                "relative": ".jit-bootstrap/transaction",
            }),
            serde_json::json!({
                "root": "data",
                "relative": "tmp/transaction",
            }),
        ] {
            assert!(deserialize_path(&layout, encoded).is_err());
        }
    }

    #[test]
    fn test_layout_rejects_deserialized_worktree_under_data_alias() {
        let layout = RepositoryLayout::new(root("/repo"), root("/repo/.jit")).unwrap();
        let encoded = serde_json::json!({
            "root": "worktree",
            "relative": ".jit/issues/one.json",
        });
        let error = deserialize_path(&layout, encoded).unwrap_err();
        assert!(error.to_string().contains("aliases the selected data root"));
    }

    #[test]
    fn test_root_relative_path_uses_variant_not_empty_string_sentinel() {
        // Empty input is the root, built as the distinct `Root` variant, not an
        // empty descendant string; a non-empty path is a `Descendant`.
        assert_eq!(RootRelativePath::parse("").unwrap(), RootRelativePath::Root);
        assert!(RootRelativePath::parse("").unwrap().is_root());
        match RootRelativePath::parse("issues/one.json").unwrap() {
            RootRelativePath::Descendant(text) => assert_eq!(&*text, "issues/one.json"),
            RootRelativePath::Root => panic!("a non-empty path must be a descendant"),
        }
        assert!(!RootRelativePath::parse("issues/one.json")
            .unwrap()
            .is_root());
        // Depth is 0 for the root and the component count for a descendant.
        assert_eq!(RootRelativePath::Root.depth(), 0);
        assert_eq!(RootRelativePath::parse("a/b").unwrap().depth(), 2);
        // The root's stable serialized wire form remains the empty string.
        assert_eq!(
            serde_json::to_string(&RootRelativePath::Root).unwrap(),
            "\"\""
        );
        assert_eq!(
            serde_json::from_str::<RootRelativePath>("\"\"").unwrap(),
            RootRelativePath::Root
        );
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
}
