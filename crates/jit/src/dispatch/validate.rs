//! `jit validate` handlers.

use super::{CommandContext, Outcome};
use crate::domain::identity::IssueIdPrefix;
use crate::output::{
    ErrorCode, RepositoryValidationResponse, ValidateResponse, ValidationCheck,
    ValidationChecksResponse, ValidationFixResponse, ValidationWarning,
};
use crate::storage::{IssueStore, RepositoryStateStore};
use crate::validation::report::RuleReport;

/// The validate leaf passed from the CLI's top-level command match.
pub enum ValidateCommand {
    Validate {
        id: Option<String>,
        explain: bool,
        fix: bool,
        dry_run: bool,
        branch_drift: bool,
        divergence: bool,
        leases: bool,
        scope: Option<String>,
        rule: Option<String>,
    },
}

fn validate<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    command: ValidateCommand,
) -> anyhow::Result<Outcome<ValidateResponse>> {
    let ValidateCommand::Validate {
        id,
        explain,
        fix,
        dry_run,
        branch_drift,
        divergence,
        leases,
        scope,
        rule,
    } = command;
    if divergence {
        anyhow::bail!(crate::errors::InvalidArgumentError::new(
            "`--divergence` is not a `jit validate` flag. Use \
             `jit validate --branch-drift` for git branch drift, or \
             `jit query divergence` for membership labels the DAG does \
             not back."
        ));
    }

    if dry_run && !fix {
        anyhow::bail!(crate::errors::InvalidArgumentError::new(
            "--dry-run requires --fix to be specified",
        ));
    }

    if scope.is_some()
        && (id.is_some() || fix || branch_drift || leases || explain || rule.is_some())
    {
        anyhow::bail!(crate::errors::InvalidArgumentError::new(
            "`--scope` cannot be combined with a positional id or with \
             `--fix`/`--branch-drift`/`--leases`/`--explain`/`--rule`",
        ));
    }

    if rule.is_some() && (fix || branch_drift || leases || explain) {
        anyhow::bail!(crate::errors::InvalidArgumentError::new(
            "`--rule` cannot be combined with `--fix`/`--branch-drift`/`--leases`/`--explain`",
        ));
    }
    if rule.is_some() && id.is_none() {
        anyhow::bail!(crate::errors::InvalidArgumentError::new(
            "`--rule` requires a positional issue id",
        ));
    }

    if id.is_some() && (fix || branch_drift || leases) {
        anyhow::bail!(
            "`--fix`/`--branch-drift`/`--leases` cannot be combined with a \
             positional issue id (they are repo-wide)"
        );
    }

    let id = id
        .map(|requested| {
            let prefix = requested.parse::<IssueIdPrefix>()?;
            ctx.executor.storage().resolve_issue_id(&prefix)
        })
        .transpose()?;

    if let Some(container) = scope {
        let prefix = container.parse::<IssueIdPrefix>()?;
        let container = ctx.executor.storage().resolve_issue_id(&prefix)?;
        let report = ctx.executor.validation().validate_scope(&container)?;
        return Ok(validation_findings(report, ValidationMode::Scope));
    }

    if let (Some(rule_name), Some(issue_id)) = (rule.as_deref(), id.as_ref()) {
        let report = ctx
            .executor
            .validation()
            .validate_rule(issue_id, rule_name)?;
        return Ok(validation_findings(report, ValidationMode::Rule));
    }

    if explain && id.is_none() {
        anyhow::bail!("--explain requires an issue id");
    }

    if let (true, Some(issue_id)) = (explain, id.as_ref()) {
        let response = ctx.executor.validation().explain_rules(issue_id)?;
        let outcome = if response.has_errors() {
            Outcome::Findings {
                code: ErrorCode::ValidationFailed,
                message: "Validation found error-severity rule failures".to_owned(),
                report: Box::new(response),
            }
        } else {
            Outcome::Success {
                response: ValidateResponse::Explanation(response),
                message: Some("Validation passed".to_owned()),
            }
        };
        return Ok(outcome);
    }

    if let Some(issue_id) = id.as_ref() {
        let report = ctx.executor.validation().run_rules(Some(issue_id))?;
        return Ok(validation_findings(report, ValidationMode::Issue));
    }

    if branch_drift || leases {
        let mut validations = Vec::new();

        if branch_drift {
            match ctx.executor.validation().validate_branch_drift() {
                Ok(()) => {
                    validations.push(ValidationCheck::new("branch_drift", true, String::new()))
                }
                Err(error) => {
                    let message = error.to_string();
                    validations.push(ValidationCheck::new("branch_drift", false, message));
                }
            }
        }

        if leases {
            match ctx.executor.validation().validate_leases() {
                Ok(invalid_leases) if invalid_leases.is_empty() => {
                    validations.push(ValidationCheck::new("leases", true, String::new()));
                }
                Ok(invalid_leases) => {
                    let message = format!(
                        "Found {} invalid lease(s):\n{}",
                        invalid_leases.len(),
                        invalid_leases.join("\n\n")
                    );
                    validations.push(ValidationCheck::new("leases", false, message));
                }
                Err(error) => {
                    let message = format!("Error: {error}");
                    validations.push(ValidationCheck::new("leases", false, message));
                }
            }
        }

        let response = ValidationChecksResponse::new(validations);
        let message = if response.valid {
            "Validation passed".to_owned()
        } else {
            "Validation failed".to_owned()
        };
        return Ok(if response.valid {
            Outcome::Success {
                response: ValidateResponse::Checks(response),
                message: Some(message),
            }
        } else {
            Outcome::Findings {
                code: ErrorCode::ValidationFailed,
                message: response.failure_message(),
                report: Box::new(response),
            }
        });
    }

    if fix {
        let (fixes_applied, messages) =
            ctx.executor.validation().validate_with_fix(true, dry_run)?;
        let response = ValidationFixResponse::new(fixes_applied, dry_run, messages);
        return Ok(Outcome::Success {
            response: ValidateResponse::Fix(response),
            message: None,
        });
    }

    let (validation_report, divergence_report) = ctx
        .executor
        .validation()
        .validate_repository_report_with_divergences()?;
    let integrity_error = validation_report
        .structural_failure
        .map(|error| format!("Invalid repository: {error:#}"));
    let rule_report = validation_report.rule_report;
    let rules_failed = rule_report.has_errors();
    let validation_failed = rules_failed || integrity_error.is_some();
    let warning_findings: Vec<_> = rule_report
        .findings
        .iter()
        .filter(|finding| !finding.is_error())
        .collect();
    let warnings = warning_findings
        .into_iter()
        .map(|finding| ValidationWarning {
            kind: "rule_warning",
            issue_id: finding.issue_id.clone(),
            rule: finding.rule.clone(),
            message: finding.message.clone(),
        })
        .collect::<Vec<_>>();
    let error_count = rule_report.error_count();
    let integrity_message = integrity_error.as_ref().map(ToString::to_string);
    let message = if let Some(error) = &integrity_message {
        if rules_failed {
            format!(
                "Repository validation failed: {error_count} rule error(s) and a \
                 repository-integrity error: {error}"
            )
        } else {
            format!("Repository integrity validation failed: {error}")
        }
    } else if rules_failed {
        format!("Repository validation failed with {error_count} rule error(s)")
    } else {
        "Repository validation passed".to_owned()
    };
    let divergence_count = divergence_report.count();
    let response = RepositoryValidationResponse {
        valid: !validation_failed,
        integrity_error: integrity_message,
        warning_count: warnings.len(),
        warnings,
        membership_divergences: divergence_report.items,
        divergence_count,
        rule_findings: rule_report.findings,
        error_count,
        message: message.clone(),
    };

    Ok(if validation_failed {
        Outcome::Findings {
            code: ErrorCode::ValidationFailed,
            message,
            report: Box::new(response),
        }
    } else {
        Outcome::Success {
            response: ValidateResponse::Repository(response),
            message: Some(message),
        }
    })
}

