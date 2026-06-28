//! Graph-template apply engine (the `jit apply` engine).
//!
//! [`apply_template`](CommandExecutor::apply_template) instantiates a
//! [`GraphTemplate`](crate::templates::GraphTemplate) onto a container. On a
//! fresh apply it:
//!
//! 1. validates every precondition BEFORE the first mutation (anchors resolve,
//!    container type ∈ `applies_to`, every node AND anchor gate resolves — as a
//!    gate preset OR a registry gate key, node writes would pass validation, not
//!    already-applied unless `--force`);
//! 2. snapshots each bound anchor's current dependencies;
//! 3. creates the template's nodes with interpolated descriptions and
//!    their declared gates; then
//! 4. wires the template's edges and runs its transforms: the internal
//!    `depends_on` edges (e.g. `B → P`), the `anchor_edges` (e.g. `C → B`), and
//!    the `move-upstream-to-role` transform that moves the container's pre-apply
//!    snapshot deps onto the named role's node — all via
//!    [`add_dependency`](CommandExecutor::add_dependency), so the result is
//!    acyclic and transitively reduced; then
//! 5. attaches each anchor's declared gates to its bound anchor issue, through
//!    the same shared resolve-and-attach path node gates use (each gate is a
//!    preset OR a registry gate key).
//!
//! This is the plan-before-fan-out scaffold: the create+gate+wire sequence
//! that produces the `C → B → P` bracket from a template. The planning node's
//! `doc` template seeds its description with the plan-doc location to author and
//! link; the engine attaches no document reference (the author links the plan
//! once written, so apply never leaves a reference to a not-yet-created file). The `--force` refresh
//! path re-seeds existing nodes' prose in place
//! and re-runs neither node creation, edge wiring, nor transforms (the spine
//! already exists; re-running the transform over now-scaffold-bearing live deps
//! would corrupt it).
//!
//! # Domain-agnostic
//!
//! No `epic` / `planning` / `breakdown` literal is hardcoded. Node types, gates
//! (preset or registry key), doc locations, descriptions, and labels all come
//! from the template;
//! the only roles this engine reaches for by name are the conventional
//! [`PLANNING_ROLE`](crate::templates::PLANNING_ROLE) /
//! [`BREAKDOWN_ROLE`](crate::templates::BREAKDOWN_ROLE), and only on the
//! `--force` refresh path to locate already-applied nodes.

use std::collections::BTreeMap;

use super::*;
use crate::templates::{GraphTemplate, TemplateNode, BREAKDOWN_ROLE};
use serde::Serialize;

/// How a template node/anchor gate NAME resolves: a registered gate PRESET
/// bundle, or a single gate KEY declared in the gate registry (`.jit/gates.json`).
///
/// Resolving anchors/nodes against BOTH lets a config-declared gate (e.g.
/// `repo-validate`) be referenced from a template without being a built-in
/// preset, keeping engine code free of any gate/container literal.
enum TemplateGateResolution {
    /// A registered gate preset; apply its whole bundle via `apply_gate_preset`.
    Preset,
    /// A single gate key in the registry; attach that one gate via `add_gates`.
    RegistryKey,
}

