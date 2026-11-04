//! Success responses and human renderings of the `jit config` leaves.
//!
//! Each response wraps the command-layer result it reports. A non-`transparent`
//! newtype serializes as its contents and takes its schema name from the wrapper.

use super::{
    render_config_get_value, render_config_snapshot, render_validation_findings, FindingsSummary,
    HumanRender,
};
use crate::commands::{ConfigGetOutcome, ConfigSetOutcome, ConfigValidationReport};
use crate::declarations::rules::Severity;
use crate::validation::report::ReportedFinding;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

/// `jit config show`: the effective-configuration snapshot.
#[derive(Serialize, JsonSchema)]
pub struct ConfigShowResponse(pub Value);

impl HumanRender for ConfigShowResponse {
    fn render_human(&self) -> String {
        format!(
            "Effective Configuration:\n\n{}",
            render_config_snapshot(&self.0)
        )
    }
}

/// `jit config get <key>`: the value resolved at the key.
#[derive(Serialize, JsonSchema)]
pub struct ConfigGetResponse(pub ConfigGetOutcome);

impl HumanRender for ConfigGetResponse {
    fn render_human(&self) -> String {
        format!("{}\n", render_config_get_value(&self.0.value))
    }
}

/// `jit config set`: the key, value, and file that were written.
#[derive(Serialize, JsonSchema)]
pub struct ConfigSetResponse(pub ConfigSetOutcome);

impl HumanRender for ConfigSetResponse {
    fn render_human(&self) -> String {
        format!(
            "Set {} = {} in {}\n",
            self.0.key,
            self.0.value,
            self.0.file.display()
        )
    }
}

/// `jit config validate`: the verdict and its errors. Also the failure report
/// when the configuration is invalid.
#[derive(Serialize, JsonSchema)]
pub struct ConfigValidateResponse(pub ConfigValidationReport);

impl HumanRender for ConfigValidateResponse {
    fn render_human(&self) -> String {
        let findings: Vec<_> = self
            .0
            .errors
            .iter()
            .map(|message| ReportedFinding {
                issue_id: None,
                rule: "config".to_string(),
                severity: Severity::Error,
                message: message.clone(),
            })
            .collect();
        // The failure summary is the outcome's message, written to stderr by the emitter.
        render_validation_findings(
            &findings,
            Some(FindingsSummary {
                pass: "Configuration is valid",
                failure: "Configuration validation failed",
            }),
        )
        .stdout
    }
}

/// `jit config show-hierarchy`: each declared type name mapped to its level.
#[derive(Serialize, JsonSchema)]
pub struct TypeHierarchyResponse(pub HashMap<String, u8>);

impl HumanRender for TypeHierarchyResponse {
    fn render_human(&self) -> String {
        let mut sorted: Vec<_> = self.0.iter().collect();
        sorted.sort_by_key(|(_, level)| **level);
        sorted.into_iter().fold(
            "Type Hierarchy:\n\n".to_owned(),
            |out, (type_name, level)| format!("{out}  {type_name} → Level {level}\n"),
        )
    }
}
