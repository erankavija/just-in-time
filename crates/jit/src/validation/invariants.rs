//! Declarative project-invariant registry (driven by `.jit/invariants.toml`).
//!
//! An invariant is a project-scoped (`@`) statement of a property the project
//! intends to hold, optionally bound to the rule or gate that enforces it. This
//! module is the registry-first SOURCE for those entries: it owns only the pure
//! parse/load step (read the file if present, parse with serde + toml, return a
//! typed error on malformed/invalid content). Wiring the loaded registry into
//! [`JitConfig::load`](crate::config::JitConfig::load) is the config layer's job,
//! and indexing/querying invariants as addressable items (`@/<self-id>`) is built
//! on top of this registry by a later layer.
//!
//! The loader mirrors [`RuleSet::load`](crate::validation::rules::RuleSet::load):
//! an absent file is graceful (an empty registry, NOT an error), and each entry
//! is keyed by its `id` field — the entry's SELF-ID, from which the project-scoped
//! qualified id `@/<self-id>` is derived.

use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors that can occur while loading and parsing `.jit/invariants.toml`.
///
/// Every variant carries enough context (path, entry id, or the underlying
/// parse error) to point an author at the offending entry. Mirrors
/// [`RuleConfigError`](crate::validation::rules::RuleConfigError).
///
/// # Examples
///
/// ```
/// use jit::validation::invariants::{InvariantConfigError, InvariantRegistry};
///
/// // An entry with a missing `statement` is a typed, descriptive error.
/// let toml = r#"
/// [[invariants]]
/// id = "INV-01"
/// kind = "advisory"
/// "#;
/// let err = InvariantRegistry::from_toml_str(toml).unwrap_err();
/// assert!(matches!(err, InvariantConfigError::Toml(_)));
/// assert!(err.to_string().contains("statement"));
/// ```
#[derive(Debug, Error)]
pub enum InvariantConfigError {
    /// The invariants file could not be read from disk.
    #[error("failed to read invariants file '{path}': {source}")]
    Io {
        /// Path that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// The invariants file is not valid TOML or does not match the schema. This
    /// covers a missing required field (`statement`, `id`, `kind`) and an
    /// unrecognized `kind` token, both of which surface as descriptive serde
    /// messages naming the offending field or value.
    #[error("failed to parse invariants file: {0}")]
    Toml(#[from] toml::de::Error),

    /// Two or more invariants share the same `id`. Ids MUST be unique so the
    /// project-scoped qualified id `@/<self-id>` addresses exactly one entry.
    #[error("duplicate invariant id '{id}': invariant ids must be unique")]
    DuplicateId {
        /// The id that appeared more than once.
        id: String,
    },
}

/// Whether an invariant is mechanically enforced or merely advisory.
///
/// Deserialized from the kebab-case tokens `"enforced"` / `"advisory"`; an
/// unrecognized value is a descriptive parse error rather than a silent default
/// (there is no `Default`, so the field is required on every entry).
///
/// # Examples
///
/// ```
/// use jit::validation::invariants::{InvariantKind, InvariantRegistry};
///
/// // Parsed from its token inside an invariant entry.
/// let toml = "[[invariants]]\nid = \"INV-01\"\nstatement = \"s\"\nkind = \"enforced\"\n";
/// let reg = InvariantRegistry::from_toml_str(toml).unwrap();
/// assert_eq!(reg.invariants[0].kind, InvariantKind::Enforced);
///
/// // An invalid token is a descriptive error listing the valid values.
/// let bad = "[[invariants]]\nid = \"INV-02\"\nstatement = \"s\"\nkind = \"both\"\n";
/// let err = InvariantRegistry::from_toml_str(bad).unwrap_err();
/// assert!(err.to_string().contains("enforced"));
/// assert!(err.to_string().contains("advisory"));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InvariantKind {
    /// The invariant is mechanically enforced (typically by the bound rule/gate
    /// named in [`Invariant::enforced_by`]).
    Enforced,
    /// The invariant is documented intent without mechanical enforcement.
    Advisory,
}

/// A single project-scoped invariant entry as loaded from `.jit/invariants.toml`.
///
/// The `id` is the entry's SELF-ID; its project-scoped qualified id is
/// `@/<id>`. `statement` and `kind` are required; `enforced_by` (authored as
/// `enforced-by`) is an optional binding to a rule name or gate key.
///
/// # Examples
///
/// ```
/// use jit::validation::invariants::{Invariant, InvariantKind, InvariantRegistry};
///
/// let toml = r#"
/// [[invariants]]
/// id = "INV-01"
/// statement = "Every dependency edge stays acyclic."
/// kind = "enforced"
/// enforced-by = "dag-no-cycles"
/// "#;
/// let reg = InvariantRegistry::from_toml_str(toml).unwrap();
/// let inv: &Invariant = &reg.invariants[0];
/// assert_eq!(inv.id, "INV-01");
/// assert_eq!(inv.kind, InvariantKind::Enforced);
/// assert_eq!(inv.enforced_by.as_deref(), Some("dag-no-cycles"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Invariant {
    /// The entry's self-id; `@/<id>` is its project-scoped qualified id.
    pub id: String,
    /// The invariant statement (the property the project intends to hold).
    pub statement: String,
    /// Whether the invariant is mechanically enforced or advisory.
    pub kind: InvariantKind,
    /// Optional binding to the rule name or gate key that enforces it.
    #[serde(default, rename = "enforced-by")]
    pub enforced_by: Option<String>,
}

