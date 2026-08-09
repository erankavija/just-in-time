//! Neutral authored quality-gate declarations and pure TOML preservation.

use crate::domain::repository_inputs::RepositoryInputs;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

/// Error returned for an unknown gate stage.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GateStageParseError {
    /// The input was not a canonical stage token.
    #[error("Invalid gate stage: '{0}' (expected 'precheck' or 'postcheck')")]
    UnknownStage(String),
}

/// Gate execution stage.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum GateStage {
    /// Runs before work starts.
    Precheck,
    /// Runs after work completes.
    Postcheck,
}

impl GateStage {
    /// Canonical serialized token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Precheck => "precheck",
            Self::Postcheck => "postcheck",
        }
    }
}

impl FromStr for GateStage {
    type Err = GateStageParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "precheck" => Ok(Self::Precheck),
            "postcheck" => Ok(Self::Postcheck),
            _ => Err(GateStageParseError::UnknownStage(value.to_string())),
        }
    }
}

impl std::fmt::Display for GateStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Error returned for an unknown gate mode.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GateModeParseError {
    /// The input was not a canonical mode token.
    #[error("Invalid gate mode: '{0}' (expected 'manual' or 'auto')")]
    UnknownMode(String),
}

/// Gate execution mode.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum GateMode {
    /// Requires an explicit pass/fail.
    Manual,
    /// Has an automated checker.
    Auto,
}

impl GateMode {
    /// Canonical serialized token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Auto => "auto",
        }
    }
}

impl FromStr for GateMode {
    type Err = GateModeParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "manual" => Ok(Self::Manual),
            "auto" => Ok(Self::Auto),
            _ => Err(GateModeParseError::UnknownMode(value.to_string())),
        }
    }
}

impl std::fmt::Display for GateMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Authored automated checker configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GateChecker {
    /// Execute a declared process.
    Exec {
        /// Program and arguments parsed by the checker boundary.
        command: String,
        /// Maximum execution time in seconds.
        timeout_seconds: u64,
        /// Optional worktree-relative working directory.
        working_dir: Option<String>,
        /// Additional declared environment.
        #[serde(default)]
        env: HashMap<String, String>,
        /// Whether the boundary supplies structured gate context.
        #[serde(default)]
        pass_context: bool,
        /// Inline checker prompt.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        /// Worktree-relative prompt file.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt_file: Option<String>,
    },
    /// Run whole-repository validation in process.
    RepositoryValidation,
    /// Run issue validation in process.
    IssueValidation,
    /// Resolve the target from a declared label namespace.
    LabelTargetValidation {
        /// Namespace whose label value selects the validation target.
        label_namespace: String,
    },
    /// Evaluate one configured graph rule with the gated issue as its sole
    /// firing subject and the captured repository as resolution context.
    RuleValidation {
        /// Exact rule name to evaluate.
        rule: String,
    },
    /// Advisory placeholder for an external review integration.
    ReviewPlaceholder,
}

/// Warning emitted by the review placeholder checker.
pub const REVIEW_PLACEHOLDER_WARNING: &str = "WARNING: EXTERNAL REVIEW PLACEHOLDER PASSED WITHOUT RUNNING A REVIEWER. Replace this checker with a real external review integration before relying on this gate.";

/// One authored quality-gate definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateDefinition {
    /// Schema version for future evolution.
    #[serde(default = "default_gate_version")]
    pub version: u32,
    /// Unique registry key.
    pub key: String,
    /// Human-readable name.
    pub title: String,
    /// Explanation of the check.
    pub description: String,
    /// Execution stage.
    pub stage: GateStage,
    /// Manual or automated mode.
    pub mode: GateMode,
    /// Automated checker declaration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checker: Option<GateChecker>,
    /// Repository files this gate's checker reads.
    ///
    /// Declaring them opts the gate into verdict reuse: an evaluation digests
    /// the declared set and, when a prior run of this gate recorded the same
    /// digest, takes that run's verdict instead of executing the checker
    /// again. Declare inputs only for a checker whose verdict is a function of
    /// those files alone — a checker scoped to one issue, or one that consults
    /// the clock, the network, or machine state, is not, and a gate that
    /// declares nothing executes on every evaluation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<RepositoryInputs>,
    /// Lower values execute first.
    #[serde(default = "default_gate_priority")]
    pub priority: u32,
    /// Reserved declaration extensions.
    #[serde(default)]
    pub reserved: HashMap<String, serde_json::Value>,
    /// Deprecated authored field preserved during round trips.
    #[serde(default)]
    pub auto: bool,
    /// Deprecated authored field preserved during round trips.
    pub example_integration: Option<String>,
}

impl GateDefinition {
    /// The declaration a verdict over this gate's inputs is bound to.
    ///
    /// A verdict is a function of two things: the content the checker reads,
    /// and the checker itself. Digesting the input content alone would let an
    /// edit to the checker — a changed command, timeout, working directory,
    /// environment, or prompt — leave the digest untouched, and an evaluation
    /// after that edit would carry a verdict the new checker never produced.
    /// This is the second half of that key: the stage the checker runs at, the
    /// checker declaration, and the input declaration bounding it, in a
    /// canonical byte encoding whose map ordering does not vary between runs.
    /// The fields left out — key, title, description, priority — cannot change
    /// what a checker does.
    ///
    /// # Errors
    ///
    /// Returns an error when the declaration cannot be encoded.
    pub fn verdict_declaration(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(&serde_json::json!({
            "stage": self.stage,
            "checker": self.checker,
            "inputs": self.inputs,
        }))
    }
}

fn default_gate_version() -> u32 {
    1
}

