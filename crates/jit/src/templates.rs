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
//! - each node `type` exists in the configured `[type_hierarchy].types`;
//! - each transform `kind` is a supported [`TransformKind`];
//! - the internal `depends_on` edges form a DAG (no cycle).
//!
//! Following the [`RuleSet::load`](crate::validation::rules::RuleSet::load)
//! precedent: an absent `templates.toml` loads as an empty registry, and an
//! invalid file fails at load with a descriptive [`TemplateConfigError`].
//!
//! **Deferred:** gate-preset existence is NOT validated here. A node's `gates`
//! are preset names resolved by the gate-preset manager
//! ([`crate::gate_presets`]); that registry is not available at config-load time
//! in this layer, so preset existence is checked by the apply engine in a later
//! task (W2). The reference-integrity checks above are all resolvable from the
//! template file plus the type hierarchy alone.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors that can occur while loading and validating `.jit/templates.toml`.
#[derive(Debug, Error)]
pub enum TemplateConfigError {
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

    /// A node declares a `type` absent from `[type_hierarchy].types`.
    #[error("template '{template}': node '{role}' has type '{type_name}', which is not declared in [type_hierarchy].types")]
    UnknownType {
        /// Name of the offending template.
        template: String,
        /// The role of the offending node.
        role: String,
        /// The undeclared type name.
        type_name: String,
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

    /// A transform declares a `kind` outside the supported set
    /// ([`TransformKind`]).
    #[error(
        "template '{template}': unsupported transform kind '{kind}'; supported kinds: {supported}"
    )]
    UnknownTransformKind {
        /// Name of the offending template.
        template: String,
        /// The unsupported kind string.
        kind: String,
        /// Comma-separated list of the kinds the engine supports.
        supported: String,
    },

    /// `applies_to` is empty: a template must name at least one container type.
    #[error("template '{template}': applies_to must list at least one container type")]
    EmptyAppliesTo {
        /// Name of the offending template.
        template: String,
    },
}

/// A supported graph-transform kind, parsed from a [`Transform::kind`] string.
///
/// The registry loader rejects any other kind ([`TemplateConfigError::UnknownTransformKind`]),
/// so a template that reaches `jit apply` declares only kinds the engine can
/// dispatch. Dispatch is by variant, keeping the transform set extensible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformKind {
    /// Move the container's pre-apply upstream deps onto a named role's node.
    MoveUpstreamToRole,
}

impl TransformKind {
    /// The `kind` string of [`TransformKind::MoveUpstreamToRole`].
    pub const MOVE_UPSTREAM_TO_ROLE: &'static str = "move-upstream-to-role";

    /// Every supported `kind` string, for error messages and documentation.
    pub const SUPPORTED: &'static [&'static str] = &[Self::MOVE_UPSTREAM_TO_ROLE];

    /// Parse a transform `kind` string, or `None` when the kind is unsupported.
    pub fn from_kind(kind: &str) -> Option<Self> {
        match kind {
            Self::MOVE_UPSTREAM_TO_ROLE => Some(Self::MoveUpstreamToRole),
            _ => None,
        }
    }
}

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
    /// Gate-preset names attached to the BOUND anchor issue at apply time,
    /// declared exactly like a node's [`gates`](TemplateNode::gates). Preset
    /// existence is validated by the apply engine (not at config load), and the
    /// presets are attached through the same shared `apply_gate_preset` path.
    #[serde(default)]
    pub gates: Vec<String>,
}

