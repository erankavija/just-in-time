//! Parses of declaration sources, keyed on the sources' bytes.

use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use super::rules::{RuleConfigError, RuleSet, RulesDocument, SchemaRequest};
use super::{
    ConfigurationAssemblyError, ConfigurationDocument, GateDeclarationError, GateRegistry,
};
use crate::config::JitConfig;
use crate::repository_state::{RepositoryLayoutError, VirtualPath};

/// The parses one command's captures share: every capture of byte-identical
/// declaration sources reuses one parse, and other sources parse afresh. The
/// key is the bytes themselves, so the memo is never stale against disk.
///
/// A command creates one memo at its start and hands it to its store; the memo
/// keeps every distinct source it parsed until the command drops it, so its
/// size is bounded by the command's distinct sources.
#[derive(Default)]
pub struct DeclarationMemo {
    documents: Parses<ParsedDocument>,
    configurations: Parses<ParsedConfiguration>,
    rules: Parses<ParsedRules>,
    gates: Parses<ParsedGates>,
}

impl DeclarationMemo {
    /// The assembly of `config.toml`, `templates.toml` and `invariants.toml`,
    /// each `None` when absent; the `config.toml` parse is shared by every
    /// assembly of the same bytes.
    pub(crate) fn configuration(&self, sources: [Option<&[u8]>; 3]) -> Arc<ParsedConfiguration> {
        self.configurations.get_or_insert(
            |parsed| {
                parsed
                    .sources
                    .iter()
                    .map(Option::as_deref)
                    .eq(sources.iter().copied())
            },
            || ParsedConfiguration {
                document: self.documents.get_or_insert(
                    |parsed| parsed.bytes.as_deref() == sources[0],
                    || {
                        let document = ConfigurationDocument::parse(sources[0]);
                        ParsedDocument {
                            bytes: sources[0].map(<[u8]>::to_vec),
                            selection: document.worktree_view(),
                            document,
                        }
                    },
                ),
                sources: sources.map(|source| source.map(<[u8]>::to_vec)),
                assembled: OnceLock::new(),
                default_schemas: OnceLock::new(),
            },
        )
    }

    /// The decode and parse of `rules.toml` bytes.
    pub(crate) fn rules(&self, bytes: &[u8]) -> Arc<ParsedRules> {
        self.rules.get_or_insert(
            |parsed| parsed.bytes == bytes,
            || ParsedRules {
                source: std::str::from_utf8(bytes)
                    .map(|text| (text.to_owned(), RulesDocument::parse(text)))
                    .map_err(Arc::new),
                bytes: bytes.to_vec(),
            },
        )
    }
}

impl DeclarationMemo {
    /// The parse of `gates.toml` bytes.
    pub(crate) fn gates(&self, bytes: &[u8]) -> Arc<ParsedGates> {
        self.gates.get_or_insert(
            |parsed| parsed.bytes == bytes,
            || ParsedGates {
                registry: super::parse_gate_registry(bytes),
                bytes: bytes.to_vec(),
            },
        )
    }
}

/// One parse of `config.toml` bytes.
struct ParsedDocument {
    bytes: Option<Vec<u8>>,
    document: ConfigurationDocument,
    /// The document's `[worktree]` view.
    selection: ConfigurationDocument,
}

/// One set of configuration sources, assembled on first use.
pub(crate) struct ParsedConfiguration {
    sources: [Option<Vec<u8>>; 3],
    document: Arc<ParsedDocument>,
    assembled: OnceLock<Result<JitConfig, ConfigurationAssemblyError>>,
    default_schemas: OnceLock<Result<Vec<VirtualPath>, RepositoryLayoutError>>,
}

/// The schema files the default rules of a configuration derive.
pub(crate) type DefaultSchemas = Result<Vec<VirtualPath>, RepositoryLayoutError>;

