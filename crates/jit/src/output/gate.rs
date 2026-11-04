//! Typed gate definition and preset responses and human rendering.

use super::{GateDefinition, GateListResponse, GatePresetListResponse, HumanRender, OutputMode};
use crate::declarations::GateChecker;
use crate::domain::identity::GateKey;
use crate::gate_presets::GatePresetDefinition;
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Write;

#[derive(Serialize, JsonSchema)]
pub struct GateMutationResponse {
    pub key: GateKey,
    pub message: String,
}

impl GateMutationResponse {
    pub fn new(key: GateKey, action: &str) -> Self {
        Self {
            message: format!("{action} gate '{key}'"),
            key,
        }
    }
}

impl HumanRender for GateMutationResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        if mode.quiet {
            String::new()
        } else {
            format!("{}\n", self.message)
        }
    }
}

/// The complete registry declaration returned by show and update.
#[derive(Serialize, JsonSchema)]
pub struct GateDefinitionResponse {
    version: u32,
    #[serde(flatten)]
    definition: GateDefinition,
    priority: u32,
    reserved: HashMap<String, serde_json::Value>,
    #[serde(skip)]
    updated: bool,
}

impl GateDefinitionResponse {
    pub fn shown(gate: crate::declarations::GateDefinition) -> Self {
        Self {
            version: gate.version,
            priority: gate.priority,
            reserved: gate.reserved.clone(),
            definition: gate.into(),
            updated: false,
        }
    }

    pub fn updated(gate: crate::declarations::GateDefinition) -> Self {
        Self {
            updated: true,
            ..Self::shown(gate)
        }
    }
}

impl HumanRender for GateDefinitionResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        let gate = &self.definition;
        if self.updated {
            return if mode.quiet {
                String::new()
            } else {
                format!("Updated gate '{}'\n", gate.key)
            };
        }
        let mut text = format!(
            "Gate: {}\n  Title: {}\n  Description: {}\n  Stage: {}\n  Mode: {}\n",
            gate.key, gate.title, gate.description, gate.stage, gate.mode,
        );
        if let Some(checker) = &gate.checker {
            text.push_str(&render_checker(checker, false));
        }
        text
    }
}

impl HumanRender for GateListResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        let heading = if mode.quiet {
            ""
        } else if self.items.is_empty() {
            "No gates defined\n"
        } else {
            "Gates:\n"
        };
        let rows: String = self
            .items
            .iter()
            .map(|gate| {
                format!(
                    "  {} - {} ({}, {})\n",
                    gate.key, gate.title, gate.stage, gate.mode
                )
            })
            .collect();
        format!("{heading}{rows}")
    }
}

#[derive(Serialize, JsonSchema)]
pub struct GateAddResponse {
    issue_id: String,
    added: Vec<GateKey>,
    already_exist: Vec<GateKey>,
    message: String,
}

impl GateAddResponse {
    pub fn new(issue_id: String, added: Vec<GateKey>, already_exist: Vec<GateKey>) -> Self {
        Self {
            message: format!("Added {} gate(s) to issue {}", added.len(), issue_id),
            issue_id,
            added,
            already_exist,
        }
    }
}

impl HumanRender for GateAddResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        let mut text = String::new();
        if !self.added.is_empty() {
            if !mode.quiet {
                let _ = writeln!(text, "{}:", self.message);
            }
            for key in &self.added {
                let _ = writeln!(text, "  • {key}");
            }
        }
        if !self.already_exist.is_empty() {
            let _ = writeln!(
                text,
                "Already required ({} gate(s)):",
                self.already_exist.len()
            );
            for key in &self.already_exist {
                let _ = writeln!(text, "  • {key}");
            }
        }
        text
    }
}

impl HumanRender for GatePresetListResponse {
    fn render_human(&self, _: OutputMode) -> String {
        if self.items.is_empty() {
            return "No gate presets available\n".to_owned();
        }
        self.items
            .iter()
            .map(|preset| {
                let word = if preset.gate_count == 1 {
                    "gate"
                } else {
                    "gates"
                };
                format!(
                    "{} - {} ({} {})\n",
                    preset.name, preset.description, preset.gate_count, word
                )
            })
            .collect()
    }
}

#[derive(Serialize, JsonSchema)]
pub struct GatePresetShowResponse {
    name: String,
    description: String,
    gates: Vec<GateDefinition>,
}

impl From<GatePresetDefinition> for GatePresetShowResponse {
    fn from(preset: GatePresetDefinition) -> Self {
        Self {
            name: preset.name,
            description: preset.description,
            gates: preset
                .gates
                .into_iter()
                .map(|gate| GateDefinition::from(gate.to_gate()))
                .collect(),
        }
    }
}

impl HumanRender for GatePresetShowResponse {
    fn render_human(&self, _: OutputMode) -> String {
        let mut text = format!(
            "Preset: {}\nDescription: {}\n\nGates:\n",
            self.name, self.description
        );
        for gate in &self.gates {
            let _ = writeln!(
                text,
                "  {} - {} ({}:{})",
                gate.key, gate.title, gate.stage, gate.mode
            );
            if let Some(checker) = &gate.checker {
                text.push_str(&render_checker(checker, true));
            }
        }
        text
    }
}

