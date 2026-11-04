//! Success responses and human renderings of the `jit config` leaves.

use super::{
    render_config_get_value, render_config_snapshot, render_validation_findings, FindingsSummary,
    HumanRender,
};
use crate::commands::{ConfigGetOutcome, ConfigSetOutcome, ConfigValidationReport};
use crate::declarations::rules::Severity;
use crate::validation::report::ReportedFinding;
use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

/// `jit config show`: the effective-configuration snapshot.
///
/// Serializes as the snapshot itself; the schema is named for this response
/// rather than for the untyped value it wraps.
#[derive(Serialize)]
#[serde(transparent)]
pub struct ConfigShowResponse(pub Value);

impl JsonSchema for ConfigShowResponse {
    fn schema_name() -> String {
        "ConfigShowResponse".to_owned()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        Value::json_schema(generator)
    }
}

impl HumanRender for ConfigShowResponse {
    fn render_human(&self) -> String {
        format!(
            "Effective Configuration:\n\n{}",
            render_config_snapshot(&self.0)
        )
    }
}

/// `jit config get <key>`: the value resolved at `key`.
#[derive(Serialize, JsonSchema)]
pub struct ConfigGetResponse {
    pub key: String,
    pub value: Value,
}

impl From<ConfigGetOutcome> for ConfigGetResponse {
    fn from(outcome: ConfigGetOutcome) -> Self {
        Self {
            key: outcome.key,
            value: outcome.value,
        }
    }
}

impl HumanRender for ConfigGetResponse {
    fn render_human(&self) -> String {
        format!("{}\n", render_config_get_value(&self.value))
    }
}

/// `jit config set`: the key, value, and file that were written.
#[derive(Serialize, JsonSchema)]
pub struct ConfigSetResponse {
    pub key: String,
    pub value: String,
    pub file: String,
}

impl From<ConfigSetOutcome> for ConfigSetResponse {
    fn from(outcome: ConfigSetOutcome) -> Self {
        Self {
            key: outcome.key,
            value: outcome.value,
            file: outcome.file.display().to_string(),
        }
    }
}

impl HumanRender for ConfigSetResponse {
    fn render_human(&self) -> String {
        format!("Set {} = {} in {}\n", self.key, self.value, self.file)
    }
}

/// `jit config validate`: the verdict and its errors. Also the failure report
/// when the configuration is invalid.
#[derive(Serialize, JsonSchema)]
pub struct ConfigValidateResponse {
    pub valid: bool,
    pub errors: Vec<String>,
}

impl From<ConfigValidationReport> for ConfigValidateResponse {
    fn from(report: ConfigValidationReport) -> Self {
        Self {
            valid: report.errors.is_empty(),
            errors: report.errors,
        }
    }
}

impl HumanRender for ConfigValidateResponse {
    fn render_human(&self) -> String {
        let findings: Vec<_> = self
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
///
/// Serializes as the map itself; the schema is named for this response.
#[derive(Serialize)]
#[serde(transparent)]
pub struct TypeHierarchyResponse(pub HashMap<String, u8>);

impl JsonSchema for TypeHierarchyResponse {
    fn schema_name() -> String {
        "TypeHierarchyResponse".to_owned()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        HashMap::<String, u8>::json_schema(generator)
    }
}

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
