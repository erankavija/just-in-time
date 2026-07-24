//! ApplyProfile materialization: compose a profile package's canonical claims
//! (declaration-overlay registry edits, exact assets, and managed regions) plus the
//! configured projections those declarations imply into the exact set of
//! profile-owned targets.
//!
//! `repository_state` owns this composition; the profile package produces the
//! neutral [`ProfileClaims`] and the command captures the base image and applies the
//! resulting delta. This module imports no profile, storage, or command code — the
//! profile-package parsing (`profile::package`) and the typed `ApplyProfile`
//! materialization request sit on either side of it.

use std::collections::{BTreeMap, BTreeSet};

use super::materialize::{
    assemble_config, compose_configured_projections, serialized_default_ruleset,
};
use super::{
    apply_overlay, compose_managed_documents, declarations_from_image, FileMode,
    ManagedDocumentClaim, ProducerError, ProfileRegistryParseError, RegionPlacement,
    RepositoryAction, RepositoryEntry, RepositoryImage, RepositoryStateError, TargetClaim,
    VirtualPath,
};
use crate::config::{ProjectionKinds, ProjectionMode, ProjectionStyle};
use crate::domain::ProfileOrigin;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

/// A semantic contribution to one JIT registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Contribution {
    MapEntry {
        target: MapEntryTarget,
        identity: String,
        value: JsonValue,
    },
    SetString {
        target: SetStringTarget,
        value: String,
    },
    KeyedArray {
        target: KeyedArrayTarget,
        identity: String,
        value: JsonValue,
    },
    Projection {
        name: String,
        value: CompleteProjectionConfig,
    },
}

