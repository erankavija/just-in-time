//! Graph-template data model and `.jit/templates.toml` loader.
//!
//! A **graph template** is a named, parameterized subgraph that `jit apply`
//! instantiates into the work DAG. The first (and, this epic, only) template is
//! `plan`: the plan-before-fan-out bracket of a planning node `P` and a
//! breakdown node `B`. The mechanism is generic and domain-agnostic — nothing
//! here hardcodes `epic` / `planning` / `breakdown`; those strings come from
//! `.jit/templates.toml`.
//!
//! This module defines the MODEL ([`GraphTemplate`], [`TemplateNode`],
//! [`AnchorSlot`], [`AnchorEdge`], [`Transform`]), the BINDINGS
//! ([`RoleBindings`], [`AnchorBindings`]) that name the roles and anchor the
//! bracket tooling reaches for, and the LOADER ([`TemplateRegistry::load`]) with
//! load-time **structural validation**:
//!
//! - node `role`s are unique within a template;
//! - every `depends_on`, `anchor_edges`, and `transforms` reference resolves to
//!   a declared node role / anchor name;
//! - each `applies_to` type exists in the configured `[type_hierarchy].types`;
//! - each transform `kind` decodes to a [`TransformKind`];
//! - the internal `depends_on` edges form a DAG (no cycle).
//!
//! Following the production rules filesystem boundary
//! precedent: an absent `templates.toml` loads as an empty registry, and an
//! invalid file fails at load with a descriptive [`TemplateConfigError`].
//!
//! The repository declarations applying a template depends on — node types,
//! label namespaces, gates, and document areas — are its
//! [`requirements`](GraphTemplate::requirements), checked together by the apply
//! engine so one failed application names every unmet one.

use crate::config::DocumentationConfig;
use crate::domain::artifact_directory::{resolve_artifact_directory, ArtifactDirectoryError};
use crate::domain::identity::{GateKey, IssueId};
use crate::domain::type_taxonomy::HierarchyConfig;
use crate::domain::{Issue, State};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::vocabulary::wire_vocabulary;
use crate::repository_state::{RepositoryLayout, RepositoryLayoutError, VirtualPath};

/// Errors that can occur while loading and validating `.jit/templates.toml`.
#[derive(Debug, Error)]
pub enum TemplateConfigError {
    /// The repository layout could not resolve the templates path.
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),

    /// The templates file could not be read from disk.
    #[error("failed to read templates file '{path}': {source}")]
    Io {
        /// Path that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// The templates file is not valid TOML or does not match the schema.
    #[error("failed to parse templates file: {0}")]
    Toml(#[from] toml::de::Error),

    /// Two templates share the same `name`.
    #[error("duplicate template name '{name}': template names must be unique")]
    DuplicateTemplate {
        /// The name that appeared more than once.
        name: String,
    },

    /// Two nodes within one template share the same `role`.
    #[error("template '{template}': duplicate node role '{role}'; roles must be unique within a template")]
    DuplicateRole {
        /// Name of the offending template.
        template: String,
        /// The role that appeared more than once.
        role: String,
    },

    /// Two anchors within one template share the same `name`.
    #[error("template '{template}': duplicate anchor name '{anchor}'; anchor names must be unique within a template")]
    DuplicateAnchor {
        /// Name of the offending template.
        template: String,
        /// The anchor name that appeared more than once.
        anchor: String,
    },

    /// A `depends_on`, `anchor_edges`, or `transforms` entry references a node
    /// role that no node declares.
    #[error("template '{template}': {context} references undeclared node role '{role}'")]
    UnknownRole {
        /// Name of the offending template.
        template: String,
        /// Where the dangling reference appeared (e.g. `"node 'breakdown' depends_on"`).
        context: String,
        /// The role that was referenced but not declared.
        role: String,
    },

    /// An `anchor_edges` entry references an anchor name that no anchor declares.
    #[error("template '{template}': anchor_edge references undeclared anchor '{anchor}'")]
    UnknownAnchor {
        /// Name of the offending template.
        template: String,
        /// The anchor name that was referenced but not declared.
        anchor: String,
    },

    /// An `applies_to` entry names a container type absent from
    /// `[type_hierarchy].types`.
    #[error("template '{template}': applies_to contains '{type_name}', which is not declared in [type_hierarchy].types")]
    UnknownAppliesToType {
        /// Name of the offending template.
        template: String,
        /// The undeclared container type name.
        type_name: String,
    },

    /// The internal `depends_on` edges form a cycle.
    #[error(
        "template '{template}': internal depends_on edges form a cycle involving role '{role}'"
    )]
    CyclicDependsOn {
        /// Name of the offending template.
        template: String,
        /// A role participating in the detected cycle.
        role: String,
    },

    /// A template does not match the schema (e.g. a transform declares an
    /// unknown `kind`).
    #[error("template '{template}': {source}")]
    InvalidTemplate {
        /// Name of the offending template.
        template: String,
        /// Underlying schema error.
        source: toml::de::Error,
    },

    /// `applies_to` is empty: a template must name at least one container type.
    #[error("template '{template}': applies_to must list at least one container type")]
    EmptyAppliesTo {
        /// Name of the offending template.
        template: String,
    },
}

wire_vocabulary! {
    /// A graph-transform kind, decoded when the template loads.
    ///
    /// An unknown kind fails loading, so a template that reaches `jit apply`
    /// declares only kinds the engine can dispatch.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum TransformKind {
        /// Move the container's pre-apply upstream deps onto a named role's node.
        MoveUpstreamToRole = "move-upstream-to-role",
    }
}

/// A repository declaration applying a template depends on
/// ([`GraphTemplate::requirements`]), by the registry that declares it.
///
/// Serializes as `{"kind": "<kind>", "name": "<declared name>"}`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(tag = "kind", content = "name", rename_all = "kebab-case")]
pub enum TemplateRequirement {
    /// A node's issue type, declared in `[type_hierarchy].types`.
    Type(String),
    /// The namespace of a label a node writes, declared in `[namespaces]`.
    LabelNamespace(String),
    /// A node or anchor gate, supplied by a gate preset or the gate registry.
    Gate(GateKey),
    /// A node's `doc_area`, declared in `[documentation] issue_scoped_areas`.
    DocumentationArea(String),
}

impl std::fmt::Display for TemplateRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Type(name) => write!(f, "type '{name}' in [type_hierarchy].types"),
            Self::LabelNamespace(name) => write!(f, "label namespace '{name}' in [namespaces]"),
            Self::Gate(key) => write!(f, "gate '{key}' as a gate preset or in the gate registry"),
            Self::DocumentationArea(area) => write!(
                f,
                "documentation area '{area}' in [documentation] issue_scoped_areas"
            ),
        }
    }
}

/// Applying a template found requirements the repository does not declare.
///
/// Names every unmet [`TemplateRequirement`] at once, in requirement order; the
/// CLI renders the template and the requirements into the `--json` envelope's
/// `details`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplatePrerequisitesUnmetError {
    template: String,
    unmet: Vec<TemplateRequirement>,
}

impl TemplatePrerequisitesUnmetError {
    /// Report `unmet` requirements of the template named `template`.
    pub fn new(template: impl Into<String>, unmet: Vec<TemplateRequirement>) -> Self {
        Self {
            template: template.into(),
            unmet,
        }
    }

    /// The template whose requirements are unmet.
    pub fn template(&self) -> &str {
        &self.template
    }

    /// Every unmet requirement.
    pub fn unmet(&self) -> &[TemplateRequirement] {
        &self.unmet
    }
}

