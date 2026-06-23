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
//! [`AnchorSlot`], [`AnchorEdge`], [`Transform`]) and the LOADER
//! ([`TemplateRegistry::load`]) with load-time **structural validation**:
//!
//! - node `role`s are unique within a template;
//! - every `depends_on`, `anchor_edges`, and `transforms` reference resolves to
//!   a declared node role / anchor name;
//! - each node `type` exists in the configured `[type_hierarchy].types`;
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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors that can occur while loading and validating `.jit/templates.toml`.
///
/// # Examples
///
/// ```
/// use jit::templates::{TemplateConfigError, TemplateRegistry};
/// use std::path::Path;
///
/// // Two nodes sharing a role is a config error.
/// let toml = r#"
/// [[template]]
/// name = "dup"
/// applies_to = ["epic"]
/// [[template.nodes]]
/// role = "a"
/// type = "planning"
/// [[template.nodes]]
/// role = "a"
/// type = "planning"
/// "#;
/// let err = TemplateRegistry::from_toml_str(toml, &["planning"]).unwrap_err();
/// assert!(matches!(err, TemplateConfigError::DuplicateRole { .. }));
/// ```
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

    /// `applies_to` is empty: a template must name at least one container type.
    #[error("template '{template}': applies_to must list at least one container type")]
    EmptyAppliesTo {
        /// Name of the offending template.
        template: String,
    },
}

/// A loaded, validated set of graph templates from `.jit/templates.toml`.
///
/// Built by [`TemplateRegistry::load`] (file → registry) or
/// [`TemplateRegistry::from_toml_str`] (string → registry). An absent file
/// yields an empty registry.
///
/// # Examples
///
/// ```
/// use jit::templates::TemplateRegistry;
///
/// let toml = r#"
/// [[template]]
/// name = "plan"
/// applies_to = ["epic"]
/// "#;
/// let reg = TemplateRegistry::from_toml_str(toml, &["epic"]).unwrap();
/// assert_eq!(reg.templates.len(), 1);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateRegistry {
    /// The declared templates, in authored order.
    #[serde(default, rename = "template")]
    pub templates: Vec<GraphTemplate>,
}

/// A named, parameterized subgraph applied to a container by `jit apply`.
///
/// # Examples
///
/// ```
/// use jit::templates::TemplateRegistry;
///
/// let toml = r#"
/// [[template]]
/// name = "plan"
/// description = "Plan-before-fan-out bracket."
/// applies_to = ["epic"]
/// "#;
/// let reg = TemplateRegistry::from_toml_str(toml, &["epic"]).unwrap();
/// let template = reg.get("plan").unwrap();
/// assert_eq!(template.applies_to, vec!["epic".to_string()]);
/// assert_eq!(template.description.as_deref(), Some("Plan-before-fan-out bracket."));
/// ```
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
///
/// # Examples
///
/// ```
/// use jit::templates::AnchorSlot;
///
/// let slot = AnchorSlot { name: "container".to_string() };
/// assert_eq!(slot.name, "container");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorSlot {
    /// The anchor's name, referenced by `anchor_edges.from`.
    pub name: String,
}

/// A node the template creates when applied.
///
/// # Examples
///
/// ```
/// use jit::templates::TemplateRegistry;
///
/// let toml = r#"
/// [[template]]
/// name = "plan"
/// applies_to = ["epic"]
/// [[template.nodes]]
/// role = "breakdown"
/// type = "breakdown"
/// depends_on = ["planning"]
/// [[template.nodes]]
/// role = "planning"
/// type = "planning"
/// "#;
/// let reg = TemplateRegistry::from_toml_str(toml, &["planning", "breakdown"]).unwrap();
/// let node = &reg.get("plan").unwrap().nodes[0];
/// assert_eq!(node.role, "breakdown");
/// assert_eq!(node.type_name, "breakdown");
/// assert_eq!(node.depends_on, vec!["planning".to_string()]);
/// ```
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
///
/// # Examples
///
/// ```
/// use jit::templates::AnchorEdge;
///
/// let edge = AnchorEdge { from: "container".to_string(), to: "breakdown".to_string() };
/// assert_eq!(edge.from, "container");
/// assert_eq!(edge.to, "breakdown");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorEdge {
    /// Anchor name the edge originates from (the dependent side).
    pub from: String,
    /// Node role the edge points to (the dependency side).
    pub to: String,
}

