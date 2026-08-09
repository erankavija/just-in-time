//! Pure resolution and bounded substitution for profile variables.
//!
//! The package reader owns the unresolved bytes and package identity.  This
//! module only consumes that canonical package plus already-captured inputs;
//! filesystem and environment access stay at the command boundary.

use super::manifest::{ProfilePackageModel, ProfileVariableDeclaration};
use super::package::ProfilePackage;
use crate::repository_state::Contribution;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const REFERENCE_PREFIX: &str = "{{jit:";
const REFERENCE_KIND: &str = "var:";

/// Inputs captured by a command before it enters pure package resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VariableInputs {
    /// Values read from the optional values file, keyed by declared variable.
    pub values_file: BTreeMap<String, String>,
    /// Environment values keyed by the package variable they supply.
    pub environment: BTreeMap<String, String>,
    /// Repeated `--set NAME=VALUE` assignments in occurrence order.
    pub command_line: Vec<(String, String)>,
}

impl VariableInputs {
    /// Retain only inputs declared by one package while preserving assignment
    /// occurrence order.  Aggregate commands validate the complete input set
    /// before applying this package-local view.
    pub fn for_declarations(&self, declarations: &[ProfileVariableDeclaration]) -> Self {
        let names = declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<BTreeSet<_>>();
        Self {
            values_file: self
                .values_file
                .iter()
                .filter(|(name, _)| names.contains(name.as_str()))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            environment: self
                .environment
                .iter()
                .filter(|(name, _)| names.contains(name.as_str()))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            command_line: self
                .command_line
                .iter()
                .filter(|(name, _)| names.contains(name.as_str()))
                .cloned()
                .collect(),
        }
    }

    /// Reject input names that are not in a participating package set.
    pub fn validate_against_names(&self, names: &BTreeSet<String>) -> Result<(), VariableError> {
        self.values_file
            .keys()
            .try_for_each(|name| validate_input_name(names, name, "values file"))?;
        self.environment
            .keys()
            .try_for_each(|name| validate_input_name(names, name, "environment"))?;
        self.command_line
            .iter()
            .try_for_each(|(name, _)| validate_input_name(names, name, "--set"))
    }
}

/// The source that supplied one resolved variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VariableSource {
    /// The declaration supplied the value.
    Default,
    /// The optional values file supplied the value.
    ValuesFile,
    /// The declaration's environment variable supplied the value.
    Environment,
    /// A repeated command-line assignment supplied the value.
    Set,
}

/// One resolved value and its non-sensitive source classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedVariable {
    /// The resolved UTF-8 value.
    pub value: String,
    /// Which precedence tier supplied `value`.
    pub source: VariableSource,
}

/// Deterministic resolved variables, ordered by variable name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedVariables(BTreeMap<String, ResolvedVariable>);

impl ResolvedVariables {
    /// Return resolved values in a stable map suitable for a later record.
    pub fn values(&self) -> BTreeMap<String, String> {
        self.0
            .iter()
            .map(|(name, resolved)| (name.clone(), resolved.value.clone()))
            .collect()
    }

    /// Return source classifications in the same stable variable order.
    pub fn sources(&self) -> BTreeMap<String, VariableSource> {
        self.0
            .iter()
            .map(|(name, resolved)| (name.clone(), resolved.source))
            .collect()
    }

    fn get(&self, name: &str) -> Option<&ResolvedVariable> {
        self.0.get(name)
    }
}

/// Content after one non-recursive variable resolution pass.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedProfileContent {
    model: ProfilePackageModel,
    source_bytes: BTreeMap<String, Vec<u8>>,
    variables: ResolvedVariables,
}

impl ResolvedProfileContent {
    /// Canonical model with supported contribution strings substituted.
    pub fn model(&self) -> &ProfilePackageModel {
        &self.model
    }

    /// Resolved bytes for a declared asset or region source.
    pub fn source_bytes(&self, source: &str) -> Option<&[u8]> {
        self.source_bytes.get(source).map(Vec::as_slice)
    }

    /// Values returned for a caller that will persist resolved public inputs.
    pub fn variables(&self) -> &ResolvedVariables {
        &self.variables
    }
}

