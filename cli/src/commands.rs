use crate::domain::{Event, Gate, GateState, GateStatus, Issue, Priority, State};
use crate::graph::DependencyGraph;
use crate::storage::Storage;
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::collections::HashMap;

pub struct CommandExecutor {
    storage: Storage,
}

impl CommandExecutor {
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    pub fn init(&self) -> Result<()> {
        self.storage.init()?;
        println!("Initialized jit repository");
        Ok(())
    }

    pub fn create_issue(
        &self,
        title: String,
        description: String,
        priority: Priority,
        gates: Vec<String>,
    ) -> Result<String> {
        let mut issue = Issue::new(title, description);
        issue.priority = priority;
        issue.gates_required = gates;

        self.storage.save_issue(&issue)?;

        // Log event
        let event = Event::new_issue_created(&issue);
        self.storage.append_event(&event)?;

        Ok(issue.id)
    }

    pub fn list_issues(
        &self,
        state_filter: Option<State>,
        assignee_filter: Option<String>,
        priority_filter: Option<Priority>,
    ) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;

        let filtered = issues
            .into_iter()
            .filter(|issue| {
                if let Some(ref state) = state_filter {
                    if &issue.state != state {
                        return false;
                    }
                }
                if let Some(ref assignee) = assignee_filter {
                    if issue.assignee.as_ref() != Some(assignee) {
                        return false;
                    }
                }
                if let Some(ref priority) = priority_filter {
                    if &issue.priority != priority {
                        return false;
                    }
                }
                true
            })
            .collect();

