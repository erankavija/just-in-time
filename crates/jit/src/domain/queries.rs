//! Pure query operations on issue collections.
//!
//! This module provides domain-level query functions that operate on slices of issues
//! without requiring storage access. These are pure functions that can be used
//! independently of the CLI orchestration layer.

use crate::domain::{GateStatus, Issue, Priority, State};
use std::collections::HashMap;

/// Build a HashMap of issue ID to issue reference for dependency resolution.
///
/// This is a common pattern used throughout query and validation code to enable
/// efficient lookups when checking if issues are blocked by dependencies.
///
/// # Example
///
/// ```rust
/// use jit::domain::queries::build_issue_map;
/// use jit::domain::Issue;
///
/// let issues = vec![
///     Issue::new("Task 1".to_string(), String::new()),
///     Issue::new("Task 2".to_string(), String::new()),
/// ];
///
/// let map = build_issue_map(&issues);
/// assert_eq!(map.len(), 2);
/// ```
pub fn build_issue_map(issues: &[Issue]) -> HashMap<String, &Issue> {
    issues.iter().map(|i| (i.id.clone(), i)).collect()
}

/// Query issues that are ready to be worked on.
///
/// Returns issues that are:
/// - In `Ready` state
/// - Unassigned
/// - Not blocked by dependencies or gates
pub fn query_ready(issues: &[Issue]) -> Vec<Issue> {
    let resolved = build_issue_map(issues);

    issues
        .iter()
        .filter(|i| i.state == State::Ready && i.assignee.is_none() && !i.is_blocked(&resolved))
        .cloned()
        .collect()
}

/// A typed reason explaining why an issue is blocked.
///
/// Produced by [`query_blocked`] in place of the previously hand-formatted
/// strings: the CLI matches on these variants to build its presentation form
/// without re-parsing text. The [`Display`](std::fmt::Display) implementation
/// renders the canonical reason string (`dependency:<id> (<title>:<state>)` and
/// `gate:<key> (<status>)`), so the human and `--json` output stay byte-identical
/// to the original.
///
/// # Examples
///
/// ```
/// use jit::domain::queries::BlockingReason;
/// use jit::domain::{GateStatus, State};
///
/// let dep = BlockingReason::Dependency {
///     id: "abc123".to_string(),
///     title: "Build parser".to_string(),
///     state: State::InProgress,
/// };
/// assert_eq!(dep.to_string(), "dependency:abc123 (Build parser:InProgress)");
///
/// let gate = BlockingReason::Gate {
///     key: "cargo-ci".to_string(),
///     status: GateStatus::Pending,
/// };
/// assert_eq!(gate.to_string(), "gate:cargo-ci (Pending)");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockingReason {
    /// An incomplete dependency: the issue waits on `id` (titled `title`),
    /// which is in `state` (anything other than `Done`).
    Dependency {
        /// Id of the depended-on issue.
        id: String,
        /// Title of the depended-on issue.
        title: String,
        /// Current state of the depended-on issue.
        state: State,
    },
    /// A required gate that is not yet `Passed`. `status` is the gate's current
    /// status, defaulting to [`GateStatus::Pending`] when it has never been
    /// evaluated.
    Gate {
        /// The required gate key.
        key: String,
        /// Current gate status.
        status: GateStatus,
    },
}

impl std::fmt::Display for BlockingReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockingReason::Dependency { id, title, state } => {
                write!(f, "dependency:{} ({}:{:?})", id, title, state)
            }
            BlockingReason::Gate { key, status } => {
                write!(f, "gate:{} ({:?})", key, status)
            }
        }
    }
}