/// A graph transform applied after nodes are created and edges wired.
///
/// The only `kind` shipped this epic is `move-upstream-to-role`, which moves the
/// container's pre-apply upstream dependencies onto the node of the named role.
/// Dispatch is by `kind` string, kept extensible for future transforms.
///
/// # Examples
///
/// ```
/// use jit::templates::Transform;
///
/// let transform = Transform {
///     kind: "move-upstream-to-role".to_string(),
///     role: "planning".to_string(),
/// };
/// assert_eq!(transform.kind, "move-upstream-to-role");
/// assert_eq!(transform.role, "planning");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transform {
    /// Transform kind (e.g. `"move-upstream-to-role"`).
    pub kind: String,
    /// Target node role the transform acts on.
    pub role: String,
}

impl TemplateRegistry {
    /// An empty registry (used when no `templates.toml` exists).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::templates::TemplateRegistry;
    ///
    /// let reg = TemplateRegistry::empty();
    /// assert!(reg.templates.is_empty());
    /// ```
    pub fn empty() -> Self {
        Self::default()
    }

    /// Look up a template by name.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::templates::TemplateRegistry;
    ///
    /// let toml = r#"
    /// [[template]]
    /// name = "plan"
    /// applies_to = ["epic"]
    /// "#;
    /// let reg = TemplateRegistry::from_toml_str(toml, &["epic"]).unwrap();
    /// assert!(reg.get("plan").is_some());
    /// assert!(reg.get("missing").is_none());
    /// ```
    pub fn get(&self, name: &str) -> Option<&GraphTemplate> {
        self.templates.iter().find(|t| t.name == name)
    }

    /// Load and validate `.jit/templates.toml` relative to the given `.jit` root.
    ///
    /// Returns an empty registry when the file does not exist. `hierarchy_types`
    /// is the configured `[type_hierarchy].types` key set, against which node
    /// `type`s are checked; pass an empty slice to skip the type check (when no
    /// hierarchy is configured).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::templates::TemplateRegistry;
    ///
    /// // A directory with no `templates.toml` loads as an empty registry.
    /// let dir = tempfile::tempdir().unwrap();
    /// let reg = TemplateRegistry::load(dir.path(), &["epic"]).unwrap();
    /// assert!(reg.templates.is_empty());
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::templates::TemplateRegistry;
    ///
    /// let toml = r#"
    /// [[template]]
    /// name = "plan"
    /// applies_to = ["epic"]
    /// [[template.nodes]]
    /// role = "planning"
    /// type = "planning"
    /// "#;
    /// let reg = TemplateRegistry::from_toml_str(toml, &["planning"]).unwrap();
    /// assert_eq!(reg.templates[0].name, "plan");
    /// ```
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

    /// Detect a cycle in the internal `depends_on` edges via DFS with a
    /// recursion stack. Returns the first role found on a back-edge.
    fn check_acyclic(&self) -> Result<(), TemplateConfigError> {
        let adjacency: HashMap<&str, &[String]> = self
            .nodes
            .iter()
            .map(|n| (n.role.as_str(), n.depends_on.as_slice()))
            .collect();

        // 0 = unvisited, 1 = on stack, 2 = done.
        let mut state: HashMap<&str, u8> = HashMap::new();

        for node in &self.nodes {
            if let Some(role) = visit_cycle(node.role.as_str(), &adjacency, &mut state) {
                return Err(TemplateConfigError::CyclicDependsOn {
                    template: self.name.clone(),
                    role: role.to_string(),
                });
            }
        }
        Ok(())
    }
}

/// DFS helper for [`GraphTemplate::check_acyclic`]. Returns the role on a
/// detected back-edge, or `None` if the subtree rooted at `role` is acyclic.
fn visit_cycle<'a>(
    role: &'a str,
    adjacency: &HashMap<&'a str, &'a [String]>,
    state: &mut HashMap<&'a str, u8>,
) -> Option<&'a str> {
    match state.get(role) {
        Some(2) => return None,       // already fully explored
        Some(1) => return Some(role), // back-edge: cycle
        _ => {}
    }
    state.insert(role, 1);
    if let Some(deps) = adjacency.get(role) {
        for dep in deps.iter() {
            // Resolve to the borrowed key so lifetimes line up; a dep referencing
            // an undeclared role is caught by reference validation before this runs.
            if let Some((&key, _)) = adjacency.get_key_value(dep.as_str()) {
                if let Some(found) = visit_cycle(key, adjacency, state) {
                    return Some(found);
                }
            }
        }
    }
    state.insert(role, 2);
    None
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