fn validation_findings(report: RuleReport, mode: ValidationMode) -> Outcome<ValidateResponse> {
    if report.has_errors() {
        Outcome::Findings {
            code: ErrorCode::ValidationFailed,
            message: format!(
                "{} with {} error(s)",
                mode.failure_message(),
                report.error_count()
            ),
            report: Box::new(report),
        }
    } else {
        let response = match mode {
            ValidationMode::Scope => ValidateResponse::Scope(report),
            ValidationMode::Rule => ValidateResponse::Rule(report),
            ValidationMode::Issue => ValidateResponse::Issue(report),
        };
        Outcome::Success {
            response,
            message: Some(mode.success_message().to_owned()),
        }
    }
}

#[derive(Clone, Copy)]
enum ValidationMode {
    Scope,
    Rule,
    Issue,
}

impl ValidationMode {
    fn success_message(self) -> &'static str {
        match self {
            Self::Scope => "Scope validation passed",
            Self::Rule => "Rule validation passed",
            Self::Issue => "Validation passed",
        }
    }

    fn failure_message(self) -> &'static str {
        match self {
            Self::Scope => "Scope validation failed",
            Self::Rule => "Rule validation failed",
            Self::Issue => "Validation failed",
        }
    }
}

command_group! {
    command: ValidateCommand,
    storage: [IssueStore + RepositoryStateStore],
    leaves: {
        "validate" => ValidateCommand::Validate {
            id,
            explain,
            fix,
            dry_run,
            branch_drift,
            divergence,
            leases,
            scope,
            rule,
        } => validate(ValidateCommand::Validate {
            id,
            explain,
            fix,
            dry_run,
            branch_drift,
            divergence,
            leases,
            scope,
            rule,
        }) -> ValidateResponse,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_registered_output_schema_validate_names_the_response_union() {
        let registered = crate::dispatch::registered_output_schema("validate").unwrap();

        assert_eq!(registered.success, "ValidateResponse");
        assert!(registered.success_schema.is_some());
    }
}
