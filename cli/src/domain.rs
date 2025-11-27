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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    IssueCreated {
        id: String,
        issue_id: String,
        timestamp: DateTime<Utc>,
        title: String,
        priority: Priority,
    },
    IssueClaimed {
        id: String,
        issue_id: String,
        timestamp: DateTime<Utc>,
        assignee: String,
    },
    IssueStateChanged {
        id: String,
        issue_id: String,
        timestamp: DateTime<Utc>,
        from: State,
        to: State,
    },
    GatePassed {
        id: String,
        issue_id: String,
        timestamp: DateTime<Utc>,
        gate_key: String,
        updated_by: Option<String>,
    },
    GateFailed {
        id: String,
        issue_id: String,
        timestamp: DateTime<Utc>,
        gate_key: String,
        updated_by: Option<String>,
    },
    IssueCompleted {
        id: String,
        issue_id: String,
        timestamp: DateTime<Utc>,
    },
}

impl Event {
    pub fn new_issue_created(issue: &Issue) -> Self {
        Event::IssueCreated {
            id: Uuid::new_v4().to_string(),
            issue_id: issue.id.clone(),
            timestamp: Utc::now(),
            title: issue.title.clone(),
            priority: issue.priority,
        }
    }

    pub fn new_issue_claimed(issue_id: String, assignee: String) -> Self {
        Event::IssueClaimed {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            assignee,
        }
    }

    pub fn new_issue_state_changed(issue_id: String, from: State, to: State) -> Self {
        Event::IssueStateChanged {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            from,
            to,
        }
    }

    pub fn new_gate_passed(issue_id: String, gate_key: String, updated_by: Option<String>) -> Self {
        Event::GatePassed {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            gate_key,
            updated_by,
        }
    }

    pub fn new_gate_failed(issue_id: String, gate_key: String, updated_by: Option<String>) -> Self {
        Event::GateFailed {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
            gate_key,
            updated_by,
        }
    }

    pub fn new_issue_completed(issue_id: String) -> Self {
        Event::IssueCompleted {
            id: Uuid::new_v4().to_string(),
            issue_id,
            timestamp: Utc::now(),
        }
    }

    pub fn get_issue_id(&self) -> &str {
        match self {
            Event::IssueCreated { issue_id, .. } => issue_id,
            Event::IssueClaimed { issue_id, .. } => issue_id,
            Event::IssueStateChanged { issue_id, .. } => issue_id,
            Event::GatePassed { issue_id, .. } => issue_id,
            Event::GateFailed { issue_id, .. } => issue_id,
            Event::IssueCompleted { issue_id, .. } => issue_id,
        }
    }

    pub fn get_type(&self) -> &str {
        match self {
            Event::IssueCreated { .. } => "issue_created",
            Event::IssueClaimed { .. } => "issue_claimed",
            Event::IssueStateChanged { .. } => "issue_state_changed",
            Event::GatePassed { .. } => "gate_passed",
            Event::GateFailed { .. } => "gate_failed",
            Event::IssueCompleted { .. } => "issue_completed",
        }
    }
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
