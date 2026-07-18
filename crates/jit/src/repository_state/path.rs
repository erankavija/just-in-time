//! Canonical repository-root and virtual-path identities.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// A normalized path relative to one selected repository root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RootRelativePath {
    /// The selected root itself.
    Root,
    /// A non-empty normalized descendant.
    Descendant(String),
}

impl RootRelativePath {
    /// Parse a root-relative path without consulting the filesystem.
    pub fn parse(path: impl AsRef<Path>) -> Result<Self, RepositoryLayoutError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Ok(Self::Root);
        }
        let text = path.to_string_lossy();
        if path.is_absolute()
            || text.contains('\\')
            || text.contains(':')
            || text.chars().any(char::is_control)
            || !path
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(RepositoryLayoutError::LexicalEscape(text.into_owned()));
        }
        Ok(Self::Descendant(text.into_owned()))
    }

    /// Return this identity as a relative filesystem path.
    pub fn as_path(&self) -> &Path {
        match self {
            Self::Root => Path::new(""),
            Self::Descendant(path) => Path::new(path),
        }
    }

    /// Number of path components below the selected root.
    pub fn depth(&self) -> usize {
        self.as_path().components().count()
    }
}

/// A canonical semantic path qualified by its selected root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum VirtualPath {
    /// An authored worktree path.
    Worktree(RootRelativePath),
    /// A repository data path.
    Data(RootRelativePath),
}

impl VirtualPath {
    /// Construct a worktree-relative semantic identity.
    pub fn worktree(path: impl AsRef<Path>) -> Result<Self, RepositoryLayoutError> {
        Self::Worktree(RootRelativePath::parse(path)?).checked()
    }

    /// Construct a data-root-relative semantic identity.
    pub fn data(path: impl AsRef<Path>) -> Result<Self, RepositoryLayoutError> {
        Self::Data(RootRelativePath::parse(path)?).checked()
    }

    /// Root-relative portion of this identity.
    pub fn relative(&self) -> &RootRelativePath {
        match self {
            Self::Worktree(path) | Self::Data(path) => path,
        }
    }

    fn checked(self) -> Result<Self, RepositoryLayoutError> {
        let reserved = match &self {
            Self::Worktree(RootRelativePath::Descendant(path)) => {
                path == ".jit-bootstrap" || path.starts_with(".jit-bootstrap/")
            }
            Self::Data(RootRelativePath::Descendant(path)) => {
                path == "tmp" || path.starts_with("tmp/")
            }
            _ => false,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectivityProof {
    worktree_identity: String,
    data_identity: String,
    nested_data_relative: Option<RootRelativePath>,
}

/// Canonical relationship between worktree and selected data roots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        let nested_data_relative = data
            .path
            .strip_prefix(&worktree.path)
            .ok()
            .map(RootRelativePath::parse)
            .transpose()?;
        if let Some(RootRelativePath::Descendant(path)) = &nested_data_relative {
            if path == ".jit-bootstrap" || path.starts_with(".jit-bootstrap/") {
                return Err(RepositoryLayoutError::ReservedTransactionPath(
                    VirtualPath::Worktree(RootRelativePath::Descendant(path.clone())),
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

    /// Stable virtual-to-physical uniqueness proof.
    pub fn injectivity_proof(&self) -> &InjectivityProof {
        &self.proof
    }

    /// Resolve an already-canonical virtual identity.
    pub fn resolve(&self, path: &VirtualPath) -> Result<PathBuf, RepositoryLayoutError> {
        self.ensure_canonical(path)?;
        let (root, relative) = match path {
            VirtualPath::Worktree(relative) => (&self.worktree.path, relative),
            VirtualPath::Data(relative) => (&self.data.path, relative),
        };
        Ok(root.join(relative.as_path()))
    }

    /// Classify a normalized absolute path, applying Data precedence.
    pub fn classify_and_canonicalize(
        &self,
        physical: impl AsRef<Path>,
    ) -> Result<VirtualPath, RepositoryLayoutError> {
        let physical = lexical_absolute(physical.as_ref())?;
        let virtual_path = if let Ok(relative) = physical.strip_prefix(&self.data.path) {
            VirtualPath::Data(RootRelativePath::parse(relative)?)
        } else if let Ok(relative) = physical.strip_prefix(&self.worktree.path) {
            VirtualPath::Worktree(RootRelativePath::parse(relative)?)
        } else {
            return Err(RepositoryLayoutError::OutsideRepositoryRoots(physical));
        };
        virtual_path.checked()
    }

    /// Reject a Worktree spelling beneath a nested data root.
    pub fn ensure_canonical(&self, path: &VirtualPath) -> Result<(), RepositoryLayoutError> {
        path.ensure_semantic()?;
        if let (
            Some(RootRelativePath::Descendant(prefix)),
            VirtualPath::Worktree(RootRelativePath::Descendant(relative)),
        ) = (&self.nested_data_relative, path)
        {
            if relative == prefix || relative.starts_with(&format!("{prefix}/")) {
                return Err(RepositoryLayoutError::DataRootAlias(path.clone()));
            }
        }
        Ok(())
    }
}

/// Invalid root or virtual-path construction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RepositoryLayoutError {
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
}

fn validate_root(root: &RepositoryRootEvidence) -> Result<(), RepositoryLayoutError> {
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
            RepositoryLayout::new(root("/repo/../escape"), root("/data")),
            Err(RepositoryLayoutError::LexicalEscape(_))
        ));
        assert!(matches!(
            RepositoryLayout::new(root("/repo"), root("/repo/.jit-bootstrap/data")),
            Err(RepositoryLayoutError::ReservedTransactionPath(_))
        ));
    }

    #[test]
    fn test_layout_applies_data_precedence_and_rejects_alias() {
        let layout = RepositoryLayout::new(root("/repo"), root("/repo/.jit")).unwrap();
        assert_eq!(
            layout.classify_and_canonicalize("/repo/.jit/index.json"),
            Ok(VirtualPath::Data(RootRelativePath::Descendant(
                "index.json".into()
            )))
        );
        assert!(matches!(
            layout.resolve(&VirtualPath::Worktree(RootRelativePath::Descendant(
                ".jit/index.json".into()
            ))),
            Err(RepositoryLayoutError::DataRootAlias(_))
        ));
    }

    #[test]
    fn test_virtual_path_rejects_control_namespaces() {
        assert!(VirtualPath::worktree(".jit-bootstrap/tx").is_err());
        assert!(VirtualPath::data("tmp/tx").is_err());
        assert!(VirtualPath::worktree("../outside").is_err());
    }
}
