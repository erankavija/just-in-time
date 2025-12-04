//! Full-text search using ripgrep.
//!
//! Provides search across issues and referenced documents using `ripgrep` as the backend.
//! This approach requires no index management and always searches the current state.
//!
//! # Example
//!
//! ```no_run
//! use jit::search::{search, SearchOptions};
//! use std::path::Path;
//!
//! let options = SearchOptions::default();
//! let results = search(Path::new(".jit"), "authentication", options).unwrap();
//! println!("Found {} matches", results.len());
//! ```
//!
//! # Ripgrep Requirement
//!
//! This module requires `ripgrep` (command `rg`) to be installed on the system.
//! If not found, search operations return a helpful error message with installation instructions.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Stdio};

/// Search result from ripgrep
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    /// Issue ID extracted from filename (None for non-issue files like documents)
    pub issue_id: Option<String>,
    /// File path relative to search root
    pub path: String,
    /// Line number where match was found
    pub line_number: u64,
    /// Full text of the matched line
    pub line_text: String,
    /// Individual match positions within the line
    pub matches: Vec<SearchMatch>,
}

/// Individual match within a search result
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchMatch {
    /// Matched text
    pub text: String,
    /// Start position in line
    pub start: usize,
    /// End position in line
    pub end: usize,
}

/// Configuration options for search
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// Case-sensitive search
    pub case_sensitive: bool,
    /// Use regex pattern matching (default: literal string)
    pub regex: bool,
    /// Number of context lines to include
    pub context_lines: usize,
    /// Maximum number of results to return
    pub max_results: Option<usize>,
    /// File pattern filter (e.g., "*.json", "*.md")
    pub file_pattern: Option<String>,
}

/// Execute ripgrep search in the specified directory
///
/// # Arguments
///
/// * `data_dir` - Directory to search in (typically `.jit/`)
/// * `query` - Search query string
/// * `options` - Search configuration options
///
/// # Returns
///
/// Vector of search results, sorted by file path and line number.
/// Returns empty vector if no matches found (not an error).
///
/// # Errors
///
/// Returns error if:
/// - Ripgrep is not installed
/// - Invalid regex pattern
/// - Search execution fails
pub fn search(data_dir: &Path, query: &str, options: SearchOptions) -> Result<Vec<SearchResult>> {
    // Check if ripgrep is available
    if which::which("rg").is_err() {
        bail!(
            "ripgrep (rg) is not installed\n\n\
             Ripgrep is required for search functionality.\n\n\
             Install ripgrep:\n\
             - Ubuntu/Debian: apt install ripgrep\n\
             - macOS: brew install ripgrep\n\
             - Windows: choco install ripgrep\n\
             - Other: https://github.com/BurntSushi/ripgrep#installation"
        );
    }

    // Build ripgrep command
    let mut cmd = Command::new("rg");
    cmd.arg("--json").arg("--no-heading").arg("--with-filename");

    if !options.case_sensitive {
        cmd.arg("--ignore-case");
    }

    if options.regex {
        cmd.arg("--regexp");
    } else {
        cmd.arg("--fixed-strings");
    }

    if options.context_lines > 0 {
        cmd.arg(format!("--context={}", options.context_lines));
    }

    if let Some(pattern) = &options.file_pattern {
        cmd.arg("--glob").arg(pattern);
    }

    cmd.arg(query)
        .arg(data_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Execute command
    let output = cmd.output()?;

    // Exit code 1 means no matches (not an error)
    if !output.status.success() && output.status.code() != Some(1) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("ripgrep failed: {}", stderr);
    }

    // Parse JSON output
    parse_ripgrep_json(&output.stdout, options.max_results)
}