/// Outcome of applying a graph template to a container.
///
/// Names the template applied and the anchor bindings used, maps each created
/// node's template ROLE to the id of the issue created (or refreshed) for it,
/// and carries the PRE-APPLY snapshot of each bound anchor's dependencies. The
/// snapshot is what the `move-upstream-to-role` transform moves onto the planning
/// node; capturing it before any mutation is what lets the transform move exactly
/// the container's ORIGINAL upstream deps (and never the freshly-wired scaffold
/// edges). It is also surfaced for callers/tests that inspect the pre-apply
/// shape.
///
/// # Examples
///
/// ```
/// use jit::commands::TemplateApplyResult;
/// use std::collections::BTreeMap;
///
/// let result = TemplateApplyResult {
///     template: "plan".to_string(),
///     anchor_bindings: BTreeMap::from([("container".to_string(), "c1".to_string())]),
///     created_node_ids_by_role: BTreeMap::from([
///         ("planning".to_string(), "p1".to_string()),
///         ("breakdown".to_string(), "b1".to_string()),
///     ]),
///     anchor_dependency_snapshots: BTreeMap::from([
///         ("container".to_string(), vec!["u1".to_string()]),
///     ]),
/// };
/// assert_eq!(result.created_node_ids_by_role["planning"], "p1");
/// assert_eq!(result.anchor_dependency_snapshots["container"], vec!["u1".to_string()]);
/// ```
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TemplateApplyResult {
    /// The applied template's name (e.g. `"plan"`).
    pub template: String,
    /// Anchor name → bound issue id (full id), in anchor-name order.
    pub anchor_bindings: BTreeMap<String, String>,
    /// Template node role → the created (or `--force`-refreshed) issue id.
    pub created_node_ids_by_role: BTreeMap<String, String>,
    /// Anchor name → that anchor's `dependencies` as snapshotted BEFORE any
    /// mutation. Consumed by the `move-upstream-to-role` transform and surfaced
    /// for callers inspecting the pre-apply shape.
    pub anchor_dependency_snapshots: BTreeMap<String, Vec<String>>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Apply a graph template named `template_name` to `container_id`
    /// (`jit apply <template> <container>`).
    ///
    /// Reads the template from the cached [`TemplateRegistry`](crate::templates::TemplateRegistry)
    /// (`.jit/templates.toml`) and delegates to
    /// [`apply_template_with`](Self::apply_template_with). `anchor_bindings` maps
    /// each declared anchor name to an issue id; the `container` anchor is
    /// commonly bound to `container_id` by the CLI before calling.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    /// use std::collections::BTreeMap;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let bindings = BTreeMap::from([("container".to_string(), "epic-123".to_string())]);
    /// let (result, _warnings) =
    ///     executor.apply_template("plan", "epic-123", &bindings, false).unwrap();
    /// println!("applied {} → {:?}", result.template, result.created_node_ids_by_role);
    /// ```
    pub fn apply_template(
        &self,
        template_name: &str,
        container_id: &str,
        anchor_bindings: &BTreeMap<String, String>,
        force: bool,
    ) -> Result<(TemplateApplyResult, Vec<String>)> {
        // Clone the template out of the cached config: the engine mutates issues
        // through `&self`, so it cannot hold a borrow into the config cache.
        let template = self
            .cached_config()?
            .templates
            .get(template_name)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "no template '{template_name}' in .jit/templates.toml; \
                     declare it or check the name"
                )
            })?;
        self.apply_template_with(&template, container_id, anchor_bindings, force)
    }

    /// Apply an explicit [`GraphTemplate`] — the registry-independent core of
    /// [`apply_template`](Self::apply_template).
    ///
    /// Separated so the engine is testable without an on-disk `templates.toml`
    /// (the registry-independent core). Steps:
    ///
    /// 1. **Validate before mutating** — the single complete gate. The container
    ///    type is in the template's `applies_to`; every declared anchor is bound
    ///    and resolves to an existing issue; every node's AND anchor's gate presets
    ///    exist and each node's projected write would pass validation; every
    ///    transform's `kind` is supported; the container is not already-applied
    ///    unless `force`. Any failure aborts BEFORE the first `create_issue`, so a
    ///    validation failure creates zero nodes. After this gate the mutation phase
    ///    can only fail on I/O.
    /// 2. **Snapshot** each bound anchor's `dependencies` (the
    ///    `move-upstream-to-role` transform reads this pre-apply set).
    /// 3. **Instantiate** each template node in order (or refresh in place under
    ///    `force`): create the typed issue with inherited container membership
    ///    labels plus the node's interpolated labels and a non-empty interpolated
    ///    description (the planning node's interpolated `doc` location is seeded
    ///    into its description as an instruction to author and link there), and
    ///    attach each declared gate preset.
    /// 4. **Wire edges + run transforms** (fresh apply only): the template's
    ///    internal `depends_on` edges and `anchor_edges` via
    ///    [`add_dependency`](Self::add_dependency) (cycle-checked + transitively
    ///    reduced), then each transform (`move-upstream-to-role`) over the step-2
    ///    snapshot. The `--force` refresh path skips this: the edges already exist.
    /// 5. **Attach anchor gates** (fresh apply only): each anchor's declared gate
    ///    presets are attached to its bound anchor issue via the same shared
    ///    `apply_gate_preset` path node gates use.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    /// use jit::templates::TemplateRegistry;
    /// use std::collections::BTreeMap;
    ///
    /// let toml = r#"
    /// [[template]]
    /// name = "plan"
    /// applies_to = ["epic"]
    /// [[template.nodes]]
    /// role = "planning"
    /// type = "planning"
    /// description = "Plan {container.title}."
    /// "#;
    /// let registry = TemplateRegistry::from_toml_str(toml, &["epic", "planning"]).unwrap();
    /// let template = registry.get("plan").unwrap();
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let bindings = BTreeMap::from([("container".to_string(), "epic-123".to_string())]);
    /// let (result, _warnings) =
    ///     executor.apply_template_with(template, "epic-123", &bindings, false).unwrap();
    /// assert_eq!(result.template, "plan");
    /// ```
    pub fn apply_template_with(
        &self,
        template: &GraphTemplate,
        container_id: &str,
        anchor_bindings: &BTreeMap<String, String>,
        force: bool,
    ) -> Result<(TemplateApplyResult, Vec<String>)> {
        // === 1. Validate every non-edge precondition BEFORE mutating ===
        let full_container_id = self.storage.resolve_issue_id(container_id)?;
        let container = self.storage.load_issue(&full_container_id)?;

        // Container type ∈ applies_to.
        match label_utils::type_label_value(&container.labels) {
            Some(ty) if template.applies_to.iter().any(|a| a == ty) => {}
            Some(ty) => {
                return Err(anyhow!(
                    "template '{}' does not apply to container type '{ty}'; \
                     applies_to: {}",
                    template.name,
                    template.applies_to.join(", ")
                ))
            }
            None => {
                return Err(anyhow!(
                    "container {full_container_id} has no type: label; \
                     template '{}' applies to: {}",
                    template.name,
                    template.applies_to.join(", ")
                ))
            }
        }

        // Every declared anchor is bound and resolves to an existing issue.
        // Resolve into full ids so the snapshot and result are unambiguous.
        let mut resolved_bindings: BTreeMap<String, String> = BTreeMap::new();
        for anchor in &template.anchors {
            let bound = anchor_bindings.get(&anchor.name).ok_or_else(|| {
                anyhow!(
                    "template '{}' anchor '{}' is not bound; \
                     bind it with --anchor {}=<id>",
                    template.name,
                    anchor.name,
                    anchor.name
                )
            })?;
            let full = self.storage.resolve_issue_id(bound).with_context(|| {
                format!(
                    "template '{}' anchor '{}' is bound to '{bound}', which does not resolve \
                     to an existing issue",
                    template.name, anchor.name
                )
            })?;
            // A bound id must name a real issue, not just resolve syntactically.
            self.storage.load_issue(&full).with_context(|| {
                format!(
                    "template '{}' anchor '{}' is bound to '{bound}', which does not name an \
                     existing issue",
                    template.name, anchor.name
                )
            })?;
            resolved_bindings.insert(anchor.name.clone(), full);
        }

        // Every gate NAME declared across the template's nodes must resolve
        // BEFORE the first mutation — either as a registered gate PRESET bundle
        // or as a single gate KEY in the gate registry (`.jit/gates.json`).
        // `apply_gate_preset` / `add_gates` resolve lazily during instantiation,
        // so an unknown name would otherwise fail AFTER one or more nodes are
        // persisted — violating "a failure creates zero nodes" (APPA-01). Resolve
        // it read-only up front.
        for node in &template.nodes {
            for gate in &node.gates {
                self.resolve_template_gate(gate).with_context(|| {
                    format!(
                        "template '{}' node '{}' references gate '{gate}', which is neither a \
                         registered gate preset nor a gate defined in the registry",
                        template.name, node.role
                    )
                })?;
            }
        }

        // Anchor gates (jit:2614ecf2 — REQ-13) resolve the SAME way (preset OR
        // registry gate key) before any mutation, so an unknown anchor gate fails
        // up front rather than after the bound anchor issue is mutated. This is
        // what makes a config-declared gate like `repo-validate` usable from an
        // anchor without being a built-in preset.
        for anchor in &template.anchors {
            for gate in &anchor.gates {
                self.resolve_template_gate(gate).with_context(|| {
                    format!(
                        "template '{}' anchor '{}' references gate '{gate}', which is neither a \
                         registered gate preset nor a gate defined in the registry",
                        template.name, anchor.name
                    )
                })?;
            }
        }

        // Every declared transform's `kind` must be a supported kind BEFORE the
        // first mutation. The mutation-phase dispatch in `wire_template_edges`
        // parses the kind too, but doing it ONLY there would let an unknown kind
        // create nodes/edges and then fail — violating validate-before-mutate. The
        // loader validates a transform's role-target but not its kind, so this is
        // the gate for kinds. (Pure parse, no I/O.)
        for transform in &template.transforms {
            TransformKind::parse(&transform.kind).with_context(|| {
                format!(
                    "template '{}' declares an unsupported transform kind '{}'",
                    template.name, transform.kind
                )
            })?;
        }

        // Already-applied detection: the breakdown node carries
        // `brackets:<container-short-id>` and sits among the container's deps.
        let existing_breakdown = self.find_applied_breakdown(template, &container)?;
        if existing_breakdown.is_some() && !force {
            return Err(anyhow!(
                "container {full_container_id} already has template '{}' applied; \
                 pass --force to refresh the existing nodes in place",
                template.name
            ));
        }

        // Legacy P-only bracket detection: a container scaffolded by the removed
        // `jit plan` carries a planning node but no breakdown node. A fresh apply
        // would create a SECOND planning node, and the `move-upstream-to-role`
        // transform would demote the old planning node into the new one's deps —
        // a duplicate, malformed bracket. Detect a pre-existing planning-typed
        // dependency (with no breakdown node) and reject with guidance. (`--force`
        // targets the refresh path, which requires a breakdown node to locate the
        // bracket, so it cannot adopt a legacy P-only container either.)
        if existing_breakdown.is_none() {
            if let Some(planning_type) = template.planning_type() {
                let existing_planning = container.dependencies.iter().find_map(|dep_id| {
                    let dep = self.storage.load_issue(dep_id).ok()?;
                    (label_utils::type_label_value(&dep.labels) == Some(planning_type))
                        .then_some(dep)
                });
                if let Some(planning) = existing_planning {
                    return Err(anyhow!(
                        "container {full_container_id} already has a planning node ({}) but no \
                         breakdown node — a legacy P-only bracket. Applying '{}' would create a \
                         duplicate planning node. Remove the legacy planning node and its \
                         container edge first, then re-apply.",
                        planning.short_id(),
                        template.name
                    ));
                }
            }
        }

        // === 2. Snapshot each bound anchor's dependencies BEFORE any mutation ===
        // (The container's own deps are part of this when it is a bound anchor;
        // the edge-wiring + transform task consumes these pre-apply sets.)
        let mut anchor_dependency_snapshots: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, full_id) in &resolved_bindings {
            let anchor_issue = self.storage.load_issue(full_id)?;
            anchor_dependency_snapshots.insert(name.clone(), anchor_issue.dependencies.clone());
        }

        // Interpolation context is fixed for the whole apply: container-derived
        // tokens. The per-node `{doc}` token is layered in during instantiation.
        let context = InterpolationContext::for_container(&container);

        // On the fresh-apply path, pre-validate that every node's `create_issue`
        // can only fail on I/O — not on write-time validation (e.g. an
        // interpolated label that violates the canonical `namespace:value` rule).
        // This closes the last APPA-01 gap: a later node's invalid shape must be
        // caught BEFORE the first node is persisted. (The refresh path does not
        // create issues, so it has no such window.)
        if existing_breakdown.is_none() {
            self.prevalidate_node_writes(template, &container, &context)?;

            // Simulate the FULL prospective edge set over the current store + the
            // to-be-created nodes and verify it is acyclic, with ZERO mutation if
            // not. `wire_template_edges` adds edges one-by-one, so a cycle-forming
            // LATER edge would otherwise fail only after nodes + earlier edges are
            // persisted (a partial apply). This pre-check makes the precondition
            // phase the complete gate: acyclicity is verified up front (APPLY-04).
            self.prevalidate_acyclic(
                template,
                &full_container_id,
                &resolved_bindings,
                &anchor_dependency_snapshots,
            )?;
        }

        let mut warnings = Vec::new();
        let created_node_ids_by_role = if let Some(breakdown_id) = existing_breakdown {
            // === Force refresh: update existing nodes in place, no duplicates ===
            // Edges + transforms are NOT re-run here: they were wired by the
            // original fresh apply, the nodes already exist among the container's
            // deps, and a re-run transform would snapshot the (now scaffold-bearing)
            // live deps and move `B` onto `P`, breaking the spine. Refresh only
            // re-seeds prose.
            self.refresh_template_nodes(template, &breakdown_id, &context)?
        } else {
            // === 3. Instantiate nodes (fresh apply) ===
            let created =
                self.instantiate_template_nodes(template, &container, &context, &mut warnings)?;

            // === 4. Wire edges + run transforms (jit:73e5e853) ===
            // Internal `depends_on` + `anchor_edges` via `add_dependency` (which
            // runs cycle-check + eager transitive reduction, giving APPB-01's
            // acyclic+reduced result), then `move-upstream-to-role` over the
            // PRE-APPLY snapshot (APPB-02) — never a live read of the container's
            // deps, which now include the freshly-wired anchor edges.
            self.wire_template_edges(
                template,
                &full_container_id,
                &resolved_bindings,
                &created,
                &anchor_dependency_snapshots,
                &mut warnings,
            )?;

            // === 5. Attach anchor-level gate presets (jit:2614ecf2 — REQ-13) ===
            // After the bound anchor's template edges are wired, attach each
            // anchor's declared gate presets to its bound issue, through the same
            // shared preset path node gates use. Like node gates, this runs only
            // on the fresh-apply path (the refresh path re-seeds prose only).
            self.attach_anchor_gates(template, &resolved_bindings, &mut warnings)?;

            created
        };

        Ok((
            TemplateApplyResult {
                template: template.name.clone(),
                anchor_bindings: resolved_bindings,
                created_node_ids_by_role,
                anchor_dependency_snapshots,
            },
            warnings,
        ))
    }

    /// Pre-validate, BEFORE any mutation, that every node's `create_issue` call
    /// during instantiation can only fail on I/O, not on write-time validation
    /// (APPA-01).
    ///
    /// `create_issue` runs the effective local rules over the FINAL issue shape,
    /// including the always-enforced canonical `namespace:value` label-format
    /// rule. A template whose first node is valid but a LATER node interpolates to
    /// a label the rule rejects would otherwise persist the earlier nodes and then
    /// fail mid-instantiation. So here we build the SAME final issue shape each
    /// node would persist (the helpers below are the single source of truth shared
    /// with instantiation) and run it through the same
    /// [`validate_for_write`](Self::validate_for_write) read-only — it never
    /// touches storage — surfacing the offending node up front.
    fn prevalidate_node_writes(
        &self,
        template: &GraphTemplate,
        container: &Issue,
        context: &InterpolationContext,
    ) -> Result<()> {
        let inherited = inherited_membership_labels(container);
        for node in &template.nodes {
            let node_context = context.with_doc(node);
            let projected = self.project_node_issue(node, container, &inherited, &node_context);
            // Any validation failure for a projected node is reported as an
            // argument error (exit 2) carrying this message verbatim, exactly as
            // the previous `.with_context(...)` text was classified. Mapping
            // (rather than wrapping) keeps the inner error's type from shifting the
            // exit code via downcast, and the top-level Display is unchanged.
            self.validate_for_write(&projected, false).map_err(|_| {
                crate::errors::InvalidArgumentError::new(format!(
                    "template '{}' node '{}' would create an invalid issue",
                    template.name, node.role
                ))
            })?;
        }
        Ok(())
    }

    /// Simulate the full prospective post-apply edge set, read-only, and verify it
    /// is acyclic BEFORE any mutation (APPLY-04 / plan: "simulate edges for
    /// acyclicity").
    ///
    /// `wire_template_edges` adds edges one at a time, so a cycle introduced by a
    /// LATER edge would only surface after nodes and earlier edges are persisted —
    /// a partial apply. Here the complete prospective graph is built over the
    /// current store plus PLACEHOLDER ids for the to-be-created role nodes, and
    /// checked once with [`DependencyGraph::validate_dag`]. The prospective edges
    /// mirror `wire_template_edges` exactly:
    ///
    /// - internal `depends_on`: role placeholder → dep-role placeholder;
    /// - anchor edges: bound anchor existing id → role placeholder;
    /// - `move-upstream-to-role`: the target role placeholder gains the container's
    ///   pre-apply snapshot deps, and the container ANCHOR loses them (the
    ///   transform's removal — modeled so a cycle the removal actually breaks is
    ///   not falsely flagged).
    ///
    /// Extra (un-reduced) edges only make a cycle MORE likely, so an acyclic
    /// prospective graph guarantees the reduced result `add_dependency` produces is
    /// acyclic too. On a cycle, returns a clear error before the first
    /// `create_issue`.
    fn prevalidate_acyclic(
        &self,
        template: &GraphTemplate,
        full_container_id: &str,
        resolved_bindings: &BTreeMap<String, String>,
        anchor_dependency_snapshots: &BTreeMap<String, Vec<String>>,
    ) -> Result<()> {
        // Stable placeholder id per role for the not-yet-created nodes. Prefixed
        // so it cannot collide with a real (uuid) issue id in the store.
        let placeholder = |role: &str| format!("__apply_placeholder__:{role}");

        // Start from the current store's edges, keyed by id → deps, so we can apply
        // the transform's REMOVALS without mutating stored issues.
        let store_issues = self.storage.list_issues()?;
        let mut deps_by_id: BTreeMap<String, Vec<String>> = store_issues
            .iter()
            .map(|i| (i.id.clone(), i.dependencies.clone()))
            .collect();

        // The container anchor (the bound anchor resolving to this container) is
        // the one whose snapshot the `move-upstream-to-role` transform moves.
        let container_anchor = resolved_bindings
            .iter()
            .find(|(_, id)| id.as_str() == full_container_id)
            .map(|(name, _)| name.clone());

        // Placeholder nodes for the created roles, with their internal depends_on
        // edges (role → dep-role placeholder).
        for node in &template.nodes {
            let role_deps: Vec<String> = node.depends_on.iter().map(|r| placeholder(r)).collect();
            deps_by_id.insert(placeholder(&node.role), role_deps);
        }

        // Anchor edges: the bound anchor gains a dep on the created role node.
        for edge in &template.anchor_edges {
            let Some(anchor_id) = resolved_bindings.get(&edge.from) else {
                continue; // unbound anchors are rejected earlier; skip defensively
            };
            deps_by_id
                .entry(anchor_id.clone())
                .or_default()
                .push(placeholder(&edge.to));
        }

        // Transforms: move-upstream-to-role moves the container's snapshot deps
        // onto the target role placeholder and removes them from the container.
        for transform in &template.transforms {
            match TransformKind::parse(&transform.kind)? {
                TransformKind::MoveUpstreamToRole => {
                    let snapshot = container_anchor
                        .as_deref()
                        .and_then(|name| anchor_dependency_snapshots.get(name))
                        .cloned()
                        .unwrap_or_default();
                    // Role placeholder gains each snapshot dep.
                    deps_by_id
                        .entry(placeholder(&transform.role))
                        .or_default()
                        .extend(snapshot.iter().cloned());
                    // Container loses each snapshot dep (the transform's removal).
                    if let Some(container_deps) = deps_by_id.get_mut(full_container_id) {
                        container_deps.retain(|d| !snapshot.contains(d));
                    }
                }
            }
        }

        // Build the prospective graph and check it is a DAG. A dep that names an
        // id outside the node set simply has no outgoing edges during traversal
        // (the DAG check tolerates missing targets), so the structure that matters
        // for cycle detection is fully represented.
        let nodes: Vec<ProspectiveNode> = deps_by_id
            .into_iter()
            .map(|(id, dependencies)| ProspectiveNode { id, dependencies })
            .collect();
        let refs: Vec<&ProspectiveNode> = nodes.iter().collect();
        let graph = crate::graph::DependencyGraph::new(&refs);
        if graph.validate_dag().is_err() {
            return Err(anyhow!(
                "applying template '{}' to container {full_container_id} would create a \
                 dependency cycle; no nodes were created",
                template.name
            ));
        }
        Ok(())
    }

    /// Build the FINAL persisted [`Issue`] shape a node's `create_issue` would
    /// produce, for read-only pre-validation. Mirrors `create_issue`'s
    /// construction for a node that always carries a `type:` label and has no
    /// dependencies at creation: fields set, then auto-promoted to
    /// [`State::Ready`] (so a state-keyed rule sees the persisted shape).
    fn project_node_issue(
        &self,
        node: &TemplateNode,
        container: &Issue,
        inherited: &[String],
        node_context: &InterpolationContext,
    ) -> Issue {
        let mut issue = Issue::new(
            node_title(node, container),
            node_description(node, node_context),
        );
        issue.priority = container.priority;
        issue.labels = node_labels(node, inherited, node_context);
        // A freshly-created issue with no dependencies is auto-promoted to Ready
        // by `create_issue`; replicate so state-keyed rules see the same shape.
        issue.state = State::Ready;
        issue
    }

    /// Instantiate every template node fresh: create the typed issue and attach
    /// its gate presets. Returns role → created id.
    fn instantiate_template_nodes(
        &self,
        template: &GraphTemplate,
        container: &Issue,
        context: &InterpolationContext,
        warnings: &mut Vec<String>,
    ) -> Result<BTreeMap<String, String>> {
        let inherited = inherited_membership_labels(container);

        let mut created: BTreeMap<String, String> = BTreeMap::new();
        for node in &template.nodes {
            let node_context = context.with_doc(node);
            let description = node_description(node, &node_context);
            let labels = node_labels(node, &inherited, &node_context);
            let title = node_title(node, container);

            let (node_id, mut create_warnings) = self.create_issue(
                title,
                description,
                container.priority,
                vec![],
                labels,
                None,
                None,
                false,
            )?;
            warnings.append(&mut create_warnings);

            self.attach_template_gates(&node.gates, &node_id, warnings)?;

            created.insert(node.role.clone(), node_id);
        }
        Ok(created)
    }

    /// Wire the template's edges, then run its transforms, over the freshly
    /// created nodes (jit:73e5e853 — APPB-01 / APPB-02).
    ///
    /// In order:
    ///
    /// 1. **Internal `depends_on` edges (by role):** for each node, for each role
    ///    it depends on, wire `node → dep_node` (e.g. breakdown
    ///    `depends_on = ["planning"]` → `B → P`).
    /// 2. **Anchor edges:** for each `{from: anchor, to: role}`, wire the bound
    ///    anchor depending on the created node (`C → B` for `container → breakdown`).
    /// 3. **Transforms:** dispatched by `kind`. `move-upstream-to-role` moves the
    ///    container's PRE-APPLY snapshot deps onto the named role's node.
    ///
    /// Every edge goes through [`add_dependency`](Self::add_dependency), which runs
    /// cycle detection and eager transitive reduction, so the resulting graph is
    /// acyclic and transitively reduced (APPB-01). Transforms read the snapshot,
    /// not live deps, so a freshly-wired anchor edge cannot be moved (APPB-02).
    fn wire_template_edges(
        &self,
        template: &GraphTemplate,
        full_container_id: &str,
        resolved_bindings: &BTreeMap<String, String>,
        created: &BTreeMap<String, String>,
        anchor_dependency_snapshots: &BTreeMap<String, Vec<String>>,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        // 1. Internal depends_on edges (node → dep-role node).
        for node in &template.nodes {
            let Some(node_id) = created.get(&node.role) else {
                continue;
            };
            for dep_role in &node.depends_on {
                let dep_id = created.get(dep_role).ok_or_else(|| {
                    anyhow!(
                        "template '{}' node '{}' depends_on role '{dep_role}', \
                         which was not created",
                        template.name,
                        node.role
                    )
                })?;
                let (_, mut w) = self.add_dependency(node_id, dep_id)?;
                warnings.append(&mut w);
            }
        }

        // 2. Anchor edges (bound anchor → created node): "anchor depends on node".
        for edge in &template.anchor_edges {
            let anchor_id = resolved_bindings.get(&edge.from).ok_or_else(|| {
                anyhow!(
                    "template '{}' anchor_edge references unbound anchor '{}'",
                    template.name,
                    edge.from
                )
            })?;
            let node_id = created.get(&edge.to).ok_or_else(|| {
                anyhow!(
                    "template '{}' anchor_edge points to role '{}', which was not created",
                    template.name,
                    edge.to
                )
            })?;
            let (_, mut w) = self.add_dependency(anchor_id, node_id)?;
            warnings.append(&mut w);
        }

        // 3. Transforms, dispatched by kind (extensible). Kinds were already
        //    validated in the precondition phase, so this parse cannot reach a
        //    caller as a partial-apply error; it stays the single dispatch point.
        for transform in &template.transforms {
            match TransformKind::parse(&transform.kind)? {
                TransformKind::MoveUpstreamToRole => {
                    self.move_upstream_to_role(
                        template,
                        full_container_id,
                        &transform.role,
                        created,
                        resolved_bindings,
                        anchor_dependency_snapshots,
                        warnings,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// `move-upstream-to-role`: move the container's PRE-APPLY upstream deps onto
    /// the node created for `role` (snapshot deps, wire each onto the role node,
    /// then drop them from the container).
    ///
    /// Operates on the step-2 snapshot — taken before any mutation, so it cannot
    /// contain the freshly-wired anchor/scaffold nodes — never a live read of the
    /// container's deps (APPB-02). `add_dependency` then keeps the graph acyclic
    /// and transitively reduced (APPB-01).
    #[allow(clippy::too_many_arguments)]
    fn move_upstream_to_role(
        &self,
        template: &GraphTemplate,
        full_container_id: &str,
        role: &str,
        created: &BTreeMap<String, String>,
        resolved_bindings: &BTreeMap<String, String>,
        anchor_dependency_snapshots: &BTreeMap<String, Vec<String>>,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        let role_node_id = created.get(role).ok_or_else(|| {
            anyhow!(
                "template '{}' transform targets role '{role}', which was not created",
                template.name
            )
        })?;

        // The container's pre-apply snapshot: the anchor whose bound id is the
        // container. The loader guarantees a transform's role is declared; the
        // container anchor is the one resolving to this container.
        let container_anchor = resolved_bindings
            .iter()
            .find(|(_, id)| id.as_str() == full_container_id)
            .map(|(name, _)| name.as_str());
        let original_deps = container_anchor
            .and_then(|name| anchor_dependency_snapshots.get(name))
            .cloned()
            .unwrap_or_default();

        // Move each pre-apply upstream dep onto the role node, then drop it from
        // the container. Wire the new edges before removing the old ones so
        // transitive reduction never strands an edge mid-operation (plan.rs order).
        for dep_id in &original_deps {
            let (_, mut w) = self.add_dependency(role_node_id, dep_id)?;
            warnings.append(&mut w);
        }
        for dep_id in &original_deps {
            let mut w = self.remove_dependency(full_container_id, dep_id)?;
            warnings.append(&mut w);
        }
        Ok(())
    }

    /// Refresh an already-applied template's nodes IN PLACE (the `--force` path).
    ///
    /// Locates each role's existing node from the breakdown node found by
    /// [`find_applied_breakdown`](Self::find_applied_breakdown): the breakdown
    /// node itself, and every node reached through the breakdown node's template
    /// `depends_on` (e.g. the planning node via `B → P`). Re-interpolates each
    /// node's description / doc against the current container and writes them back
    /// without creating duplicate nodes. Gate presets are NOT re-applied
    /// (idempotent attach is the instantiate path's job; a refresh only re-seeds
    /// prose).
    ///
    /// Every template role MUST map to an existing issue: a bracket that has lost
    /// its planning node (or any role) is broken, and refreshing it partially
    /// would silently report success while leaving stale prose. Such a case
    /// returns an error rather than a partial result (APPA-03).
    fn refresh_template_nodes(
        &self,
        template: &GraphTemplate,
        breakdown_id: &str,
        context: &InterpolationContext,
    ) -> Result<BTreeMap<String, String>> {
        let mut existing: BTreeMap<String, String> = BTreeMap::new();
        existing.insert(BREAKDOWN_ROLE.to_string(), breakdown_id.to_string());

        // Reach the breakdown node's template `depends_on` roles through the
        // persisted breakdown issue's dependencies, matching each role's node by
        // its `type:` label. The plan template wires `B → P`, so this resolves P.
        if let Some(breakdown_node) = template.node(BREAKDOWN_ROLE) {
            let breakdown_node_issue = self.storage.load_issue(breakdown_id)?;
            for dep_role in &breakdown_node.depends_on {
                if let Some(dep_node) = template.node(dep_role) {
                    if let Some(dep_id) =
                        self.find_dep_by_type(&breakdown_node_issue, &dep_node.type_name)?
                    {
                        existing.insert(dep_role.clone(), dep_id);
                    }
                }
            }
        }

        // Every template role must have been located, or the existing bracket is
        // broken/incomplete: fail rather than refresh a subset and report success.
        if let Some(missing) = template
            .nodes
            .iter()
            .find(|n| !existing.contains_key(&n.role))
        {
            return Err(anyhow!(
                "cannot --force refresh template '{}': its '{}' node could not be located \
                 from the existing bracket (the applied bracket is broken or incomplete); \
                 the bracket must be repaired before it can be refreshed",
                template.name,
                missing.role
            ));
        }

        for node in &template.nodes {
            // The completeness check above guarantees every role is present, so
            // this branch is unreachable in practice; return a contextual error
            // rather than panic (library code must not `expect`).
            let node_id = existing.get(&node.role).cloned().ok_or_else(|| {
                anyhow!(
                    "internal error: template '{}' role '{}' was not located during \
                     --force refresh despite passing the completeness check",
                    template.name,
                    node.role
                )
            })?;
            let node_context = context.with_doc(node);

            let mut issue = self.storage.load_issue(&node_id)?;
            issue.description = node_description(node, &node_context);
            self.storage.save_issue(issue)?;
            self.storage
                .append_event(&crate::domain::Event::new_issue_updated(
                    node_id.clone(),
                    "agent:apply".to_string(),
                    vec!["description".to_string()],
                ))?;
        }
        Ok(existing)
    }

    /// Resolve a template node/anchor gate `name` to either a registered gate
    /// PRESET bundle or a single gate KEY declared in the gate registry
    /// (`.jit/gates.json`), PREFERRING the preset.
    ///
    /// Config-declared gates (e.g. `repo-validate`) are usable from template
    /// anchors/nodes this way without being built-in presets. Returns an error
    /// when the name is neither, so callers keep the existing hard failure for an
    /// unknown gate. Pure read; performs no mutation.
    fn resolve_template_gate(&self, name: &str) -> Result<TemplateGateResolution> {
        // Prefer a preset bundle; fall back to a single registry gate key ONLY
        // when `name` is genuinely not a known preset. Any OTHER error from
        // preset loading (e.g. a malformed custom preset under
        // `.jit/config/gate-presets/`) must propagate with context rather than be
        // silently treated as "not a preset" — otherwise a broken preset config
        // could let `jit apply` succeed by accidentally matching a registry key.
        match self.storage.get_gate_preset(name) {
            Ok(_) => return Ok(TemplateGateResolution::Preset),
            Err(e)
                if e.downcast_ref::<crate::storage::PresetNotFoundError>()
                    .is_some() =>
            {
                // Not a registered preset — fall through to the registry-key lookup.
            }
            Err(e) => return Err(e),
        }
        if self.storage.load_gate_registry()?.gates.contains_key(name) {
            return Ok(TemplateGateResolution::RegistryKey);
        }
        Err(anyhow!(
            "gate '{name}' is neither a registered gate preset nor a gate defined in the registry"
        ))
    }

    /// Attach each named template gate to `issue_id`, collecting any lease
    /// warnings. Each name resolves through
    /// [`resolve_template_gate`](Self::resolve_template_gate) to either a gate
    /// PRESET (applied as a bundle via `apply_gate_preset`) or a single gate KEY
    /// in the registry (attached via `add_gates`, the same effect as
    /// `jit gate add <issue> <key>`). Shared by node- and anchor-gate attachment
    /// (jit:2614ecf2 — REQ-13) so both flow through the same resolution.
    fn attach_template_gates(
        &self,
        gates: &[String],
        issue_id: &str,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        for gate in gates {
            let mut w = match self.resolve_template_gate(gate)? {
                TemplateGateResolution::Preset => {
                    self.apply_gate_preset(issue_id, gate, None, false, false, &[])?
                        .1
                }
                TemplateGateResolution::RegistryKey => {
                    self.add_gates(issue_id, std::slice::from_ref(gate))?.1
                }
            };
            warnings.append(&mut w);
        }
        Ok(())
    }

    /// Attach every gate declared on each template anchor to its bound anchor
    /// issue, through the shared [`attach_template_gates`](Self::attach_template_gates)
    /// path node gates use (jit:2614ecf2 — REQ-13). Each anchor gate resolves as a
    /// preset OR a registry gate key; existence is validated in the precondition
    /// phase, mirroring node gates, so this only runs on the fresh-apply path after
    /// the bound anchor's template edges are wired.
    fn attach_anchor_gates(
        &self,
        template: &GraphTemplate,
        resolved_bindings: &BTreeMap<String, String>,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        for anchor in &template.anchors {
            if anchor.gates.is_empty() {
                continue;
            }
            let anchor_id = resolved_bindings.get(&anchor.name).ok_or_else(|| {
                anyhow!(
                    "template '{}' anchor '{}' declares gates but has no resolved binding",
                    template.name,
                    anchor.name
                )
            })?;
            self.attach_template_gates(&anchor.gates, anchor_id, warnings)?;
        }
        Ok(())
    }

    /// Locate an already-applied template's breakdown node `B` for `container`:
    /// the issue carrying the breakdown node's `type:` label AND the
    /// `brackets:<container-short-id>` label the template seeds onto it.
    ///
    /// Returns the breakdown node's full id, or `None` when the template has not
    /// been applied (no such issue). Only `B` carries the `brackets:<C-short-id>` label, so
    /// the pair uniquely identifies an applied bracket.
    ///
    /// The lookup is **store-wide**, not limited to `container.dependencies`: a
    /// fresh apply wires `C → B` directly, but a subsequent breakdown splices the
    /// spine `C → sink … → B` and relies on transitive reduction to DROP the direct
    /// `C → B` edge — after which `B` is still in `C`'s closure but no longer a
    /// direct dependency. Scanning only direct deps would then miss `B` and let
    /// `--force` take the fresh-apply path, duplicating `P` + `B`. Matching by the
    /// unique label pair across the store finds `B` regardless of edge distance.
    fn find_applied_breakdown(
        &self,
        template: &GraphTemplate,
        container: &Issue,
    ) -> Result<Option<String>> {
        let Some(breakdown_node) = template.node(BREAKDOWN_ROLE) else {
            return Ok(None);
        };
        let bracket_label = format!("brackets:{}", container.short_id());
        let found = self.storage.list_issues()?.into_iter().find(|issue| {
            label_utils::type_label_value(&issue.labels) == Some(breakdown_node.type_name.as_str())
                && issue.labels.iter().any(|l| l == &bracket_label)
        });
        Ok(found.map(|issue| issue.id))
    }

    /// Find the first dependency of `issue` carrying the given `type:` label.
    fn find_dep_by_type(&self, issue: &Issue, type_name: &str) -> Result<Option<String>> {
        for dep_id in &issue.dependencies {
            let dep = self.storage.load_issue(dep_id)?;
            if label_utils::type_label_value(&dep.labels) == Some(type_name) {
                return Ok(Some(dep.id));
            }
        }
        Ok(None)
    }
}

/// A synthetic graph node for the prospective-cycle simulation: an id and its
/// dependency ids, implementing [`GraphNode`](crate::graph::GraphNode) so the
/// prospective post-apply graph (store issues + placeholder role nodes) can be
/// checked with the existing [`DependencyGraph`](crate::graph::DependencyGraph)
/// without mutating any stored issue.
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

/// A recognized graph-transform kind, parsed from a [`Transform::kind`] string.
///
/// Dispatch is by `kind` so the transform set stays extensible; only
/// `move-upstream-to-role` ships this epic. An unknown kind is rejected with a
/// clear error rather than silently ignored.
///
/// [`Transform::kind`]: crate::templates::Transform::kind
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransformKind {
    /// Move the container's pre-apply upstream deps onto a named role's node.
    MoveUpstreamToRole,
}

impl TransformKind {
    /// The `kind` string of `move-upstream-to-role`.
    const MOVE_UPSTREAM_TO_ROLE: &'static str = "move-upstream-to-role";

    /// Parse a transform `kind` string, erroring on an unrecognized kind.
    fn parse(kind: &str) -> Result<Self> {
        match kind {
            Self::MOVE_UPSTREAM_TO_ROLE => Ok(Self::MoveUpstreamToRole),
            other => Err(anyhow!(
                "unknown graph transform kind '{other}'; supported kinds: {}",
                Self::MOVE_UPSTREAM_TO_ROLE
            )),
        }
    }
}

/// The container membership labels every created node inherits: all of the
/// container's labels EXCEPT its own `type:` label (which each node replaces with
/// its own). Pure helper, shared by the
/// pre-validation and instantiation paths so both see the identical label set.
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
/// `labels`. Single source of truth shared by pre-validation and instantiation,
/// so the shape pre-validated is exactly the shape persisted.
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

/// The title a node's created issue carries (`<role>: <container title>`). Shared
/// so pre-validation and instantiation agree on the persisted shape.
fn node_title(node: &TemplateNode, container: &Issue) -> String {
    format!("{}: {}", node.role, container.title)
}

/// Compute a template node's final, GUARANTEED non-empty interpolated description.
///
/// A node with an explicit `description` template has its `{...}` tokens
/// resolved; a node with none, or whose template is absent/blank or interpolates
/// to whitespace-only, falls back to a generic role/title line so the created
/// issue is never seeded with an empty body (APPA-02).
fn node_description(node: &TemplateNode, context: &InterpolationContext) -> String {
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
/// simple `{token}` replace over a fixed map, NOT a templating language.
#[derive(Debug, Clone)]
struct InterpolationContext {
    id: String,
    short_id: String,
    title: String,
    hard_criteria: String,
    doc: Option<String>,
}

impl InterpolationContext {
    /// Build the container-derived context (the `{doc}` token is unset until a
    /// node is selected via [`with_doc`](Self::with_doc)).
    fn for_container(container: &Issue) -> Self {
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
    fn with_doc(&self, node: &TemplateNode) -> Self {
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

    #[test]
    fn test_type_label_value_extracts_type() {
        let issue = Issue::new_with_labels(
            "T".to_string(),
            String::new(),
            vec!["type:epic".to_string(), "area:auth".to_string()],
        );
        assert_eq!(label_utils::type_label_value(&issue.labels), Some("epic"));
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
        let mut issue = Issue::new("Auth epic".to_string(), "- [hard] REQ-01: x".to_string());
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
        let issue = Issue::new("Epic X".to_string(), String::new());
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
        let issue = Issue::new("Epic Y".to_string(), String::new());
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
    fn test_transform_kind_parse_accepts_move_upstream_to_role() {
        // The one shipped kind parses to its variant.
        assert_eq!(
            TransformKind::parse("move-upstream-to-role").unwrap(),
            TransformKind::MoveUpstreamToRole
        );
    }

    #[test]
    fn test_transform_kind_parse_rejects_unknown_kind() {
        // An unrecognized kind errors (it is never silently ignored), and the
        // message names both the offending kind and the supported set so the
        // misconfiguration is actionable.
        let err = TransformKind::parse("teleport").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("teleport"), "{msg}");
        assert!(msg.contains("move-upstream-to-role"), "{msg}");
    }

    #[test]
    fn test_interpolate_leaves_unknown_token_verbatim() {
        // The interpolation context is a fixed `{token}` substitution, not a
        // templating language: an unsupported `{token}` is left untouched while the
        // known container tokens around it still resolve.
        let mut issue = Issue::new("Auth epic".to_string(), String::new());
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
        let mut container = Issue::new_with_labels(
            "Auth epic".to_string(),
            String::new(),
            vec!["type:epic".to_string(), "area:auth".to_string()],
        );
        container.id = "abc123def456".to_string();
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
