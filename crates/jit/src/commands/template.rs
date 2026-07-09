//! Graph-template apply engine (the `jit apply` engine).
//!
//! [`apply_template`](CommandExecutor::apply_template) instantiates a
//! [`GraphTemplate`](crate::templates::GraphTemplate) onto a container in two
//! phases with a hard boundary between them:
//!
//! 1. **Expand** — [`expand_template`](crate::commands::expand_template) turns
//!    the template, a container snapshot, and the bound anchors' pre-apply
//!    dependency snapshots into a [`TemplateDelta`]: the issues to create, the
//!    edges to add, the edges to remove, and the anchor gates to attach. Pure:
//!    no storage access.
//! 2. **Commit** — the executor validates the delta (gates resolve, each
//!    projected issue would pass write validation, the prospective graph is
//!    acyclic) and then commits it, all while holding ONE repository write lock:
//!    [`IssueStore::acquire_repo_write_lock`](crate::storage::IssueStore::acquire_repo_write_lock),
//!    the outermost lock of every ordinary issue/dependency write, so no other
//!    writer can interleave with the apply. A validation or write failure inside
//!    the lock leaves the issue store as it was: created nodes are deleted and
//!    mutated issues are reverted from a pre-mutation snapshot before the error is
//!    returned. Because no concurrent writer could land inside the window, the
//!    rollback can only undo writes the apply itself made.
//!
//! Committing the delta produces the plan-before-fan-out scaffold: the
//! `C → B → P` bracket. Nodes are created with interpolated descriptions and
//! their declared gates; the planning node's `doc` template seeds its description
//! with the plan-doc location to author and link (the engine attaches no document
//! reference, so apply never leaves a reference to a not-yet-created file). Edges
//! go through [`add_dependency`](CommandExecutor::add_dependency), so the result
//! is acyclic and transitively reduced, and every edge write emits the same
//! events as the ordinary dependency path.
//!
//! The `--force` refresh path re-seeds existing nodes' prose in place and commits
//! no edges or transforms (the spine already exists; re-running the transform
//! over now-scaffold-bearing live deps would corrupt it).
//!
//! # Domain-agnostic
//!
//! No `epic` / `planning` / `breakdown` literal is hardcoded. Node types, gates
//! (preset or registry key), doc locations, descriptions, and labels all come
//! from the template; the only roles this engine reaches for by name are the
//! conventional [`PLANNING_ROLE`](crate::templates::PLANNING_ROLE) /
//! [`BREAKDOWN_ROLE`](crate::templates::BREAKDOWN_ROLE), and only on the
//! `--force` refresh path to locate already-applied nodes.

use std::collections::BTreeMap;

use super::template_expand::{
    expand_template, node_description, validate_delta_acyclic, DeltaEndpoint, InterpolationContext,
    PlannedNode, TemplateDelta,
};
use super::*;
use crate::templates::{GraphTemplate, BREAKDOWN_ROLE};
use serde::Serialize;

/// Actor recorded on the events the apply engine appends directly.
const APPLY_ACTOR: &str = "agent:apply";

