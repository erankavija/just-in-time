//! Graph-template apply engine — validate, snapshot, and instantiate
//! (W2 task 1 of the `jit apply` engine).
//!
//! [`apply_template`](CommandExecutor::apply_template) instantiates a
//! [`GraphTemplate`](crate::templates::GraphTemplate) onto a container: it
//! validates every non-edge precondition BEFORE the first mutation, snapshots
//! each bound anchor's current dependencies, then creates the template's nodes
//! with interpolated descriptions / docs and their declared gate presets.
//!
//! This module generalizes the create+gate+doc sequence that
//! [`bracket_container`](super::plan) performs for the hardcoded planning
//! bracket. It deliberately stops short of edge wiring and graph transforms:
//!
//! - **internal `depends_on` edges** (e.g. `B → P`),
//! - **`anchor_edges`** (e.g. `C → B`), and
//! - the **`move-upstream-to-role`** transform that moves the container's
//!   pre-apply upstream deps onto the planning node,
//!
//! are the next task's responsibility (W2 task 2, jit:73e5e853). The snapshot
//! captured here (the anchors' pre-apply dependency sets) is exactly what that
//! transform consumes, so it is carried out of this engine on
//! [`TemplateApplyResult`].
//!
//! # Domain-agnostic
//!
//! No `epic` / `planning` / `breakdown` literal is hardcoded. Node types, gate
//! presets, doc locations, descriptions, and labels all come from the template;
//! the only roles this engine reaches for by name are the conventional
//! [`PLANNING_ROLE`](crate::templates::PLANNING_ROLE) /
//! [`BREAKDOWN_ROLE`](crate::templates::BREAKDOWN_ROLE), and only on the
//! `--force` refresh path to locate already-applied nodes.

use std::collections::BTreeMap;

use super::*;
use crate::domain::DocumentReference;
use crate::templates::{GraphTemplate, TemplateNode, BREAKDOWN_ROLE};
use serde::Serialize;

