//! Addressable structured items: a pure projection over issue descriptions.
//!
//! An **addressable item** is a structured list entry in a declared section of an
//! issue description that carries a *self-id* matched by an id-pattern. Its
//! **qualified id** is *derived* from existing data (the resolved scope, the kind,
//! and the parsed self-id) under the uniform kind-segmented scheme (epic 2821e177
//! REQ-01): `@/<kind>/<self-id>` for a project item, `@/issue/<short-id>/<kind>/
//! <self-id>` for an issue item. Nothing is persisted twice (REQ-02, REQ-03).
//! Because the kind is a segment of the id, two items of *different* kinds sharing
//! a scope and self-id (e.g. a `rule` and a `gate` both named `coverage-preview`)
//! derive DISTINCT qualified ids (`@/rule/coverage-preview` vs
//! `@/gate/coverage-preview`), so a minted qualified id is unique per
//! `(scope, kind, self-id)`.
//!
//! A **scope** ([`Scope`]) is the substrate a qualified id addresses. It is either
//! an issue (by its short-id, [`Scope::Issue`]) or the whole project (the sentinel
//! `@`, [`Scope::Project`], for items not tied to any single issue). Self-id
//! uniqueness is enforced *per (scope, kind)*: the same self-id may exist under two
//! different scopes, or under two different kinds within one scope, without
//! conflict; a self-id repeated within one scope under the SAME kind is a
//! [`ItemError::DuplicateSelfId`].
//!
//! An **item kind** ([`ItemKind`]) is the config-declared projection
//! `(section, id-pattern, marker(s), link-namespace(s), scope, source-of-truth)`
//! that says which entries are addressable and how. The model is generic: no kind
//! NAME is interpreted by this module, only the tuple (REQ-01). Kinds are authored
//! entirely in the `[item_kinds]` config table (scaffolded by `jit init`); this
//! module bakes in no domain defaults. With no `[item_kinds]` table the kind set
//! is empty (single-consumer design, no backward-compat layer — see
//! [`resolve_item_kinds`]). A kind's tuple is chosen to align with the
//! `label-coverage` rule's own defaults, so the coverage machinery is compatible
//! with the model without rewriting any rule (REQ-05).
//!
//! Indexing is pure and substrate-specific but shares one derivation core
//! ([`derive_scope_items`], which enforces per-(scope, kind) uniqueness and mints
//! qualified ids): [`index_items`] projects an issue's markdown (markdown is the
//! single source of truth, recomputed on demand), while [`index_markdown_items`]
//! projects a standalone markdown source file (used for project-scope (`@`)
//! kinds, whose source path comes from config). A list entry without a matching
//! self-id is plain prose and incurs no addressing requirement (REQ-06).

use crate::config::{
    ItemKindConfig, ItemKindSource, KindScopeConfig, SourceOfTruth, TomlSourceDescriptor,
};
use crate::document::ContentParser;
use crate::domain::{project, Issue};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Default section slug scanned for items when a kind declares none.
///
/// Matches the `label-coverage` rule's `criteria-section` default so the built-in
/// `requirement` kind and the coverage rule read the SAME section.
pub const DEFAULT_ITEM_SECTION: &str = "success_criteria";

/// Default self-id pattern when a kind declares none — the repo default id shape
/// (`REQ-01`, `D-2`, `RISK-03`, ...), identical to the `label-coverage` rule's
/// `id-pattern` default.
pub const DEFAULT_ITEM_ID_PATTERN: &str = "[A-Z][A-Z0-9]*-[0-9]+";

/// Default link-label namespace referencing items of a kind when none is
/// declared, matching the `label-coverage` rule's `satisfies-namespace` default.
pub const DEFAULT_ITEM_LINK_NAMESPACE: &str = "satisfies";

/// The scope sentinel that addresses project-level items not tied to any single
/// issue. As a standalone scope token it denotes [`Scope::Project`] (REQ-01); it
/// also opens every minted qualified id — `@/<kind>/<self-id>` for a project item,
/// `@/issue/<short-id>/<kind>/<self-id>` for an issue item — so the `issue`
/// reserved segment, not the leading `@`, is what distinguishes the two.
pub const PROJECT_SCOPE_SENTINEL: &str = "@";

/// The substrate an addressable item belongs to (REQ-01), one input to its
/// derived qualified id.
///
/// A scope is EITHER one issue (addressed by its short-id) or the whole project
/// (the `@` sentinel, for items such as invariants that no single issue owns).
/// Self-id uniqueness is enforced *per (scope, kind)* (REQ-04), so the same
/// self-id may appear under two distinct scopes, or under two different kinds
/// within one scope, without collision.
///
/// The qualified id is a pure projection ([`qualified_id`]): the scope selects the
/// uniform address shape (`@/<kind>/<self-id>` for the project, `@/issue/<short-id>/
/// <kind>/<self-id>` for an issue) and [`Scope::prefix`] renders the item's own
/// `scope` field (`@` or the short-id); nothing about the scope is persisted
/// separately (REQ-05).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// An issue scope, carrying the issue's short-id (the qualified-id prefix).
    Issue(String),
    /// The project scope, rendered with the [`PROJECT_SCOPE_SENTINEL`] (`@`).
    Project,
}

impl Scope {
    /// Parse a qualified id's scope segment into a [`Scope`].
    ///
    /// The [`PROJECT_SCOPE_SENTINEL`] (`@`) yields [`Scope::Project`]; every other
    /// segment is taken as an issue scope verbatim (resolution of a short-id /
    /// unique prefix to a full id is a storage concern handled by the caller, not
    /// this pure parser).
    pub fn parse(segment: &str) -> Self {
        if segment == PROJECT_SCOPE_SENTINEL {
            Scope::Project
        } else {
            Scope::Issue(segment.to_string())
        }
    }

    /// The scope prefix this renders to (`@` for the project, the issue short-id
    /// otherwise): the value carried in an item's `scope` field and named in a
    /// [`ItemError::DuplicateSelfId`] error. The minted qualified id embeds this
    /// through [`qualified_id`] rather than concatenating it directly.
    pub fn prefix(&self) -> &str {
        match self {
            Scope::Project => PROJECT_SCOPE_SENTINEL,
            Scope::Issue(short_id) => short_id,
        }
    }

    /// Whether this is the project scope (`@`).
    pub fn is_project(&self) -> bool {
        matches!(self, Scope::Project)
    }
}

/// The scope half of a [`KindSegmentedAddress`], as recognized by
/// [`parse_kind_segmented_address`].
///
/// This is a SEPARATE type from [`Scope`], not a variant added to it: a
/// kind-segmented address recognizes a third scope token beyond [`Scope`]'s
/// issue/project split — a named-project reference `@<name>` — and folding it
/// into [`Scope`] would make every existing exhaustive `match` on [`Scope`]
/// non-exhaustive. Wiring [`Scope`] itself to the third form is later work; this
/// parser is additive only.
///
/// [`AddressScope::NamedProject`] carries `<name>` purely STRUCTURALLY as
/// parsed — this parser never matches on `@`-prefixes beyond splitting out the
/// token. Binding `<name>` against a declared local project identity is
/// [`AddressScope::bind_local`]'s job, not this parser's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressScope {
    /// The local project, addressed as bare `@`.
    Project,
    /// A named-project reference `@<name>`, carrying the unresolved name.
    NamedProject(String),
    /// An issue scope, carrying the issue short-id from `@/issue/<short-id>/...`.
    Issue(String),
}

impl AddressScope {
    /// Bind this parsed address-grammar scope token to local project identity,
    /// collapsing it to the two-variant [`Scope`] every non-address caller
    /// already resolves against (REQ-01, REQ-09, design decision D5).
    ///
    /// `local_project_name` is the repo's declared `[project] name`
    /// (`None` when undeclared), supplied by the caller from an
    /// already-loaded config — this function does no I/O.
    ///
    /// - [`AddressScope::Project`] (bare `@`) binds to [`Scope::Project`],
    ///   unchanged from today.
    /// - [`AddressScope::NamedProject`] binds to [`Scope::Project`] ONLY when
    ///   `name` equals `local_project_name`: a `@<name>` address naming the
    ///   local project resolves identically to bare `@`. Every other
    ///   named-project reference — a different name, or no local name
    ///   declared at all — is [`ItemError::NotResolvable`]: this module
    ///   performs no federation or remote lookup, so a project other than the
    ///   local one is syntactically valid but never resolves here.
    /// - [`AddressScope::Issue`] passes through unchanged: an issue-scoped
    ///   address carries no project identity to bind.
    ///
    /// This consumes the already-PARSED scope token and never re-matches on
    /// any `@`-prefix itself (REQ-01).
    pub fn bind_local(self, local_project_name: Option<&str>) -> Result<Scope, ItemError> {
        match self {
            AddressScope::Project => Ok(Scope::Project),
            AddressScope::Issue(short_id) => Ok(Scope::Issue(short_id)),
            AddressScope::NamedProject(name) => {
                if local_project_name == Some(name.as_str()) {
                    Ok(Scope::Project)
                } else {
                    Err(ItemError::NotResolvable { project: name })
                }
            }
        }
    }
}

/// A kind-segmented address, parsed into its `(scope, kind, self-id)` components
/// by [`parse_kind_segmented_address`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindSegmentedAddress {
    /// The address's scope: the local project, a named project, or an issue.
    pub scope: AddressScope,
    /// The kind segment, carried verbatim — not validated against any kind
    /// registry, since this parser is a pure structural split.
    pub kind: String,
    /// The self-id segment.
    pub self_id: String,
}

/// Parse a kind-segmented address into its `(scope, kind, self-id)` components.
///
/// Recognizes exactly three explicit `@`-prefixed forms:
/// - `@/<kind>/<self-id>` — the local-project form ([`AddressScope::Project`]).
/// - `@<name>/<kind>/<self-id>` — a named-project form
///   ([`AddressScope::NamedProject`]).
/// - `@/issue/<short-id>/<kind>/<self-id>` — the issue-item form
///   ([`AddressScope::Issue`]); `issue` is a reserved, built-in segment, never a
///   kind name in any form.
///
/// This is a pure structural parse: `kind` and `self_id` are returned verbatim,
/// unvalidated against any kind registry (the caller matches them against its own
/// configured kinds, keeping this domain function free of kind-name literals).
/// The `<short-id>/<self-id>` sugar form is separate: it is expanded to this same
/// triple shape by [`expand_sugar_address`], which infers the kind segment this
/// parser requires explicit.
///
/// Malformed input — a missing kind or self-id segment, an empty scope/kind/
/// self-id segment, an unrecognized reserved segment where `issue` was expected,
/// the wrong number of `/`-separated segments, or a `:` anywhere in the address
/// (the colon is reserved for the label `namespace:value` separator and never
/// valid inside an address) — is a typed [`ItemError::InvalidAddress`] naming the
/// offending address and why it failed.
pub fn parse_kind_segmented_address(address: &str) -> Result<KindSegmentedAddress, ItemError> {
    let invalid = |reason: String| ItemError::InvalidAddress {
        address: address.to_string(),
        reason,
    };

    if address.contains(':') {
        return Err(invalid(
            "the address contains a colon ':', which is reserved for the label \
             'namespace:value' separator and is never valid inside an address"
                .to_string(),
        ));
    }

    let mut segments = address.split('/');
    let scope_token = segments
        .next()
        .filter(|token| token.starts_with(PROJECT_SCOPE_SENTINEL))
        .ok_or_else(|| invalid("address must start with the '@' scope sentinel".to_string()))?;
    let project_name = &scope_token[PROJECT_SCOPE_SENTINEL.len()..];
    let rest: Vec<&str> = segments.collect();

    match rest.len() {
        4 if rest[0] == KindScope::ISSUE_TOKEN => {
            if !project_name.is_empty() {
                return Err(invalid(
                    "the issue-item form must use the bare '@' scope, not a named project"
                        .to_string(),
                ));
            }
            let (short_id, kind, self_id) = (rest[1], rest[2], rest[3]);
            require_non_empty(short_id, "issue short-id", address)?;
            require_non_empty(kind, "kind", address)?;
            require_non_empty(self_id, "self-id", address)?;
            Ok(KindSegmentedAddress {
                scope: AddressScope::Issue(short_id.to_string()),
                kind: kind.to_string(),
                self_id: self_id.to_string(),
            })
        }
        4 => Err(invalid(format!(
            "expected the reserved segment '{}' after the scope, found '{}'",
            KindScope::ISSUE_TOKEN,
            rest[0]
        ))),
        2 if rest[0] == KindScope::ISSUE_TOKEN => Err(invalid(format!(
            "'{}' is a reserved segment, not a kind name; use \
             @/issue/<short-id>/<kind>/<self-id> for issue-scoped addresses",
            KindScope::ISSUE_TOKEN
        ))),
        2 => {
            let (kind, self_id) = (rest[0], rest[1]);
            require_non_empty(kind, "kind", address)?;
            require_non_empty(self_id, "self-id", address)?;
            let scope = if project_name.is_empty() {
                AddressScope::Project
            } else {
                AddressScope::NamedProject(project_name.to_string())
            };
            Ok(KindSegmentedAddress {
                scope,
                kind: kind.to_string(),
                self_id: self_id.to_string(),
            })
        }
        other => Err(invalid(format!(
            "expected 2 segments (<kind>/<self-id>) or 4 segments \
             (issue/<short-id>/<kind>/<self-id>) after the scope, found {other}"
        ))),
    }
}

/// Reject an empty address segment, naming it `label` in the error.
fn require_non_empty(segment: &str, label: &str, address: &str) -> Result<(), ItemError> {
    if segment.is_empty() {
        Err(ItemError::InvalidAddress {
            address: address.to_string(),
            reason: format!("the {label} segment is empty"),
        })
    } else {
        Ok(())
    }
}