/// The set of project invariants loaded from `.jit/invariants.toml`.
///
/// Held as a field on [`JitConfig`](crate::config::JitConfig) so downstream
/// indexing can project each entry as a project-scoped (`@`) addressable item.
/// An absent file loads as an empty registry.
///
/// # Examples
///
/// ```
/// use jit::validation::invariants::InvariantRegistry;
///
/// // An empty document is a valid, empty registry.
/// let reg = InvariantRegistry::from_toml_str("").unwrap();
/// assert!(reg.invariants.is_empty());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvariantRegistry {
    /// The declared invariants, in authored order.
    pub invariants: Vec<Invariant>,
}

/// Top-level `invariants.toml` document.
#[derive(Debug, Deserialize)]
struct RawInvariantsFile {
    #[serde(default, rename = "invariants")]
    invariants: Vec<Invariant>,
}

impl InvariantRegistry {
    /// An empty registry (used when no `invariants.toml` exists).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::invariants::InvariantRegistry;
    ///
    /// let reg = InvariantRegistry::empty();
    /// assert!(reg.invariants.is_empty());
    /// ```
    pub fn empty() -> Self {
        Self::default()
    }

    /// Load and parse `.jit/invariants.toml` relative to the given `.jit` root.
    ///
    /// Returns an empty [`InvariantRegistry`] when the file does not exist
    /// (graceful, NOT an error), mirroring
    /// [`RuleSet::load`](crate::validation::rules::RuleSet::load).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::invariants::InvariantRegistry;
    ///
    /// // A directory with no `invariants.toml` loads as an empty registry.
    /// let dir = tempfile::tempdir().unwrap();
    /// let reg = InvariantRegistry::load(dir.path()).unwrap();
    /// assert!(reg.invariants.is_empty());
    /// ```
    pub fn load(jit_root: &Path) -> Result<Self, InvariantConfigError> {
        let path = jit_root.join("invariants.toml");
        if !path.exists() {
            return Ok(Self::empty());
        }
        let content =
            std::fs::read_to_string(&path).map_err(|source| InvariantConfigError::Io {
                path: path.clone(),
                source,
            })?;
        Self::from_toml_str(&content)
    }