impl Contribution {
    pub fn registry_path(&self) -> &'static str {
        match self {
            Self::MapEntry { .. } | Self::SetString { .. } | Self::Projection { .. } => {
                ".jit/config.toml"
            }
            Self::KeyedArray { target, .. } => target.registry_path(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MapEntryTarget {
    TypeHierarchyTypes,
    LabelAssociations,
    Namespaces,
    ItemKinds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SetStringTarget {
    StrategicTypes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum KeyedArrayTarget {
    Gates,
    Rules,
    Templates,
}

impl KeyedArrayTarget {
    pub(crate) fn registry_path(self) -> &'static str {
        match self {
            Self::Gates => ".jit/gates.toml",
            Self::Rules => ".jit/rules.toml",
            Self::Templates => ".jit/templates.toml",
        }
    }

    pub(crate) fn identity_field(self) -> &'static str {
        match self {
            Self::Gates => "key",
            Self::Rules | Self::Templates => "name",
        }
    }

    pub(crate) fn array_name(self) -> &'static str {
        match self {
            Self::Gates => "gates",
            Self::Rules => "rules",
            Self::Templates => "template",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompleteProjectionConfig {
    pub kind: ProjectionKinds,
    pub mode: ProjectionMode,
    pub target: String,
    pub style: ProjectionStyle,
}

#[derive(Clone)]
pub struct ProfileAssetClaim {
    pub claim: TargetClaim,
    pub bytes: Vec<u8>,
    pub mode: FileMode,
    pub replace_owned: bool,
}

#[derive(Clone)]
pub struct ProfileRegionClaim {
    pub claim: TargetClaim,
    pub region_id: String,
    pub content: Vec<u8>,
}

/// A package asset cannot replace an authored occupant it does not own.
#[derive(Debug, thiserror::Error)]
#[error("profile asset target {path:?} contains differing bytes")]
pub struct ProfileTargetConflictError {
    /// Conflicting canonical repository path.
    pub path: VirtualPath,
}

/// A profile package's contribution to a repository, in canonical repository-state
/// vocabulary. Produced by `profile::package` from the immutable manifest and
/// consumed only by [`compose_profile_targets`].
#[derive(Clone)]
pub struct ProfileClaims {
    pub contributions: Vec<Contribution>,
    pub assets: Vec<ProfileAssetClaim>,
    pub regions: Vec<ProfileRegionClaim>,
}

impl ProfileClaims {
    pub(crate) fn target_paths(&self) -> Result<Vec<VirtualPath>, super::RepositoryLayoutError> {
        let mut paths = self
            .contributions
            .iter()
            .map(Contribution::registry_target)
            .collect::<Result<Vec<_>, _>>()?;
        paths.extend(self.assets.iter().map(|asset| asset.claim.target().clone()));
        paths.extend(
            self.regions
                .iter()
                .map(|region| region.claim.target().clone()),
        );
        Ok(paths)
    }
}

impl Contribution {
    fn registry_target(&self) -> Result<VirtualPath, super::RepositoryLayoutError> {
        let relative = match self {
            Self::MapEntry { .. } | Self::SetString { .. } | Self::Projection { .. } => {
                "config.toml"
            }
            Self::KeyedArray { target, .. } => match target {
                KeyedArrayTarget::Gates => "gates.toml",
                KeyedArrayTarget::Rules => "rules.toml",
                KeyedArrayTarget::Templates => "templates.toml",
            },
        };
        VirtualPath::data(relative)
    }
}

/// Canonical repository-local provenance for one installed profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppliedProfileRecord {
    /// Stable profile identifier.
    pub id: String,
    /// Installed package version.
    pub version: String,
    /// Package discovery source.
    pub origin: ProfileOrigin,
    /// Digest of the complete package manifest and content.
    pub package_hash: String,
    /// Digests of every installed package target, keyed by repository-relative path.
    pub target_hashes: BTreeMap<String, String>,
}

impl AppliedProfileRecord {
    /// Construct canonical installed-profile provenance from typed package metadata.
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        origin: ProfileOrigin,
        package_hash: impl Into<String>,
        target_hashes: BTreeMap<String, String>,
    ) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            origin,
            package_hash: package_hash.into(),
            target_hashes,
        }
    }

    /// Encode the stable installed-record image.
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

/// Neutral profile package input consumed by the one materialization dispatcher.
#[derive(Clone)]
pub struct ProfileApplicationInput {
    pub id: String,
    pub version: String,
    pub package_hash: String,
    pub target_hashes: BTreeMap<String, String>,
    pub origin: ProfileOrigin,
    pub claims: ProfileClaims,
    pub record_path: VirtualPath,
}

impl ProfileApplicationInput {
    pub(crate) fn record(&self) -> AppliedProfileRecord {
        AppliedProfileRecord::new(
            self.id.clone(),
            self.version.clone(),
            self.origin,
            self.package_hash.clone(),
            self.target_hashes.clone(),
        )
    }
}

/// Enumerate paths implied by a profile's proposed declarations before rendering.
pub(crate) fn profile_capture_closure(
    base: &RepositoryImage,
    claims: &ProfileClaims,
) -> Result<Vec<VirtualPath>, RepositoryStateError> {
    let registries = merge_semantic_contributions(base, &claims.contributions)?;
    let proposed = apply_overlay(
        base,
        registries
            .into_iter()
            .map(|(path, (bytes, _))| (path, Some(bytes))),
    )?;
    let config = super::materialize::assemble_config(&proposed)?;
    let rules = proposed
        .file_bytes(&VirtualPath::data("rules.toml")?)
        .map_err(ProducerError::from)?
        .map(|bytes| String::from_utf8(bytes.to_vec()))
        .transpose()
        .map_err(ProducerError::from)?;
    Ok(super::materialize::render_capture_closure(
        proposed.layout(),
        &config,
        &[],
        rules.as_deref(),
    )?)
}

/// Derive every profile-owned target's exact final bytes and mode from a captured
/// base image.
///
/// The declaration-overlay registries and exact assets land verbatim; the managed
/// regions compose over the captured base through the one managed-document engine;
/// and every configured projection the merged declarations imply is re-rendered
/// over the resulting proposed image and folded back into the target it already
/// produced. A projection whose target is not itself a profile-owned
/// asset/region/registry leaves that target untouched (matching the
/// profile-application contract: a projection only rewrites bytes the profile
/// otherwise publishes).
///
/// The result equals the byte-for-byte final image of every profile-owned target
/// regardless of whether it changed from the captured occupant; the caller
/// (the typed `ApplyProfile` materialization request) decides which targets to
/// write.
pub(super) fn compose_profile_targets(
    base: &RepositoryImage,
    claims: ProfileClaims,
) -> Result<BTreeMap<VirtualPath, (Vec<u8>, FileMode)>, RepositoryStateError> {
    let mut targets = merge_semantic_contributions(base, &claims.contributions)?;
    let projection_targets = configured_projection_targets(base, &targets)?;
    for asset in claims.assets {
        let path = asset.claim.target().clone();
        if !asset.replace_owned && !projection_targets.contains(&path) {
            if let RepositoryEntry::File { bytes, .. } =
                base.entry(&path).map_err(ProducerError::from)?
            {
                if bytes != &asset.bytes {
                    return Err(RepositoryStateError::ProfileTargetConflict(
                        ProfileTargetConflictError { path },
                    ));
                }
            }
        }
        targets.insert(path, (asset.bytes, asset.mode));
    }
    // Profile-owned regions compose over the captured base; a region target keeps
    // its captured file mode (a fresh target is Regular), matching the profile's
    // region-target mode contract.
    let regions = claims.regions.into_iter().map(|region| {
        let path = region.claim.target().clone();
        let claim = ManagedDocumentClaim::Region {
            owner: region.claim.owner().to_string(),
            region_id: region.region_id.clone(),
            begin: format!("<!-- jit:{}:begin -->", region.region_id).into_bytes(),
            end: format!("<!-- jit:{}:end -->", region.region_id).into_bytes(),
            content: region.content,
            placement: RegionPlacement::AppendIfAbsent,
        };
        (path, claim)
    });
    for (path, bytes) in compose_managed_documents(base, regions)? {
        let mode = existing_file_mode(base, &path)?;
        targets.insert(path, (bytes, mode));
    }

    // Build the proposed image (base overlaid with every target computed so far) and
    // re-render every configured projection from the PROPOSED declarations. The
    // occupant `compose_configured_projections` compares against IS the target
    // computed above, so it yields the rendered bytes whether or not it emits an
    // action: an emitted write updates the target to the rendered bytes, and a
    // no-op means the target already holds them. Either way the target ends at the
    // exact rendered projection bytes.
    let overlay = targets
        .iter()
        .map(|(path, (bytes, _))| (path.clone(), Some(bytes.clone())));
    let proposed = apply_overlay(base, overlay)?;
    let config = assemble_config(&proposed)?;
    let schema_overlay = serialized_default_ruleset(&config)
        .schema_files
        .into_iter()
        .map(|schema| {
            Ok((
                VirtualPath::data(format!("schemas/{}", schema.name))?,
                Some(schema.content.into_bytes()),
            ))
        })
        .collect::<Result<Vec<_>, super::RepositoryLayoutError>>()?;
    let proposed = apply_overlay(&proposed, schema_overlay)?;
    let declarations =
        declarations_from_image(&proposed).map_err(ProducerError::DeclarationAssembly)?;
    let projection_actions = compose_configured_projections(
        &proposed,
        declarations.config(),
        &declarations.borrowed(),
        None,
    )?
    .actions;
    for action in projection_actions {
        if let RepositoryAction::WriteFile { path, bytes, .. } = action {
            if let Some(target) = targets.get_mut(&path) {
                target.0 = bytes;
            }
        }
    }
    Ok(targets)
}

fn merge_semantic_contributions(
    base: &RepositoryImage,
    contributions: &[Contribution],
) -> Result<BTreeMap<VirtualPath, (Vec<u8>, FileMode)>, RepositoryStateError> {
    let mut documents = BTreeMap::<String, MergeDocument>::new();
    for contribution in contributions {
        let target = contribution.registry_path().to_string();
        if !documents.contains_key(&target) {
            let path = base.layout().classify_repository_relative(&target)?;
            let existing = match base.entry(&path).map_err(ProducerError::from)? {
                RepositoryEntry::File { bytes, mode, .. } => Some((bytes.clone(), *mode)),
                RepositoryEntry::Absent => None,
                _ => return Err(ProducerError::ProfileRegistryNotFile { target }.into()),
            };
            documents.insert(target.clone(), MergeDocument::load(&target, existing)?);
        }
        let document = documents
            .get_mut(&target)
            .expect("profile registry document was inserted");
        merge_contribution(&target, &mut document.document, contribution)?;
    }
    documents
        .into_iter()
        .map(|(path, document)| {
            Ok((
                base.layout().classify_repository_relative(&path)?,
                (document.document.to_string().into_bytes(), document.mode),
            ))
        })
        .collect()
}

fn configured_projection_targets(
    base: &RepositoryImage,
    registries: &BTreeMap<VirtualPath, (Vec<u8>, FileMode)>,
) -> Result<BTreeSet<VirtualPath>, RepositoryStateError> {
    let config_path = VirtualPath::data("config.toml")?;
    let bytes = match registries.get(&config_path) {
        Some((bytes, _)) => Some(bytes.as_slice()),
        None => base.file_bytes(&config_path).map_err(ProducerError::from)?,
    };
    let Some(bytes) = bytes else {
        return Ok(BTreeSet::new());
    };
    let declarations =
        crate::declarations::parse_configuration(bytes).map_err(ProducerError::from)?;
    declarations
        .projections
        .values()
        .filter_map(|projection| projection.target.as_deref())
        .map(|target| {
            base.layout()
                .classify_repository_relative(target)
                .map_err(Into::into)
        })
        .collect()
}

struct MergeDocument {
    document: DocumentMut,
    mode: FileMode,
}

impl MergeDocument {
    fn load(
        target: &str,
        existing: Option<(Vec<u8>, FileMode)>,
    ) -> Result<Self, RepositoryStateError> {
        let (bytes, mode) = existing.unwrap_or_else(|| (Vec::new(), FileMode::Regular));
        let text = std::str::from_utf8(&bytes)
            .map_err(|error| profile_registry_error(target, error.into()))?;
        let document = text
            .parse::<DocumentMut>()
            .map_err(|error| profile_registry_error(target, error.into()))?;
        Ok(Self { document, mode })
    }
}

fn profile_registry_error(target: &str, source: ProfileRegistryParseError) -> RepositoryStateError {
    ProducerError::ProfileRegistryParse {
        target: target.to_string(),
        source: Box::new(source),
    }
    .into()
}

fn merge_contribution(
    registry: &str,
    document: &mut DocumentMut,
    contribution: &Contribution,
) -> Result<(), RepositoryStateError> {
    let semantic = semantic_document(registry, document)?;
    match contribution {
        Contribution::MapEntry {
            target,
            identity,
            value,
        } => merge_map_entry(registry, document, &semantic, *target, identity, value),
        Contribution::SetString { target, value } => {
            merge_set_string(registry, document, &semantic, *target, value)
        }
        Contribution::KeyedArray {
            target,
            identity,
            value,
        } => merge_keyed_array(registry, document, *target, identity, value),
        Contribution::Projection { name, value } => {
            merge_projection(registry, document, &semantic, name, value)
        }
    }
}

fn merge_map_entry(
    registry: &str,
    document: &mut DocumentMut,
    semantic: &JsonValue,
    target: MapEntryTarget,
    identity: &str,
    candidate: &JsonValue,
) -> Result<(), RepositoryStateError> {
    let existing = match target {
        MapEntryTarget::TypeHierarchyTypes => {
            semantic_map_entry(semantic, &["type_hierarchy", "types"], identity)
        }
        MapEntryTarget::LabelAssociations => semantic_map_entry(
            semantic,
            &["type_hierarchy", "label_associations"],
            identity,
        ),
        MapEntryTarget::Namespaces => semantic_map_entry(semantic, &["namespaces"], identity),
        MapEntryTarget::ItemKinds => semantic_map_entry(semantic, &["item_kinds"], identity),
    };
    if let Some(existing) = existing {
        return equal_or_conflict(registry, identity, existing, candidate);
    }
    match target {
        MapEntryTarget::TypeHierarchyTypes => {
            ensure_inline_table(
                ensure_table(document.as_table_mut(), "type_hierarchy", registry)?,
                "types",
                registry,
            )?
            .insert(identity, json_to_edit_value(candidate, registry)?);
        }
        MapEntryTarget::LabelAssociations => {
            ensure_table(
                ensure_table(document.as_table_mut(), "type_hierarchy", registry)?,
                "label_associations",
                registry,
            )?
            .insert(
                identity,
                Item::Value(json_to_edit_value(candidate, registry)?),
            );
        }
        MapEntryTarget::Namespaces | MapEntryTarget::ItemKinds => {
            let root = if target == MapEntryTarget::Namespaces {
                "namespaces"
            } else {
                "item_kinds"
            };
            ensure_table(document.as_table_mut(), root, registry)?.insert(
                identity,
                Item::Table(json_object_to_table(candidate, registry)?),
            );
        }
    }
    Ok(())
}

fn semantic_map_entry<'a>(
    semantic: &'a JsonValue,
    path: &[&str],
    identity: &str,
) -> Option<&'a JsonValue> {
    path.iter()
        .try_fold(semantic, |value, key| value.get(*key))?
        .as_object()?
        .get(identity)
}

