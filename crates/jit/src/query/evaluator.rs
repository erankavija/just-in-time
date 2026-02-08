//! Evaluator for query expressions against Issue objects
//!
//! This layer contains all domain knowledge and reuses existing Issue methods.

use super::parser::{QueryCondition, QueryExpr};
use crate::domain::{Issue, Priority, State};
use crate::labels;
use std::collections::HashMap;
use std::str::FromStr;

/// Context needed for evaluating queries
///
/// Contains all issues for dependency graph evaluation (blocking checks).
pub struct QueryContext<'a> {
    pub all_issues: HashMap<String, &'a Issue>,
}

impl<'a> QueryContext<'a> {
    /// Create context from issue collection
    pub fn from_issues(issues: &'a [Issue]) -> Self {
        let all_issues = issues.iter().map(|i| (i.id.clone(), i)).collect();
        QueryContext { all_issues }
    }
}

/// Evaluator for query expressions
pub struct QueryEvaluator;

impl QueryEvaluator {
    /// Check if an issue matches a query expression
    pub fn matches(expr: &QueryExpr, issue: &Issue, context: &QueryContext) -> bool {
        match expr {
            QueryExpr::Condition(cond) => Self::eval_condition(cond, issue, context),
            QueryExpr::And(left, right) => {
                Self::matches(left, issue, context) && Self::matches(right, issue, context)
            }
            QueryExpr::Or(left, right) => {
                Self::matches(left, issue, context) || Self::matches(right, issue, context)
            }
            QueryExpr::Not(inner) => !Self::matches(inner, issue, context),
        }
    }

