//! Build a profile package's canonical [`ProfileClaims`] from the immutable
//! manifest and a captured base image.
//!
//! This is the profile side of the ApplyProfile pipeline: it parses the package's
//! semantic contributions into merged registry bytes (declaration-overlay edits),
//! its one-to-one assets into exact target bytes, and its managed-region
//! declarations into [`ManagedDocumentClaim`] values, then hands the neutral
//! [`ProfileClaims`] to [`repository_state::derive_profile_materializations`]. The
//! merge preserves authored comments and ordering through `toml_edit`, so a
//! re-applied contribution is a byte-level no-op and a divergent one conflicts.
//!
//! Profile parses; `repository_state` composes. This module reads registry and
//! asset preimages from the captured [`RepositoryImage`] (never the live
//! filesystem) and imports no storage, command, or validation code.

use super::{
    CompleteProjectionConfig, Contribution, EmbeddedProfilePackage, KeyedArrayTarget,
    MapEntryTarget, SetStringTarget,
};
use crate::repository_state::{
    FileMode, ManagedDocumentClaim, ProfileClaims, RegionPlacement, RepositoryImage, VirtualPath,
};
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

// V1 has no variable surface. Reject the reserved profile interpolation namespace
// while leaving ordinary shell/template syntax as opaque asset bytes.
const RESERVED_INTERPOLATION_PREFIX: &[u8] = b"{{jit:";

