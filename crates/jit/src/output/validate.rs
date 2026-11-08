//! Typed output for `jit validate`.

use super::{render_validation_findings, DivergenceView, FindingsSummary, HumanRender, OutputMode};
use crate::domain::identity::IssueId;
use crate::validation::report::{ExplainReport, RuleReport};
use schemars::JsonSchema;
use serde::Serialize;
use std::fmt::Write;

/// The registered success response for every `jit validate` mode.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ValidateResponse {
    Scope(RuleReport),
    Rule(RuleReport),
    Issue(RuleReport),
    Explanation(ExplainReport),
    Checks(ValidationChecksResponse),
    Repository(RepositoryValidationResponse),
}

impl HumanRender for ValidateResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        match self {
            Self::Scope(report) => {
                render_rule_report(report, "Scope validation passed", "Scope validation failed")
            }
            Self::Rule(report) => {
                render_rule_report(report, "Rule validation passed", "Rule validation failed")
            }
            Self::Issue(report) => {
                render_rule_report(report, "Issue validation passed", "Validation failed")
            }
            Self::Explanation(report) => report.render_human(mode),
            Self::Checks(response) => response.render_human(mode),
            Self::Repository(response) => response.render_human(mode),
        }
    }
}

fn render_rule_report(report: &RuleReport, pass: &'static str, failure: &'static str) -> String {
    render_validation_findings(&report.findings, Some(FindingsSummary { pass, failure })).stdout
}

impl HumanRender for RuleReport {
    fn render_human(&self, _: OutputMode) -> String {
        render_validation_findings(&self.findings, None).stdout
    }
}

impl HumanRender for ExplainReport {
    fn render_human(&self, _: OutputMode) -> String {
        let mut rendered = format!("Rule explanation for issue {}\n", self.issue_id);
        if self.outcomes.is_empty() {
            rendered.push_str("  (no rules defined)\n");
        }
        let matched = self.outcomes.iter().filter(|outcome| outcome.matched);
        let mut any_matched = false;
        for outcome in matched {
            any_matched = true;
            let status = if outcome.passed { "PASS" } else { "FAIL" };
            let _ = writeln!(
                rendered,
                "  [{}] {} ({}, {}) selector: {}",
                status,
                outcome.rule,
                outcome.scope.as_str(),
                outcome.severity.as_str(),
                outcome.selector
            );
            for message in &outcome.messages {
                let _ = writeln!(rendered, "      - {}", message);
            }
        }
        if !any_matched && !self.outcomes.is_empty() {
            rendered.push_str("  (no rules match this issue)\n");
        }
        for outcome in self.outcomes.iter().filter(|outcome| !outcome.matched) {
            let reason = outcome
                .skip_reason
                .as_deref()
                .unwrap_or("selector did not match");
            let _ = writeln!(
                rendered,
                "  [SKIP] {} ({}, {}) selector: {} — {}",
                outcome.rule,
                outcome.scope.as_str(),
                outcome.severity.as_str(),
                outcome.selector,
                reason
            );
        }
        rendered
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ValidationCheck {
    pub validation: String,
    pub valid: bool,
    pub message: String,
}

impl ValidationCheck {
    pub fn new(validation: &'static str, valid: bool, message: String) -> Self {
        Self {
            validation: validation.to_owned(),
            valid,
            message,
        }
    }
}

/// Response for `--branch-drift`, `--leases`, or both.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ValidationChecksResponse {
    pub valid: bool,
    pub validations: Vec<ValidationCheck>,
}

impl ValidationChecksResponse {
    pub fn new(validations: Vec<ValidationCheck>) -> Self {
        Self {
            valid: validations.iter().all(|check| check.valid),
            validations,
        }
    }