    fn eval_condition(cond: &QueryCondition, issue: &Issue, ctx: &QueryContext) -> bool {
        match cond {
            QueryCondition::State(s) => {
                // Use FromStr trait for parsing
                State::from_str(s)
                    .map(|state| issue.state == state)
                    .unwrap_or(false)
            }

            QueryCondition::Label(pattern) => {
                // Reuse label matching logic from query_by_label
                if pattern.ends_with(":*") {
                    // Wildcard: match all labels in namespace
                    let namespace = &pattern[..pattern.len() - 2];
                    issue.labels.iter().any(|label| {
                        labels::parse_label(label)
                            .map(|(ns, _)| ns == namespace)
                            .unwrap_or(false)
                    })
                } else {
                    // Exact match
                    issue.labels.contains(&pattern.to_string())
                }
            }

            QueryCondition::Priority(p) => {
                // Use FromStr trait for parsing
                Priority::from_str(p)
                    .map(|priority| issue.priority == priority)
                    .unwrap_or(false)
            }

            QueryCondition::Assignee(assignee) => {
                issue.assignee.as_deref() == Some(assignee.as_str())
            }

            QueryCondition::Unassigned => issue.assignee.is_none(),

            QueryCondition::Blocked => {
                // Reuse Issue::is_blocked method
                issue.is_blocked(&ctx.all_issues)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Priority, State};

    fn create_issue(
        id: &str,
        state: State,
        priority: Priority,
        assignee: Option<&str>,
        labels: Vec<&str>,
        dependencies: Vec<&str>,
    ) -> Issue {
        Issue {
            id: id.to_string(),
            title: format!("Test {}", id),
            description: String::new(),
            state,
            priority,
            assignee: assignee.map(|s| s.to_string()),
            dependencies: dependencies.iter().map(|s| s.to_string()).collect(),
            gates_required: vec![],
            gates_status: Default::default(),
            context: Default::default(),
            documents: vec![],
            labels: labels.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn test_eval_state_condition() {
        let issue = create_issue("1", State::Ready, Priority::Normal, None, vec![], vec![]);
        let issues = [issue.clone()];
        let context = QueryContext::from_issues(&issues);

        let cond = QueryCondition::State("ready".to_string());
        assert!(QueryEvaluator::eval_condition(&cond, &issue, &context));

        let cond = QueryCondition::State("done".to_string());
        assert!(!QueryEvaluator::eval_condition(&cond, &issue, &context));
    }

    #[test]
    fn test_eval_priority_condition() {
        let issue = create_issue("1", State::Ready, Priority::High, None, vec![], vec![]);
        let issues = [issue.clone()];
        let context = QueryContext::from_issues(&issues);

        let cond = QueryCondition::Priority("high".to_string());
        assert!(QueryEvaluator::eval_condition(&cond, &issue, &context));

        let cond = QueryCondition::Priority("low".to_string());
        assert!(!QueryEvaluator::eval_condition(&cond, &issue, &context));
    }

    #[test]
    fn test_eval_label_exact_match() {
        let issue = create_issue(
            "1",
            State::Ready,
            Priority::Normal,
            None,
            vec!["epic:auth", "type:task"],
            vec![],
        );
        let issues = [issue.clone()];
        let context = QueryContext::from_issues(&issues);

        let cond = QueryCondition::Label("epic:auth".to_string());
        assert!(QueryEvaluator::eval_condition(&cond, &issue, &context));

        let cond = QueryCondition::Label("epic:other".to_string());
        assert!(!QueryEvaluator::eval_condition(&cond, &issue, &context));
    }

    #[test]
    fn test_eval_label_wildcard() {
        let issue = create_issue(
            "1",
            State::Ready,
            Priority::Normal,
            None,
            vec!["epic:auth", "type:task"],
            vec![],
        );
        let issues = [issue.clone()];
        let context = QueryContext::from_issues(&issues);

        let cond = QueryCondition::Label("epic:*".to_string());
        assert!(QueryEvaluator::eval_condition(&cond, &issue, &context));

        let cond = QueryCondition::Label("milestone:*".to_string());
        assert!(!QueryEvaluator::eval_condition(&cond, &issue, &context));
    }

    #[test]
    fn test_eval_assignee_condition() {
        let issue = create_issue(
            "1",
            State::Ready,
            Priority::Normal,
            Some("agent:worker-1"),
            vec![],
            vec![],
        );
        let issues = [issue.clone()];
        let context = QueryContext::from_issues(&issues);

        let cond = QueryCondition::Assignee("agent:worker-1".to_string());
        assert!(QueryEvaluator::eval_condition(&cond, &issue, &context));

        let cond = QueryCondition::Assignee("agent:worker-2".to_string());
        assert!(!QueryEvaluator::eval_condition(&cond, &issue, &context));
    }

    #[test]
    fn test_eval_unassigned_condition() {
        let assigned = create_issue(
            "1",
            State::Ready,
            Priority::Normal,
            Some("agent:worker-1"),
            vec![],
            vec![],
        );
        let unassigned = create_issue("2", State::Ready, Priority::Normal, None, vec![], vec![]);

        let issues = [assigned.clone(), unassigned.clone()];
        let context = QueryContext::from_issues(&issues);

        assert!(!QueryEvaluator::eval_condition(
            &QueryCondition::Unassigned,
            &assigned,
            &context
        ));
        assert!(QueryEvaluator::eval_condition(
            &QueryCondition::Unassigned,
            &unassigned,
            &context
        ));
    }

    #[test]
    fn test_eval_blocked_condition() {
        let dep = create_issue("dep", State::Ready, Priority::Normal, None, vec![], vec![]);
        let blocked = create_issue(
            "blocked",
            State::Backlog,
            Priority::Normal,
            None,
            vec![],
            vec!["dep"],
        );
        let not_blocked = create_issue("ok", State::Ready, Priority::Normal, None, vec![], vec![]);

        let issues = [dep, blocked.clone(), not_blocked.clone()];
        let context = QueryContext::from_issues(&issues);

        assert!(QueryEvaluator::eval_condition(
            &QueryCondition::Blocked,
            &blocked,
            &context
        ));
        assert!(!QueryEvaluator::eval_condition(
            &QueryCondition::Blocked,
            &not_blocked,
            &context
        ));
    }

    #[test]
    fn test_eval_and_expression() {
        let issue = create_issue("1", State::Ready, Priority::High, None, vec![], vec![]);
        let issues = [issue.clone()];
        let context = QueryContext::from_issues(&issues);

        let expr = QueryExpr::And(
            Box::new(QueryExpr::Condition(QueryCondition::State(
                "ready".to_string(),
            ))),
            Box::new(QueryExpr::Condition(QueryCondition::Priority(
                "high".to_string(),
            ))),
        );

        assert!(QueryEvaluator::matches(&expr, &issue, &context));
    }

    #[test]
    fn test_eval_or_expression() {
        let issue = create_issue("1", State::Ready, Priority::Normal, None, vec![], vec![]);
        let issues = [issue.clone()];
        let context = QueryContext::from_issues(&issues);

        let expr = QueryExpr::Or(
            Box::new(QueryExpr::Condition(QueryCondition::State(
                "ready".to_string(),
            ))),
            Box::new(QueryExpr::Condition(QueryCondition::State(
                "done".to_string(),
            ))),
        );

        assert!(QueryEvaluator::matches(&expr, &issue, &context));
    }

    #[test]
    fn test_eval_not_expression() {
        let issue = create_issue("1", State::Ready, Priority::Normal, None, vec![], vec![]);
        let issues = [issue.clone()];
        let context = QueryContext::from_issues(&issues);

        let expr = QueryExpr::Not(Box::new(QueryExpr::Condition(QueryCondition::Blocked)));

        assert!(QueryEvaluator::matches(&expr, &issue, &context));
    }

    #[test]
    fn test_eval_complex_expression() {
        let issue1 = create_issue("1", State::Ready, Priority::High, None, vec![], vec![]);
        let issue2 = create_issue("2", State::Done, Priority::Normal, None, vec![], vec![]);
        let issue3 = create_issue("3", State::InProgress, Priority::Low, None, vec![], vec![]);

        let issues = [issue1.clone(), issue2.clone(), issue3.clone()];
        let context = QueryContext::from_issues(&issues);

        // (state:ready OR state:done) AND priority:high
        let expr = QueryExpr::And(
            Box::new(QueryExpr::Or(
                Box::new(QueryExpr::Condition(QueryCondition::State(
                    "ready".to_string(),
                ))),
                Box::new(QueryExpr::Condition(QueryCondition::State(
                    "done".to_string(),
                ))),
            )),
            Box::new(QueryExpr::Condition(QueryCondition::Priority(
                "high".to_string(),
            ))),
        );

        assert!(QueryEvaluator::matches(&expr, &issue1, &context)); // ready + high
        assert!(!QueryEvaluator::matches(&expr, &issue2, &context)); // done + normal
        assert!(!QueryEvaluator::matches(&expr, &issue3, &context)); // in_progress + low
    }
}
