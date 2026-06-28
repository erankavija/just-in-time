//! Bracket-aware breakdown operations.
//!
//! The bracket-aware breakdown path lives here:
//!
//! - [`bracket_breakdown`](CommandExecutor::bracket_breakdown) — the
//!   **bracket-aware** path (design doc T10). For a breakable container `C`
//!   already scaffolded by `jit apply plan <C>` into the bracket `C → B → P`,
//!   it **consumes** the pre-created breakdown node `B` (it does NOT create it),
//!   drafts the impl children in Backlog, and splices a **source/sink-only
//!   spine** `C → impl → B → P`.
//!
//! The bracket-aware path reads its vocabulary (planning/breakdown types, the
//! plan-quality gate) from the [`TemplateRegistry`](crate::templates::TemplateRegistry)
//! / [`GraphTemplate`](crate::templates::GraphTemplate) — the same source the
//! `jit apply plan` scaffold uses, so the two halves of the workflow agree on the
//! `C → B → P` shape.

use super::*;
use crate::templates::{GraphTemplate, BREAKDOWN_ROLE, PLANNING_ROLE};
use serde::Serialize;

/// One drafted implementation child for [`bracket_breakdown`].
///
/// Intra-subgraph dependency structure is expressed by `deps`: each entry is a
/// **0-based index into the children list** naming another child this one
/// depends on. From this the breakdown path derives the spine boundary —
/// *sources* (no intra-subgraph predecessor) depend on `B`, *sinks* (no
/// intra-subgraph successor) are depended-on by `C`.
///
/// # Examples
///
/// ```
/// use jit::commands::BracketChild;
/// use jit::domain::Priority;
///
/// // child 1 depends on child 0 (a two-step chain).
/// let source = BracketChild {
///     title: "Scaffold API".to_string(),
///     description: String::new(),
///     priority: Priority::Normal,
///     gates: vec![],
///     labels: vec!["satisfies:REQ-01".to_string()],
///     deps: vec![],
/// };
/// let sink = BracketChild {
///     title: "Wire handlers".to_string(),
///     description: String::new(),
///     priority: Priority::Normal,
///     gates: vec!["code-review".to_string()],
///     labels: vec![],
///     deps: vec![0],
/// };
/// assert!(source.deps.is_empty());
/// assert_eq!(sink.deps, vec![0]);
/// ```
#[derive(Debug, Clone)]
pub struct BracketChild {
    /// Issue title.
    pub title: String,
    /// Issue description (markdown body).
    pub description: String,
    /// Issue priority.
    pub priority: Priority,
    /// Per-issue quality gates (gate keys) to attach at creation. The skill
    /// assigns these per task; the engine simply forwards them.
    pub gates: Vec<String>,
    /// Per-child labels to attach in addition to the container's inherited
    /// membership labels — notably the `satisfies:<id>` labels that credit this
    /// child against the container's `[hard]` criteria for the coverage-preview
    /// check. The engine forwards them; the skill assigns coverage per task.
    pub labels: Vec<String>,
    /// 0-based indices of sibling children this child depends on (the
    /// intra-subgraph edges).
    pub deps: Vec<usize>,
}

