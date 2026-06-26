//! Lexical analysis for query filter language
//!
//! Converts raw query strings into tokens for parsing.

use super::error::QueryParseError;
use std::str::FromStr;

type Result<T> = std::result::Result<T, QueryParseError>;

/// The recognized field name on the left of a `field:value` filter.
///
/// The lexer parses the field portion into this closed set (via [`FromStr`]) so
/// an unknown field is rejected at tokenization, and the parser's match over
/// fields is exhaustive with no string fallback. `value` semantics differ per
/// field and are resolved later: `State`/`Priority` parse into typed domain
/// values, while `Label`/`Assignee` carry open free-form strings.
///
/// # Examples
///
/// ```
/// use jit::query_engine::FilterField;
/// use std::str::FromStr;
///
/// assert_eq!(FilterField::from_str("state").unwrap(), FilterField::State);
/// assert_eq!(FilterField::from_str("assignee").unwrap(), FilterField::Assignee);
/// assert!(FilterField::from_str("colour").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterField {
    /// `state:` — filter by lifecycle state (closed, typed value).
    State,
    /// `label:` — filter by label (open value, supports wildcards).
    Label,
    /// `priority:` — filter by priority (closed, typed value).
    Priority,
    /// `assignee:` — filter by assignee (open value).
    Assignee,
}

impl FromStr for FilterField {
    type Err = QueryParseError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "state" => Ok(FilterField::State),
            "label" => Ok(FilterField::Label),
            "priority" => Ok(FilterField::Priority),
            "assignee" => Ok(FilterField::Assignee),
            other => Err(QueryParseError::UnknownField(other.to_string())),
        }
    }
}

/// Token types in the query language
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// Filter condition: field:value (e.g., "state:ready", "label:epic:auth")
    Filter { field: FilterField, value: String },
    /// Boolean AND operator
    And,
    /// Boolean OR operator
    Or,
    /// Boolean NOT operator
    Not,
    /// Left parenthesis
    LParen,
    /// Right parenthesis
    RParen,
    /// Special: unassigned issues
    Unassigned,
    /// Special: blocked issues
    Blocked,
}

/// Lexer for tokenizing query strings
pub struct Lexer<'a> {
    input: &'a str,
    position: usize,
}

impl<'a> Lexer<'a> {
    /// Create a new lexer for the given input
    pub fn new(input: &'a str) -> Self {
        Lexer { input, position: 0 }
    }

    /// Tokenize the entire input string
    pub fn tokenize(input: &str) -> Result<Vec<Token>> {
        let mut lexer = Lexer::new(input);
        let mut tokens = Vec::new();

        while let Some(token) = lexer.next_token()? {
            tokens.push(token);
        }

        Ok(tokens)
    }

    fn next_token(&mut self) -> Result<Option<Token>> {
        self.skip_whitespace();

        if self.is_at_end() {
            return Ok(None);
        }

        let ch = self.current_char();

        match ch {
            '(' => {
                self.advance();
                Ok(Some(Token::LParen))
            }
            ')' => {
                self.advance();
                Ok(Some(Token::RParen))
            }
            _ => {
                // Try to read a word
                let word = self.read_word()?;

                match word.as_str() {
                    "AND" => Ok(Some(Token::And)),
                    "OR" => Ok(Some(Token::Or)),
                    "NOT" => Ok(Some(Token::Not)),
                    "unassigned" => Ok(Some(Token::Unassigned)),
                    "blocked" => Ok(Some(Token::Blocked)),
                    _ => {
                        // Parse as filter (field:value)
                        self.parse_filter(&word)
                    }
                }
            }
        }
    }

    fn parse_filter(&self, word: &str) -> Result<Option<Token>> {
        // Split on first colon only (values can contain colons, like label:epic:auth)
        let Some(colon_pos) = word.find(':') else {
            return Err(QueryParseError::MissingColon(word.to_string()));
        };
        let field = &word[..colon_pos];
        let value = &word[colon_pos + 1..];

        if field.is_empty() {
            return Err(QueryParseError::EmptyField(word.to_string()));
        }
        if value.is_empty() {
            return Err(QueryParseError::EmptyValue(word.to_string()));
        }

        Ok(Some(Token::Filter {
            field: field.parse()?,
            value: value.to_string(),
        }))
    }

    fn read_word(&mut self) -> Result<String> {
        let start = self.position;

        while !self.is_at_end() {
            let ch = self.current_char();
            if ch.is_whitespace() || ch == '(' || ch == ')' {
                break;
            }
            self.advance();
        }

        if start == self.position {
            return Err(QueryParseError::UnexpectedChar(self.position));
        }

        Ok(self.input[start..self.position].to_string())
    }

