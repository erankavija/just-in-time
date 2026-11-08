//! Neutral authored quality-gate declarations and pure TOML preservation.

use crate::domain::identity::GateKey;
use crate::domain::repository_inputs::RepositoryInputs;
use crate::domain::vocabulary::wire_vocabulary;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

wire_vocabulary! {
    /// Gate execution stage.
    #[derive(
        Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
    )]
    pub enum GateStage {
        /// Runs before work starts.
        Precheck = "precheck",
        /// Runs after work completes.
        Postcheck = "postcheck",
    }
}

wire_vocabulary! {
    /// Gate execution mode.
    #[derive(
        Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
    )]
    pub enum GateMode {
        /// Requires an explicit pass/fail.
        Manual = "manual",
        /// Has an automated checker.
        Auto = "auto",
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

/// One authored quality-gate definition. An unknown key fails the load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateDefinition {
    /// Schema version for future evolution.
    #[serde(default = "default_gate_version")]
    pub version: u32,
    /// Unique registry key.
    pub key: GateKey,
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
    pub gates: HashMap<GateKey, GateDefinition>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AuthoredGateFile {
    #[serde(default)]
    gates: Vec<GateDefinition>,
}

/// Parse exact authored `gates.toml` bytes without filesystem access.
pub fn parse_gate_registry(bytes: &[u8]) -> Result<GateRegistry, GateDeclarationError> {
    #[cfg(test)]
    super::count_parse(super::ParsedSource::Gates);
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

/// Remove reserved null values that a TOML document cannot represent.
pub(crate) fn normalize_gate_registry_for_document(registry: &mut GateRegistry) {
    registry.gates.values_mut().for_each(|gate| {
        gate.reserved.retain(|_, value| !value.is_null());
    });
}

/// Serialize a registry as deterministic TOML for a new document.
pub fn serialize_gate_registry(registry: &GateRegistry) -> Result<Vec<u8>, GateDeclarationError> {
    let mut registry = registry.clone();
    normalize_gate_registry_for_document(&mut registry);
    let mut gates = registry.gates.into_values().collect::<Vec<_>>();
    gates.sort_by(|left, right| left.key.cmp(&right.key));
    let mut bytes = toml::to_string_pretty(&AuthoredGateFile { gates })?.into_bytes();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    Ok(bytes)
}

/// Apply a registry proposal to an authored gates document, whose parse is
/// `previous`, without replacing declarations that did not change.
pub fn edit_gate_registry_document(
    authored: Option<&[u8]>,
    previous: &GateRegistry,
    registry: &GateRegistry,
) -> Result<Vec<u8>, GateDeclarationError> {
    use toml_edit::{ArrayOfTables, DocumentMut, Item};

    let mut document = authored
        .map(std::str::from_utf8)
        .transpose()?
        .map(str::parse::<DocumentMut>)
        .transpose()?
        .unwrap_or_default();

    match document.get("gates") {
        None => {
            document
                .as_table_mut()
                .insert("gates", Item::ArrayOfTables(ArrayOfTables::new()));
        }
        Some(item) if item.as_array_of_tables().is_some() => {}
        Some(item) if item.as_array().is_some_and(|array| array.is_empty()) => {
            document
                .as_table_mut()
                .insert("gates", Item::ArrayOfTables(ArrayOfTables::new()));
        }
        Some(_) => return Err(GateDeclarationError::MalformedDocument),
    }

    let gates = document
        .get_mut("gates")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or(GateDeclarationError::MalformedDocument)?;
    let removed = gates
        .iter()
        .enumerate()
        .map(|(index, table)| {
            gate_table_key(table).map(|key| (index, !registry.gates.contains_key(&key)))
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|(index, should_remove)| should_remove.then_some(index))
        .collect::<Vec<_>>();
    removed
        .into_iter()
        .rev()
        .for_each(|index| gates.remove(index));

    for table in gates.iter_mut() {
        let key = gate_table_key(table)?;
        if let (Some(previous), Some(updated)) =
            (previous.gates.get(&key), registry.gates.get(&key))
        {
            if previous != updated {
                let previous_rendered = render_gate_table(previous)?;
                let rendered = render_gate_table(updated)?;
                update_gate_table(table, &previous_rendered, &rendered);
            }
        }
    }

    let present = gates
        .iter()
        .map(gate_table_key)
        .collect::<Result<std::collections::HashSet<_>, _>>()?;
    let mut added = registry
        .gates
        .iter()
        .filter(|(key, _)| !present.contains(*key))
        .collect::<Vec<_>>();
    added.sort_by_key(|(key, _)| *key);
    for (_, gate) in added {
        gates.push(render_gate_table(gate)?);
    }

    let mut bytes = document.to_string().into_bytes();
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn gate_table_key(table: &toml_edit::Table) -> Result<GateKey, GateDeclarationError> {
    table
        .get("key")
        .and_then(toml_edit::Item::as_str)
        .ok_or(GateDeclarationError::MalformedDocument)?
        .parse()
        .map_err(|_| GateDeclarationError::MalformedDocument)
}

fn render_gate_table(gate: &GateDefinition) -> Result<toml_edit::Table, GateDeclarationError> {
    use toml_edit::{DocumentMut, Item};

    let registry = GateRegistry {
        gates: HashMap::from([(gate.key.clone(), gate.clone())]),
    };
    let bytes = serialize_gate_registry(&registry)?;
    let text = std::str::from_utf8(&bytes)?;
    let document = text.parse::<DocumentMut>()?;
    let mut table = document
        .get("gates")
        .and_then(Item::as_array_of_tables)
        .and_then(|gates| gates.get(0))
        .cloned()
        .ok_or(GateDeclarationError::MalformedDocument)?;
    if gate.reserved.is_empty() {
        table.remove("reserved");
    }
    // Cloned table positions refer to the temporary document, not this one.
    set_table_positions(&mut table, usize::MAX);
    Ok(table)
}

fn update_gate_table(
    table: &mut toml_edit::Table,
    previous: &toml_edit::Table,
    rendered: &toml_edit::Table,
) {
    let inserted_table_position = table.position().unwrap_or_default().saturating_add(1);
    let fields = previous
        .iter()
        .map(|(name, _)| name.to_owned())
        .chain(rendered.iter().map(|(name, _)| name.to_owned()))
        .collect::<std::collections::BTreeSet<_>>();

    fields.into_iter().for_each(|name| {
        let previous_item = render_table_item(previous, &name);
        let rendered_item = render_table_item(rendered, &name);
        if previous_item == rendered_item {
            return;
        }
        match rendered.get(&name) {
            Some(incoming) => match table.get_mut(&name) {
                Some(existing) => merge_authored_item(existing, incoming, inserted_table_position),
                None => {
                    table.insert(&name, incoming.clone());
                }
            },
            None => {
                table.remove(&name);
            }
        }
    });
}

fn render_table_item(table: &toml_edit::Table, name: &str) -> Option<String> {
    table.get(name).map(|item| {
        let mut document = toml_edit::DocumentMut::new();
        document.as_table_mut().insert(name, item.clone());
        document.to_string()
    })
}

fn merge_authored_item(
    existing: &mut toml_edit::Item,
    incoming: &toml_edit::Item,
    inserted_table_position: usize,
) {
    match (existing, incoming) {
        (toml_edit::Item::Value(existing), toml_edit::Item::Value(incoming)) => {
            let decor = existing.decor().clone();
            let mut updated = incoming.clone();
            *updated.decor_mut() = decor;
            *existing = updated;
        }
        (toml_edit::Item::Table(existing), toml_edit::Item::Table(incoming)) => {
            merge_authored_table(existing, incoming, inserted_table_position);
        }
        (existing, incoming) => *existing = incoming.clone(),
    }
}

fn merge_authored_table(
    existing: &mut toml_edit::Table,
    incoming: &toml_edit::Table,
    inserted_table_position: usize,
) {
    let existing_keys = existing
        .iter()
        .map(|(key, _)| key.to_string())
        .collect::<Vec<_>>();
    for key in existing_keys {
        match incoming.get(&key) {
            Some(incoming_item) => {
                if let Some(existing_item) = existing.get_mut(&key) {
                    merge_authored_item(existing_item, incoming_item, inserted_table_position);
                }
            }
            None => {
                existing.remove(&key);
            }
        }
    }
    for (key, incoming_item) in incoming.iter() {
        if existing.get(key).is_none() {
            let mut incoming_item = incoming_item.clone();
            set_item_table_positions(&mut incoming_item, inserted_table_position);
            existing.insert(key, incoming_item);
        }
    }
}

fn set_table_positions(table: &mut toml_edit::Table, position: usize) {
    table.set_position(position);
    table.iter_mut().for_each(|(_, item)| {
        set_item_table_positions(item, position);
    });
}

fn set_item_table_positions(item: &mut toml_edit::Item, position: usize) {
    match item {
        toml_edit::Item::Table(table) => set_table_positions(table, position),
        toml_edit::Item::ArrayOfTables(tables) => tables
            .iter_mut()
            .for_each(|table| set_table_positions(table, position)),
        _ => {}
    }
}

/// Invalid authored gate declaration.
#[derive(Debug, Clone, thiserror::Error)]
pub enum GateDeclarationError {
    #[error("gate registry is not UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("gate registry TOML is invalid: {}", .0.message())]
    Toml(#[from] toml::de::Error),
    #[error("gate registry cannot be serialized: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("gate registry document cannot be edited because its gates table is malformed")]
    MalformedDocument,
    #[error("gate registry TOML cannot be edited: {0}")]
    Edit(#[from] toml_edit::TomlError),
    #[error("duplicate gate key '{0}'")]
    DuplicateKey(GateKey),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gate_registry_parser_rejects_malformed_gate_key_with_its_error() {
        let key = "Not_Kebab";
        let error = parse_gate_registry(
            format!(
                "[[gates]]\n\
                 key = \"{key}\"\n\
                 title = \"Review\"\n\
                 description = \"Review\"\n\
                 stage = \"postcheck\"\n\
                 mode = \"manual\"\n"
            )
            .as_bytes(),
        )
        .expect_err("a malformed gate key must fail the load");
        let refusal = GateKey::try_from(key).unwrap_err().to_string();

        assert!(error.to_string().contains(&refusal), "{error}");
    }

    /// REQ-01: a key outside the declaration schema, including the removed
    /// `auto` and `example_integration`, fails the load naming the key.
    #[test]
    fn test_gate_registry_parser_rejects_unknown_key_by_name() {
        for key in ["auto", "example_integration", "undeclared_key"] {
            let error = parse_gate_registry(
                format!(
                    "[[gates]]\n\
                     key = \"review\"\n\
                     title = \"Review\"\n\
                     description = \"Review\"\n\
                     stage = \"postcheck\"\n\
                     mode = \"manual\"\n\
                     {key} = true\n"
                )
                .as_bytes(),
            )
            .expect_err("an unknown key must fail to parse");
            assert!(
                error
                    .to_string()
                    .contains(&format!("unknown field `{key}`")),
                "{key}: {error}"
            );
        }
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
        let inputs = accepted.gates[&"workspace".parse().unwrap()]
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
            key: key.parse().unwrap(),
            title: key.to_string(),
            description: key.to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Manual,
            checker: None,
            inputs: None,
            priority: 100,
            reserved: HashMap::new(),
        };
        let registry = GateRegistry {
            gates: HashMap::from([
                ("zeta".parse().unwrap(), gate("zeta")),
                ("alpha".parse().unwrap(), gate("alpha")),
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

    /// The canonical serializer reaches a fixed point over declarations. Edits
    /// to an existing file use edit_gate_registry_document to preserve its
    /// authored comments and order.
    #[test]
    fn test_serialize_gate_registry_reaches_a_fixed_point_over_authored_declarations() {
        let authored = br#"
# An authored comment the rewrite drops.
[[gates]]
version = 1
key = "workspace"
title = "Workspace"
description = "Runs a command over declared inputs."
stage = "postcheck"
mode = "auto"
priority = 50

[gates.checker]
type = "exec"
command = "./ci.sh"
timeout_seconds = 60
pass_context = true
prompt_file = "./prompt.md"

[gates.checker.env]
REVIEWER = "agent"

[gates.inputs]
roots = ["crates"]
exclude = ["crates/target/**"]

[[gates]]
key = "attested"
title = "Attested"
description = ""
stage = "precheck"
mode = "manual"
"#;
        let registry = parse_gate_registry(authored).unwrap();
        let bytes = serialize_gate_registry(&registry).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();

        assert_eq!(
            serialize_gate_registry(&parse_gate_registry(&bytes).unwrap()).unwrap(),
            bytes
        );
        assert_eq!(parse_gate_registry(&bytes).unwrap(), registry);
        registry.gates.values().for_each(|gate| {
            assert!(text.contains(&format!("stage = \"{}\"", gate.stage)));
            assert!(text.contains(&format!("mode = \"{}\"", gate.mode)));
        });
    }
}