fn merge_set_string(
    registry: &str,
    document: &mut DocumentMut,
    semantic: &JsonValue,
    target: SetStringTarget,
    candidate: &str,
) -> Result<(), RepositoryStateError> {
    let values = match target {
        SetStringTarget::StrategicTypes => semantic.pointer("/type_hierarchy/strategic_types"),
    };
    if let Some(values) = values {
        let values = values.as_array().ok_or_else(|| {
            profile_registry_error(registry, ProfileRegistryParseError::SetTargetNotArray)
        })?;
        if values.iter().any(|value| !value.is_string()) {
            return Err(profile_registry_error(
                registry,
                ProfileRegistryParseError::SetTargetNonStringMember,
            ));
        }
        if values.iter().any(|value| value.as_str() == Some(candidate)) {
            return Ok(());
        }
    }
    ensure_array(
        ensure_table(document.as_table_mut(), "type_hierarchy", registry)?,
        "strategic_types",
        registry,
    )?
    .push(candidate);
    Ok(())
}

fn merge_keyed_array(
    registry: &str,
    document: &mut DocumentMut,
    target: KeyedArrayTarget,
    identity: &str,
    candidate: &JsonValue,
) -> Result<(), RepositoryStateError> {
    let field = target.identity_field();
    let (array, preserved_comment) =
        ensure_array_of_tables(document.as_table_mut(), target.array_name(), registry)?;
    let mut identities = BTreeSet::new();
    let mut existing = None;
    for table in array.iter() {
        let Some(actual) = table.get(field).and_then(Item::as_str) else {
            return Err(profile_registry_error(
                registry,
                ProfileRegistryParseError::MissingIdentity {
                    field: field.to_string(),
                },
            ));
        };
        if !identities.insert(actual.to_string()) {
            return Err(profile_registry_error(
                registry,
                ProfileRegistryParseError::DuplicateIdentity {
                    field: field.to_string(),
                },
            ));
        }
        if actual == identity {
            existing = Some(table_to_json(table, registry)?);
        }
    }
    if let Some(existing) = existing {
        return equal_or_conflict(registry, identity, &existing, candidate);
    }
    let mut table = json_object_to_table(candidate, registry)?;
    if let Some(comment) = preserved_comment {
        table.decor_mut().set_prefix(comment);
    }
    array.push(table);
    Ok(())
}