impl std::fmt::Display for TemplatePrerequisitesUnmetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "template '{}' requires {} repository declaration(s) this repository lacks; declare each, then re-apply:",
            self.template,
            self.unmet.len()
        )?;
        self.unmet
            .iter()
            .try_for_each(|requirement| write!(f, "\n  - {requirement}"))
    }
}

impl std::error::Error for TemplatePrerequisitesUnmetError {}

/// A loaded, validated set of graph templates from `.jit/templates.toml`, plus
/// the repository's [`RoleBindings`] and [`AnchorBindings`].
///
/// Built by [`TemplateRegistry::load`] (file → registry) or
/// [`TemplateRegistry::from_toml_str`] (string → registry). An absent file
/// yields an empty registry with default bindings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateRegistry {
    /// Repository names for the node roles the bracket tooling reaches for
    /// (the `[roles]` table). Absent keys resolve to their defaults.
    #[serde(default)]
    pub roles: RoleBindings,
    /// Repository names for the anchors the CLI binds implicitly (the
    /// `[anchors]` table). Absent keys resolve to their defaults.
    #[serde(default)]
    pub anchors: AnchorBindings,
    /// The declared templates, in authored order.
    #[serde(default, rename = "template")]
    pub templates: Vec<GraphTemplate>,
}

/// The role name [`RoleBindings::planning_role`] resolves to when `[roles]
/// planning` is undeclared. The SOLE place this name lives.
pub const DEFAULT_PLANNING_ROLE: &str = "planning";

/// The role name [`RoleBindings::breakdown_role`] resolves to when `[roles]
/// breakdown` is undeclared. The SOLE place this name lives.
pub const DEFAULT_BREAKDOWN_ROLE: &str = "breakdown";

/// The anchor name [`AnchorBindings::container_anchor`] resolves to when
/// `[anchors] container` is undeclared. The SOLE place this name lives.
pub const DEFAULT_CONTAINER_ANCHOR: &str = "container";

/// The repository's names for the two template node roles the bracket tooling
/// reaches for by meaning: the node that holds the plan, and the node that holds
/// the fan-out.
///
/// Declared in `.jit/templates.toml` as a top-level `[roles]` table; an absent
/// table (or an absent key) resolves to [`DEFAULT_PLANNING_ROLE`] /
/// [`DEFAULT_BREAKDOWN_ROLE`], so a repository that names its roles the usual way
/// declares nothing. Binding these keeps `jit apply`, bracket breakdown, forced
/// refresh, and validation free of any role literal (`@/inv/domain-agnostic`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleBindings {
    /// Name of the role carried by the planning node `P`. Resolve it with
    /// [`RoleBindings::planning_role`], which supplies the default.
    #[serde(default)]
    pub planning: Option<String>,
    /// Name of the role carried by the breakdown node `B`. Resolve it with
    /// [`RoleBindings::breakdown_role`], which supplies the default.
    #[serde(default)]
    pub breakdown: Option<String>,
}

impl RoleBindings {
    /// The role the planning node `P` carries, defaulting to
    /// [`DEFAULT_PLANNING_ROLE`].
    pub fn planning_role(&self) -> &str {
        self.planning.as_deref().unwrap_or(DEFAULT_PLANNING_ROLE)
    }

    /// The role the breakdown node `B` carries, defaulting to
    /// [`DEFAULT_BREAKDOWN_ROLE`].
    pub fn breakdown_role(&self) -> &str {
        self.breakdown.as_deref().unwrap_or(DEFAULT_BREAKDOWN_ROLE)
    }
}

/// The repository's names for the template anchors the CLI binds implicitly.
///
/// Declared in `.jit/templates.toml` as a top-level `[anchors]` table. The single
/// implicit binding is the CONTAINER anchor: `jit apply <template> <container>`
/// binds it to the positional `<container>` argument, so a template whose
/// container anchor is named `target` needs no `--anchor target=…`. An absent
/// table (or key) resolves to [`DEFAULT_CONTAINER_ANCHOR`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorBindings {
    /// Name of the anchor bound to `jit apply`'s positional `<container>`.
    /// Resolve it with [`AnchorBindings::container_anchor`], which supplies the
    /// default.
    #[serde(default)]
    pub container: Option<String>,
}

impl AnchorBindings {
    /// The anchor bound to `jit apply`'s positional `<container>` argument,
    /// defaulting to [`DEFAULT_CONTAINER_ANCHOR`].
    pub fn container_anchor(&self) -> &str {
        self.container
            .as_deref()
            .unwrap_or(DEFAULT_CONTAINER_ANCHOR)
    }
}

/// A named, parameterized subgraph applied to a container by `jit apply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphTemplate {
    /// Unique template name (e.g. `"plan"`), the `jit apply <name>` selector.
    pub name: String,
    /// Human-readable description of what the template does.
    #[serde(default)]
    pub description: Option<String>,
    /// Container types this template may be applied to (e.g. `["epic"]`). Each
    /// should also appear in `[type_hierarchy].types`.
    pub applies_to: Vec<String>,
    /// Named anchor slots bound to existing issues at apply time.
    #[serde(default)]
    pub anchors: Vec<AnchorSlot>,
    /// The nodes the template creates.
    #[serde(default)]
    pub nodes: Vec<TemplateNode>,
    /// Edges between a bound anchor and a created node.
    #[serde(default)]
    pub anchor_edges: Vec<AnchorEdge>,
    /// Graph transforms applied after node creation and edge wiring.
    #[serde(default)]
    pub transforms: Vec<Transform>,
}

/// A named anchor slot, bound at apply time to an existing issue.
///
/// The `plan` template has a single anchor, `container`, bound to the target
/// issue; templates may declare several.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorSlot {
    /// The anchor's name, referenced by `anchor_edges.from`.
    pub name: String,
    /// Gate or preset keys attached to the bound issue, like a node's
    /// [`gates`](TemplateNode::gates). Spelling is checked on decode;
    /// existence is checked at apply time.
    #[serde(default)]
    pub gates: Vec<GateKey>,
}

/// A node the template creates when applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateNode {
    /// Role identifying this node within the template (e.g. `"planning"`),
    /// unique per template. Roles are template metadata, not stored on the
    /// created issue.
    pub role: String,
    /// Issue type the created node carries (e.g. `"planning"`); a
    /// [`TemplateRequirement::Type`] of applying the template.
    #[serde(rename = "type")]
    pub type_name: String,
    /// Gate or preset keys attached to the node. Spelling is checked on decode;
    /// existence is checked at apply time.
    #[serde(default)]
    pub gates: Vec<GateKey>,
    /// Plan-doc location template for the node, with `{...}` interpolation
    /// tokens resolved at apply time.
    ///
    /// A node that declares a [`doc_area`](Self::doc_area) names its document
    /// relative to `{container.dir}`, the canonical artifact directory the
    /// container owns in that area (e.g. `"{container.dir}/plan.md"`). Without an
    /// area declaration that token is left verbatim, so such a node's path names
    /// its own location.
    #[serde(default)]
    pub doc: Option<String>,
    /// Issue-scoped area the node's document belongs in (e.g. `"dev/active"`),
    /// selecting the area `{container.dir}` resolves in at apply time.
    ///
    /// The area is matched against the repository's issue-scoped area registry
    /// ([`DocumentationConfig::issue_scoped_areas`](crate::config::DocumentationConfig::issue_scoped_areas)),
    /// so an undeclared area fails the apply rather than producing a path
    /// outside the convention. Absent leaves `{container.dir}` out of scope.
    #[serde(default)]
    pub doc_area: Option<String>,
    /// Interpolated description seeded onto the created node.
    #[serde(default)]
    pub description: Option<String>,
    /// Additional labels (interpolated) set on the created node, beyond the
    /// container's inherited membership labels.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Roles of other nodes in this template that the node depends on (internal
    /// edges, e.g. breakdown `depends_on = ["planning"]` wires `B → P`).
    #[serde(default)]
    pub depends_on: Vec<String>,
}