/// Outcome of a [`bracket_breakdown`] operation.
///
/// Names the bracketed container, the **pre-created** breakdown node `B` the
/// children fan out behind, the planning node `P` the breakdown waits on, and the
/// drafted impl children (in declaration order, so indices match the input). Used
/// for `--json` output and as the in-process return value.
///
/// This is a purely structural result. The breakdown step is a **spine-splicer**:
/// the breakdown node `B` and its two gates were already created/attached by
/// `jit apply plan <C>`; breakdown CONSUMES that `B` and reports the gate-preset
/// names `B` carries in [`coverage_gate_preset`](Self::coverage_gate_preset) and
/// [`breakdown_review_gate_preset`](Self::breakdown_review_gate_preset). It does
/// **not** run, stamp, or fabricate a coverage verdict — those gates are run
/// separately by the standard gate runner (`jit gate pass`) as breakdown-workflow
/// steps, exactly as every gate in the project is run by the orchestrator. So
/// there is no `coverage_passed`/`coverage_report` field here.
///
/// # Examples
///
/// ```
/// use jit::commands::BracketBreakdownResult;
///
/// let result = BracketBreakdownResult {
///     container_id: "c1".to_string(),
///     breakdown_id: "b1".to_string(),
///     planning_id: "p1".to_string(),
///     child_ids: vec!["k1".to_string(), "k2".to_string()],
///     coverage_gate_preset: "coverage-preview".to_string(),
///     breakdown_review_gate_preset: "breakdown-review".to_string(),
/// };
/// assert_eq!(result.child_ids.len(), 2);
/// assert_eq!(result.coverage_gate_preset, "coverage-preview");
/// ```
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BracketBreakdownResult {
    /// The bracketed container `C`.
    pub container_id: String,
    /// The pre-created breakdown node `B` this step consumed.
    pub breakdown_id: String,
    /// The planning node `P` the breakdown node depends on (found through `B`).
    pub planning_id: String,
    /// The drafted impl children, in declaration order.
    pub child_ids: Vec<String>,
    /// The deterministic coverage gate preset `B` carries (the FIRST gate on the
    /// template's breakdown node, e.g. `coverage-preview`), attached by
    /// `jit apply plan` and left PENDING for the standard gate runner. Reported
    /// for `--json` consumers; breakdown does not re-attach or run it.
    pub coverage_gate_preset: String,
    /// The agent-review gate preset `B` carries (the SECOND gate on the
    /// template's breakdown node, e.g. `breakdown-review`), attached by
    /// `jit apply plan` and left PENDING for the standard gate runner. Reported
    /// for `--json` consumers; breakdown does not re-attach or run it.
    pub breakdown_review_gate_preset: String,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Bracket-aware breakdown (design doc T10).
    ///
    /// Resolves the container's `plan`-shaped [`GraphTemplate`] from the cached
    /// [`TemplateRegistry`](crate::templates::TemplateRegistry)
    /// (`.jit/templates.toml`) by the container's `type:` label and delegates to
    /// [`bracket_breakdown_with_template`](Self::bracket_breakdown_with_template).
    /// See that method for the full spine-wiring contract.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::{BracketChild, CommandExecutor};
    /// use jit::domain::Priority;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let children = vec![BracketChild {
    ///     title: "Build login".to_string(),
    ///     description: String::new(),
    ///     priority: Priority::Normal,
    ///     gates: vec![],
    ///     labels: vec![],
    ///     deps: vec![],
    /// }];
    /// let result = executor.bracket_breakdown("epic-123", children).unwrap();
    /// println!("consumed breakdown node {}", result.breakdown_id);
    /// ```
    pub fn bracket_breakdown(
        &self,
        container_id: &str,
        children: Vec<BracketChild>,
    ) -> Result<BracketBreakdownResult> {
        // Resolve the container's type up front so the template can be selected by
        // `applies_to`, mirroring how `jit apply plan` picks the template.
        let full_container_id = self.storage.resolve_issue_id(container_id)?;
        let container = self.storage.load_issue(&full_container_id)?;
        let container_type = label_utils::type_label_value(&container.labels).ok_or_else(|| {
            anyhow!(
                "container '{}' has no type: label; bracket breakdown requires a breakable type",
                container.short_id()
            )
        })?;
        // Clone the template out of the cached registry: breakdown mutates issues
        // through `&self`, so it cannot hold a borrow into the config cache.
        let template = self
            .cached_config()?
            .templates
            .template_for_container(container_type)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "container type '{container_type}' has no graph template in \
                     .jit/templates.toml; declare a template whose applies_to lists it"
                )
            })?;
        self.bracket_breakdown_with_template(&template, container_id, children)
    }

    /// Bracket-aware breakdown using an explicit [`GraphTemplate`].
    ///
    /// The template-injecting core of [`bracket_breakdown`](Self::bracket_breakdown),
    /// separated so the spine logic is testable without an on-disk
    /// `templates.toml` (mirroring the `*_with` split in [`template`](super::template)).
    /// Given a breakable container `C` already scaffolded by `jit apply plan <C>`
    /// into the bracket `C → B → P`, and a set of drafted children with their
    /// intra-subgraph dependency structure, it:
    ///
    /// 1. Validates `C`'s `type:` label is in the template's `applies_to`, then
    ///    LOCATES the pre-created breakdown node `B`: the dependency of `C` typed
    ///    as the template's breakdown type AND carrying the `brackets:<C-short-id>`
    ///    label the apply engine seeds. If absent, errors: run `jit apply plan <C>`
    ///    first. It does NOT create `B` or re-attach `B`'s gates.
    /// 2. Finds the planning node `P` THROUGH `B`: `P` is `B`'s dependency typed as
    ///    the template's planning type (the apply engine wired `B → P`).
    /// 3. Enforces an APPROVED plan: `P`'s plan-quality gate (the planning node's
    ///    first declared gate, e.g. `plan-review`) must have PASSED.
    /// 4. Creates the impl children in **Backlog** (they each gain a dependency,
    ///    so [`create_issue`](Self::create_issue)'s auto-Ready promotion is
    ///    immediately demoted).
    /// 5. Splices the spine: each SOURCE child (no intra-subgraph predecessor)
    ///    depends on `B`; the internal edges are added as given; finally `C` is
    ///    made to depend on each SINK child (no intra-subgraph successor).
    ///    Transitive reduction (maintained by [`add_dependency`](Self::add_dependency))
    ///    drops the scaffold's now-redundant `C → B` edge and any redundant
    ///    `C → non-sink` edge.
    ///
    /// It deliberately avoids any parent-centric wiring: children never copy
    /// `C`'s dependencies, and `C` depends on sinks only — not on every child.
    ///
    /// All pre-mutation checks (empty child set, out-of-range/self `deps`, cycles
    /// in the child subgraph, breakable container type, a located `B`, and an
    /// APPROVED plan) run BEFORE any `create_issue`/`add_dependency`, so a rejected
    /// breakdown leaves NO partial state.
    ///
    /// `B`'s two gates — the deterministic coverage gate and the agent
    /// breakdown-review gate — were ATTACHED by `jit apply plan` and are left
    /// PENDING. Breakdown neither re-attaches nor runs them: they are run later by
    /// the standard gate runner (`jit gate pass <B> <gate>`) as breakdown-workflow
    /// steps — the orchestrator runs every gate in this project, never command
    /// code. Because the impl subgraph transitively depends on `B`, jit's gate
    /// enforcement is self-guiding: the fan-out cannot release until both gates on
    /// `B` pass.
    ///
    /// # Errors
    ///
    /// Returns an error if the container is not breakable, has no pre-created
    /// breakdown node (run `jit apply plan <C>` first), `B` has no planning-node
    /// dependency, that planning node's plan-quality gate has not passed, the child
    /// set is empty, a child's `deps` index is out of range or self-referent, or
    /// the children's `deps` form a cycle.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::{BracketChild, CommandExecutor};
    /// use jit::domain::Priority;
    /// use jit::storage::JsonFileStorage;
    /// use jit::templates::TemplateRegistry;
    ///
    /// let toml = r#"
    /// [[template]]
    /// name = "plan"
    /// applies_to = ["epic"]
    /// [[template.nodes]]
    /// role = "planning"
    /// type = "planning"
    /// gates = ["plan-review"]
    /// [[template.nodes]]
    /// role = "breakdown"
    /// type = "breakdown"
    /// gates = ["coverage-preview", "breakdown-review"]
    /// depends_on = ["planning"]
    /// "#;
    /// let registry =
    ///     TemplateRegistry::from_toml_str(toml, &["epic", "planning", "breakdown"]).unwrap();
    /// let template = registry.get("plan").unwrap();
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let children = vec![BracketChild {
    ///     title: "Build login".into(),
    ///     description: String::new(),
    ///     priority: Priority::Normal,
    ///     gates: vec![],
    ///     labels: vec![],
    ///     deps: vec![],
    /// }];
    /// let result = executor
    ///     .bracket_breakdown_with_template(template, "epic-123", children)
    ///     .unwrap();
    /// assert_eq!(result.coverage_gate_preset, "coverage-preview");
    /// ```
    pub fn bracket_breakdown_with_template(
        &self,
        template: &GraphTemplate,
        container_id: &str,
        children: Vec<BracketChild>,
    ) -> Result<BracketBreakdownResult> {
        if children.is_empty() {
            return Err(anyhow!(
                "bracket breakdown requires at least one child issue"
            ));
        }
        // Validate the children's intra-subgraph edges reference real siblings
        // BEFORE creating anything, so a malformed plan leaves no orphaned nodes.
        let n = children.len();
        for (i, child) in children.iter().enumerate() {
            for &dep in &child.deps {
                if dep >= n {
                    return Err(anyhow!(
                        "child {i} ('{}') depends on out-of-range sibling index {dep} \
                         (only {n} children provided)",
                        child.title
                    ));
                }
                if dep == i {
                    return Err(anyhow!(
                        "child {i} ('{}') cannot depend on itself",
                        child.title
                    ));
                }
            }
        }

        // Reject a cyclic child plan BEFORE any mutation: a cycle in the `deps`
        // index adjacency would otherwise create B and some children before
        // `add_dependency` rejects the closing edge, leaving partial state.
        if let Some(cycle) = first_child_cycle(&children) {
            let names: Vec<String> = cycle
                .iter()
                .map(|&i| format!("{i} ('{}')", children[i].title))
                .collect();
            return Err(anyhow!(
                "child dependency cycle detected among siblings: {}; \
                 the child subgraph must be acyclic",
                names.join(" -> ")
            ));
        }

        let full_container_id = self.storage.resolve_issue_id(container_id)?;
        let container = self.storage.load_issue(&full_container_id)?;

        // 1. Validate the container is breakable: its type must be in the
        //    template's `applies_to` (the same gate `jit apply plan` enforces).
        match label_utils::type_label_value(&container.labels) {
            Some(ty) if template.applies_to.iter().any(|b| b == ty) => {}
            Some(ty) => {
                return Err(anyhow!(
                    "container type '{ty}' is not breakable; template '{}' applies to: {}",
                    template.name,
                    template.applies_to.join(", ")
                ))
            }
            None => {
                return Err(anyhow!(
                    "container has no type: label; bracket breakdown requires a breakable type \
                     (one of: {})",
                    template.applies_to.join(", ")
                ))
            }
        }

        // The template must declare the conventional planning + breakdown nodes;
        // their types name what we locate among the scaffold's nodes.
        let breakdown_type = template.breakdown_type().ok_or_else(|| {
            anyhow!(
                "template '{}' declares no '{}' node; bracket breakdown needs a breakdown node",
                template.name,
                BREAKDOWN_ROLE
            )
        })?;
        let planning_type = template.planning_type().ok_or_else(|| {
            anyhow!(
                "template '{}' declares no '{}' node; bracket breakdown needs a planning node",
                template.name,
                PLANNING_ROLE
            )
        })?;

        // 1b. LOCATE the pre-created breakdown node B (created by `jit apply
        //     plan <C>`): a dependency of C typed as the breakdown type AND
        //     carrying the `brackets:<C-short-id>` label the apply engine seeds.
        //     Breakdown consumes this B; it does not create one.
        let breakdown_id = self.find_breakdown_node(&container, breakdown_type)?;

        // 1c. Find the planning node P THROUGH B: P is B's dependency typed as the
        //     planning type (the apply engine wired `B → P`). The container no
        //     longer points directly at P (the spine is `C → B → P`).
        let breakdown_node = self.storage.load_issue(&breakdown_id)?;
        let planning_id = self.find_planning_node(&breakdown_node, planning_type)?;

        // 1d. Enforce an APPROVED plan BEFORE any mutation: P's plan-quality gate
        //     (the planning node's first declared gate, e.g. `plan-review`) must
        //     have PASSED. Breakdown consumes an approved plan; a pending/unset
        //     plan gate rejects with no partial state created.
        let plan_gate = template
            .planning_node()
            .and_then(|n| n.gates.first())
            .ok_or_else(|| {
                anyhow!(
                    "template '{}' planning node declares no gate; cannot verify an approved plan",
                    template.name
                )
            })?;
        let planning_node = self.storage.load_issue(&planning_id)?;
        let plan_gate_passed = matches!(
            planning_node.gates_status.get(plan_gate),
            Some(gate_state) if gate_state.status == GateStatus::Passed
        );
        if !plan_gate_passed {
            return Err(anyhow!(
                "planning node P ({}) {plan_gate} gate has not passed; breakdown requires an \
                 approved plan",
                planning_node.short_id()
            ));
        }

        // The breakdown node B and its two gates already exist (attached by
        // `jit apply plan`); breakdown does NOT re-create B or re-attach its gates.
        // Report the gate-preset names B carries for `--json` consumers.
        let breakdown_gates = template
            .breakdown_node()
            .map(|n| n.gates.clone())
            .unwrap_or_default();
        let coverage_gate_preset = breakdown_gates.first().cloned().unwrap_or_default();
        let breakdown_review_gate_preset = breakdown_gates.get(1).cloned().unwrap_or_default();

        // 3. Create the impl children. They inherit C's membership labels (not
        //    its type). Children are created dependency-less (so auto-Ready),
        //    then immediately gain spine/internal edges below, which demote them
        //    to Backlog — drafted, never surfaced by query_ready pre-approval.
        let membership = membership_labels(&container.labels);
        let mut child_ids = Vec::with_capacity(n);
        for child in &children {
            // Inherited membership labels plus the child's own labels (e.g.
            // satisfies:<id> coverage credits), de-duplicated.
            let mut child_labels = membership.clone();
            for label in &child.labels {
                if !child_labels.contains(label) {
                    child_labels.push(label.clone());
                }
            }
            let (id, _warnings) = self.create_issue(
                child.title.clone(),
                child.description.clone(),
                child.priority,
                child.gates.clone(),
                child_labels,
                None,
                None,
                false,
            )?;
            child_ids.push(id);
        }

        // 4. Splice the spine.
        //
        // 4a. Internal edges: child i depends on each sibling in its `deps`.
        for (i, child) in children.iter().enumerate() {
            for &dep in &child.deps {
                self.add_dependency(&child_ids[i], &child_ids[dep])?;
            }
        }

        // 4b. Sources → B. A source has no intra-subgraph predecessor, i.e. no
        //     non-empty `deps`. Every source depends on the approved breakdown,
        //     transitively gating ALL impl behind B.
        let sources: Vec<usize> = (0..n).filter(|&i| children[i].deps.is_empty()).collect();
        for &i in &sources {
            self.add_dependency(&child_ids[i], &breakdown_id)?;
        }

        // 4c. C → sinks. A sink has no intra-subgraph successor, i.e. no other
        //     child lists it in `deps`. C depends on each sink; reduction drops
        //     the redundant C → B edge once the spine connects them.
        let has_successor: Vec<bool> = {
            let mut flags = vec![false; n];
            for child in &children {
                for &dep in &child.deps {
                    flags[dep] = true;
                }
            }
            flags
        };
        for (i, &succ) in has_successor.iter().enumerate() {
            if !succ {
                self.add_dependency(&full_container_id, &child_ids[i])?;
            }
        }

        // B's two gates were ATTACHED by `jit apply plan` and left PENDING; the
        // standard gate runner (`jit gate pass <B> <gate>`) evaluates them as
        // breakdown-workflow steps — the orchestrator runs every gate, never this
        // command code. Breakdown is a clean spine-splicer: no faked verdict, no
        // gate stamping, no gate events emitted by breakdown.
        Ok(BracketBreakdownResult {
            container_id: full_container_id,
            breakdown_id,
            planning_id,
            child_ids,
            coverage_gate_preset,
            breakdown_review_gate_preset,
        })
    }

    /// Locate the pre-created breakdown node `B` among a container `C`'s
    /// dependencies.
    ///
    /// `B` is the dependency of `C` typed as `breakdown_type` AND carrying the
    /// `brackets:<C-short-id>` label that `jit apply plan` seeds (matching the
    /// engine's [`find_applied_breakdown`](super::template) convention). Errors
    /// clearly if `C` has not been scaffolded.
    fn find_breakdown_node(&self, container: &Issue, breakdown_type: &str) -> Result<String> {
        let bracket_label = format!("brackets:{}", container.short_id());
        for dep_id in &container.dependencies {
            let dep = self.storage.load_issue(dep_id)?;
            let has_type = label_utils::type_label_value(&dep.labels) == Some(breakdown_type);
            let has_bracket = dep.labels.iter().any(|l| l == &bracket_label);
            if has_type && has_bracket {
                return Ok(dep_id.clone());
            }
        }
        Err(anyhow!(
            "container '{}' has no breakdown bracket (no '{breakdown_type}'-typed dependency \
             carrying '{bracket_label}'); run `jit apply plan <id>` first",
            container.short_id()
        ))
    }

    /// Locate the planning node `P` THROUGH the breakdown node `B`.
    ///
    /// `P` is the dependency of `B` typed as `planning_type` (the apply engine
    /// wired `B → P`). Errors clearly if `B` has no planning-node dependency.
    fn find_planning_node(&self, breakdown: &Issue, planning_type: &str) -> Result<String> {
        for dep_id in &breakdown.dependencies {
            let dep = self.storage.load_issue(dep_id)?;
            if label_utils::type_label_value(&dep.labels) == Some(planning_type) {
                return Ok(dep_id.clone());
            }
        }
        Err(anyhow!(
            "breakdown node '{}' has no planning node (no '{planning_type}'-typed dependency); \
             the bracket is incomplete — re-run `jit apply plan <id>`",
            breakdown.short_id()
        ))
    }
}

/// Detect a cycle in the children's intra-subgraph `deps` index adjacency,
/// returning one offending cycle as a list of child indices if present (pure
/// helper). Assumes indices are already range/self checked.
///
/// Iterative DFS with a recursion stack over the index graph; on the first
/// back-edge it reconstructs the cycle from the stack.
fn first_child_cycle(children: &[BracketChild]) -> Option<Vec<usize>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Unvisited,
        InStack,
        Done,
    }
    let n = children.len();
    let mut mark = vec![Mark::Unvisited; n];

    // Each stack frame tracks a node and the next dep index to explore, so the
    // live stack always spells the current DFS path (for cycle reconstruction).
    for start in 0..n {
        if mark[start] != Mark::Unvisited {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
        mark[start] = Mark::InStack;
        // Take a mutable handle to the top frame each iteration; `last_mut()`
        // returning `Some` is also the loop condition, so no fallible unwrap is
        // needed (and library code must not `expect`).
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            if *next < children[node].deps.len() {
                let dep = children[node].deps[*next];
                *next += 1;
                match mark[dep] {
                    Mark::Unvisited => {
                        mark[dep] = Mark::InStack;
                        stack.push((dep, 0));
                    }
                    Mark::InStack => {
                        // Back-edge to `dep`: the cycle is the path suffix from
                        // `dep` to the current node, closed by `node -> dep`. `dep`
                        // is marked InStack, so it is on the stack; guard with
                        // `if let` rather than `expect` (no panic in library code).
                        if let Some(from) = stack.iter().position(|&(id, _)| id == dep) {
                            return Some(stack[from..].iter().map(|&(id, _)| id).collect());
                        }
                    }
                    Mark::Done => {}
                }
            } else {
                mark[node] = Mark::Done;
                stack.pop();
            }
        }
    }
    None
}

/// The container's labels minus its `type:*` label — the membership labels
/// (epic/milestone grouping) that bracket children and the breakdown node
/// inherit so they are grouped with their container (pure helper).
fn membership_labels(labels: &[String]) -> Vec<String> {
    labels
        .iter()
        .filter(|l| label_utils::type_label_value(std::slice::from_ref(*l)).is_none())
        .cloned()
        .collect()
}
