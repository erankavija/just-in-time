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
//!
//! # Examples
//!
//! ```
//! use jit::domain::{Issue, State};
//! use jit::query::QueryFilter;
//!
//! # fn example() -> anyhow::Result<()> {
//! let filter = QueryFilter::parse("state:ready AND priority:high")?;
//! # Ok(())
//! # }
//! ```

mod evaluator;
mod lexer;
mod parser;

pub use evaluator::{QueryContext, QueryEvaluator};
pub use lexer::{Lexer, Token};
pub use parser::{Parser, QueryCondition, QueryExpr};

use crate::domain::Issue;
use anyhow::Result;

/// A compiled query filter ready for evaluation
pub struct QueryFilter {
    expr: QueryExpr,
}

impl QueryFilter {
    /// Parse a query string into a filter
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::query::QueryFilter;
    ///
    /// # fn example() -> anyhow::Result<()> {
    /// let filter = QueryFilter::parse("state:ready")?;
    /// let filter = QueryFilter::parse("state:ready AND priority:high")?;
    /// let filter = QueryFilter::parse("(state:ready OR state:done) NOT blocked")?;
    /// # Ok(())
    /// # }
    /// ```
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

    fn create_test_issue(id: &str, state: State, priority: Priority, labels: Vec<&str>) -> Issue {
        Issue {
            id: id.to_string(),
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
    fn test_filter_by_state() {
        let issues = vec![
            create_test_issue("1", State::Ready, Priority::Normal, vec![]),
            create_test_issue("2", State::Done, Priority::Normal, vec![]),
            create_test_issue("3", State::Ready, Priority::High, vec![]),
        ];

        let filter = QueryFilter::parse("state:ready").unwrap();
        let matched = filter.filter_issues(&issues).unwrap();

        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].id, "1");
        assert_eq!(matched[1].id, "3");
    }

    #[test]
    fn test_filter_by_priority() {
        let issues = vec![
            create_test_issue("1", State::Ready, Priority::High, vec![]),
            create_test_issue("2", State::Ready, Priority::Normal, vec![]),
        ];

        let filter = QueryFilter::parse("priority:high").unwrap();
        let matched = filter.filter_issues(&issues).unwrap();

        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].id, "1");
    }
}