/// Failure building a profile's canonical claims from a captured base.
#[derive(Debug, thiserror::Error)]
pub enum ProfileClaimError {
    /// Package target layout contains an ancestor/descendant collision.
    #[error("profile targets '{first}' and '{second}' overlap")]
    OverlappingTargets {
        /// First colliding target.
        first: String,
        /// Second colliding target.
        second: String,
    },
    /// An ordinary package asset differs from the occupied target.
    #[error("profile asset target '{target}' contains differing bytes")]
    AssetConflict {
        /// The conflicting repository-relative target.
        target: String,
    },
    /// Existing semantic identity has a differing value.
    #[error("profile contribution '{identity}' conflicts in '{target}'")]
    ContributionConflict {
        /// The registry receiving the contribution.
        target: String,
        /// The conflicting semantic identity.
        identity: String,
    },
    /// Existing keyed registry entry lacks its required stable identity.
    #[error("registry '{target}' contains an unknown or duplicate '{field}' identity")]
    UnknownContributionIdentity {
        /// The registry receiving the contribution.
        target: String,
        /// The identity field expected on each row.
        field: String,
    },
    /// Registry bytes cannot be interpreted without rewriting user content.
    #[error("invalid profile registry '{target}': {message}")]
    InvalidRegistry {
        /// The registry that could not be parsed.
        target: String,
        /// Specific parse failure.
        message: String,
    },
    /// V1 packages cannot carry reserved interpolation tokens.
    #[error("profile source '{location}' contains unsupported interpolation token")]
    InvalidInterpolation {
        /// The source or contribution index that carried the token.
        location: String,
    },
    /// A declared embedded source disappeared after package validation.
    #[error("declared embedded source '{0}' is unavailable")]
    MissingSource(String),
    /// A target path could not be canonicalized.
    #[error("profile target path is not canonical: {0}")]
    Layout(#[from] crate::repository_state::RepositoryLayoutError),
    /// Reading a captured registry or asset preimage failed.
    #[error("failed to read captured profile preimage: {0}")]
    Capture(#[from] crate::repository_state::CaptureError),
}

/// Build the profile's canonical claims from the immutable package and a captured
/// base image.
///
/// Validates the reserved-interpolation ban and target-overlap layout, merges every
/// semantic contribution into its registry (preserving authored bytes), reads each
/// asset's exact bytes (rejecting a differing occupant that is not a configured
/// projection target), and turns each region declaration into an append-if-absent
/// [`ManagedDocumentClaim`].
pub fn build_profile_claims(
    package: &EmbeddedProfilePackage<'_>,
    base: &RepositoryImage,
) -> Result<ProfileClaims, ProfileClaimError> {
    validate_interpolation(package)?;
    validate_target_overlaps(package)?;

    let registries = merge_semantic_contributions(package, base)?;
    let projection_targets = configured_projection_targets(base, &registries)?;
    let assets = asset_claims(package, base, &projection_targets)?;
    let regions = region_claims(package)?;

    Ok(ProfileClaims {
        registries,
        assets,
        regions,
    })
}

/// Map a repository-relative path to its canonical virtual path (`.jit/...` is
/// `Data`, everything else `Worktree`).
fn repo_rel(path: &str) -> Result<VirtualPath, ProfileClaimError> {
    match path.strip_prefix(".jit/") {
        Some(rest) => Ok(VirtualPath::data(rest)?),
        None => Ok(VirtualPath::worktree(path)?),
    }
}

/// Read a repository-relative file's bytes and mode from the captured base.
fn base_file(
    base: &RepositoryImage,
    repo_relative: &str,
) -> Result<Option<(Vec<u8>, FileMode)>, ProfileClaimError> {
    use crate::repository_state::RepositoryEntry;
    let vpath = repo_rel(repo_relative)?;
    match base.entry(&vpath)? {
        RepositoryEntry::File { bytes, mode, .. } => Ok(Some((bytes.clone(), *mode))),
        _ => Ok(None),
    }
}

/// The exact one-to-one asset claims, rejecting a differing occupant that is not a
/// configured projection target (a projection target's bytes are re-derived from
/// the merged registries, so a difference there is expected, not a conflict).
fn asset_claims(
    package: &EmbeddedProfilePackage<'_>,
    base: &RepositoryImage,
    projection_targets: &BTreeSet<String>,
) -> Result<BTreeMap<VirtualPath, (Vec<u8>, FileMode)>, ProfileClaimError> {
    package
        .manifest()
        .assets
        .iter()
        .map(|asset| {
            let bytes = package
                .source_bytes(&asset.source)
                .ok_or_else(|| ProfileClaimError::MissingSource(asset.source.clone()))?
                .to_vec();
            let mode = if asset.executable {
                FileMode::Executable
            } else {
                FileMode::Regular
            };
            if !projection_targets.contains(&asset.target) {
                if let Some((existing, _)) = base_file(base, &asset.target)? {
                    if existing != bytes {
                        return Err(ProfileClaimError::AssetConflict {
                            target: asset.target.clone(),
                        });
                    }
                }
            }
            Ok((repo_rel(&asset.target)?, (bytes, mode)))
        })
        .collect()
}

/// Turn each region declaration into an append-if-absent managed-document claim
/// with the canonical `<!-- jit:{id}:begin/end -->` markers and its source content.
fn region_claims(
    package: &EmbeddedProfilePackage<'_>,
) -> Result<Vec<(VirtualPath, ManagedDocumentClaim)>, ProfileClaimError> {
    package
        .manifest()
        .regions
        .iter()
        .map(|region| {
            let content = package
                .source_bytes(&region.source)
                .ok_or_else(|| ProfileClaimError::MissingSource(region.source.clone()))?
                .to_vec();
            let claim = ManagedDocumentClaim::Region {
                owner: format!("profile-region:{}", region.region_id),
                region_id: region.region_id.clone(),
                begin: format!("<!-- jit:{}:begin -->", region.region_id).into_bytes(),
                end: format!("<!-- jit:{}:end -->", region.region_id).into_bytes(),
                content,
                placement: RegionPlacement::AppendIfAbsent,
            };
            Ok((repo_rel(&region.target)?, claim))
        })
        .collect()
}

/// The configured-projection target set implied by the merged (or captured)
/// `config.toml`, used to exempt those targets from the asset-conflict check.
fn configured_projection_targets(
    base: &RepositoryImage,
    registries: &BTreeMap<VirtualPath, (Vec<u8>, FileMode)>,
) -> Result<BTreeSet<String>, ProfileClaimError> {
    let config_path = repo_rel(".jit/config.toml")?;
    let config_bytes = match registries.get(&config_path) {
        Some((bytes, _)) => Some(bytes.clone()),
        None => base_file(base, ".jit/config.toml")?.map(|(bytes, _)| bytes),
    };
    let Some(bytes) = config_bytes else {
        return Ok(BTreeSet::new());
    };
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| invalid_registry(".jit/config.toml", error.to_string()))?;
    let config: crate::config::JitConfig = toml::from_str(text)
        .map_err(|error| invalid_registry(".jit/config.toml", error.to_string()))?;
    Ok(config
        .projection
        .unwrap_or_default()
        .values()
        .filter_map(|projection| projection.target.clone())
        .collect())
}

fn validate_interpolation(package: &EmbeddedProfilePackage<'_>) -> Result<(), ProfileClaimError> {
    for declaration in package
        .manifest()
        .assets
        .iter()
        .map(|asset| asset.source.as_str())
        .chain(
            package
                .manifest()
                .regions
                .iter()
                .map(|region| region.source.as_str()),
        )
    {
        if package
            .source_bytes(declaration)
            .is_some_and(|bytes| contains_bytes(bytes, RESERVED_INTERPOLATION_PREFIX))
        {
            return Err(ProfileClaimError::InvalidInterpolation {
                location: declaration.to_string(),
            });
        }
    }
    for (index, contribution) in package.manifest().contributions.iter().enumerate() {
        let value = serde_json::to_value(contribution).expect("manifest values serialize");
        if json_contains_interpolation(&value) {
            return Err(ProfileClaimError::InvalidInterpolation {
                location: format!("contribution[{index}]"),
            });
        }
    }
    Ok(())
}

fn validate_target_overlaps(package: &EmbeddedProfilePackage<'_>) -> Result<(), ProfileClaimError> {
    let semantic_targets = package
        .manifest()
        .contributions
        .iter()
        .map(|contribution| contribution.registry_path().to_string())
        .collect::<BTreeSet<_>>();
    let content_targets = package
        .manifest()
        .assets
        .iter()
        .map(|asset| asset.target.clone())
        .chain(
            package
                .manifest()
                .regions
                .iter()
                .map(|region| region.target.clone()),
        )
        .collect::<BTreeSet<_>>();

    if let Some(target) = semantic_targets.intersection(&content_targets).next() {
        return Err(ProfileClaimError::OverlappingTargets {
            first: target.clone(),
            second: target.clone(),
        });
    }
    let mut targets = semantic_targets
        .iter()
        .chain(content_targets.iter())
        .collect::<Vec<_>>();
    targets.sort();
    for pair in targets.windows(2) {
        let first = Path::new(pair[0]);
        let second = Path::new(pair[1]);
        if second.starts_with(first) && second != first {
            return Err(ProfileClaimError::OverlappingTargets {
                first: (*pair[0]).clone(),
                second: (*pair[1]).clone(),
            });
        }
    }
    Ok(())
}

/// Merge every semantic contribution into its registry document, returning the
/// final bytes and mode for each registry that receives at least one contribution.
/// An unchanged registry keeps its original bytes so re-application is a no-op.
fn merge_semantic_contributions(
    package: &EmbeddedProfilePackage<'_>,
    base: &RepositoryImage,
) -> Result<BTreeMap<VirtualPath, (Vec<u8>, FileMode)>, ProfileClaimError> {
    let mut documents = BTreeMap::<String, MergeDocument>::new();
    for contribution in &package.manifest().contributions {
        let target = contribution.registry_path().to_string();
        if !documents.contains_key(&target) {
            let existing = base_file(base, &target)?;
            documents.insert(target.clone(), MergeDocument::load(&target, existing)?);
        }
        let document = documents
            .get_mut(&target)
            .expect("registry document was inserted");
        merge_contribution(&target, &mut document.document, contribution)?;
        document.changed = document.document.to_string().as_bytes() != document.original.as_slice();
    }

    documents
        .into_iter()
        .map(|(path, document)| {
            let bytes = if document.changed {
                document.document.to_string().into_bytes()
            } else {
                document.original
            };
            Ok((repo_rel(&path)?, (bytes, document.mode)))
        })
        .collect()
}

struct MergeDocument {
    original: Vec<u8>,
    document: DocumentMut,
    mode: FileMode,
    changed: bool,
}

impl MergeDocument {
    fn load(
        target: &str,
        existing: Option<(Vec<u8>, FileMode)>,
    ) -> Result<Self, ProfileClaimError> {
        let (original, mode) = existing.unwrap_or_else(|| (Vec::new(), FileMode::Regular));
        let document = std::str::from_utf8(&original)
            .map_err(|error| error.to_string())
            .and_then(|text| {
                text.parse::<DocumentMut>()
                    .map_err(|error| error.to_string())
            })
            .map_err(|error| invalid_registry(target, error))?;
        Ok(Self {
            original,
            document,
            mode,
            changed: false,
        })
    }
}

fn merge_contribution(
    registry: &str,
    document: &mut DocumentMut,
    contribution: &Contribution,
) -> Result<(), ProfileClaimError> {
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
) -> Result<(), ProfileClaimError> {
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
            let hierarchy = ensure_table(document.as_table_mut(), "type_hierarchy", registry)?;
            let types = ensure_inline_table(hierarchy, "types", registry)?;
            types.insert(identity, json_to_edit_value(candidate, registry)?);
        }
        MapEntryTarget::LabelAssociations => {
            let hierarchy = ensure_table(document.as_table_mut(), "type_hierarchy", registry)?;
            let associations = ensure_table(hierarchy, "label_associations", registry)?;
            associations.insert(
                identity,
                Item::Value(json_to_edit_value(candidate, registry)?),
            );
        }
        MapEntryTarget::Namespaces | MapEntryTarget::ItemKinds => {
            let root = match target {
                MapEntryTarget::Namespaces => "namespaces",
                MapEntryTarget::ItemKinds => "item_kinds",
                _ => unreachable!(),
            };
            let table = ensure_table(document.as_table_mut(), root, registry)?;
            table.insert(
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
) -> Result<(), ProfileClaimError> {
    let values = match target {
        SetStringTarget::StrategicTypes => semantic.pointer("/type_hierarchy/strategic_types"),
    };
    if let Some(values) = values {
        let values = values
            .as_array()
            .ok_or_else(|| invalid_registry(registry, "set target is not an array"))?;
        if values.iter().any(|value| !value.is_string()) {
            return Err(invalid_registry(
                registry,
                "set target contains a non-string member",
            ));
        }
        if values.iter().any(|value| value.as_str() == Some(candidate)) {
            return Ok(());
        }
    }

    let hierarchy = ensure_table(document.as_table_mut(), "type_hierarchy", registry)?;
    let array = ensure_array(hierarchy, "strategic_types", registry)?;
    array.push(candidate);
    Ok(())
}

fn merge_keyed_array(
    registry: &str,
    document: &mut DocumentMut,
    target: KeyedArrayTarget,
    identity: &str,
    candidate: &JsonValue,
) -> Result<(), ProfileClaimError> {
    let array_name = target.array_name();
    let field = target.identity_field();
    let (array, preserved_comment) =
        ensure_array_of_tables(document.as_table_mut(), array_name, registry)?;
    let mut identities = BTreeSet::new();
    let mut existing = None;
    for table in array.iter() {
        let Some(actual) = table.get(field).and_then(Item::as_str) else {
            return Err(ProfileClaimError::UnknownContributionIdentity {
                target: registry.to_string(),
                field: field.to_string(),
            });
        };
        if !identities.insert(actual.to_string()) {
            return Err(ProfileClaimError::UnknownContributionIdentity {
                target: registry.to_string(),
                field: field.to_string(),
            });
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
) -> Result<(), ProfileClaimError> {
    let candidate = serde_json::to_value(candidate).expect("projection config serializes");
    if let Some(existing) = semantic
        .get("projection")
        .and_then(|projections| projections.get(name))
    {
        return equal_or_conflict(registry, name, existing, &candidate);
    }
    let projection = ensure_table(document.as_table_mut(), "projection", registry)?;
    projection.insert(
        name,
        Item::Table(json_object_to_table(&candidate, registry)?),
    );
    Ok(())
}

fn semantic_document(
    registry: &str,
    document: &DocumentMut,
) -> Result<JsonValue, ProfileClaimError> {
    toml_edit::de::from_str(&document.to_string())
        .map_err(|error| invalid_registry(registry, error.to_string()))
}

fn equal_or_conflict(
    registry: &str,
    identity: &str,
    existing: &JsonValue,
    candidate: &JsonValue,
) -> Result<(), ProfileClaimError> {
    if existing == candidate {
        Ok(())
    } else {
        Err(ProfileClaimError::ContributionConflict {
            target: registry.to_string(),
            identity: identity.to_string(),
        })
    }
}

fn ensure_table<'a>(
    parent: &'a mut Table,
    key: &str,
    registry: &str,
) -> Result<&'a mut Table, ProfileClaimError> {
    if !parent.contains_key(key) {
        parent.insert(key, Item::Table(Table::new()));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| invalid_registry(registry, format!("'{key}' is not a table")))
}

fn ensure_inline_table<'a>(
    parent: &'a mut Table,
    key: &str,
    registry: &str,
) -> Result<&'a mut InlineTable, ProfileClaimError> {
    if !parent.contains_key(key) {
        parent.insert(key, Item::Value(Value::InlineTable(InlineTable::new())));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_inline_table_mut)
        .ok_or_else(|| invalid_registry(registry, format!("'{key}' is not an inline table")))
}

