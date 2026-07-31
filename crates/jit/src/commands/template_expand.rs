//! Pure expansion of a graph template into a [`TemplateDelta`].
//!
//! [`expand_template`] takes a template, a CONTAINER SNAPSHOT, the resolved
//! anchor bindings, and each bound anchor's pre-apply dependency snapshot, and
//! returns the complete set of writes an apply would perform: the issues to
//! create, the edges to add, the edges to remove, and the gates to attach to
//! bound anchors. It performs no storage access, so the whole shape of an apply
//! is computable, testable, and checkable (see [`validate_delta_acyclic`])
//! before the first mutation.
//!
//! The executor ([`apply_template_with`](crate::commands::CommandExecutor::apply_template_with))
//! is the only layer that touches storage: it validates the delta, then commits
//! it under one repository lock.
//!
//! # Domain-agnostic
//!
//! Node types, gates, doc locations, descriptions, and labels all come from the
//! template. The only interpretation this module applies is the fixed `{token}`
//! substitution of [`InterpolationContext`], whose `{container.dir}` value is
//! the domain resolver's own answer
//! ([`resolve_artifact_directory`](crate::domain::artifact_directory::resolve_artifact_directory))
//! for the area the node declares.

use std::collections::BTreeMap;

use anyhow::{anyhow, Result};

use crate::config::DocumentationConfig;
use crate::domain::artifact_directory::{resolve_artifact_directory, ArtifactDirectoryError};
use crate::domain::type_taxonomy::HierarchyConfig;
use crate::domain::{Issue, Priority};
use crate::labels as label_utils;
use crate::templates::{GraphTemplate, TemplateNode, TransformKind};

/// One endpoint of a [`DeltaEdge`]: a node the delta creates (named by its
/// template role), or an issue that already exists (named by its full id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaEndpoint {
    /// A node the delta creates, identified by its template role.
    CreatedRole(String),
    /// An issue already in the store, identified by its full id.
    ExistingIssue(String),
}

/// A dependency edge in a [`TemplateDelta`]: `dependent` depends on `dependency`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaEdge {
    /// The dependent side (`C` in `C → B`).
    pub dependent: DeltaEndpoint,
    /// The dependency side (`B` in `C → B`).
    pub dependency: DeltaEndpoint,
}

/// An issue the delta creates: the FINAL shape a template node instantiates to,
/// with every `{token}` already resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedNode {
    /// The template role this issue is created for.
    pub role: String,
    /// Title of the created issue.
    pub title: String,
    /// Interpolated, guaranteed non-empty description.
    pub description: String,
    /// Final label set (inherited membership labels + own `type:` + interpolated).
    pub labels: Vec<String>,
    /// Gate names to attach (each a gate preset OR a registry gate key).
    pub gates: Vec<String>,
    /// Priority inherited from the container.
    pub priority: Priority,
}

/// The gates a template anchor declares, bound to the anchor's issue id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorGates {
    /// Full id of the issue bound to the anchor.
    pub anchor_issue_id: String,
    /// Gate names to attach to it.
    pub gates: Vec<String>,
}

/// The complete, storage-free description of what applying a template does.
///
/// Produced by [`expand_template`] and committed by the apply executor in this
/// order: `creates`, `add_edges`, `remove_edges`, `anchor_gates`. Adding the
/// transform's new edges before removing the old ones is what keeps transitive
/// reduction from stranding an edge mid-operation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateDelta {
    /// Issues to create, in template-node order.
    pub creates: Vec<PlannedNode>,
    /// Edges to add: the internal `depends_on` edges, then the `anchor_edges`,
    /// then each transform's new edges.
    pub add_edges: Vec<DeltaEdge>,
    /// Edges to remove: each transform's removals.
    pub remove_edges: Vec<DeltaEdge>,
    /// Gates to attach to bound anchor issues.
    pub anchor_gates: Vec<AnchorGates>,
}

