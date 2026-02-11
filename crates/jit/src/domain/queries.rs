//! Pure query operations on issue collections.
//!
//! This module provides domain-level query functions that operate on slices of issues
//! without requiring storage access. These are pure functions that can be used
//! independently of the CLI orchestration layer.

use crate::domain::{GateStatus, Issue, Priority, State};
use std::collections::HashMap;

/// Query issues that are ready to be worked on.
///
/// Returns issues that are:
/// - In `Ready` state
/// - Unassigned
/// - Not blocked by dependencies or gates
pub fn query_ready(issues: &[Issue]) -> Vec<Issue> {
    let issue_refs: Vec<&Issue> = issues.iter().collect();
    let resolved: HashMap<String, &Issue> = issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

    issues
        .iter()
        .filter(|i| i.state == State::Ready && i.assignee.is_none() && !i.is_blocked(&resolved))
        .cloned()
        .collect()
}

/// Query blocked issues with reasons for being blocked.
///
/// Returns issues that have incomplete dependencies or unfulfilled gates,
/// along with a list of reasons explaining why each issue is blocked.
pub fn query_blocked(issues: &[Issue]) -> Vec<(Issue, Vec<String>)> {
    let issue_refs: Vec<&Issue> = issues.iter().collect();
    let resolved: HashMap<String, &Issue> = issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

    let mut blocked = Vec::new();

    for issue in issues {
        if issue.is_blocked(&resolved) {
            let mut reasons = Vec::new();

            // Check dependencies
            for dep_id in &issue.dependencies {
                if let Some(dep) = resolved.get(dep_id) {
                    if dep.state != State::Done {
                        reasons.push(format!(
                            "dependency:{} ({}:{:?})",
                            dep_id, dep.title, dep.state
                        ));
                    }
                }
            }

            // Check gates
            for gate_key in &issue.gates_required {
                let gate_state = issue.gates_status.get(gate_key);
                let is_passed = gate_state
                    .map(|gs| gs.status == GateStatus::Passed)
                    .unwrap_or(false);

                if !is_passed {
                    let status_str = gate_state
                        .map(|gs| format!("{:?}", gs.status))
                        .unwrap_or_else(|| "Pending".to_string());
                    reasons.push(format!("gate:{} ({})", gate_key, status_str));
                }
            }

            blocked.push((issue.clone(), reasons));
        }
    }

    blocked
}

/// Query issues by assignee.
pub fn query_by_assignee(issues: &[Issue], assignee: &str) -> Vec<Issue> {
    issues
        .iter()
        .filter(|i| i.assignee.as_deref() == Some(assignee))
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