/// Expand a `<short-id>/<self-id>` sugar address into the canonical
/// kind-segmented triple by inferring the kind from `self_id`'s shape.
///
/// The sugar form omits the kind segment [`parse_kind_segmented_address`]
/// requires explicit; this recovers it by matching `self_id` against every
/// ISSUE-scoped kind's `id-pattern` in `kinds` (the same `id_pattern.find`
/// mechanism [`extract_raw_items`] uses to mint items). PROJECT-scoped kinds
/// (`scope = "project"`, e.g. `invariant`, `definition`) are excluded from
/// candidate matching regardless of whether their pattern would otherwise
/// match: their self-ids are free-form slugs with no distinguishing shape, so
/// they have no sugar form and always require the explicit
/// `@/<kind>/<self-id>` address.
///
/// This is a pure function over its `kinds` parameter — like
/// [`resolve_item_kinds`], it takes the kind registry as data rather than
/// hardcoding any kind name, so no kind identity is baked into this module.
///
/// - Exactly one issue-scoped kind matches: expands to a
///   [`KindSegmentedAddress`] with [`AddressScope::Issue`] carrying the
///   address's short-id, that kind's name, and `self_id` verbatim.
/// - No issue-scoped kind matches: [`ItemError::SugarKindNotFound`].
/// - More than one issue-scoped kind matches: [`ItemError::SugarKindAmbiguous`],
///   naming every matching candidate kind — this function never silently picks
///   the first.
///
/// `address` itself must split into two `/`-separated segments
/// (`<short-id>/<self-id>`); anything else is [`ItemError::InvalidAddress`]. A `:`
/// anywhere in `address` is also rejected as [`ItemError::InvalidAddress`]: the
/// colon is reserved for the label `namespace:value` separator and never valid
/// inside an address.
pub fn expand_sugar_address(
    address: &str,
    kinds: &[ItemKind],
) -> Result<KindSegmentedAddress, ItemError> {
    if address.contains(':') {
        return Err(ItemError::InvalidAddress {
            address: address.to_string(),
            reason: "the address contains a colon ':', which is reserved for the label \
                     'namespace:value' separator and is never valid inside an address"
                .to_string(),
        });
    }

    // Split on the FIRST `/` (a self-id may itself contain slashes), matching the
    // two-segment shape the sugar form requires; a value with no `/` is not a sugar
    // address at all.
    let (short_id, self_id) = address
        .split_once('/')
        .ok_or_else(|| ItemError::InvalidAddress {
            address: address.to_string(),
            reason: "sugar address must be '<short-id>/<self-id>'".to_string(),
        })?;

    let candidates: Vec<&ItemKind> = kinds
        .iter()
        .filter(|kind| kind.kind_scope() == KindScope::Issue)
        .filter(|kind| kind.id_pattern.find(self_id).is_some())
        .collect();

    match candidates.as_slice() {
        [] => Err(ItemError::SugarKindNotFound {
            address: address.to_string(),
            self_id: self_id.to_string(),
        }),
        [only] => Ok(KindSegmentedAddress {
            scope: AddressScope::Issue(short_id.to_string()),
            kind: only.name().to_string(),
            self_id: self_id.to_string(),
        }),
        _ => Err(ItemError::SugarKindAmbiguous {
            address: address.to_string(),
            self_id: self_id.to_string(),
            candidates: candidates
                .iter()
                .map(|kind| kind.name().to_string())
                .collect(),
        }),
    }
}

/// The declared addressing scope of an item *kind* (as opposed to a resolved
/// [`Scope`], which carries a concrete issue short-id).
///
/// A kind is either issue-scoped (its items come from issue descriptions) or
/// project-scoped (its items come from a config-declared `source` file and address
/// as `@/<kind>/<self-id>`). This is the `scope` half of the kind registry's six-tuple
/// the epic builds toward; sibling work adds the remaining `source-of-truth` field
/// without disturbing this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KindScope {
    /// Items are projected from issue descriptions
    /// (`@/issue/<short-id>/<kind>/<self-id>`).
    Issue,
    /// Items are projected from a config-declared source file
    /// (`@/<kind>/<self-id>`).
    Project,
}

impl KindScope {
    /// The token a kind declares its issue scope with.
    pub const ISSUE_TOKEN: &'static str = "issue";
    /// The token a kind declares its project scope with.
    pub const PROJECT_TOKEN: &'static str = "project";

    /// Parse a kind's optional `scope` config string.
    ///
    /// `None` defaults to [`KindScope::Issue`] (the prior, issue-scoped behavior).
    /// An unrecognized value is rejected with the kind name supplied by the caller
    /// via [`KindScope::parse_for`]; this `parse` variant uses a placeholder name
    /// in its error.
    pub fn parse(scope: Option<&str>) -> Result<Self, ItemError> {
        Self::parse_for("<kind>", scope)
    }

    /// Parse a kind's optional `scope` config string, naming `kind` in any error.
    pub fn parse_for(kind: &str, scope: Option<&str>) -> Result<Self, ItemError> {
        match scope {
            None => Ok(KindScope::Issue),
            Some(Self::ISSUE_TOKEN) => Ok(KindScope::Issue),
            Some(Self::PROJECT_TOKEN) => Ok(KindScope::Project),
            Some(other) => Err(ItemError::InvalidScope {
                kind: kind.to_string(),
                scope: other.to_string(),
            }),
        }
    }

    /// Whether this kind is project-scoped (`@`).
    pub fn is_project(&self) -> bool {
        matches!(self, KindScope::Project)
    }
}

/// Errors raised while resolving item kinds or indexing items.
#[derive(Debug, Error)]
pub enum ItemError {
    /// A kind's `id-pattern` is not a valid regular expression.
    #[error("item kind '{kind}' has an invalid id-pattern '{pattern}': {source}")]
    InvalidIdPattern {
        /// The offending kind name.
        kind: String,
        /// The pattern string that failed to compile.
        pattern: String,
        /// The underlying regex compilation error.
        source: regex::Error,
    },
    /// Two addressable items of the SAME kind in one scope share a self-id
    /// (REQ-03). The same self-id under a *different* scope, or under a
    /// *different* kind in the same scope, is fine (REQ-04).
    #[error(
        "scope {scope} declares self-id '{self_id}' more than once for kind '{kind}'; \
         self-ids must be unique within a scope for a given kind"
    )]
    DuplicateSelfId {
        /// The scope whose items collide (issue short-id or `@` for project).
        scope: String,
        /// The duplicated self-id.
        self_id: String,
        /// The kind under which the collision occurred.
        kind: String,
    },
    /// The body could not be parsed for the issue's content format.
    #[error("failed to parse description of issue {issue}: {source}")]
    Parse {
        /// Short id of the issue whose body failed to parse.
        issue: String,
        /// The underlying parser error.
        source: crate::document::ContentParserError,
    },
    /// A kind declares an unrecognized `scope` (not `issue` or `project`).
    #[error("item kind '{kind}' has an invalid scope '{scope}'; expected 'issue' or 'project'")]
    InvalidScope {
        /// The offending kind name.
        kind: String,
        /// The unrecognized scope string.
        scope: String,
    },
    /// A `scope = "project"` kind declares no `source` for the substrate its
    /// `source-of-truth` reads from, so it cannot be indexed at project scope: a
    /// markdown-first kind needs a markdown `source` path, a registry-first kind
    /// needs a structured toml `source` descriptor. Either way an absent source
    /// declaration is a typed error rather than a silent empty result.
    #[error(
        "project-scope item kind '{kind}' declares no 'source'; \
         a project-scope kind must name a repository-local source \
         (a markdown file for markdown-first, or a toml descriptor for registry-first)"
    )]
    MissingProjectSource {
        /// The offending kind name.
        kind: String,
    },
    /// A reference (e.g. a rule's `kind =` key) names a kind not declared in the
    /// `[item_kinds]` registry, so it cannot be expanded to a triple. Surfaced as
    /// a typed error rather than a silent pass.
    #[error(
        "item kind '{kind}' is not declared in [item_kinds]; \
         declare it or reference an existing kind"
    )]
    UnknownKind {
        /// The undeclared kind name that was referenced.
        kind: String,
    },
    /// The `.toml` file backing a registry-first kind's source descriptor is not
    /// valid TOML, so its entries cannot be projected.
    #[error("item kind '{kind}' toml source '{path}' is not valid TOML: {source}")]
    TomlSourceParse {
        /// The kind whose descriptor names the file.
        kind: String,
        /// The repository-local path of the offending `.toml` file.
        path: String,
        /// The underlying TOML parse error (boxed to keep [`ItemError`] small).
        source: Box<toml::de::Error>,
    },
    /// An entry of a registry-first kind's source table is missing its mapped
    /// `id-field`, so no self-id can be projected for that entry. (The
    /// `text-field` is optional and falls back to the id-field value when absent,
    /// so a missing text-field does not raise this error.)
    #[error(
        "item kind '{kind}' toml source table '{table}' has an entry missing required \
         field '{field}'"
    )]
    TomlSourceMissingField {
        /// The kind whose descriptor names the table.
        kind: String,
        /// The array-of-tables key being projected.
        table: String,
        /// The descriptor-mapped field absent from the entry.
        field: String,
    },
    /// A descriptor-mapped field of a registry-first kind's source has an
    /// unexpected TOML type (e.g. an `id-field` that is not a string, or a
    /// `link-fields` value that is neither a string nor an array of strings).
    #[error(
        "item kind '{kind}' toml source table '{table}' field '{field}' has an \
         unexpected type: expected {expected}"
    )]
    TomlSourceFieldType {
        /// The kind whose descriptor names the table.
        kind: String,
        /// The array-of-tables key being projected.
        table: String,
        /// The mapped field with the wrong type.
        field: String,
        /// A short description of the type(s) the loader accepts.
        expected: String,
    },
    /// A kind-segmented address (`@/<kind>/<self-id>`, `@<name>/<kind>/<self-id>`,
    /// or `@/issue/<short-id>/<kind>/<self-id>`) failed to parse: a missing kind
    /// or self-id segment, an empty scope/kind/self-id segment, an unrecognized
    /// reserved segment where `issue` was expected, or the wrong number of
    /// `/`-separated segments.
    #[error("address '{address}' is not a valid kind-segmented address: {reason}")]
    InvalidAddress {
        /// The raw address string that failed to parse.
        address: String,
        /// Human-readable description of why parsing failed.
        reason: String,
    },
    /// A `<short-id>/<self-id>` sugar address's self-id matched NO issue-scoped
    /// item kind's `id-pattern`, so [`expand_sugar_address`] cannot infer which
    /// kind the address belongs to.
    #[error(
        "self-id '{self_id}' in sugar address '{address}' matches no issue-scoped \
         item kind's id-pattern"
    )]
    SugarKindNotFound {
        /// The full sugar address that failed to expand.
        address: String,
        /// The self-id segment that matched no issue-scoped kind.
        self_id: String,
    },
    /// A `<short-id>/<self-id>` sugar address's self-id matched MORE THAN ONE
    /// issue-scoped item kind's `id-pattern`, so [`expand_sugar_address`] cannot
    /// pick a kind without guessing. Every matching candidate kind is named so
    /// the caller can disambiguate — e.g. by using the explicit
    /// `@/issue/<short-id>/<kind>/<self-id>` address instead.
    #[error(
        "self-id '{self_id}' in sugar address '{address}' matches more than one \
         issue-scoped item kind's id-pattern: {}",
        candidates.join(", ")
    )]
    SugarKindAmbiguous {
        /// The full sugar address that failed to expand.
        address: String,
        /// The self-id segment that matched multiple issue-scoped kinds.
        self_id: String,
        /// Every issue-scoped kind name whose id-pattern matched, in
        /// kind-list order.
        candidates: Vec<String>,
    },
    /// A named-project address `@<name>/...` named a project OTHER than the
    /// locally declared one, or no local project name is declared at all, so
    /// [`AddressScope::bind_local`] could not bind it to local project
    /// identity. Resolution is local-only: no federation or remote lookup was
    /// attempted for `project`.
    #[error(
        "project '{project}' is not resolvable here: resolution is local-only \
         and no federation or remote lookup was attempted; only the locally \
         declared project name binds to '@'"
    )]
    NotResolvable {
        /// The named-project token that could not be bound to any local
        /// identity.
        project: String,
    },
}

/// A resolved item-kind projection: the `(section, id-pattern, markers,
/// link-namespaces)` four-tuple with all defaults already applied.
///
/// Resolved from an [`ItemKindConfig`] via [`ItemKind::from_config`]. The
/// `id_pattern` is pre-compiled so indexing is regex-error-free.
///
/// # Examples
///
/// ```
/// use jit::config::ItemKindConfig;
/// use jit::domain::item::ItemKind;
///
/// let kind = ItemKind::from_config(
///     "example",
///     &ItemKindConfig {
///         section: Some("success_criteria".into()),
///         id_pattern: Some("[A-Z][A-Z0-9]*-[0-9]+".into()),
///         markers: Some(vec!["[hard]".into()]),
///         link_namespaces: Some(vec!["satisfies".into()]),
///         ..Default::default()
///     },
/// )
/// .unwrap();
/// assert_eq!(kind.name(), "example");
/// // The kind exposes the same triple the label-coverage rule consumes.
/// let (section, marker, pattern) = kind.as_triple();
/// assert_eq!(section, "success_criteria");
/// assert_eq!(marker, Some("[hard]"));
/// assert_eq!(pattern, "[A-Z][A-Z0-9]*-[0-9]+");
/// ```
#[derive(Debug, Clone)]
pub struct ItemKind {
    name: String,
    section: String,
    id_pattern_src: String,
    id_pattern: regex::Regex,
    markers: Vec<String>,
    link_namespaces: Vec<String>,
    kind_scope: KindScope,
    source_path: Option<String>,
    toml_source: Option<TomlSourceDescriptor>,
    source_of_truth: SourceOfTruth,
    aliases: Vec<String>,
}

