//! Structured gate findings parsed from checker stdout.
//!
//! Automated gate checkers emit a freetext report to stdout, but rework loops
//! need the verdict and the individual findings as *data*, not a blob to grep.
//! This module defines that structured form ([`GateFindings`] /
//! [`GateFinding`]) and the pure parser ([`parse_gate_findings`]) that extracts
//! it from a checker's stdout.
//!
//! # The contract
//!
//! A conforming checker appends a machine-readable block to its stdout, fenced
//! by two line-exact markers:
//!
//! ```text
//! <<<JIT-FINDINGS-JSON
//! {"verdict":"fail","summary":"2 issues","findings":[
//!   {"id":"F1","severity":"high","summary":"missing error context",
//!    "file":"src/x.rs","line":42}
//! ]}
//! JIT-FINDINGS-JSON>>>
//! ```
//!
//! The markers are matched on their own trimmed line, so the JSON payload may
//! span multiple lines and the whole block may itself sit inside a markdown
//! code fence — the surrounding ```` ``` ```` lines are simply ignored. The
//! raw stdout is kept verbatim alongside the parsed struct; parsing never
//! rewrites the report text.
//!
//! # Graceful degradation
//!
//! A checker that emits no block, or a block whose JSON is malformed, yields
//! `None`: parsing is best-effort and never errors, so plain-text checkers keep
//! working unchanged. Missing optional fields inside a well-formed block default
//! to empty strings rather than rejecting the whole block.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Line-exact marker that opens the machine-readable findings block.
const FINDINGS_BEGIN: &str = "<<<JIT-FINDINGS-JSON";
/// Line-exact marker that closes the machine-readable findings block.
const FINDINGS_END: &str = "JIT-FINDINGS-JSON>>>";

/// One structured finding emitted by a gate checker.
///
/// `id`, `severity`, and `summary` are the minimum a finding carries;
/// `disposition` and `origin` are optional checker-defined classifications,
/// while `file` and `line` are optional locators. All fields default (to empty
/// / `None`) when absent from the checker's JSON, so older checkers and stored
/// findings remain compatible.
///
/// # Examples
///
/// ```
/// use jit::domain::GateFinding;
///
/// let json = r#"{"id":"F1","severity":"high","summary":"missing guard"}"#;
/// let finding: GateFinding = serde_json::from_str(json).unwrap();
/// assert_eq!(finding.id, "F1");
/// assert_eq!(finding.severity, "high");
/// assert!(finding.file.is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GateFinding {
    /// Checker-assigned finding identifier (e.g. `"F1"`).
    #[serde(default)]
    pub id: String,
    /// Checker-defined severity label (e.g. `"high"`, `"medium"`, `"low"`).
    #[serde(default)]
    pub severity: String,
    /// Whether the finding affects the verdict (e.g. `"blocking"`, `"advisory"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<String>,
    /// How the finding relates to the reviewed work (e.g. `"issue-impact"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// One-line human-readable description of the finding.
    #[serde(default)]
    pub summary: String,
    /// Optional file path the finding refers to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Optional line number within [`file`](Self::file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// Structured result parsed from a checker's machine-readable findings block.
///
/// Carries the checker-declared `verdict` (typically `"pass"` / `"fail"`, kept
/// as a free string so a checker's own vocabulary is never rejected), a
/// one-line `summary`, and the array of individual `findings`. This is stored
/// alongside — never in place of — the raw stdout on a gate run result.
///
/// # Examples
///
/// ```
/// use jit::domain::{GateFindings, parse_gate_findings};
///
/// let stdout = "review text\n\
///     <<<JIT-FINDINGS-JSON\n\
///     {\"verdict\":\"pass\",\"summary\":\"all clear\",\"findings\":[]}\n\
///     JIT-FINDINGS-JSON>>>\n";
/// let parsed: GateFindings = parse_gate_findings(stdout).unwrap();
/// assert_eq!(parsed.verdict, "pass");
/// assert_eq!(parsed.summary, "all clear");
/// assert!(parsed.findings.is_empty());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GateFindings {
    /// Checker-declared verdict (e.g. `"pass"` / `"fail"`).
    #[serde(default)]
    pub verdict: String,
    /// One-line summary of the run.
    #[serde(default)]
    pub summary: String,
    /// Individual findings; empty when the checker reports none.
    #[serde(default)]
    pub findings: Vec<GateFinding>,
}

