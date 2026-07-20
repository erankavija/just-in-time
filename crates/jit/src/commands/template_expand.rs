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
//! substitution of [`InterpolationContext`].

use std::collections::BTreeMap;

use anyhow::{anyhow, Result};

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
/// # Errors
///
/// Returns an error when the template references an unbound anchor, a role no
/// node declares, or an unsupported transform kind (the registry loader rejects
/// all three, so this is the defensive gate for hand-built templates).
///
/// # Examples
///
/// ```
/// use jit::commands::expand_template;
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
/// let delta = expand_template(template, &container, &bindings, &snapshots).unwrap();
/// assert_eq!(delta.creates.len(), 1);
/// assert_eq!(delta.creates[0].description, "Plan Auth epic.");
/// assert_eq!(delta.add_edges.len(), 1);
/// ```
pub fn expand_template(
    template: &GraphTemplate,
    container: &Issue,
    resolved_bindings: &BTreeMap<String, String>,
    anchor_dependency_snapshots: &BTreeMap<String, Vec<String>>,
) -> Result<TemplateDelta> {
    let context = InterpolationContext::for_container(container);
    let inherited = inherited_membership_labels(container);

    let creates: Vec<PlannedNode> = template
        .nodes
        .iter()
        .map(|node| {
            let node_context = context.with_doc(node);
            PlannedNode {
                role: node.role.clone(),
                title: node_title(node, container),
                description: node_description(node, &node_context),
                labels: node_labels(node, &inherited, &node_context),
                gates: node.gates.clone(),
                priority: container.priority,
            }
        })
        .collect();

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
/// `{container.hard_criteria}` — plus the per-node `{doc}` token (the node's own
/// interpolated `doc`). Built once per apply via
/// [`for_container`](InterpolationContext::for_container); a per-node copy adding
/// `{doc}` is produced by [`with_doc`](InterpolationContext::with_doc). This is a
/// simple `{token}` replace over a fixed map, and not a templating language.
#[derive(Debug, Clone)]
pub(super) struct InterpolationContext {
    id: String,
    short_id: String,
    title: String,
    hard_criteria: String,
    doc: Option<String>,
}

impl InterpolationContext {
    /// Build the container-derived context (the `{doc}` token is unset until a
    /// node is selected via [`with_doc`](Self::with_doc)).
    pub(super) fn for_container(container: &Issue) -> Self {
        Self {
            id: container.id.clone(),
            short_id: container.short_id(),
            title: container.title.clone(),
            hard_criteria: extract_hard_criteria(&container.description),
            doc: None,
        }
    }

    /// Produce a per-node copy of this context whose `{doc}` token resolves to
    /// `node`'s own interpolated `doc` template (empty when the node has none).
    /// The node's `doc` is interpolated WITHOUT `{doc}` in scope, so `{doc}` in a
    /// description always refers to the node's resolved doc path, never itself.
    pub(super) fn with_doc(&self, node: &TemplateNode) -> Self {
        let doc = node
            .doc
            .as_deref()
            .map(|template| self.interpolate(template))
            .unwrap_or_default();
        Self {
            doc: Some(doc),
            ..self.clone()
        }
    }

    /// Substitute every supported `{token}` in `template` with its context value.
    ///
    /// Unset tokens (`{doc}` before a node is selected) substitute to the empty
    /// string; unknown `{...}` text is left verbatim.
    fn interpolate(&self, template: &str) -> String {
        let mut out = template
            .replace("{container.id}", &self.id)
            .replace("{container.short_id}", &self.short_id)
            .replace("{container.title}", &self.title)
            .replace("{container.hard_criteria}", &self.hard_criteria);
        if let Some(doc) = &self.doc {
            out = out.replace("{doc}", doc);
        }
        out
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::templates::TemplateRegistry;

    const HIERARCHY: [&str; 3] = ["epic", "planning", "breakdown"];

    fn template_from(toml: &str, name: &str) -> GraphTemplate {
        TemplateRegistry::from_toml_str(toml, &HIERARCHY)
            .unwrap()
            .get(name)
            .unwrap()
            .clone()
    }

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

    fn epic(id: &str) -> Issue {
        let mut issue = crate::domain::types::fixture_issue(
            "Auth epic".to_string(),
            "- [hard] REQ-01: x".to_string(),
        );
        issue.labels = vec!["type:epic".to_string(), "area:auth".to_string()];
        issue.id = id.to_string();
        issue
    }

    fn bindings(container_id: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("container".to_string(), container_id.to_string())])
    }

    fn snapshots(container_deps: &[&str]) -> BTreeMap<String, Vec<String>> {
        BTreeMap::from([(
            "container".to_string(),
            container_deps.iter().map(|s| s.to_string()).collect(),
        )])
    }

    #[test]
    fn test_expand_template_produces_creates_edges_and_removals() {
        let container = epic("c1");
        let delta = expand_template(
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
        let container = epic("c1");
        let delta = expand_template(
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
    fn test_expand_template_rejects_unbound_anchor() {
        let container = epic("c1");
        let err = expand_template(
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
        let container = epic("c1");
        let err = expand_template(&template, &container, &bindings("c1"), &snapshots(&["u1"]))
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("teleport"), "{msg}");
        assert!(msg.contains("move-upstream-to-role"), "{msg}");
    }

    #[test]
    fn test_validate_delta_acyclic_accepts_the_plan_spine() {
        let container = epic("c1");
        let delta = expand_template(
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
        let container = epic("c1");
        let bindings = BTreeMap::from([
            ("container".to_string(), "c1".to_string()),
            ("upstream".to_string(), "u1".to_string()),
        ]);
        let delta = expand_template(&template, &container, &bindings, &snapshots(&["u1"])).unwrap();
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
        let container = epic("c1");
        let delta =
            expand_template(&template, &container, &bindings("c1"), &snapshots(&["u1"])).unwrap();
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
            description: Some(
                "Plan {container.title} ({container.short_id}). Doc: {doc}. Cover: {container.hard_criteria}."
                    .to_string(),
            ),
            labels: vec![],
            depends_on: vec![],
        };
        let ctx = InterpolationContext::for_container(&issue).with_doc(&node);
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
            description: None,
            labels: vec![],
            depends_on: vec![],
        };
        let ctx = InterpolationContext::for_container(&issue).with_doc(&node);
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
                description: Some(desc.to_string()),
                labels: vec![],
                depends_on: vec![],
            };
            let ctx = InterpolationContext::for_container(&issue).with_doc(&node);
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
        let container = epic("abc123def456");
        let short = container.short_id();

        let node = TemplateNode {
            role: "breakdown".to_string(),
            type_name: "breakdown".to_string(),
            gates: vec![],
            doc: None,
            description: None,
            labels: vec!["brackets:{container.short_id}".to_string()],
            depends_on: vec![],
        };
        let inherited = inherited_membership_labels(&container);
        let ctx = InterpolationContext::for_container(&container).with_doc(&node);
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