impl ItemKind {
    /// Resolve a configured kind into its four-tuple, applying repo defaults for
    /// any field the config leaves unset.
    ///
    /// `name` labels the kind (for display and `--kind` filtering only). An
    /// invalid `id-pattern` regex is surfaced as [`ItemError::InvalidIdPattern`]
    /// rather than silently dropped.
    pub fn from_config(name: &str, config: &ItemKindConfig) -> Result<Self, ItemError> {
        let section = config
            .section
            .clone()
            .unwrap_or_else(|| DEFAULT_ITEM_SECTION.to_string());
        let id_pattern_src = config
            .id_pattern
            .clone()
            .unwrap_or_else(|| DEFAULT_ITEM_ID_PATTERN.to_string());
        let id_pattern =
            regex::Regex::new(&id_pattern_src).map_err(|source| ItemError::InvalidIdPattern {
                kind: name.to_string(),
                pattern: id_pattern_src.clone(),
                source,
            })?;
        let markers = config.markers.clone().unwrap_or_default();
        let link_namespaces = config
            .link_namespaces
            .clone()
            .unwrap_or_else(|| vec![DEFAULT_ITEM_LINK_NAMESPACE.to_string()]);
        // Scope is now a typed field — invalid tokens are rejected at TOML parse
        // time, so this conversion is infallible.
        let kind_scope = match config.scope {
            None | Some(KindScopeConfig::Issue) => KindScope::Issue,
            Some(KindScopeConfig::Project) => KindScope::Project,
        };
        let source_of_truth = config.source_of_truth();
        // Split the polymorphic `source` into the markdown PATH and the structured
        // toml DESCRIPTOR; at most one is ever set (the two shapes are mutually
        // exclusive at parse time).
        let (source_path, toml_source) = match &config.source {
            Some(ItemKindSource::Path(path)) => (Some(path.clone()), None),
            Some(ItemKindSource::Toml(descriptor)) => (None, Some(descriptor.clone())),
            None => (None, None),
        };
        // A project-scope kind must declare the `source` its `source-of-truth`
        // reads from, or it cannot be indexed at project scope: a markdown-first
        // kind needs a markdown `source` PATH, a registry-first kind needs a
        // structured toml `source` DESCRIPTOR. Reject a missing declaration at
        // resolution so a misconfigured kind surfaces a typed error, not a silent
        // empty result. The check is symmetric across both directions and branches
        // only on `source-of-truth`, never on any kind NAME, so the engine
        // hardcodes no domain concept (REQ-03).
        let has_source = match source_of_truth {
            SourceOfTruth::MarkdownFirst => source_path.is_some(),
            SourceOfTruth::RegistryFirst => toml_source.is_some(),
        };
        if kind_scope.is_project() && !has_source {
            return Err(ItemError::MissingProjectSource {
                kind: name.to_string(),
            });
        }
        let aliases = config.aliases.clone().unwrap_or_default();
        Ok(Self {
            name: name.to_string(),
            section,
            id_pattern_src,
            id_pattern,
            markers,
            link_namespaces,
            kind_scope,
            source_path,
            toml_source,
            source_of_truth,
            aliases,
        })
    }

    /// The kind's display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The kind's config-declared aliases: shorthand names it may ALSO be
    /// addressed by, beyond its registry [`name`](Self::name). Empty when the
    /// kind declares none. Aliases are input sugar only — canonical output always
    /// uses the registry name.
    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }

    /// Whether `name` addresses this kind by its registry [`name`](Self::name) or
    /// any of its declared [`aliases`](Self::aliases).
    pub fn matches_name(&self, name: &str) -> bool {
        self.name == name || self.aliases.iter().any(|alias| alias == name)
    }

    /// The section slug scanned for this kind's items.
    pub fn section(&self) -> &str {
        &self.section
    }

    /// The markers an item must begin with to qualify (empty = any item).
    pub fn markers(&self) -> &[String] {
        &self.markers
    }

    /// The link-label namespaces that reference this kind by qualified id.
    pub fn link_namespaces(&self) -> &[String] {
        &self.link_namespaces
    }

    /// The kind's declared addressing scope (issue or project).
    pub fn kind_scope(&self) -> KindScope {
        self.kind_scope
    }

    /// The repository-local MARKDOWN source file a project-scope, markdown-first
    /// kind reads its items from, or `None` for an issue-scope kind or a
    /// registry-first kind backed by a [`toml_source`](Self::toml_source)
    /// descriptor.
    pub fn source(&self) -> Option<&str> {
        self.source_path.as_deref()
    }

    /// The structured TOML source descriptor a registry-first project kind reads
    /// its items from, or `None` for an issue-scope or markdown-first kind. A
    /// registry-first project kind returns `Some(descriptor)` naming the `.toml`
    /// registry its items are projected from (its `source` is a
    /// `{ toml = "...", table = "...", ... }` descriptor).
    ///
    /// When present, [`commands`](crate::commands) reads the descriptor's `toml`
    /// file through the storage boundary and projects each table entry into an
    /// addressable item via [`load_toml_scope_items`].
    pub fn toml_source(&self) -> Option<&crate::config::TomlSourceDescriptor> {
        self.toml_source.as_ref()
    }

    /// The kind's authoring DIRECTION (which substrate is canonical).
    ///
    /// `markdown-first` kinds are parsed from a markdown substrate (issue
    /// descriptions, or a project-scope `source` file); `registry-first` kinds are
    /// projected from a structured registry. Callers route the sourcing path on
    /// this value.
    pub fn source_of_truth(&self) -> SourceOfTruth {
        self.source_of_truth
    }

    /// The kind as the `(section, marker, id-pattern)` triple consumed by the
    /// validation engine's `criterion_ids` / `label-coverage` machinery.
    ///
    /// Only the FIRST marker is returned: the engine's coverage rule accepts a
    /// single `marker`, and this triple is what proves model/rule compatibility
    /// (REQ-05). A kind with no markers yields `None`.
    pub fn as_triple(&self) -> (&str, Option<&str>, &str) {
        (
            &self.section,
            self.markers.first().map(String::as_str),
            &self.id_pattern_src,
        )
    }

    /// Whether an item's text qualifies under this kind's markers.
    ///
    /// True when the kind declares no markers, or the text — after leading
    /// whitespace and at most one leading GitHub-style checkbox token (see
    /// [`text_after_checkbox`]) — begins with ANY declared marker.
    fn marker_matches(&self, text: &str) -> bool {
        self.markers.is_empty() || {
            let candidate = text_after_checkbox(text);
            self.markers.iter().any(|m| candidate.starts_with(m))
        }
    }
}

/// A leading GitHub-style task-list checkbox token skipped before a marker check.
const CHECKBOX_TOKENS: [&str; 3] = ["[ ]", "[x]", "[X]"];

/// `text`, minus one leading GitHub-style task-list checkbox token, if present.
///
/// The markdown parser used for criteria projection
/// ([`MarkdownContentParser`](crate::document::MarkdownContentParser)) does not
/// enable pulldown-cmark's task-list extension, so a checkbox-style bullet
/// (`- [ ] [hard] REQ-01: ...`) projects the checkbox as literal item text
/// (`"[ ] [hard] REQ-01: ..."`) rather than stripping it. A start-anchored marker
/// check (`starts_with("[hard]")`) run directly against that text never matches,
/// silently dropping the criterion from every marker-gated rule (`label-coverage`,
/// `criteria-to-check`, `criteria-label-match`) and from `jit item list`
/// (jit:16402e14). Skipping at most one checkbox token — `[ ]`, `[x]`, or `[X]` —
/// before the marker check tolerates the checkbox without changing behavior for
/// text that never had one.
pub(crate) fn text_after_checkbox(text: &str) -> &str {
    let trimmed = text.trim_start();
    CHECKBOX_TOKENS
        .iter()
        .find_map(|token| trimmed.strip_prefix(token))
        .map(str::trim_start)
        .unwrap_or(trimmed)
}

/// Mint an addressable item's canonical qualified id under the uniform
/// kind-segmented scheme (epic 2821e177 REQ-01):
///
/// - project scope: `@/<kind>/<self-id>`
/// - issue scope: `@/issue/<short-id>/<kind>/<self-id>` (`issue` is the reserved
///   built-in segment, never a kind name)
///
/// A pure projection over the scope, the kind, and the parsed self-id; nothing is
/// persisted (REQ-05). Every minted id carries its kind segment, so the string is
/// unique per `(scope, kind, self-id)` — the address round-trips through
/// [`parse_kind_segmented_address`] and resolves via
/// [`show_item`](crate::commands::CommandExecutor::show_item). The
/// `<short-id>/<self-id>` sugar remains an accepted INPUT form (expanded by
/// [`expand_sugar_address`]) but is never minted.
pub fn qualified_id(scope: &Scope, kind: &str, self_id: &str) -> String {
    match scope {
        Scope::Project => format!("{PROJECT_SCOPE_SENTINEL}/{kind}/{self_id}"),
        Scope::Issue(short_id) => format!(
            "{PROJECT_SCOPE_SENTINEL}/{}/{short_id}/{kind}/{self_id}",
            KindScope::ISSUE_TOKEN
        ),
    }
}

/// Whether `value` is a qualified item reference under the address grammar,
/// rather than a bare self-id.
///
/// Used to classify the value half of a `<namespace>:<value>` link label: a
/// qualified value addresses a specific item and is resolved through
/// [`expand_sugar_address`] / [`parse_kind_segmented_address`], whereas a bare
/// self-id (`REQ-01`) is left to the legacy unqualified coverage rules.
///
/// Every address form carries at least one `/`: the explicit `@`-prefixed
/// kind-segmented forms (`@/<kind>/<self-id>`,
/// `@/issue/<short-id>/<kind>/<self-id>`) and the `<short-id>/<self-id>` sugar. A
/// bare self-id has no `/`. That single structural distinction is the
/// qualified/unqualified boundary.
pub fn is_qualified_reference(value: &str) -> bool {
    value.contains('/')
}

/// One addressable item projected from a scope's source.
///
/// Carries the *derived* qualified id alongside the source self-id, the owning
/// scope prefix (an issue short-id or `@`), the kind name, and the raw item text.
/// Nothing here is stored — it is recomputed on demand by the indexers (REQ-05).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AddressableItem {
    /// The kind this item belongs to (display name).
    pub kind: String,
    /// Derived qualified id in the uniform kind-segmented scheme:
    /// `@/<kind>/<self-id>` for a project item, `@/issue/<short-id>/<kind>/<self-id>`
    /// for an issue item. Because the kind is a segment, the string is unique per
    /// `(scope, kind, self-id)`: two items of different kinds sharing a scope and
    /// self-id derive DISTINCT qualified ids (`@/rule/coverage-preview` vs
    /// `@/gate/coverage-preview`).
    pub qualified_id: String,
    /// The human-authored self-id, unique within its scope for a given kind.
    pub self_id: String,
    /// Scope prefix this item was projected from: an issue short-id, or `@` for
    /// the project scope.
    pub scope: String,
    /// The raw text of the source list entry.
    pub text: String,
    /// `<namespace>:<target>` link labels the item carries, projected from a
    /// registry-first kind's `link-fields` mapping. Empty for markdown-sourced and
    /// invariant items (links there are carried by the linking NODE, not the item),
    /// in which case the field is omitted from serialized output.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
}

/// A candidate addressable item extracted from a scope's source, before
/// per-(scope, kind) uniqueness has been enforced and the qualified id derived.
///
/// This is the single shape every substrate (issue-scope markdown, project-scope
/// registry) funnels into [`derive_scope_items`], so the dedup + qualified-id
/// derivation lives in exactly one place (REQ-03, REQ-04, REQ-05).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawScopeItem {
    /// The kind this candidate belongs to (display name).
    pub kind: String,
    /// The human-authored self-id.
    pub self_id: String,
    /// The raw source text of the item.
    pub text: String,
    /// `<namespace>:<target>` link labels carried by the item (empty for markdown
    /// substrates; populated by a registry-first kind's `link-fields` mapping).
    pub links: Vec<String>,
}

/// Enforce per-(scope, kind) self-id uniqueness over raw candidates and derive
/// their uniform kind-segmented qualified ids (`@/<kind>/<self-id>` for project
/// scope, `@/issue/<short-id>/<kind>/<self-id>` for issue scope) via
/// [`qualified_id`] (REQ-03, REQ-04, REQ-05).
///
/// This is the one code path that turns extracted candidates into addressable
/// items, shared by [`index_items`] (issue scope) and [`index_markdown_items`]
/// (any scope, including project). Uniqueness is keyed on the pair `(self_id,
/// kind)`: two DIFFERENT kinds minting the same self-id in one scope coexist as
/// distinct items and derive distinct qualified-id strings (the kind is a segment
/// of the minted id), while a self-id repeated under the SAME kind in one scope is
/// a [`ItemError::DuplicateSelfId`]. The same self-id under a *different* scope is
/// fine because each call is scoped to one [`Scope`] (REQ-04).
pub fn derive_scope_items(
    scope: &Scope,
    raw: Vec<RawScopeItem>,
) -> Result<Vec<AddressableItem>, ItemError> {
    let prefix = scope.prefix();
    let mut out = Vec::with_capacity(raw.len());
    // Tracks each claimed (self-id, kind) pair. Scoped to this single scope, so
    // the same self-id under a different scope never collides here (REQ-04); the
    // kind is part of the key, so two different kinds may claim the same self-id
    // in this scope without colliding.
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for candidate in raw {
        if !seen.insert((candidate.self_id.clone(), candidate.kind.clone())) {
            return Err(ItemError::DuplicateSelfId {
                scope: prefix.to_string(),
                self_id: candidate.self_id,
                kind: candidate.kind,
            });
        }
        out.push(AddressableItem {
            qualified_id: qualified_id(scope, &candidate.kind, &candidate.self_id),
            scope: prefix.to_string(),
            kind: candidate.kind,
            self_id: candidate.self_id,
            text: candidate.text,
            links: candidate.links,
        });
    }
    Ok(out)
}