fn ensure_array<'a>(
    parent: &'a mut Table,
    key: &str,
    registry: &str,
) -> Result<&'a mut Array, ProfileClaimError> {
    if !parent.contains_key(key) {
        parent.insert(key, Item::Value(Value::Array(Array::new())));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_array_mut)
        .ok_or_else(|| invalid_registry(registry, format!("'{key}' is not an array")))
}

fn ensure_array_of_tables<'a>(
    parent: &'a mut Table,
    key: &str,
    registry: &str,
) -> Result<(&'a mut ArrayOfTables, Option<String>), ProfileClaimError> {
    let preserved_comment = empty_array_comments(parent, key);
    let needs_array_of_tables = !parent.contains_key(key)
        || parent
            .get(key)
            .and_then(Item::as_array)
            .is_some_and(Array::is_empty);
    if needs_array_of_tables {
        parent.insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let array = parent
        .get_mut(key)
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| invalid_registry(registry, format!("'{key}' is not an array of tables")))?;
    Ok((array, preserved_comment))
}

fn empty_array_comments(parent: &Table, key: &str) -> Option<String> {
    let array = parent.get(key)?.as_array()?;
    if !array.is_empty() {
        return None;
    }
    let key_decor = parent.key(key)?.leaf_decor();
    let fragments = [
        key_decor.prefix(),
        key_decor.suffix(),
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

fn json_object_to_table(value: &JsonValue, registry: &str) -> Result<Table, ProfileClaimError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid_registry(registry, "contribution value is not a table"))?;
    object
        .iter()
        .try_fold(Table::new(), |mut table, (key, value)| {
            table.insert(key, json_to_item(value, registry)?);
            Ok(table)
        })
}