    /// The one-line stderr summary naming each failed check.
    pub fn failure_message(&self) -> String {
        let failed: Vec<_> = self
            .validations
            .iter()
            .filter(|check| !check.valid)
            .map(|check| match check.validation.as_str() {
                "branch_drift" => "branch drift",
                _ => "leases",
            })
            .collect();
        format!("Validation failed: {}", failed.join(", "))
    }
}

impl ValidationChecksResponse {
    fn human_text(&self) -> String {
        self.validations
            .iter()
            .filter_map(|check| match (check.validation.as_str(), check.valid) {
                ("branch_drift", true) => Some("Branch is up-to-date with origin/main".to_owned()),
                ("branch_drift", false) => Some(format!(
                    "Branch-drift validation failed:\n{}",
                    check.message
                )),
                ("leases", true) => Some("All active leases are valid".to_owned()),
                ("leases", false) => Some(match check.message.strip_prefix("Error: ") {
                    Some(error) => format!("Lease validation error: {error}"),
                    None => format!("Lease validation failed:\n{}", check.message),
                }),
                _ => None,
            })
            .map(|line| format!("{line}\n"))
            .collect()
    }
}

impl HumanRender for ValidationChecksResponse {
    fn render_human(&self, _: OutputMode) -> String {
        self.human_text()
    }
}

/// What `--fix` did before the repository was validated.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ValidationFixSummary {
    pub fixes_applied: usize,
    pub fix_messages: Vec<String>,
    pub dry_run: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ValidationWarning {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub issue_id: Option<IssueId>,
    pub rule: String,
    pub message: String,
}

/// Whole-repository validation result from the captured report.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RepositoryValidationResponse {
    pub valid: bool,
    pub integrity_error: Option<String>,
    pub validation_warnings: Vec<ValidationWarning>,
    pub warning_count: usize,
    pub membership_divergences: Vec<DivergenceView>,
    pub divergence_count: usize,
    pub rule_findings: Vec<crate::validation::report::ReportedFinding>,
    pub error_count: usize,
    pub message: String,
    /// Present under `--fix`.
    #[serde(flatten)]
    pub fix: Option<ValidationFixSummary>,
}

impl RepositoryValidationResponse {
    fn human_text(&self) -> String {
        let mut rendered: String = self
            .fix
            .iter()
            .flat_map(|fix| &fix.fix_messages)
            .map(|message| format!("{message}\n"))
            .collect();
        if self.valid {
            rendered.push_str("Repository validation passed\n");
        } else {
            if self.error_count > 0 {
                let _ = writeln!(
                    rendered,
                    "error: Repository validation failed with {} rule error(s)",
                    self.error_count
                );
            }
            if let Some(error) = &self.integrity_error {
                let _ = writeln!(rendered, "error: Repository integrity error: {error}");
            }
        }
        rendered.push_str(&render_validation_findings(&self.rule_findings, None).stdout);
        if self.warning_count > 0 {
            let _ = writeln!(rendered, "\nWarnings: {}", self.warning_count);
        }
        // Advisory only: does not affect the exit status.
        if self.divergence_count > 0 {
            let _ = writeln!(
                rendered,
                "warning: {} membership label(s) not backed by the DAG \
                 (run `jit query divergence` for details)",
                self.divergence_count
            );
        }
        rendered
    }
}

impl HumanRender for RepositoryValidationResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        let _ = mode;
        self.human_text()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::Outcome;
    use crate::output::{emit_outcome, ErrorCode};

    fn failing(validation: &'static str, message: &str) -> ValidationCheck {
        ValidationCheck::new(validation, false, message.to_owned())
    }

    fn streams(response: ValidationChecksResponse) -> (String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mode = OutputMode {
            json: false,
            quiet: false,
        };
        let outcome: Outcome<ValidateResponse> = Outcome::findings(
            ErrorCode::ValidationFailed,
            response.failure_message(),
            response,
        );
        emit_outcome(&outcome, mode, &mut out, &mut err).unwrap();
        (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn test_checks_report_prints_every_failed_check_on_stdout_and_one_stderr_line() {
        let (stdout, stderr) = streams(ValidationChecksResponse::new(vec![
            failing("branch_drift", "diverged\nFix: git rebase origin/main"),
            failing("leases", "Found 1 invalid lease(s):\nlease-a"),
        ]));

        assert!(stdout.contains("diverged\nFix: git rebase origin/main"));
        assert!(stdout.contains("Found 1 invalid lease(s):\nlease-a"));
        assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
        assert!(stderr.ends_with('\n'));
    }

    #[test]
    fn test_checks_report_prints_passed_check_beside_failed_check() {
        let (stdout, stderr) = streams(ValidationChecksResponse::new(vec![
            ValidationCheck::new("branch_drift", true, String::new()),
            failing("leases", "Error: unreadable"),
        ]));

        assert!(stdout.contains("Branch is up-to-date with origin/main"));
        assert!(stdout.contains("Lease validation error: unreadable"));
        assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    }

    #[test]
    fn test_checks_report_prints_failed_branch_drift_beside_passed_leases() {
        let (stdout, stderr) = streams(ValidationChecksResponse::new(vec![
            failing("branch_drift", "diverged"),
            ValidationCheck::new("leases", true, String::new()),
        ]));

        assert!(stdout.contains("diverged"));
        assert!(stdout.contains("All active leases are valid"));
        assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    }
}