/// Expand `template` over a container snapshot into the delta an apply commits.
/// PURE: no storage access, no clock, no I/O.
///
/// `resolved_bindings` maps each anchor name to the full id it is bound to, and
/// `anchor_dependency_snapshots` maps each anchor name to that anchor's
/// dependencies as read BEFORE any mutation. The `move-upstream-to-role`
/// transform reads the container anchor's snapshot, so it can only ever move the
/// container's ORIGINAL upstream deps, never the scaffold edges the same delta
/// adds.
///
/// `documentation` and `hierarchy` are the repository declarations the
/// `{container.dir}` field is resolved against: a node declaring a
/// [`doc_area`](crate::templates::TemplateNode::doc_area) resolves that area
/// through [`resolve_artifact_directory`], so the directory the container owns
/// is composed in one place.
///
/// # Errors
///
/// Returns an error when a node declares a document area the configured
/// registry does not declare (naming the template, the node, and the area), or
/// when the template references an unbound anchor, a role no node declares, or
/// an unsupported transform kind (the registry loader rejects the latter three,
/// so this is the defensive gate for hand-built templates).
///
/// # Examples
///
/// ```
/// use jit::commands::expand_template;
/// use jit::config::DocumentationConfig;
/// use jit::domain::type_taxonomy::HierarchyConfig;
/// use jit::domain::Issue;
/// use jit::templates::TemplateRegistry;
/// use std::collections::BTreeMap;
///
/// let toml = r#"
/// [[template]]
/// name = "plan"
/// applies_to = ["epic"]
/// [[template.anchors]]
/// name = "container"
/// [[template.nodes]]
/// role = "planning"
/// type = "planning"
/// description = "Plan {container.title}."
/// [[template.anchor_edges]]
/// from = "container"
/// to   = "planning"
/// "#;
/// let registry = TemplateRegistry::from_toml_str(toml, &["epic", "planning"]).unwrap();
/// let template = registry.get("plan").unwrap();
///
/// let mut container = Issue::draft("Auth epic".to_string(), String::new());
/// container.labels = vec!["type:epic".to_string()];
/// let bindings = BTreeMap::from([("container".to_string(), container.id.clone())]);
/// let snapshots = BTreeMap::from([("container".to_string(), vec![])]);
///
/// let delta = expand_template(
///     template,
///     &container,
///     &bindings,
///     &snapshots,
///     &DocumentationConfig::default(),
///     &HierarchyConfig::default(),
/// )
/// .unwrap();
/// assert_eq!(delta.creates.len(), 1);
/// assert_eq!(delta.creates[0].description, "Plan Auth epic.");
/// assert_eq!(delta.add_edges.len(), 1);
/// ```
pub fn expand_template(
    template: &GraphTemplate,
    container: &Issue,
    resolved_bindings: &BTreeMap<String, String>,
    anchor_dependency_snapshots: &BTreeMap<String, Vec<String>>,
    documentation: &DocumentationConfig,
    hierarchy: &HierarchyConfig,
) -> Result<TemplateDelta> {
    let inherited = inherited_membership_labels(container);

    let creates: Vec<PlannedNode> = template
        .nodes
        .iter()
        .map(|node| {
            let node_context =
                InterpolationContext::for_node(container, node, documentation, hierarchy)
                    .map_err(|error| located_document_area_error(template, node, error))?;
            Ok(PlannedNode {
                role: node.role.clone(),
                title: node_title(node, container),
                description: node_description(node, &node_context),
                labels: node_labels(node, &inherited, &node_context),
                gates: node.gates.clone(),
                priority: container.priority,
            })
        })
        .collect::<Result<_>>()?;

    let declared_role = |role: &str| template.nodes.iter().any(|n| n.role == role);

    // 1. Internal `depends_on` edges (node → dep-role node).
    let mut add_edges: Vec<DeltaEdge> = Vec::new();
    for node in &template.nodes {
        for dep_role in &node.depends_on {
            if !declared_role(dep_role) {
                return Err(anyhow!(
                    "template '{}' node '{}' depends_on role '{dep_role}', which no node declares",
                    template.name,
                    node.role
                ));
            }
            add_edges.push(DeltaEdge {
                dependent: DeltaEndpoint::CreatedRole(node.role.clone()),
                dependency: DeltaEndpoint::CreatedRole(dep_role.clone()),
            });
        }
    }

    // 2. Anchor edges (bound anchor → created node): "anchor depends on node".
    for edge in &template.anchor_edges {
        let anchor_id = binding(resolved_bindings, template, &edge.from)?;
        if !declared_role(&edge.to) {
            return Err(anyhow!(
                "template '{}' anchor_edge points to role '{}', which no node declares",
                template.name,
                edge.to
            ));
        }
        add_edges.push(DeltaEdge {
            dependent: DeltaEndpoint::ExistingIssue(anchor_id.to_string()),
            dependency: DeltaEndpoint::CreatedRole(edge.to.clone()),
        });
    }

    // 3. Transforms, dispatched by kind. `move-upstream-to-role` moves the
    //    container's PRE-APPLY upstream deps onto the target role's node: the
    //    role node gains each dep, the container loses it.
    let mut remove_edges: Vec<DeltaEdge> = Vec::new();
    let container_anchor = resolved_bindings
        .iter()
        .find(|(_, id)| id.as_str() == container.id)
        .map(|(name, _)| name.as_str());
    for transform in &template.transforms {
        let kind = TransformKind::from_kind(&transform.kind).ok_or_else(|| {
            anyhow!(
                "template '{}' declares an unsupported transform kind '{}'; supported kinds: {}",
                template.name,
                transform.kind,
                TransformKind::SUPPORTED.join(", ")
            )
        })?;
        if !declared_role(&transform.role) {
            return Err(anyhow!(
                "template '{}' transform targets role '{}', which no node declares",
                template.name,
                transform.role
            ));
        }
        match kind {
            TransformKind::MoveUpstreamToRole => {
                let upstream = container_anchor
                    .and_then(|name| anchor_dependency_snapshots.get(name))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                for dep_id in upstream {
                    add_edges.push(DeltaEdge {
                        dependent: DeltaEndpoint::CreatedRole(transform.role.clone()),
                        dependency: DeltaEndpoint::ExistingIssue(dep_id.clone()),
                    });
                    remove_edges.push(DeltaEdge {
                        dependent: DeltaEndpoint::ExistingIssue(container.id.clone()),
                        dependency: DeltaEndpoint::ExistingIssue(dep_id.clone()),
                    });
                }
            }
        }
    }

    // 4. Anchor gates, attached to each bound anchor issue.
    let anchor_gates: Vec<AnchorGates> = template
        .anchors
        .iter()
        .filter(|anchor| !anchor.gates.is_empty())
        .map(|anchor| {
            Ok(AnchorGates {
                anchor_issue_id: binding(resolved_bindings, template, &anchor.name)?.to_string(),
                gates: anchor.gates.clone(),
            })
        })
        .collect::<Result<_>>()?;

    Ok(TemplateDelta {
        creates,
        add_edges,
        remove_edges,
        anchor_gates,
    })
}

