//! Pure resolution and bounded substitution for profile variables.
//!
//! The package reader owns the unresolved bytes and package identity.  This
//! module only consumes that canonical package plus already-captured inputs;
//! filesystem and environment access stay at the command boundary.

use super::manifest::{
    EnvironmentVariableName, ProfilePackageModel, ProfileVariableDeclaration, ProfileVariableName,
};
use super::package::ProfilePackage;
use crate::repository_state::{Contribution, KeyedArrayTarget, MapEntryTarget};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const REFERENCE_PREFIX: &str = "{{jit:";
const REFERENCE_KIND: &str = "var:";
const RESOLVED_TARGET_HASH_DOMAIN: &[u8] = b"jit-profile-resolved-target-v1\0";

/// Inputs captured by a command before it enters pure package resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VariableInputs {
    /// Values read from the optional values file, keyed by declared variable.
    pub values_file: BTreeMap<ProfileVariableName, String>,
    /// Environment values keyed by the declared operating-system name.
    pub environment: BTreeMap<EnvironmentVariableName, String>,
    /// Repeated `--set NAME=VALUE` assignments in occurrence order.
    pub command_line: Vec<ProfileVariableAssignment>,
}

/// One typed command-line assignment after the CLI adapter has parsed NAME=VALUE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileVariableAssignment {
    /// Declared profile-variable name.
    pub name: ProfileVariableName,
    /// Non-secret value supplied by the caller.
    pub value: String,
}

impl ProfileVariableAssignment {
    /// Construct an assignment from already-validated semantic pieces.
    pub fn new(name: ProfileVariableName, value: impl Into<String>) -> Self {
        Self {
            name,
            value: value.into(),
        }
    }
}

impl VariableInputs {
    /// Retain only inputs declared by one package while preserving assignment
    /// occurrence order.  Aggregate commands validate the complete input set
    /// before applying this package-local view.
    pub fn for_declarations(&self, declarations: &[ProfileVariableDeclaration]) -> Self {
        let names = declarations
            .iter()
            .map(|declaration| &declaration.name)
            .collect::<BTreeSet<_>>();
        let environment_names = declarations
            .iter()
            .filter_map(|declaration| declaration.env.as_ref())
            .collect::<BTreeSet<_>>();
        Self {
            values_file: self
                .values_file
                .iter()
                .filter(|(name, _)| names.contains(name))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            environment: self
                .environment
                .iter()
                .filter(|(name, _)| environment_names.contains(name))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            command_line: self
                .command_line
                .iter()
                .filter(|assignment| names.contains(&assignment.name))
                .cloned()
                .collect(),
        }
    }

    /// Reject input names that are not in a participating package set.
    pub fn validate_against_names(
        &self,
        names: &BTreeSet<ProfileVariableName>,
        environment_names: &BTreeSet<EnvironmentVariableName>,
    ) -> Result<(), VariableError> {
        self.values_file
            .keys()
            .try_for_each(|name| validate_input_name(names, name, "values file"))?;
        self.environment
            .keys()
            .try_for_each(|name| validate_environment_name(environment_names, name))?;
        self.command_line
            .iter()
            .try_for_each(|assignment| validate_input_name(names, &assignment.name, "--set"))
    }
}

/// The source that supplied one resolved variable.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedVariable {
    /// The resolved UTF-8 value.
    pub value: String,
    /// Which precedence tier supplied `value`.
    pub source: VariableSource,
}

/// Deterministic resolved variables, ordered by variable name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ResolvedVariables(BTreeMap<ProfileVariableName, ResolvedVariable>);

impl ResolvedVariables {
    /// Return resolved values in a stable typed-name map.
    pub fn values(&self) -> BTreeMap<ProfileVariableName, String> {
        self.0
            .iter()
            .map(|(name, resolved)| (name.clone(), resolved.value.clone()))
            .collect()
    }