    fn skip_whitespace(&mut self) {
        while !self.is_at_end() && self.current_char().is_whitespace() {
            self.advance();
        }
    }

    fn current_char(&self) -> char {
        self.input.chars().nth(self.position).unwrap()
    }

    fn advance(&mut self) {
        self.position += 1;
    }

    fn is_at_end(&self) -> bool {
        self.position >= self.input.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_field_from_str_valid() {
        assert_eq!(FilterField::from_str("state").unwrap(), FilterField::State);
        assert_eq!(FilterField::from_str("label").unwrap(), FilterField::Label);
        assert_eq!(
            FilterField::from_str("priority").unwrap(),
            FilterField::Priority
        );
        assert_eq!(
            FilterField::from_str("assignee").unwrap(),
            FilterField::Assignee
        );
    }

    #[test]
    fn test_filter_field_from_str_invalid() {
        let err = FilterField::from_str("colour").unwrap_err();
        assert_eq!(err, QueryParseError::UnknownField("colour".to_string()));
    }

    #[test]
    fn test_tokenize_simple_filter() {
        let tokens = Lexer::tokenize("state:ready").unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token::Filter {
                field: FilterField::State,
                value: "ready".to_string()
            }
        );
    }

    #[test]
    fn test_tokenize_label_with_colon() {
        let tokens = Lexer::tokenize("label:epic:auth").unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token::Filter {
                field: FilterField::Label,
                value: "epic:auth".to_string()
            }
        );
    }

    #[test]
    fn test_tokenize_unknown_field_errors() {
        let result = Lexer::tokenize("colour:blue");
        assert_eq!(
            result.unwrap_err(),
            QueryParseError::UnknownField("colour".to_string())
        );
    }

    #[test]
    fn test_tokenize_and_operator() {
        let tokens = Lexer::tokenize("state:ready AND priority:high").unwrap();
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens[0],
            Token::Filter {
                field: FilterField::State,
                value: "ready".to_string()
            }
        );
        assert_eq!(tokens[1], Token::And);
        assert_eq!(
            tokens[2],
            Token::Filter {
                field: FilterField::Priority,
                value: "high".to_string()
            }
        );
    }

    #[test]
    fn test_tokenize_or_operator() {
        let tokens = Lexer::tokenize("state:ready OR state:in_progress").unwrap();
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[1], Token::Or);
    }

    #[test]
    fn test_tokenize_not_operator() {
        let tokens = Lexer::tokenize("NOT blocked").unwrap();
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0], Token::Not);
        assert_eq!(tokens[1], Token::Blocked);
    }

    #[test]
    fn test_tokenize_parentheses() {
        let tokens = Lexer::tokenize("(state:ready OR state:done)").unwrap();
        assert_eq!(tokens.len(), 5);
        assert_eq!(tokens[0], Token::LParen);
        assert_eq!(tokens[4], Token::RParen);
    }

    #[test]
    fn test_tokenize_unassigned() {
        let tokens = Lexer::tokenize("unassigned").unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Unassigned);
    }

    #[test]
    fn test_tokenize_blocked() {
        let tokens = Lexer::tokenize("blocked").unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Blocked);
    }

    #[test]
    fn test_tokenize_complex_query() {
        let tokens = Lexer::tokenize(
            "(state:ready OR state:in_progress) AND label:milestone:v1.0 NOT blocked",
        )
        .unwrap();

        assert_eq!(tokens.len(), 9);
        assert_eq!(tokens[0], Token::LParen);
        assert_eq!(tokens[2], Token::Or);
        assert_eq!(tokens[4], Token::RParen);
        assert_eq!(tokens[5], Token::And);
        assert_eq!(tokens[7], Token::Not);
        assert_eq!(tokens[8], Token::Blocked);
    }

    #[test]
    fn test_tokenize_implicit_and() {
        // Space between filters means AND
        let tokens = Lexer::tokenize("state:ready priority:high").unwrap();
        assert_eq!(tokens.len(), 2);
        // No explicit AND token - parser will handle implicit AND
    }

    #[test]
    fn test_tokenize_error_no_colon() {
        let result = Lexer::tokenize("invalidfilter");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("expected format 'field:value'"));
    }

    #[test]
    fn test_tokenize_error_empty_field() {
        let result = Lexer::tokenize(":value");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("field cannot be empty"));
    }

    #[test]
    fn test_tokenize_error_empty_value() {
        let result = Lexer::tokenize("field:");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("value cannot be empty"));
    }

    #[test]
    fn test_tokenize_whitespace_handling() {
        let tokens = Lexer::tokenize("  state:ready   AND   priority:high  ").unwrap();
        assert_eq!(tokens.len(), 3);
        // Whitespace should be ignored
    }
}