/// The canonical artifact directory `node`'s document belongs in, or `None` when
/// the node declares no [`doc_area`](TemplateNode::doc_area).
///
/// The directory is the domain resolver's own answer
/// ([`resolve_artifact_directory`]), so the composition rule — the area, the
/// container's short id, and its membership slug — has one implementation.
///
/// # Errors
///
/// [`ArtifactDirectoryError`] when the declared area is absent from the
/// configured issue-scoped registry.
fn node_artifact_directory(
    node: &TemplateNode,
    container: &Issue,
    documentation: &DocumentationConfig,
    hierarchy: &HierarchyConfig,
) -> std::result::Result<Option<String>, ArtifactDirectoryError> {
    node.doc_area
        .as_deref()
        .map(|area| resolve_artifact_directory(container, area, documentation, hierarchy))
        .transpose()
}

/// The repository-relative document path `node` declares for `container`, or
/// `None` when the node declares no [`doc`](TemplateNode::doc).
///
/// This is the path an apply writes into the node's `{doc}` token: it resolves
/// the same per-node context ([`InterpolationContext::for_node`]), so a
/// derivation that pre-declares or reconciles the document names the file the
/// apply produces instead of substituting the declaration a second time.
///
/// # Errors
///
/// [`ArtifactDirectoryError`] when the node's declared area is absent from the
/// configured issue-scoped registry.
pub(super) fn node_document_path(
    node: &TemplateNode,
    container: &Issue,
    documentation: &DocumentationConfig,
    hierarchy: &HierarchyConfig,
) -> std::result::Result<Option<String>, ArtifactDirectoryError> {
    node.doc
        .is_some()
        .then(|| {
            InterpolationContext::for_node(container, node, documentation, hierarchy)
                .map(|context| context.doc.unwrap_or_default())
        })
        .transpose()
}

/// Locate an artifact-directory failure at the declaration that produced it.
///
/// The reported message names the template, the node, the area, and the registry
/// the area was matched against, so it is actionable on its own, while the typed
/// cause stays in the chain for callers that match on it.
pub(super) fn located_document_area_error(
    template: &GraphTemplate,
    node: &TemplateNode,
    error: ArtifactDirectoryError,
) -> anyhow::Error {
    let located = format!("template '{}' node '{}': {error}", template.name, node.role);
    anyhow::Error::new(error).context(located)
}

/// The full id bound to `anchor_name`, or an error naming the unbound anchor.
fn binding<'a>(
    resolved_bindings: &'a BTreeMap<String, String>,
    template: &GraphTemplate,
    anchor_name: &str,
) -> Result<&'a str> {
    resolved_bindings
        .get(anchor_name)
        .map(String::as_str)
        .ok_or_else(|| {
            anyhow!(
                "template '{}' references anchor '{anchor_name}', which has no resolved binding",
                template.name
            )
        })
}

/// Verify that committing `delta` over a store whose edges are `store_deps`
/// (issue id → its dependency ids) leaves an acyclic graph. PURE: no storage
/// access.
///
/// The executor commits edges one at a time, so a cycle formed by a LATER edge
/// would surface only after earlier writes landed. Simulating the whole
/// prospective graph up front lets the precondition phase be the complete gate:
/// the created nodes enter as placeholder ids, the transform's removals are
/// modeled (so a cycle the removal actually breaks is not falsely flagged), and
/// [`DependencyGraph::validate_dag`](crate::graph::DependencyGraph::validate_dag)
/// checks the result once.
///
/// The simulated edge set is a superset of what the executor persists (eager
/// transitive reduction only DROPS edges), so an acyclic simulation guarantees an
/// acyclic result.
pub fn validate_delta_acyclic(
    delta: &TemplateDelta,
    store_deps: BTreeMap<String, Vec<String>>,
) -> Result<()> {
    let mut deps_by_id = store_deps;

    // Every created node enters the graph under a placeholder id, with no deps
    // until `add_edges` supplies them.
    for planned in &delta.creates {
        deps_by_id.insert(placeholder_id(&planned.role), Vec::new());
    }
    for edge in &delta.add_edges {
        deps_by_id
            .entry(endpoint_id(&edge.dependent))
            .or_default()
            .push(endpoint_id(&edge.dependency));
    }
    for edge in &delta.remove_edges {
        let dependency = endpoint_id(&edge.dependency);
        if let Some(deps) = deps_by_id.get_mut(&endpoint_id(&edge.dependent)) {
            deps.retain(|d| d != &dependency);
        }
    }

    // A dep naming an id outside the node set simply has no outgoing edges during
    // traversal, so every structure that matters for cycle detection is present.
    let nodes: Vec<ProspectiveNode> = deps_by_id
        .into_iter()
        .map(|(id, dependencies)| ProspectiveNode { id, dependencies })
        .collect();
    let refs: Vec<&ProspectiveNode> = nodes.iter().collect();
    // Propagate the typed `GraphError::CycleDetected` (not a bare message) so the
    // failure classifies as a validation error (exit 4) through the shared
    // `error_to_exit_code` downcast, matching `dep add`'s cycle rejection.
    crate::graph::DependencyGraph::new(&refs)
        .validate_dag()
        .map_err(anyhow::Error::new)
}

/// The simulation id of a not-yet-created role node. Prefixed so it cannot
/// collide with a real (uuid) issue id.
fn placeholder_id(role: &str) -> String {
    format!("__apply_placeholder__:{role}")
}