    /// Return source classifications in the same stable typed-name order.
    pub fn sources(&self) -> BTreeMap<ProfileVariableName, VariableSource> {
        self.0
            .iter()
            .map(|(name, resolved)| (name.clone(), resolved.source))
            .collect()
    }

    fn get(&self, name: &ProfileVariableName) -> Option<&ResolvedVariable> {
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
    let environment_names = declarations
        .iter()
        .filter_map(|declaration| declaration.env.clone())
        .collect::<BTreeSet<_>>();
    inputs.validate_against_names(&declared, &environment_names)?;

    // A repeated --set intentionally has last-occurrence-wins semantics.
    let command_line = inputs
        .command_line
        .iter()
        .map(|assignment| (assignment.name.clone(), assignment.value.clone()))
        .collect::<BTreeMap<_, _>>();
    let resolved = declarations
        .iter()
        .filter_map(|declaration| {
            let selected = command_line
                .get(&declaration.name)
                .map(|value| (value.clone(), VariableSource::Set))
                .or_else(|| {
                    declaration.env.as_ref().and_then(|environment| {
                        inputs
                            .environment
                            .get(environment)
                            .map(|value| (value.clone(), VariableSource::Environment))
                    })
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

/// Resolve declared variables from applied-record values, with newly supplied
/// values taking their ordinary precedence over the record.
///
/// Reconfiguration deliberately never falls back to declaration defaults:
/// once a value was recorded, that record is the deterministic baseline for
/// replaying the installed package.
pub fn resolve_variables_from_record(
    declarations: &[ProfileVariableDeclaration],
    recorded: &ResolvedVariables,
    inputs: &VariableInputs,
) -> Result<ResolvedVariables, VariableError> {
    resolve_variables_from_record_with_defaults(declarations, recorded, inputs, false)
}

/// Resolve variables for a replacement package from an installed record.
///
/// Values whose declarations survive are carried forward from `recorded`.
/// Values first declared by the replacement package fall back to that package's
/// defaults. Values no longer declared by the replacement package are dropped.
pub fn resolve_variables_for_upgrade(
    declarations: &[ProfileVariableDeclaration],
    recorded: &ResolvedVariables,
    inputs: &VariableInputs,
) -> Result<ResolvedVariables, VariableError> {
    let declared = declarations
        .iter()
        .map(|declaration| declaration.name.clone())
        .collect::<BTreeSet<_>>();
    let retained = ResolvedVariables(
        recorded
            .0
            .iter()
            .filter(|(name, _)| declared.contains(*name))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    );
    resolve_variables_from_record_with_defaults(declarations, &retained, inputs, true)
}

fn resolve_variables_from_record_with_defaults(
    declarations: &[ProfileVariableDeclaration],
    recorded: &ResolvedVariables,
    inputs: &VariableInputs,
    include_defaults_for_missing_values: bool,
) -> Result<ResolvedVariables, VariableError> {
    validate_declarations(declarations)?;
    validate_recorded_variables(declarations, recorded)?;
    let declared = declarations
        .iter()
        .map(|declaration| declaration.name.clone())
        .collect::<BTreeSet<_>>();
    let environment_names = declarations
        .iter()
        .filter_map(|declaration| declaration.env.clone())
        .collect::<BTreeSet<_>>();
    inputs.validate_against_names(&declared, &environment_names)?;

    let command_line = inputs
        .command_line
        .iter()
        .map(|assignment| (assignment.name.clone(), assignment.value.clone()))
        .collect::<BTreeMap<_, _>>();
    let resolved = declarations
        .iter()
        .filter_map(|declaration| {
            let selected = command_line
                .get(&declaration.name)
                .map(|value| (value.clone(), VariableSource::Set))
                .or_else(|| {
                    declaration.env.as_ref().and_then(|environment| {
                        inputs
                            .environment
                            .get(environment)
                            .map(|value| (value.clone(), VariableSource::Environment))
                    })
                })
                .or_else(|| {
                    inputs
                        .values_file
                        .get(&declaration.name)
                        .map(|value| (value.clone(), VariableSource::ValuesFile))
                })
                .or_else(|| {
                    recorded
                        .get(&declaration.name)
                        .map(|value| (value.value.clone(), value.source))
                })
                .or_else(|| {
                    include_defaults_for_missing_values.then(|| {
                        declaration
                            .default
                            .clone()
                            .map(|value| (value, VariableSource::Default))
                    })?
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
    resolve_package_content(package, variables)
}

/// Resolve a package exclusively from the public variable provenance stored
/// in its applied-profile record.
///
/// The current process environment and declaration defaults are never input
/// channels here. Recorded names and source kinds are checked against the
/// current unresolved package before its exact stored values are rendered.
///
/// # Errors
///
/// Returns [`VariableError`] when the record names an undeclared variable, a
/// recorded source kind is impossible for its declaration, or the package's
/// bounded references cannot be rendered from the recorded values.
pub fn resolve_package_from_record(
    package: &ProfilePackage,
    variables: &ResolvedVariables,
) -> Result<ResolvedProfileContent, VariableError> {
    resolve_package_from_record_with_inputs(package, variables, &VariableInputs::default())
}

/// Resolve an installed package from its recorded values and newly supplied
/// command inputs, without consulting declaration defaults for a stored value.
pub fn resolve_package_from_record_with_inputs(
    package: &ProfilePackage,
    variables: &ResolvedVariables,
    inputs: &VariableInputs,
) -> Result<ResolvedProfileContent, VariableError> {
    let model = package.model();
    validate_model_references(model)?;
    resolve_package_content(
        package,
        resolve_variables_from_record(&model.variables, variables, inputs)?,
    )
}

/// Resolve a replacement package from the surviving values of its installed
/// predecessor, defaulting only variables newly declared by that replacement.
pub fn resolve_package_for_upgrade(
    package: &ProfilePackage,
    variables: &ResolvedVariables,
    inputs: &VariableInputs,
) -> Result<ResolvedProfileContent, VariableError> {
    let model = package.model();
    validate_model_references(model)?;
    resolve_package_content(
        package,
        resolve_variables_for_upgrade(&model.variables, variables, inputs)?,
    )
}

fn resolve_package_content(
    package: &ProfilePackage,
    variables: ResolvedVariables,
) -> Result<ResolvedProfileContent, VariableError> {
    let model = package.model();
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

fn validate_recorded_variables(
    declarations: &[ProfileVariableDeclaration],
    variables: &ResolvedVariables,
) -> Result<(), VariableError> {
    validate_declarations(declarations)?;
    let declarations = declarations
        .iter()
        .map(|declaration| (&declaration.name, declaration))
        .collect::<BTreeMap<_, _>>();
    variables.0.iter().try_for_each(|(name, resolved)| {
        let declaration = declarations
            .get(name)
            .ok_or_else(|| VariableError::UndeclaredInput {
                name: name.to_string(),
                tier: "applied profile record",
            })?;
        let valid_source = match resolved.source {
            VariableSource::Default => declaration.default.as_ref() == Some(&resolved.value),
            VariableSource::Environment => declaration.env.is_some(),
            VariableSource::ValuesFile | VariableSource::Set => true,
        };
        if valid_source {
            Ok(())
        } else {
            Err(VariableError::InvalidRecordedSource {
                name: name.to_string(),
                source_kind: resolved.source,
            })
        }
    })
}

impl ResolvedProfileContent {
    /// Hash resolved semantic contributions and resolved file bytes by their
    /// repository target without exposing the resolved values themselves.
    pub fn target_hashes(&self) -> Result<BTreeMap<String, String>, VariableError> {
        let variable_frame = canonical_json_bytes(&self.variables)?;
        let mut frames = BTreeMap::<String, Vec<Vec<u8>>>::new();
        for contribution in &self.model.contributions {
            frames
                .entry(contribution.registry_path().to_string())
                .or_default()
                .push(canonical_json_bytes(contribution)?);
        }
        for asset in &self.model.assets {
            let mut frame = canonical_json_bytes(asset)?;
            append_frame(
                &mut frame,
                self.source_bytes(&asset.source)
                    .ok_or_else(|| VariableError::MissingSource(asset.source.clone()))?,
            );
            frames.entry(asset.target.clone()).or_default().push(frame);
        }
        for region in &self.model.regions {
            let mut frame = canonical_json_bytes(region)?;
            append_frame(
                &mut frame,
                self.source_bytes(&region.source)
                    .ok_or_else(|| VariableError::MissingSource(region.source.clone()))?,
            );
            frames.entry(region.target.clone()).or_default().push(frame);
        }
        frames
            .into_iter()
            .map(|(target, target_frames)| {
                let mut hasher = Sha256::new();
                hasher.update(RESOLVED_TARGET_HASH_DOMAIN);
                hash_frame(&mut hasher, b"variables");
                hash_frame(&mut hasher, &variable_frame);
                hash_frame(&mut hasher, target.as_bytes());
                target_frames
                    .iter()
                    .for_each(|frame| hash_frame(&mut hasher, frame));
                Ok((target, format!("{:x}", hasher.finalize())))
            })
            .collect()
    }
}

fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, VariableError> {
    serde_json::to_vec(&canonicalize_json(
        serde_json::to_value(value)
            .map_err(|error| VariableError::Serialization(error.to_string()))?,
    ))
    .map_err(|error| VariableError::Serialization(error.to_string()))
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        scalar => scalar,
    }
}

fn append_frame(frame: &mut Vec<u8>, bytes: &[u8]) {
    frame.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    frame.extend_from_slice(bytes);
}

fn hash_frame<D: Digest>(hasher: &mut D, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
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
            region.region_id.as_str(),
        )
    })?;
    model.live_sources.iter().try_for_each(|live_source| {
        reject_references(
            &format!("live-source root '{}'", live_source.root),
            live_source.root.as_str(),
        )?;
        live_source.exclude.iter().try_for_each(|exclude| {
            reject_references(
                &format!("live-source exclusion '{}'", exclude),
                exclude.as_str(),
            )
        })
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
        if !names.insert(declaration.name.clone()) {
            return Err(VariableError::DuplicateDeclaration(
                declaration.name.to_string(),
            ));
        }
        Ok(())
    })
}

fn validate_input_name(
    declared: &BTreeSet<ProfileVariableName>,
    name: &ProfileVariableName,
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

fn validate_environment_name(
    declared: &BTreeSet<EnvironmentVariableName>,
    name: &EnvironmentVariableName,
) -> Result<(), VariableError> {
    if declared.contains(name) {
        Ok(())
    } else {
        Err(VariableError::UndeclaredInput {
            name: name.to_string(),
            tier: "environment",
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
            reject_references(&field("value"), value)
        }
        Contribution::MapEntry {
            target,
            identity,
            value,
        } => {
            reject_references(&field("identity"), identity)?;
            validate_json_references(
                value,
                &field("value"),
                json_reference_policy_for_map(*target),
                Vec::new(),
            )
        }
        Contribution::KeyedArray {
            target,
            identity,
            value,
        } => {
            reject_references(&field("identity"), identity)?;
            validate_json_references(
                value,
                &field("value"),
                JsonReferencePolicy::KeyedArray(*target),
                Vec::new(),
            )
        }
        Contribution::Projection { name, value } => {
            reject_references(&field("name"), name)?;
            reject_references(&field("value.target"), &value.target)
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum JsonReferencePolicy {
    None,
    Namespace,
    KeyedArray(KeyedArrayTarget),
}

fn json_reference_policy_for_map(target: MapEntryTarget) -> JsonReferencePolicy {
    match target {
        MapEntryTarget::Namespaces => JsonReferencePolicy::Namespace,
        MapEntryTarget::TypeHierarchyTypes
        | MapEntryTarget::LabelAssociations
        | MapEntryTarget::ItemKinds => JsonReferencePolicy::None,
    }
}

fn allows_free_form_reference(policy: JsonReferencePolicy, path: &[String]) -> bool {
    match (policy, path) {
        (JsonReferencePolicy::Namespace, [field]) => field == "description",
        (JsonReferencePolicy::KeyedArray(KeyedArrayTarget::Gates), [field]) => {
            field == "title" || field == "description"
        }
        (JsonReferencePolicy::KeyedArray(KeyedArrayTarget::Gates), [parent, field]) => {
            parent == "checker" && field == "prompt"
        }
        (
            JsonReferencePolicy::KeyedArray(
                KeyedArrayTarget::Invariants
                | KeyedArrayTarget::Rules
                | KeyedArrayTarget::Templates,
            ),
            [field],
        ) => field == "description",
        (JsonReferencePolicy::None, _)
        | (JsonReferencePolicy::Namespace, _)
        | (JsonReferencePolicy::KeyedArray(_), _) => false,
    }
}

fn validate_json_references(
    value: &Value,
    field: &str,
    policy: JsonReferencePolicy,
    path: Vec<String>,
) -> Result<(), VariableError> {
    match value {
        Value::String(value) => {
            if allows_free_form_reference(policy, &path) {
                reference_names(value, field).map(|_| ())
            } else {
                reject_references(field, value)
            }
        }
        Value::Array(values) => values.iter().enumerate().try_for_each(|(index, value)| {
            let mut child_path = path.clone();
            child_path.push(index.to_string());
            validate_json_references(value, &format!("{field}[{index}]"), policy, child_path)
        }),
        Value::Object(values) => values.iter().try_for_each(|(key, value)| {
            reject_references(&format!("{field} key"), key)?;
            let mut child_path = path.clone();
            child_path.push(key.clone());
            let child = format!("{field}.{key}");
            validate_json_references(value, &child, policy, child_path)
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
    }
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
            value: reject_and_clone(&field("value"), value)?,
        }),
        Contribution::SetString { target, value } => Ok(Contribution::SetString {
            target: *target,
            value: reject_and_clone(&field("value"), value)?,
        }),
        Contribution::MapEntry {
            target,
            identity,
            value,
        } => Ok(Contribution::MapEntry {
            target: *target,
            identity: reject_and_clone(&field("identity"), identity)?,
            value: substitute_json(
                value,
                &field("value"),
                variables,
                json_reference_policy_for_map(*target),
                Vec::new(),
            )?,
        }),
        Contribution::KeyedArray {
            target,
            identity,
            value,
        } => Ok(Contribution::KeyedArray {
            target: *target,
            identity: reject_and_clone(&field("identity"), identity)?,
            value: substitute_json(
                value,
                &field("value"),
                variables,
                JsonReferencePolicy::KeyedArray(*target),
                Vec::new(),
            )?,
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
    policy: JsonReferencePolicy,
    path: Vec<String>,
) -> Result<Value, VariableError> {
    match value {
        Value::String(value) => {
            if allows_free_form_reference(policy, &path) {
                substitute_string(field, value, variables).map(Value::String)
            } else {
                reject_references(field, value)?;
                Ok(Value::String(value.clone()))
            }
        }
        Value::Array(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let mut child_path = path.clone();
                child_path.push(index.to_string());
                substitute_json(
                    value,
                    &format!("{field}[{index}]"),
                    variables,
                    policy,
                    child_path,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| {
                reject_references(&format!("{field} key"), key)?;
                let mut child_path = path.clone();
                child_path.push(key.clone());
                substitute_json(
                    value,
                    &format!("{field}.{key}"),
                    variables,
                    policy,
                    child_path,
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
        let name =
            ProfileVariableName::try_from(name).map_err(|_| VariableError::MalformedReference {
                field: field.to_string(),
            })?;
        let resolved = variables
            .get(&name)
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
    /// A resolved semantic definition could not be serialized for hashing.
    #[error("failed to serialize resolved profile content for hashing: {0}")]
    Serialization(String),
    /// Stored provenance assigns a source kind the current declaration cannot
    /// have produced.
    #[error(
        "applied profile record gives variable '{name}' an invalid source kind {source_kind:?}"
    )]
    InvalidRecordedSource {
        /// Declared variable whose stored provenance is invalid.
        name: String,
        /// Stored source kind rejected by the declaration.
        source_kind: VariableSource,
    },
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
            name: name.try_into().unwrap(),
            default: default.map(str::to_string),
            env: env.map(|env| env.try_into().unwrap()),
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
                .map(|(name, value)| (name.try_into().unwrap(), value.to_string()))
                .collect(),
            environment: [("PROFILE_ENV", "environment")]
                .into_iter()
                .map(|(name, value)| (name.try_into().unwrap(), value.to_string()))
                .collect(),
            command_line: vec![
                ProfileVariableAssignment::new("COMMAND_LINE".try_into().unwrap(), "first"),
                ProfileVariableAssignment::new("COMMAND_LINE".try_into().unwrap(), "last"),
            ],
        };

        let resolved = resolve_variables(&declarations, &inputs).unwrap();

        assert_eq!(
            resolved.values()[&"DEFAULT_ONLY".try_into().unwrap()],
            "default"
        );
        assert_eq!(
            resolved.values()[&"VALUES_FILE".try_into().unwrap()],
            "file"
        );
        assert_eq!(
            resolved.values()[&"ENVIRONMENT".try_into().unwrap()],
            "environment"
        );
        assert_eq!(
            resolved.values()[&"COMMAND_LINE".try_into().unwrap()],
            "last"
        );
        assert_eq!(
            resolved.sources()[&"COMMAND_LINE".try_into().unwrap()],
            VariableSource::Set
        );
    }

    #[test]
    fn test_resolve_variables_from_record_preserves_stored_values_until_supplied_inputs_override_them(
    ) {
        let declarations = [
            declaration("UNCHANGED", Some("manifest-default"), None),
            declaration("FROM_ENV", Some("manifest-default"), Some("PROFILE_ENV")),
            declaration("FROM_SET", Some("manifest-default"), None),
        ];
        let stored = ResolvedVariables(
            [
                (
                    "UNCHANGED".try_into().unwrap(),
                    ResolvedVariable {
                        value: "stored-value".to_string(),
                        source: VariableSource::ValuesFile,
                    },
                ),
                (
                    "FROM_ENV".try_into().unwrap(),
                    ResolvedVariable {
                        value: "stored-environment".to_string(),
                        source: VariableSource::Environment,
                    },
                ),
                (
                    "FROM_SET".try_into().unwrap(),
                    ResolvedVariable {
                        value: "stored-set".to_string(),
                        source: VariableSource::Set,
                    },
                ),
            ]
            .into_iter()
            .collect(),
        );
        let inputs = VariableInputs {
            values_file: [("UNCHANGED", "file-value")]
                .into_iter()
                .map(|(name, value)| (name.try_into().unwrap(), value.to_string()))
                .collect(),
            environment: [("PROFILE_ENV", "environment-value")]
                .into_iter()
                .map(|(name, value)| (name.try_into().unwrap(), value.to_string()))
                .collect(),
            command_line: vec![ProfileVariableAssignment::new(
                "FROM_SET".try_into().unwrap(),
                "set-value",
            )],
        };

        let resolved = resolve_variables_from_record(&declarations, &stored, &inputs).unwrap();

        assert_eq!(
            resolved.values()[&"UNCHANGED".try_into().unwrap()],
            "file-value"
        );
        assert_eq!(
            resolved.values()[&"FROM_ENV".try_into().unwrap()],
            "environment-value"
        );
        assert_eq!(
            resolved.values()[&"FROM_SET".try_into().unwrap()],
            "set-value"
        );
        assert_eq!(
            resolved.sources()[&"UNCHANGED".try_into().unwrap()],
            VariableSource::ValuesFile
        );
    }

    #[test]
    fn test_resolve_variables_for_upgrade_carries_stored_values_and_defaults_new_declarations() {
        let declarations = [
            declaration("CARRIED", Some("new-default"), None),
            declaration("NEW", Some("new-default"), None),
        ];
        let stored = ResolvedVariables(
            [
                (
                    "CARRIED".try_into().unwrap(),
                    ResolvedVariable {
                        value: "stored-value".to_string(),
                        source: VariableSource::Set,
                    },
                ),
                (
                    "REMOVED".try_into().unwrap(),
                    ResolvedVariable {
                        value: "obsolete-value".to_string(),
                        source: VariableSource::Set,
                    },
                ),
            ]
            .into_iter()
            .collect(),
        );

        let resolved =
            resolve_variables_for_upgrade(&declarations, &stored, &VariableInputs::default())
                .unwrap();

        assert_eq!(
            resolved.values(),
            [
                ("CARRIED".try_into().unwrap(), "stored-value".to_string()),
                ("NEW".try_into().unwrap(), "new-default".to_string()),
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn test_variable_inputs_route_same_name_through_each_package_environment_declaration() {
        let inputs = VariableInputs {
            environment: [("ENV_A", "value-from-a"), ("ENV_B", "value-from-b")]
                .into_iter()
                .map(|(name, value)| (name.try_into().unwrap(), value.to_string()))
                .collect(),
            ..VariableInputs::default()
        };
        let package_a = [declaration("NAME", None, Some("ENV_A"))];
        let package_b = [declaration("NAME", None, Some("ENV_B"))];

        let resolved_a =
            resolve_variables(&package_a, &inputs.for_declarations(&package_a)).unwrap();
        let resolved_b =
            resolve_variables(&package_b, &inputs.for_declarations(&package_b)).unwrap();

        assert_eq!(
            resolved_a.values()[&"NAME".try_into().unwrap()],
            "value-from-a"
        );
        assert_eq!(
            resolved_b.values()[&"NAME".try_into().unwrap()],
            "value-from-b"
        );
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
                command_line: vec![ProfileVariableAssignment::new(
                    "OTHER".try_into().unwrap(),
                    "value",
                )],
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
                value: "docs".to_string(),
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
                region_id: "guidance".try_into().expect("test region id is canonical"),
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
    fn test_substitute_rejects_every_nested_semantic_key_and_object_key_reference() {
        let resolved = resolve_variables(
            &[declaration("NAME", Some("value"), None)],
            &VariableInputs::default(),
        )
        .unwrap();
        for key in [
            "working_dir",
            "prompt_file",
            "roots",
            "applies_to",
            "mode",
            "path",
            "target",
            "source",
            "name",
            "key",
        ] {
            let contribution = Contribution::KeyedArray {
                target: KeyedArrayTarget::Gates,
                identity: "gate".to_string(),
                value: serde_json::json!({key: "{{jit:var:NAME}}"}),
            };
            let error = substitute_contribution(0, &contribution, &resolved).unwrap_err();
            assert!(
                error.to_string().contains(key),
                "semantic key {key} must reject references: {error}"
            );
        }

        let keyed_object = Contribution::KeyedArray {
            target: KeyedArrayTarget::Templates,
            identity: "template".to_string(),
            value: serde_json::json!({
                "{{jit:var:NAME}}": "literal",
            }),
        };
        assert!(substitute_contribution(0, &keyed_object, &resolved).is_err());
    }

    #[test]
    fn test_substitute_rejects_constrained_scalar_and_set_string_values() {
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
        assert!(substitute_contribution(0, &scalar, &resolved).is_err());
        assert!(substitute_contribution(1, &set, &resolved).is_err());

        let free_form = Contribution::MapEntry {
            target: crate::repository_state::MapEntryTarget::Namespaces,
            identity: "component".to_string(),
            value: serde_json::json!({
                "description": "owned by {{jit:var:NAME}}",
            }),
        };
        assert!(matches!(
            substitute_contribution(2, &free_form, &resolved).unwrap(),
            Contribution::MapEntry { value, .. }
                if value["description"] == "owned by value"
        ));
    }
}