fn render_checker(checker: &GateChecker, preset: bool) -> String {
    match checker {
        GateChecker::Exec {
            command,
            timeout_seconds,
            working_dir,
            ..
        } => {
            let mut text = if preset {
                String::new()
            } else {
                "  Checker:\n".to_owned()
            };
            let _ = write!(
                text,
                "    Command: {command}\n    Timeout: {timeout_seconds}s\n"
            );
            if !preset {
                if let Some(dir) = working_dir {
                    let _ = writeln!(text, "    Working dir: {dir}");
                }
            }
            text
        }
        GateChecker::RepositoryValidation => {
            if preset {
                "    Built-in: repository validation\n".to_owned()
            } else {
                "  Checker: repository validation (built-in)\n".to_owned()
            }
        }
        GateChecker::IssueValidation => {
            if preset {
                "    Built-in: issue validation\n".to_owned()
            } else {
                "  Checker: issue validation (built-in)\n".to_owned()
            }
        }
        GateChecker::LabelTargetValidation { label_namespace } => {
            if preset {
                format!("    Built-in: label-target validation ({label_namespace}:)\n")
            } else {
                format!(
                    "  Checker: label-target validation (built-in, namespace: {label_namespace})\n"
                )
            }
        }
        GateChecker::RuleValidation { rule } => {
            if preset {
                format!("    Built-in: rule validation ({rule})\n")
            } else {
                format!("  Checker: rule validation (built-in, rule: {rule})\n")
            }
        }
        GateChecker::ReviewPlaceholder => {
            if preset {
                "    Built-in: WARNING — external review placeholder\n".to_owned()
            } else {
                "  Checker: WARNING — external review placeholder\n".to_owned()
            }
        }
    }
}

#[derive(Serialize, JsonSchema)]
pub struct GatePresetCreateResponse {
    pub name: String,
    pub path: String,
}

impl HumanRender for GatePresetCreateResponse {
    fn render_human(&self, _: OutputMode) -> String {
        format!("Created preset '{}' at {}\n", self.name, self.path)
    }
}

#[derive(Serialize, JsonSchema)]
pub struct GatePresetSuccess {
    pub issue_id: String,
    pub gates_added: Vec<GateKey>,
    pub already_existed: Vec<GateKey>,
}

#[derive(Serialize, JsonSchema)]
pub struct GatePresetError {
    pub issue_id: String,
    pub error: String,
}

#[derive(Serialize, JsonSchema)]
pub struct GatePresetApplyResponse {
    pub preset: String,
    pub success: Vec<GatePresetSuccess>,
    pub errors: Vec<GatePresetError>,
}

impl HumanRender for GatePresetApplyResponse {
    fn render_human(&self, _: OutputMode) -> String {
        let mut text = String::new();
        if !self.success.is_empty() {
            let _ = writeln!(
                text,
                "Applied preset '{}' to {} issue(s):",
                self.preset,
                self.success.len()
            );
            for result in &self.success {
                let keys = result
                    .gates_added
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(text, "  {} - gates added: {}", result.issue_id, keys);
            }
        }
        if !self.errors.is_empty() {
            let _ = writeln!(text, "\nErrors ({}):", self.errors.len());
            for error in &self.errors {
                let _ = writeln!(text, "  {} - {}", error.issue_id, error.error);
            }
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::{GateMode, GateStage};
    use crate::gate_presets::GateTemplate;
    use serde_json::json;

    #[test]
    fn test_gate_definition_response_preserves_complete_declaration_payload() {
        let gate = crate::declarations::GateDefinition {
            version: 1,
            key: "sample".parse().unwrap(),
            title: "Sample".to_owned(),
            description: "A declaration".to_owned(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: Some(GateChecker::RepositoryValidation),
            inputs: None,
            priority: 12,
            reserved: HashMap::from([("extension".to_owned(), json!("preserved"))]),
        };
        let expected = serde_json::to_value(&gate).unwrap();
        assert_eq!(
            serde_json::to_value(GateDefinitionResponse::shown(gate.clone())).unwrap(),
            expected
        );
        assert_eq!(
            serde_json::to_value(GateDefinitionResponse::updated(gate)).unwrap(),
            expected
        );
    }

    #[test]
    fn test_gate_preset_show_response_preserves_preset_payload() {
        let preset = GatePresetDefinition {
            name: "sample".to_owned(),
            description: "A preset".to_owned(),
            gates: vec![GateTemplate {
                key: "review".parse().unwrap(),
                title: "Review".to_owned(),
                description: "Human review".to_owned(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
            }],
        };
        let expected = serde_json::to_value(&preset).unwrap();
        assert_eq!(
            serde_json::to_value(GatePresetShowResponse::from(preset)).unwrap(),
            expected
        );
    }
}
