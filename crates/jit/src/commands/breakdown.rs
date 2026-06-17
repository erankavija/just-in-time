//! Issue breakdown operations.
//!
//! Two distinct breakdown paths live here, and they must NOT be conflated:
//!
//! - [`breakdown_issue`](CommandExecutor::breakdown_issue) — the legacy,
//!   **parent-centric** helper. It copies the parent's dependencies onto every
//!   child and makes the parent depend on *all* children. Used for plain
//!   (non-breakable) containers where simple containment is the desired shape.
//! - [`bracket_breakdown`](CommandExecutor::bracket_breakdown) — the
//!   **bracket-aware** path (design doc T10). For a breakable container `C`
//!   already scaffolded to its planning node `P` (`C → P`), it creates the
//!   breakdown node `B`, drafts the impl children in Backlog, and splices a
//!   **source/sink-only spine** `C → impl → B → P`. It deliberately does NOT
//!   reuse the parent-centric wiring.

use super::*;
use crate::config::PlanningConfig;
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
///     deps: vec![],
/// };
/// let sink = BracketChild {
///     title: "Wire handlers".to_string(),
///     description: String::new(),
///     priority: Priority::Normal,
///     gates: vec!["code-review".to_string()],
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
    /// 0-based indices of sibling children this child depends on (the
    /// intra-subgraph edges).
    pub deps: Vec<usize>,
}