fn merge_projection(
    registry: &str,
    document: &mut DocumentMut,
    semantic: &JsonValue,
    name: &str,
    candidate: &CompleteProjectionConfig,
) -> Result<(), RepositoryStateError> {
    let candidate = serde_json::to_value(candidate).expect("projection config serializes");
    if let Some(existing) = semantic
        .get("projection")
        .and_then(|projections| projections.get(name))
    {
        return equal_or_conflict(registry, name, existing, &candidate);
    }
    ensure_table(document.as_table_mut(), "projection", registry)?.insert(
        name,
        Item::Table(json_object_to_table(&candidate, registry)?),
    );
    Ok(())
}

fn semantic_document(
    registry: &str,
    document: &DocumentMut,
) -> Result<JsonValue, RepositoryStateError> {
    toml_edit::de::from_str(&document.to_string())
        .map_err(|error| profile_registry_error(registry, error.into()))
}

fn equal_or_conflict(
    registry: &str,
    identity: &str,
    existing: &JsonValue,
    candidate: &JsonValue,
) -> Result<(), RepositoryStateError> {
    if existing == candidate {
        Ok(())
    } else {
        Err(ProducerError::ProfileContributionConflict {
            identity: identity.to_string(),
            registry: registry.to_string(),
        }
        .into())
    }
}