/// Query blocked issues with typed reasons for being blocked.
///
/// Returns issues that have incomplete dependencies or unfulfilled gates,
/// along with a list of [`BlockingReason`]s explaining why each issue is blocked.
pub fn query_blocked(issues: &[Issue]) -> Vec<(Issue, Vec<BlockingReason>)> {
    let resolved = build_issue_map(issues);

    issues
        .iter()
        .filter(|issue| issue.is_blocked(&resolved))
        .map(|issue| {
            // Incomplete dependencies (anything not yet Done).
            let dep_reasons = issue.dependencies.iter().filter_map(|dep_id| {
                resolved.get(dep_id).and_then(|dep| {
                    (dep.state != State::Done).then(|| BlockingReason::Dependency {
                        id: dep_id.clone(),
                        title: dep.title.clone(),
                        state: dep.state,
                    })
                })
            });

            // Required gates that have not passed (absent status reads as Pending).
            let gate_reasons = issue.gates_required.iter().filter_map(|gate_key| {
                let gate_state = issue.gates_status.get(gate_key);
                let is_passed = gate_state
                    .map(|gs| gs.status == GateStatus::Passed)
                    .unwrap_or(false);

                (!is_passed).then(|| BlockingReason::Gate {
                    key: gate_key.clone(),
                    status: gate_state
                        .map(|gs| gs.status)
                        .unwrap_or(GateStatus::Pending),
                })
            });

            (issue.clone(), dep_reasons.chain(gate_reasons).collect())
        })
        .collect()
}

/// Query issues by assignee.
pub fn query_by_assignee(issues: &[Issue], assignee: &str) -> Vec<Issue> {
    issues
        .iter()
        .filter(|i| i.assignee.as_ref().is_some_and(|a| a == assignee))
        .cloned()
        .collect()
}

/// Query issues by state.
pub fn query_by_state(issues: &[Issue], state: State) -> Vec<Issue> {
    issues
        .iter()
        .filter(|i| i.state == state)
        .cloned()
        .collect()
}

/// Query issues by priority.
pub fn query_by_priority(issues: &[Issue], priority: Priority) -> Vec<Issue> {
    issues
        .iter()
        .filter(|i| i.priority == priority)
        .cloned()
        .collect()
}

/// Query issues by label pattern.
///
/// Pattern format: `namespace:value` or `namespace:*` for wildcard matching.
pub fn query_by_label(issues: &[Issue], pattern: &str) -> Vec<Issue> {
    use crate::labels;

    issues
        .iter()
        .filter(|issue| labels::matches_pattern(&issue.labels, pattern))
        .cloned()
        .collect()
}

/// Query strategic issues (those with strategic type labels).
///
/// Strategic types are defined in configuration (e.g., milestone, epic).
/// This function takes the list of strategic type names as a parameter.
pub fn query_strategic(issues: &[Issue], strategic_types: &[String]) -> Vec<Issue> {
    use crate::labels;

    if strategic_types.is_empty() {
        return Vec::new();
    }

    issues
        .iter()
        .filter(|issue| {
            strategic_types.iter().any(|type_value| {
                labels::matches_pattern(&issue.labels, &format!("type:{}", type_value))
            })
        })
        .cloned()
        .collect()
}

/// Query closed issues (Done or Rejected states).
pub fn query_closed(issues: &[Issue]) -> Vec<Issue> {
    issues
        .iter()
        .filter(|i| i.state.is_closed())
        .cloned()
        .collect()
}