    /// Parse an `invariants.toml` string into a registry.
    ///
    /// A missing required field or an unrecognized `kind` token surfaces as a
    /// typed [`InvariantConfigError::Toml`] with a descriptive message; a
    /// duplicate `id` is a typed [`InvariantConfigError::DuplicateId`].
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::invariants::InvariantRegistry;
    ///
    /// let toml = r#"
    /// [[invariants]]
    /// id = "INV-01"
    /// statement = "Gates must pass before Done."
    /// kind = "advisory"
    /// "#;
    /// let reg = InvariantRegistry::from_toml_str(toml).unwrap();
    /// assert_eq!(reg.invariants[0].id, "INV-01");
    /// assert!(reg.invariants[0].enforced_by.is_none());
    /// ```
    pub fn from_toml_str(content: &str) -> Result<Self, InvariantConfigError> {
        let raw: RawInvariantsFile = toml::from_str(content)?;

        // Enforce id uniqueness so each project-scoped `@/<self-id>` addresses
        // exactly one entry; detect the first collision via a `HashSet` insert.
        let mut seen = HashSet::new();
        if let Some(inv) = raw.invariants.iter().find(|i| !seen.insert(i.id.as_str())) {
            return Err(InvariantConfigError::DuplicateId { id: inv.id.clone() });
        }

        Ok(Self {
            invariants: raw.invariants,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_toml_str_loads_full_entry() {
        let toml = r#"
[[invariants]]
id = "INV-01"
statement = "Every dependency edge stays acyclic."
kind = "enforced"
enforced-by = "dag-no-cycles"
"#;
        let reg = InvariantRegistry::from_toml_str(toml).unwrap();
        assert_eq!(reg.invariants.len(), 1);
        let inv = &reg.invariants[0];
        assert_eq!(inv.id, "INV-01");
        assert_eq!(inv.statement, "Every dependency edge stays acyclic.");
        assert_eq!(inv.kind, InvariantKind::Enforced);
        assert_eq!(inv.enforced_by.as_deref(), Some("dag-no-cycles"));
    }

    #[test]
    fn test_from_toml_str_enforced_by_is_optional() {
        let toml = r#"
[[invariants]]
id = "INV-02"
statement = "Issues prefer functional style."
kind = "advisory"
"#;
        let reg = InvariantRegistry::from_toml_str(toml).unwrap();
        assert_eq!(reg.invariants[0].kind, InvariantKind::Advisory);
        assert!(reg.invariants[0].enforced_by.is_none());
    }

    #[test]
    fn test_from_toml_str_empty_document_is_empty_registry() {
        let reg = InvariantRegistry::from_toml_str("").unwrap();
        assert!(reg.invariants.is_empty());
    }

    #[test]
    fn test_from_toml_str_missing_statement_is_typed_error() {
        let toml = r#"
[[invariants]]
id = "INV-01"
kind = "advisory"
"#;
        let err = InvariantRegistry::from_toml_str(toml).unwrap_err();
        assert!(matches!(err, InvariantConfigError::Toml(_)));
        assert!(
            err.to_string().contains("statement"),
            "error should name the missing field: {err}"
        );
    }

    #[test]
    fn test_from_toml_str_bad_kind_is_typed_error() {
        let toml = r#"
[[invariants]]
id = "INV-01"
statement = "x"
kind = "mandatory"
"#;
        let err = InvariantRegistry::from_toml_str(toml).unwrap_err();
        assert!(matches!(err, InvariantConfigError::Toml(_)));
        let msg = err.to_string();
        assert!(
            msg.contains("enforced") && msg.contains("advisory"),
            "error should list the valid kind values: {msg}"
        );
    }

    #[test]
    fn test_from_toml_str_duplicate_id_is_typed_error() {
        let toml = r#"
[[invariants]]
id = "INV-01"
statement = "a"
kind = "advisory"

[[invariants]]
id = "INV-01"
statement = "b"
kind = "advisory"
"#;
        let err = InvariantRegistry::from_toml_str(toml).unwrap_err();
        assert!(matches!(
            err,
            InvariantConfigError::DuplicateId { ref id } if id == "INV-01"
        ));
    }

    #[test]
    fn test_load_absent_file_is_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        let reg = InvariantRegistry::load(dir.path()).unwrap();
        assert!(reg.invariants.is_empty());
    }

    #[test]
    fn test_load_reads_present_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("invariants.toml"),
            "[[invariants]]\nid = \"INV-09\"\nstatement = \"s\"\nkind = \"enforced\"\n",
        )
        .unwrap();
        let reg = InvariantRegistry::load(dir.path()).unwrap();
        assert_eq!(reg.invariants.len(), 1);
        assert_eq!(reg.invariants[0].id, "INV-09");
        assert_eq!(reg.invariants[0].kind, InvariantKind::Enforced);
    }

    #[test]
    fn test_load_malformed_file_is_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("invariants.toml"), "not = valid = toml").unwrap();
        let err = InvariantRegistry::load(dir.path()).unwrap_err();
        assert!(matches!(err, InvariantConfigError::Toml(_)));
    }
}