fn ensure_table<'a>(
    parent: &'a mut Table,
    key: &str,
    registry: &str,
) -> Result<&'a mut Table, RepositoryStateError> {
    if !parent.contains_key(key) {
        parent.insert(key, Item::Table(Table::new()));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| {
            profile_registry_error(
                registry,
                ProfileRegistryParseError::NotTable {
                    key: key.to_string(),
                },
            )
        })
}

fn ensure_inline_table<'a>(
    parent: &'a mut Table,
    key: &str,
    registry: &str,
) -> Result<&'a mut InlineTable, RepositoryStateError> {
    if !parent.contains_key(key) {
        parent.insert(key, Item::Value(Value::InlineTable(InlineTable::new())));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_inline_table_mut)
        .ok_or_else(|| {
            profile_registry_error(
                registry,
                ProfileRegistryParseError::NotInlineTable {
                    key: key.to_string(),
                },
            )
        })
}

fn ensure_array<'a>(
    parent: &'a mut Table,
    key: &str,
    registry: &str,
) -> Result<&'a mut Array, RepositoryStateError> {
    if !parent.contains_key(key) {
        parent.insert(key, Item::Value(Value::Array(Array::new())));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_array_mut)
        .ok_or_else(|| {
            profile_registry_error(
                registry,
                ProfileRegistryParseError::NotArray {
                    key: key.to_string(),
                },
            )
        })
}