/// An edge between a bound anchor and a created node.
///
/// Direction is "anchor depends on node": the issue bound to `from` gains a
/// dependency on the node created for `to` (e.g. `container` depends on
/// `breakdown`, wiring `C → B`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorEdge {
    /// Anchor name the edge originates from (the dependent side).
    pub from: String,
    /// Node role the edge points to (the dependency side).
    pub to: String,
}

/// A graph transform applied after nodes are created and edges wired.
///
/// The shipped kind is `move-upstream-to-role`, which moves the container's
/// pre-apply upstream dependencies onto the node of the named role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transform {
    /// Transform kind.
    pub kind: TransformKind,
    /// Target node role the transform acts on.
    pub role: String,
}

impl TemplateRegistry {
    /// An empty registry (used when no `templates.toml` exists).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Look up a template by name.
    pub fn get(&self, name: &str) -> Option<&GraphTemplate> {
        self.templates.iter().find(|t| t.name == name)
    }

    /// The set of breakable container types: the union of every template's
    /// `applies_to`, deduplicated, in first-seen order.
    ///
    /// This is the set of breakable container types declared by the registry:
    /// a container type appears here iff some template may be applied to it.
    pub fn breakable_types(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        self.templates
            .iter()
            .flat_map(|t| t.applies_to.iter())
            .filter(|ty| seen.insert(ty.as_str()))
            .cloned()
            .collect()
    }

    /// The template applicable to a container `type` (e.g. `"epic"`): the first
    /// template whose `applies_to` lists that type, or `None` when no template
    /// brackets it.
    pub fn template_for_container(&self, container_type: &str) -> Option<&GraphTemplate> {
        self.templates
            .iter()
            .find(|t| t.applies_to.iter().any(|ty| ty == container_type))
    }

    /// Load and validate `.jit/templates.toml` relative to the given `.jit` root.
    ///
    /// Returns an empty registry when the file does not exist. `hierarchy_types`
    /// is the configured `[type_hierarchy].types` key set, against which
    /// `applies_to` types are checked; pass an empty slice to skip the type check
    /// (when no hierarchy is configured).
    pub fn load<S: AsRef<str>>(
        layout: &RepositoryLayout,
        hierarchy_types: &[S],
    ) -> Result<Self, TemplateConfigError> {
        let path = layout.resolve(&VirtualPath::TEMPLATES)?;
        if !path.exists() {
            return Ok(Self::empty());
        }
        let content = std::fs::read_to_string(&path).map_err(|source| TemplateConfigError::Io {
            path: path.clone(),
            source,
        })?;
        Self::from_toml_str(&content, hierarchy_types)
    }

    /// Parse and validate a `templates.toml` string.
    ///
    /// `hierarchy_types` is the configured `[type_hierarchy].types` key set;
    /// pass an empty slice to skip the `applies_to` type check.
    pub fn from_toml_str<S: AsRef<str>>(
        content: &str,
        hierarchy_types: &[S],
    ) -> Result<Self, TemplateConfigError> {
        let mut table: toml::Table = toml::from_str(content)?;
        let templates = match table.remove("template") {
            Some(toml::Value::Array(items)) => items
                .into_iter()
                .map(GraphTemplate::from_value)
                .collect::<Result<Vec<_>, _>>()?,
            Some(other) => {
                table.insert("template".to_string(), other);
                Vec::new()
            }
            None => Vec::new(),
        };
        let registry = TemplateRegistry {
            templates,
            ..table.try_into()?
        };

        // Template names must be unique so `jit apply <name>` is unambiguous.
        let mut seen_templates = HashSet::new();
        if let Some(template) = registry
            .templates
            .iter()
            .find(|t| !seen_templates.insert(t.name.as_str()))
        {
            return Err(TemplateConfigError::DuplicateTemplate {
                name: template.name.clone(),
            });
        }

        let known_type = |name: &str| {
            hierarchy_types.is_empty() || hierarchy_types.iter().any(|t| t.as_ref() == name)
        };

        for template in &registry.templates {
            template.validate(&known_type)?;
        }

        Ok(registry)
    }
}

impl GraphTemplate {
    /// Decode one `[[template]]` entry, naming the template on a schema error.
    fn from_value(value: toml::Value) -> Result<Self, TemplateConfigError> {
        let name = value
            .get("name")
            .and_then(toml::Value::as_str)
            .map(str::to_owned);
        value.try_into().map_err(|source| match name {
            Some(template) => TemplateConfigError::InvalidTemplate { template, source },
            None => TemplateConfigError::Toml(source),
        })
    }

    /// The repository declarations applying this template depends on, derived
    /// from its own nodes and anchors; empty for a template that declares none.
    ///
    /// A label's namespace is static while its value may be interpolated, so a
    /// label whose namespace is itself interpolated or absent names none.
    pub fn requirements(&self) -> BTreeSet<TemplateRequirement> {
        let node_requirements = self.nodes.iter().flat_map(|node| {
            let namespaces = std::iter::once(crate::labels::TYPE_NAMESPACE)
                .chain(node.labels.iter().filter_map(|label| {
                    label
                        .split_once(':')
                        .map(|(namespace, _)| namespace)
                        .filter(|namespace| !namespace.is_empty() && !namespace.contains('{'))
                }))
                .map(|namespace| TemplateRequirement::LabelNamespace(namespace.to_owned()));
            std::iter::once(TemplateRequirement::Type(node.type_name.clone()))
                .chain(namespaces)
                .chain(node.gates.iter().cloned().map(TemplateRequirement::Gate))
                .chain(
                    node.doc_area
                        .iter()
                        .cloned()
                        .map(TemplateRequirement::DocumentationArea),
                )
        });
        let anchor_gates = self
            .anchors
            .iter()
            .flat_map(|anchor| anchor.gates.iter().cloned().map(TemplateRequirement::Gate));
        node_requirements.chain(anchor_gates).collect()
    }

    /// The [`requirements`](Self::requirements) that `config`, the gate
    /// `registry`, and the gate `presets` leave undeclared, in requirement order.
    ///
    /// An empty type hierarchy or namespace registry declares no vocabulary, so
    /// it requires none, as the default rules keyed on those registries do. Pure,
    /// so every checker is a view over what `jit apply` enforces
    /// (`@/charter/D-20`).
    pub fn unmet_requirements(
        &self,
        config: &crate::config::JitConfig,
        registry: &crate::declarations::GateRegistry,
        presets: &HashMap<GateKey, crate::gate_presets::GatePresetDefinition>,
    ) -> Vec<TemplateRequirement> {
        let namespaces = crate::config_manager::namespaces_from_config(config);
        let types = namespaces.declared_type_hierarchy();
        let documentation = config.documentation.clone().unwrap_or_default();
        let is_declared = |requirement: &TemplateRequirement| match requirement {
            TemplateRequirement::Type(name) => types.is_empty() || types.contains_key(name),
            TemplateRequirement::LabelNamespace(name) => {
                namespaces.namespaces.is_empty() || namespaces.namespaces.contains_key(name)
            }
            TemplateRequirement::Gate(key) => {
                presets.contains_key(key) || registry.gates.contains_key(key)
            }
            TemplateRequirement::DocumentationArea(area) => {
                documentation.is_issue_scoped_area(area)
            }
        };
        self.requirements()
            .into_iter()
            .filter(|requirement| !is_declared(requirement))
            .collect()
    }

