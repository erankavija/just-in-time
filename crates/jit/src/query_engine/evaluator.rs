//! Evaluator for query expressions against Issue objects
//!
//! This layer contains all domain knowledge and reuses existing Issue methods.

use super::parser::{QueryCondition, QueryExpr};
use crate::domain::identity::IssueId;
use crate::domain::Issue;
use crate::labels;
use std::collections::HashMap;

/// Context needed for evaluating queries
///
/// Contains all issues for dependency graph evaluation (blocking checks).
pub struct QueryContext<'a> {
    pub all_issues: HashMap<IssueId, &'a Issue>,
}

impl<'a> QueryContext<'a> {
    /// Create context from issue collection
    pub fn from_issues(issues: &'a [Issue]) -> Self {
        let all_issues = crate::domain::queries::build_issue_map(issues);
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
            // Conditions carry typed values parsed at parse time, so matching is
            // a direct comparison (no fail-open FromStr round-trip here).
            QueryCondition::State(state) => issue.state == *state,

            QueryCondition::Label(pattern) => {
                // Use centralized label matching from labels module
                labels::matches_pattern(&issue.labels, pattern)
            }

            QueryCondition::Priority(priority) => issue.priority == *priority,

            QueryCondition::Assignee(assignee) => issue.assignee.as_ref() == Some(assignee),

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
    use crate::domain::{Assignee, Priority, State};

    fn create_issue(
        id: &IssueId,
        state: State,
        priority: Priority,
        assignee: Option<Assignee>,
        labels: Vec<&str>,
        dependencies: Vec<IssueId>,
    ) -> Issue {
        Issue {
            id: id.clone(),
            title: format!("Test {}", id),
            description: String::new(),
            state,
            priority,
            assignee,
            dependencies,
            gates_required: vec![],
            gates_status: Default::default(),
            context: Default::default(),
            documents: vec![],
            labels: labels.iter().map(|s| s.to_string()).collect(),
            content_format: None,
            created_at: "2024-01-01T00:00:00Z".parse().unwrap(),
            updated_at: "2024-01-01T00:00:00Z".parse().unwrap(),
            first_ready_at: None,
            claimed_at: None,
            done_at: None,
            archived_from: None,
        }
    }

    #[test]
    fn test_eval_state_condition() {
        let issue = create_issue(
            &crate::test_utils::issue_id(0x4d6d_537d_80cc_5cce_1e28_447c_f692_926d),
            State::Ready,
            Priority::Normal,
            None,
            vec![],
            vec![],
        );
        let issues = [issue.clone()];
        let context = QueryContext::from_issues(&issues);

        let cond = QueryCondition::State(State::Ready);
        assert!(QueryEvaluator::eval_condition(&cond, &issue, &context));

        let cond = QueryCondition::State(State::Done);
        assert!(!QueryEvaluator::eval_condition(&cond, &issue, &context));
    }

    #[test]
    fn test_eval_priority_condition() {
        let issue = create_issue(
            &crate::test_utils::issue_id(0x4d6d_537d_80cc_5cce_1e28_447c_f692_926d),
            State::Ready,
            Priority::High,
            None,
            vec![],
            vec![],
        );
        let issues = [issue.clone()];
        let context = QueryContext::from_issues(&issues);

        let cond = QueryCondition::Priority(Priority::High);
        assert!(QueryEvaluator::eval_condition(&cond, &issue, &context));

        let cond = QueryCondition::Priority(Priority::Low);
        assert!(!QueryEvaluator::eval_condition(&cond, &issue, &context));
    }

    #[test]
    fn test_eval_label_exact_match() {
        let issue = create_issue(
            &crate::test_utils::issue_id(0x4d6d_537d_80cc_5cce_1e28_447c_f692_926d),
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
            &crate::test_utils::issue_id(0x4d6d_537d_80cc_5cce_1e28_447c_f692_926d),
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
            &crate::test_utils::issue_id(0x4d6d_537d_80cc_5cce_1e28_447c_f692_926d),
            State::Ready,
            Priority::Normal,
            Some("agent:worker-1".parse().unwrap()),
            vec![],
            vec![],
        );
        let issues = [issue.clone()];
        let context = QueryContext::from_issues(&issues);

        let cond = QueryCondition::Assignee("agent:worker-1".parse().unwrap());
        assert!(QueryEvaluator::eval_condition(&cond, &issue, &context));

        let cond = QueryCondition::Assignee("agent:worker-2".parse().unwrap());
        assert!(!QueryEvaluator::eval_condition(&cond, &issue, &context));
    }

    #[test]
    fn test_eval_unassigned_condition() {
        let assigned = create_issue(
            &crate::test_utils::issue_id(0x4d6d_537d_80cc_5cce_1e28_447c_f692_926d),
            State::Ready,
            Priority::Normal,
            Some("agent:worker-1".parse().unwrap()),
            vec![],
            vec![],
        );
        let unassigned = create_issue(
            &crate::test_utils::issue_id(0xf80e_59d7_a3b8_cbf8_57ae_21a2_6463_191f),
            State::Ready,
            Priority::Normal,
            None,
            vec![],
            vec![],
        );

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
        let dep = create_issue(
            &crate::test_utils::issue_id(0x7af7_5d08_8331_30d3_05c2_2da0_f4a6_98f0),
            State::Ready,
            Priority::Normal,
            None,
            vec![],
            vec![],
        );
        let blocked = create_issue(
            &crate::test_utils::issue_id(0xa715_50ab_b754_0cce_3ea8_599e_3e96_88bd),
            State::Backlog,
            Priority::Normal,
            None,
            vec![],
            vec![crate::test_utils::issue_id(
                0x7af75d08833130d305c22da0f4a698f0,
            )],
        );
        let not_blocked = create_issue(
            &crate::test_utils::issue_id(0x320a_5274_a9f2_3a1a_a166_a253_c646_cd52),
            State::Ready,
            Priority::Normal,
            None,
            vec![],
            vec![],
        );

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
        let issue = create_issue(
            &crate::test_utils::issue_id(0x4d6d_537d_80cc_5cce_1e28_447c_f692_926d),
            State::Ready,
            Priority::High,
            None,
            vec![],
            vec![],
        );
        let issues = [issue.clone()];
        let context = QueryContext::from_issues(&issues);

        let expr = QueryExpr::And(
            Box::new(QueryExpr::Condition(QueryCondition::State(State::Ready))),
            Box::new(QueryExpr::Condition(QueryCondition::Priority(
                Priority::High,
            ))),
        );

        assert!(QueryEvaluator::matches(&expr, &issue, &context));
    }

    #[test]
    fn test_eval_or_expression() {
        let issue = create_issue(
            &crate::test_utils::issue_id(0x4d6d_537d_80cc_5cce_1e28_447c_f692_926d),
            State::Ready,
            Priority::Normal,
            None,
            vec![],
            vec![],
        );
        let issues = [issue.clone()];
        let context = QueryContext::from_issues(&issues);

        let expr = QueryExpr::Or(
            Box::new(QueryExpr::Condition(QueryCondition::State(State::Ready))),
            Box::new(QueryExpr::Condition(QueryCondition::State(State::Done))),
        );

        assert!(QueryEvaluator::matches(&expr, &issue, &context));
    }

    #[test]
    fn test_eval_not_expression() {
        let issue = create_issue(
            &crate::test_utils::issue_id(0x4d6d_537d_80cc_5cce_1e28_447c_f692_926d),
            State::Ready,
            Priority::Normal,
            None,
            vec![],
            vec![],
        );
        let issues = [issue.clone()];
        let context = QueryContext::from_issues(&issues);

        let expr = QueryExpr::Not(Box::new(QueryExpr::Condition(QueryCondition::Blocked)));

        assert!(QueryEvaluator::matches(&expr, &issue, &context));
    }

    #[test]
    fn test_eval_complex_expression() {
        let issue1 = create_issue(
            &crate::test_utils::issue_id(0x4d6d_537d_80cc_5cce_1e28_447c_f692_926d),
            State::Ready,
            Priority::High,
            None,
            vec![],
            vec![],
        );
        let issue2 = create_issue(
            &crate::test_utils::issue_id(0xf80e_59d7_a3b8_cbf8_57ae_21a2_6463_191f),
            State::Done,
            Priority::Normal,
            None,
            vec![],
            vec![],
        );
        let issue3 = create_issue(
            &crate::test_utils::issue_id(0xe93c_56db_9922_c57f_0008_1a4f_b309_dea0),
            State::InProgress,
            Priority::Low,
            None,
            vec![],
            vec![],
        );

        let issues = [issue1.clone(), issue2.clone(), issue3.clone()];
        let context = QueryContext::from_issues(&issues);

        // (state:ready OR state:done) AND priority:high
        let expr = QueryExpr::And(
            Box::new(QueryExpr::Or(
                Box::new(QueryExpr::Condition(QueryCondition::State(State::Ready))),
                Box::new(QueryExpr::Condition(QueryCondition::State(State::Done))),
            )),
            Box::new(QueryExpr::Condition(QueryCondition::Priority(
                Priority::High,
            ))),
        );

        assert!(QueryEvaluator::matches(&expr, &issue1, &context)); // ready + high
        assert!(!QueryEvaluator::matches(&expr, &issue2, &context)); // done + normal
        assert!(!QueryEvaluator::matches(&expr, &issue3, &context)); // in_progress + low
    }
}
