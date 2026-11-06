//! Full-text search using ripgrep.
//!
//! Provides search across issues and referenced documents using `ripgrep` as the backend.
//! This approach requires no index management and always searches the current state.
//!
//! # Ripgrep Requirement
//!
//! This module requires `ripgrep` (command `rg`) to be installed on the system.
//! If not found, search operations return a helpful error message with installation instructions.

use crate::domain::identity::IssueId;
use crate::domain::relative_path::RootRelativePath;
use crate::repository_state::VirtualPath;
use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Stdio};

/// A failure from the full-text search backend (`ripgrep`).
///
/// Distinguishes "`ripgrep` is not installed" from a search-execution failure so
/// the CLI can pick the JSON `error_code`
/// ([`RipgrepNotFound`](crate::output::ErrorCode::RipgrepNotFound) or
/// [`SearchFailed`](crate::output::ErrorCode::SearchFailed)) and its suggestion
/// by downcasting to this type rather than scanning the
/// message text. Each variant's `Display` reproduces the original message
/// verbatim, so user-facing output is unchanged.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// `ripgrep` (`rg`) was not found on `PATH`.
    #[error(
        "ripgrep (rg) is not installed\n\n\
         Ripgrep is required for search functionality.\n\n\
         Install ripgrep:\n\
         - Ubuntu/Debian: apt install ripgrep\n\
         - macOS: brew install ripgrep\n\
         - Windows: choco install ripgrep\n\
         - Other: https://github.com/BurntSushi/ripgrep#installation"
    )]
    RipgrepNotInstalled,
    /// `ripgrep` ran but exited with an error; carries its stderr.
    #[error("ripgrep failed: {0}")]
    Failed(String),
}

/// Search result from ripgrep
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SearchResult {
    /// Issue ID extracted from filename (None for non-issue files like documents)
    pub issue_id: Option<IssueId>,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    /// Multiple file patterns (e.g., specific document paths)
    /// If provided, overrides file_pattern
    ///
    /// With neither selection given, history paths are excluded from the search.
    pub file_patterns: Vec<String>,
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
    search_selected(data_dir, query, options, None)
}

/// Search only explicit repository-relative files; an empty list searches nothing.
pub fn search_files(
    data_dir: &Path,
    query: &str,
    options: SearchOptions,
    files: &[String],
) -> Result<Vec<SearchResult>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let root = data_dir.canonicalize()?;
    let mut paths = Vec::new();
    for file in files {
        let relative = RootRelativePath::parse(file)?;
        if relative.is_root() {
            anyhow::bail!("search path must name a descendant: {file}");
        }
        let candidate = root.join(relative.as_path());
        let resolved = match candidate.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !resolved.starts_with(&root) {
            anyhow::bail!("search path escapes repository: {file}");
        }
        if resolved.is_file() {
            paths.push(resolved);
        }
    }
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    search_selected(data_dir, query, options, Some(&paths))
}