/// The simulation id of a delta endpoint.
fn endpoint_id(endpoint: &DeltaEndpoint) -> String {
    match endpoint {
        DeltaEndpoint::CreatedRole(role) => placeholder_id(role),
        DeltaEndpoint::ExistingIssue(id) => id.clone(),
    }
}

/// A synthetic graph node for the prospective-cycle simulation: an id and its
/// dependency ids, so the prospective post-apply graph (store issues + created
/// placeholders) can be checked with the existing
/// [`DependencyGraph`](crate::graph::DependencyGraph) without mutating any
/// stored issue.
struct ProspectiveNode {
    id: String,
    dependencies: Vec<String>,
}

impl crate::graph::GraphNode for ProspectiveNode {
    fn id(&self) -> &str {
        &self.id
    }

    fn dependencies(&self) -> &[String] {
        &self.dependencies
    }
}

/// The container membership labels every created node inherits: all of the
/// container's labels EXCEPT its own `type:` label (which each node replaces with
/// its own).
fn inherited_membership_labels(container: &Issue) -> Vec<String> {
    container
        .labels
        .iter()
        .filter(|l| !label_utils::is_type_label(l))
        .cloned()
        .collect()
}

/// The FINAL label set a node's created issue carries: the inherited container
/// membership labels, the node's own `type:<node.type>`, and its interpolated
/// `labels`.
fn node_labels(
    node: &TemplateNode,
    inherited: &[String],
    node_context: &InterpolationContext,
) -> Vec<String> {
    let mut labels = inherited.to_vec();
    labels.push(label_utils::type_label(&node.type_name));
    labels.extend(node.labels.iter().map(|l| node_context.interpolate(l)));
    labels
}

/// The title a node's created issue carries (`<role>: <container title>`).
fn node_title(node: &TemplateNode, container: &Issue) -> String {
    format!("{}: {}", node.role, container.title)
}

/// Compute a template node's final, GUARANTEED non-empty interpolated description.
///
/// A node with an explicit `description` template has its `{...}` tokens
/// resolved; a node with none, or whose template is absent/blank or interpolates
/// to whitespace-only, falls back to a generic role/title line so the created
/// issue is never seeded with an empty body (APPA-02).
pub(super) fn node_description(node: &TemplateNode, context: &InterpolationContext) -> String {
    let interpolated = node
        .description
        .as_deref()
        .map(|template| context.interpolate(template))
        .unwrap_or_default();
    if interpolated.trim().is_empty() {
        format!("{} node for {}.", node.role, context.title)
    } else {
        interpolated
    }
}

/// Fixed token-substitution context for template interpolation (PURE: no I/O).
///
/// Resolves the container-derived tokens declared by the template schema —
/// `{container.id}`, `{container.short_id}`, `{container.title}`,
/// `{container.hard_criteria}` — plus the per-node `{container.dir}` (the
/// canonical artifact directory of the area the node declares) and `{doc}` (the
/// node's own interpolated `doc`). [`for_node`](InterpolationContext::for_node)
/// is the one constructor, so every derivation of a node's prose, labels, and
/// document path substitutes the same context. This is a simple `{token}`
/// replace over a fixed map, and not a templating language.
#[derive(Debug, Clone)]
pub(super) struct InterpolationContext {
    id: String,
    short_id: String,
    title: String,
    hard_criteria: String,
    directory: Option<String>,
    doc: Option<String>,
}

impl InterpolationContext {
    /// Build the context `node`'s declarations are substituted into: the
    /// container tokens, the `{container.dir}` directory of the area `node`
    /// declares, and `node`'s own interpolated `{doc}`.
    ///
    /// Every derivation that resolves a template declaration for a node builds
    /// its context here, so the apply engine and the derivations around it
    /// resolve one declaration through one substitution.
    ///
    /// # Errors
    ///
    /// [`ArtifactDirectoryError`] when the node's declared area is absent from
    /// the configured issue-scoped registry.
    pub(super) fn for_node(
        container: &Issue,
        node: &TemplateNode,
        documentation: &DocumentationConfig,
        hierarchy: &HierarchyConfig,
    ) -> std::result::Result<Self, ArtifactDirectoryError> {
        let directory = node_artifact_directory(node, container, documentation, hierarchy)?;
        Ok(Self::for_container(container).with_doc(node, directory))
    }

    /// Build the container-derived context (the `{container.dir}` and `{doc}`
    /// tokens are unset until a node is selected via [`with_doc`](Self::with_doc)).
    fn for_container(container: &Issue) -> Self {
        Self {
            id: container.id.clone(),
            short_id: container.short_id(),
            title: container.title.clone(),
            hard_criteria: extract_hard_criteria(&container.description),
            directory: None,
            doc: None,
        }
    }