/// Index the addressable items of a single issue across all given kinds.
///
/// A pure projection: parses the issue description with `parser`, and for each
/// kind scans its declared section's list entries, keeping those that match the
/// kind's markers and yield a self-id under its id-pattern. The qualified id is
/// derived as `@/issue/<issue-short-id>/<kind>/<self-id>` via the shared
/// [`derive_scope_items`].
///
/// Self-id uniqueness is enforced per (scope, kind) (here, the issue): a self-id
/// repeated under the SAME kind is an [`ItemError::DuplicateSelfId`]; the same
/// self-id under a *different* kind coexists. A list entry with no self-id match
/// is plain prose and is skipped, never an error (REQ-06).
///
/// # Examples
///
/// ```
/// use jit::config::ItemKindConfig;
/// use jit::document::MarkdownContentParser;
/// use jit::domain::item::{index_items, ItemKind};
/// use jit::domain::Issue;
///
/// let issue = Issue::draft(
///     "T".to_string(),
///     "## Success Criteria\n\n- [hard] REQ-01: a\n- prose line\n".to_string(),
/// );
/// let kind = ItemKind::from_config(
///     "example",
///     &ItemKindConfig {
///         section: Some("success_criteria".into()),
///         id_pattern: Some("REQ-[0-9]+".into()),
///         markers: Some(vec!["[hard]".into()]),
///         ..Default::default()
///     },
/// )
/// .unwrap();
/// let items = index_items(&issue, &[kind], &MarkdownContentParser).unwrap();
/// assert_eq!(items.len(), 1);
/// assert_eq!(items[0].self_id, "REQ-01");
/// assert!(items[0].qualified_id.ends_with("/REQ-01"));
/// ```
pub fn index_items(
    issue: &Issue,
    kinds: &[ItemKind],
    parser: &dyn ContentParser,
) -> Result<Vec<AddressableItem>, ItemError> {
    let projection = project(issue).with_sections(&issue.description, parser);
    let sections = projection.sections.unwrap_or_default();
    let raw = extract_raw_items(&sections, kinds);
    derive_scope_items(&Scope::Issue(issue.short_id()), raw)
}

/// Extract raw item candidates from already-parsed sections across all kinds.
///
/// The single section-scanning path shared by every markdown substrate
/// ([`index_items`] for issue descriptions, [`index_markdown_items`] for a
/// project-scope source file): for each kind it scans its declared section's list
/// entries, keeps those that match the kind's markers, and extracts the self-id
/// under its id-pattern. A line with no self-id match is plain prose and is
/// skipped, never an error (REQ-06). Per-(scope, kind) uniqueness is NOT enforced
/// here — that is [`derive_scope_items`]' job — so this stays a pure, reusable
/// scanner.
fn extract_raw_items(
    sections: &std::collections::BTreeMap<String, crate::domain::ProjectedSection>,
    kinds: &[ItemKind],
) -> Vec<RawScopeItem> {
    kinds
        .iter()
        .filter_map(|kind| sections.get(kind.section()).map(|section| (kind, section)))
        .flat_map(|(kind, section)| {
            section.items.iter().filter_map(move |text| {
                if !kind.marker_matches(text) {
                    return None;
                }
                // No self-id match means plain prose: skip, never error (REQ-06).
                let self_id = kind.id_pattern.find(text).map(|m| m.as_str().to_string())?;
                Some(RawScopeItem {
                    kind: kind.name().to_string(),
                    self_id,
                    text: text.clone(),
                    // Markdown items carry no item-side links: a markdown link lives
                    // on the linking NODE's labels, not the addressed item.
                    links: Vec::new(),
                })
            })
        })
        .collect()
}

/// Index the addressable items of a scope from a standalone markdown source.
///
/// Project-scoped kinds are markdown-first, sourced from a repository-local file
/// declared in config (the path comes only from config — no filename is
/// hardcoded). This parses `markdown` with the SAME [`ContentParser`] and
/// section-scanning path ([`extract_raw_items`]) as issue descriptions, then runs
/// the candidates through the SAME [`derive_scope_items`] derivation, so
/// qualified-id derivation and per-(scope, kind) uniqueness (REQ-03, REQ-04,
/// REQ-05) are identical across substrates. With `scope = Scope::Project` each item's
/// qualified id is `@/<kind>/<self-id>` and resolution of `@/<kind>/<self-id>` finds
/// it (REQ-01).
pub fn index_markdown_items(
    markdown: &str,
    scope: &Scope,
    kinds: &[ItemKind],
    parser: &dyn ContentParser,
) -> Result<Vec<AddressableItem>, ItemError> {
    let sections = sections_from_markdown(markdown, parser);
    let raw = extract_raw_items(&sections, kinds);
    derive_scope_items(scope, raw)
}

/// Parse standalone markdown into the section map [`extract_raw_items`] consumes.
///
/// A thin wrapper over the [`ContentParser`] reusing the exact section model the
/// issue projection uses, so a project-scope source file is scanned identically to
/// an issue description.
fn sections_from_markdown(
    markdown: &str,
    parser: &dyn ContentParser,
) -> std::collections::BTreeMap<String, crate::domain::ProjectedSection> {
    parser
        .parse(markdown)
        .sections
        .into_iter()
        .map(|(name, section)| (name, section.into()))
        .collect()
}

/// One project-scope source: a kind paired with the markdown text of its
/// config-declared `source` file.
///
/// [`index_project_sources`] consumes these so several project-scope kinds (each
/// reading its own file) are deduped together under the single `@` scope.
#[derive(Debug, Clone)]
pub struct ProjectSource {
    /// The project-scope kind whose items this source holds.
    pub kind: ItemKind,
    /// The markdown text of the kind's `source` file (already read from disk).
    pub markdown: String,
}

/// Index every project-scope (`@`) substrate through ONE per-(scope, kind) dedup
/// pass.
///
/// Two substrates feed the project scope and BOTH funnel through the SAME single
/// [`derive_scope_items`] call here, so per-(scope, kind) uniqueness and
/// qualified-id derivation are identical across them (REQ-03, REQ-04, REQ-05):
///
/// 1. **Markdown-first** kinds (each a [`ProjectSource`]) are parsed and scanned
///    via the same [`extract_raw_items`] path as issue descriptions.
/// 2. **Registry-first** kinds supply their candidates directly as
///    `registry_items` — already projected from a structured registry, NOT a
///    markdown section, since the registry is their authoritative source (REQ-02).
///
/// Pooling all candidates before deriving means a self-id repeated across a
/// markdown source and a registry candidate of the SAME kind is reported as a
/// duplicate (REQ-03); different kinds sharing a self-id coexist and derive
/// distinct qualified-id strings (the kind is a segment). Qualified-id derivation
/// matches issue scope (REQ-01, REQ-05). Empty inputs yield no items (graceful),
/// never an error.
///
/// # Examples
///
/// ```
/// use jit::config::ItemKindConfig;
/// use jit::document::MarkdownContentParser;
/// use jit::domain::item::{index_project_sources, ItemKind, ProjectSource, RawScopeItem};
///
/// // A markdown-first kind (`definition`, sourced from the glossary).
/// let kind = ItemKind::from_config(
///     "definition",
///     &ItemKindConfig {
///         section: Some("core_concepts".into()),
///         id_pattern: Some("[A-Z][a-z]+".into()),
///         ..Default::default()
///     },
/// )
/// .unwrap();
/// let sources = vec![ProjectSource {
///     kind,
///     markdown: "## Core Concepts\n\n- State: current lifecycle stage\n".to_string(),
/// }];
/// // A registry-first candidate (an invariant) is supplied directly.
/// let registry = vec![RawScopeItem {
///     kind: "invariant".to_string(),
///     self_id: "atomic-writes".to_string(),
///     text: "all writes are atomic".to_string(),
///     links: Vec::new(),
/// }];
/// let items = index_project_sources(&sources, registry, &MarkdownContentParser).unwrap();
/// let qids: Vec<&str> = items.iter().map(|i| i.qualified_id.as_str()).collect();
/// assert!(qids.contains(&"@/definition/State"));
/// assert!(qids.contains(&"@/invariant/atomic-writes"));
/// ```
pub fn index_project_sources(
    sources: &[ProjectSource],
    registry_items: Vec<RawScopeItem>,
    parser: &dyn ContentParser,
) -> Result<Vec<AddressableItem>, ItemError> {
    let mut raw: Vec<RawScopeItem> = sources
        .iter()
        .flat_map(|source| {
            let sections = sections_from_markdown(&source.markdown, parser);
            extract_raw_items(&sections, std::slice::from_ref(&source.kind))
        })
        .collect();
    raw.extend(registry_items);
    derive_scope_items(&Scope::Project, raw)
}

/// Project a registry-first kind's `.toml` source into raw scope candidates
/// through its declared field mapping.
///
/// This is the generic analogue of the hard-wired invariant projection: it takes
/// the already-read `toml_content` (the file I/O happens in the command layer
/// through the storage boundary, so this stays a PURE function) and maps each
/// entry of the descriptor's named array-of-tables into a [`RawScopeItem`]:
/// `id-field` -> self-id (so the derived qualified id is `@/<kind>/<self-id>`),
/// `text-field` -> text, and each `link-fields` entry -> `<namespace>:<target>`
/// link labels (the mapped field may be a single string or an array of strings).
/// `kind_name` tags every candidate so the derived item reports the right kind.
///
/// Graceful vs. typed-error contract:
/// - A missing `table` key yields NO items (an empty registry, like an absent
///   file), never an error.
/// - A malformed file is [`ItemError::TomlSourceParse`].
/// - An entry missing the mapped `id-field` is
///   [`ItemError::TomlSourceMissingField`]; the `text-field` is OPTIONAL and,
///   when absent, falls back to the id-field value (so a description-less rule
///   projects its `name` as display text). A mapped field of an unexpected TOML
///   type is [`ItemError::TomlSourceFieldType`].
/// - An entry that simply lacks a mapped LINK field contributes no labels for it
///   (graceful) — only the addressing fields are mandatory.
///
/// # Examples
///
/// ```
/// use jit::config::TomlSourceDescriptor;
/// use jit::domain::item::load_toml_scope_items;
///
/// let descriptor = TomlSourceDescriptor {
///     toml: "policies.toml".into(),
///     table: "policies".into(),
///     id_field: "id".into(),
///     text_field: "statement".into(),
///     link_fields: [("enforces".to_string(), "enforced-by".to_string())]
///         .into_iter()
///         .collect(),
/// };
/// let content = "\
/// [[policies]]\n\
/// id = \"POL-01\"\n\
/// statement = \"all writes are atomic\"\n\
/// enforced-by = [\"cargo-ci\"]\n";
/// let rows = load_toml_scope_items("policy", &descriptor, content).unwrap();
/// assert_eq!(rows.len(), 1);
/// assert_eq!(rows[0].self_id, "POL-01");
/// assert_eq!(rows[0].links, vec!["enforces:cargo-ci".to_string()]);
/// ```
pub fn load_toml_scope_items(
    kind_name: &str,
    descriptor: &TomlSourceDescriptor,
    toml_content: &str,
) -> Result<Vec<RawScopeItem>, ItemError> {
    let table: toml::Table =
        toml::from_str(toml_content).map_err(|source| ItemError::TomlSourceParse {
            kind: kind_name.to_string(),
            path: descriptor.toml.clone(),
            source: Box::new(source),
        })?;
    // A missing table is an empty registry (graceful), mirroring an absent file.
    let Some(entries) = table.get(&descriptor.table) else {
        return Ok(Vec::new());
    };
    let entries = entries
        .as_array()
        .ok_or_else(|| ItemError::TomlSourceFieldType {
            kind: kind_name.to_string(),
            table: descriptor.table.clone(),
            field: descriptor.table.clone(),
            expected: "an array of tables".to_string(),
        })?;
    entries
        .iter()
        .map(|entry| project_toml_entry(kind_name, descriptor, entry))
        .collect()
}

/// Map one TOML table entry into a [`RawScopeItem`] through `descriptor`'s field
/// mapping (the per-entry core of [`load_toml_scope_items`]).
fn project_toml_entry(
    kind_name: &str,
    descriptor: &TomlSourceDescriptor,
    entry: &toml::Value,
) -> Result<RawScopeItem, ItemError> {
    let self_id = required_toml_str(kind_name, descriptor, entry, &descriptor.id_field)?;
    // The text field is OPTIONAL: an entry lacking it falls back to its self-id
    // (the id-field value), so a description-less rule projects its `name` as
    // display text. A present-but-wrong-type field is still a typed error.
    let text = optional_toml_str(kind_name, descriptor, entry, &descriptor.text_field)?
        .unwrap_or_else(|| self_id.clone());
    // Link fields iterate in namespace order (BTreeMap) for deterministic labels.
    let links = descriptor
        .link_fields
        .iter()
        .map(|(namespace, field)| toml_link_labels(kind_name, descriptor, entry, namespace, field))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    Ok(RawScopeItem {
        kind: kind_name.to_string(),
        self_id,
        text,
        links,
    })
}

/// Read a required string `field` from a TOML entry, or a typed error naming the
/// missing field / wrong type.
fn required_toml_str(
    kind_name: &str,
    descriptor: &TomlSourceDescriptor,
    entry: &toml::Value,
    field: &str,
) -> Result<String, ItemError> {
    let value = entry
        .get(field)
        .ok_or_else(|| ItemError::TomlSourceMissingField {
            kind: kind_name.to_string(),
            table: descriptor.table.clone(),
            field: field.to_string(),
        })?;
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ItemError::TomlSourceFieldType {
            kind: kind_name.to_string(),
            table: descriptor.table.clone(),
            field: field.to_string(),
            expected: "a string".to_string(),
        })
}

