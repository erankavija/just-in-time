use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Open,
    Ready,
    InProgress,
    Done,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Pending,
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateState {
    pub status: GateStatus,
    pub updated_by: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub title: String,
    pub description: String,
    pub state: State,
    pub priority: Priority,
    pub assignee: Option<String>,
    pub dependencies: Vec<String>,
    pub gates_required: Vec<String>,
    pub gates_status: HashMap<String, GateState>,
    pub context: HashMap<String, String>,
}

impl Issue {
    pub fn new(title: String, description: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            title,
            description,
            state: State::Open,
            priority: Priority::Normal,
            assignee: None,
            dependencies: Vec::new(),
            gates_required: Vec::new(),
            gates_status: HashMap::new(),
            context: HashMap::new(),
        }
    }

    pub fn is_blocked(&self, resolved_issues: &HashMap<String, &Issue>) -> bool {
        // Check if any dependency is not done
        let has_incomplete_deps = self
            .dependencies
            .iter()
            .any(|dep_id| !matches!(resolved_issues.get(dep_id), Some(issue) if issue.state == State::Done));

        if has_incomplete_deps {
            return true;
        }

        // Check if any required gate is not passed
        self.gates_required
            .iter()
            .any(|gate_key| !matches!(self.gates_status.get(gate_key), Some(gate_state) if gate_state.status == GateStatus::Passed))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gate {
    pub key: String,
    pub title: String,
    pub description: String,
    pub auto: bool,
    pub example_integration: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_issue_has_correct_defaults() {
        let issue = Issue::new("Test Issue".to_string(), "Description".to_string());

        assert_eq!(issue.title, "Test Issue");
        assert_eq!(issue.description, "Description");
        assert_eq!(issue.state, State::Open);
        assert_eq!(issue.priority, Priority::Normal);
        assert_eq!(issue.assignee, None);
        assert!(issue.dependencies.is_empty());
        assert!(issue.gates_required.is_empty());
        assert!(issue.gates_status.is_empty());
        assert!(issue.context.is_empty());
        assert!(!issue.id.is_empty());
    }

    #[test]
    fn test_issue_not_blocked_with_no_dependencies_or_gates() {
        let issue = Issue::new("Test".to_string(), "Desc".to_string());
        let resolved = HashMap::new();

        assert!(!issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_blocked_by_incomplete_dependency() {
        let mut issue = Issue::new("Dependent".to_string(), "Desc".to_string());
        let dependency = Issue::new("Dependency".to_string(), "Desc".to_string());

        issue.dependencies.push(dependency.id.clone());

        let mut resolved = HashMap::new();
        resolved.insert(dependency.id.clone(), &dependency);

        assert!(issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_not_blocked_when_dependency_is_done() {
        let mut issue = Issue::new("Dependent".to_string(), "Desc".to_string());
        let mut dependency = Issue::new("Dependency".to_string(), "Desc".to_string());
        dependency.state = State::Done;

        issue.dependencies.push(dependency.id.clone());

        let mut resolved = HashMap::new();
        resolved.insert(dependency.id.clone(), &dependency);

        assert!(!issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_blocked_by_unpassed_gate() {
        let mut issue = Issue::new("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());

        let resolved = HashMap::new();

        assert!(issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_blocked_by_pending_gate() {
        let mut issue = Issue::new("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Pending,
                updated_by: None,
                updated_at: Utc::now(),
            },
        );

        let resolved = HashMap::new();

        assert!(issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_blocked_by_failed_gate() {
        let mut issue = Issue::new("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Failed,
                updated_by: Some("human:reviewer".to_string()),
                updated_at: Utc::now(),
            },
        );

        let resolved = HashMap::new();

        assert!(issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_not_blocked_when_gate_passed() {
        let mut issue = Issue::new("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("human:reviewer".to_string()),
                updated_at: Utc::now(),
            },
        );

        let resolved = HashMap::new();

        assert!(!issue.is_blocked(&resolved));
    }


}