    /// Produce a per-node copy of this context whose `{container.dir}` token
    /// resolves to `directory` and whose `{doc}` token resolves to `node`'s own
    /// interpolated `doc` template (empty when the node has none).
    ///
    /// `directory` is the canonical artifact directory of the area `node`
    /// declares ([`node_artifact_directory`]); `None` leaves `{container.dir}`
    /// out of scope, which is what a node declaring no area resolves under. The
    /// node's `doc` is interpolated with `{container.dir}` already in scope and
    /// WITHOUT `{doc}`, so `{doc}` in a description always refers to the node's
    /// resolved doc path, never itself.
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
    fn interpolate(&self, template: &str) -> String {
        let mut out = template
            .replace("{container.id}", &self.id)
            .replace("{container.short_id}", &self.short_id)
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

/// Resolve the breakdown node's declared container label for `container`.
///
/// The label declaration is the source of truth for the namespace that ties a
/// breakdown node to its container. A declaration is eligible when it
/// interpolates either supported container identifier token; the resulting
/// label is what bracket lookup must use.
pub(super) fn declared_container_label(
    template: &GraphTemplate,
    roles: &crate::templates::RoleBindings,
    container: &Issue,
) -> Result<String> {
    let node = template.breakdown_node(roles).ok_or_else(|| {
        anyhow!(
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
            anyhow!(
                "template '{}' breakdown node '{}' declares no container label; bracket lookup cannot resolve the applied node",
                template.name,
                node.role
            )
        })
}

/// Extract the container's `[hard]` success criteria as a newline-joined block
/// for the `{container.hard_criteria}` token (PURE: line scan, no parser).
///
/// Collects each list item marked `[hard]` (after stripping a leading `-`/`*`
/// bullet and whitespace) from the description, preserving order. Returns the
/// empty string when none are present.
fn extract_hard_criteria(description: &str) -> String {
    description
        .lines()
        .map(str::trim)
        .map(|line| line.trim_start_matches(['-', '*', '+']).trim())
        .filter(|line| line.starts_with("[hard]"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The declarations shared by the tests of every derivation that resolves a
/// template document declaration: the apply engine here, and the capture-path
/// and refresh derivations in [`super::template`].
///
/// One area registry, one taxonomy, one container, and one bracket template for
/// all of them, so a test comparing two derivations compares their answers to
/// the same declaration rather than to a copied string
/// (`@/inv/shared-test-contracts`).
#[cfg(test)]
pub(crate) mod test_declarations {
    use super::*;
    use crate::templates::TemplateRegistry;
    use std::collections::HashMap;

    /// The type vocabulary the shared templates are loaded against.
    pub(crate) const HIERARCHY: [&str; 3] = ["epic", "planning", "breakdown"];

    /// The one area these declarations name. Unrelated to the shipped
    /// vocabulary, so a `dev/`-shaped assumption in a derivation fails here
    /// rather than passing by coincidence.
    pub(crate) const AREA: &str = "workspace/notes";

    /// The area registry the `{container.dir}` field is resolved against.
    pub(crate) fn documentation() -> DocumentationConfig {
        DocumentationConfig {
            issue_scoped_areas: Some(vec![AREA.to_string()]),
            ..Default::default()
        }
    }

    /// The type taxonomy the resolver reads the membership namespace from: the
    /// container fixture's single `area:` value names its directory's slug.
    pub(crate) fn hierarchy() -> HierarchyConfig {
        HierarchyConfig::new(
            HashMap::from([("epic".to_string(), 1)]),
            HashMap::from([("epic".to_string(), "area".to_string())]),
        )
        .unwrap()
    }

    /// The container every derivation resolves its declarations for.
    pub(crate) fn container(id: &str) -> Issue {
        let mut issue = crate::domain::types::fixture_issue(
            "Auth epic".to_string(),
            "- [hard] REQ-01: x".to_string(),
        );
        issue.labels = vec!["type:epic".to_string(), "area:auth".to_string()];
        issue.id = id.to_string();
        issue
    }

    /// The named template of `toml`, loaded against [`HIERARCHY`].
    pub(crate) fn template_from(toml: &str, name: &str) -> GraphTemplate {
        TemplateRegistry::from_toml_str(toml, &HIERARCHY)
            .unwrap()
            .get(name)
            .unwrap()
            .clone()
    }

    /// A bracket template whose planning node declares `doc` (resolved in
    /// `doc_area`, absent for a declaration naming no area) and whose
    /// description is exactly its interpolated document path, so a delta reports
    /// the path the declaration produces ([`planned_document`]).
    ///
    /// The breakdown node carries its declared container label, which locates an
    /// applied bracket, so the same template drives a fresh apply and a refresh.
    pub(crate) fn document_template(doc: &str, doc_area: Option<&str>) -> GraphTemplate {
        let toml = r#"
[[template]]
name       = "doc"
applies_to = ["epic"]
  [[template.anchors]]
  name = "container"
  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  doc         = "@DOC@"
@AREA@  description = "{doc}"
  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
  labels      = ["brackets:{container.short_id}"]
  depends_on  = ["planning"]
  [[template.anchor_edges]]
  from = "container"
  to   = "breakdown"
"#
        .replace("@DOC@", doc)
        .replace(
            "@AREA@",
            &doc_area
                .map(|area| format!("  doc_area    = \"{area}\"\n"))
                .unwrap_or_default(),
        );
        template_from(&toml, "doc")
    }

    /// The document path the planning node of a [`document_template`] resolved.
    pub(crate) fn planned_document(delta: &TemplateDelta) -> &str {
        &delta
            .creates
            .iter()
            .find(|planned| planned.role == "planning")
            .expect("the shared template declares a planning node")
            .description
    }

    /// The anchor bindings a shared template is expanded under.
    pub(crate) fn bindings(container_id: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("container".to_string(), container_id.to_string())])
    }

    /// The container anchor's pre-apply dependency snapshot.
    pub(crate) fn snapshots(container_deps: &[&str]) -> BTreeMap<String, Vec<String>> {
        BTreeMap::from([(
            "container".to_string(),
            container_deps.iter().map(|s| s.to_string()).collect(),
        )])
    }

    /// Expand `template` against these declarations.
    pub(crate) fn expand(
        template: &GraphTemplate,
        container: &Issue,
        resolved_bindings: &BTreeMap<String, String>,
        anchor_dependency_snapshots: &BTreeMap<String, Vec<String>>,
    ) -> Result<TemplateDelta> {
        expand_template(
            template,
            container,
            resolved_bindings,
            anchor_dependency_snapshots,
            &documentation(),
            &hierarchy(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::test_declarations::*;
    use super::*;

    fn plan_template() -> GraphTemplate {
        template_from(
            r#"
[[template]]
name       = "plan"
applies_to = ["epic"]
  [[template.anchors]]
  name  = "container"
  gates = ["repo-validate"]
  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  gates       = ["plan-review"]
  doc         = "dev/active/{container.id}-plan.md"
  description = "Plan {container.title}. Doc: {doc}."
  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
  labels      = ["brackets:{container.short_id}"]
  depends_on  = ["planning"]
  [[template.anchor_edges]]
  from = "container"
  to   = "breakdown"
  [[template.transforms]]
  kind = "move-upstream-to-role"
  role = "planning"
"#,
            "plan",
        )
    }

    #[test]
    fn test_expand_template_produces_creates_edges_and_removals() {
        let container = container("c1");
        let delta = expand(
            &plan_template(),
            &container,
            &bindings("c1"),
            &snapshots(&["u1"]),
        )
        .unwrap();

        // Two nodes, in template order, with interpolated prose and labels.
        assert_eq!(delta.creates.len(), 2);
        assert_eq!(delta.creates[0].role, "planning");
        assert_eq!(
            delta.creates[0].description,
            "Plan Auth epic. Doc: dev/active/c1-plan.md."
        );
        assert_eq!(delta.creates[0].gates, vec!["plan-review".to_string()]);
        assert!(delta.creates[1]
            .labels
            .contains(&format!("brackets:{}", container.short_id())));

        // Internal B→P, anchor C→B, transform P→u1.
        assert_eq!(
            delta.add_edges,
            vec![
                DeltaEdge {
                    dependent: DeltaEndpoint::CreatedRole("breakdown".to_string()),
                    dependency: DeltaEndpoint::CreatedRole("planning".to_string()),
                },
                DeltaEdge {
                    dependent: DeltaEndpoint::ExistingIssue("c1".to_string()),
                    dependency: DeltaEndpoint::CreatedRole("breakdown".to_string()),
                },
                DeltaEdge {
                    dependent: DeltaEndpoint::CreatedRole("planning".to_string()),
                    dependency: DeltaEndpoint::ExistingIssue("u1".to_string()),
                },
            ]
        );

        // The container loses the upstream dep the transform moved.
        assert_eq!(
            delta.remove_edges,
            vec![DeltaEdge {
                dependent: DeltaEndpoint::ExistingIssue("c1".to_string()),
                dependency: DeltaEndpoint::ExistingIssue("u1".to_string()),
            }]
        );

        // The anchor's gates ride along, bound to the container's id.
        assert_eq!(
            delta.anchor_gates,
            vec![AnchorGates {
                anchor_issue_id: "c1".to_string(),
                gates: vec!["repo-validate".to_string()],
            }]
        );
    }

    #[test]
    fn test_expand_template_moves_only_pre_apply_upstream_deps() {
        // The transform reads the SNAPSHOT, so an id absent from it is never
        // moved even when the same delta wires an edge to it (APPB-02).
        let container = container("c1");
        let delta = expand(
            &plan_template(),
            &container,
            &bindings("c1"),
            &snapshots(&[]),
        )
        .unwrap();
        assert!(delta.remove_edges.is_empty());
        assert!(!delta
            .add_edges
            .iter()
            .any(|e| e.dependent == DeltaEndpoint::CreatedRole("planning".to_string())));
    }

    #[test]
    fn test_expand_template_uses_declared_breakdown_label_namespace() {
        let mut template = plan_template();
        template
            .nodes
            .iter_mut()
            .find(|node| node.role == "breakdown")
            .expect("plan template declares a breakdown node")
            .labels = vec!["custom-anchor:{container.short_id}".to_string()];

        let container = container("abcdef123456");
        let delta = expand(
            &template,
            &container,
            &bindings(&container.id),
            &snapshots(&[]),
        )
        .unwrap();
        let breakdown = delta
            .creates
            .iter()
            .find(|node| node.role == "breakdown")
            .expect("plan template creates a breakdown node");

        assert!(breakdown
            .labels
            .contains(&format!("custom-anchor:{}", container.short_id())));
    }

    #[test]
    fn test_expand_template_rejects_unbound_anchor() {
        let container = container("c1");
        let err = expand(
            &plan_template(),
            &container,
            &BTreeMap::new(),
            &snapshots(&[]),
        )
        .unwrap_err();
        assert!(err.to_string().contains("container"), "{err}");
    }

    #[test]
    fn test_expand_template_rejects_unsupported_transform_kind() {
        // The loader rejects an unsupported kind, so this can only be reached with
        // a hand-built template; expansion is the defensive gate and it still runs
        // before any write.
        let mut template = plan_template();
        template.transforms[0].kind = "teleport".to_string();
        let container = container("c1");
        let err = expand(&template, &container, &bindings("c1"), &snapshots(&["u1"])).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("teleport"), "{msg}");
        assert!(msg.contains("move-upstream-to-role"), "{msg}");
    }

    /// A one-node template whose node's description is exactly its interpolated
    /// `doc`, so the delta reports the document path the declaration produces.
    fn document_template(doc: &str, doc_area: Option<&str>) -> GraphTemplate {
        let toml = r#"
[[template]]
name       = "doc"
applies_to = ["epic"]
  [[template.anchors]]
  name = "container"
  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  doc         = "@DOC@"
@AREA@  description = "{doc}"
"#
        .replace("@DOC@", doc)
        .replace(
            "@AREA@",
            &doc_area
                .map(|area| format!("  doc_area    = \"{area}\"\n"))
                .unwrap_or_default(),
        );
        template_from(&toml, "doc")
    }

    /// The document path the single node of a [`document_template`] resolved.
    fn planned_document(delta: &TemplateDelta) -> &str {
        &delta.creates[0].description
    }

    #[test]
    fn test_expand_template_resolves_a_declared_document_area_into_the_document_path() {
        let container = container("c1");
        let template = document_template("{container.dir}/plan.md", Some(AREA));

        let delta = expand(&template, &container, &bindings("c1"), &snapshots(&[])).unwrap();

        // The interpolated directory is the domain resolver's own answer for the
        // declared area, so the composition rule is not derived a second time;
        // the declared filename rides along verbatim, inside that directory.
        let directory =
            resolve_artifact_directory(&container, AREA, &documentation(), &hierarchy()).unwrap();
        assert_eq!(planned_document(&delta), format!("{directory}/plan.md"));
        assert!(
            directory.starts_with(&format!("{AREA}/")),
            "the declared area selects where the directory is resolved: {directory}"
        );
    }

    #[test]
    fn test_expand_template_rejects_a_document_area_the_registry_does_not_declare() {
        let container = container("c1");
        // A sibling of the declared area: close enough that only the registry
        // distinguishes it.
        let undeclared = format!("{AREA}-drafts");
        let template = document_template("{container.dir}/plan.md", Some(&undeclared));

        let error = expand(&template, &container, &bindings("c1"), &snapshots(&[])).unwrap_err();

        match error.downcast_ref::<ArtifactDirectoryError>() {
            Some(ArtifactDirectoryError::UndeclaredArea { area, declared }) => {
                assert_eq!(area, &undeclared);
                assert_eq!(declared, &documentation().issue_scoped_areas());
            }
            other => panic!("expected the resolver's undeclared-area rejection, got {other:?}"),
        }
        // The reported message locates the declaration that failed and names
        // the registry the area was matched against, without reading the chain.
        let message = error.to_string();
        assert!(message.contains("planning"), "{message}");
        assert!(message.contains(&undeclared), "{message}");
        assert!(message.contains(AREA), "{message}");

        // The rejection is the registry's doing: the same declaration naming a
        // declared area expands.
        assert!(expand(
            &document_template("{container.dir}/plan.md", Some(AREA)),
            &container,
            &bindings("c1"),
            &snapshots(&[]),
        )
        .is_ok());
    }

    #[test]
    fn test_expand_template_resolves_a_node_that_declares_no_document_area_without_a_directory() {
        let container = container("c1");

        // A flat area-plus-prefix declaration resolves from the container tokens
        // alone.
        let flat = expand(
            &document_template("dev/active/{container.short_id}-plan.md", None),
            &container,
            &bindings("c1"),
            &snapshots(&[]),
        )
        .unwrap();
        assert_eq!(
            planned_document(&flat),
            format!("dev/active/{}-plan.md", container.short_id())
        );

        // With no area to resolve in, the directory field is a token this context
        // does not carry, and stays verbatim like any other.
        let unscoped = expand(
            &document_template("{container.dir}/plan.md", None),
            &container,
            &bindings("c1"),
            &snapshots(&[]),
        )
        .unwrap();
        assert_eq!(planned_document(&unscoped), "{container.dir}/plan.md");
    }

    #[test]
    fn test_validate_delta_acyclic_accepts_the_plan_spine() {
        let container = container("c1");
        let delta = expand(
            &plan_template(),
            &container,
            &bindings("c1"),
            &snapshots(&["u1"]),
        )
        .unwrap();
        let store = BTreeMap::from([
            ("c1".to_string(), vec!["u1".to_string()]),
            ("u1".to_string(), vec![]),
        ]);
        assert!(validate_delta_acyclic(&delta, store).is_ok());
    }

    #[test]
    fn test_validate_delta_acyclic_rejects_a_prospective_cycle() {
        // `u1` depends on the breakdown node, B→P (internal), and the transform
        // moves C's snapshot dep `u1` onto P: u1→B→P→u1.
        let template = template_from(
            r#"
[[template]]
name       = "cyclic"
applies_to = ["epic"]
  [[template.anchors]]
  name = "container"
  [[template.anchors]]
  name = "upstream"
  [[template.nodes]]
  role = "planning"
  type = "planning"
  [[template.nodes]]
  role       = "breakdown"
  type       = "breakdown"
  depends_on = ["planning"]
  [[template.anchor_edges]]
  from = "upstream"
  to   = "breakdown"
  [[template.transforms]]
  kind = "move-upstream-to-role"
  role = "planning"
"#,
            "cyclic",
        );
        let container = container("c1");
        let bindings = BTreeMap::from([
            ("container".to_string(), "c1".to_string()),
            ("upstream".to_string(), "u1".to_string()),
        ]);
        let delta = expand(&template, &container, &bindings, &snapshots(&["u1"])).unwrap();
        let store = BTreeMap::from([
            ("c1".to_string(), vec!["u1".to_string()]),
            ("u1".to_string(), vec![]),
        ]);
        let err = validate_delta_acyclic(&delta, store).unwrap_err();
        assert!(err.to_string().contains("cycle"), "{err}");
    }

    #[test]
    fn test_validate_delta_acyclic_models_transform_removals() {
        // C→u1 exists and the transform moves it onto P, so the prospective graph
        // is C→B→P→u1. Without modeling the REMOVAL of C→u1 this is still acyclic,
        // so assert the positive case: a graph whose only cycle the removal breaks.
        let template = template_from(
            r#"
[[template]]
name       = "mover"
applies_to = ["epic"]
  [[template.anchors]]
  name = "container"
  [[template.nodes]]
  role = "planning"
  type = "planning"
  [[template.transforms]]
  kind = "move-upstream-to-role"
  role = "planning"
"#,
            "mover",
        );
        let container = container("c1");
        let delta = expand(&template, &container, &bindings("c1"), &snapshots(&["u1"])).unwrap();
        // u1 depends on c1, and c1 depends on u1: the store is already cyclic
        // EXCEPT that the transform removes c1→u1, leaving u1→c1 and P→u1.
        let store = BTreeMap::from([
            ("c1".to_string(), vec!["u1".to_string()]),
            ("u1".to_string(), vec!["c1".to_string()]),
        ]);
        assert!(validate_delta_acyclic(&delta, store).is_ok());
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
        let mut issue = crate::domain::types::fixture_issue(
            "Auth epic".to_string(),
            "- [hard] REQ-01: x".to_string(),
        );
        issue.id = "abc123def456".to_string();
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
        let rendered = node_description(&node, &ctx);
        assert!(rendered.contains("Auth epic"));
        assert!(rendered.contains("abc123de")); // short id (8 chars)
        assert!(rendered.contains("dev/active/abc123def456-plan.md")); // {doc}
        assert!(rendered.contains("[hard] REQ-01: x"));
    }

    #[test]
    fn test_node_description_falls_back_when_absent() {
        let issue = crate::domain::types::fixture_issue("Epic X".to_string(), String::new());
        let node = TemplateNode {
            role: "breakdown".to_string(),
            type_name: "breakdown".to_string(),
            gates: vec![],
            doc: None,
            doc_area: None,
            description: None,
            labels: vec![],
            depends_on: vec![],
        };
        let ctx = InterpolationContext::for_container(&issue).with_doc(&node, None);
        let rendered = node_description(&node, &ctx);
        assert!(!rendered.is_empty());
        assert!(rendered.contains("Epic X"));
    }

    #[test]
    fn test_node_description_falls_back_when_blank_or_whitespace() {
        let issue = crate::domain::types::fixture_issue("Epic Y".to_string(), String::new());
        // An explicitly empty template and a whitespace-only one must both fall
        // back to the non-empty role/title line (APPA-02).
        for desc in ["", "   \n\t "] {
            let node = TemplateNode {
                role: "planning".to_string(),
                type_name: "planning".to_string(),
                gates: vec![],
                doc: None,
                doc_area: None,
                description: Some(desc.to_string()),
                labels: vec![],
                depends_on: vec![],
            };
            let ctx = InterpolationContext::for_container(&issue).with_doc(&node, None);
            let rendered = node_description(&node, &ctx);
            assert!(!rendered.trim().is_empty(), "desc {desc:?} yielded empty");
            assert!(rendered.contains("Epic Y"));
        }
    }

    #[test]
    fn test_interpolate_leaves_unknown_token_verbatim() {
        // The interpolation context is a fixed `{token}` substitution, not a
        // templating language: an unsupported `{token}` is left untouched while the
        // known container tokens around it still resolve.
        let mut issue = crate::domain::types::fixture_issue("Auth epic".to_string(), String::new());
        issue.id = "abc123def456".to_string();
        let ctx = InterpolationContext::for_container(&issue);
        let rendered = ctx.interpolate("{container.title} then {totally.unknown} end");
        assert_eq!(rendered, "Auth epic then {totally.unknown} end");
    }

    #[test]
    fn test_node_labels_interpolates_and_replaces_container_type_label() {
        // The final label set a created node carries: inherited container
        // membership labels (NOT the container's own `type:`), the node's own
        // `type:<node.type>`, and its interpolated `labels`. The `{container.short_id}`
        // token in a node label must resolve to the container's short id.
        let container = container("abc123def456");
        let short = container.short_id();

        let node = TemplateNode {
            role: "breakdown".to_string(),
            type_name: "breakdown".to_string(),
            gates: vec![],
            doc: None,
            doc_area: None,
            description: None,
            labels: vec!["brackets:{container.short_id}".to_string()],
            depends_on: vec![],
        };
        let inherited = inherited_membership_labels(&container);
        let ctx = InterpolationContext::for_container(&container).with_doc(&node, None);
        let labels = node_labels(&node, &inherited, &ctx);

        // Inherited non-type label is carried; the container's `type:epic` is not.
        assert!(labels.contains(&"area:auth".to_string()));
        assert!(!labels.iter().any(|l| l == "type:epic"));
        // The node's own type label is present.
        assert!(labels.contains(&"type:breakdown".to_string()));
        // The node label's `{container.short_id}` token resolved.
        assert!(labels.contains(&format!("brackets:{short}")));
        assert!(!labels.iter().any(|l| l.contains("{container.")));
    }
}