    /// The node carrying the given `role`, or `None` if no node declares it.
    pub fn node(&self, role: &str) -> Option<&TemplateNode> {
        self.nodes.iter().find(|n| n.role == role)
    }

    /// The planning node `P` ([`RoleBindings::planning_role`]), or `None` if the
    /// template has none.
    pub fn planning_node(&self, roles: &RoleBindings) -> Option<&TemplateNode> {
        self.node(roles.planning_role())
    }

    /// The breakdown node `B` ([`RoleBindings::breakdown_role`]), or `None` if the
    /// template has none.
    pub fn breakdown_node(&self, roles: &RoleBindings) -> Option<&TemplateNode> {
        self.node(roles.breakdown_role())
    }

    /// The issue type carried by the planning node `P` (e.g. `"planning"`).
    pub fn planning_type(&self, roles: &RoleBindings) -> Option<&str> {
        self.planning_node(roles).map(|n| n.type_name.as_str())
    }

    /// The issue type carried by the breakdown node `B` (e.g. `"breakdown"`).
    pub fn breakdown_type(&self, roles: &RoleBindings) -> Option<&str> {
        self.breakdown_node(roles).map(|n| n.type_name.as_str())
    }

    /// The planning node's doc-location template (e.g.
    /// `"{container.dir}/plan.md"`), with `{...}` tokens resolved at apply time.
    pub fn plan_doc_location(&self, roles: &RoleBindings) -> Option<&str> {
        self.planning_node(roles).and_then(|n| n.doc.as_deref())
    }

    /// Validate one template's internal structure: unique roles/anchors, every
    /// reference resolves, `applies_to` types are known, and `depends_on` is
    /// acyclic.
    ///
    /// `known_type` answers whether a type name is declared in the hierarchy
    /// (always `true` when no hierarchy is configured). Pure: no I/O.
    fn validate(&self, known_type: &impl Fn(&str) -> bool) -> Result<(), TemplateConfigError> {
        if self.applies_to.is_empty() {
            return Err(TemplateConfigError::EmptyAppliesTo {
                template: self.name.clone(),
            });
        }

        // Each declared container type must exist in the hierarchy (skipped when
        // no hierarchy is configured, via `known_type`), so a typo'd
        // `applies_to` type is rejected rather than silently registered.
        for container_type in &self.applies_to {
            if !known_type(container_type) {
                return Err(TemplateConfigError::UnknownAppliesToType {
                    template: self.name.clone(),
                    type_name: container_type.clone(),
                });
            }
        }

        // Roles unique within the template.
        let mut roles = HashSet::new();
        for node in &self.nodes {
            if !roles.insert(node.role.as_str()) {
                return Err(TemplateConfigError::DuplicateRole {
                    template: self.name.clone(),
                    role: node.role.clone(),
                });
            }
        }

        // Anchor names unique within the template.
        let mut anchors = HashSet::new();
        for anchor in &self.anchors {
            if !anchors.insert(anchor.name.as_str()) {
                return Err(TemplateConfigError::DuplicateAnchor {
                    template: self.name.clone(),
                    anchor: anchor.name.clone(),
                });
            }
        }

        // `depends_on` references a declared role.
        for node in &self.nodes {
            for dep in &node.depends_on {
                if !roles.contains(dep.as_str()) {
                    return Err(TemplateConfigError::UnknownRole {
                        template: self.name.clone(),
                        context: format!("node '{}' depends_on", node.role),
                        role: dep.clone(),
                    });
                }
            }
        }

        // anchor_edges reference a declared anchor (`from`) and node role (`to`).
        for edge in &self.anchor_edges {
            if !anchors.contains(edge.from.as_str()) {
                return Err(TemplateConfigError::UnknownAnchor {
                    template: self.name.clone(),
                    anchor: edge.from.clone(),
                });
            }
            if !roles.contains(edge.to.as_str()) {
                return Err(TemplateConfigError::UnknownRole {
                    template: self.name.clone(),
                    context: "anchor_edge `to`".to_string(),
                    role: edge.to.clone(),
                });
            }
        }

        // transforms reference a declared node role.
        for transform in &self.transforms {
            if !roles.contains(transform.role.as_str()) {
                return Err(TemplateConfigError::UnknownRole {
                    template: self.name.clone(),
                    context: format!("transform '{}' role", transform.kind),
                    role: transform.role.clone(),
                });
            }
        }

        self.check_acyclic()?;
        Ok(())
    }

    /// Detect a cycle in the internal `depends_on` edges over the role adjacency,
    /// reporting the first role on the closed path (`@/inv/dag-acyclic`).
    fn check_acyclic(&self) -> Result<(), TemplateConfigError> {
        let adjacency: Vec<(&str, Vec<&str>)> = self
            .nodes
            .iter()
            .map(|n| {
                (
                    n.role.as_str(),
                    n.depends_on.iter().map(String::as_str).collect(),
                )
            })
            .collect();

        match crate::graph::find_keyed_cycle(&adjacency) {
            Some(cycle) => Err(TemplateConfigError::CyclicDependsOn {
                template: self.name.clone(),
                // The cycle is a closed path, so its first key is on the cycle.
                role: cycle.first().map(|r| (*r).to_string()).unwrap_or_default(),
            }),
            None => Ok(()),
        }
    }
}

/// Find the planning dependency of a breakdown node carrying the template-declared
/// container label. `by_id` must cover the whole repository, even for scoped validation.
fn find_planning_node<'a>(
    container: &Issue,
    template: &GraphTemplate,
    roles: &RoleBindings,
    by_id: &HashMap<&IssueId, &'a Issue>,
) -> Option<&'a Issue> {
    let planning_type = template.planning_type(roles)?;
    let bracket_label = declared_container_label(template, roles, container).ok()?;
    let planning_type_label = crate::labels::type_label(planning_type);

    by_id
        .values()
        .filter(|b| b.labels.contains(&bracket_label))
        .find_map(|b| {
            b.dependencies.iter().find_map(|dep_id| {
                by_id
                    .get(dep_id)
                    .copied()
                    .filter(|p| p.labels.contains(&planning_type_label))
            })
        })
}

/// Linked plan documents remain authoritative when moved or archived.
const PLAN_DOC_LABEL: &str = "plan";

/// A breakable container's linked plan document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanDocumentRef {
    /// Repository-relative path.
    pub(crate) path: String,
    /// Whether the planning node is `done`, which makes an absent plan an error.
    pub(crate) planning_done: bool,
}

/// The `plan`-labelled document of `container`'s planning node, when its template
/// declares a plan-document location; `None` for an inline plan. `by_id` must
/// cover the whole repository.
pub(crate) fn plan_document_ref(
    container: &Issue,
    by_id: &HashMap<&IssueId, &Issue>,
    templates: &TemplateRegistry,
) -> Option<PlanDocumentRef> {
    let template = crate::labels::type_label_value(&container.labels)
        .and_then(|container_type| templates.template_for_container(container_type))
        .filter(|template| template.plan_doc_location(&templates.roles).is_some())?;
    let planning = find_planning_node(container, template, &templates.roles, by_id)?;
    planning
        .documents
        .iter()
        .find(|d| d.label.as_deref() == Some(PLAN_DOC_LABEL))
        .map(|d| PlanDocumentRef {
            path: d.path.clone(),
            planning_done: planning.state == State::Done,
        })
}

/// Fixed template-token substitution shared by container lookup and node expansion.
#[derive(Debug, Clone)]
pub(crate) struct InterpolationContext {
    id: IssueId,
    pub(crate) title: String,
    hard_criteria: String,
    directory: Option<String>,
    pub(crate) doc: Option<String>,
}