/// A node the template creates when applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateNode {
    /// Role identifying this node within the template (e.g. `"planning"`),
    /// unique per template. Roles are template metadata, not stored on the
    /// created issue.
    pub role: String,
    /// Issue type the created node carries (e.g. `"planning"`); must exist in
    /// `[type_hierarchy].types`.
    #[serde(rename = "type")]
    pub type_name: String,
    /// Gate-preset names attached to the node (resolved by the gate-preset
    /// manager at apply time, not validated here).
    #[serde(default)]
    pub gates: Vec<String>,
    /// Plan-doc location template for the node, with `{...}` interpolation
    /// tokens resolved at apply time (e.g. `"dev/active/{container.id}-plan.md"`).
    #[serde(default)]
    pub doc: Option<String>,
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
/// `kind` names a [`TransformKind`]; the loader rejects any other value. The
/// shipped kind is `move-upstream-to-role`, which moves the container's pre-apply
/// upstream dependencies onto the node of the named role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transform {
    /// Transform kind (e.g. `"move-upstream-to-role"`).
    pub kind: String,
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
    /// is the configured `[type_hierarchy].types` key set, against which node
    /// `type`s are checked; pass an empty slice to skip the type check (when no
    /// hierarchy is configured).
    pub fn load<S: AsRef<str>>(
        jit_root: &Path,
        hierarchy_types: &[S],
    ) -> Result<Self, TemplateConfigError> {
        let path = jit_root.join("templates.toml");
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
    /// pass an empty slice to skip the node-`type` check.
    pub fn from_toml_str<S: AsRef<str>>(
        content: &str,
        hierarchy_types: &[S],
    ) -> Result<Self, TemplateConfigError> {
        let registry: TemplateRegistry = toml::from_str(content)?;

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
    /// `"dev/active/{container.id}-plan.md"`), with `{...}` tokens resolved at
    /// apply time.
    pub fn plan_doc_location(&self, roles: &RoleBindings) -> Option<&str> {
        self.planning_node(roles).and_then(|n| n.doc.as_deref())
    }

    /// Validate one template's internal structure: unique roles/anchors, every
    /// reference resolves, node types are known, and `depends_on` is acyclic.
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

        // Node types exist in the hierarchy; `depends_on` references a declared role.
        for node in &self.nodes {
            if !known_type(&node.type_name) {
                return Err(TemplateConfigError::UnknownType {
                    template: self.name.clone(),
                    role: node.role.clone(),
                    type_name: node.type_name.clone(),
                });
            }
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

        // transforms declare a supported kind and reference a declared node role.
        // The kind check belongs here, with the other structural checks: an
        // unsupported kind is a static config error, so `jit` fails at load and
        // the apply engine only ever dispatches kinds it can execute.
        for transform in &self.transforms {
            if TransformKind::from_kind(&transform.kind).is_none() {
                return Err(TemplateConfigError::UnknownTransformKind {
                    template: self.name.clone(),
                    kind: transform.kind.clone(),
                    supported: TransformKind::SUPPORTED.join(", "),
                });
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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
        assert_eq!(planning.gates, vec!["plan-review"]);
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
        );
        assert_eq!(breakdown.labels, vec!["brackets:{container.short_id}"]);
        assert_eq!(breakdown.depends_on, vec!["planning"]);

        assert_eq!(t.anchor_edges.len(), 1);
        assert_eq!(t.anchor_edges[0].from, "container");
        assert_eq!(t.anchor_edges[0].to, "breakdown");

        assert_eq!(t.transforms.len(), 1);
        assert_eq!(t.transforms[0].kind, "move-upstream-to-role");
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
        );
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
        let reg = TemplateRegistry::load(dir.path(), &HIERARCHY).unwrap();
        assert!(reg.templates.is_empty());
    }

    #[test]
    fn test_load_valid_file_from_disk() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("templates.toml"), plan_template_toml()).unwrap();
        let reg = TemplateRegistry::load(dir.path(), &HIERARCHY).unwrap();
        assert_eq!(reg.templates.len(), 1);
        assert_eq!(reg.templates[0].name, "plan");
    }

    #[test]
    fn test_malformed_toml_errors() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("templates.toml"), "[[template").unwrap();
        let err = TemplateRegistry::load(dir.path(), &HIERARCHY).unwrap_err();
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
        // A transform kind the engine cannot dispatch is a static config error:
        // load fails, naming both the template and the offending kind, so the
        // apply engine never receives it.
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
        match &err {
            TemplateConfigError::UnknownTransformKind {
                template,
                kind,
                supported,
            } => {
                assert_eq!(template, "weird");
                assert_eq!(kind, "teleport");
                assert!(supported.contains("move-upstream-to-role"), "{supported}");
            }
            other => panic!("expected UnknownTransformKind, got {other:?}"),
        }
        let msg = err.to_string();
        assert!(msg.contains("weird"), "{msg}");
        assert!(msg.contains("teleport"), "{msg}");
    }

    #[test]
    fn test_supported_transform_kind_accepted() {
        let reg = TemplateRegistry::from_toml_str(plan_template_toml(), &HIERARCHY).unwrap();
        assert_eq!(
            TransformKind::from_kind(&reg.templates[0].transforms[0].kind),
            Some(TransformKind::MoveUpstreamToRole)
        );
    }

    #[test]
    fn test_unknown_node_type_rejected() {
        let toml = r#"
[[template]]
name = "t"
applies_to = ["epic"]
[[template.nodes]]
role = "planning"
type = "bogus"
"#;
        let err = TemplateRegistry::from_toml_str(toml, &HIERARCHY).unwrap_err();
        match err {
            TemplateConfigError::UnknownType { type_name, .. } => assert_eq!(type_name, "bogus"),
            other => panic!("expected UnknownType, got {other:?}"),
        }
    }

    #[test]
    fn test_empty_hierarchy_skips_type_check() {
        let toml = r#"
[[template]]
name = "t"
applies_to = ["epic"]
[[template.nodes]]
role = "planning"
type = "anything"
"#;
        let empty: [&str; 0] = [];
        let reg = TemplateRegistry::from_toml_str(toml, &empty).unwrap();
        assert_eq!(reg.templates[0].nodes[0].type_name, "anything");
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

    // REGB-01: this repo's authored `.jit/templates.toml` round-trips through the
    // loader. The path is resolved relative to the crate manifest so the test does
    // not depend on the process working directory or on production issue state.

    #[test]
    fn test_repo_plan_template_parses() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("crate is nested two levels under the repo root");
        let jit_dir = repo_root.join(".jit");
        let reg = TemplateRegistry::load(&jit_dir, &HIERARCHY).unwrap();
        let plan = reg
            .get("plan")
            .expect("repo .jit/templates.toml declares a `plan` template");
        assert_eq!(plan.applies_to, vec!["epic"]);
        assert_eq!(plan.planning_type(&reg.roles), Some("planning"));
        assert_eq!(plan.breakdown_type(&reg.roles), Some("breakdown"));
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
