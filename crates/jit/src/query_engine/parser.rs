//! Parser for query filter language
//!
//! Builds an abstract syntax tree (AST) from tokens.

use super::lexer::Token;
use anyhow::{anyhow, Result};

/// Abstract syntax tree node for query expressions
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryExpr {
    /// Single condition
    Condition(QueryCondition),
    /// Logical AND
    And(Box<QueryExpr>, Box<QueryExpr>),
    /// Logical OR
    Or(Box<QueryExpr>, Box<QueryExpr>),
    /// Logical NOT
    Not(Box<QueryExpr>),
}

/// Individual query conditions
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryCondition {
    /// Filter by state (value unparsed, e.g., "ready")
    State(String),
    /// Filter by label (supports wildcards, e.g., "epic:*")
    Label(String),
    /// Filter by priority (value unparsed, e.g., "high")
    Priority(String),
    /// Filter by assignee
    Assignee(String),
    /// Issues with no assignee
    Unassigned,
    /// Issues blocked by dependencies or gates
    Blocked,
}

/// Parser for building AST from tokens
pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    /// Create a new parser
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser {
            tokens,
            position: 0,
        }
    }

    /// Parse tokens into an expression tree
    pub fn parse(tokens: Vec<Token>) -> Result<QueryExpr> {
        let mut parser = Parser::new(tokens);
        parser.parse_expr()
    }

    /// Parse OR expression (lowest precedence)
    fn parse_expr(&mut self) -> Result<QueryExpr> {
        let mut left = self.parse_term()?;

        while self.match_token(&Token::Or) {
            self.advance();
            let right = self.parse_term()?;
            left = QueryExpr::Or(Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    /// Parse AND expression (medium precedence)
    /// Also handles implicit AND (two conditions next to each other)
    fn parse_term(&mut self) -> Result<QueryExpr> {
        let mut left = self.parse_factor()?;

        while !self.is_at_end()
            && !self.match_token(&Token::Or)
            && !self.match_token(&Token::RParen)
        {
            // Check for explicit AND or implicit AND
            if self.match_token(&Token::And) {
                self.advance();
            }
            // If next token is a condition/NOT/LParen, treat as implicit AND
            if self.is_condition_start() {
                let right = self.parse_factor()?;
                left = QueryExpr::And(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }

        Ok(left)
    }

    /// Parse NOT expression and atoms (highest precedence)
    fn parse_factor(&mut self) -> Result<QueryExpr> {
        if self.match_token(&Token::Not) {
            self.advance();
            let inner = self.parse_factor()?;
            return Ok(QueryExpr::Not(Box::new(inner)));
        }

        if self.match_token(&Token::LParen) {
            self.advance();
            let expr = self.parse_expr()?;

            if !self.match_token(&Token::RParen) {
                return Err(anyhow!("Expected closing parenthesis"));
            }
            self.advance();
            return Ok(expr);
        }

        self.parse_condition()
    }

    /// Parse a single condition (atom)
    fn parse_condition(&mut self) -> Result<QueryExpr> {
        if self.is_at_end() {
            return Err(anyhow!("Unexpected end of query"));
        }

        let token = self.current_token().clone();
        self.advance();

        match token {
            Token::Filter { field, value } => {
                let condition = match field.as_str() {
                    "state" => QueryCondition::State(value),
                    "label" => QueryCondition::Label(value),
                    "priority" => QueryCondition::Priority(value),
                    "assignee" => QueryCondition::Assignee(value),
                    _ => return Err(anyhow!("Unknown filter field: '{}'", field)),
                };
                Ok(QueryExpr::Condition(condition))
            }
            Token::Unassigned => Ok(QueryExpr::Condition(QueryCondition::Unassigned)),
            Token::Blocked => Ok(QueryExpr::Condition(QueryCondition::Blocked)),
            _ => Err(anyhow!("Expected condition, found {:?}", token)),
        }
    }

    fn is_condition_start(&self) -> bool {
        if self.is_at_end() {
            return false;
        }
        matches!(
            self.current_token(),
            Token::Filter { .. } | Token::Unassigned | Token::Blocked | Token::Not | Token::LParen
        )
    }

    fn match_token(&self, expected: &Token) -> bool {
        if self.is_at_end() {
            return false;
        }
        std::mem::discriminant(self.current_token()) == std::mem::discriminant(expected)
    }

    fn current_token(&self) -> &Token {
        &self.tokens[self.position]
    }

    fn advance(&mut self) {
        if !self.is_at_end() {
            self.position += 1;
        }
    }

    fn is_at_end(&self) -> bool {
        self.position >= self.tokens.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_engine::lexer::Lexer;

    #[test]
    fn test_parse_single_condition() {
        let tokens = Lexer::tokenize("state:ready").unwrap();
        let expr = Parser::parse(tokens).unwrap();

        assert_eq!(
            expr,
            QueryExpr::Condition(QueryCondition::State("ready".to_string()))
        );
    }

    #[test]
    fn test_parse_and_operator() {
        let tokens = Lexer::tokenize("state:ready AND priority:high").unwrap();
        let expr = Parser::parse(tokens).unwrap();

        match expr {
            QueryExpr::And(left, right) => {
                assert_eq!(
                    *left,
                    QueryExpr::Condition(QueryCondition::State("ready".to_string()))
                );
                assert_eq!(
                    *right,
                    QueryExpr::Condition(QueryCondition::Priority("high".to_string()))
                );
            }
            _ => panic!("Expected And expression"),
        }
    }

    #[test]
    fn test_parse_or_operator() {
        let tokens = Lexer::tokenize("state:ready OR state:done").unwrap();
        let expr = Parser::parse(tokens).unwrap();

        match expr {
            QueryExpr::Or(left, right) => {
                assert_eq!(
                    *left,
                    QueryExpr::Condition(QueryCondition::State("ready".to_string()))
                );
                assert_eq!(
                    *right,
                    QueryExpr::Condition(QueryCondition::State("done".to_string()))
                );
            }
            _ => panic!("Expected Or expression"),
        }
    }

    #[test]
    fn test_parse_not_operator() {
        let tokens = Lexer::tokenize("NOT blocked").unwrap();
        let expr = Parser::parse(tokens).unwrap();

        match expr {
            QueryExpr::Not(inner) => {
                assert_eq!(*inner, QueryExpr::Condition(QueryCondition::Blocked));
            }
            _ => panic!("Expected Not expression"),
        }
    }

    #[test]
    fn test_parse_parentheses() {
        let tokens = Lexer::tokenize("(state:ready OR state:done)").unwrap();
        let expr = Parser::parse(tokens).unwrap();

        match expr {
            QueryExpr::Or(_, _) => {} // Correct
            _ => panic!("Expected Or expression"),
        }
    }

    #[test]
    fn test_parse_implicit_and() {
        let tokens = Lexer::tokenize("state:ready priority:high").unwrap();
        let expr = Parser::parse(tokens).unwrap();

        match expr {
            QueryExpr::And(left, right) => {
                assert_eq!(
                    *left,
                    QueryExpr::Condition(QueryCondition::State("ready".to_string()))
                );
                assert_eq!(
                    *right,
                    QueryExpr::Condition(QueryCondition::Priority("high".to_string()))
                );
            }
            _ => panic!("Expected And expression for implicit AND"),
        }
    }

    #[test]
    fn test_parse_complex_expression() {
        let tokens =
            Lexer::tokenize("(state:ready OR state:in_progress) AND priority:high NOT blocked")
                .unwrap();
        let expr = Parser::parse(tokens).unwrap();

        // Should parse as: ((state:ready OR state:in_progress) AND priority:high) AND (NOT blocked)
        match expr {
            QueryExpr::And(_, _) => {} // At least an AND at top level
            _ => panic!("Expected And at top level"),
        }
    }

    #[test]
    fn test_parse_label_condition() {
        let tokens = Lexer::tokenize("label:epic:auth").unwrap();
        let expr = Parser::parse(tokens).unwrap();

        assert_eq!(
            expr,
            QueryExpr::Condition(QueryCondition::Label("epic:auth".to_string()))
        );
    }

    #[test]
    fn test_parse_unassigned() {
        let tokens = Lexer::tokenize("unassigned").unwrap();
        let expr = Parser::parse(tokens).unwrap();

        assert_eq!(expr, QueryExpr::Condition(QueryCondition::Unassigned));
    }

    #[test]
    fn test_parse_blocked() {
        let tokens = Lexer::tokenize("blocked").unwrap();
        let expr = Parser::parse(tokens).unwrap();

        assert_eq!(expr, QueryExpr::Condition(QueryCondition::Blocked));
    }

    #[test]
    fn test_parse_error_unknown_field() {
        let tokens = vec![Token::Filter {
            field: "unknown".to_string(),
            value: "value".to_string(),
        }];
        let result = Parser::parse(tokens);

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unknown filter field"));
    }

    #[test]
    fn test_parse_error_unclosed_paren() {
        let tokens = Lexer::tokenize("(state:ready").unwrap();
        let result = Parser::parse(tokens);

        assert!(result.is_err());
    }

    #[test]
    fn test_operator_precedence() {
        // OR has lower precedence than AND
        // "a AND b OR c" should parse as "(a AND b) OR c"
        let tokens = Lexer::tokenize("state:ready AND priority:high OR state:done").unwrap();
        let expr = Parser::parse(tokens).unwrap();

        match expr {
            QueryExpr::Or(left, _right) => {
                // Left side should be AND
                match *left {
                    QueryExpr::And(_, _) => {} // Correct
                    _ => panic!("Expected AND on left side of OR"),
                }
            }
            _ => panic!("Expected OR at top level"),
        }
    }
}
