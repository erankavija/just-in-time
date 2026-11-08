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
    /// The one-line `message`, also the findings stderr line; the integrity
    /// error's details stay in `integrity_error`.
    pub fn summary(error_count: usize, has_integrity_error: bool) -> String {
        match (error_count, has_integrity_error) {
            (0, false) => "Repository validation passed".to_owned(),
            (0, true) => "Repository integrity validation failed".to_owned(),
            (count, true) => format!(
                "Repository validation failed: {count} rule error(s) and a \
                 repository-integrity error"
            ),
            (count, false) => format!("Repository validation failed with {count} rule error(s)"),
        }
    }

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

    fn emit(outcome: &Outcome<ValidateResponse>, json: bool) -> (String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mode = OutputMode { json, quiet: false };
        emit_outcome(outcome, mode, &mut out, &mut err).unwrap();
        (
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    fn streams(response: ValidationChecksResponse) -> (String, String) {
        let message = response.failure_message();
        emit(
            &Outcome::findings(ErrorCode::ValidationFailed, message, response),
            false,
        )
    }

    const DRIFT: &str = "Invalid repository: derived-state drift (repair with \
                         `jit validate --fix`):\n- .jit/rules.toml: stale\n- .jit/schemas/a.json: missing";

    /// A failed whole-repository report findings outcome, as dispatch builds it.
    fn repository_findings(error_count: usize) -> Outcome<ValidateResponse> {
        let response = RepositoryValidationResponse {
            valid: false,
            integrity_error: Some(DRIFT.to_owned()),
            validation_warnings: Vec::new(),
            warning_count: 0,
            membership_divergences: Vec::new(),
            divergence_count: 0,
            rule_findings: Vec::new(),
            error_count,
            message: RepositoryValidationResponse::summary(error_count, true),
            fix: None,
        };
        let message = response.message.clone();
        Outcome::findings(ErrorCode::ValidationFailed, message, response)
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

    #[test]
    fn test_repository_report_prints_drift_on_stdout_and_one_stderr_line() {
        let (stdout, stderr) = emit(&repository_findings(0), false);

        assert!(stdout.contains(DRIFT), "{stdout:?}");
        assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
        assert!(stderr.ends_with('\n'));
    }

    #[test]
    fn test_repository_report_prints_drift_beside_rule_errors_with_one_stderr_line() {
        let (stdout, stderr) = emit(&repository_findings(2), false);

        assert!(stdout.contains(DRIFT), "{stdout:?}");
        assert!(stdout.contains("2 rule error(s)"), "{stdout:?}");
        assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
        assert!(stderr.contains("2 rule error(s)"), "{stderr:?}");
    }

    #[test]
    fn test_repository_report_json_keeps_drift_in_details_beside_one_line_message() {
        let (stdout, _) = emit(&repository_findings(0), true);
        let envelope: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        let error = &envelope["error"];

        assert_eq!(error["details"]["integrity_error"], DRIFT);
        let message = error["message"].as_str().unwrap();
        assert_eq!(message.lines().count(), 1, "{message:?}");
        assert_eq!(error["details"]["message"], message);
    }
}