/// Resolve every declared value by the fixed package-variable precedence.
pub fn resolve_variables(
    declarations: &[ProfileVariableDeclaration],
    inputs: &VariableInputs,
) -> Result<ResolvedVariables, VariableError> {
    validate_declarations(declarations)?;
    let declared = declarations
        .iter()
        .map(|declaration| declaration.name.clone())
        .collect::<BTreeSet<_>>();
    inputs.validate_against_names(&declared)?;

    // A repeated --set intentionally has last-occurrence-wins semantics.
    let command_line = inputs
        .command_line
        .iter()
        .cloned()
        .collect::<BTreeMap<_, _>>();
    let resolved = declarations
        .iter()
        .filter_map(|declaration| {
            let selected = command_line
                .get(&declaration.name)
                .map(|value| (value.clone(), VariableSource::Set))
                .or_else(|| {
                    inputs
                        .environment
                        .get(&declaration.name)
                        .map(|value| (value.clone(), VariableSource::Environment))
                })
                .or_else(|| {
                    inputs
                        .values_file
                        .get(&declaration.name)
                        .map(|value| (value.clone(), VariableSource::ValuesFile))
                })
                .or_else(|| {
                    declaration
                        .default
                        .clone()
                        .map(|value| (value, VariableSource::Default))
                });
            selected.map(|(value, source)| {
                (declaration.name.clone(), ResolvedVariable { value, source })
            })
        })
        .collect();
    Ok(ResolvedVariables(resolved))
}

/// Resolve and substitute one immutable package without changing its package
/// identity or unresolved source bytes.
pub fn resolve_package(
    package: &ProfilePackage,
    inputs: &VariableInputs,
) -> Result<ResolvedProfileContent, VariableError> {
    let model = package.model();
    validate_model_references(model)?;
    let variables = resolve_variables(&model.variables, inputs)?;
    let contributions = model
        .contributions
        .iter()
        .enumerate()
        .map(|(index, contribution)| substitute_contribution(index, contribution, &variables))
        .collect::<Result<Vec<_>, _>>()?;
    let mut resolved_model = model.clone();
    resolved_model.contributions = contributions;

    let mut source_bytes = BTreeMap::new();
    for asset in &model.assets {
        let bytes = package
            .source_bytes(&asset.source)
            .ok_or_else(|| VariableError::MissingSource(asset.source.clone()))?;
        source_bytes.insert(
            asset.source.clone(),
            substitute_body(
                &format!("asset body '{}'", asset.source),
                asset.template,
                bytes,
                &variables,
            )?,
        );
    }
    for region in &model.regions {
        let bytes = package
            .source_bytes(&region.source)
            .ok_or_else(|| VariableError::MissingSource(region.source.clone()))?;
        source_bytes.insert(
            region.source.clone(),
            substitute_body(
                &format!("region body '{}'", region.source),
                region.template,
                bytes,
                &variables,
            )?,
        );
    }

    Ok(ResolvedProfileContent {
        model: resolved_model,
        source_bytes,
        variables,
    })
}

/// Validate reference syntax and positions without requiring input values.
pub(crate) fn validate_model_references(model: &ProfilePackageModel) -> Result<(), VariableError> {
    validate_declarations(&model.variables)?;

    model
        .contributions
        .iter()
        .enumerate()
        .try_for_each(|(index, contribution)| {
            validate_contribution_references(index, contribution)
        })?;

    model.assets.iter().try_for_each(|asset| {
        reject_references(&format!("asset source '{}'", asset.source), &asset.source)?;
        reject_references(&format!("asset target '{}'", asset.target), &asset.target)
    })?;
    model.regions.iter().try_for_each(|region| {
        reject_references(
            &format!("region source '{}'", region.source),
            &region.source,
        )?;
        reject_references(
            &format!("region target '{}'", region.target),
            &region.target,
        )?;
        reject_references(
            &format!("region id '{}'", region.region_id),
            &region.region_id,
        )
    })
}

/// Validate bodies at package-read time.  A templated body must be UTF-8;
/// unopted-in bodies may be binary but may not contain the reference prefix.
pub(crate) fn validate_body_references<'a, F>(
    model: &ProfilePackageModel,
    source_bytes: F,
) -> Result<(), VariableError>
where
    F: Fn(&str) -> Option<&'a [u8]>,
{
    for asset in &model.assets {
        let bytes = source_bytes(&asset.source)
            .ok_or_else(|| VariableError::MissingSource(asset.source.clone()))?;
        validate_body(
            &format!("asset body '{}'", asset.source),
            asset.template,
            bytes,
        )?;
    }
    for region in &model.regions {
        let bytes = source_bytes(&region.source)
            .ok_or_else(|| VariableError::MissingSource(region.source.clone()))?;
        validate_body(
            &format!("region body '{}'", region.source),
            region.template,
            bytes,
        )?;
    }
    Ok(())
}