impl ParsedConfiguration {
    /// The bytes parsed, in `config.toml`, `templates.toml`, `invariants.toml` order.
    pub(crate) fn sources(&self) -> &[Option<Vec<u8>>; 3] {
        &self.sources
    }

    /// The `config.toml` document's `[worktree]` view.
    pub(crate) fn selection(&self) -> &ConfigurationDocument {
        &self.document.selection
    }

    /// The one assembly of these sources, its failure included.
    pub(crate) fn assembled(&self) -> Result<&JitConfig, &ConfigurationAssemblyError> {
        self.assembled
            .get_or_init(|| {
                self.document
                    .document
                    .clone()
                    .into_assembled(self.sources[1].as_deref(), self.sources[2].as_deref())
            })
            .as_ref()
    }

    /// [`Self::assembled`] by value.
    pub(crate) fn assemble(&self) -> Result<JitConfig, ConfigurationAssemblyError> {
        self.assembled().cloned().map_err(Clone::clone)
    }

    /// The schema files the default rules of the assembly derive, derived
    /// once; `None` when the assembly failed.
    pub(crate) fn default_schemas(&self) -> Option<&DefaultSchemas> {
        let config = self.assembled().ok()?;
        Some(
            self.default_schemas
                .get_or_init(|| crate::repository_state::default_schema_paths(config)),
        )
    }
}

/// One decode and parse of `rules.toml` bytes, its failure included.
pub(crate) struct ParsedRules {
    bytes: Vec<u8>,
    source: Result<(String, RulesDocument), Arc<std::str::Utf8Error>>,
}

impl ParsedRules {
    fn document(&self) -> Result<&RulesDocument, RuleConfigError> {
        self.source
            .as_ref()
            .map(|(_, document)| document)
            .map_err(|error| RuleConfigError::Utf8(Arc::clone(error)))
    }

    /// The decoded text of the bytes.
    pub(crate) fn text(&self) -> Result<&str, RuleConfigError> {
        self.source
            .as_ref()
            .map(|(text, _)| text.as_str())
            .map_err(|error| RuleConfigError::Utf8(Arc::clone(error)))
    }

    /// [`RulesDocument::schema_requests`] over the decoded bytes.
    pub(crate) fn schema_requests(&self) -> Result<&[SchemaRequest], RuleConfigError> {
        self.document()?.schema_requests()
    }

    /// [`RulesDocument::rule_set`] over the decoded bytes.
    pub(crate) fn rule_set(
        &self,
        config: Option<&JitConfig>,
        schemas: impl IntoIterator<Item = (String, Vec<u8>)>,
    ) -> Result<RuleSet, RuleConfigError> {
        self.document()?.rule_set(config, schemas)
    }
}

/// One parse of `gates.toml` bytes.
pub(crate) struct ParsedGates {
    bytes: Vec<u8>,
    registry: Result<GateRegistry, GateDeclarationError>,
}

impl ParsedGates {
    /// The registry, or its parse failure.
    pub(crate) fn registry(&self) -> Result<&GateRegistry, &GateDeclarationError> {
        self.registry.as_ref()
    }

    /// [`Self::registry`] by value.
    pub(crate) fn to_registry(&self) -> Result<GateRegistry, GateDeclarationError> {
        self.registry().cloned().map_err(Clone::clone)
    }
}

/// Every parse of one source kind a memo made.
struct Parses<T>(Mutex<Vec<Arc<T>>>);

impl<T> Default for Parses<T> {
    fn default() -> Self {
        Self(Mutex::new(Vec::new()))
    }
}

impl<T> Parses<T> {
    fn get_or_insert(&self, matches: impl Fn(&T) -> bool, parse: impl FnOnce() -> T) -> Arc<T> {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match held.iter().find(|parsed| matches(parsed)) {
            Some(parsed) => Arc::clone(parsed),
            None => {
                let parsed = Arc::new(parse());
                held.push(Arc::clone(&parsed));
                parsed
            }
        }
    }
}
