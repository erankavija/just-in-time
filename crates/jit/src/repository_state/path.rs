//! Canonical repository-root and virtual-path identities.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::path::{Component, Path, PathBuf};

/// A normalized path relative to one selected repository root.
///
/// [`Root`](RootRelativePath::Root) denotes the selected root itself;
/// [`Descendant`](RootRelativePath::Descendant) holds a normalized, non-empty
/// path strictly below it (no parent, prefix, control, or alternate-separator
/// component). The root is a distinct variant, never an empty-string sentinel,
/// so root-vs-descendant is a type-level distinction rather than a value test.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RootRelativePath {
    /// The selected repository root itself.
    Root,
    /// A normalized, non-empty path strictly below the selected root.
    Descendant(String),
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
        Ok(Self::Descendant(text.to_owned()))
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

/// A canonical semantic path qualified by its selected root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct VirtualPath {
    root: RepositoryRootClass,
    relative: RootRelativePath,
}

impl VirtualPath {
    /// Construct a worktree-relative semantic identity.
    pub fn worktree(path: impl AsRef<Path>) -> Result<Self, RepositoryLayoutError> {
        Self::new(
            RepositoryRootClass::Worktree,
            RootRelativePath::parse(path)?,
        )
    }

    /// Construct a data-root-relative semantic identity.
    pub fn data(path: impl AsRef<Path>) -> Result<Self, RepositoryLayoutError> {
        Self::new(RepositoryRootClass::Data, RootRelativePath::parse(path)?)
    }

    fn new(
        root: RepositoryRootClass,
        relative: RootRelativePath,
    ) -> Result<Self, RepositoryLayoutError> {
        Self { root, relative }.checked()
    }

    /// Construct a virtual identity from an explicit selected-root class.
    ///
    /// Storage journal decoding uses this constructor so recovery re-applies the
    /// same canonicality checks as live mutation planning instead of accepting a
    /// string path and inferring its root.
    pub(crate) fn from_root(
        root: RepositoryRootClass,
        relative: RootRelativePath,
    ) -> Result<Self, RepositoryLayoutError> {
        Self::new(root, relative)
    }

    /// Selected root class.
    pub fn root_class(&self) -> RepositoryRootClass {
        self.root
    }

    /// Root-relative portion of this identity.
    pub fn relative(&self) -> &RootRelativePath {
        &self.relative
    }

    /// Canonical descendant directories above this path, nearest root first.
    pub(crate) fn ancestor_directories(&self) -> Result<Vec<Self>, RepositoryLayoutError> {
        let RootRelativePath::Descendant(relative) = &self.relative else {
            return Ok(Vec::new());
        };
        let components = relative.split('/').collect::<Vec<_>>();
        (1..components.len())
            .map(|length| {
                let relative = components[..length].join("/");
                match self.root {
                    RepositoryRootClass::Worktree => Self::worktree(relative),
                    RepositoryRootClass::Data => Self::data(relative),
                }
            })
            .collect()
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

    pub(crate) fn ensure_semantic(&self) -> Result<(), RepositoryLayoutError> {
        self.clone().checked().map(drop)
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
pub struct InjectivityProof {
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

    /// Stable virtual-to-physical uniqueness proof.
    pub fn injectivity_proof(&self) -> &InjectivityProof {
        &self.proof
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
        let path = VirtualPath::new(path.root, path.relative).map_err(serde::de::Error::custom)?;
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
        let virtual_path = if let Ok(relative) = physical.strip_prefix(&self.data.path) {
            VirtualPath::new(
                RepositoryRootClass::Data,
                RootRelativePath::parse(relative)?,
            )?
        } else if let Ok(relative) = physical.strip_prefix(&self.worktree.path) {
            VirtualPath::new(
                RepositoryRootClass::Worktree,
                RootRelativePath::parse(relative)?,
            )?
        } else {
            return Err(RepositoryLayoutError::OutsideRepositoryRoots(physical));
        };
        virtual_path.checked()
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
                let relative = path.relative.as_str();
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
            RootRelativePath::Descendant(text) => assert_eq!(text, "issues/one.json"),
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
}