fn validate_declarations(declarations: &[ProfileVariableDeclaration]) -> Result<(), VariableError> {
    let mut names = BTreeSet::new();
    declarations.iter().try_for_each(|declaration| {
        validate_name("variable name", &declaration.name)?;
        if !names.insert(declaration.name.clone()) {
            return Err(VariableError::DuplicateDeclaration(
                declaration.name.clone(),
            ));
        }
        if let Some(env) = &declaration.env {
            validate_name("environment variable name", env)?;
        }
        Ok(())
    })
}

fn validate_input_name(
    declared: &BTreeSet<String>,
    name: &str,
    source: &'static str,
) -> Result<(), VariableError> {
    if declared.contains(name) {
        Ok(())
    } else {
        Err(VariableError::UndeclaredInput {
            name: name.to_string(),
            tier: source,
        })
    }
}

fn validate_name(field: &'static str, name: &str) -> Result<(), VariableError> {
    let valid = name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_uppercase())
        && name.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        });
    if valid {
        Ok(())
    } else {
        Err(VariableError::InvalidName {
            field,
            name: name.to_string(),
        })
    }
}

fn validate_contribution_references(
    index: usize,
    contribution: &Contribution,
) -> Result<(), VariableError> {
    let field = |suffix: &str| format!("contribution[{index}].{suffix}");
    match contribution {
        Contribution::Scalar { value, .. } | Contribution::SetString { value, .. } => {
            reference_names(value, &field("value")).map(|_| ())
        }
        Contribution::MapEntry {
            identity, value, ..
        }
        | Contribution::KeyedArray {
            identity, value, ..
        } => {
            reject_references(&field("identity"), identity)?;
            validate_json_references(value, &field("value"), false)
        }
        Contribution::Projection { name, value } => {
            reject_references(&field("name"), name)?;
            reject_references(&field("value.target"), &value.target)
        }
    }
}