fn ensure_array_of_tables<'a>(
    parent: &'a mut Table,
    key: &str,
    registry: &str,
) -> Result<(&'a mut ArrayOfTables, Option<String>), RepositoryStateError> {
    let preserved_comment = empty_array_comments(parent, key);
    let convert = !parent.contains_key(key)
        || parent
            .get(key)
            .and_then(Item::as_array)
            .is_some_and(Array::is_empty);
    if convert {
        parent.insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let array = parent
        .get_mut(key)
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| {
            profile_registry_error(
                registry,
                ProfileRegistryParseError::NotArrayOfTables {
                    key: key.to_string(),
                },
            )
        })?;
    Ok((array, preserved_comment))
}

fn empty_array_comments(parent: &Table, key: &str) -> Option<String> {
    let array = parent.get(key)?.as_array()?;
    if !array.is_empty() {
        return None;
    }
    let decor = parent.key(key)?.leaf_decor();
    let fragments = [
        decor.prefix(),
        decor.suffix(),
        array.decor().prefix(),
        array.decor().suffix(),
    ]
    .into_iter()
    .filter_map(comment_fragment)
    .collect::<String>();
    (!fragments.is_empty()).then_some(fragments)
}

fn comment_fragment(raw: Option<&toml_edit::RawString>) -> Option<String> {
    let text = raw?.as_str()?;
    let comment = text.get(text.find('#')?..)?.trim_end_matches(['\r', '\n']);
    Some(format!("{comment}\n"))
}