/// Parse ripgrep JSON output into SearchResult structs
fn parse_ripgrep_json(json: &[u8], max: Option<usize>) -> Result<Vec<SearchResult>> {
    use std::io::BufRead;

    let mut results = Vec::new();
    let reader = std::io::BufReader::new(json);

    for line in reader.lines() {
        let line = line?;

        // Parse each JSON line
        let value: serde_json::Value = serde_json::from_str(&line)?;

        // Only process "match" events
        if value["type"] != "match" {
            continue;
        }

        let data = &value["data"];
        let path = data["path"]["text"].as_str().unwrap_or("");

        // Extract issue ID from filename
        let issue_id = extract_issue_id(path);

        let result = SearchResult {
            issue_id,
            path: path.to_string(),
            line_number: data["line_number"].as_u64().unwrap_or(0),
            line_text: data["lines"]["text"].as_str().unwrap_or("").to_string(),
            matches: data["submatches"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| {
                            Some(SearchMatch {
                                text: m["match"]["text"].as_str()?.to_string(),
                                start: m["start"].as_u64()? as usize,
                                end: m["end"].as_u64()? as usize,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };

        results.push(result);

        if let Some(max) = max {
            if results.len() >= max {
                break;
            }
        }
    }

    Ok(results)
}

/// Extract issue ID from a file path
///
/// # Examples
///
/// ```
/// # use jit::search::extract_issue_id;
/// assert_eq!(extract_issue_id("issues/abc123.json"), Some("abc123".to_string()));
/// assert_eq!(extract_issue_id("docs/design.md"), None);
/// ```
pub fn extract_issue_id(path: &str) -> Option<String> {
    let path_obj = Path::new(path);

    // Only extract from files in "issues/" directory
    if !path.contains("issues/") {
        return None;
    }

    path_obj
        .file_stem()
        .and_then(|s| s.to_str())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_issue_id_from_issue_file() {
        assert_eq!(
            extract_issue_id("issues/abc123.json"),
            Some("abc123".to_string())
        );
        assert_eq!(
            extract_issue_id(".jit/issues/def456.json"),
            Some("def456".to_string())
        );
    }

    #[test]
    fn test_extract_issue_id_from_document_path() {
        assert_eq!(extract_issue_id("docs/design.md"), None);
        assert_eq!(extract_issue_id("README.md"), None);
        assert_eq!(extract_issue_id(".jit/gates.json"), None);
    }

    #[test]
    fn test_search_options_default() {
        let options = SearchOptions::default();
        assert!(!options.case_sensitive);
        assert!(!options.regex);
        assert_eq!(options.context_lines, 0);
        assert_eq!(options.max_results, None);
        assert_eq!(options.file_pattern, None);
    }

    #[test]
    fn test_search_result_serialization() {
        let result = SearchResult {
            issue_id: Some("abc123".to_string()),
            path: "issues/abc123.json".to_string(),
            line_number: 5,
            line_text: "  \"title\": \"Test Issue\",\n".to_string(),
            matches: vec![SearchMatch {
                text: "Test".to_string(),
                start: 12,
                end: 16,
            }],
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: SearchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, deserialized);
    }

    #[test]
    fn test_parse_ripgrep_json_single_match() {
        let json = r#"{"type":"match","data":{"path":{"text":"issues/abc123.json"},"lines":{"text":"  \"title\": \"Test\",\n"},"line_number":3,"submatches":[{"match":{"text":"Test"},"start":12,"end":16}]}}
"#;

        let results = parse_ripgrep_json(json.as_bytes(), None).unwrap();
        assert_eq!(results.len(), 1);

        let result = &results[0];
        assert_eq!(result.issue_id, Some("abc123".to_string()));
        assert_eq!(result.path, "issues/abc123.json");
        assert_eq!(result.line_number, 3);
        assert!(result.line_text.contains("Test"));
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].text, "Test");
    }

    #[test]
    fn test_parse_ripgrep_json_multiple_matches() {
        let json = r#"{"type":"match","data":{"path":{"text":"issues/abc123.json"},"lines":{"text":"test test\n"},"line_number":1,"submatches":[{"match":{"text":"test"},"start":0,"end":4},{"match":{"text":"test"},"start":5,"end":9}]}}
"#;

        let results = parse_ripgrep_json(json.as_bytes(), None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matches.len(), 2);
    }

    #[test]
    fn test_parse_ripgrep_json_ignores_non_match_events() {
        let json = r#"{"type":"begin","data":{"path":{"text":"issues/abc123.json"}}}
{"type":"match","data":{"path":{"text":"issues/abc123.json"},"lines":{"text":"test\n"},"line_number":1,"submatches":[{"match":{"text":"test"},"start":0,"end":4}]}}
{"type":"end","data":{"path":{"text":"issues/abc123.json"}}}
"#;

        let results = parse_ripgrep_json(json.as_bytes(), None).unwrap();
        assert_eq!(results.len(), 1); // Only the match event
    }

    #[test]
    fn test_parse_ripgrep_json_respects_max_results() {
        let json = r#"{"type":"match","data":{"path":{"text":"issues/1.json"},"lines":{"text":"test\n"},"line_number":1,"submatches":[]}}
{"type":"match","data":{"path":{"text":"issues/2.json"},"lines":{"text":"test\n"},"line_number":1,"submatches":[]}}
{"type":"match","data":{"path":{"text":"issues/3.json"},"lines":{"text":"test\n"},"line_number":1,"submatches":[]}}
"#;

        let results = parse_ripgrep_json(json.as_bytes(), Some(2)).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_ripgrep_json_no_matches() {
        let json = r#"{"type":"begin","data":{"path":{"text":"issues/abc123.json"}}}
{"type":"end","data":{"path":{"text":"issues/abc123.json"}}}
"#;

        let results = parse_ripgrep_json(json.as_bytes(), None).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_search_match_serialization() {
        let search_match = SearchMatch {
            text: "test".to_string(),
            start: 0,
            end: 4,
        };

        let json = serde_json::to_string(&search_match).unwrap();
        let deserialized: SearchMatch = serde_json::from_str(&json).unwrap();
        assert_eq!(search_match, deserialized);
    }
}