        Ok(filtered)
    }

    pub fn show_issue(&self, id: &str) -> Result<Issue> {
        self.storage.load_issue(id)
    }

    pub fn update_issue(
        &self,
        id: &str,
        title: Option<String>,
        description: Option<String>,
        priority: Option<Priority>,
        state: Option<State>,
    ) -> Result<()> {
        let mut issue = self.storage.load_issue(id)?;

        if let Some(t) = title {
            issue.title = t;
        }
        if let Some(d) = description {
            issue.description = d;
        }
        if let Some(p) = priority {
            issue.priority = p;
        }
        let old_state = issue.state;

        if let Some(s) = state {
            // Validate state transition
            if s == State::Ready || s == State::Done {
                let issues = self.storage.list_issues()?;
                let issue_refs: Vec<&Issue> = issues.iter().collect();
                let resolved: HashMap<String, &Issue> =
                    issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

                if issue.is_blocked(&resolved) {
                    return Err(anyhow!("Cannot transition to {:?}: issue is blocked", s));
                }
            }
            issue.state = s;

            // Log state change event
            if old_state != issue.state {
                let event =
                    Event::new_issue_state_changed(issue.id.clone(), old_state, issue.state);
                self.storage.append_event(&event)?;

                // Log completion event if transitioning to Done
                if issue.state == State::Done {
                    let event = Event::new_issue_completed(issue.id.clone());
                    self.storage.append_event(&event)?;
                }
            }
        }

        self.storage.save_issue(&issue)?;
        Ok(())
    }

    pub fn delete_issue(&self, id: &str) -> Result<()> {
        self.storage.delete_issue(id)
    }

    pub fn assign_issue(&self, id: &str, assignee: String) -> Result<()> {
        let mut issue = self.storage.load_issue(id)?;
        issue.assignee = Some(assignee);
        self.storage.save_issue(&issue)?;
        Ok(())
    }

    pub fn claim_issue(&self, id: &str, assignee: String) -> Result<()> {
        let mut issue = self.storage.load_issue(id)?;

        if issue.assignee.is_some() {
            return Err(anyhow!("Issue is already assigned"));
        }

        issue.assignee = Some(assignee.clone());
        self.storage.save_issue(&issue)?;

        // Log event
        let event = Event::new_issue_claimed(issue.id.clone(), assignee);
        self.storage.append_event(&event)?;

        Ok(())
    }

    pub fn unassign_issue(&self, id: &str) -> Result<()> {
        let mut issue = self.storage.load_issue(id)?;
        issue.assignee = None;
        self.storage.save_issue(&issue)?;
        Ok(())
    }

    pub fn claim_next(&self, assignee: String, _filter: Option<String>) -> Result<String> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let resolved: HashMap<String, &Issue> =
            issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

        // Find first ready, unassigned issue with highest priority
        let mut candidates: Vec<&Issue> = issues
            .iter()
            .filter(|i| i.assignee.is_none() && !i.is_blocked(&resolved))
            .collect();

        candidates.sort_by_key(|i| match i.priority {
            Priority::Critical => 0,
            Priority::High => 1,
            Priority::Normal => 2,
            Priority::Low => 3,
        });

        if let Some(issue) = candidates.first() {
            let id = issue.id.clone();
            self.claim_issue(&id, assignee)?;
            Ok(id)
        } else {
            Err(anyhow!("No ready issues available"))
        }
    }

    pub fn add_dependency(&self, issue_id: &str, dep_id: &str) -> Result<()> {
        // Validate both issues exist
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        // Check for cycles
        graph.validate_add_dependency(issue_id, dep_id)?;

        // Add the dependency
        let mut issue = self.storage.load_issue(issue_id)?;
        if !issue.dependencies.contains(&dep_id.to_string()) {
            issue.dependencies.push(dep_id.to_string());
            self.storage.save_issue(&issue)?;
        }

        Ok(())
    }

    pub fn remove_dependency(&self, issue_id: &str, dep_id: &str) -> Result<()> {
        let mut issue = self.storage.load_issue(issue_id)?;
        issue.dependencies.retain(|d| d != dep_id);
        self.storage.save_issue(&issue)?;
        Ok(())
    }

    pub fn add_gate(&self, issue_id: &str, gate_key: String) -> Result<()> {
        let mut issue = self.storage.load_issue(issue_id)?;
        if !issue.gates_required.contains(&gate_key) {
            issue.gates_required.push(gate_key);
            self.storage.save_issue(&issue)?;
        }
        Ok(())
    }

    pub fn pass_gate(&self, issue_id: &str, gate_key: String, by: Option<String>) -> Result<()> {
        let mut issue = self.storage.load_issue(issue_id)?;

        if !issue.gates_required.contains(&gate_key) {
            return Err(anyhow!(
                "Gate '{}' is not required for this issue",
                gate_key
            ));
        }

        issue.gates_status.insert(
            gate_key.clone(),
            GateState {
                status: GateStatus::Passed,
                updated_by: by.clone(),
                updated_at: Utc::now(),
            },
        );

        self.storage.save_issue(&issue)?;

        // Log event
        let event = Event::new_gate_passed(issue.id.clone(), gate_key, by);
        self.storage.append_event(&event)?;

        Ok(())
    }

    pub fn fail_gate(&self, issue_id: &str, gate_key: String, by: Option<String>) -> Result<()> {
        let mut issue = self.storage.load_issue(issue_id)?;

        if !issue.gates_required.contains(&gate_key) {
            return Err(anyhow!(
                "Gate '{}' is not required for this issue",
                gate_key
            ));
        }

        issue.gates_status.insert(
            gate_key.clone(),
            GateState {
                status: GateStatus::Failed,
                updated_by: by.clone(),
                updated_at: Utc::now(),
            },
        );

        self.storage.save_issue(&issue)?;

        // Log event
        let event = Event::new_gate_failed(issue.id.clone(), gate_key, by);
        self.storage.append_event(&event)?;

        Ok(())
    }

    pub fn show_graph(&self, issue_id: &str) -> Result<Vec<Issue>> {
        let issue = self.storage.load_issue(issue_id)?;
        let mut result = vec![issue.clone()];

        // Recursively get dependencies
        let mut to_process = issue.dependencies.clone();
        let mut processed = std::collections::HashSet::new();

        while let Some(dep_id) = to_process.pop() {
            if processed.contains(&dep_id) {
                continue;
            }
            processed.insert(dep_id.clone());

            if let Ok(dep_issue) = self.storage.load_issue(&dep_id) {
                to_process.extend(dep_issue.dependencies.clone());
                result.push(dep_issue);
            }
        }

        Ok(result)
    }

    pub fn show_downstream(&self, issue_id: &str) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        let dependents = graph.get_transitive_dependents(issue_id);
        Ok(dependents.into_iter().cloned().collect())
    }

    pub fn show_roots(&self) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        let roots = graph.get_roots();
        Ok(roots.into_iter().cloned().collect())
    }

    pub fn validate(&self) -> Result<()> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let graph = DependencyGraph::new(&issue_refs);

        graph.validate_dag()?;
        println!("✓ Repository is valid");
        Ok(())
    }

    pub fn status(&self) -> Result<()> {
        let issues = self.storage.list_issues()?;
        let issue_refs: Vec<&Issue> = issues.iter().collect();
        let resolved: HashMap<String, &Issue> =
            issue_refs.iter().map(|i| (i.id.clone(), *i)).collect();

        let open = issues.iter().filter(|i| i.state == State::Open).count();
        let ready = issues.iter().filter(|i| i.state == State::Ready).count();
        let in_progress = issues
            .iter()
            .filter(|i| i.state == State::InProgress)
            .count();
        let done = issues.iter().filter(|i| i.state == State::Done).count();
        let blocked = issues.iter().filter(|i| i.is_blocked(&resolved)).count();

        println!("Status:");
        println!("  Open: {}", open);
        println!("  Ready: {}", ready);
        println!("  In Progress: {}", in_progress);
        println!("  Done: {}", done);
        println!("  Blocked: {}", blocked);

        Ok(())
    }

    // Registry commands
    pub fn list_gates(&self) -> Result<Vec<Gate>> {
        let registry = self.storage.load_gate_registry()?;
        Ok(registry.gates.into_values().collect())
    }

    pub fn add_gate_definition(
        &self,
        key: String,
        title: String,
        description: String,
        auto: bool,
        example_integration: Option<String>,
    ) -> Result<()> {
        let mut registry = self.storage.load_gate_registry()?;

        if registry.gates.contains_key(&key) {
            return Err(anyhow!("Gate '{}' already exists", key));
        }

        registry.gates.insert(
            key.clone(),
            Gate {
                key,
                title,
                description,
                auto,
                example_integration,
            },
        );

        self.storage.save_gate_registry(&registry)?;
        Ok(())
    }

    pub fn remove_gate_definition(&self, key: &str) -> Result<()> {
        let mut registry = self.storage.load_gate_registry()?;

        if !registry.gates.contains_key(key) {
            return Err(anyhow!("Gate '{}' not found", key));
        }

        registry.gates.remove(key);
        self.storage.save_gate_registry(&registry)?;
        Ok(())
    }

    pub fn show_gate_definition(&self, key: &str) -> Result<Gate> {
        let registry = self.storage.load_gate_registry()?;
        registry
            .gates
            .get(key)
            .cloned()
            .ok_or_else(|| anyhow!("Gate '{}' not found", key))
    }

    // Event commands
    pub fn tail_events(&self, n: usize) -> Result<Vec<Event>> {
        let events = self.storage.read_events()?;
        let start = events.len().saturating_sub(n);
        Ok(events[start..].to_vec())
    }

    pub fn query_events(
        &self,
        event_type: Option<String>,
        issue_id: Option<String>,
        limit: usize,
    ) -> Result<Vec<Event>> {
        let events = self.storage.read_events()?;

        let filtered: Vec<Event> = events
            .into_iter()
            .rev()
            .filter(|e| {
                if let Some(ref et) = event_type {
                    if e.get_type() != et {
                        return false;
                    }
                }
                if let Some(ref iid) = issue_id {
                    if e.get_issue_id() != iid {
                        return false;
                    }
                }
                true
            })
            .take(limit)
            .collect();

        Ok(filtered.into_iter().rev().collect())
    }
}

