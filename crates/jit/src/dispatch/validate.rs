//! `jit validate` handlers.

use super::{resolve_issue, CommandContext, Outcome};
use crate::cli::ValidateArgs;
use crate::output::{
    ErrorCode, RepositoryValidationResponse, ValidateResponse, ValidationCheck,
    ValidationChecksResponse, ValidationFixResponse, ValidationWarning,
};
use crate::storage::{IssueStore, RepositoryStateStore};
use crate::validation::report::RuleReport;

fn validate<S: IssueStore + RepositoryStateStore>(
    ctx: &CommandContext<'_, S>,
    args: ValidateArgs,
) -> anyhow::Result<Outcome<ValidateResponse>> {
    let ValidateArgs {
        id,
        explain,
        fix,
        dry_run,
        branch_drift,
        divergence,
        leases,
        scope,
        rule,
    } = args;
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
        .as_deref()
        .map(|requested| resolve_issue(ctx.executor.storage(), requested))
        .transpose()?;

    if let Some(container) = scope {
        let container = resolve_issue(ctx.executor.storage(), &container)?;
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
            Outcome::findings(
                ErrorCode::ValidationFailed,
                "Validation found error-severity rule failures".to_owned(),
                response,
            )
        } else {
            Outcome::success_with(ValidateResponse::Explanation(response), "Validation passed")
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
            Outcome::success_with(ValidateResponse::Checks(response), message)
        } else {
            Outcome::findings(
                ErrorCode::ValidationFailed,
                response.failure_message(),
                response,
            )
        });
    }

    if fix {
        let (fixes_applied, messages) =
            ctx.executor.validation().validate_with_fix(true, dry_run)?;
        let response = ValidationFixResponse::new(fixes_applied, dry_run, messages);
        return Ok(Outcome::success(ValidateResponse::Fix(response)));
    }

    let (validation_report, divergence_report) = ctx
        .executor
        .validation()
        .validate_repository_report_with_divergences()?;
    let (structural_failure, rule_report) = validation_report.into_parts();
    let integrity_error = structural_failure.map(|error| format!("Invalid repository: {error:#}"));
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
        Outcome::findings(ErrorCode::ValidationFailed, message, response)
    } else {
        let mut warnings = Vec::new();
        if response.warning_count > 0 {
            warnings.push(format!("{} warning finding(s)", response.warning_count));
        }
        if response.divergence_count > 0 {
            warnings.push(format!(
                "{} membership label(s) not backed by the DAG (run `jit query divergence` for details)",
                response.divergence_count
            ));
        }
        Outcome::success_with(ValidateResponse::Repository(response), message)
            .with_warnings(warnings)
    })
}

fn validation_findings(report: RuleReport, mode: ValidationMode) -> Outcome<ValidateResponse> {
    if report.has_errors() {
        Outcome::findings(
            ErrorCode::ValidationFailed,
            format!(
                "{} with {} error(s)",
                mode.failure_message(),
                report.error_count()
            ),
            report,
        )
    } else {
        let response = match mode {
            ValidationMode::Scope => ValidateResponse::Scope(report),
            ValidationMode::Rule => ValidateResponse::Rule(report),
            ValidationMode::Issue => ValidateResponse::Issue(report),
        };
        Outcome::success_with(response, mode.success_message())
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
    command: ValidateArgs,
    storage: [IssueStore + RepositoryStateStore],
    leaves: {
        "validate" => args => validate(args) -> ValidateResponse
            as union [
                RuleReport,
                crate::validation::report::ExplainReport,
                ValidationChecksResponse,
                ValidationFixResponse,
                RepositoryValidationResponse,
            ] "The validation report for the selected mode",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_registered_output_schema_validate_names_the_response_union() {
        let registered = crate::dispatch::registered_output_schema("validate").unwrap();

        assert_eq!(registered.success, "RuleReport");
        let schema = registered.success_schema.unwrap();
        assert!(schema["oneOf"].is_array());
        assert_eq!(
            schema["description"],
            "The validation report for the selected mode"
        );
    }
}
