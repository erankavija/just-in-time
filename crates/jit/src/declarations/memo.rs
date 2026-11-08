//! Parses of declaration sources, keyed on the sources' bytes.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use super::rules::RulesDocument;
use super::{
    ConfigurationAssemblyError, ConfigurationDocument, GateDeclarationError, GateRegistry,
};
use crate::config::JitConfig;
use crate::repository_state::{RepositoryLayoutError, VirtualPath};

/// The distinct sources of each kind a memo keeps: a command reads its base's
/// sources and, when it edits them, the edited ones. The bound keeps a
/// long-lived executor's memo from growing with every edit it observes.
const RETAINED: usize = 4;

/// The parses one executor's captures share: every capture of byte-identical
/// declaration sources reuses one parse, and other sources parse afresh. The
/// key is the bytes themselves, so the memo is never stale against disk.
#[derive(Default)]
pub struct DeclarationMemo {
    configurations: Recent<ParsedConfiguration>,
    rules: Recent<ParsedRules>,
    gates: Recent<ParsedGates>,
}

impl DeclarationMemo {
    /// The parse of `config.toml`, `templates.toml` and `invariants.toml`, each
    /// `None` when absent.
    pub(crate) fn configuration(&self, sources: [Option<&[u8]>; 3]) -> Arc<ParsedConfiguration> {
        self.configurations.get_or_insert(
            |parsed| {
                parsed
                    .sources
                    .iter()
                    .map(Option::as_deref)
                    .eq(sources.iter().copied())
            },
            || {
                let document = ConfigurationDocument::parse(sources[0]);
                ParsedConfiguration {
                    selection: document.worktree_view(),
                    document: Mutex::new(Some(document)),
                    sources: sources.map(|source| source.map(<[u8]>::to_vec)),
                    assembled: OnceLock::new(),
                    default_schemas: OnceLock::new(),
                }
            },
        )
    }

    /// The parse of `rules.toml` content.
    pub(crate) fn rules(&self, content: &str) -> Arc<ParsedRules> {
        self.rules.get_or_insert(
            |parsed| parsed.content == content,
            || ParsedRules {
                document: RulesDocument::parse(content),
                content: content.to_owned(),
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

/// One parse of the configuration sources, assembled on first use.
pub(crate) struct ParsedConfiguration {
    sources: [Option<Vec<u8>>; 3],
    /// The `[worktree]` view, which outlives the assembly.
    selection: ConfigurationDocument,
    /// The whole document, until the one assembly consumes it.
    document: Mutex<Option<ConfigurationDocument>>,
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

    /// The document's `[worktree]` view.
    pub(crate) fn selection(&self) -> &ConfigurationDocument {
        &self.selection
    }

    /// The one assembly of these sources, its failure included.
    pub(crate) fn assembled(&self) -> Result<&JitConfig, &ConfigurationAssemblyError> {
        self.assembled
            .get_or_init(|| {
                let document = self
                    .document
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()
                    .unwrap_or_else(|| ConfigurationDocument::parse(self.sources[0].as_deref()));
                document.into_assembled(self.sources[1].as_deref(), self.sources[2].as_deref())
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

/// One parse of `rules.toml` content.
pub(crate) struct ParsedRules {
    content: String,
    document: RulesDocument,
}

impl std::ops::Deref for ParsedRules {
    type Target = RulesDocument;

    fn deref(&self) -> &RulesDocument {
        &self.document
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

/// The most recently used [`RETAINED`] parses of one source kind.
struct Recent<T>(Mutex<VecDeque<Arc<T>>>);

impl<T> Default for Recent<T> {
    fn default() -> Self {
        Self(Mutex::new(VecDeque::new()))
    }
}

impl<T> Recent<T> {
    fn get_or_insert(&self, matches: impl Fn(&T) -> bool, parse: impl FnOnce() -> T) -> Arc<T> {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let parsed = held
            .iter()
            .position(|parsed| matches(parsed))
            .and_then(|index| held.remove(index))
            .unwrap_or_else(|| Arc::new(parse()));
        held.push_front(Arc::clone(&parsed));
        held.truncate(RETAINED);
        parsed
    }
}