/// Outcome of applying a graph template to a container.
///
/// Names the template applied and the anchor bindings used, maps each created
/// node's template ROLE to the id of the issue created (or refreshed) for it,
/// and carries the PRE-APPLY snapshot of each bound anchor's dependencies. The
/// snapshot is the input the edge-wiring + `move-upstream-to-role` transform
/// (the next task) consumes; capturing it before any mutation is what lets that
/// transform move exactly the container's ORIGINAL upstream deps.
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
    /// mutation. Consumed by the edge-wiring + transform task (jit:73e5e853).
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
    /// (mirroring the `*_with_config` split in [`plan`](super::plan)). Steps:
    ///
    /// 1. **Validate before mutating.** The container type is in the template's
    ///    `applies_to`; every declared anchor is bound and resolves to an
    ///    existing issue; the container is not already-applied unless `force`.
    ///    Any failure aborts BEFORE the first `create_issue`, so a validation
    ///    failure creates zero nodes.
    /// 2. **Snapshot** each bound anchor's `dependencies` (carried on the result
    ///    for the edge-wiring + transform task).
    /// 3. **Instantiate** each template node in order (or refresh in place under
    ///    `force`): create the typed issue with inherited container membership
    ///    labels plus the node's interpolated labels and a non-empty interpolated
    ///    description, set the node's interpolated `doc` as a [`DocumentReference`],
    ///    and attach each declared gate preset.
    ///
    /// It does NOT wire internal/anchor edges or run transforms — see the module
    /// docs and the seam left in this method.
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
        let container_type = type_label_value(&container.labels);
        match container_type.as_deref() {
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

        // Every gate preset declared across the template's nodes must resolve in
        // the preset registry BEFORE the first mutation. `apply_gate_preset`
        // resolves a preset lazily during instantiation, so an unknown name would
        // otherwise fail AFTER one or more nodes are persisted — violating "a
        // failure creates zero nodes" (APPA-01). Look it up read-only up front.
        for node in &template.nodes {
            for preset in &node.gates {
                self.storage.get_gate_preset(preset).with_context(|| {
                    format!(
                        "template '{}' node '{}' references gate preset '{preset}', \
                         which is not registered",
                        template.name, node.role
                    )
                })?;
            }
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
        }

        let mut warnings = Vec::new();
        let created_node_ids_by_role = if let Some(breakdown_id) = existing_breakdown {
            // === Force refresh: update existing nodes in place, no duplicates ===
            self.refresh_template_nodes(template, &breakdown_id, &context, &mut warnings)?
        } else {
            // === 3. Instantiate nodes (fresh apply) ===
            self.instantiate_template_nodes(template, &container, &context, &mut warnings)?
        };

        // === SEAM: W2 task 2 (jit:73e5e853) — wire edges + run transforms here ===
        // The next task wires the template's internal `depends_on` edges (by
        // role) and `anchor_edges` (anchor↔node) via `add_dependency`, then runs
        // the `move-upstream-to-role` transform over `anchor_dependency_snapshots`
        // (NOT a live re-read of the container's deps). It consumes
        // `created_node_ids_by_role` + `anchor_dependency_snapshots` from the
        // result below. This engine intentionally creates only the NODES.

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
            self.validate_for_write(&projected, false)
                .with_context(|| {
                    format!(
                        "template '{}' node '{}' would create an invalid issue",
                        template.name, node.role
                    )
                })?;
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

    /// Instantiate every template node fresh: create the typed issue, set its
    /// interpolated doc, and attach its gate presets. Returns role → created id.
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
                false,
            )?;
            warnings.append(&mut create_warnings);

            self.set_node_doc(node, &node_id, &node_context, warnings)?;
            self.attach_node_gates(node, &node_id, warnings)?;

            created.insert(node.role.clone(), node_id);
        }
        Ok(created)
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
        warnings: &mut [String],
    ) -> Result<BTreeMap<String, String>> {
        let mut existing: BTreeMap<String, String> = BTreeMap::new();
        existing.insert(BREAKDOWN_ROLE.to_string(), breakdown_id.to_string());

        // Reach the breakdown node's template `depends_on` roles through the
        // persisted breakdown issue's dependencies, matching each role's node by
        // its `type:` label. The plan template wires `B → P`, so this resolves P.
        if let Some(breakdown_node) = template.node(BREAKDOWN_ROLE) {
            let breakdown_issue = self.storage.load_issue(breakdown_id)?;
            for dep_role in &breakdown_node.depends_on {
                if let Some(dep_node) = template.node(dep_role) {
                    if let Some(dep_id) =
                        self.find_dep_by_type(&breakdown_issue, &dep_node.type_name)?
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

            self.set_node_doc(node, &node_id, &node_context, warnings)?;
        }
        Ok(existing)
    }

    /// Set a node's interpolated `doc` as a `plan`-labeled [`DocumentReference`].
    ///
    /// A node with no `doc` template (or one resolving to the inline sentinel) is
    /// left as its own body — nothing is attached. Otherwise the interpolated path
    /// is recorded as a document reference (replacing any prior `plan`-labeled ref
    /// so a `--force` refresh does not accumulate duplicates) and the change is
    /// audited.
    fn set_node_doc(
        &self,
        node: &TemplateNode,
        node_id: &str,
        node_context: &InterpolationContext,
        _warnings: &mut [String],
    ) -> Result<()> {
        let Some(doc_template) = node.doc.as_deref() else {
            return Ok(());
        };
        let resolved = node_context.interpolate(doc_template);
        if resolved == plan_doc::INLINE_LOCATION || resolved.is_empty() {
            return Ok(());
        }

        let mut issue = self.storage.load_issue(node_id)?;
        // Drop any prior plan-labeled doc ref so refresh replaces rather than
        // appends, then record the freshly-resolved one.
        issue
            .documents
            .retain(|d| d.label.as_deref() != Some("plan"));
        issue
            .documents
            .push(DocumentReference::new(resolved).with_label("plan".to_string()));
        self.storage.save_issue(issue)?;
        self.storage
            .append_event(&crate::domain::Event::new_issue_updated(
                node_id.to_string(),
                "agent:apply".to_string(),
                vec!["documents".to_string()],
            ))?;
        Ok(())
    }

    /// Attach every gate preset declared on a template node to the created issue.
    fn attach_node_gates(
        &self,
        node: &TemplateNode,
        node_id: &str,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        for preset in &node.gates {
            let (_, mut w) = self.apply_gate_preset(node_id, preset, None, false, false, &[])?;
            warnings.append(&mut w);
        }
        Ok(())
    }

    /// Locate an already-applied template's breakdown node among the container's
    /// dependencies: the dep carrying the breakdown node's `type:` label AND the
    /// `brackets:<container-short-id>` label the template seeds onto it.
    ///
    /// Returns the breakdown node's full id, or `None` when the template has not
    /// been applied (no such dep). Only the breakdown node carries a `brackets:`
    /// label, so this uniquely identifies an applied bracket.
    fn find_applied_breakdown(
        &self,
        template: &GraphTemplate,
        container: &Issue,
    ) -> Result<Option<String>> {
        let Some(breakdown_node) = template.node(BREAKDOWN_ROLE) else {
            return Ok(None);
        };
        let bracket_label = format!("brackets:{}", container.short_id());
        for dep_id in &container.dependencies {
            let dep = self.storage.load_issue(dep_id)?;
            let has_type =
                type_label_value(&dep.labels).as_deref() == Some(&breakdown_node.type_name);
            let has_bracket = dep.labels.iter().any(|l| l == &bracket_label);
            if has_type && has_bracket {
                return Ok(Some(dep.id));
            }
        }
        Ok(None)
    }

    /// Find the first dependency of `issue` carrying the given `type:` label.
    fn find_dep_by_type(&self, issue: &Issue, type_name: &str) -> Result<Option<String>> {
        for dep_id in &issue.dependencies {
            let dep = self.storage.load_issue(dep_id)?;
            if type_label_value(&dep.labels).as_deref() == Some(type_name) {
                return Ok(Some(dep.id));
            }
        }
        Ok(None)
    }
}

/// The container membership labels every created node inherits: all of the
/// container's labels EXCEPT its own `type:` label (which each node replaces with
/// its own), as `bracket_container` does today. Pure helper, shared by the
/// pre-validation and instantiation paths so both see the identical label set.
fn inherited_membership_labels(container: &Issue) -> Vec<String> {
    container
        .labels
        .iter()
        .filter(|l| {
            label_utils::parse_label(l)
                .map(|(ns, _)| ns != "type")
                .unwrap_or(true)
        })
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
    labels.push(format!("type:{}", node.type_name));
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

/// Pull the `type:*` value out of a label list, if present (pure helper).
fn type_label_value(labels: &[String]) -> Option<String> {
    labels.iter().find_map(|l| {
        label_utils::parse_label(l)
            .ok()
            .and_then(|(ns, v)| (ns == "type").then_some(v))
    })
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
        assert_eq!(type_label_value(&issue.labels).as_deref(), Some("epic"));
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
}