/// How a template node/anchor gate NAME resolves: a registered gate PRESET
/// bundle, or a single gate KEY declared in the gate registry (`.jit/gates.toml`).
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
    /// Separated so the engine is testable without an on-disk `templates.toml`.
    /// The whole call runs under ONE repository write lock — the one every
    /// ordinary writer takes — so a concurrent writer observes the store before
    /// the apply or after it, never midway. Steps:
    ///
    /// 1. **Resolve** — the container type is in the template's `applies_to`;
    ///    every declared anchor is bound and resolves to an existing issue; each
    ///    bound anchor's `dependencies` are snapshotted.
    /// 2. **Expand** — [`expand_template`] turns the template plus those snapshots
    ///    into a [`TemplateDelta`], purely.
    /// 3. **Validate the delta** — every node AND anchor gate resolves (as a gate
    ///    preset OR a registry gate key), every projected node write would pass
    ///    validation, the prospective post-apply graph is acyclic, and the
    ///    container is not already-applied unless `force`. Any failure aborts
    ///    BEFORE the first write.
    /// 4. **Commit the delta** — create the nodes with their gates, wire the
    ///    `add_edges` and drop the `remove_edges` via
    ///    [`add_dependency`](Self::add_dependency) /
    ///    [`remove_dependency`](Self::remove_dependency) (cycle-checked,
    ///    transitively reduced, event-emitting), then attach the anchor gates. A
    ///    failure at any point restores the pre-mutation issue snapshot before the
    ///    error is returned.
    ///
    /// Under `force`, an already-applied container takes the refresh path instead:
    /// each existing node's prose is re-seeded in place, with the same
    /// restore-on-failure guarantee.
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
        // One repository write lock for the whole apply: the reads that validation
        // depends on (the store snapshot the cycle check simulates over, the
        // already-applied probe) and every write that follows are serialized
        // against every other writer. It is the SAME lock the ordinary
        // issue/dependency write path takes as its outermost lock, so no
        // concurrent `jit issue create` / `jit dep add` can interleave: a mutation
        // landing between the snapshot and the writes could otherwise invalidate
        // the checks the writes rely on, and the compensating rollback would
        // revert work this apply never made. Reentrant, so the nested storage
        // writes below take it again without deadlocking.
        let _repo_lock = self.storage.acquire_repo_write_lock()?;

        // === 1. Resolve the container and the anchor bindings ===
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

        // Every gate NAME declared across the template's nodes must resolve BEFORE
        // the first mutation — either as a registered gate PRESET bundle or as a
        // single gate KEY in the gate registry (`.jit/gates.toml`).
        // `apply_gate_preset` / `add_gates` resolve lazily during instantiation, so
        // an unknown name would otherwise fail after nodes are persisted. Resolve
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

        // Snapshot each bound anchor's dependencies BEFORE any mutation. (The
        // container's own deps are part of this when it is a bound anchor; the
        // `move-upstream-to-role` transform consumes exactly these pre-apply sets.)
        let anchor_dependency_snapshots: BTreeMap<String, Vec<String>> = resolved_bindings
            .iter()
            .map(|(name, full_id)| {
                Ok((name.clone(), self.storage.load_issue(full_id)?.dependencies))
            })
            .collect::<Result<_>>()?;

        // The whole-store snapshot the commit phase rolls back to, and the edge set
        // the prospective-cycle check simulates over. Taken inside the lock, so it
        // is the exact state the writes are about to mutate.
        let pre_apply_issues = self.storage.list_issues()?;

        let mut warnings = Vec::new();
        let created_node_ids_by_role = match existing_breakdown {
            // === Force refresh: update existing nodes in place, no duplicates ===
            // Edges + transforms are NOT re-run here: they were wired by the
            // original fresh apply, the nodes already exist among the container's
            // deps, and a re-run transform would snapshot the (now scaffold-bearing)
            // live deps and move `B` onto `P`, breaking the spine. Refresh only
            // re-seeds prose.
            Some(breakdown_id) => self
                .refresh_template_nodes(template, &breakdown_id, &container)
                .map_err(|e| self.restore_or_report(&pre_apply_issues, e))?,

            // === Expand, validate the delta, then commit it ===
            None => {
                let delta = expand_template(
                    template,
                    &container,
                    &resolved_bindings,
                    &anchor_dependency_snapshots,
                )?;
                self.prevalidate_delta(template, &delta, &pre_apply_issues)?;
                self.commit_delta(&delta, &mut warnings)
                    .map_err(|e| self.restore_or_report(&pre_apply_issues, e))?
            }
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

    /// Validate a [`TemplateDelta`] against the pre-apply store, read-only, so the
    /// commit phase can only fail on I/O.
    ///
    /// Two checks, both of which would otherwise surface mid-commit:
    ///
    /// - **Node writes.** `create_issue` runs the effective local rules over the
    ///   FINAL issue shape, including the always-enforced canonical
    ///   `namespace:value` label-format rule. A delta whose first node is valid but
    ///   whose LATER node interpolates to a rejected label must fail before the
    ///   first node is persisted, so each planned node is projected into the issue
    ///   it would persist and run through the same
    ///   [`validate_for_write`](Self::validate_for_write).
    /// - **Acyclicity.** The commit phase adds edges one at a time, so a cycle
    ///   formed by a LATER edge would surface after earlier writes landed;
    ///   [`validate_delta_acyclic`] simulates the whole prospective graph instead.
    fn prevalidate_delta(
        &self,
        template: &GraphTemplate,
        delta: &TemplateDelta,
        pre_apply_issues: &[Issue],
    ) -> Result<()> {
        for planned in &delta.creates {
            let projected = project_planned_issue(planned);
            // Any validation failure for a projected node is reported as an
            // argument error (exit 2) carrying this message verbatim. Mapping
            // (rather than wrapping) keeps the inner error's type from shifting the
            // exit code via downcast.
            self.validate_for_write(&projected, false).map_err(|_| {
                crate::errors::InvalidArgumentError::new(format!(
                    "template '{}' node '{}' would create an invalid issue",
                    template.name, planned.role
                ))
            })?;
        }

        let store_deps: BTreeMap<String, Vec<String>> = pre_apply_issues
            .iter()
            .map(|i| (i.id.clone(), i.dependencies.clone()))
            .collect();
        validate_delta_acyclic(delta, store_deps).map_err(|_| {
            anyhow!(
                "applying template '{}' would create a dependency cycle; \
                 no nodes were created",
                template.name
            )
        })
    }

    /// Commit a validated [`TemplateDelta`]: create its nodes (with gates), wire
    /// its `add_edges`, drop its `remove_edges`, then attach its anchor gates.
    /// Returns role → created issue id.
    ///
    /// Every edge goes through [`add_dependency`](Self::add_dependency) /
    /// [`remove_dependency`](Self::remove_dependency), the ordinary dependency
    /// path: cycle-checked, eagerly transitively reduced, and event-emitting.
    /// Adds precede removals so transitive reduction never strands an edge
    /// mid-operation.
    ///
    /// The caller holds the repository lock and restores the pre-mutation snapshot
    /// if this returns an error.
    fn commit_delta(
        &self,
        delta: &TemplateDelta,
        warnings: &mut Vec<String>,
    ) -> Result<BTreeMap<String, String>> {
        let mut created: BTreeMap<String, String> = BTreeMap::new();
        for planned in &delta.creates {
            let (node_id, mut create_warnings) = self.create_issue(
                planned.title.clone(),
                planned.description.clone(),
                planned.priority,
                vec![],
                planned.labels.clone(),
                None,
                None,
                false,
            )?;
            warnings.append(&mut create_warnings);
            self.attach_template_gates(&planned.gates, &node_id, warnings)?;
            created.insert(planned.role.clone(), node_id);
        }

        for edge in &delta.add_edges {
            let (_, mut w) = self.add_dependency(
                &resolve_endpoint(&edge.dependent, &created)?,
                &resolve_endpoint(&edge.dependency, &created)?,
            )?;
            warnings.append(&mut w);
        }

        for edge in &delta.remove_edges {
            let mut w = self.remove_dependency(
                &resolve_endpoint(&edge.dependent, &created)?,
                &resolve_endpoint(&edge.dependency, &created)?,
            )?;
            warnings.append(&mut w);
        }

        for attachment in &delta.anchor_gates {
            self.attach_template_gates(&attachment.gates, &attachment.anchor_issue_id, warnings)?;
        }

        Ok(created)
    }

    /// Roll the issue store back to `pre_apply_issues` after a failed commit, and
    /// return the error the caller reports.
    ///
    /// All-or-nothing by COMPENSATION rather than a write-ahead journal: the apply
    /// holds the repository lock across the whole sequence, so the only observer of
    /// an intermediate state is this process, and the pre-mutation snapshot it
    /// already holds is enough to undo the sequence. Reverting mutated issues
    /// precedes deleting created ones, so no restored issue ever names an
    /// already-deleted node.
    ///
    /// The append-only event log keeps the events of the attempted apply and gains
    /// the compensating `issue_updated` / `issue_deleted` events, so the audit
    /// trail records both directions (`@/inv/event-log`).
    ///
    /// When the rollback itself fails, the returned error says so and names both
    /// causes: the store may be left partially applied and needs manual repair.
    fn restore_or_report(&self, pre_apply_issues: &[Issue], cause: anyhow::Error) -> anyhow::Error {
        match self.restore_issue_snapshot(pre_apply_issues) {
            Ok(()) => cause,
            Err(restore_error) => anyhow!(
                "apply failed ({cause:#}) and rolling it back also failed \
                 ({restore_error:#}); the issue store may be left partially applied \
                 and needs manual repair"
            ),
        }
    }

    /// Restore the issue store to `pre_apply_issues`: revert every issue whose
    /// content changed, then delete every issue the apply created.
    fn restore_issue_snapshot(&self, pre_apply_issues: &[Issue]) -> Result<()> {
        let prior: BTreeMap<&str, &Issue> = pre_apply_issues
            .iter()
            .map(|issue| (issue.id.as_str(), issue))
            .collect();

        for current in self.storage.list_issues()? {
            match prior.get(current.id.as_str()) {
                Some(original) => {
                    let fields = changed_fields(original, &current);
                    if !fields.is_empty() {
                        self.storage.save_issue((*original).clone())?;
                        self.storage
                            .append_event(&crate::domain::Event::new_issue_updated(
                                current.id.clone(),
                                APPLY_ACTOR.to_string(),
                                fields,
                            ))?;
                    }
                }
                None => {
                    self.storage.delete_issue(&current.id)?;
                    self.storage
                        .append_event(&crate::domain::Event::new_issue_deleted(current.id))?;
                }
            }
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
    /// without creating duplicate nodes. Gates are NOT re-attached (idempotent
    /// attach is the commit path's job; a refresh only re-seeds prose).
    ///
    /// Every template role MUST map to an existing issue: a bracket that has lost
    /// its planning node (or any role) is broken, and refreshing it partially
    /// would silently report success while leaving stale prose. Such a case
    /// returns an error rather than a partial result (APPA-03).
    fn refresh_template_nodes(
        &self,
        template: &GraphTemplate,
        breakdown_id: &str,
        container: &Issue,
    ) -> Result<BTreeMap<String, String>> {
        let context = InterpolationContext::for_container(container);
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
                    APPLY_ACTOR.to_string(),
                    vec!["description".to_string()],
                ))?;
        }
        Ok(existing)
    }

    /// Resolve a template node/anchor gate `name` to either a registered gate
    /// PRESET bundle or a single gate KEY declared in the gate registry
    /// (`.jit/gates.toml`), PREFERRING the preset.
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

