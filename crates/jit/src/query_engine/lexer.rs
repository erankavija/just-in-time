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
/// values, while `Assignee` parses into a typed assignee and `Label` carries an open string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterField {
    /// `state:` — filter by lifecycle state (closed, typed value).
    State,
    /// `label:` — filter by label (open value, supports wildcards).
    Label,
    /// `priority:` — filter by priority (closed, typed value).
    Priority,
    /// `assignee:` — filter by assignee (`type:identifier` value).
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
    /// Byte offset into `input`, always on a character boundary.
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

        match self.peek_char() {
            None => Ok(None),
            Some('(') => {
                self.advance();
                Ok(Some(Token::LParen))
            }
            Some(')') => {
                self.advance();
                Ok(Some(Token::RParen))
            }
            Some(_) => {
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

        while self
            .peek_char()
            .is_some_and(|ch| !ch.is_whitespace() && ch != '(' && ch != ')')
        {
            self.advance();
        }

        if start == self.position {
            return Err(QueryParseError::UnexpectedChar(self.position));
        }

        Ok(self.input[start..self.position].to_string())
    }

    fn skip_whitespace(&mut self) {
        while self.peek_char().is_some_and(char::is_whitespace) {
            self.advance();
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.position..].chars().next()
    }

    fn advance(&mut self) {
        self.position += self.peek_char().map_or(0, char::len_utf8);
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

    fn filter(field: FilterField, value: &str) -> Token {
        Token::Filter {
            field,
            value: value.to_string(),
        }
    }

    #[test]
    fn test_tokenize_non_ascii_value_is_kept_verbatim() {
        assert_eq!(
            Lexer::tokenize("label:ä").unwrap(),
            vec![filter(FilterField::Label, "ä")]
        );
        assert_eq!(
            Lexer::tokenize("assignee:human:日本語").unwrap(),
            vec![filter(FilterField::Assignee, "human:日本語")]
        );
    }

    #[test]
    fn test_tokenize_multibyte_char_followed_by_more_terms() {
        assert_eq!(
            Lexer::tokenize("label:ä state:ready").unwrap(),
            vec![
                filter(FilterField::Label, "ä"),
                filter(FilterField::State, "ready"),
            ]
        );
    }

    #[test]
    fn test_tokenize_non_ascii_matches_ascii_equivalent_with_operators() {
        let non_ascii = Lexer::tokenize("label:ä AND (state:ready OR assignee:é😀)").unwrap();
        let ascii = Lexer::tokenize("label:a AND (state:ready OR assignee:b)").unwrap();
        let substitute = |token: &Token| match token {
            Token::Filter { field, value } => Token::Filter {
                field: *field,
                value: match value.as_str() {
                    "ä" => "a",
                    "é😀" => "b",
                    other => other,
                }
                .to_string(),
            },
            other => other.clone(),
        };
        assert_eq!(non_ascii.iter().map(substitute).collect::<Vec<_>>(), ascii);
    }

    #[test]
    fn test_tokenize_non_ascii_whitespace_separates_terms() {
        assert_eq!(
            Lexer::tokenize("label:ä\u{a0}blocked\u{3000}unassigned").unwrap(),
            vec![
                filter(FilterField::Label, "ä"),
                Token::Blocked,
                Token::Unassigned
            ]
        );
    }

    #[test]
    fn test_tokenize_non_ascii_field_is_unknown_field_error() {
        assert_eq!(
            Lexer::tokenize("é:x").unwrap_err(),
            QueryParseError::UnknownField("é".to_string())
        );
    }

    /// Strings over every Unicode scalar value; `any::<String>()` excludes control characters.
    fn arbitrary_utf8() -> impl proptest::strategy::Strategy<Value = String> {
        use proptest::strategy::Strategy;
        proptest::collection::vec(proptest::char::any(), 0..64).prop_map(String::from_iter)
    }

    proptest::proptest! {
        #[test]
        fn test_tokenize_never_panics_on_arbitrary_utf8(input in arbitrary_utf8()) {
            let _ = Lexer::tokenize(&input);
        }
    }
}