fn json_object_to_table(value: &JsonValue, registry: &str) -> Result<Table, RepositoryStateError> {
    value
        .as_object()
        .ok_or_else(|| {
            profile_registry_error(registry, ProfileRegistryParseError::ContributionNotTable)
        })?
        .iter()
        .try_fold(Table::new(), |mut table, (key, value)| {
            table.insert(key, json_to_item(value, registry)?);
            Ok(table)
        })
}

fn json_to_item(value: &JsonValue, registry: &str) -> Result<Item, RepositoryStateError> {
    match value {
        JsonValue::Object(_) => Ok(Item::Value(Value::InlineTable(json_to_inline_table(
            value, registry,
        )?))),
        _ => Ok(Item::Value(json_to_edit_value(value, registry)?)),
    }
}

fn json_to_edit_value(value: &JsonValue, registry: &str) -> Result<Value, RepositoryStateError> {
    match value {
        JsonValue::Null => Err(profile_registry_error(
            registry,
            ProfileRegistryParseError::NullValue,
        )),
        JsonValue::Bool(value) => Ok(Value::from(*value)),
        JsonValue::Number(value) => value
            .as_i64()
            .map(Value::from)
            .or_else(|| value.as_f64().map(Value::from))
            .ok_or_else(|| {
                profile_registry_error(registry, ProfileRegistryParseError::UnsupportedNumericValue)
            }),
        JsonValue::String(value) => Ok(Value::from(value.clone())),
        JsonValue::Array(values) => values
            .iter()
            .try_fold(Array::new(), |mut array, value| {
                array.push(json_to_edit_value(value, registry)?);
                Ok(array)
            })
            .map(Value::Array),
        JsonValue::Object(_) => json_to_inline_table(value, registry).map(Value::InlineTable),
    }
}

fn json_to_inline_table(
    value: &JsonValue,
    registry: &str,
) -> Result<InlineTable, RepositoryStateError> {
    value
        .as_object()
        .ok_or_else(|| profile_registry_error(registry, ProfileRegistryParseError::ValueNotTable))?
        .iter()
        .try_fold(InlineTable::new(), |mut table, (key, value)| {
            table.insert(key, json_to_edit_value(value, registry)?);
            Ok(table)
        })
}

fn table_to_json(table: &Table, registry: &str) -> Result<JsonValue, RepositoryStateError> {
    toml_edit::de::from_str(&table.to_string())
        .map_err(|error| profile_registry_error(registry, error.into()))
}

/// The captured file mode at `path`, or `Regular` for an absent or non-file entry.
fn existing_file_mode(
    base: &RepositoryImage,
    path: &VirtualPath,
) -> Result<FileMode, RepositoryStateError> {
    match base.entry(path).map_err(ProducerError::from)? {
        RepositoryEntry::File { mode, .. } => Ok(*mode),
        _ => Ok(FileMode::Regular),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equal_or_conflict_preserves_raw_profile_identifiers() {
        let error = equal_or_conflict(
            ".jit/config.toml",
            "task",
            &serde_json::json!({"level": 3}),
            &serde_json::json!({"level": 4}),
        )
        .expect_err("different contribution values must conflict");

        assert!(matches!(
            error,
            RepositoryStateError::Producer(
                ProducerError::ProfileContributionConflict { identity, registry }
            ) if identity == "task" && registry == ".jit/config.toml"
        ));
    }

    #[test]
    fn test_profile_registry_error_preserves_raw_target_and_typed_source() {
        let error = profile_registry_error(
            ".jit/config.toml",
            ProfileRegistryParseError::NotArray {
                key: "strategic_types".to_string(),
            },
        );

        assert!(matches!(
            error,
            RepositoryStateError::Producer(ProducerError::ProfileRegistryParse {
                target,
                source,
            }) if target == ".jit/config.toml"
                && matches!(*source, ProfileRegistryParseError::NotArray { ref key }
                    if key == "strategic_types")
        ));
    }
}