/// Read an OPTIONAL string `field` from a TOML entry: `Ok(None)` when the field
/// is absent (the caller supplies a fallback), `Ok(Some(_))` when present as a
/// string, and a typed [`ItemError::TomlSourceFieldType`] when present as a
/// non-string. Used for the display `text-field`, which falls back to the
/// id-field value when unset (so a description-less rule projects its name).
fn optional_toml_str(
    kind_name: &str,
    descriptor: &TomlSourceDescriptor,
    entry: &toml::Value,
    field: &str,
) -> Result<Option<String>, ItemError> {
    match entry.get(field) {
        None => Ok(None),
        Some(value) => value.as_str().map(|s| Some(s.to_string())).ok_or_else(|| {
            ItemError::TomlSourceFieldType {
                kind: kind_name.to_string(),
                table: descriptor.table.clone(),
                field: field.to_string(),
                expected: "a string".to_string(),
            }
        }),
    }
}

/// Project one mapped link `field` of a TOML entry into `<namespace>:<target>`
/// labels (a single string or an array of strings); an absent field is graceful.
fn toml_link_labels(
    kind_name: &str,
    descriptor: &TomlSourceDescriptor,
    entry: &toml::Value,
    namespace: &str,
    field: &str,
) -> Result<Vec<String>, ItemError> {
    let field_type_err = || ItemError::TomlSourceFieldType {
        kind: kind_name.to_string(),
        table: descriptor.table.clone(),
        field: field.to_string(),
        expected: "a string or an array of strings".to_string(),
    };
    // An absent link field contributes no labels (graceful).
    let Some(value) = entry.get(field) else {
        return Ok(Vec::new());
    };
    let targets: Vec<&str> = match value {
        toml::Value::String(single) => vec![single.as_str()],
        toml::Value::Array(items) => items
            .iter()
            .map(|item| item.as_str().ok_or_else(field_type_err))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(field_type_err()),
    };
    Ok(targets
        .into_iter()
        .map(|target| format!("{namespace}:{target}"))
        .collect())
}

/// Resolve the effective set of item kinds from an optional config registry.
///
/// The engine bakes in no domain defaults: when the registry is `None` (no
/// `[item_kinds]` table) the kind set is EMPTY (per design D4, single consumer,
/// no backward-compat layer). Kinds are authored entirely in the `[item_kinds]`
/// config table, which `jit init` scaffolds with a complete, editable set. When
/// the registry is present it is used verbatim (each entry resolved through
/// [`ItemKind::from_config`]); the caller opts in to every kind it wants.
///
/// Kinds are returned in name order for deterministic output.
pub fn resolve_item_kinds(
    registry: Option<&HashMap<String, ItemKindConfig>>,
) -> Result<Vec<ItemKind>, ItemError> {
    match registry {
        None => Ok(Vec::new()),
        Some(map) => {
            let mut names: Vec<&String> = map.keys().collect();
            names.sort();
            names
                .into_iter()
                .map(|name| ItemKind::from_config(name, &map[name]))
                .collect()
        }
    }
}

/// Resolve a kind name OR one of its config-declared aliases to the kind's
/// canonical registry name.
///
/// An alias is accepted anywhere a kind name is (the kind segment of an address,
/// or a `--kind` filter); this maps whatever the caller typed back to the
/// registry name so downstream matching — which always compares against the
/// canonical [`AddressableItem::kind`] — works uniformly. Returns `None` when
/// `name` matches no kind by name or alias, leaving the caller to report a
/// descriptive not-found error. A pure lookup over its `kinds` parameter: no kind
/// identity is baked into this module.
pub fn resolve_kind_alias<'a>(kinds: &'a [ItemKind], name: &str) -> Option<&'a str> {
    kinds
        .iter()
        .find(|kind| kind.matches_name(name))
        .map(ItemKind::name)
}

/// The `(section, marker, id-pattern)` triple a named kind expands to, as owned
/// strings ready to splice into a free-form rule config.
///
/// This is the SAME triple [`ItemKind::as_triple`] exposes (the engine's
/// `criterion_ids` / `label-coverage` machinery consumes exactly these three
/// keys), captured here as owned values so a config layer can rewrite a rule's
/// assert table without holding a borrow. `marker` is `None` when the kind
/// declares no marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindTriple {
    /// Section slug whose list items hold the kind's items.
    pub section: String,
    /// The kind's first marker, or `None` when it declares none.
    pub marker: Option<String>,
    /// Regex extracting a self-id from an item's text.
    pub id_pattern: String,
}

