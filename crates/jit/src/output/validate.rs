//! Typed output for `jit validate`.

use super::{render_validation_findings, DivergenceView, FindingsSummary, HumanRender};
use crate::declarations::rules::Severity;
use crate::domain::identity::IssueId;
use crate::validation::report::{ExplainReport, RuleOutcome, RuleReport};
use schemars::gen::SchemaGenerator;
use schemars::schema::{InstanceType, ObjectValidation, Schema, SchemaObject, SubschemaValidation};
use schemars::JsonSchema;
use serde::Serialize;
use std::fmt::Write;

/// The registered success response for every `jit validate` mode.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ValidateResponse {
    Scope(RuleReport),
    Rule(RuleReport),
    Issue(RuleReport),
    Explanation(ExplainReport),
    Checks(ValidationChecksResponse),
    Fix(ValidationFixResponse),
    Repository(RepositoryValidationResponse),
}

impl JsonSchema for ValidateResponse {
    fn schema_name() -> String {
        "ValidateResponse".to_owned()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        SchemaObject {
            subschemas: Some(Box::new(SubschemaValidation {
                one_of: Some(vec![
                    generator.subschema_for::<RuleReport>(),
                    generator.subschema_for::<ExplainReport>(),
                    generator.subschema_for::<ValidationChecksResponse>(),
                    generator.subschema_for::<ValidationFixResponse>(),
                    generator.subschema_for::<RepositoryValidationResponse>(),
                ]),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

impl HumanRender for ValidateResponse {
    fn render_human(&self) -> String {
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
            Self::Explanation(report) => report.render_human(),
            Self::Checks(response) => response.render_human(),
            Self::Fix(response) => response.render_human(),
            Self::Repository(response) => response.render_human(),
        }
    }
}

fn render_rule_report(report: &RuleReport, pass: &'static str, failure: &'static str) -> String {
    render_validation_findings(&report.findings, Some(FindingsSummary { pass, failure })).stdout
}

impl JsonSchema for RuleReport {
    fn schema_name() -> String {
        "RuleReport".to_owned()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        object_schema([(
            "findings",
            generator.subschema_for::<Vec<crate::validation::report::ReportedFinding>>(),
            true,
        )])
    }
}

impl HumanRender for RuleReport {
    fn render_human(&self) -> String {
        render_validation_findings(&self.findings, None).stdout
    }
}

impl JsonSchema for RuleOutcome {
    fn schema_name() -> String {
        "RuleOutcome".to_owned()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        object_schema([
            ("rule", generator.subschema_for::<String>(), true),
            ("scope", generator.subschema_for::<String>(), true),
            ("severity", generator.subschema_for::<Severity>(), true),
            ("selector", generator.subschema_for::<String>(), true),
            ("matched", generator.subschema_for::<bool>(), true),
            ("skip_reason", generator.subschema_for::<String>(), false),
            ("passed", generator.subschema_for::<bool>(), true),
            ("messages", generator.subschema_for::<Vec<String>>(), true),
        ])
    }
}

impl JsonSchema for ExplainReport {
    fn schema_name() -> String {
        "ExplainReport".to_owned()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        object_schema([
            ("issue_id", generator.subschema_for::<IssueId>(), true),
            (
                "outcomes",
                generator.subschema_for::<Vec<RuleOutcome>>(),
                true,
            ),
        ])
    }
}

fn object_schema<const N: usize>(properties: [(&str, Schema, bool); N]) -> Schema {
    let mut object = ObjectValidation::default();
    for (name, schema, required) in properties {
        object.properties.insert(name.to_owned(), schema);
        if required {
            object.required.insert(name.to_owned());
        }
    }
    SchemaObject {
        instance_type: Some(InstanceType::Object.into()),
        object: Some(Box::new(object)),
        ..Default::default()
    }
    .into()
}

impl HumanRender for ExplainReport {
    fn render_human(&self) -> String {
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

    pub fn failure_message(&self) -> String {
        self.validations
            .iter()
            .find(|check| !check.valid)
            .map_or_else(String::new, |check| match check.validation.as_str() {
                "branch_drift" => {
                    format!("error: Branch-drift validation failed:\n{}", check.message)
                }
                "leases" if check.message.starts_with("Error: ") => format!(
                    "error: Lease validation error: {}",
                    check.message.trim_start_matches("Error: ")
                ),
                _ => format!("error: Lease validation failed:\n{}", check.message),
            })
    }
}

impl ValidationChecksResponse {
    fn human_text(&self) -> String {
        self.validations
            .iter()
            .take_while(|check| check.valid)
            .filter_map(|check| match check.validation.as_str() {
                "branch_drift" => Some("Branch is up-to-date with origin/main"),
                "leases" => Some("All active leases are valid"),
                _ => None,
            })
            .map(|line| format!("{line}\n"))
            .collect()
    }
}

impl HumanRender for ValidationChecksResponse {
    fn render_human(&self) -> String {
        self.human_text()
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ValidationFixResponse {
    pub valid: bool,
    pub fixes_applied: usize,
    pub dry_run: bool,
    pub message: String,
    #[serde(skip)]
    #[schemars(skip)]
    messages: Vec<String>,
}

impl ValidationFixResponse {
    pub fn new(fixes_applied: usize, dry_run: bool, messages: Vec<String>) -> Self {
        Self {
            valid: true,
            fixes_applied,
            dry_run,
            message: if dry_run {
                format!("{fixes_applied} fixes would be applied")
            } else if fixes_applied > 0 {
                format!("Applied {fixes_applied} fixes, repository is now valid")
            } else {
                "Repository is valid".to_owned()
            },
            messages,
        }
    }

    fn human_text(&self) -> String {
        self.messages
            .iter()
            .map(|message| format!("{message}\n"))
            .collect()
    }
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
    pub warnings: Vec<ValidationWarning>,
    pub warning_count: usize,
    pub membership_divergences: Vec<DivergenceView>,
    pub divergence_count: usize,
    pub rule_findings: Vec<crate::validation::report::ReportedFinding>,
    pub error_count: usize,
    pub message: String,
}

impl RepositoryValidationResponse {
    fn human_text(&self) -> String {
        let mut rendered = String::new();
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

impl HumanRender for ValidationFixResponse {
    fn render_human(&self) -> String {
        self.human_text()
    }
}

impl HumanRender for RepositoryValidationResponse {
    fn render_human(&self) -> String {
        self.human_text()
    }
}