pub fn parse_priority(s: &str) -> Result<Priority> {
    match s.to_lowercase().as_str() {
        "low" => Ok(Priority::Low),
        "normal" => Ok(Priority::Normal),
        "high" => Ok(Priority::High),
        "critical" => Ok(Priority::Critical),
        _ => Err(anyhow!("Invalid priority: {}", s)),
    }
}

pub fn parse_state(s: &str) -> Result<State> {
    match s.to_lowercase().as_str() {
        "open" => Ok(State::Open),
        "ready" => Ok(State::Ready),
        "in_progress" | "inprogress" => Ok(State::InProgress),
        "done" => Ok(State::Done),
        "archived" => Ok(State::Archived),
        _ => Err(anyhow!("Invalid state: {}", s)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, CommandExecutor) {
        let temp_dir = TempDir::new().unwrap();
        let storage = Storage::new(temp_dir.path());
        let executor = CommandExecutor::new(storage);
        executor.init().unwrap();
        (temp_dir, executor)
    }

    #[test]
    fn test_create_and_show_issue() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Test Issue".to_string(),
                "Description".to_string(),
                Priority::High,
                vec![],
            )
            .unwrap();

        let issue = executor.show_issue(&id).unwrap();
        assert_eq!(issue.title, "Test Issue");
        assert_eq!(issue.priority, Priority::High);
    }

    #[test]
    fn test_list_issues_with_filters() {
        let (_temp, executor) = setup();

        executor
            .create_issue(
                "Issue 1".to_string(),
                "Desc".to_string(),
                Priority::High,
                vec![],
            )
            .unwrap();
        executor
            .create_issue(
                "Issue 2".to_string(),
                "Desc".to_string(),
                Priority::Low,
                vec![],
            )
            .unwrap();

        let all = executor.list_issues(None, None, None).unwrap();
        assert_eq!(all.len(), 2);

        let high_priority = executor
            .list_issues(None, None, Some(Priority::High))
            .unwrap();
        assert_eq!(high_priority.len(), 1);
    }

    #[test]
    fn test_update_issue() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Original".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor
            .update_issue(&id, Some("Updated".to_string()), None, None, None)
            .unwrap();

        let issue = executor.show_issue(&id).unwrap();
        assert_eq!(issue.title, "Updated");
    }

    #[test]
    fn test_add_dependency_prevents_cycles() {
        let (_temp, executor) = setup();

        let id1 = executor
            .create_issue(
                "Issue 1".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();
        let id2 = executor
            .create_issue(
                "Issue 2".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.add_dependency(&id1, &id2).unwrap();

        let result = executor.add_dependency(&id2, &id1);
        assert!(result.is_err());
    }

    #[test]
    fn test_claim_issue_requires_unassigned() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Issue".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec![],
            )
            .unwrap();

        executor.claim_issue(&id, "user1".to_string()).unwrap();

        let result = executor.claim_issue(&id, "user2".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_claim_next_returns_highest_priority() {
        let (_temp, executor) = setup();

        let _low = executor
            .create_issue("Low".to_string(), "Desc".to_string(), Priority::Low, vec![])
            .unwrap();
        let high_id = executor
            .create_issue(
                "High".to_string(),
                "Desc".to_string(),
                Priority::High,
                vec![],
            )
            .unwrap();

        let claimed_id = executor.claim_next("user".to_string(), None).unwrap();
        assert_eq!(claimed_id, high_id);
    }

    #[test]
    fn test_gate_operations() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Issue".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec!["review".to_string()],
            )
            .unwrap();

        executor
            .pass_gate(&id, "review".to_string(), Some("reviewer".to_string()))
            .unwrap();

        let issue = executor.show_issue(&id).unwrap();
        assert_eq!(
            issue.gates_status.get("review").unwrap().status,
            GateStatus::Passed
        );
    }

    #[test]
    fn test_cannot_transition_to_done_when_blocked() {
        let (_temp, executor) = setup();

        let id = executor
            .create_issue(
                "Issue".to_string(),
                "Desc".to_string(),
                Priority::Normal,
                vec!["review".to_string()],
            )
            .unwrap();

        let result = executor.update_issue(&id, None, None, None, Some(State::Done));
        assert!(result.is_err());
    }
}