/// Compute the bracket-scope membership for `jit validate --scope <container>`:
/// the container's transitive dependency closure, **including** any breakdown
/// boundary node `B` reached, but **bounded** so the walk does not descend
/// through `B` into its dependencies (`P` / upstream). The container id itself is
/// always a member.
///
/// The breakdown boundary type is supplied by the caller, not baked in: `B` is
/// the node carrying `type:<breakdown_type>`, where `breakdown_type` is the
/// caller's configured breakdown-node type (e.g. `"breakdown"`, or any custom
/// name) — derived by the caller from the applicable graph template's breakdown
/// node. The label string is built at runtime from that name, so the engine stays
/// domain-agnostic.
///
/// When `breakdown_type` is `None` — i.e. no applicable graph template, so the
/// container is not bracketed — there is no breakdown boundary: the walk is the
/// full transitive dependency closure of the container, with no halt.
///
/// This is a pure graph walk over dependency edges (no I/O): a BFS from the
/// container that adds each dependency it reaches and continues descending,
/// except that (when a boundary type is configured) it never enqueues the
/// dependencies OF a boundary node — so `B` is collected as the boundary but
/// everything strictly upstream of it is excluded. The container is matched by
/// its full id (the caller resolves partial ids before calling). A missing
/// container yields just that id.
///
/// Membership here decides whose `when` rules are evaluated (D14); it is
/// independent of `child-type-exclude`, which governs only the coverage rule's
/// candidate traversal.
///
/// # Examples
///
/// ```rust
/// use jit::domain::queries::bracket_scope_ids;
/// use jit::domain::Issue;
///
/// // A spine `C -> I -> B -> P`: container, an impl task, a breakdown node,
/// // and the plan upstream of the breakdown. Edges point dependent -> dependency.
/// let mut c = Issue::new("container".to_string(), String::new());
/// c.id = "C".to_string();
/// c.labels = vec!["type:epic".to_string()];
/// c.dependencies = vec!["I".to_string()];
///
/// let mut i = Issue::new("impl".to_string(), String::new());
/// i.id = "I".to_string();
/// i.dependencies = vec!["B".to_string()];
///
/// let mut b = Issue::new("breakdown".to_string(), String::new());
/// b.id = "B".to_string();
/// b.labels = vec!["type:breakdown".to_string()];
/// b.dependencies = vec!["P".to_string()];
///
/// let mut p = Issue::new("plan".to_string(), String::new());
/// p.id = "P".to_string();
///
/// let issues = vec![c, i, b, p];
///
/// // The walk halts AT the breakdown node: C, I and B are in scope, P is not.
/// let ids = bracket_scope_ids("C", &issues, Some("breakdown"));
/// assert!(ids.contains("C") && ids.contains("I") && ids.contains("B"));
/// assert!(!ids.contains("P"));
///
/// // With no configured boundary the walk descends the full closure.
/// let full = bracket_scope_ids("C", &issues, None);
/// assert!(full.contains("P"));
/// ```
pub fn bracket_scope_ids(
    container_id: &str,
    issues: &[Issue],
    breakdown_type: Option<&str>,
) -> std::collections::HashSet<String> {
    use std::collections::{HashSet, VecDeque};

    // The boundary label, built from the caller-supplied breakdown-node type
    // (e.g. `type:breakdown`). Absent when the container is not bracketed (no
    // applicable graph template), in which case nothing halts the walk and the
    // result is the full dependency closure.
    let breakdown_label = breakdown_type.map(|t| format!("type:{t}"));

    let is_breakdown = |id: &str| match &breakdown_label {
        Some(label) => issues
            .iter()
            .find(|i| i.id == id)
            .is_some_and(|i| i.labels.iter().any(|l| l == label)),
        None => false,
    };

    let mut included: HashSet<String> = HashSet::new();
    included.insert(container_id.to_string());

    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(container_id.to_string());

    while let Some(current) = queue.pop_front() {
        // Halt the walk AT a breakdown node: it is in scope, but its own
        // dependencies (the plan `P` / upstream) are out of scope.
        if current != container_id && is_breakdown(&current) {
            continue;
        }
        let Some(node) = issues.iter().find(|i| i.id == current) else {
            continue;
        };
        for dep in &node.dependencies {
            // Skip dangling edges to ids absent from the store (defensive: the
            // integrity check owns broken-dependency reporting).
            if !issues.iter().any(|i| &i.id == dep) {
                continue;
            }
            if included.insert(dep.clone()) {
                queue.push_back(dep.clone());
            }
        }
    }

    included
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_issue_map() {
        // Create test issues
        let issue1 = Issue::new("Task 1".to_string(), String::new());
        let issue2 = Issue::new("Task 2".to_string(), String::new());
        let issue3 = Issue::new("Task 3".to_string(), String::new());

        let issues = vec![issue1.clone(), issue2.clone(), issue3.clone()];

        // Build the map
        let map = build_issue_map(&issues);

        // Verify all issues are in the map
        assert_eq!(map.len(), 3);
        assert_eq!(map.get(&issue1.id).unwrap().id, issue1.id);
        assert_eq!(map.get(&issue2.id).unwrap().id, issue2.id);
        assert_eq!(map.get(&issue3.id).unwrap().id, issue3.id);

        // Verify we can look up by ID
        assert_eq!(map.get(&issue1.id).unwrap().title, "Task 1");
        assert_eq!(map.get(&issue2.id).unwrap().title, "Task 2");
        assert_eq!(map.get(&issue3.id).unwrap().title, "Task 3");
    }

    #[test]
    fn test_build_issue_map_empty() {
        let issues: Vec<Issue> = vec![];
        let map = build_issue_map(&issues);
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn test_build_issue_map_single() {
        let issue = Issue::new("Single task".to_string(), String::new());
        let issues = vec![issue.clone()];
        let map = build_issue_map(&issues);

        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&issue.id).unwrap().title, "Single task");
    }

    /// Build an issue with the given id, type label, and dependency ids.
    fn scope_issue(id: &str, type_: &str, deps: &[&str]) -> Issue {
        let mut issue = Issue::new(format!("issue {id}"), String::new());
        issue.id = id.to_string();
        issue.labels = vec![format!("type:{type_}")];
        issue.dependencies = deps.iter().map(|s| s.to_string()).collect();
        issue
    }

    /// A bracket spine `C -> impl1 -> impl2 -> B -> P` plus an upstream `U`
    /// that `P` depends on. Edges point from dependent to dependency (an issue
    /// depends on the ones before it close).
    fn bracket_spine() -> Vec<Issue> {
        vec![
            scope_issue("C0000000", "epic", &["I1000000"]),
            scope_issue("I1000000", "task", &["I2000000"]),
            scope_issue("I2000000", "task", &["B0000000"]),
            scope_issue("B0000000", "breakdown", &["P0000000"]),
            scope_issue("P0000000", "planning", &["U0000000"]),
            scope_issue("U0000000", "epic", &[]),
        ]
    }

    #[test]
    fn test_bracket_scope_includes_container_impl_and_breakdown() {
        let issues = bracket_spine();
        let ids = bracket_scope_ids("C0000000", &issues, Some("breakdown"));
        assert!(ids.contains("C0000000"), "container is in scope");
        assert!(ids.contains("I1000000"), "impl source is in scope");
        assert!(ids.contains("I2000000"), "impl sink is in scope");
        assert!(
            ids.contains("B0000000"),
            "breakdown node B is in scope (its rule fires)"
        );
    }

    #[test]
    fn test_bracket_scope_stops_at_breakdown_excludes_plan_and_upstream() {
        let issues = bracket_spine();
        let ids = bracket_scope_ids("C0000000", &issues, Some("breakdown"));
        assert!(
            !ids.contains("P0000000"),
            "walk halts at B; plan P is out of scope"
        );
        assert!(
            !ids.contains("U0000000"),
            "upstream beyond B is out of scope"
        );
        assert_eq!(ids.len(), 4, "exactly C + 2 impl + B");
    }

    #[test]
    fn test_bracket_scope_boundary_type_is_caller_supplied() {
        // The boundary is whatever breakdown-node type the caller passes — here a
        // CUSTOM type `synthesis`, not the literal "breakdown". The walk must
        // halt at the `type:synthesis` node and exclude its upstream.
        let issues = vec![
            scope_issue("C0000000", "goal", &["I1000000"]),
            scope_issue("I1000000", "task", &["S0000000"]),
            scope_issue("S0000000", "synthesis", &["P0000000"]),
            scope_issue("P0000000", "planning", &["U0000000"]),
            scope_issue("U0000000", "goal", &[]),
        ];
        let ids = bracket_scope_ids("C0000000", &issues, Some("synthesis"));
        assert!(ids.contains("C0000000"), "container is in scope");
        assert!(ids.contains("I1000000"), "impl is in scope");
        assert!(
            ids.contains("S0000000"),
            "the custom-named boundary node is in scope"
        );
        assert!(
            !ids.contains("P0000000"),
            "walk halts at the synthesis boundary; plan is out of scope"
        );
        assert!(
            !ids.contains("U0000000"),
            "upstream beyond the synthesis boundary is out of scope"
        );
        assert_eq!(ids.len(), 3, "exactly C + impl + synthesis boundary");
    }

    #[test]
    fn test_bracket_scope_no_boundary_walks_full_closure() {
        // With no boundary type (breakdown_type = None — a non-bracketed
        // container) there is no bracket boundary, so the walk is the full
        // transitive closure: even a node that WOULD be a boundary under a
        // configured type does not halt it.
        let issues = bracket_spine();
        let ids = bracket_scope_ids("C0000000", &issues, None);
        assert!(
            ids.contains("B0000000") && ids.contains("P0000000") && ids.contains("U0000000"),
            "without a configured boundary the walk descends the whole closure: {ids:?}"
        );
        assert_eq!(ids.len(), 6, "all six spine nodes are reachable");
    }

    #[test]
    fn test_bracket_scope_lone_container_is_just_itself() {
        let issues = vec![scope_issue("C0000000", "epic", &[])];
        let ids = bracket_scope_ids("C0000000", &issues, Some("breakdown"));
        assert_eq!(ids.len(), 1);
        assert!(ids.contains("C0000000"));
    }

    #[test]
    fn test_query_blocked_dependency_reason_is_typed() {
        let mut dep = Issue::new("Upstream".to_string(), String::new());
        dep.id = "dep1".to_string();
        dep.state = State::InProgress;

        let mut blocked = Issue::new("Downstream".to_string(), String::new());
        blocked.id = "blocked1".to_string();
        blocked.state = State::Backlog;
        blocked.dependencies = vec!["dep1".to_string()];

        let issues = vec![dep, blocked];
        let result = query_blocked(&issues);

        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].1,
            vec![BlockingReason::Dependency {
                id: "dep1".to_string(),
                title: "Upstream".to_string(),
                state: State::InProgress,
            }]
        );
        // Display matches the historical string format byte-for-byte.
        assert_eq!(
            result[0].1[0].to_string(),
            "dependency:dep1 (Upstream:InProgress)"
        );
    }

    #[test]
    fn test_query_blocked_gate_reason_is_typed_and_defaults_pending() {
        use crate::domain::{GateState, GateStatus};

        // A Backlog issue with an unmet dependency keeps it blocked; a required
        // gate with no recorded status contributes a Pending gate reason.
        let mut dep = Issue::new("Upstream".to_string(), String::new());
        dep.id = "dep1".to_string();
        dep.state = State::InProgress;

        let mut blocked = Issue::new("Downstream".to_string(), String::new());
        blocked.id = "blocked1".to_string();
        blocked.state = State::Backlog;
        blocked.dependencies = vec!["dep1".to_string()];
        blocked.gates_required = vec!["never-run".to_string(), "failed".to_string()];
        blocked.gates_status.insert(
            "failed".to_string(),
            GateState {
                status: GateStatus::Failed,
                updated_by: None,
                updated_at: chrono::Utc::now(),
            },
        );

        let issues = vec![dep, blocked];
        let result = query_blocked(&issues);

        assert_eq!(result.len(), 1);
        let reasons = &result[0].1;
        assert!(reasons.contains(&BlockingReason::Gate {
            key: "never-run".to_string(),
            status: GateStatus::Pending,
        }));
        assert!(reasons.contains(&BlockingReason::Gate {
            key: "failed".to_string(),
            status: GateStatus::Failed,
        }));
        assert!(reasons
            .iter()
            .any(|r| r.to_string() == "gate:never-run (Pending)"));
        assert!(reasons
            .iter()
            .any(|r| r.to_string() == "gate:failed (Failed)"));
    }

    #[test]
    fn test_bracket_scope_ignores_dangling_dependency() {
        // Container depends on an id absent from the store; the walk must not
        // panic and must not include the missing id.
        let issues = vec![scope_issue("C0000000", "epic", &["MISSING0"])];
        let ids = bracket_scope_ids("C0000000", &issues, Some("breakdown"));
        assert_eq!(
            ids,
            std::collections::HashSet::from(["C0000000".to_string()])
        );
    }
}