fn search_selected(
    data_dir: &Path,
    query: &str,
    options: SearchOptions,
    paths: Option<&[std::path::PathBuf]>,
) -> Result<Vec<SearchResult>> {
    // Check if ripgrep is available
    if which::which("rg").is_err() {
        return Err(SearchError::RipgrepNotInstalled.into());
    }

    // Build ripgrep command
    let mut cmd = Command::new("rg");
    cmd.arg("--json").arg("--no-heading").arg("--with-filename");

    if !options.case_sensitive {
        cmd.arg("--ignore-case");
    }

    if !options.regex {
        cmd.arg("--fixed-strings");
    }

    if options.context_lines > 0 {
        cmd.arg(format!("--context={}", options.context_lines));
    }

    if paths.is_none() {
        if !options.file_patterns.is_empty() {
            for pattern in &options.file_patterns {
                cmd.arg("--glob").arg(pattern);
            }
        } else if let Some(pattern) = &options.file_pattern {
            cmd.arg("--glob").arg(pattern);
        } else {
            // Anchored, so a same-named path nested elsewhere stays searchable.
            VirtualPath::history_paths().iter().for_each(|path| {
                cmd.arg("--glob").arg(format!("!/{path}"));
            });
        }
    }

    // ripgrep matches globs relative to its working directory, so run it in the search root.
    // The pattern is the value of `--regexp=`, so a leading `-` is never read as a flag.
    cmd.arg(format!("--regexp={query}")).arg("--");
    if let Some(paths) = paths {
        cmd.args(paths);
    } else {
        cmd.arg(std::path::absolute(data_dir)?);
    }
    cmd.current_dir(data_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Execute command
    let output = cmd.output()?;

    // Exit code 1 means no matches (not an error)
    if !output.status.success() && output.status.code() != Some(1) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SearchError::Failed(stderr.into_owned()).into());
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
pub fn extract_issue_id(path: &str) -> Option<IssueId> {
    let path_obj = Path::new(path);

    // Only extract from files in "issues/" directory
    if !path.contains("issues/") {
        return None;
    }

    path_obj
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|id| serde_json::from_value(serde_json::json!(id)).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_issue_id_from_issue_file() {
        assert_eq!(
            extract_issue_id("issues/fb90f3d2-ffbd-5dbd-9531-bdc92f12bb6b.json"),
            Some(crate::test_utils::issue_id(
                0xffbd5dbd9531bdc92f12bb6bfb90f3d2
            ))
        );
        assert_eq!(
            extract_issue_id(".jit/issues/8fde4411-626f-5466-82f0-605630dfc6ee.json"),
            Some(crate::test_utils::issue_id(
                0x626f546682f0605630dfc6ee8fde4411
            ))
        );
    }

    #[test]
    fn test_extract_issue_id_from_document_path() {
        assert_eq!(extract_issue_id("docs/design.md"), None);
        assert_eq!(extract_issue_id("README.md"), None);
        assert_eq!(extract_issue_id(".jit/gates.toml"), None);
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
            issue_id: Some(crate::test_utils::issue_id(
                0xffbd5dbd9531bdc92f12bb6bfb90f3d2,
            )),
            path: "issues/fb90f3d2-ffbd-5dbd-9531-bdc92f12bb6b.json".to_string(),
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
        let json = r#"{"type":"match","data":{"path":{"text":"issues/fb90f3d2-ffbd-5dbd-9531-bdc92f12bb6b.json"},"lines":{"text":"  \"title\": \"Test\",\n"},"line_number":3,"submatches":[{"match":{"text":"Test"},"start":12,"end":16}]}}
"#;

        let results = parse_ripgrep_json(json.as_bytes(), None).unwrap();
        assert_eq!(results.len(), 1);

        let result = &results[0];
        assert_eq!(
            result.issue_id,
            Some(crate::test_utils::issue_id(
                0xffbd5dbd9531bdc92f12bb6bfb90f3d2
            ))
        );
        assert_eq!(
            result.path,
            format!("issues/{}.json", result.issue_id.as_ref().unwrap())
        );
        assert_eq!(result.line_number, 3);
        assert!(result.line_text.contains("Test"));
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].text, "Test");
    }

    #[test]
    fn test_parse_ripgrep_json_multiple_matches() {
        let json = r#"{"type":"match","data":{"path":{"text":"issues/fb90f3d2-ffbd-5dbd-9531-bdc92f12bb6b.json"},"lines":{"text":"test test\n"},"line_number":1,"submatches":[{"match":{"text":"test"},"start":0,"end":4},{"match":{"text":"test"},"start":5,"end":9}]}}
"#;

        let results = parse_ripgrep_json(json.as_bytes(), None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matches.len(), 2);
    }

    #[test]
    fn test_parse_ripgrep_json_ignores_non_match_events() {
        let json = r#"{"type":"begin","data":{"path":{"text":"issues/fb90f3d2-ffbd-5dbd-9531-bdc92f12bb6b.json"}}}
{"type":"match","data":{"path":{"text":"issues/fb90f3d2-ffbd-5dbd-9531-bdc92f12bb6b.json"},"lines":{"text":"test\n"},"line_number":1,"submatches":[{"match":{"text":"test"},"start":0,"end":4}]}}
{"type":"end","data":{"path":{"text":"issues/fb90f3d2-ffbd-5dbd-9531-bdc92f12bb6b.json"}}}
"#;

        let results = parse_ripgrep_json(json.as_bytes(), None).unwrap();
        assert_eq!(results.len(), 1); // Only the match event
    }

    #[test]
    fn test_parse_ripgrep_json_respects_max_results() {
        let json = [
            0x400080000000000000010f0e0d0c,
            0x400080000000000000020f0e0d0c,
            0x400080000000000000030f0e0d0c,
        ]
        .map(crate::test_utils::issue_id)
        .iter()
        .map(|id| {
            serde_json::json!({
                "type": "match",
                "data": {
                    "path": {"text": format!("issues/{id}.json")},
                    "lines": {"text": "test\n"},
                    "line_number": 1,
                    "submatches": [],
                },
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");

        let results = parse_ripgrep_json(json.as_bytes(), Some(2)).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_ripgrep_json_no_matches() {
        let json = r#"{"type":"begin","data":{"path":{"text":"issues/fb90f3d2-ffbd-5dbd-9531-bdc92f12bb6b.json"}}}
{"type":"end","data":{"path":{"text":"issues/fb90f3d2-ffbd-5dbd-9531-bdc92f12bb6b.json"}}}
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

    // SearchError exists so the CLI can pick the JSON `error_code` by downcast
    // rather than by scanning the message. These tests pin both halves of that contract: each variant's
    // `Display` reproduces the original `bail!` message verbatim, and the type
    // survives an `anyhow::Error` round trip so the downcast in `main.rs` works.

    #[test]
    fn test_search_error_ripgrep_not_installed_display_is_verbatim() {
        let expected = "ripgrep (rg) is not installed\n\n\
             Ripgrep is required for search functionality.\n\n\
             Install ripgrep:\n\
             - Ubuntu/Debian: apt install ripgrep\n\
             - macOS: brew install ripgrep\n\
             - Windows: choco install ripgrep\n\
             - Other: https://github.com/BurntSushi/ripgrep#installation";
        assert_eq!(SearchError::RipgrepNotInstalled.to_string(), expected);

        let any: anyhow::Error = SearchError::RipgrepNotInstalled.into();
        assert_eq!(any.to_string(), expected);
        assert!(matches!(
            any.downcast_ref::<SearchError>(),
            Some(SearchError::RipgrepNotInstalled)
        ));
    }

    #[test]
    fn test_search_error_failed_display_is_verbatim() {
        let err = SearchError::Failed("regex parse error".to_string());
        assert_eq!(err.to_string(), "ripgrep failed: regex parse error");

        let any: anyhow::Error = err.into();
        assert_eq!(any.to_string(), "ripgrep failed: regex parse error");
        assert!(matches!(
            any.downcast_ref::<SearchError>(),
            Some(SearchError::Failed(_))
        ));
    }

    fn corpus() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.json"),
            "before\nFlag one\n--verbose here\n-flag there\nafter\n",
        )
        .unwrap();
        dir
    }

    fn matched_lines(dir: &tempfile::TempDir, query: &str, options: SearchOptions) -> Vec<String> {
        search(dir.path(), query, options)
            .unwrap()
            .into_iter()
            .map(|r| r.line_text.trim().to_string())
            .collect()
    }

    #[test]
    fn test_search_regex_matches_for_every_context_case_and_glob_combination() {
        let dir = corpus();
        for context_lines in [0, 1] {
            for case_sensitive in [false, true] {
                for globs in [vec![], vec!["*.json".to_string()]] {
                    let options = SearchOptions {
                        regex: true,
                        context_lines,
                        case_sensitive,
                        file_patterns: globs.clone(),
                        ..SearchOptions::default()
                    };
                    let query = if case_sensitive { "Flag o." } else { "flag o." };
                    assert_eq!(
                        matched_lines(&dir, query, options),
                        ["Flag one"],
                        "context={context_lines} case={case_sensitive} globs={globs:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_search_dash_leading_query_matches_as_text_in_literal_and_regex_modes() {
        let dir = corpus();
        for regex in [false, true] {
            for (query, expected) in [("--verbose", "--verbose here"), ("-flag", "-flag there")] {
                let options = SearchOptions {
                    regex,
                    ..SearchOptions::default()
                };
                assert_eq!(
                    matched_lines(&dir, query, options),
                    [expected],
                    "regex={regex} query={query}"
                );
            }
        }
    }

    #[test]
    fn test_search_files_empty_selection_never_searches_root() {
        let dir = corpus();
        assert!(
            search_files(dir.path(), "Flag", SearchOptions::default(), &[])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn test_search_files_rejects_paths_outside_repository() {
        let dir = corpus();
        assert!(search_files(
            dir.path(),
            "Flag",
            SearchOptions::default(),
            &["../outside".into()]
        )
        .is_err());
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("secret"), "Flag").unwrap();
            std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
            assert!(search_files(
                dir.path(),
                "Flag",
                SearchOptions::default(),
                &["link/secret".into()]
            )
            .is_err());
        }
    }

    #[test]
    fn test_search_files_treats_metacharacters_as_literal_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("[a].md"), "needle\n").unwrap();
        std::fs::write(dir.path().join("a.md"), "needle\n").unwrap();
        let matches = search_files(
            dir.path(),
            "needle",
            SearchOptions::default(),
            &["[a].md".into()],
        )
        .unwrap();
        assert_eq!(matches.len(), 1);
        assert!(matches[0].path.ends_with("[a].md"));
    }

    #[test]
    fn test_search_default_skips_history_files_and_explicit_glob_reaches_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let write = |rel: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "needle\n").unwrap();
        };
        let history = VirtualPath::history_paths();
        let [events, runs, scratch] = &history;
        let run = VirtualPath::gate_run_record(
            &crate::test_utils::issue_id(1),
            &crate::test_utils::gate_run_id(1),
        );
        let run = run.relative().as_str();
        write(events);
        write(run);
        write(&format!("{scratch}/redo.bin"));
        write(&format!("nested/{events}"));
        write(&format!("nested/{scratch}/x.txt"));
        write("issues/a.json");

        let found = |options: SearchOptions| -> std::collections::BTreeSet<String> {
            search(root, "needle", options)
                .unwrap()
                .into_iter()
                .map(|r| {
                    Path::new(&r.path)
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect()
        };

        assert_eq!(
            found(SearchOptions::default()),
            [
                "issues/a.json".to_string(),
                format!("nested/{events}"),
                format!("nested/{scratch}/x.txt"),
            ]
            .into()
        );
        assert_eq!(
            found(SearchOptions {
                file_pattern: Some(format!("{runs}/**")),
                ..SearchOptions::default()
            }),
            [run.to_string()].into()
        );
    }
}