impl InterpolationContext {
    /// Resolve container tokens, the node's artifact directory, and its own `{doc}`.
    /// Returns an error when the node's area is not configured as issue-scoped.
    pub(crate) fn for_node(
        container: &Issue,
        node: &TemplateNode,
        documentation: &DocumentationConfig,
        hierarchy: &HierarchyConfig,
    ) -> std::result::Result<Self, ArtifactDirectoryError> {
        let directory = node
            .doc_area
            .as_deref()
            .map(|area| resolve_artifact_directory(container, area, documentation, hierarchy))
            .transpose()?;
        Ok(Self::for_container(container).with_doc(node, directory))
    }

    /// Build the container-derived context (the `{container.dir}` and `{doc}`
    /// tokens are unset until a node is selected via [`with_doc`](Self::with_doc)).
    fn for_container(container: &Issue) -> Self {
        Self {
            id: container.id.clone(),
            title: container.title.clone(),
            hard_criteria: extract_hard_criteria(&container.description),
            directory: None,
            doc: None,
        }
    }

    /// Resolve the node's document with its directory in scope and `{doc}` still unset.
    fn with_doc(&self, node: &TemplateNode, directory: Option<String>) -> Self {
        let scoped = Self {
            directory,
            ..self.clone()
        };
        let doc = node
            .doc
            .as_deref()
            .map(|template| scoped.interpolate(template))
            .unwrap_or_default();
        Self {
            doc: Some(doc),
            ..scoped
        }
    }

    /// Substitute every supported `{token}` in `template` with its context value.
    ///
    /// A token this context does not carry — `{container.dir}` for a node that
    /// declares no area, `{doc}` before a node is selected — is left verbatim,
    /// like any unknown `{...}` text.
    pub(crate) fn interpolate(&self, template: &str) -> String {
        let mut out = template
            .replace("{container.id}", self.id.as_str())
            .replace("{container.short_id}", self.id.short_id())
            .replace("{container.title}", &self.title)
            .replace("{container.hard_criteria}", &self.hard_criteria);
        if let Some(directory) = &self.directory {
            out = out.replace("{container.dir}", directory);
        }
        if let Some(doc) = &self.doc {
            out = out.replace("{doc}", doc);
        }
        out
    }
}

/// Resolve the breakdown node's first label containing a container identifier token.
pub(crate) fn declared_container_label(
    template: &GraphTemplate,
    roles: &RoleBindings,
    container: &Issue,
) -> anyhow::Result<String> {
    let node = template.breakdown_node(roles).ok_or_else(|| {
        anyhow::anyhow!(
            "template '{}' declares no '{}' node; bracket lookup needs a breakdown node label",
            template.name,
            roles.breakdown_role()
        )
    })?;
    let context = InterpolationContext::for_container(container);
    node.labels
        .iter()
        .find(|label| {
            label.contains("{container.short_id}") || label.contains("{container.id}")
        })
        .map(|label| context.interpolate(label))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "template '{}' breakdown node '{}' declares no container label; bracket lookup cannot resolve the applied node",
                template.name,
                node.role
            )
        })
}