/// Expand a named kind from the config registry into its `(section, marker,
/// id-pattern)` [`KindTriple`].
///
/// This is the ONE kind→triple resolver shared by every config layer that offers
/// `kind =` sugar: it resolves the kind through [`ItemKind::from_config`] (so
/// repo defaults and id-pattern validation are applied identically to indexing)
/// and returns the same triple [`ItemKind::as_triple`] exposes. The engine then
/// consumes the triple and never sees the kind NAME, keeping it domain-agnostic
/// (REQ-05). A name absent from `registry` is an [`ItemError::UnknownKind`]; a
/// `None` registry has no declared kinds, so any name is unknown.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use jit::config::ItemKindConfig;
/// use jit::domain::item::{expand_kind_triple, ItemError};
///
/// let mut registry = HashMap::new();
/// registry.insert(
///     "example".to_string(),
///     ItemKindConfig {
///         section: Some("success_criteria".to_string()),
///         markers: Some(vec!["[hard]".to_string()]),
///         id_pattern: Some("REQ-\\d+".to_string()),
///         ..Default::default()
///     },
/// );
/// let triple = expand_kind_triple(Some(&registry), "example").unwrap();
/// assert_eq!(triple.section, "success_criteria");
/// assert_eq!(triple.marker.as_deref(), Some("[hard]"));
///
/// // An undeclared name is a typed error, not a silent pass.
/// let err = expand_kind_triple(Some(&registry), "bogus").unwrap_err();
/// assert!(matches!(err, ItemError::UnknownKind { .. }));
/// ```
pub fn expand_kind_triple(
    registry: Option<&HashMap<String, ItemKindConfig>>,
    name: &str,
) -> Result<KindTriple, ItemError> {
    let config = registry
        .and_then(|map| map.get(name))
        .ok_or_else(|| ItemError::UnknownKind {
            kind: name.to_string(),
        })?;
    let kind = ItemKind::from_config(name, config)?;
    let (section, marker, id_pattern) = kind.as_triple();
    Ok(KindTriple {
        section: section.to_string(),
        marker: marker.map(str::to_string),
        id_pattern: id_pattern.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::MarkdownContentParser;

    // The canonical kinds `jit init` authors into the `[item_kinds]` table, rebuilt
    // here from their exact field shape so the domain layer can pin that those
    // fields project as expected. They are no longer baked into the engine (a repo
    // with no `[item_kinds]` table has no kinds); these helpers stand in for the
    // config-authored table.
    fn req_cfg() -> ItemKindConfig {
        ItemKindConfig {
            section: Some("success_criteria".to_string()),
            id_pattern: Some("[A-Z][A-Z0-9]*-[0-9]+".to_string()),
            markers: Some(vec!["[hard]".to_string()]),
            link_namespaces: Some(vec!["satisfies".to_string()]),
            scope: Some(KindScopeConfig::Issue),
            source: None,
            source_of_truth: Some(SourceOfTruth::MarkdownFirst),
            aliases: None,
        }
    }

    fn decision_cfg() -> ItemKindConfig {
        ItemKindConfig {
            section: Some("decisions".to_string()),
            id_pattern: Some("D-[0-9]+".to_string()),
            markers: Some(vec![]),
            link_namespaces: Some(vec!["per".to_string()]),
            scope: Some(KindScopeConfig::Issue),
            source: None,
            source_of_truth: Some(SourceOfTruth::MarkdownFirst),
            aliases: None,
        }
    }

    fn risk_cfg() -> ItemKindConfig {
        ItemKindConfig {
            section: Some("risks".to_string()),
            id_pattern: Some("RISK-[0-9]+".to_string()),
            markers: Some(vec![]),
            link_namespaces: Some(vec!["mitigates".to_string(), "resolves".to_string()]),
            scope: Some(KindScopeConfig::Issue),
            source: None,
            source_of_truth: Some(SourceOfTruth::MarkdownFirst),
            aliases: None,
        }
    }

    fn invariant_cfg() -> ItemKindConfig {
        ItemKindConfig {
            section: Some("success_criteria".to_string()),
            id_pattern: Some("[A-Z][A-Z0-9]*-[0-9]+".to_string()),
            markers: Some(vec![]),
            link_namespaces: Some(vec!["enforces".to_string()]),
            scope: Some(KindScopeConfig::Project),
            source: Some(ItemKindSource::Toml(TomlSourceDescriptor {
                toml: ".jit/invariants.toml".to_string(),
                table: "invariants".to_string(),
                id_field: "id".to_string(),
                text_field: "statement".to_string(),
                link_fields: std::collections::BTreeMap::new(),
            })),
            source_of_truth: Some(SourceOfTruth::RegistryFirst),
            aliases: None,
        }
    }

    fn req_kind() -> ItemKind {
        ItemKind::from_config("requirement", &req_cfg()).unwrap()
    }

    fn decision_kind() -> ItemKind {
        ItemKind::from_config("decision", &decision_cfg()).unwrap()
    }

    fn risk_kind() -> ItemKind {
        ItemKind::from_config("risk", &risk_cfg()).unwrap()
    }

    fn invariant_kind() -> ItemKind {
        ItemKind::from_config("invariant", &invariant_cfg()).unwrap()
    }

    /// The four canonical kinds, in name order (the set `jit init` authors).
    fn canonical_kinds() -> Vec<ItemKind> {
        vec![decision_kind(), invariant_kind(), req_kind(), risk_kind()]
    }

    #[test]
    fn test_qualified_id_is_derived() {
        // REQ-01: qualified id is the uniform kind-segmented address, a pure
        // projection over (scope, kind, self-id).
        let mut issue = crate::domain::types::fixture_issue("T".to_string(), String::new());
        issue.id = "56ab0224-fd6e-4929-a61e-ffb1a3104496".to_string();
        assert_eq!(
            qualified_id(&Scope::Issue(issue.short_id()), "requirement", "REQ-01"),
            "@/issue/56ab0224/requirement/REQ-01"
        );
        assert_eq!(
            qualified_id(&Scope::Project, "invariant", "sample-invariant"),
            "@/invariant/sample-invariant"
        );
    }

    #[test]
    fn test_parse_kind_segmented_address_roundtrip() {
        // The kind-segmented parser replaces the old two-segment `split_qualified_id`
        // as the qualified-address parser: an explicit `@`-prefixed address round-trips
        // into its (scope, kind, self-id) triple, and a bare self-id is rejected.
        let addr = parse_kind_segmented_address("@/requirement/REQ-01").unwrap();
        assert_eq!(addr.scope, AddressScope::Project);
        assert_eq!(addr.kind, "requirement");
        assert_eq!(addr.self_id, "REQ-01");

        let issue_addr =
            parse_kind_segmented_address("@/issue/56ab0224/requirement/REQ-01").unwrap();
        assert_eq!(
            issue_addr.scope,
            AddressScope::Issue("56ab0224".to_string())
        );
        assert_eq!(issue_addr.self_id, "REQ-01");

        assert!(parse_kind_segmented_address("bare").is_err());

        // The qualified/unqualified boundary the link classifiers rely on.
        assert!(is_qualified_reference("56ab0224/REQ-01"));
        assert!(!is_qualified_reference("bare"));
    }

    #[test]
    fn test_index_items_projects_requirements() {
        let issue = crate::domain::types::fixture_issue(
            "T".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: first\n- [hard] REQ-02: second\n".to_string(),
        );
        let items = index_items(&issue, &[req_kind()], &MarkdownContentParser).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].self_id, "REQ-01");
        assert_eq!(items[0].kind, "requirement");
        assert_eq!(
            items[0].qualified_id,
            qualified_id(&Scope::Issue(issue.short_id()), "requirement", "REQ-01")
        );
        assert_eq!(
            items[0].qualified_id,
            format!("@/issue/{}/requirement/REQ-01", issue.short_id())
        );
        assert_eq!(items[1].self_id, "REQ-02");
    }

    #[test]
    fn test_index_items_graceful_degradation() {
        // REQ-06: a list line with no self-id is plain prose, not an error.
        let issue = crate::domain::types::fixture_issue(
            "T".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: real\n- [hard] just prose, no id\n"
                .to_string(),
        );
        let items = index_items(&issue, &[req_kind()], &MarkdownContentParser).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].self_id, "REQ-01");
    }

    #[test]
    fn test_index_items_marker_filters_prose() {
        // An unmarked criterion line is ignored by a marker-gated kind.
        let issue = crate::domain::types::fixture_issue(
            "T".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: hard one\n- REQ-99: soft, no marker\n"
                .to_string(),
        );
        let items = index_items(&issue, &[req_kind()], &MarkdownContentParser).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].self_id, "REQ-01");
    }

    // --- checkbox-prefixed criteria (jit:16402e14) --------------------------

    #[test]
    fn test_text_after_checkbox_strips_unchecked_and_checked_tokens() {
        assert_eq!(
            text_after_checkbox("[ ] [hard] REQ-01: a"),
            "[hard] REQ-01: a"
        );
        assert_eq!(
            text_after_checkbox("[x] [hard] REQ-01: a"),
            "[hard] REQ-01: a"
        );
        assert_eq!(
            text_after_checkbox("[X] [hard] REQ-01: a"),
            "[hard] REQ-01: a"
        );
    }

    #[test]
    fn test_text_after_checkbox_is_noop_without_a_checkbox() {
        assert_eq!(text_after_checkbox("[hard] REQ-01: a"), "[hard] REQ-01: a");
        assert_eq!(text_after_checkbox("plain prose"), "plain prose");
    }

    #[test]
    fn test_index_items_recognizes_checkbox_prefixed_hard_criterion() {
        // jit:16402e14 REQ-01/REQ-02: a GitHub task-list checkbox ahead of the
        // marker (`- [ ] [hard] ...`, `- [x] [hard] ...`) does not hide the
        // criterion from the item projection.
        let issue = crate::domain::types::fixture_issue(
            "T".to_string(),
            "## Success Criteria\n\n\
             - [ ] [hard] REQ-01: unchecked box\n\
             - [x] [hard] REQ-02: checked box\n"
                .to_string(),
        );
        let items = index_items(&issue, &[req_kind()], &MarkdownContentParser).unwrap();
        assert_eq!(
            items.len(),
            2,
            "both checkbox-prefixed [hard] criteria must register: {items:?}"
        );
        let self_ids: HashSet<&str> = items.iter().map(|item| item.self_id.as_str()).collect();
        assert_eq!(self_ids, HashSet::from(["REQ-01", "REQ-02"]));
    }

    #[test]
    fn test_index_items_duplicate_self_id_is_error() {
        // REQ-02: self-id uniqueness within an issue is validated.
        let issue = crate::domain::types::fixture_issue(
            "T".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: a\n- [hard] REQ-01: dup\n".to_string(),
        );
        let err = index_items(&issue, &[req_kind()], &MarkdownContentParser).unwrap_err();
        assert!(matches!(err, ItemError::DuplicateSelfId { .. }));
    }

    #[test]
    fn test_index_items_cross_kind_same_self_id_coexists() {
        // REQ-01: two DIFFERENT kinds minting the same self-id now coexist as two
        // distinct items — uniqueness is keyed on (self_id, kind), not self_id
        // alone, so this is no longer a collision.
        let other = ItemKind::from_config(
            "decision",
            &ItemKindConfig {
                // Read the SAME section so both kinds see REQ-01.
                section: Some(DEFAULT_ITEM_SECTION.to_string()),
                id_pattern: Some("REQ-\\d+".to_string()),
                markers: None,
                link_namespaces: None,
                ..Default::default()
            },
        )
        .unwrap();
        let issue = crate::domain::types::fixture_issue(
            "T".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: a\n".to_string(),
        );
        // The marker-gated requirement kind and the unmarked `decision` kind both
        // claim REQ-01 from the same line; both are returned.
        let items = index_items(&issue, &[req_kind(), other], &MarkdownContentParser).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item.self_id == "REQ-01"));
        let kinds: HashSet<&str> = items.iter().map(|item| item.kind.as_str()).collect();
        assert_eq!(kinds, HashSet::from(["requirement", "decision"]));
        // The kind is a segment of the minted id, so the two coexisting items derive
        // DISTINCT qualified ids that differ only by that segment.
        assert_ne!(items[0].qualified_id, items[1].qualified_id);
        let qids: HashSet<&str> = items.iter().map(|i| i.qualified_id.as_str()).collect();
        assert!(qids.iter().any(|q| q.ends_with("/requirement/REQ-01")));
        assert!(qids.iter().any(|q| q.ends_with("/decision/REQ-01")));
    }

    #[test]
    fn test_index_items_missing_section_is_empty() {
        let issue = crate::domain::types::fixture_issue(
            "T".to_string(),
            "## Other\n\n- nothing here\n".to_string(),
        );
        let items = index_items(&issue, &[req_kind()], &MarkdownContentParser).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_generic_kind_no_kind_name_branch() {
        // REQ-01: a kind defined entirely by config (a name the engine has never
        // heard of) indexes purely from its four-tuple, proving no kind name is
        // hardcoded in indexing logic.
        let cfg = ItemKindConfig {
            section: Some("decisions".to_string()),
            id_pattern: Some("D-\\d+".to_string()),
            markers: None,
            link_namespaces: Some(vec!["per".to_string()]),
            ..Default::default()
        };
        let kind = ItemKind::from_config("decision", &cfg).unwrap();
        let issue = crate::domain::types::fixture_issue(
            "T".to_string(),
            "## Decisions\n\n- D-1: use json\n- D-2: atomic writes\n".to_string(),
        );
        let items = index_items(&issue, &[kind], &MarkdownContentParser).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, "decision");
        assert_eq!(items[0].self_id, "D-1");
    }

    #[test]
    fn test_invalid_id_pattern_is_typed_error() {
        let cfg = ItemKindConfig {
            section: None,
            id_pattern: Some("REQ-(".to_string()),
            markers: None,
            link_namespaces: None,
            ..Default::default()
        };
        let err = ItemKind::from_config("broken", &cfg).unwrap_err();
        assert!(matches!(err, ItemError::InvalidIdPattern { .. }));
    }

    #[test]
    fn test_requirement_default_triple_matches_coverage_defaults() {
        // REQ-05: the requirement kind expands to the SAME (section, marker,
        // id-pattern) triple the label-coverage rule uses by default.
        let kind = req_kind();
        let (section, marker, pattern) = kind.as_triple();
        assert_eq!(section, "success_criteria");
        assert_eq!(marker, Some("[hard]"));
        assert_eq!(pattern, "[A-Z][A-Z0-9]*-[0-9]+");
    }

    #[test]
    fn test_resolve_kind_alias_maps_alias_and_name_to_registry_name() {
        // An alias declared on a kind resolves to that kind's registry name, as
        // does the registry name itself; an unknown token resolves to nothing.
        let invariant = ItemKind::from_config(
            "invariant",
            &ItemKindConfig {
                aliases: Some(vec!["inv".to_string()]),
                ..invariant_cfg()
            },
        )
        .unwrap();
        let requirement = ItemKind::from_config("requirement", &req_cfg()).unwrap();
        let kinds = [invariant, requirement];

        // Alias and registry name both resolve to the registry name.
        assert_eq!(resolve_kind_alias(&kinds, "inv"), Some("invariant"));
        assert_eq!(resolve_kind_alias(&kinds, "invariant"), Some("invariant"));
        // A kind with no alias still resolves by its registry name.
        assert_eq!(
            resolve_kind_alias(&kinds, "requirement"),
            Some("requirement")
        );
        // An unknown token (neither a name nor an alias) resolves to nothing.
        assert_eq!(resolve_kind_alias(&kinds, "req"), None);
        assert_eq!(resolve_kind_alias(&kinds, "bogus"), None);
    }

    #[test]
    fn test_resolve_item_kinds_empty_when_absent() {
        // No `[item_kinds]` table -> no kinds. The engine bakes in no domain
        // defaults (D4: single consumer, no backward-compat layer); kinds are
        // authored entirely in config (scaffolded by `jit init`).
        assert!(resolve_item_kinds(None).unwrap().is_empty());
    }

    #[test]
    fn test_resolve_item_kinds_canonical_table_resolves_all_four() {
        // The complete table `jit init` authors resolves to the four canonical
        // kinds, in name order — each through the generic `from_config` path.
        let map: HashMap<String, ItemKindConfig> = [
            ("requirement", req_cfg()),
            ("decision", decision_cfg()),
            ("risk", risk_cfg()),
            ("invariant", invariant_cfg()),
        ]
        .into_iter()
        .map(|(name, cfg)| (name.to_string(), cfg))
        .collect();
        let kinds = resolve_item_kinds(Some(&map)).unwrap();
        let names: Vec<&str> = kinds.iter().map(ItemKind::name).collect();
        assert_eq!(names, vec!["decision", "invariant", "requirement", "risk"]);
    }

    #[test]
    fn test_decision_kind_tuple() {
        // The canonical decision kind: section `decisions`, D-NN ids, no marker,
        // `per` link namespace, issue-scoped, markdown-first.
        let decision = decision_kind();
        assert_eq!(decision.name(), "decision");
        assert_eq!(decision.section(), "decisions");
        assert!(decision.markers().is_empty());
        assert_eq!(decision.link_namespaces(), &["per".to_string()]);
        assert_eq!(decision.kind_scope(), KindScope::Issue);
        let (section, marker, pattern) = decision.as_triple();
        assert_eq!(section, "decisions");
        assert_eq!(marker, None);
        assert_eq!(pattern, "D-[0-9]+");
    }

    #[test]
    fn test_invariant_kind_is_project_scope_registry_first() {
        // The canonical invariant kind: project-scoped, registry-first, `enforces`
        // link namespace, and NO markdown `source` file (items come from the toml
        // registry named by its descriptor).
        let inv = invariant_kind();
        assert_eq!(inv.name(), "invariant");
        assert_eq!(inv.kind_scope(), KindScope::Project);
        assert_eq!(inv.source_of_truth(), SourceOfTruth::RegistryFirst);
        assert_eq!(inv.source(), None);
        assert_eq!(inv.link_namespaces(), &["enforces".to_string()]);
    }

    #[test]
    fn test_registry_first_project_kind_requires_toml_descriptor() {
        // Symmetric to the markdown-first source requirement (REQ-03): a
        // registry-first project kind must declare a toml `source` descriptor, or it
        // is rejected at resolution with a typed MissingProjectSource — never a
        // silent empty result.
        let base = |source: Option<ItemKindSource>| ItemKindConfig {
            section: Some(DEFAULT_ITEM_SECTION.to_string()),
            id_pattern: Some(DEFAULT_ITEM_ID_PATTERN.to_string()),
            markers: Some(vec![]),
            link_namespaces: Some(vec!["enforces".to_string()]),
            scope: Some(KindScopeConfig::Project),
            source,
            source_of_truth: Some(SourceOfTruth::RegistryFirst),
            aliases: None,
        };

        // No descriptor → rejected (the registry-first analogue of a missing
        // markdown source).
        let err = ItemKind::from_config("policy", &base(None)).unwrap_err();
        assert!(matches!(err, ItemError::MissingProjectSource { .. }));

        // With a toml descriptor → accepted; the descriptor is exposed by
        // `toml_source`, and no markdown `source` path is set.
        let descriptor = TomlSourceDescriptor {
            toml: "policies.toml".to_string(),
            table: "policies".to_string(),
            id_field: "id".to_string(),
            text_field: "statement".to_string(),
            link_fields: std::collections::BTreeMap::new(),
        };
        let kind =
            ItemKind::from_config("policy", &base(Some(ItemKindSource::Toml(descriptor)))).unwrap();
        assert_eq!(kind.source_of_truth(), SourceOfTruth::RegistryFirst);
        assert_eq!(kind.source(), None);
        assert_eq!(kind.toml_source().unwrap().table, "policies");
    }

    #[test]
    fn test_invariant_kind_routes_through_toml_descriptor() {
        // REQ-03: the canonical `invariant` kind carries a toml `source` descriptor
        // naming `.jit/invariants.toml`, so it routes through the GENERIC
        // registry-first path with no reserved-name branch. The descriptor maps
        // `id`/`statement` and NO link-fields (so each item's links stay empty,
        // matching the prior typed projection byte-for-byte).
        let inv = invariant_kind();
        assert_eq!(inv.name(), "invariant");
        assert_eq!(inv.kind_scope(), KindScope::Project);
        assert_eq!(inv.source_of_truth(), SourceOfTruth::RegistryFirst);
        assert_eq!(inv.source(), None);
        let descriptor = inv
            .toml_source()
            .expect("invariant carries a toml descriptor");
        assert_eq!(descriptor.toml, ".jit/invariants.toml");
        assert_eq!(descriptor.table, "invariants");
        assert_eq!(descriptor.id_field, "id");
        assert_eq!(descriptor.text_field, "statement");
        assert!(descriptor.link_fields.is_empty());
    }

    #[test]
    fn test_invariant_name_is_no_longer_reserved() {
        // REQ-03: the `invariant` NAME no longer carries any special routing. A
        // config-declared `invariant` kind resolves as ordinary config like any
        // other name — declarations the old reserved-name branch rejected are now
        // accepted because the engine hardcodes no domain concept.
        let cfg = |scope: KindScopeConfig, sot: SourceOfTruth, source: Option<ItemKindSource>| {
            ItemKindConfig {
                section: Some(DEFAULT_ITEM_SECTION.to_string()),
                id_pattern: Some("[a-z][a-z0-9-]*".to_string()),
                markers: Some(vec![]),
                link_namespaces: Some(vec!["enforces".to_string()]),
                scope: Some(scope),
                source,
                source_of_truth: Some(sot),
                aliases: None,
            }
        };

        // Markdown-first project `invariant` with a source: once a reserved-name
        // rejection, now an ordinary markdown-first project kind.
        let md = ItemKind::from_config(
            "invariant",
            &cfg(
                KindScopeConfig::Project,
                SourceOfTruth::MarkdownFirst,
                Some(ItemKindSource::Path("project-items.md".to_string())),
            ),
        )
        .unwrap();
        assert_eq!(md.source_of_truth(), SourceOfTruth::MarkdownFirst);
        assert_eq!(md.source(), Some("project-items.md"));

        // Registry-first issue-scoped `invariant`: once rejected to keep invariants
        // out of the issue-description parser, now an ordinary issue-scope kind (the
        // project-source requirement does not apply to issue scope).
        let issue = ItemKind::from_config(
            "invariant",
            &cfg(KindScopeConfig::Issue, SourceOfTruth::RegistryFirst, None),
        )
        .unwrap();
        assert_eq!(issue.kind_scope(), KindScope::Issue);
    }

    #[test]
    fn test_markdown_first_project_kind_still_requires_source() {
        // The guard still fires for a markdown-first project kind with no source.
        let cfg = ItemKindConfig {
            section: Some(DEFAULT_ITEM_SECTION.to_string()),
            id_pattern: Some(DEFAULT_ITEM_ID_PATTERN.to_string()),
            markers: Some(vec![]),
            link_namespaces: Some(vec!["upholds".to_string()]),
            scope: Some(KindScopeConfig::Project),
            source: None,
            source_of_truth: Some(SourceOfTruth::MarkdownFirst),
            aliases: None,
        };
        let err = ItemKind::from_config("doc-req", &cfg).unwrap_err();
        assert!(matches!(err, ItemError::MissingProjectSource { .. }));
    }

    #[test]
    fn test_index_project_sources_pools_registry_and_markdown() {
        // Both substrates dedup through one pass: a markdown source and a
        // registry-derived candidate both surface under `@/<kind>/<self-id>`.
        let sources = vec![ProjectSource {
            kind: req_kind(),
            markdown: "## Success Criteria\n\n- [hard] REQ-01: a\n".to_string(),
        }];
        let registry = vec![RawScopeItem {
            kind: "invariant".to_string(),
            self_id: "sample-invariant".to_string(),
            text: "atomic writes".to_string(),
            links: Vec::new(),
        }];
        let items = index_project_sources(&sources, registry, &MarkdownContentParser).unwrap();
        let qids: Vec<&str> = items.iter().map(|i| i.qualified_id.as_str()).collect();
        assert!(qids.contains(&"@/requirement/REQ-01"));
        assert!(qids.contains(&"@/invariant/sample-invariant"));
    }

    #[test]
    fn test_index_project_sources_cross_substrate_duplicate_is_error() {
        // A self-id shared by a markdown source and a registry candidate of the
        // SAME kind collides in the single project-scope dedup pass (REQ-03):
        // pooling across substrates does not bypass per-(scope, kind) uniqueness.
        let sources = vec![ProjectSource {
            kind: req_kind(),
            markdown: "## Success Criteria\n\n- [hard] REQ-01: a\n".to_string(),
        }];
        let registry = vec![RawScopeItem {
            kind: "requirement".to_string(),
            self_id: "REQ-01".to_string(),
            text: "dup".to_string(),
            links: Vec::new(),
        }];
        let err = index_project_sources(&sources, registry, &MarkdownContentParser).unwrap_err();
        assert!(matches!(err, ItemError::DuplicateSelfId { .. }));
    }

    fn policy_descriptor() -> TomlSourceDescriptor {
        TomlSourceDescriptor {
            toml: "policies.toml".to_string(),
            table: "policies".to_string(),
            id_field: "id".to_string(),
            text_field: "statement".to_string(),
            link_fields: [("enforces".to_string(), "enforced-by".to_string())]
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn test_load_toml_scope_items_maps_fields_and_links() {
        // REQ-02: id-field -> self-id, text-field -> text, and a string-array link
        // field -> `<namespace>:<target>` labels; a single-string link field maps to
        // one label.
        let content = "\
[[policies]]
id = \"POL-01\"
statement = \"all writes are atomic\"
enforced-by = [\"cargo-ci\", \"jit-validate\"]

[[policies]]
id = \"POL-02\"
statement = \"single enforcer\"
enforced-by = \"tests\"
";
        let rows = load_toml_scope_items("policy", &policy_descriptor(), content).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, "policy");
        assert_eq!(rows[0].self_id, "POL-01");
        assert_eq!(rows[0].text, "all writes are atomic");
        assert_eq!(
            rows[0].links,
            vec!["enforces:cargo-ci", "enforces:jit-validate"]
        );
        // A single string link value yields exactly one label.
        assert_eq!(rows[1].links, vec!["enforces:tests"]);
    }

    #[test]
    fn test_load_toml_scope_items_absent_table_is_graceful() {
        // A file with no matching table is an empty registry, never an error.
        let rows =
            load_toml_scope_items("policy", &policy_descriptor(), "[other]\nx = 1\n").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn test_load_toml_scope_items_absent_link_field_is_graceful() {
        // An entry lacking the mapped link field contributes no labels (only the
        // addressing fields are mandatory).
        let content = "[[policies]]\nid = \"POL-03\"\nstatement = \"no enforcer\"\n";
        let rows = load_toml_scope_items("policy", &policy_descriptor(), content).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].links.is_empty());
    }

    #[test]
    fn test_load_toml_scope_items_missing_id_field_is_error() {
        // A missing mapped id-field is a typed, descriptive error naming the field.
        let content = "[[policies]]\nstatement = \"x\"\n";
        let err = load_toml_scope_items("policy", &policy_descriptor(), content).unwrap_err();
        assert!(matches!(
            err,
            ItemError::TomlSourceMissingField { ref field, .. } if field == "id"
        ));
    }

    #[test]
    fn test_load_toml_scope_items_absent_text_field_falls_back_to_id() {
        // REQ-03: the text-field is optional. An entry lacking it projects its
        // id-field value as display text (a description-less rule shows its
        // name), rather than erroring. A `rule`-shaped descriptor whose
        // text-field is `description`, with an entry that omits it, exercises the
        // fallback the item-kind flip relies on.
        let descriptor = TomlSourceDescriptor {
            toml: ".jit/rules.toml".to_string(),
            table: "rules".to_string(),
            id_field: "name".to_string(),
            text_field: "description".to_string(),
            link_fields: Default::default(),
        };
        let content = "\
[[rules]]
name = \"described\"
description = \"Every label must be namespace:value.\"

[[rules]]
name = \"bare\"
";
        let rows = load_toml_scope_items("rule", &descriptor, content).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].text, "Every label must be namespace:value.");
        // Name fallback: the description-less rule's text is its self-id.
        assert_eq!(rows[1].self_id, "bare");
        assert_eq!(rows[1].text, "bare");
    }

    #[test]
    fn test_load_toml_scope_items_wrong_text_type_is_error() {
        // A present-but-non-string text-field is still a typed field-type error
        // (the fallback only covers an ABSENT field, not a malformed one).
        let content = "[[policies]]\nid = \"POL-05\"\nstatement = 7\n";
        let err = load_toml_scope_items("policy", &policy_descriptor(), content).unwrap_err();
        assert!(matches!(
            err,
            ItemError::TomlSourceFieldType { ref field, .. } if field == "statement"
        ));
    }

    #[test]
    fn test_load_toml_scope_items_wrong_link_type_is_error() {
        // A link field that is neither a string nor an array of strings is a typed
        // field-type error.
        let content = "[[policies]]\nid = \"POL-04\"\nstatement = \"x\"\nenforced-by = 7\n";
        let err = load_toml_scope_items("policy", &policy_descriptor(), content).unwrap_err();
        assert!(matches!(
            err,
            ItemError::TomlSourceFieldType { ref field, .. } if field == "enforced-by"
        ));
    }

    #[test]
    fn test_load_toml_scope_items_malformed_toml_is_parse_error() {
        let err =
            load_toml_scope_items("policy", &policy_descriptor(), "not = = toml").unwrap_err();
        assert!(matches!(err, ItemError::TomlSourceParse { .. }));
    }

    #[test]
    fn test_index_items_projects_decisions_with_canonical_kinds() {
        // The canonical kind set (as `jit init` authors it) indexes a `## Decisions`
        // section's D-NN lines through the same generic parse path as requirements.
        let issue = crate::domain::types::fixture_issue(
            "T".to_string(),
            "## Decisions\n\n- D-01: use json\n- D-02: atomic writes\n".to_string(),
        );
        let kinds = canonical_kinds();
        let items = index_items(&issue, &kinds, &MarkdownContentParser).unwrap();
        let decisions: Vec<&AddressableItem> =
            items.iter().filter(|i| i.kind == "decision").collect();
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].self_id, "D-01");
        assert!(decisions[0].qualified_id.ends_with("/D-01"));
    }

    #[test]
    fn test_resolve_item_kinds_uses_registry() {
        let mut map = HashMap::new();
        map.insert("requirement".to_string(), ItemKindConfig::default());
        map.insert(
            "decision".to_string(),
            ItemKindConfig {
                section: Some("decisions".to_string()),
                id_pattern: Some("D-\\d+".to_string()),
                markers: None,
                link_namespaces: None,
                ..Default::default()
            },
        );
        let kinds = resolve_item_kinds(Some(&map)).unwrap();
        // Returned in name order.
        assert_eq!(kinds.len(), 2);
        assert_eq!(kinds[0].name(), "decision");
        assert_eq!(kinds[1].name(), "requirement");
    }

    #[test]
    fn test_expand_kind_triple_matches_as_triple() {
        // REQ-02: expanding a named kind yields the SAME triple as_triple exposes,
        // so the `kind=` sugar and the inline form resolve identically.
        let mut registry = HashMap::new();
        registry.insert(
            "requirement".to_string(),
            ItemKindConfig {
                section: Some("success_criteria".to_string()),
                markers: Some(vec!["[hard]".to_string()]),
                id_pattern: Some("REQ-\\d+".to_string()),
                ..Default::default()
            },
        );
        let triple = expand_kind_triple(Some(&registry), "requirement").unwrap();
        let kind = ItemKind::from_config("requirement", &registry["requirement"]).unwrap();
        let (section, marker, pattern) = kind.as_triple();
        assert_eq!(triple.section, section);
        assert_eq!(triple.marker.as_deref(), marker);
        assert_eq!(triple.id_pattern, pattern);
    }

    #[test]
    fn test_expand_kind_triple_applies_defaults() {
        // A minimally-declared kind expands with the repo defaults applied.
        let mut registry = HashMap::new();
        registry.insert("requirement".to_string(), ItemKindConfig::default());
        let triple = expand_kind_triple(Some(&registry), "requirement").unwrap();
        assert_eq!(triple.section, DEFAULT_ITEM_SECTION);
        assert_eq!(triple.id_pattern, DEFAULT_ITEM_ID_PATTERN);
        // No markers declared -> no marker in the triple.
        assert_eq!(triple.marker, None);
    }

    #[test]
    fn test_expand_kind_triple_unknown_is_error() {
        // The constraint: an undeclared kind reference is a typed error.
        let registry: HashMap<String, ItemKindConfig> = HashMap::new();
        let err = expand_kind_triple(Some(&registry), "requirement").unwrap_err();
        assert!(matches!(err, ItemError::UnknownKind { ref kind } if kind == "requirement"));

        // A None registry likewise has no declared kinds.
        let err = expand_kind_triple(None, "requirement").unwrap_err();
        assert!(matches!(err, ItemError::UnknownKind { .. }));
    }

    #[test]
    fn test_expand_kind_triple_propagates_invalid_pattern() {
        let mut registry = HashMap::new();
        registry.insert(
            "broken".to_string(),
            ItemKindConfig {
                id_pattern: Some("REQ-(".to_string()),
                ..Default::default()
            },
        );
        let err = expand_kind_triple(Some(&registry), "broken").unwrap_err();
        assert!(matches!(err, ItemError::InvalidIdPattern { .. }));
    }

    fn raw(kind: &str, self_id: &str) -> RawScopeItem {
        RawScopeItem {
            kind: kind.to_string(),
            self_id: self_id.to_string(),
            text: format!("{self_id} text"),
            links: Vec::new(),
        }
    }

    #[test]
    fn test_scope_parse_and_prefix() {
        // REQ-01: `@` is the project scope sentinel; anything else is an issue scope.
        assert_eq!(Scope::parse("@"), Scope::Project);
        assert!(Scope::parse("@").is_project());
        assert_eq!(Scope::Project.prefix(), "@");

        let issue = Scope::parse("56ab0224");
        assert_eq!(issue, Scope::Issue("56ab0224".to_string()));
        assert!(!issue.is_project());
        assert_eq!(issue.prefix(), "56ab0224");
    }

    #[test]
    fn test_derive_scope_items_derives_at_project_scope() {
        // REQ-01: project-scoped candidates mint `@/<kind>/<self-id>`; the item's
        // own `scope` field stays the bare `@` prefix.
        let items = derive_scope_items(&Scope::Project, vec![raw("invariant", "sample-invariant")])
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].qualified_id, "@/invariant/sample-invariant");
        assert_eq!(items[0].scope, "@");
        assert_eq!(items[0].self_id, "sample-invariant");
    }

    #[test]
    fn test_index_markdown_items_at_project_scope() {
        // REQ-01: a markdown source scanned at project scope mints
        // `@/<kind>/<self-id>`, through the SAME parse + extract + derive path as
        // issue scope.
        let kinds = vec![req_kind()];
        let md = "## Success Criteria\n\n- [hard] REQ-01: all writes are atomic\n- prose line\n";
        let items =
            index_markdown_items(md, &Scope::Project, &kinds, &MarkdownContentParser).unwrap();
        // The prose line without a self-id is skipped (REQ-06).
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].qualified_id, "@/requirement/REQ-01");
        assert_eq!(items[0].scope, "@");
    }

    #[test]
    fn test_derive_scope_items_duplicate_within_scope_is_error() {
        // REQ-03: a self-id repeated within ONE scope is a duplicate, not silently
        // resolved to one.
        let err = derive_scope_items(
            &Scope::Project,
            vec![
                raw("invariant", "sample-invariant"),
                raw("invariant", "sample-invariant"),
            ],
        )
        .unwrap_err();
        match err {
            ItemError::DuplicateSelfId { scope, self_id, .. } => {
                assert_eq!(scope, "@");
                assert_eq!(self_id, "sample-invariant");
            }
            other => panic!("expected DuplicateSelfId, got {other:?}"),
        }
    }

    #[test]
    fn test_derive_scope_items_cross_kind_same_self_id_coexists() {
        // REQ-03: uniqueness is keyed on (self_id, kind), so two DIFFERENT kinds
        // minting the same self-id in one scope coexist rather than colliding.
        let items = derive_scope_items(
            &Scope::Project,
            vec![raw("invariant", "X-1"), raw("decision", "X-1")],
        )
        .unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item.self_id == "X-1"));
        let kinds: HashSet<&str> = items.iter().map(|item| item.kind.as_str()).collect();
        assert_eq!(kinds, HashSet::from(["invariant", "decision"]));
        // The kind is a segment of the minted id, so the two coexisting items derive
        // DISTINCT qualified ids.
        assert_ne!(items[0].qualified_id, items[1].qualified_id);
        let qids: HashSet<&str> = items.iter().map(|i| i.qualified_id.as_str()).collect();
        assert_eq!(qids, HashSet::from(["@/invariant/X-1", "@/decision/X-1"]));
    }

    #[test]
    fn test_parse_kind_segmented_address_local_project_form() {
        // REQ-01: `@/<kind>/<self-id>` parses into (Scope::Project, kind, self_id).
        let addr = parse_kind_segmented_address("@/requirement/REQ-01").unwrap();
        assert_eq!(addr.scope, AddressScope::Project);
        assert_eq!(addr.kind, "requirement");
        assert_eq!(addr.self_id, "REQ-01");
    }

    #[test]
    fn test_parse_kind_segmented_address_named_project_form() {
        // REQ-01: `@<name>/<kind>/<self-id>` carries `<name>` structurally, with
        // no resolution against any declared project identity.
        let addr = parse_kind_segmented_address("@acme/requirement/REQ-01").unwrap();
        assert_eq!(addr.scope, AddressScope::NamedProject("acme".to_string()));
        assert_eq!(addr.kind, "requirement");
        assert_eq!(addr.self_id, "REQ-01");
    }

    #[test]
    fn test_bind_local_project_scope_resolves_regardless_of_declared_name() {
        // REQ-01: bare `@` always binds to the local project, whether or not a
        // local name is declared, consuming the already-parsed scope token.
        assert_eq!(
            AddressScope::Project.bind_local(Some("acme")).unwrap(),
            Scope::Project
        );
        assert_eq!(
            AddressScope::Project.bind_local(None).unwrap(),
            Scope::Project
        );
    }

    #[test]
    fn test_bind_local_named_project_matching_local_name_resolves_as_local() {
        // REQ-01: `AddressScope::NamedProject("acme")` binds identically to the
        // bare-`@` local scope when "acme" is the declared local project name.
        assert_eq!(
            AddressScope::NamedProject("acme".to_string())
                .bind_local(Some("acme"))
                .unwrap(),
            Scope::Project
        );
    }

    #[test]
    fn test_bind_local_named_project_different_name_is_not_resolvable() {
        // REQ-02: a named-project reference to a project OTHER than the local one
        // is syntactically valid but not resolvable, and the error names it and
        // states resolution is local-only with no remote attempt.
        let err = AddressScope::NamedProject("other-project".to_string())
            .bind_local(Some("acme"))
            .unwrap_err();
        assert!(matches!(
            err,
            ItemError::NotResolvable { ref project } if project == "other-project"
        ));
        let msg = err.to_string();
        assert!(msg.contains("other-project"), "message: {msg}");
        assert!(msg.contains("local-only"), "message: {msg}");
        assert!(
            msg.contains("no federation or remote lookup"),
            "message: {msg}"
        );
    }

    #[test]
    fn test_bind_local_named_project_no_declared_name_is_not_resolvable() {
        // REQ-02: with no local project name declared at all, ANY named-project
        // reference is not resolvable.
        let err = AddressScope::NamedProject("acme".to_string())
            .bind_local(None)
            .unwrap_err();
        assert!(matches!(
            err,
            ItemError::NotResolvable { ref project } if project == "acme"
        ));
    }

    #[test]
    fn test_bind_local_issue_scope_passes_through_unchanged() {
        // Issue-scoped addresses carry no project identity, so the binding leaves
        // them untouched regardless of the declared local project name.
        assert_eq!(
            AddressScope::Issue("56ab0224".to_string())
                .bind_local(Some("acme"))
                .unwrap(),
            Scope::Issue("56ab0224".to_string())
        );
        assert_eq!(
            AddressScope::Issue("56ab0224".to_string())
                .bind_local(None)
                .unwrap(),
            Scope::Issue("56ab0224".to_string())
        );
    }

    #[test]
    fn test_parse_kind_segmented_address_issue_form() {
        // REQ-01: `@/issue/<short-id>/<kind>/<self-id>` parses into
        // (Scope::Issue(short_id), kind, self_id); `issue` is a reserved segment.
        let addr = parse_kind_segmented_address("@/issue/56ab0224/requirement/REQ-01").unwrap();
        assert_eq!(addr.scope, AddressScope::Issue("56ab0224".to_string()));
        assert_eq!(addr.kind, "requirement");
        assert_eq!(addr.self_id, "REQ-01");
    }

    #[test]
    fn test_parse_kind_segmented_address_missing_kind_segment_is_error() {
        // REQ-02: an empty kind segment is a typed error naming the input.
        let err = parse_kind_segmented_address("@//REQ-01").unwrap_err();
        match err {
            ItemError::InvalidAddress { address, reason } => {
                assert_eq!(address, "@//REQ-01");
                assert!(reason.contains("kind"), "reason was: {reason}");
            }
            other => panic!("expected InvalidAddress, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_kind_segmented_address_missing_self_id_segment_is_error() {
        // REQ-02: an empty self-id segment is a typed error naming the input.
        let err = parse_kind_segmented_address("@/requirement/").unwrap_err();
        match err {
            ItemError::InvalidAddress { address, reason } => {
                assert_eq!(address, "@/requirement/");
                assert!(reason.contains("self-id"), "reason was: {reason}");
            }
            other => panic!("expected InvalidAddress, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_kind_segmented_address_bad_reserved_segment_is_error() {
        // REQ-02: a 4-segment form whose first segment isn't `issue` is rejected,
        // naming the offending segment.
        let err = parse_kind_segmented_address("@/task/56ab0224/requirement/REQ-01").unwrap_err();
        match err {
            ItemError::InvalidAddress { address, reason } => {
                assert_eq!(address, "@/task/56ab0224/requirement/REQ-01");
                assert!(reason.contains("issue"), "reason was: {reason}");
                assert!(reason.contains("task"), "reason was: {reason}");
            }
            other => panic!("expected InvalidAddress, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_kind_segmented_address_missing_scope_sentinel_is_error() {
        // Not `@`-prefixed at all: still a typed error, not a panic.
        let err = parse_kind_segmented_address("56ab0224/requirement/REQ-01").unwrap_err();
        assert!(matches!(err, ItemError::InvalidAddress { .. }));
    }

    #[test]
    fn test_parse_kind_segmented_address_wrong_segment_count_is_error() {
        // Too few segments (only a kind, no self-id at all) is rejected.
        let err = parse_kind_segmented_address("@/requirement").unwrap_err();
        assert!(matches!(err, ItemError::InvalidAddress { .. }));

        // Too many segments (neither the 2- nor 4-segment shape) is rejected.
        let err = parse_kind_segmented_address("@/a/b/c").unwrap_err();
        assert!(matches!(err, ItemError::InvalidAddress { .. }));
    }

    #[test]
    fn test_parse_kind_segmented_address_issue_form_rejects_named_project_scope() {
        // The issue-item form is defined only for the bare `@` scope.
        let err =
            parse_kind_segmented_address("@acme/issue/56ab0224/requirement/REQ-01").unwrap_err();
        assert!(matches!(err, ItemError::InvalidAddress { .. }));
    }

    #[test]
    fn test_parse_kind_segmented_address_issue_reserved_word_rejected_as_kind_name() {
        // `issue` is reserved and never a kind name, even in the 2-segment form.
        let err = parse_kind_segmented_address("@/issue/REQ-01").unwrap_err();
        match err {
            ItemError::InvalidAddress { reason, .. } => {
                assert!(reason.contains("reserved"), "reason was: {reason}");
            }
            other => panic!("expected InvalidAddress, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_kind_segmented_address_empty_issue_short_id_is_error() {
        let err = parse_kind_segmented_address("@/issue//requirement/REQ-01").unwrap_err();
        match err {
            ItemError::InvalidAddress { reason, .. } => {
                assert!(reason.contains("short-id"), "reason was: {reason}");
            }
            other => panic!("expected InvalidAddress, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_kind_segmented_address_colon_in_self_id_is_error() {
        // The colon is reserved for the label `namespace:value` separator and is
        // never valid inside an address, even carried inside a single component.
        let err = parse_kind_segmented_address("@/rule/foo:bar").unwrap_err();
        match err {
            ItemError::InvalidAddress { address, reason } => {
                assert_eq!(address, "@/rule/foo:bar");
                assert!(reason.contains("colon"), "reason was: {reason}");
            }
            other => panic!("expected InvalidAddress, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_kind_segmented_address_colon_in_project_name_is_error() {
        let err = parse_kind_segmented_address("@bad:name/rule/x").unwrap_err();
        match err {
            ItemError::InvalidAddress { address, reason } => {
                assert_eq!(address, "@bad:name/rule/x");
                assert!(reason.contains("colon"), "reason was: {reason}");
            }
            other => panic!("expected InvalidAddress, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_kind_segmented_address_colon_in_issue_self_id_is_error() {
        let err = parse_kind_segmented_address("@/issue/56ab0224/requirement/REQ:01").unwrap_err();
        match err {
            ItemError::InvalidAddress { address, reason } => {
                assert_eq!(address, "@/issue/56ab0224/requirement/REQ:01");
                assert!(reason.contains("colon"), "reason was: {reason}");
            }
            other => panic!("expected InvalidAddress, got {other:?}"),
        }
    }

    #[test]
    fn test_same_self_id_distinct_across_scopes() {
        // REQ-04: the SAME self-id under two different scopes does not conflict and
        // yields two distinct qualified ids.
        let issue = crate::domain::types::fixture_issue(
            "T".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: issue one\n".to_string(),
        );
        let issue_items = index_items(&issue, &[req_kind()], &MarkdownContentParser).unwrap();
        let project_items =
            derive_scope_items(&Scope::Project, vec![raw("requirement", "REQ-01")]).unwrap();

        assert_eq!(issue_items.len(), 1);
        assert_eq!(project_items.len(), 1);
        // Distinct qualified ids: the issue-scope address carries the reserved
        // `issue` segment and short-id, the project one carries the bare `@`.
        assert_ne!(issue_items[0].qualified_id, project_items[0].qualified_id);
        assert_eq!(issue_items[0].self_id, project_items[0].self_id);
        assert_eq!(project_items[0].qualified_id, "@/requirement/REQ-01");
        assert_eq!(
            issue_items[0].qualified_id,
            format!("@/issue/{}/requirement/REQ-01", issue.short_id())
        );
        // Both minted ids now carry the `@` sentinel; the issue form's `/issue/`
        // segment is what distinguishes it from the project form.
        assert!(issue_items[0].qualified_id.starts_with("@/issue/"));
        assert!(!project_items[0].qualified_id.starts_with("@/issue/"));
    }

    #[test]
    fn test_expand_sugar_address_matches_decision_kind() {
        // REQ-01: a self-id matching exactly one issue-scoped kind's id-pattern
        // expands to that kind, with the address's short-id carried as
        // AddressScope::Issue and self_id passed through verbatim.
        let addr = expand_sugar_address("56ab0224/D-1", &[decision_kind(), risk_kind()]).unwrap();
        assert_eq!(addr.scope, AddressScope::Issue("56ab0224".to_string()));
        assert_eq!(addr.kind, "decision");
        assert_eq!(addr.self_id, "D-1");
    }

    #[test]
    fn test_expand_sugar_address_matches_risk_kind() {
        // REQ-01: a second, distinct issue-scoped kind resolves the same way.
        let addr =
            expand_sugar_address("56ab0224/RISK-1", &[decision_kind(), risk_kind()]).unwrap();
        assert_eq!(addr.scope, AddressScope::Issue("56ab0224".to_string()));
        assert_eq!(addr.kind, "risk");
        assert_eq!(addr.self_id, "RISK-1");
    }

    #[test]
    fn test_expand_sugar_address_no_match_is_error() {
        // REQ-02: a self-id matching no issue-scoped kind's id-pattern is a
        // distinct typed error, not the ambiguous-match error.
        let err =
            expand_sugar_address("56ab0224/REQ-01", &[decision_kind(), risk_kind()]).unwrap_err();
        match err {
            ItemError::SugarKindNotFound { address, self_id } => {
                assert_eq!(address, "56ab0224/REQ-01");
                assert_eq!(self_id, "REQ-01");
            }
            other => panic!("expected SugarKindNotFound, got {other:?}"),
        }
    }

    #[test]
    fn test_expand_sugar_address_ambiguous_match_lists_all_candidates() {
        // REQ-03: two ISSUE-scoped kinds with deliberately overlapping
        // id-patterns both match "X-1"; the error names every matching
        // candidate kind, not just the first.
        let alpha = ItemKind::from_config(
            "alpha",
            &ItemKindConfig {
                id_pattern: Some("X-[0-9]+".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let beta = ItemKind::from_config(
            "beta",
            &ItemKindConfig {
                id_pattern: Some("[A-Z]-[0-9]+".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let err = expand_sugar_address("56ab0224/X-1", &[alpha, beta]).unwrap_err();
        match err {
            ItemError::SugarKindAmbiguous {
                self_id,
                candidates,
                ..
            } => {
                assert_eq!(self_id, "X-1");
                assert_eq!(candidates, vec!["alpha".to_string(), "beta".to_string()]);
            }
            other => panic!("expected SugarKindAmbiguous, got {other:?}"),
        }
    }

    #[test]
    fn test_expand_sugar_address_excludes_project_scoped_kind() {
        // REQ-04: `invariant` is project-scoped and shares `requirement`'s exact
        // id-pattern, but is excluded from candidate matching regardless of the
        // pattern match — only the issue-scoped `requirement` kind is selected.
        let addr =
            expand_sugar_address("56ab0224/REQ-01", &[invariant_kind(), req_kind()]).unwrap();
        assert_eq!(addr.scope, AddressScope::Issue("56ab0224".to_string()));
        assert_eq!(addr.kind, "requirement");
        assert_eq!(addr.self_id, "REQ-01");
    }

    #[test]
    fn test_expand_sugar_address_missing_separator_is_invalid_address() {
        // Not a two-segment sugar form at all: a typed error, not a panic.
        let err = expand_sugar_address("REQ-01", &[req_kind()]).unwrap_err();
        assert!(matches!(err, ItemError::InvalidAddress { .. }));
    }

    #[test]
    fn test_expand_sugar_address_colon_in_self_id_is_error() {
        // The colon is reserved for the label `namespace:value` separator and is
        // never valid inside an address.
        let err = expand_sugar_address("56ab0224/REQ:01", &[req_kind()]).unwrap_err();
        match err {
            ItemError::InvalidAddress { address, reason } => {
                assert_eq!(address, "56ab0224/REQ:01");
                assert!(reason.contains("colon"), "reason was: {reason}");
            }
            other => panic!("expected InvalidAddress, got {other:?}"),
        }
    }
}