/// Outcome of a [`bracket_breakdown`] operation.
///
/// Names the bracketed container, the breakdown node `B` created in front of it,
/// the planning node `P` the breakdown waits on, and the drafted impl children
/// (in declaration order, so indices match the input). Used for `--json` output
/// and as the in-process return value.
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
/// };
/// assert_eq!(result.child_ids.len(), 2);
/// ```
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BracketBreakdownResult {
    /// The bracketed container `C`.
    pub container_id: String,
    /// The breakdown node `B` created by this step.
    pub breakdown_id: String,
    /// The planning node `P` the breakdown node depends on.
    pub planning_id: String,
    /// The drafted impl children, in declaration order.
    pub child_ids: Vec<String>,
    /// The gate preset applied to `B` (from `[planning].coverage_gate_preset`).
    pub coverage_gate_preset: String,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Bracket-aware breakdown (`jit issue breakdown --bracket`, design doc T10).
    ///
    /// Reads the `[planning]` vocabulary from `.jit/config.toml` and delegates to
    /// [`bracket_breakdown_with_config`](Self::bracket_breakdown_with_config).
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
    ///     deps: vec![],
    /// }];
    /// let result = executor.bracket_breakdown("epic-123", children).unwrap();
    /// println!("created breakdown node {}", result.breakdown_id);
    /// ```
    pub fn bracket_breakdown(
        &self,
        container_id: &str,
        children: Vec<BracketChild>,
    ) -> Result<BracketBreakdownResult> {
        let config = self.bracket_planning_config()?;
        self.bracket_breakdown_with_config(&config, container_id, children)
    }

    /// Resolve the effective `[planning]` configuration, erroring clearly when
    /// the repository has not declared a bracket vocabulary.
    fn bracket_planning_config(&self) -> Result<PlanningConfig> {
        self.cached_config()?.planning.clone().ok_or_else(|| {
            anyhow!(
                "no [planning] section in .jit/config.toml: planning brackets require a \
                 declared vocabulary (breakable_types, planning_type, breakdown_type, ...)"
            )
        })
    }

    /// Bracket-aware breakdown using an explicit `[planning]` config.
    ///
    /// The config-injecting core of [`bracket_breakdown`](Self::bracket_breakdown),
    /// separated so the bracket logic is testable without an on-disk
    /// `config.toml`. Given a breakable container `C` already scaffolded to its
    /// planning node `P` (`C → P`), and a set of drafted children with their
    /// intra-subgraph dependency structure, it:
    ///
    /// 1. Validates `C`'s `type:` label is breakable and locates its planning
    ///    node `P` (a `type:<planning_type>` dependency of `C`).
    /// 2. Creates the breakdown node `B` (`type:<breakdown_type>`, a
    ///    `brackets:<C-id>` label, inheriting `C`'s membership labels), applies
    ///    the coverage-preview preset, and wires `B → P`.
    /// 3. Creates the impl children in **Backlog** (they each gain a dependency,
    ///    so [`create_issue`](Self::create_issue)'s auto-Ready promotion is
    ///    immediately demoted).
    /// 4. Splices the spine: each SOURCE child (no intra-subgraph predecessor)
    ///    depends on `B`; the internal edges are added as given; finally `C` is
    ///    made to depend on each SINK child (no intra-subgraph successor).
    ///    Transitive reduction (maintained by [`add_dependency`](Self::add_dependency))
    ///    drops the scaffold's now-redundant `C → P` edge and any redundant
    ///    `C → non-sink` edge.
    ///
    /// It deliberately does **not** reuse the parent-centric wiring of
    /// [`breakdown_issue`](Self::breakdown_issue): children never copy `C`'s
    /// dependencies, and `C` depends on sinks only — not on every child.
    ///
    /// # Errors
    ///
    /// Returns an error if the container is not breakable, has no scaffolded
    /// planning node, the child set is empty, or a child's `deps` index is out
    /// of range.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::{BracketChild, CommandExecutor};
    /// use jit::config::PlanningConfig;
    /// use jit::domain::Priority;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let config = PlanningConfig {
    ///     breakable_types: vec!["epic".into()],
    ///     planning_type: "planning".into(),
    ///     breakdown_type: "breakdown".into(),
    ///     plan_doc_location: "inline".into(),
    ///     plan_gate_preset: "plan-review".into(),
    ///     coverage_gate_preset: "coverage-preview".into(),
    /// };
    /// let children = vec![BracketChild {
    ///     title: "Build login".into(),
    ///     description: String::new(),
    ///     priority: Priority::Normal,
    ///     gates: vec![],
    ///     deps: vec![],
    /// }];
    /// let result = executor
    ///     .bracket_breakdown_with_config(&config, "epic-123", children)
    ///     .unwrap();
    /// assert_eq!(result.coverage_gate_preset, "coverage-preview");
    /// ```
    pub fn bracket_breakdown_with_config(
        &self,
        config: &PlanningConfig,
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

        let full_container_id = self.storage.resolve_issue_id(container_id)?;
        let container = self.storage.load_issue(&full_container_id)?;

        // 1. Validate the container is breakable.
        let container_type = type_label_value(&container.labels);
        match container_type.as_deref() {
            Some(ty) if config.breakable_types.iter().any(|b| b == ty) => {}
            Some(ty) => {
                return Err(anyhow!(
                    "container type '{ty}' is not breakable; declared breakable types: {}",
                    config.breakable_types.join(", ")
                ))
            }
            None => {
                return Err(anyhow!(
                    "container has no type: label; planning brackets require a breakable type \
                     (one of: {})",
                    config.breakable_types.join(", ")
                ))
            }
        }

        // 1b. Locate the planning node P: a dependency of C typed as the
        //     planning type. The scaffold (`jit plan`) created the `C → P` edge.
        let planning_id = self.find_planning_node(&container, config)?;

        // 2. Create the breakdown node B. It inherits C's non-type labels
        //    (epic/milestone membership), carries the breakdown type and a
        //    brackets:<C-id> label, and gets the coverage-preview preset.
        let mut breakdown_labels = membership_labels(&container.labels);
        breakdown_labels.push(format!("type:{}", config.breakdown_type));
        breakdown_labels.push(format!("brackets:{}", full_container_id));

        let (breakdown_id, _create_warnings) = self.create_issue(
            format!("Breakdown: {}", container.title),
            String::new(),
            container.priority,
            vec![],
            breakdown_labels,
            None,
            false,
        )?;

        // Apply the coverage-preview preset (registers the gate, initializes
        // status, logs events) and wire B → P.
        self.apply_gate_preset(
            &breakdown_id,
            &config.coverage_gate_preset,
            None,
            false,
            false,
            &[],
        )?;
        self.add_dependency(&breakdown_id, &planning_id)?;

        // 3. Create the impl children. They inherit C's membership labels (not
        //    its type). Children are created dependency-less (so auto-Ready),
        //    then immediately gain spine/internal edges below, which demote them
        //    to Backlog — drafted, never surfaced by query_ready pre-approval.
        let child_labels = membership_labels(&container.labels);
        let mut child_ids = Vec::with_capacity(n);
        for child in &children {
            let (id, _warnings) = self.create_issue(
                child.title.clone(),
                child.description.clone(),
                child.priority,
                child.gates.clone(),
                child_labels.clone(),
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
        //     the redundant C → P edge once the spine connects them.
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

        Ok(BracketBreakdownResult {
            container_id: full_container_id,
            breakdown_id,
            planning_id,
            child_ids,
            coverage_gate_preset: config.coverage_gate_preset.clone(),
        })
    }

    /// Locate the planning node `P` for a scaffolded container `C`.
    ///
    /// `P` is the dependency of `C` typed as `config.planning_type` (the scaffold
    /// step wired `C → P`). Errors clearly if `C` has not been scaffolded.
    fn find_planning_node(&self, container: &Issue, config: &PlanningConfig) -> Result<String> {
        for dep_id in &container.dependencies {
            let dep = self.storage.load_issue(dep_id)?;
            if type_label_value(&dep.labels).as_deref() == Some(config.planning_type.as_str()) {
                return Ok(dep_id.clone());
            }
        }
        Err(anyhow!(
            "container '{}' has no scaffolded planning node (no '{}'-typed dependency); \
             run `jit plan <id>` first",
            container.short_id(),
            config.planning_type
        ))
    }
}

/// Extract the `type:*` value from a label list, if present (pure helper).
fn type_label_value(labels: &[String]) -> Option<String> {
    labels.iter().find_map(|l| {
        label_utils::parse_label(l)
            .ok()
            .and_then(|(ns, v)| (ns == "type").then_some(v))
    })
}

/// The container's labels minus its `type:*` label — the membership labels
/// (epic/milestone grouping) that bracket children and the breakdown node
/// inherit so they are grouped with their container (pure helper).
fn membership_labels(labels: &[String]) -> Vec<String> {
    labels
        .iter()
        .filter(|l| {
            label_utils::parse_label(l)
                .map(|(ns, _)| ns != "type")
                .unwrap_or(true)
        })
        .cloned()
        .collect()
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Break down an issue into subtasks with optional gate handling
    pub fn breakdown_issue(
        &self,
        parent_id: &str,
        child_type: &str,
        subtasks: Vec<(String, String)>,
        gate_preset: Option<String>,
    ) -> Result<Vec<String>> {
        self.breakdown_issue_impl(parent_id, child_type, subtasks, gate_preset, false)
    }

    /// Break down an issue with inherited gates
    pub fn breakdown_issue_with_inherit(
        &self,
        parent_id: &str,
        child_type: &str,
        subtasks: Vec<(String, String)>,
        inherit_gates: bool,
    ) -> Result<Vec<String>> {
        self.breakdown_issue_impl(parent_id, child_type, subtasks, None, inherit_gates)
    }

    /// Internal implementation for breakdown with all gate options
    fn breakdown_issue_impl(
        &self,
        parent_id: &str,
        child_type: &str,
        subtasks: Vec<(String, String)>,
        gate_preset: Option<String>,
        inherit_gates: bool,
    ) -> Result<Vec<String>> {
        // Load parent issue
        let full_parent_id = self.storage.resolve_issue_id(parent_id)?;
        let parent = self.storage.load_issue(&full_parent_id)?;
        let original_deps = parent.dependencies.clone();

        // Transform labels: replace type: with child_type
        let mut child_labels = parent.labels.clone();
        child_labels.retain(|l| !l.starts_with("type:"));
        child_labels.push(format!("type:{}", child_type));

        // Create subtasks with transformed labels and no gates initially
        let mut subtask_ids = Vec::new();
        for (title, desc) in subtasks {
            let (subtask_id, _warnings) = self.create_issue(
                title,
                desc,
                parent.priority,
                vec![], // No gates initially
                child_labels.clone(),
                None,  // inherit repo-default content format
                false, // breakdown subtasks are not force-bypassed
            )?;
            subtask_ids.push(subtask_id);
        }

        // Apply gate option after creating all subtasks
        if let Some(preset_name) = gate_preset {
            // Apply preset via the proper flow (registers gates, initializes status, logs events)
            for subtask_id in &subtask_ids {
                self.apply_gate_preset(subtask_id, &preset_name, None, false, false, &[])?;
            }
        } else if inherit_gates {
            // Copy parent's gates to all subtasks via add_gates (validates registry, initializes status)
            let parent_gates = parent.gates_required.clone();
            if !parent_gates.is_empty() {
                for subtask_id in &subtask_ids {
                    self.add_gates(subtask_id, &parent_gates)?;
                }
            }
        }
        // else: no gates (default)

        // Copy parent's dependencies to each subtask
        for subtask_id in &subtask_ids {
            for dep_id in &original_deps {
                self.add_dependency(subtask_id, dep_id)?;
            }
        }

        // Make parent depend on all subtasks
        for subtask_id in &subtask_ids {
            self.add_dependency(&full_parent_id, subtask_id)?;
        }

        // Remove parent's original dependencies (now transitive through subtasks)
        for dep_id in &original_deps {
            self.remove_dependency(&full_parent_id, dep_id)?;
        }

        Ok(subtask_ids)
    }
}
