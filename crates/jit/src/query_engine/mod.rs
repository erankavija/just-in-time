//! Query filter engine for issue filtering and bulk operations
//!
//! Provides a boolean query language for filtering issues:
//! - Simple filters: `state:ready`, `label:epic:auth`, `priority:high`
//! - Boolean operators: `AND`, `OR`, `NOT`
//! - Parentheses for grouping: `(state:ready OR state:done) AND priority:high`
//! - Special conditions: `unassigned`, `blocked`
//!
//! # Architecture
//!
//! The query engine follows a three-layer architecture:
//! 1. **Lexer**: Converts query string to tokens
//! 2. **Parser**: Builds abstract syntax tree (AST) from tokens
//! 3. **Evaluator**: Evaluates AST against Issue objects

#[cfg(test)]
use crate::domain::identity::IssueId;
mod error;
mod evaluator;
mod lexer;
mod parser;

pub use error::QueryParseError;
pub use evaluator::{QueryContext, QueryEvaluator};
pub use lexer::{FilterField, Lexer, Token};
pub use parser::{Parser, QueryCondition, QueryExpr};

use crate::domain::Issue;
use anyhow::Result;

/// A compiled query filter ready for evaluation
#[derive(Debug)]
pub struct QueryFilter {
    expr: QueryExpr,
}

impl QueryFilter {
    /// Parse a query string into a filter
    pub fn parse(query: &str) -> Result<Self> {
        let tokens = Lexer::tokenize(query)?;
        let expr = Parser::parse(tokens)?;
        Ok(QueryFilter { expr })
    }

    /// Check if an issue matches this filter
    ///
    /// Requires a QueryContext containing all issues for dependency checks.
    pub fn matches(&self, issue: &Issue, context: &QueryContext) -> bool {
        QueryEvaluator::matches(&self.expr, issue, context)
    }

    /// Filter a collection of issues
    ///
    /// Returns references to issues that match the filter.
    pub fn filter_issues<'a>(&self, issues: &'a [Issue]) -> Result<Vec<&'a Issue>> {
        let context = QueryContext::from_issues(issues);
        Ok(issues
            .iter()
            .filter(|i| self.matches(i, &context))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Issue, Priority, State};

    fn create_test_issue(
        id: &IssueId,
        state: State,
        priority: Priority,
        labels: Vec<&str>,
    ) -> Issue {
        Issue {
            id: id.clone(),
            title: format!("Test {}", id),
            description: String::new(),
            state,
            priority,
            assignee: None,
            dependencies: vec![],
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
    fn test_parse_simple_query() {
        let result = QueryFilter::parse("state:ready");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_complex_query() {
        let result = QueryFilter::parse("(state:ready OR state:done) AND priority:high");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_invalid_state_value_is_error_not_silent_no_match() {
        // Regression for the `state:notastate` fail-open bug: an invalid state
        // value must surface as a parse error instead of silently matching no
        // issue. The error must name the offending value.
        let err = QueryFilter::parse("state:notastate").unwrap_err();
        assert!(
            err.to_string().contains("notastate"),
            "error should name the invalid value, got: {err}"
        );
    }

    #[test]
    fn test_parse_invalid_priority_value_is_error() {
        let err = QueryFilter::parse("priority:nope").unwrap_err();
        assert!(err.to_string().contains("nope"), "got: {err}");
    }

    #[test]
    fn test_filter_by_state() {
        let issues = vec![
            create_test_issue(
                &crate::test_utils::issue_id(0x4d6d_537d_80cc_5cce_1e28_447c_f692_926d),
                State::Ready,
                Priority::Normal,
                vec![],
            ),
            create_test_issue(
                &crate::test_utils::issue_id(0xf80e_59d7_a3b8_cbf8_57ae_21a2_6463_191f),
                State::Done,
                Priority::Normal,
                vec![],
            ),
            create_test_issue(
                &crate::test_utils::issue_id(0xe93c_56db_9922_c57f_0008_1a4f_b309_dea0),
                State::Ready,
                Priority::High,
                vec![],
            ),
        ];

        let filter = QueryFilter::parse("state:ready").unwrap();
        let matched = filter.filter_issues(&issues).unwrap();

        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].id, issues[0].id);
        assert_eq!(matched[1].id, issues[2].id);
    }

    #[test]
    fn test_filter_by_priority() {
        let issues = vec![
            create_test_issue(
                &crate::test_utils::issue_id(0x4d6d_537d_80cc_5cce_1e28_447c_f692_926d),
                State::Ready,
                Priority::High,
                vec![],
            ),
            create_test_issue(
                &crate::test_utils::issue_id(0xf80e_59d7_a3b8_cbf8_57ae_21a2_6463_191f),
                State::Ready,
                Priority::Normal,
                vec![],
            ),
        ];

        let filter = QueryFilter::parse("priority:high").unwrap();
        let matched = filter.filter_issues(&issues).unwrap();

        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].id, issues[0].id);
    }
}
