//! Lexical analysis for query filter language
//!
//! Converts raw query strings into tokens for parsing.

use anyhow::{anyhow, Result};

/// Token types in the query language
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// Filter condition: field:value (e.g., "state:ready", "label:epic:auth")
    Filter { field: String, value: String },
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
        if !word.contains(':') {
            return Err(anyhow!(
                "Invalid filter '{}': expected format 'field:value'",
                word
            ));
        }

        // Split on first colon only (values can contain colons, like label:epic:auth)
        let colon_pos = word.find(':').unwrap();
        let field = &word[..colon_pos];
        let value = &word[colon_pos + 1..];

        if field.is_empty() {
            return Err(anyhow!("Filter field cannot be empty: '{}'", word));
        }
        if value.is_empty() {
            return Err(anyhow!("Filter value cannot be empty: '{}'", word));
        }

        Ok(Some(Token::Filter {
            field: field.to_string(),
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
            return Err(anyhow!(
                "Unexpected character at position {}",
                self.position
            ));
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
    fn test_tokenize_simple_filter() {
        let tokens = Lexer::tokenize("state:ready").unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token::Filter {
                field: "state".to_string(),
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
                field: "label".to_string(),
                value: "epic:auth".to_string()
            }
        );
    }

    #[test]
    fn test_tokenize_and_operator() {
        let tokens = Lexer::tokenize("state:ready AND priority:high").unwrap();
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens[0],
            Token::Filter {
                field: "state".to_string(),
                value: "ready".to_string()
            }
        );
        assert_eq!(tokens[1], Token::And);
        assert_eq!(
            tokens[2],
            Token::Filter {
                field: "priority".to_string(),
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