/// Extract the structured findings block from a checker's stdout.
///
/// Scans `stdout` for a block fenced by the line-exact markers
/// `<<<JIT-FINDINGS-JSON` and `JIT-FINDINGS-JSON>>>` and parses the JSON
/// between them into [`GateFindings`]. When several complete blocks are present
/// the **last** one wins, mirroring the "last verdict line wins" rule that
/// existing checkers use — the real block is appended at the end of the report,
/// so an example block quoted earlier in prose never shadows it.
///
/// Returns `None` when no block is present, when a begin marker has no matching
/// end marker, or when the enclosed text is not valid findings JSON. Parsing is
/// best-effort and never errors: a plain-text checker degrades to `None` and
/// keeps its existing behaviour.
///
/// # Examples
///
/// ```
/// use jit::domain::parse_gate_findings;
///
/// // No block at all -> None (plain-text checker degrades gracefully).
/// assert!(parse_gate_findings("just some review prose\nVERDICT: PASS").is_none());
///
/// // A well-formed block is extracted even when wrapped in a markdown fence.
/// let stdout = "```\n\
///     <<<JIT-FINDINGS-JSON\n\
///     {\"verdict\":\"fail\",\"summary\":\"1 issue\",\"findings\":[\
///     {\"id\":\"F1\",\"severity\":\"high\",\"summary\":\"bug\"}]}\n\
///     JIT-FINDINGS-JSON>>>\n\
///     ```\n";
/// let parsed = parse_gate_findings(stdout).unwrap();
/// assert_eq!(parsed.verdict, "fail");
/// assert_eq!(parsed.findings.len(), 1);
/// assert_eq!(parsed.findings[0].id, "F1");
/// ```
pub fn parse_gate_findings(stdout: &str) -> Option<GateFindings> {
    let lines: Vec<&str> = stdout.lines().collect();

    // Pair each END marker with the most recent unclosed BEGIN and keep the
    // last complete pair, so the trailing block wins and a stray unmatched
    // marker cannot swallow an earlier valid block.
    let mut last_block: Option<(usize, usize)> = None;
    let mut open: Option<usize> = None;
    for (idx, line) in lines.iter().enumerate() {
        match line.trim() {
            FINDINGS_BEGIN => open = Some(idx),
            FINDINGS_END => {
                if let Some(begin) = open.take() {
                    last_block = Some((begin, idx));
                }
            }
            _ => {}
        }
    }

    let (begin, end) = last_block?;
    let json = lines[begin + 1..end].join("\n");
    serde_json::from_str::<GateFindings>(&json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(json: &str) -> String {
        format!("{FINDINGS_BEGIN}\n{json}\n{FINDINGS_END}\n")
    }

    #[test]
    fn test_parse_gate_findings_returns_none_when_absent() {
        assert!(parse_gate_findings("plain review text\nVERDICT: PASS").is_none());
        assert!(parse_gate_findings("").is_none());
    }

    #[test]
    fn test_parse_gate_findings_extracts_verdict_summary_and_findings() {
        let stdout = block(
            r#"{"verdict":"fail","summary":"2 problems","findings":[
                {"id":"F1","severity":"high","disposition":"blocking","origin":"issue-impact","summary":"missing guard","file":"src/x.rs","line":42},
                {"id":"F2","severity":"low","summary":"nit"}
            ]}"#,
        );
        let parsed = parse_gate_findings(&stdout).unwrap();
        assert_eq!(parsed.verdict, "fail");
        assert_eq!(parsed.summary, "2 problems");
        assert_eq!(parsed.findings.len(), 2);
        assert_eq!(parsed.findings[0].id, "F1");
        assert_eq!(parsed.findings[0].severity, "high");
        assert_eq!(parsed.findings[0].disposition.as_deref(), Some("blocking"));
        assert_eq!(parsed.findings[0].origin.as_deref(), Some("issue-impact"));
        assert_eq!(parsed.findings[0].file.as_deref(), Some("src/x.rs"));
        assert_eq!(parsed.findings[0].line, Some(42));
        // Optional locators absent on the second finding.
        assert!(parsed.findings[1].file.is_none());
        assert!(parsed.findings[1].line.is_none());
        assert!(parsed.findings[1].disposition.is_none());
        assert!(parsed.findings[1].origin.is_none());
    }

    #[test]
    fn test_parse_gate_findings_ignores_surrounding_prose_and_code_fence() {
        let stdout = format!(
            "# Review\n\nSome findings follow.\n\n```json\n{}\n```\n\nVERDICT: FAIL\n",
            block(r#"{"verdict":"fail","summary":"x","findings":[]}"#).trim()
        );
        let parsed = parse_gate_findings(&stdout).unwrap();
        assert_eq!(parsed.verdict, "fail");
    }

    #[test]
    fn test_parse_gate_findings_malformed_json_degrades_to_none() {
        // Well-formed fence, but the JSON inside is broken: silent skip.
        let stdout = block("{not valid json");
        assert!(parse_gate_findings(&stdout).is_none());
    }

    #[test]
    fn test_parse_gate_findings_begin_without_end_is_none() {
        let stdout = format!("{FINDINGS_BEGIN}\n{{\"verdict\":\"pass\"}}\n");
        assert!(parse_gate_findings(&stdout).is_none());
    }

    #[test]
    fn test_parse_gate_findings_end_without_begin_is_none() {
        let stdout = format!("{{\"verdict\":\"pass\"}}\n{FINDINGS_END}\n");
        assert!(parse_gate_findings(&stdout).is_none());
    }

    #[test]
    fn test_parse_gate_findings_last_block_wins() {
        let stdout = format!(
            "{}{}",
            block(r#"{"verdict":"pass","summary":"first","findings":[]}"#),
            block(r#"{"verdict":"fail","summary":"second","findings":[]}"#),
        );
        let parsed = parse_gate_findings(&stdout).unwrap();
        assert_eq!(parsed.summary, "second");
        assert_eq!(parsed.verdict, "fail");
    }

    #[test]
    fn test_parse_gate_findings_missing_fields_default() {
        // Only findings key present; verdict/summary default to empty strings.
        let stdout = block(r#"{"findings":[{"summary":"only a summary"}]}"#);
        let parsed = parse_gate_findings(&stdout).unwrap();
        assert_eq!(parsed.verdict, "");
        assert_eq!(parsed.summary, "");
        assert_eq!(parsed.findings.len(), 1);
        assert_eq!(parsed.findings[0].id, "");
        assert_eq!(parsed.findings[0].severity, "");
        assert_eq!(parsed.findings[0].summary, "only a summary");
    }

    #[test]
    fn test_parse_gate_findings_empty_object_is_present_but_empty() {
        let parsed = parse_gate_findings(&block("{}")).unwrap();
        assert_eq!(parsed.verdict, "");
        assert!(parsed.findings.is_empty());
    }

    #[test]
    fn test_gate_findings_round_trips_through_json() {
        let findings = GateFindings {
            verdict: "fail".to_string(),
            summary: "1 issue".to_string(),
            findings: vec![GateFinding {
                id: "F1".to_string(),
                severity: "high".to_string(),
                disposition: Some("blocking".to_string()),
                origin: Some("issue-impact".to_string()),
                summary: "bug".to_string(),
                file: Some("a.rs".to_string()),
                line: Some(3),
            }],
        };
        let json = serde_json::to_string(&findings).unwrap();
        let back: GateFindings = serde_json::from_str(&json).unwrap();
        assert_eq!(findings, back);
    }
}