fn default_gate_priority() -> u32 {
    100
}

/// All authored gate definitions keyed by registry identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GateRegistry {
    /// Gate key to definition.
    pub gates: HashMap<String, GateDefinition>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AuthoredGateFile {
    #[serde(default)]
    gates: Vec<GateDefinition>,
}

/// Parse exact authored `gates.toml` bytes without filesystem access.
pub fn parse_gate_registry(bytes: &[u8]) -> Result<GateRegistry, GateDeclarationError> {
    let text = std::str::from_utf8(bytes)?;
    let file: AuthoredGateFile = toml::from_str(text)?;
    file.gates
        .into_iter()
        .try_fold(GateRegistry::default(), |mut registry, gate| {
            let key = gate.key.clone();
            if registry.gates.insert(key.clone(), gate).is_some() {
                Err(GateDeclarationError::DuplicateKey(key))
            } else {
                Ok(registry)
            }
        })
}

/// Serialize a registry deterministically while preserving authored fields.
pub fn serialize_gate_registry(registry: &GateRegistry) -> Result<Vec<u8>, GateDeclarationError> {
    let mut gates = registry.gates.values().cloned().collect::<Vec<_>>();
    gates.sort_by(|left, right| left.key.cmp(&right.key));
    for gate in &mut gates {
        gate.reserved.retain(|_, value| !value.is_null());
    }
    let mut bytes = toml::to_string_pretty(&AuthoredGateFile { gates })?.into_bytes();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    Ok(bytes)
}

/// Invalid authored gate declaration.
#[derive(Debug, thiserror::Error)]
pub enum GateDeclarationError {
    #[error("gate registry is not UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("gate registry TOML is invalid: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("gate registry cannot be serialized: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("duplicate gate key '{0}'")]
    DuplicateKey(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gate_registry_parser_rejects_duplicate_identity() {
        let bytes = br#"
[[gates]]
key = "review"
title = "One"
description = "One"
stage = "postcheck"
mode = "manual"

[[gates]]
key = "review"
title = "Two"
description = "Two"
stage = "postcheck"
mode = "manual"
"#;
        assert!(matches!(
            parse_gate_registry(bytes),
            Err(GateDeclarationError::DuplicateKey(key)) if key == "review"
        ));
    }

    /// One gate declaration whose `[gates.inputs]` body is under test.
    fn registry_with_inputs(body: &str) -> Result<GateRegistry, GateDeclarationError> {
        parse_gate_registry(
            format!(
                "[[gates]]\n\
                 key = \"workspace\"\n\
                 title = \"Workspace\"\n\
                 description = \"Workspace\"\n\
                 stage = \"postcheck\"\n\
                 mode = \"auto\"\n\
                 \n\
                 [gates.inputs]\n\
                 {body}"
            )
            .as_bytes(),
        )
    }

    /// REQ-01: the registry parser is where a root and an exclusion pattern are
    /// constrained, so no consumer can be the first to discover an unusable one.
    #[test]
    fn test_gate_registry_parser_rejects_input_values_it_cannot_constrain() {
        let accepted =
            registry_with_inputs("roots = [\"crates\"]\nexclude = [\"crates/**/target/**\"]\n")
                .expect("a declarable input set parses");
        let inputs = accepted.gates["workspace"]
            .inputs
            .as_ref()
            .expect("the parsed gate carries its declared inputs");
        assert!(inputs.covers("crates/jit/src/lib.rs"));
        assert!(!inputs.covers("crates/jit/target/debug/build.rs"));

        for rejected in [
            "roots = [\"/absolute\"]\n",
            "roots = [\"../escape\"]\n",
            "roots = []\n",
            "roots = [\"crates\"]\nexclude = [\"[unclosed\"]\n",
            "roots = [\"crates\"]\nexclude = [\"../escape/**\"]\n",
            "exclude = [\"crates/**\"]\n",
        ] {
            assert!(
                matches!(
                    registry_with_inputs(rejected),
                    Err(GateDeclarationError::Toml(_))
                ),
                "the registry parser accepted an unconstrained declaration: {rejected}"
            );
        }
    }

    /// A declared input set survives the registry's own serialize/parse round
    /// trip, so an authored declaration is not silently dropped by a rewrite.
    #[test]
    fn test_gate_registry_serialization_preserves_declared_inputs() {
        let registry = registry_with_inputs(
            "roots = [\"docs\", \"scripts\"]\nexclude = [\"docs/drafts/**\"]\n",
        )
        .expect("a declarable input set parses");
        let bytes = serialize_gate_registry(&registry).expect("the registry serializes");

        assert_eq!(
            parse_gate_registry(&bytes).expect("the rewrite parses"),
            registry
        );
    }

    #[test]
    fn test_gate_registry_serialization_is_sorted_and_round_trips() {
        let gate = |key: &str| GateDefinition {
            version: 1,
            key: key.to_string(),
            title: key.to_string(),
            description: key.to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Manual,
            checker: None,
            inputs: None,
            priority: 100,
            reserved: HashMap::new(),
            auto: false,
            example_integration: None,
        };
        let registry = GateRegistry {
            gates: HashMap::from([
                ("zeta".to_string(), gate("zeta")),
                ("alpha".to_string(), gate("alpha")),
            ]),
        };
        let bytes = serialize_gate_registry(&registry).unwrap();
        assert!(
            bytes
                .windows(13)
                .position(|window| window == b"key = \"alpha\"")
                < bytes
                    .windows(12)
                    .position(|window| window == b"key = \"zeta\"")
        );
        assert_eq!(parse_gate_registry(&bytes).unwrap(), registry);
    }
}
