//! The normalized repository-root-relative path identity.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::path::{Component, Path};

/// A path that is not a normalized descendant of the selected root.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("path '{0}' is not lexically confined")]
pub struct RootRelativePathError(pub String);

/// Whether `path` is `reserved` itself or sits below it, by whole components.
pub(crate) fn enters(path: &str, reserved: &str) -> bool {
    path.strip_prefix(reserved)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// A normalized path relative to one selected repository root.
///
/// [`Root`](RootRelativePath::Root) denotes the selected root itself;
/// [`Descendant`](RootRelativePath::Descendant) holds a normalized, non-empty
/// path strictly below it (no parent, prefix, control, or alternate-separator
/// component). The root is a distinct variant, never an empty-string sentinel,
/// so root-vs-descendant is a type-level distinction rather than a value test.
///
/// The descendant payload is [`Cow<'static, str>`] so well-known paths can be
/// `Cow::Borrowed` const associated items of [`crate::repository_state::VirtualPath`], while parsing and
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
    pub fn parse(path: impl AsRef<Path>) -> Result<Self, RootRelativePathError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Ok(Self::Root);
        }
        let text = path
            .to_str()
            .ok_or_else(|| RootRelativePathError(path.to_string_lossy().into()))?;
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
            return Err(RootRelativePathError(text.to_owned()));
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

    /// Whether `path`, relative to the same root, is this path or lies beneath
    /// it by whole components.
    pub(crate) fn contains(&self, path: &str) -> bool {
        match self {
            Self::Root => true,
            Self::Descendant(prefix) => enters(path, prefix),
        }
    }

    /// This path extended by `child`, a canonical relative path.
    pub(crate) fn join(&self, child: &str) -> Self {
        Self::Descendant(Cow::Owned(match self {
            Self::Root => child.to_owned(),
            Self::Descendant(prefix) => format!("{prefix}/{child}"),
        }))
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

/// The published shape matches the wire form the hand-written [`Serialize`] and
/// [`Deserialize`] pair above produce and accept: a string. It is inlined
/// rather than referenced so a schema carrying one reads as the string it is.
impl schemars::JsonSchema for RootRelativePath {
    fn schema_name() -> String {
        "RootRelativePath".to_string()
    }

    fn is_referenceable() -> bool {
        false
    }

    fn json_schema(generator: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        String::json_schema(generator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::RepositoryLayoutError;

    #[test]
    fn test_parse_and_serde_reject_noncanonical_spellings() {
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
        }
    }

    #[test]
    fn test_root_relative_path_uses_variant_not_empty_string_sentinel() {
        assert_eq!(RootRelativePath::parse("").unwrap(), RootRelativePath::Root);
        assert!(RootRelativePath::parse("").unwrap().is_root());
        let issue_path = "issues/one.json";
        match RootRelativePath::parse(issue_path).unwrap() {
            RootRelativePath::Descendant(text) => assert_eq!(&*text, issue_path),
            RootRelativePath::Root => panic!("a non-empty path must be a descendant"),
        }
        assert!(!RootRelativePath::parse(issue_path).unwrap().is_root());
        assert_eq!(RootRelativePath::Root.depth(), 0);
        assert_eq!(RootRelativePath::parse("a/b").unwrap().depth(), 2);
        assert_eq!(
            serde_json::to_string(&RootRelativePath::Root).unwrap(),
            "\"\""
        );
        assert_eq!(
            serde_json::from_str::<RootRelativePath>("\"\"").unwrap(),
            RootRelativePath::Root
        );
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