fn validate_json_references(
    value: &Value,
    field: &str,
    forbidden_key: bool,
) -> Result<(), VariableError> {
    match value {
        Value::String(value) => {
            if forbidden_key {
                reject_references(field, value)
            } else {
                reference_names(value, field).map(|_| ())
            }
        }
        Value::Array(values) => values.iter().enumerate().try_for_each(|(index, value)| {
            validate_json_references(value, &format!("{field}[{index}]"), forbidden_key)
        }),
        Value::Object(values) => values.iter().try_for_each(|(key, value)| {
            let child = format!("{field}.{key}");
            validate_json_references(value, &child, forbidden_key || forbidden_json_key(key))
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
}

fn forbidden_json_key(key: &str) -> bool {
    matches!(
        key,
        "id" | "key"
            | "name"
            | "target"
            | "source"
            | "path"
            | "mode"
            | "placement"
            | "style"
            | "kind"
            | "region-id"
            | "executable"
            | "template"
    )
}

fn reject_references(field: &str, value: &str) -> Result<(), VariableError> {
    reference_names(value, field)?
        .into_iter()
        .next()
        .map_or(Ok(()), |name| {
            Err(VariableError::ForbiddenReference {
                field: field.to_string(),
                name,
            })
        })
}

fn validate_body(field: &str, template: bool, bytes: &[u8]) -> Result<(), VariableError> {
    if template {
        let text = std::str::from_utf8(bytes).map_err(|_| VariableError::InvalidUtf8 {
            field: field.to_string(),
        })?;
        reference_names(text, field).map(|_| ())
    } else if let Some(start) = find_reference_prefix(bytes) {
        let text = String::from_utf8_lossy(&bytes[start..]);
        let name = text
            .strip_prefix(REFERENCE_PREFIX)
            .and_then(|tail| tail.split("}}").next())
            .and_then(|token| token.strip_prefix(REFERENCE_KIND))
            .unwrap_or("unknown")
            .to_string();
        Err(VariableError::ForbiddenReference {
            field: field.to_string(),
            name,
        })
    } else {
        Ok(())
    }
}

fn substitute_body(
    field: &str,
    template: bool,
    bytes: &[u8],
    variables: &ResolvedVariables,
) -> Result<Vec<u8>, VariableError> {
    if !template {
        validate_body(field, false, bytes)?;
        return Ok(bytes.to_vec());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| VariableError::InvalidUtf8 {
        field: field.to_string(),
    })?;
    substitute_string(field, text, variables).map(String::into_bytes)
}

fn substitute_contribution(
    index: usize,
    contribution: &Contribution,
    variables: &ResolvedVariables,
) -> Result<Contribution, VariableError> {
    let field = |suffix: &str| format!("contribution[{index}].{suffix}");
    match contribution {
        Contribution::Scalar { target, value } => Ok(Contribution::Scalar {
            target: *target,
            value: substitute_string(&field("value"), value, variables)?,
        }),
        Contribution::SetString { target, value } => Ok(Contribution::SetString {
            target: *target,
            value: substitute_string(&field("value"), value, variables)?,
        }),
        Contribution::MapEntry {
            target,
            identity,
            value,
        } => Ok(Contribution::MapEntry {
            target: *target,
            identity: reject_and_clone(&field("identity"), identity)?,
            value: substitute_json(value, &field("value"), variables, false)?,
        }),
        Contribution::KeyedArray {
            target,
            identity,
            value,
        } => Ok(Contribution::KeyedArray {
            target: *target,
            identity: reject_and_clone(&field("identity"), identity)?,
            value: substitute_json(value, &field("value"), variables, false)?,
        }),
        Contribution::Projection { name, value } => Ok(Contribution::Projection {
            name: reject_and_clone(&field("name"), name)?,
            value: value.clone(),
        }),
    }
}

fn reject_and_clone(field: &str, value: &str) -> Result<String, VariableError> {
    reject_references(field, value)?;
    Ok(value.to_string())
}

fn substitute_json(
    value: &Value,
    field: &str,
    variables: &ResolvedVariables,
    forbidden_key: bool,
) -> Result<Value, VariableError> {
    match value {
        Value::String(value) => {
            if forbidden_key || field.rsplit('.').next().is_some_and(forbidden_json_key) {
                reject_references(field, value)?;
                Ok(Value::String(value.clone()))
            } else {
                substitute_string(field, value, variables).map(Value::String)
            }
        }
        Value::Array(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                substitute_json(
                    value,
                    &format!("{field}[{index}]"),
                    variables,
                    forbidden_key,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| {
                substitute_json(
                    value,
                    &format!("{field}.{key}"),
                    variables,
                    forbidden_key || forbidden_json_key(key),
                )
                .map(|value| (key.clone(), value))
            })
            .collect::<Result<serde_json::Map<_, _>, _>>()
            .map(Value::Object),
        scalar => Ok(scalar.clone()),
    }
}

/// Substitute all references in one permitted string in one non-recursive pass.
pub fn substitute_string(
    field: &str,
    value: &str,
    variables: &ResolvedVariables,
) -> Result<String, VariableError> {
    let names = reference_names(value, field)?;
    if names.is_empty() {
        return Ok(value.to_string());
    }
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    while let Some(relative) = value[cursor..].find(REFERENCE_PREFIX) {
        let start = cursor + relative;
        output.push_str(&value[cursor..start]);
        let end = value[start..]
            .find("}}")
            .ok_or_else(|| VariableError::MalformedReference {
                field: field.to_string(),
            })?;
        let token = &value[start + REFERENCE_PREFIX.len()..start + end];
        let name = token.strip_prefix(REFERENCE_KIND).ok_or_else(|| {
            VariableError::MalformedReference {
                field: field.to_string(),
            }
        })?;
        let resolved = variables
            .get(name)
            .ok_or_else(|| VariableError::MissingValue {
                field: field.to_string(),
                name: name.to_string(),
            })?;
        output.push_str(&resolved.value);
        cursor = start + end + 2;
    }
    output.push_str(&value[cursor..]);
    Ok(output)
}

fn reference_names(value: &str, field: &str) -> Result<Vec<String>, VariableError> {
    let mut names = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = value[cursor..].find(REFERENCE_PREFIX) {
        let start = cursor + relative;
        let end = value[start..]
            .find("}}")
            .ok_or_else(|| VariableError::MalformedReference {
                field: field.to_string(),
            })?;
        let token = &value[start + REFERENCE_PREFIX.len()..start + end];
        let name = token
            .strip_prefix(REFERENCE_KIND)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| VariableError::MalformedReference {
                field: field.to_string(),
            })?;
        validate_name("reference variable name", name).map_err(|_| {
            VariableError::MalformedReference {
                field: field.to_string(),
            }
        })?;
        names.push(name.to_string());
        cursor = start + end + 2;
    }
    Ok(names)
}

fn find_reference_prefix(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(REFERENCE_PREFIX.len())
        .position(|window| window == REFERENCE_PREFIX.as_bytes())
}

/// Pure variable and reference failures.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VariableError {
    /// A declaration repeats a variable name.
    #[error("duplicate profile variable declaration '{0}'")]
    DuplicateDeclaration(String),
    /// A declaration or reference name is outside the portable grammar.
    #[error("invalid {field} '{name}'; expected [A-Z][A-Z0-9_]*")]
    InvalidName { field: &'static str, name: String },
    /// An input names no participating declaration.
    #[error("profile variable input '{name}' from {tier} is undeclared")]
    UndeclaredInput { name: String, tier: &'static str },
    /// A reference has no value from any source.
    #[error("field '{field}' references profile variable '{name}', but it has no resolved value")]
    MissingValue { field: String, name: String },
    /// A reference token is not exactly the frozen grammar.
    #[error("field '{field}' contains a malformed profile variable reference")]
    MalformedReference { field: String },
    /// A reference appears in an identity, path, mode, or other literal field.
    #[error("field '{field}' does not permit profile variable reference '{name}'")]
    ForbiddenReference { field: String, name: String },
    /// A templated body was not UTF-8.
    #[error("{field} is not UTF-8 and cannot be a profile variable body")]
    InvalidUtf8 { field: String },
    /// A declared package source was absent from the package image.
    #[error("declared package source '{0}' is unavailable")]
    MissingSource(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{
        AssetDeclaration, ProfileVariableDeclaration, RegionDeclaration, RegionPlacement,
    };
    use crate::repository_state::{Contribution, ScalarTarget, SetStringTarget};

    fn declaration(
        name: &str,
        default: Option<&str>,
        env: Option<&str>,
    ) -> ProfileVariableDeclaration {
        ProfileVariableDeclaration {
            name: name.to_string(),
            default: default.map(str::to_string),
            env: env.map(str::to_string),
        }
    }

    #[test]
    fn test_resolve_variables_applies_default_values_file_environment_and_last_set_precedence() {
        let declarations = [
            declaration("DEFAULT_ONLY", Some("default"), None),
            declaration("VALUES_FILE", Some("default"), None),
            declaration("ENVIRONMENT", Some("default"), Some("PROFILE_ENV")),
            declaration("COMMAND_LINE", Some("default"), None),
        ];
        let inputs = VariableInputs {
            values_file: [("VALUES_FILE", "file"), ("ENVIRONMENT", "file")]
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            environment: [("ENVIRONMENT", "environment")]
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            command_line: vec![
                ("COMMAND_LINE".to_string(), "first".to_string()),
                ("COMMAND_LINE".to_string(), "last".to_string()),
            ],
        };

        let resolved = resolve_variables(&declarations, &inputs).unwrap();

        assert_eq!(resolved.values()["DEFAULT_ONLY"], "default");
        assert_eq!(resolved.values()["VALUES_FILE"], "file");
        assert_eq!(resolved.values()["ENVIRONMENT"], "environment");
        assert_eq!(resolved.values()["COMMAND_LINE"], "last");
        assert_eq!(resolved.sources()["COMMAND_LINE"], VariableSource::Set);
    }

    #[test]
    fn test_resolve_variables_rejects_duplicate_declarations_and_undeclared_inputs() {
        let duplicate = resolve_variables(
            &[
                declaration("NAME", None, None),
                declaration("NAME", None, None),
            ],
            &VariableInputs::default(),
        )
        .unwrap_err();
        assert!(duplicate.to_string().contains("NAME"));
        assert!(duplicate.to_string().contains("duplicate"));

        let undeclared = resolve_variables(
            &[declaration("NAME", None, None)],
            &VariableInputs {
                command_line: vec![("OTHER".to_string(), "value".to_string())],
                ..VariableInputs::default()
            },
        )
        .unwrap_err();
        assert!(undeclared.to_string().contains("OTHER"));
        assert!(undeclared.to_string().contains("undeclared"));
    }

    #[test]
    fn test_substitute_string_is_multiple_and_non_recursive() {
        let declarations = [
            declaration("NAME", Some("Ada"), None),
            declaration("NESTED", Some("{{jit:var:NAME}}"), None),
        ];
        let resolved = resolve_variables(&declarations, &VariableInputs::default()).unwrap();

        let substituted = substitute_string(
            "contribution[0].value",
            "hello {{jit:var:NAME}}/{{jit:var:NESTED}}",
            &resolved,
        )
        .unwrap();

        assert_eq!(substituted, "hello Ada/{{jit:var:NAME}}");
    }

    #[test]
    fn test_substitute_rejects_identifiers_paths_modes_and_unopted_bodies() {
        let resolved = resolve_variables(
            &[declaration("NAME", Some("value"), None)],
            &VariableInputs::default(),
        )
        .unwrap();
        let identity = Contribution::MapEntry {
            target: crate::repository_state::MapEntryTarget::LabelAssociations,
            identity: "{{jit:var:NAME}}".to_string(),
            value: Value::String("literal".to_string()),
        };
        let error = substitute_contribution(0, &identity, &resolved).unwrap_err();
        assert!(error.to_string().contains("identity"));

        let forbidden_value = Contribution::MapEntry {
            target: crate::repository_state::MapEntryTarget::LabelAssociations,
            identity: "literal".to_string(),
            value: serde_json::json!({"path": "{{jit:var:NAME}}"}),
        };
        let error = substitute_contribution(0, &forbidden_value, &resolved).unwrap_err();
        assert!(error.to_string().contains("path"));
        let forbidden_mode = Contribution::MapEntry {
            target: crate::repository_state::MapEntryTarget::LabelAssociations,
            identity: "literal".to_string(),
            value: serde_json::json!({"mode": "{{jit:var:NAME}}"}),
        };
        let error = substitute_contribution(0, &forbidden_mode, &resolved).unwrap_err();
        assert!(error.to_string().contains("mode"));

        let model = ProfilePackageModel {
            id: "package".try_into().unwrap(),
            version: "1.0.0".to_string(),
            compatible_jit: "*".to_string(),
            dependencies: Vec::new(),
            incompatibilities: Vec::new(),
            variables: vec![declaration("NAME", Some("value"), None)],
            contributions: vec![Contribution::Scalar {
                target: ScalarTarget::DocumentationDevelopmentRoot,
                value: "{{jit:var:NAME}}".to_string(),
            }],
            assets: vec![AssetDeclaration {
                source: "asset.txt".to_string(),
                target: "docs/file.txt".to_string(),
                executable: false,
                template: false,
            }],
            regions: vec![RegionDeclaration {
                source: "region.txt".to_string(),
                target: "AGENTS.md".to_string(),
                region_id: "guidance".to_string(),
                placement: RegionPlacement::Append,
                template: false,
            }],
            live_sources: Vec::new(),
        };
        let path = AssetDeclaration {
            source: "{{jit:var:NAME}}".to_string(),
            target: "docs/file.txt".to_string(),
            executable: false,
            template: false,
        };
        let mut bad_path = model.clone();
        bad_path.assets = vec![path];
        let error = validate_model_references(&bad_path).unwrap_err();
        assert!(error.to_string().contains("asset source"));
        assert!(validate_body("asset body 'asset.txt'", false, b"{{jit:var:NAME}}").is_err());
        assert_eq!(
            substitute_body(
                "asset body 'asset.txt'",
                true,
                b"{{jit:var:NAME}}",
                &resolved
            )
            .unwrap(),
            b"value"
        );
    }

    #[test]
    fn test_substitute_allowed_scalar_and_set_string_values() {
        let resolved = resolve_variables(
            &[declaration("NAME", Some("value"), None)],
            &VariableInputs::default(),
        )
        .unwrap();
        let scalar = Contribution::Scalar {
            target: ScalarTarget::DocumentationDevelopmentRoot,
            value: "docs/{{jit:var:NAME}}".to_string(),
        };
        let set = Contribution::SetString {
            target: SetStringTarget::DocumentationManagedPaths,
            value: "docs/{{jit:var:NAME}}".to_string(),
        };
        assert!(matches!(
            substitute_contribution(0, &scalar, &resolved).unwrap(),
            Contribution::Scalar { value, .. } if value == "docs/value"
        ));
        assert!(matches!(
            substitute_contribution(1, &set, &resolved).unwrap(),
            Contribution::SetString { value, .. } if value == "docs/value"
        ));
    }
}