/// The issue id a [`DeltaEndpoint`] names: the id created for its role, or the
/// existing id it carries.
fn resolve_endpoint(
    endpoint: &DeltaEndpoint,
    created: &BTreeMap<String, String>,
) -> Result<String> {
    match endpoint {
        DeltaEndpoint::CreatedRole(role) => created.get(role).cloned().ok_or_else(|| {
            anyhow!("internal error: no issue was created for template role '{role}'")
        }),
        DeltaEndpoint::ExistingIssue(id) => Ok(id.clone()),
    }
}

/// Build the FINAL persisted [`Issue`] shape a planned node's `create_issue` would
/// produce, for read-only pre-validation. Mirrors `create_issue`'s construction
/// for a node that always carries a `type:` label and has no dependencies at
/// creation: fields set, then auto-promoted to [`State::Ready`] (so a state-keyed
/// rule sees the persisted shape).
fn project_planned_issue(planned: &PlannedNode) -> Issue {
    let mut issue = Issue::new(planned.title.clone(), planned.description.clone());
    issue.priority = planned.priority;
    issue.labels = planned.labels.clone();
    // A freshly-created issue with no dependencies is auto-promoted to Ready
    // by `create_issue`; replicate so state-keyed rules see the same shape.
    issue.state = State::Ready;
    issue
}