/// Collect `[hard]` list items in order for the `{container.hard_criteria}` token.
fn extract_hard_criteria(description: &str) -> String {
    description
        .lines()
        .map(str::trim)
        .map(|line| line.trim_start_matches(['-', '*', '+']).trim())
        .filter(|line| line.starts_with("[hard]"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// An epic bracketed by a template whose planning node declares a plan
    /// location, with its planning and breakdown nodes.
    fn bracket_fixture() -> (TemplateRegistry, Issue, Issue, Issue) {
        let template_toml = r#"
[[template]]
name = "custom-plan"
applies_to = ["epic"]
[[template.nodes]]
role = "planning"
type = "planning"
doc = "plans/{container.short_id}.md"
[[template.nodes]]
role = "breakdown"
type = "breakdown"
labels = ["scaffold:{container.short_id}"]
depends_on = ["planning"]
"#;
        let registry = TemplateRegistry::from_toml_str(template_toml, &[] as &[&str]).unwrap();

        let mut container =
            crate::test_utils::fixture_issue("container".to_string(), String::new());
        container.id = crate::test_utils::issue_id(0xcd81_56d8_b74a_832a_9736_5c98_69cf_16b3);
        container.labels = vec!["type:epic".to_string()];

        let mut planning = crate::test_utils::fixture_issue("planning".to_string(), String::new());
        planning.id = crate::test_utils::issue_id(0xb4cb_51e2_ab13_a142_2f78_4c62_d893_6a00);
        planning.labels = vec!["type:planning".to_string()];

        let mut breakdown =
            crate::test_utils::fixture_issue("breakdown".to_string(), String::new());
        breakdown.id = crate::test_utils::issue_id(0x3573_5a7e_98c3_97b9_ba56_46af_e8f2_0edc);
        breakdown.labels = vec![
            "type:breakdown".to_string(),
            format!("scaffold:{}", container.short_id()),
        ];
        breakdown.dependencies = vec![planning.id.clone()];
        (registry, container, planning, breakdown)
    }

    #[test]
    fn test_find_planning_node_resolves_template_declared_container_label() {
        let (registry, container, planning, breakdown) = bracket_fixture();
        let template = registry.get("custom-plan").unwrap();
        let by_id = HashMap::from([(&planning.id, &planning), (&breakdown.id, &breakdown)]);

        let resolved = find_planning_node(&container, template, &registry.roles, &by_id).unwrap();

        assert_eq!(resolved.id, planning.id);
    }

    #[test]
    fn test_plan_document_ref_follows_planning_node_plan_link_and_state() {
        let (registry, container, planning, breakdown) = bracket_fixture();
        let resolve = |planning: &Issue| {
            let by_id = HashMap::from([(&planning.id, planning), (&breakdown.id, &breakdown)]);
            plan_document_ref(&container, &by_id, &registry)
        };

        assert_eq!(resolve(&planning), None, "an inline plan has no reference");

        let relinked = Issue {
            documents: vec![crate::domain::DocumentReference {
                label: Some(PLAN_DOC_LABEL.to_string()),
                ..crate::test_utils::document_reference("archive/moved-plan.md")
            }],
            ..planning.clone()
        };
        assert_eq!(
            resolve(&relinked),
            Some(PlanDocumentRef {
                path: "archive/moved-plan.md".to_string(),
                planning_done: false,
            })
        );

        let done = Issue {
            state: State::Done,
            ..relinked
        };
        assert!(resolve(&done).is_some_and(|plan| plan.planning_done));
    }

    #[test]
    fn test_extract_hard_criteria_pulls_only_hard_items() {
        let desc = "## Success Criteria\n\n- [hard] REQ-01: a\n- [soft] nice\n- [hard] REQ-02: b\n";
        assert_eq!(
            extract_hard_criteria(desc),
            "[hard] REQ-01: a\n[hard] REQ-02: b"
        );
    }

    #[test]
    fn test_extract_hard_criteria_empty_when_none() {
        assert_eq!(extract_hard_criteria("no criteria here"), "");
    }

    #[test]
    fn test_interpolation_resolves_container_and_doc_tokens() {
        let mut issue = crate::test_utils::fixture_issue(
            "Auth epic".to_string(),
            "- [hard] REQ-01: x".to_string(),
        );
        issue.id = crate::test_utils::issue_id(0xf456_0000_0000_0000_0000_0000_abc1_23de);
        let node = TemplateNode {
            role: "planning".to_string(),
            type_name: "planning".to_string(),
            gates: vec![],
            doc: Some("dev/active/{container.id}-plan.md".to_string()),
            doc_area: None,
            description: Some(
                "Plan {container.title} ({container.short_id}). Doc: {doc}. Cover: {container.hard_criteria}."
                    .to_string(),
            ),
            labels: vec![],
            depends_on: vec![],
        };
        let ctx = InterpolationContext::for_container(&issue).with_doc(&node, None);
        let rendered = ctx.interpolate(node.description.as_deref().unwrap());
        assert!(rendered.contains("Auth epic"));
        assert!(rendered.contains(&issue.short_id()));
        assert!(rendered.contains(&format!("dev/active/{}-plan.md", issue.id)));
        assert!(rendered.contains("[hard] REQ-01: x"));
    }

    #[test]
    fn test_interpolate_leaves_unknown_token_verbatim() {
        // The interpolation context is a fixed `{token}` substitution, not a
        // templating language: an unsupported `{token}` is left untouched while the
        // known container tokens around it still resolve.
        let mut issue = crate::test_utils::fixture_issue("Auth epic".to_string(), String::new());
        issue.id = crate::test_utils::issue_id(0xf456_0000_0000_0000_0000_0000_abc1_23de);
        let ctx = InterpolationContext::for_container(&issue);
        let rendered = ctx.interpolate("{container.title} then {totally.unknown} end");
        assert_eq!(rendered, "Auth epic then {totally.unknown} end");
    }

    /// A complete, valid `plan`-shaped template fixture for round-trip and
    /// happy-path tests. Mirrors the plan doc's schema section.
    fn plan_template_toml() -> &'static str {
        r#"
[[template]]
name        = "plan"
description = "Plan-before-fan-out bracket."
applies_to  = ["epic"]

  [[template.anchors]]
  name = "container"

  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  gates       = ["plan-review"]
  doc         = "dev/active/{container.id}-plan.md"
  description = "Planning node for {container.title}."

  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
  gates       = ["coverage-preview", "breakdown-review"]
  labels      = ["brackets:{container.short_id}"]
  description = "Breakdown node for {container.title}."
  depends_on  = ["planning"]

  [[template.anchor_edges]]
  from = "container"
  to   = "breakdown"

  [[template.transforms]]
  kind = "move-upstream-to-role"
  role = "planning"
"#
    }

    const HIERARCHY: [&str; 3] = ["epic", "planning", "breakdown"];

    // REGA-01: types round-trip through (de)serialization.

    #[test]
    fn test_plan_template_parses_full_shape() {
        let reg = TemplateRegistry::from_toml_str(plan_template_toml(), &HIERARCHY).unwrap();
        assert_eq!(reg.templates.len(), 1);
        let t = &reg.templates[0];
        assert_eq!(t.name, "plan");
        assert_eq!(
            t.description.as_deref(),
            Some("Plan-before-fan-out bracket.")
        );
        assert_eq!(t.applies_to, vec!["epic"]);
        assert_eq!(t.anchors.len(), 1);
        assert_eq!(t.anchors[0].name, "container");
        assert_eq!(t.nodes.len(), 2);

        let planning = &t.nodes[0];
        assert_eq!(planning.role, "planning");
        assert_eq!(planning.type_name, "planning");
        assert_eq!(
            planning.gates,
            vec!["plan-review".parse::<GateKey>().unwrap()]
        );
        assert_eq!(
            planning.doc.as_deref(),
            Some("dev/active/{container.id}-plan.md")
        );
        assert!(planning.depends_on.is_empty());

        let breakdown = &t.nodes[1];
        assert_eq!(breakdown.role, "breakdown");
        assert_eq!(breakdown.type_name, "breakdown");
        assert_eq!(
            breakdown.gates,
            vec!["coverage-preview", "breakdown-review"]
                .into_iter()
                .map(|key| key.parse::<GateKey>().unwrap())
                .collect::<Vec<_>>()
        );
        assert_eq!(breakdown.labels, vec!["brackets:{container.short_id}"]);
        assert_eq!(breakdown.depends_on, vec!["planning"]);

        assert_eq!(t.anchor_edges.len(), 1);
        assert_eq!(t.anchor_edges[0].from, "container");
        assert_eq!(t.anchor_edges[0].to, "breakdown");

        assert_eq!(t.transforms.len(), 1);
        assert_eq!(t.transforms[0].kind, TransformKind::MoveUpstreamToRole);
        assert_eq!(t.transforms[0].role, "planning");
    }

    #[test]
    fn test_anchor_gates_default_empty_and_parse() {
        // The shared fixture's anchor declares no gates → `#[serde(default)]`
        // yields an empty vec, exactly like an omitted node `gates`.
        let reg = TemplateRegistry::from_toml_str(plan_template_toml(), &HIERARCHY).unwrap();
        assert!(reg.templates[0].anchors[0].gates.is_empty());

        // An anchor may declare gate presets, parsed like a node's `gates`
        // (jit:2614ecf2 — REQ-13).
        let toml = r#"
[[template]]
name        = "anchored"
applies_to  = ["epic"]
  [[template.anchors]]
  name  = "container"
  gates = ["plan-review", "coverage-preview"]
  [[template.nodes]]
  role = "planning"
  type = "planning"
"#;
        let reg = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap();
        assert_eq!(
            reg.get("anchored").unwrap().anchors[0].gates,
            vec!["plan-review", "coverage-preview"]
                .into_iter()
                .map(|key| key.parse::<GateKey>().unwrap())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_node_doc_area_parses_and_defaults_to_absent() {
        // A node states the area its document belongs in; the shared fixture's
        // node declares none, so the declaration is the node's own choice.
        let toml = r#"
[[template]]
name        = "plan"
applies_to  = ["epic"]

  [[template.nodes]]
  role     = "planning"
  type     = "planning"
  doc_area = "workspace/notes"
  doc      = "{container.dir}/plan.md"
"#;
        let reg = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap();
        assert_eq!(
            reg.get("plan").unwrap().nodes[0].doc_area.as_deref(),
            Some("workspace/notes")
        );

        let undeclared = TemplateRegistry::from_toml_str(plan_template_toml(), &HIERARCHY).unwrap();
        assert!(undeclared.get("plan").unwrap().nodes[0].doc_area.is_none());
    }

    #[test]
    fn test_registry_roundtrips_through_json() {
        let reg = TemplateRegistry::from_toml_str(plan_template_toml(), &HIERARCHY).unwrap();
        let json = serde_json::to_string(&reg).unwrap();
        let back: TemplateRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(reg, back);
    }

    #[test]
    fn test_get_finds_template_by_name() {
        let reg = TemplateRegistry::from_toml_str(plan_template_toml(), &HIERARCHY).unwrap();
        assert!(reg.get("plan").is_some());
        assert!(reg.get("nope").is_none());
    }

    // REGA-02: load-time behavior.

    #[test]
    fn test_load_missing_file_is_empty_registry() {
        let dir = TempDir::new().unwrap();
        let reg = TemplateRegistry::load(
            &crate::test_utils::repository_layout_for_data_root(dir.path()),
            &HIERARCHY,
        )
        .unwrap();
        assert!(reg.templates.is_empty());
    }

    #[test]
    fn test_load_valid_file_from_disk() {
        let dir = TempDir::new().unwrap();
        let layout = crate::test_utils::repository_layout_for_data_root(dir.path());
        std::fs::write(
            layout.resolve(&VirtualPath::TEMPLATES).unwrap(),
            plan_template_toml(),
        )
        .unwrap();
        let reg = TemplateRegistry::load(&layout, &HIERARCHY).unwrap();
        assert_eq!(reg.templates.len(), 1);
        assert_eq!(reg.templates[0].name, "plan");
    }

    #[test]
    fn test_malformed_toml_errors() {
        let dir = TempDir::new().unwrap();
        let layout = crate::test_utils::repository_layout_for_data_root(dir.path());
        std::fs::write(
            layout.resolve(&VirtualPath::TEMPLATES).unwrap(),
            "[[template",
        )
        .unwrap();
        let err = TemplateRegistry::load(&layout, &HIERARCHY).unwrap_err();
        assert!(matches!(err, TemplateConfigError::Toml(_)));
    }

    #[test]
    fn test_duplicate_template_name_rejected() {
        let toml = r#"
[[template]]
name = "plan"
applies_to = ["epic"]
[[template]]
name = "plan"
applies_to = ["epic"]
"#;
        let err = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap_err();
        match err {
            TemplateConfigError::DuplicateTemplate { name } => assert_eq!(name, "plan"),
            other => panic!("expected DuplicateTemplate, got {other:?}"),
        }
    }

    #[test]
    fn test_duplicate_role_rejected() {
        let toml = r#"
[[template]]
name = "t"
applies_to = ["epic"]
[[template.nodes]]
role = "a"
type = "planning"
[[template.nodes]]
role = "a"
type = "breakdown"
"#;
        let err = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap_err();
        assert!(matches!(err, TemplateConfigError::DuplicateRole { .. }));
    }

    #[test]
    fn test_duplicate_anchor_rejected() {
        let toml = r#"
[[template]]
name = "t"
applies_to = ["epic"]
[[template.anchors]]
name = "container"
[[template.anchors]]
name = "container"
"#;
        let err = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap_err();
        assert!(matches!(err, TemplateConfigError::DuplicateAnchor { .. }));
    }

    #[test]
    fn test_depends_on_unknown_role_rejected() {
        let toml = r#"
[[template]]
name = "t"
applies_to = ["epic"]
[[template.nodes]]
role = "breakdown"
type = "breakdown"
depends_on = ["ghost"]
"#;
        let err = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap_err();
        match err {
            TemplateConfigError::UnknownRole { role, .. } => assert_eq!(role, "ghost"),
            other => panic!("expected UnknownRole, got {other:?}"),
        }
    }

    #[test]
    fn test_anchor_edge_unknown_anchor_rejected() {
        let toml = r#"
[[template]]
name = "t"
applies_to = ["epic"]
[[template.nodes]]
role = "breakdown"
type = "breakdown"
[[template.anchor_edges]]
from = "ghost"
to = "breakdown"
"#;
        let err = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap_err();
        assert!(matches!(err, TemplateConfigError::UnknownAnchor { .. }));
    }

    #[test]
    fn test_anchor_edge_unknown_node_role_rejected() {
        let toml = r#"
[[template]]
name = "t"
applies_to = ["epic"]
[[template.anchors]]
name = "container"
[[template.anchor_edges]]
from = "container"
to = "ghost"
"#;
        let err = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap_err();
        match err {
            TemplateConfigError::UnknownRole { role, .. } => assert_eq!(role, "ghost"),
            other => panic!("expected UnknownRole, got {other:?}"),
        }
    }

    #[test]
    fn test_transform_unknown_role_rejected() {
        let toml = r#"
[[template]]
name = "t"
applies_to = ["epic"]
[[template.nodes]]
role = "planning"
type = "planning"
[[template.transforms]]
kind = "move-upstream-to-role"
role = "ghost"
"#;
        let err = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap_err();
        match err {
            TemplateConfigError::UnknownRole { role, .. } => assert_eq!(role, "ghost"),
            other => panic!("expected UnknownRole, got {other:?}"),
        }
    }

    #[test]
    fn test_unknown_transform_kind_rejected() {
        // An unknown kind fails load, naming both the template and the kind.
        let toml = r#"
[[template]]
name = "weird"
applies_to = ["epic"]
[[template.nodes]]
role = "planning"
type = "planning"
[[template.transforms]]
kind = "teleport"
role = "planning"
"#;
        let err = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap_err();
        assert!(matches!(
            err,
            TemplateConfigError::InvalidTemplate { ref template, .. } if template == "weird"
        ));
        let msg = err.to_string();
        assert!(msg.contains("weird"), "{msg}");
        assert!(msg.contains("teleport"), "{msg}");
    }

    #[test]
    fn test_supported_transform_kind_accepted() {
        let reg = TemplateRegistry::from_toml_str(plan_template_toml(), &HIERARCHY).unwrap();
        assert_eq!(
            reg.templates[0].transforms[0].kind,
            TransformKind::MoveUpstreamToRole
        );
    }

    #[test]
    fn test_from_toml_str_defers_an_undeclared_node_type_to_apply() {
        let toml = r#"
[[template]]
name = "t"
applies_to = ["epic"]
[[template.nodes]]
role = "planning"
type = "bogus"
"#;
        // A node type is a prerequisite of applying the template, reported with
        // the template's other unmet prerequisites rather than failing the load.
        let registry = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap();
        assert!(registry
            .get("t")
            .unwrap()
            .requirements()
            .contains(&TemplateRequirement::Type("bogus".to_string())));
    }

    #[test]
    fn test_graph_template_requirements_derive_each_kind_from_the_declaring_field() {
        let toml = r#"
[[template]]
name = "t"
applies_to = ["epic"]
  [[template.anchors]]
  name  = "container"
  gates = ["anchor-gate"]
  [[template.nodes]]
  role     = "first"
  type     = "first-type"
  gates    = ["node-gate"]
  doc      = "{container.dir}/x.md"
  doc_area = "some/area"
  labels   = ["static-ns:{container.short_id}", "{dynamic}:value", "no-separator"]
  [[template.nodes]]
  role   = "second"
  type   = "second-type"
  labels = ["static-ns:other"]
"#;
        let template = TemplateRegistry::from_toml_str(toml, &[] as &[&str])
            .unwrap()
            .get("t")
            .unwrap()
            .clone();

        let gate = |key: &str| TemplateRequirement::Gate(key.parse().unwrap());
        let expected = BTreeSet::from([
            TemplateRequirement::Type("first-type".to_string()),
            TemplateRequirement::Type("second-type".to_string()),
            // Every node writes its type as a label in the type namespace; a
            // label whose namespace is interpolated or absent names none.
            TemplateRequirement::LabelNamespace(crate::labels::TYPE_NAMESPACE.to_string()),
            TemplateRequirement::LabelNamespace("static-ns".to_string()),
            gate("node-gate"),
            gate("anchor-gate"),
            TemplateRequirement::DocumentationArea("some/area".to_string()),
        ]);
        assert_eq!(template.requirements(), expected);
    }

    #[test]
    fn test_graph_template_requirements_of_a_template_declaring_nothing_are_empty() {
        let toml = r#"
[[template]]
name = "t"
applies_to = ["epic"]
  [[template.anchors]]
  name = "container"
"#;
        let registry = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap();
        assert!(registry.get("t").unwrap().requirements().is_empty());
    }

    #[test]
    fn test_empty_applies_to_rejected() {
        let toml = r#"
[[template]]
name = "t"
applies_to = []
"#;
        let err = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap_err();
        assert!(matches!(err, TemplateConfigError::EmptyAppliesTo { .. }));
    }

    #[test]
    fn test_unknown_applies_to_type_rejected() {
        // Nodes are valid, but `applies_to` names a type absent from the
        // hierarchy (a typo) — config load must fail rather than silently
        // register an invalid container type.
        let toml = r#"
[[template]]
name = "t"
applies_to = ["nonexistent"]
[[template.nodes]]
role = "planning"
type = "planning"
"#;
        let err = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap_err();
        match err {
            TemplateConfigError::UnknownAppliesToType { type_name, .. } => {
                assert_eq!(type_name, "nonexistent")
            }
            other => panic!("expected UnknownAppliesToType, got {other:?}"),
        }
    }

    #[test]
    fn test_empty_hierarchy_skips_applies_to_check() {
        // With no hierarchy configured, `applies_to` types are not checked.
        let toml = r#"
[[template]]
name = "t"
applies_to = ["whatever"]
"#;
        let empty: [&str; 0] = [];
        let reg = TemplateRegistry::from_toml_str(toml, &empty).unwrap();
        assert_eq!(reg.templates[0].applies_to, vec!["whatever".to_string()]);
    }

    #[test]
    fn test_cyclic_depends_on_rejected() {
        let toml = r#"
[[template]]
name = "t"
applies_to = ["epic"]
[[template.nodes]]
role = "a"
type = "planning"
depends_on = ["b"]
[[template.nodes]]
role = "b"
type = "breakdown"
depends_on = ["a"]
"#;
        let err = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap_err();
        assert!(matches!(err, TemplateConfigError::CyclicDependsOn { .. }));
    }

    #[test]
    fn test_self_dependency_is_cycle() {
        let toml = r#"
[[template]]
name = "t"
applies_to = ["epic"]
[[template.nodes]]
role = "a"
type = "planning"
depends_on = ["a"]
"#;
        let err = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap_err();
        assert!(matches!(err, TemplateConfigError::CyclicDependsOn { .. }));
    }

    // BIND-01: role and anchor bindings parse, and an absent table defaults to
    // today's names, so an existing repository behaves unchanged.

    #[test]
    fn test_absent_binding_tables_default_to_shipped_names() {
        let reg = TemplateRegistry::from_toml_str(plan_template_toml(), &HIERARCHY).unwrap();
        assert_eq!(reg.roles.planning_role(), DEFAULT_PLANNING_ROLE);
        assert_eq!(reg.roles.breakdown_role(), DEFAULT_BREAKDOWN_ROLE);
        assert_eq!(reg.anchors.container_anchor(), DEFAULT_CONTAINER_ANCHOR);
        // The shipped defaults are exactly the names the bracket has always used.
        assert_eq!(DEFAULT_PLANNING_ROLE, "planning");
        assert_eq!(DEFAULT_BREAKDOWN_ROLE, "breakdown");
        assert_eq!(DEFAULT_CONTAINER_ANCHOR, "container");
    }

    #[test]
    fn test_declared_bindings_rename_roles_and_anchor() {
        let toml = r#"
[roles]
planning  = "spec"
breakdown = "split"

[anchors]
container = "target"

[[template]]
name       = "plan"
applies_to = ["epic"]

  [[template.anchors]]
  name = "target"

  [[template.nodes]]
  role = "spec"
  type = "planning"
  doc  = "dev/{container.id}.md"

  [[template.nodes]]
  role       = "split"
  type       = "breakdown"
  depends_on = ["spec"]

  [[template.anchor_edges]]
  from = "target"
  to   = "split"
"#;
        let reg = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap();
        assert_eq!(reg.roles.planning_role(), "spec");
        assert_eq!(reg.roles.breakdown_role(), "split");
        assert_eq!(reg.anchors.container_anchor(), "target");

        // The bracket accessors resolve through the bindings, not literals.
        let plan = reg.get("plan").unwrap();
        assert_eq!(plan.planning_node(&reg.roles).unwrap().role, "spec");
        assert_eq!(plan.breakdown_node(&reg.roles).unwrap().role, "split");
        assert_eq!(plan.planning_type(&reg.roles), Some("planning"));
        assert_eq!(plan.breakdown_type(&reg.roles), Some("breakdown"));
        assert_eq!(
            plan.plan_doc_location(&reg.roles),
            Some("dev/{container.id}.md")
        );

        // With the DEFAULT bindings, the same template exposes no bracket nodes.
        let defaults = RoleBindings::default();
        assert!(plan.planning_node(&defaults).is_none());
        assert!(plan.breakdown_node(&defaults).is_none());
    }

    #[test]
    fn test_partial_role_binding_defaults_the_other_role() {
        let toml = r#"
[roles]
breakdown = "split"

[[template]]
name       = "plan"
applies_to = ["epic"]
"#;
        let reg = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap();
        assert_eq!(reg.roles.planning_role(), DEFAULT_PLANNING_ROLE);
        assert_eq!(reg.roles.breakdown_role(), "split");
    }

    #[test]
    fn test_bindings_roundtrip_through_json() {
        let toml = r#"
[roles]
planning = "spec"

[anchors]
container = "target"

[[template]]
name       = "plan"
applies_to = ["epic"]
"#;
        let reg = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap();
        let back: TemplateRegistry = serde_json::from_str(&serde_json::to_string(&reg).unwrap())
            .expect("registry with bindings round-trips");
        assert_eq!(reg, back);
    }

    // REGB-02: the registry yields, for a container type, the applicable template
    // and its planning/breakdown node types, doc location, and breakable types.

    #[test]
    fn test_breakable_types_is_union_of_applies_to() {
        let toml = r#"
[[template]]
name = "plan"
applies_to = ["epic", "milestone"]
[[template]]
name = "other"
applies_to = ["epic"]
"#;
        // `milestone` must be in the hierarchy or the applies_to check rejects it.
        let reg =
            TemplateRegistry::from_toml_str(toml, &["epic", "milestone", "planning", "breakdown"])
                .unwrap();
        assert_eq!(
            reg.breakable_types(),
            vec!["epic".to_string(), "milestone".to_string()]
        );
    }

    #[test]
    fn test_template_for_container_resolves_epic() {
        let reg = TemplateRegistry::from_toml_str(plan_template_toml(), &HIERARCHY).unwrap();
        let t = reg.template_for_container("epic").unwrap();
        assert_eq!(t.name, "plan");
        assert!(reg.template_for_container("task").is_none());
    }

    #[test]
    fn test_accessors_derive_bracket_vocabulary_for_epic() {
        let reg = TemplateRegistry::from_toml_str(plan_template_toml(), &HIERARCHY).unwrap();
        let t = reg.template_for_container("epic").unwrap();
        assert_eq!(t.planning_type(&reg.roles), Some("planning"));
        assert_eq!(t.breakdown_type(&reg.roles), Some("breakdown"));
        assert_eq!(
            t.plan_doc_location(&reg.roles),
            Some("dev/active/{container.id}-plan.md")
        );
        assert_eq!(reg.breakable_types(), vec!["epic".to_string()]);
    }

    #[test]
    fn test_node_lookup_by_role() {
        let reg = TemplateRegistry::from_toml_str(plan_template_toml(), &HIERARCHY).unwrap();
        let t = reg.get("plan").unwrap();
        assert_eq!(
            t.node(reg.roles.planning_role()).unwrap().type_name,
            "planning"
        );
        assert_eq!(
            t.node(reg.roles.breakdown_role()).unwrap().type_name,
            "breakdown"
        );
        assert!(t.node("nonexistent").is_none());
    }

    #[test]
    fn test_empty_registry_has_no_breakable_types() {
        let reg = TemplateRegistry::empty();
        assert!(reg.breakable_types().is_empty());
        assert!(reg.template_for_container("epic").is_none());
    }

    #[test]
    fn test_acyclic_chain_accepted() {
        // a -> b -> c is a valid DAG.
        let toml = r#"
[[template]]
name = "t"
applies_to = ["epic"]
[[template.nodes]]
role = "a"
type = "planning"
depends_on = ["b"]
[[template.nodes]]
role = "b"
type = "breakdown"
depends_on = ["c"]
[[template.nodes]]
role = "c"
type = "epic"
"#;
        let reg = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap();
        assert_eq!(reg.templates[0].nodes.len(), 3);
    }
}