fn json_to_item(value: &JsonValue, registry: &str) -> Result<Item, ProfileClaimError> {
    match value {
        JsonValue::Object(_) => Ok(Item::Value(Value::InlineTable(json_to_inline_table(
            value, registry,
        )?))),
        _ => Ok(Item::Value(json_to_edit_value(value, registry)?)),
    }
}

fn json_to_edit_value(value: &JsonValue, registry: &str) -> Result<Value, ProfileClaimError> {
    match value {
        JsonValue::Null => Err(invalid_registry(registry, "null is not a TOML value")),
        JsonValue::Bool(value) => Ok(Value::from(*value)),
        JsonValue::Number(value) => value
            .as_i64()
            .map(Value::from)
            .or_else(|| value.as_f64().map(Value::from))
            .ok_or_else(|| invalid_registry(registry, "unsupported numeric value")),
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
) -> Result<InlineTable, ProfileClaimError> {
    value
        .as_object()
        .ok_or_else(|| invalid_registry(registry, "value is not a table"))?
        .iter()
        .try_fold(InlineTable::new(), |mut table, (key, value)| {
            table.insert(key, json_to_edit_value(value, registry)?);
            Ok(table)
        })
}

fn table_to_json(table: &Table, registry: &str) -> Result<JsonValue, ProfileClaimError> {
    toml_edit::de::from_str(&table.to_string())
        .map_err(|error| invalid_registry(registry, error.to_string()))
}

fn invalid_registry(registry: &str, message: impl Into<String>) -> ProfileClaimError {
    ProfileClaimError::InvalidRegistry {
        target: registry.to_string(),
        message: message.into(),
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|part| part == needle)
}

fn json_contains_interpolation(value: &JsonValue) -> bool {
    match value {
        JsonValue::String(value) => value.contains("{{jit:"),
        JsonValue::Array(values) => values.iter().any(json_contains_interpolation),
        JsonValue::Object(values) => values.values().any(json_contains_interpolation),
        _ => false,
    }
}