/// The names of the fields in which `current` differs from `original`, ignoring
/// storage-owned timestamps.
///
/// Drives rollback: an empty result means the apply never touched this issue, so
/// it needs neither a restoring write nor a compensating event. The names are the
/// ones the `issue_updated` event records.
fn changed_fields(original: &Issue, current: &Issue) -> Vec<String> {
    [
        ("title", original.title != current.title),
        ("description", original.description != current.description),
        ("state", original.state != current.state),
        ("priority", original.priority != current.priority),
        ("assignee", original.assignee != current.assignee),
        (
            "dependencies",
            original.dependencies != current.dependencies,
        ),
        (
            "gates_required",
            original.gates_required != current.gates_required,
        ),
        (
            "gates_status",
            original.gates_status != current.gates_status,
        ),
        ("labels", original.labels != current.labels),
        ("documents", original.documents != current.documents),
        ("context", original.context != current.context),
    ]
    .into_iter()
    .filter(|(_, differs)| *differs)
    .map(|(field, _)| field.to_string())
    .collect()
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
    fn test_project_planned_issue_mirrors_created_shape() {
        let planned = PlannedNode {
            role: "planning".to_string(),
            title: "planning: Epic".to_string(),
            description: "Plan it.".to_string(),
            labels: vec!["type:planning".to_string()],
            gates: vec![],
            priority: Priority::High,
        };
        let projected = project_planned_issue(&planned);
        assert_eq!(projected.title, "planning: Epic");
        assert_eq!(projected.priority, Priority::High);
        assert_eq!(projected.state, State::Ready);
        assert!(projected.dependencies.is_empty());
    }

    #[test]
    fn test_resolve_endpoint_maps_roles_and_passes_ids_through() {
        let created = BTreeMap::from([("planning".to_string(), "p1".to_string())]);
        assert_eq!(
            resolve_endpoint(
                &DeltaEndpoint::CreatedRole("planning".to_string()),
                &created
            )
            .unwrap(),
            "p1"
        );
        assert_eq!(
            resolve_endpoint(&DeltaEndpoint::ExistingIssue("c1".to_string()), &created).unwrap(),
            "c1"
        );
        assert!(
            resolve_endpoint(&DeltaEndpoint::CreatedRole("missing".to_string()), &created).is_err()
        );
    }

    #[test]
    fn test_changed_fields_is_empty_for_an_untouched_issue() {
        let issue = Issue::new("T".to_string(), "d".to_string());
        let mut same = issue.clone();
        // A storage-owned timestamp bump is not a content change.
        same.updated_at = chrono::Utc::now();
        assert!(changed_fields(&issue, &same).is_empty());
    }

    #[test]
    fn test_changed_fields_names_every_mutated_field() {
        let original = Issue::new("T".to_string(), "d".to_string());
        let mut current = original.clone();
        current.dependencies = vec!["u1".to_string()];
        current.state = State::InProgress;
        current.gates_required = vec!["cargo-ci".to_string()];
        let fields = changed_fields(&original, &current);
        assert!(fields.contains(&"dependencies".to_string()), "{fields:?}");
        assert!(fields.contains(&"state".to_string()), "{fields:?}");
        assert!(fields.contains(&"gates_required".to_string()), "{fields:?}");
        assert!(!fields.contains(&"title".to_string()), "{fields:?}");
    }
}
