use super::{
    CompleteProjectionConfig, Contribution, EmbeddedProfilePackage, KeyedArrayTarget,
    MapEntryTarget, ProjectedFileMode, ProjectionError, RepositorySnapshot, SetStringTarget,
    SnapshotEntry,
};
use crate::validation::repository::{
    projection_targets, render_projections, OverlayRepositoryView, RepositoryView,
};
use serde::Serialize;
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

const PLAN_HASH_DOMAIN: &[u8] = b"jit-profile-plan-v1\0";
// V1 has no variable surface. Reject the reserved profile interpolation
// namespace while leaving ordinary shell/template syntax as opaque asset bytes.
const RESERVED_INTERPOLATION_PREFIX: &[u8] = b"{{jit:";

/// Stable package and snapshot identities required to rebuild a plan under lock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanIdentity {
    /// Embedded package hash.
    pub package_hash: String,
    /// Per-target package hashes in deterministic path order.
    pub target_hashes: BTreeMap<String, String>,
    /// Hash of every captured target input and planned final image.
    pub plan_hash: String,
}

/// Planned action for one package-owned repository target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannedTargetAction {
    /// Target is absent and will be created.
    Create,
    /// Existing bytes or mode differ from the safe final image.
    Update,
    /// Existing bytes and mode already match exactly.
    NoOp,
}

/// Exact final image and classification for one target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedTarget {
    /// Repository-relative target path.
    pub path: String,
    /// Planned operation.
    pub action: PlannedTargetAction,
    /// Exact final bytes.
    pub bytes: Vec<u8>,
    /// Exact final mode intent.
    pub mode: ProjectedFileMode,
    /// Hash of the captured pre-plan entry, or `None` when absent.
    pub before_hash: Option<String>,
    /// Hash of final bytes and mode.
    pub after_hash: String,
}

/// Deterministic, validated pure profile application plan.
///
/// Proposed-state validation is authoritative at the command boundary (init/profile
/// capture the base image and validate the overlay), so this transitional plan
/// carries no validation report of its own; the whole type is on the increment-6
/// deletion list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileApplicationPlan {
    /// Stable package/snapshot identity.
    pub identity: PlanIdentity,
    /// Exact final targets sorted by repository-relative path.
    pub targets: BTreeMap<String, PlannedTarget>,
}

impl ProfileApplicationPlan {
    /// Whether execution would publish no bytes or mode changes.
    pub fn is_no_op(&self) -> bool {
        self.targets
            .values()
            .all(|target| target.action == PlannedTargetAction::NoOp)
    }

    /// Planned final bytes suitable for rebuilding the validation overlay.
    pub fn overlay_changes(&self) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        self.targets
            .values()
            .map(|target| (PathBuf::from(&target.path), Some(target.bytes.clone())))
            .collect()
    }
}

/// Pure profile planning failure.
#[derive(Debug, thiserror::Error)]
pub enum ProfilePlanError {
    /// Package target layout contains an ancestor/descendant collision.
    #[error("profile targets '{first}' and '{second}' overlap")]
    OverlappingTargets { first: String, second: String },
    /// A target or one of its ancestors is a symlink.
    #[error("profile target '{target}' traverses symlink '{component}'")]
    SymlinkPath { target: String, component: String },
    /// Existing filesystem entry cannot be safely represented.
    #[error("profile target '{target}' has unsupported filesystem state: {reason}")]
    UnsupportedFilesystem { target: String, reason: String },
    /// An ordinary package asset differs from the occupied target.
    #[error("profile asset target '{target}' contains differing bytes")]
    AssetConflict { target: String },
    /// Existing semantic identity has a differing value.
    #[error("profile contribution '{identity}' conflicts in '{target}'")]
    ContributionConflict { target: String, identity: String },
    /// Existing keyed registry entry lacks its required stable identity.
    #[error("registry '{target}' contains an unknown or duplicate '{field}' identity")]
    UnknownContributionIdentity { target: String, field: String },
    /// Registry bytes cannot be interpreted without rewriting user content.
    #[error("invalid profile registry '{target}': {message}")]
    InvalidRegistry { target: String, message: String },
    /// V1 packages cannot carry reserved interpolation tokens.
    #[error("profile source '{location}' contains unsupported interpolation token")]
    InvalidInterpolation { location: String },
    /// Embedded projection failed.
    #[error(transparent)]
    Projection(#[from] ProjectionError),
    /// A configured registry projection could not be rebuilt from final bytes.
    #[error("profile registry projection failed: {0}")]
    RegistryProjection(#[source] anyhow::Error),
}

/// Build and validate an exact application plan without mutating the repository.
pub fn plan_profile_application(
    package: &EmbeddedProfilePackage<'_>,
    snapshot: &RepositorySnapshot,
) -> Result<ProfileApplicationPlan, ProfilePlanError> {
    plan_profile_application_against(package, snapshot, Arc::new(snapshot.clone()))
}

/// Build a pure target plan while validating its exact overlay against a
/// separately supplied read-only repository view.
///
/// The application boundary uses this form to avoid copying unrelated build
/// trees into memory. The target snapshot remains immutable and authoritative
/// for every planned mutation; the validation view supplies only non-target
/// repository context while the caller holds the repository write lock.
pub fn plan_profile_application_against(
    package: &EmbeddedProfilePackage<'_>,
    snapshot: &RepositorySnapshot,
    validation_base: Arc<dyn RepositoryView>,
) -> Result<ProfileApplicationPlan, ProfilePlanError> {
    validate_interpolation(package)?;
    validate_target_layout(package, snapshot)?;

    let semantic = merge_semantic_contributions(package, snapshot)?;
    let existing_regions = package
        .manifest()
        .regions
        .iter()
        .filter_map(|region| {
            snapshot
                .file(&region.target)
                .map(|file| (region.target.clone(), file.bytes.clone()))
        })
        .collect();
    let projected = super::project_package(package, &existing_regions)?;
    let asset_targets = package
        .manifest()
        .assets
        .iter()
        .map(|asset| asset.target.as_str())
        .collect::<BTreeSet<_>>();
    let region_targets = package
        .manifest()
        .regions
        .iter()
        .map(|region| region.target.as_str())
        .collect::<BTreeSet<_>>();

    let mut desired = semantic;
    let semantic_view = OverlayRepositoryView::new(
        validation_base.clone(),
        desired
            .iter()
            .map(|(path, (bytes, _))| (PathBuf::from(path), Some(bytes.clone()))),
    )
    .expect("validated semantic target paths remain repository-relative");
    let managed_projection_targets =
        projection_targets(&semantic_view).map_err(ProfilePlanError::RegistryProjection)?;
    for (path, projected_file) in projected.files() {
        if asset_targets.contains(path.as_str()) {
            if let Some(existing) = snapshot.file(path) {
                if !managed_projection_targets.contains(Path::new(path))
                    && existing.bytes != projected_file.bytes
                {
                    return Err(ProfilePlanError::AssetConflict {
                        target: path.clone(),
                    });
                }
            }
        }
        let mode = if region_targets.contains(path.as_str()) {
            snapshot
                .file(path)
                .map_or(ProjectedFileMode::Regular, |file| file.mode)
        } else {
            projected_file.mode
        };
        desired.insert(path.clone(), (projected_file.bytes.clone(), mode));
    }
    let projected_view = OverlayRepositoryView::new(
        validation_base.clone(),
        desired
            .iter()
            .map(|(path, (bytes, _))| (PathBuf::from(path), Some(bytes.clone()))),
    )
    .expect("validated projected target paths remain repository-relative");
    for (target, bytes) in
        render_projections(&projected_view).map_err(ProfilePlanError::RegistryProjection)?
    {
        let target = target.to_string_lossy().into_owned();
        if let Some((existing, _)) = desired.get_mut(&target) {
            *existing = bytes;
        }
    }

    let targets = desired
        .into_iter()
        .map(|(path, (bytes, mode))| {
            let before = snapshot.file(&path);
            let action = match before {
                None => PlannedTargetAction::Create,
                Some(file) if file.bytes == bytes && file.mode == mode => PlannedTargetAction::NoOp,
                Some(_) => PlannedTargetAction::Update,
            };
            let before_hash = before.map(|file| hash_image(&file.bytes, file.mode));
            let after_hash = hash_image(&bytes, mode);
            (
                path.clone(),
                PlannedTarget {
                    path,
                    action,
                    bytes,
                    mode,
                    before_hash,
                    after_hash,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    // Proposed-state validation is authoritative at the command boundary: init and
    // profile capture the base image and validate the overlay through the closed
    // pipeline (this planner cannot open a session). `validation_base` still feeds
    // the view-based projection helpers above until increment 6 deletes this planner.
    let identity = PlanIdentity {
        package_hash: package.hashes().package.clone(),
        target_hashes: package.hashes().targets.clone(),
        plan_hash: hash_plan(package.hashes().package.as_str(), &targets),
    };
    Ok(ProfileApplicationPlan { identity, targets })
}

fn validate_interpolation(package: &EmbeddedProfilePackage<'_>) -> Result<(), ProfilePlanError> {
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
            return Err(ProfilePlanError::InvalidInterpolation {
                location: declaration.to_string(),
            });
        }
    }
    for (index, contribution) in package.manifest().contributions.iter().enumerate() {
        let value = serde_json::to_value(contribution).expect("manifest values serialize");
        if json_contains_interpolation(&value) {
            return Err(ProfilePlanError::InvalidInterpolation {
                location: format!("contribution[{index}]"),
            });
        }
    }
    Ok(())
}

fn validate_target_layout(
    package: &EmbeddedProfilePackage<'_>,
    snapshot: &RepositorySnapshot,
) -> Result<(), ProfilePlanError> {
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
    validate_target_overlaps(&semantic_targets, &content_targets)?;

    let targets = semantic_targets
        .into_iter()
        .chain(content_targets)
        .collect::<Vec<_>>();
    validate_target_filesystem(&targets, snapshot)
}

fn validate_target_overlaps(
    semantic_targets: &BTreeSet<String>,
    content_targets: &BTreeSet<String>,
) -> Result<(), ProfilePlanError> {
    if let Some(target) = semantic_targets.intersection(content_targets).next() {
        return Err(ProfilePlanError::OverlappingTargets {
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
            return Err(ProfilePlanError::OverlappingTargets {
                first: (*pair[0]).clone(),
                second: (*pair[1]).clone(),
            });
        }
    }
    Ok(())
}

fn validate_target_filesystem(
    targets: &[String],
    snapshot: &RepositorySnapshot,
) -> Result<(), ProfilePlanError> {
    for target in targets {
        let path = Path::new(target);
        for ancestor in path
            .ancestors()
            .filter(|ancestor| !ancestor.as_os_str().is_empty())
        {
            match snapshot.entry(ancestor) {
                Some(SnapshotEntry::Symlink { .. }) => {
                    return Err(ProfilePlanError::SymlinkPath {
                        target: target.clone(),
                        component: ancestor.display().to_string(),
                    });
                }
                Some(SnapshotEntry::Unsupported { reason }) => {
                    return Err(ProfilePlanError::UnsupportedFilesystem {
                        target: target.clone(),
                        reason: reason.clone(),
                    });
                }
                Some(SnapshotEntry::File(_)) if ancestor != path => {
                    return Err(ProfilePlanError::UnsupportedFilesystem {
                        target: target.clone(),
                        reason: format!("ancestor '{}' is a file", ancestor.display()),
                    });
                }
                Some(SnapshotEntry::Directory) if ancestor == path => {
                    return Err(ProfilePlanError::UnsupportedFilesystem {
                        target: target.clone(),
                        reason: "target is a directory".to_string(),
                    });
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn merge_semantic_contributions(
    package: &EmbeddedProfilePackage<'_>,
    snapshot: &RepositorySnapshot,
) -> Result<BTreeMap<String, (Vec<u8>, ProjectedFileMode)>, ProfilePlanError> {
    let mut documents = BTreeMap::<String, MergeDocument>::new();
    for contribution in &package.manifest().contributions {
        let target = contribution.registry_path().to_string();
        if !documents.contains_key(&target) {
            documents.insert(
                target.clone(),
                MergeDocument::load(&target, snapshot.file(&target))?,
            );
        }
        let document = documents
            .get_mut(&target)
            .expect("registry document was inserted");
        merge_contribution(&target, &mut document.document, contribution)?;
        document.changed = document.document.to_string().as_bytes() != document.original.as_slice();
    }

    Ok(documents
        .into_iter()
        .map(|(path, document)| {
            let mode = snapshot
                .file(&path)
                .map_or(ProjectedFileMode::Regular, |file| file.mode);
            let bytes = if document.changed {
                document.document.to_string().into_bytes()
            } else {
                document.original
            };
            (path, (bytes, mode))
        })
        .collect())
}

struct MergeDocument {
    original: Vec<u8>,
    document: DocumentMut,
    changed: bool,
}

impl MergeDocument {
    fn load(target: &str, file: Option<&super::SnapshotFile>) -> Result<Self, ProfilePlanError> {
        let original = file.map_or_else(Vec::new, |file| file.bytes.clone());
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
            changed: false,
        })
    }
}

fn merge_contribution(
    registry: &str,
    document: &mut DocumentMut,
    contribution: &Contribution,
) -> Result<(), ProfilePlanError> {
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
) -> Result<(), ProfilePlanError> {
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
) -> Result<(), ProfilePlanError> {
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
) -> Result<(), ProfilePlanError> {
    let array_name = target.array_name();
    let field = target.identity_field();
    let (array, preserved_comment) =
        ensure_array_of_tables(document.as_table_mut(), array_name, registry)?;
    let mut identities = BTreeSet::new();
    let mut existing = None;
    for table in array.iter() {
        let Some(actual) = table.get(field).and_then(Item::as_str) else {
            return Err(ProfilePlanError::UnknownContributionIdentity {
                target: registry.to_string(),
                field: field.to_string(),
            });
        };
        if !identities.insert(actual.to_string()) {
            return Err(ProfilePlanError::UnknownContributionIdentity {
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
) -> Result<(), ProfilePlanError> {
    let candidate = serde_json::to_value(candidate).expect("projection config serializes");
    // Conflict-check against the existing `[projection.<name>]` subtable so a
    // re-apply of the same projection is idempotent and a divergent one conflicts.
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
) -> Result<JsonValue, ProfilePlanError> {
    toml_edit::de::from_str(&document.to_string())
        .map_err(|error| invalid_registry(registry, error.to_string()))
}

fn equal_or_conflict(
    registry: &str,
    identity: &str,
    existing: &JsonValue,
    candidate: &JsonValue,
) -> Result<(), ProfilePlanError> {
    if existing == candidate {
        Ok(())
    } else {
        Err(ProfilePlanError::ContributionConflict {
            target: registry.to_string(),
            identity: identity.to_string(),
        })
    }
}

fn ensure_table<'a>(
    parent: &'a mut Table,
    key: &str,
    registry: &str,
) -> Result<&'a mut Table, ProfilePlanError> {
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
) -> Result<&'a mut InlineTable, ProfilePlanError> {
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
) -> Result<&'a mut Array, ProfilePlanError> {
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
) -> Result<(&'a mut ArrayOfTables, Option<String>), ProfilePlanError> {
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

fn json_object_to_table(value: &JsonValue, registry: &str) -> Result<Table, ProfilePlanError> {
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

fn json_to_item(value: &JsonValue, registry: &str) -> Result<Item, ProfilePlanError> {
    match value {
        JsonValue::Object(_) => Ok(Item::Value(Value::InlineTable(json_to_inline_table(
            value, registry,
        )?))),
        _ => Ok(Item::Value(json_to_edit_value(value, registry)?)),
    }
}

fn json_to_edit_value(value: &JsonValue, registry: &str) -> Result<Value, ProfilePlanError> {
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
) -> Result<InlineTable, ProfilePlanError> {
    value
        .as_object()
        .ok_or_else(|| invalid_registry(registry, "value is not a table"))?
        .iter()
        .try_fold(InlineTable::new(), |mut table, (key, value)| {
            table.insert(key, json_to_edit_value(value, registry)?);
            Ok(table)
        })
}

fn table_to_json(table: &Table, registry: &str) -> Result<JsonValue, ProfilePlanError> {
    toml_edit::de::from_str(&table.to_string())
        .map_err(|error| invalid_registry(registry, error.to_string()))
}

fn invalid_registry(registry: &str, message: impl Into<String>) -> ProfilePlanError {
    ProfilePlanError::InvalidRegistry {
        target: registry.to_string(),
        message: message.into(),
    }
}

fn hash_plan(package_hash: &str, targets: &BTreeMap<String, PlannedTarget>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(PLAN_HASH_DOMAIN);
    frame(&mut hasher, package_hash.as_bytes());
    for target in targets.values() {
        frame(&mut hasher, target.path.as_bytes());
        frame(
            &mut hasher,
            target.before_hash.as_deref().unwrap_or("-").as_bytes(),
        );
        frame(&mut hasher, target.after_hash.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn hash_image(bytes: &[u8], mode: ProjectedFileMode) -> String {
    let mut hasher = Sha256::new();
    hasher.update(match mode {
        ProjectedFileMode::Regular => b"regular\0".as_slice(),
        ProjectedFileMode::Executable => b"executable\0".as_slice(),
    });
    frame(&mut hasher, bytes);
    format!("{:x}", hasher.finalize())
}

fn frame(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{RegionDeclaration, RegionPlacement, SnapshotFile};
    use crate::storage::{IssueStore, JsonFileStorage};
    use include_dir::{include_dir, Dir};
    use std::fs;

    static ASSET_PACKAGE: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/tests/fixtures/profile-packages/planner-asset-only");
    static INTERPOLATION_PACKAGE: Dir<'_> = include_dir!(
        "$CARGO_MANIFEST_DIR/tests/fixtures/profile-packages/planner-invalid-interpolation"
    );
    static SYNTHETIC_PACKAGE: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/tests/fixtures/profile-packages/synthetic-valid");

    fn snapshot_from_tree(root: &Path) -> RepositorySnapshot {
        fn visit(root: &Path, current: &Path, entries: &mut Vec<(PathBuf, SnapshotEntry)>) {
            for entry in fs::read_dir(current).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                let kind = entry.file_type().unwrap();
                if kind.is_dir() {
                    entries.push((relative.clone(), SnapshotEntry::Directory));
                    visit(root, &path, entries);
                } else if kind.is_symlink() {
                    entries.push((
                        relative,
                        SnapshotEntry::Symlink {
                            target: fs::read_link(path).unwrap(),
                        },
                    ));
                } else {
                    entries.push((
                        relative,
                        SnapshotEntry::File(SnapshotFile {
                            bytes: fs::read(&path).unwrap(),
                            mode: ProjectedFileMode::Regular,
                        }),
                    ));
                }
            }
        }

        let mut entries = Vec::new();
        visit(root, root, &mut entries);
        RepositorySnapshot::new(root, entries).unwrap()
    }

    #[test]
    fn test_semantic_merges_preserve_comments_order_and_append_manifest_order() {
        let original = b"# keep\n[type_hierarchy]\n\
            types = { task = 4 }\n\
            strategic_types = [\"milestone\"]\n\n\
            [type_hierarchy.label_associations]\n\
            task = \"task\"\n";
        let snapshot = RepositorySnapshot::new(
            "/repo",
            [(
                PathBuf::from(".jit/config.toml"),
                SnapshotEntry::File(SnapshotFile {
                    bytes: original.to_vec(),
                    mode: ProjectedFileMode::Regular,
                }),
            )],
        )
        .unwrap();
        let mut document =
            MergeDocument::load(".jit/config.toml", snapshot.file(".jit/config.toml")).unwrap();
        for contribution in [
            Contribution::MapEntry {
                target: MapEntryTarget::TypeHierarchyTypes,
                identity: "epic".to_string(),
                value: JsonValue::from(2),
            },
            Contribution::MapEntry {
                target: MapEntryTarget::TypeHierarchyTypes,
                identity: "story".to_string(),
                value: JsonValue::from(3),
            },
            Contribution::SetString {
                target: SetStringTarget::StrategicTypes,
                value: "epic".to_string(),
            },
        ] {
            merge_contribution(".jit/config.toml", &mut document.document, &contribution).unwrap();
        }
        let rendered = document.document.to_string();
        assert!(rendered.starts_with("# keep\n[type_hierarchy]\n"));
        assert!(rendered.contains("task = 4"));
        assert!(rendered.contains("epic = 2"));
        assert!(rendered.contains("story = 3"));
        assert!(rendered.contains("\"milestone\""));
        assert!(rendered.contains("\"epic\""));
        assert!(rendered.find("task = 4").unwrap() < rendered.find("epic = 2").unwrap());
        assert!(rendered.find("epic = 2").unwrap() < rendered.find("story = 3").unwrap());
        assert!(rendered.ends_with("task = \"task\"\n"));
    }

    #[test]
    fn test_semantic_equality_is_noop_and_differing_identity_conflicts() {
        let mut document: DocumentMut = "[[gates]]\nkey = \"review\"\ntitle = \"Review\"\n"
            .parse()
            .unwrap();
        let equal = Contribution::KeyedArray {
            target: KeyedArrayTarget::Gates,
            identity: "review".to_string(),
            value: serde_json::json!({"title": "Review", "key": "review"}),
        };
        merge_contribution(".jit/gates.toml", &mut document, &equal).unwrap();
        assert_eq!(
            document.to_string(),
            "[[gates]]\nkey = \"review\"\ntitle = \"Review\"\n"
        );

        let conflict = Contribution::KeyedArray {
            target: KeyedArrayTarget::Gates,
            identity: "review".to_string(),
            value: serde_json::json!({"key": "review", "title": "Different"}),
        };
        assert!(matches!(
            merge_contribution(".jit/gates.toml", &mut document, &conflict),
            Err(ProfilePlanError::ContributionConflict { .. })
        ));
    }

    #[test]
    fn test_empty_keyed_registry_preserves_comments_when_appending_first_table() {
        let mut document: DocumentMut = "# registry note\ngates = [] # keep inline\n"
            .parse()
            .unwrap();
        let contribution = Contribution::KeyedArray {
            target: KeyedArrayTarget::Gates,
            identity: "review".to_string(),
            value: serde_json::json!({"key": "review", "title": "Review"}),
        };
        merge_contribution(".jit/gates.toml", &mut document, &contribution).unwrap();
        let rendered = document.to_string();
        assert!(rendered.contains("# registry note"));
        assert!(rendered.contains("# keep inline"));
        assert!(rendered.contains("[[gates]]"));
        assert!(rendered.find("# keep inline").unwrap() < rendered.find("[[gates]]").unwrap());
        assert_eq!(
            toml_edit::de::from_str::<JsonValue>(&rendered).unwrap()["gates"][0]["key"],
            "review"
        );
    }

    #[test]
    fn test_map_identity_with_json_pointer_characters_never_bypasses_conflict() {
        let original =
            "[namespaces]\n\"team/red~blue\" = { description = \"Keep\", unique = false }\n";
        let mut document: DocumentMut = original.parse().unwrap();
        let equal = Contribution::MapEntry {
            target: MapEntryTarget::Namespaces,
            identity: "team/red~blue".to_string(),
            value: serde_json::json!({"description": "Keep", "unique": false}),
        };
        merge_contribution(".jit/config.toml", &mut document, &equal).unwrap();
        assert_eq!(document.to_string(), original);

        let conflict = Contribution::MapEntry {
            target: MapEntryTarget::Namespaces,
            identity: "team/red~blue".to_string(),
            value: serde_json::json!({"description": "Different", "unique": false}),
        };
        assert!(matches!(
            merge_contribution(".jit/config.toml", &mut document, &conflict),
            Err(ProfilePlanError::ContributionConflict { identity, .. })
                if identity == "team/red~blue"
        ));
        assert_eq!(document.to_string(), original);
    }

    #[test]
    fn test_partial_projection_and_unknown_keyed_identity_are_rejected() {
        let mut projection: DocumentMut = "[projection.invariants]\nmode = \"region\"\n"
            .parse()
            .unwrap();
        let contribution = Contribution::Projection {
            name: "invariants".to_string(),
            value: toml::from_str(
                "kind = \"invariant\"\nmode = \"region\"\ntarget = \"AGENTS.md\"\nstyle = \"id-anchor\"\n",
            )
            .unwrap(),
        };
        assert!(matches!(
            merge_contribution(".jit/config.toml", &mut projection, &contribution),
            Err(ProfilePlanError::ContributionConflict { .. })
        ));

        let mut keyed: DocumentMut = "[[gates]]\ntitle = \"Missing key\"\n".parse().unwrap();
        let contribution = Contribution::KeyedArray {
            target: KeyedArrayTarget::Gates,
            identity: "review".to_string(),
            value: serde_json::json!({"key": "review", "title": "Review"}),
        };
        assert!(matches!(
            merge_contribution(".jit/gates.toml", &mut keyed, &contribution),
            Err(ProfilePlanError::UnknownContributionIdentity { .. })
        ));
    }

    #[test]
    fn test_preflight_rejects_symlink_overlap_and_unsupported_entries() {
        let package = EmbeddedProfilePackage::from_dir(&ASSET_PACKAGE).unwrap();
        let symlink = RepositorySnapshot::new(
            "/repo",
            [(
                PathBuf::from("docs"),
                SnapshotEntry::Symlink {
                    target: PathBuf::from("/outside"),
                },
            )],
        )
        .unwrap();
        assert!(matches!(
            validate_target_layout(&package, &symlink),
            Err(ProfilePlanError::SymlinkPath { .. })
        ));

        let unsupported = RepositorySnapshot::new(
            "/repo",
            [(
                PathBuf::from("docs/profile.txt"),
                SnapshotEntry::Unsupported {
                    reason: "special device".to_string(),
                },
            )],
        )
        .unwrap();
        assert!(matches!(
            validate_target_layout(&package, &unsupported),
            Err(ProfilePlanError::UnsupportedFilesystem { .. })
        ));

        assert!(matches!(
            validate_target_overlaps(
                &BTreeSet::from(["a".to_string()]),
                &BTreeSet::from(["a/b".to_string()])
            ),
            Err(ProfilePlanError::OverlappingTargets { first, second })
                if first == "a" && second == "a/b"
        ));
        assert!(matches!(
            validate_target_overlaps(
                &BTreeSet::from(["same".to_string()]),
                &BTreeSet::from(["same".to_string()])
            ),
            Err(ProfilePlanError::OverlappingTargets { first, second })
                if first == "same" && second == "same"
        ));
    }

    #[test]
    fn test_region_merge_rejects_malformed_markers_before_plan() {
        let region = RegionDeclaration {
            source: "region.md".to_string(),
            target: "AGENTS.md".to_string(),
            region_id: "profile-guidance".to_string(),
            placement: RegionPlacement::Append,
        };
        assert!(matches!(
            super::super::render_managed_region(
                &region,
                Some(b"<!-- jit:profile-guidance:begin -->"),
                b"content"
            ),
            Err(ProjectionError::MalformedRegion { .. })
        ));

        let temp = tempfile::tempdir().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        storage.init().unwrap();
        fs::write(
            temp.path().join("AGENTS.md"),
            b"<!-- jit:synthetic-guidance:begin -->",
        )
        .unwrap();
        let package = EmbeddedProfilePackage::from_dir(&SYNTHETIC_PACKAGE).unwrap();
        let result = plan_profile_application(&package, &snapshot_from_tree(temp.path()));
        assert!(
            matches!(
                &result,
                Err(ProfilePlanError::Projection(
                    ProjectionError::MalformedRegion { .. }
                ))
            ),
            "{result:?}"
        );
    }

    #[test]
    fn test_plan_validates_exact_overlay_and_exposes_stable_identities() {
        let temp = tempfile::tempdir().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        storage.init().unwrap();
        let package = EmbeddedProfilePackage::from_dir(&ASSET_PACKAGE).unwrap();
        let first = plan_profile_application(&package, &snapshot_from_tree(temp.path())).unwrap();
        assert_eq!(
            first.targets["docs/profile.txt"].action,
            PlannedTargetAction::Create
        );
        assert_eq!(first.identity.target_hashes.len(), 1);

        fs::create_dir_all(temp.path().join("docs")).unwrap();
        fs::write(
            temp.path().join("docs/profile.txt"),
            &first.targets["docs/profile.txt"].bytes,
        )
        .unwrap();
        let second = plan_profile_application(&package, &snapshot_from_tree(temp.path())).unwrap();
        assert!(second.is_no_op());
        assert_ne!(first.identity.plan_hash, second.identity.plan_hash);
        assert_eq!(
            second.identity.plan_hash,
            plan_profile_application(&package, &snapshot_from_tree(temp.path()))
                .unwrap()
                .identity
                .plan_hash
        );
    }

    #[test]
    fn test_asset_conflict_is_rejected_without_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        storage.init().unwrap();
        fs::create_dir_all(temp.path().join("docs")).unwrap();
        fs::write(temp.path().join("docs/profile.txt"), b"user content").unwrap();
        let before = fs::read(temp.path().join("docs/profile.txt")).unwrap();
        let package = EmbeddedProfilePackage::from_dir(&ASSET_PACKAGE).unwrap();

        assert!(matches!(
            plan_profile_application(&package, &snapshot_from_tree(temp.path())),
            Err(ProfilePlanError::AssetConflict { .. })
        ));
        assert_eq!(
            fs::read(temp.path().join("docs/profile.txt")).unwrap(),
            before
        );
    }

    #[test]
    fn test_plan_is_input_order_deterministic_and_detects_mode_only_updates() {
        let temp = tempfile::tempdir().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        storage.init().unwrap();
        let package = EmbeddedProfilePackage::from_dir(&ASSET_PACKAGE).unwrap();
        fs::create_dir_all(temp.path().join("docs")).unwrap();
        fs::write(
            temp.path().join("docs/profile.txt"),
            package.source_bytes("assets/profile.txt").unwrap(),
        )
        .unwrap();

        let baseline = snapshot_from_tree(temp.path());
        let mut reversed = baseline
            .entries()
            .iter()
            .map(|(path, entry)| (path.clone(), entry.clone()))
            .collect::<Vec<_>>();
        reversed.reverse();
        let reversed = RepositorySnapshot::new(temp.path(), reversed).unwrap();
        assert_eq!(
            plan_profile_application(&package, &baseline)
                .unwrap()
                .identity,
            plan_profile_application(&package, &reversed)
                .unwrap()
                .identity
        );

        let mode_changed = RepositorySnapshot::new(
            temp.path(),
            baseline.entries().iter().map(|(path, entry)| {
                let entry = if path == Path::new("docs/profile.txt") {
                    SnapshotEntry::File(SnapshotFile {
                        bytes: package.source_bytes("assets/profile.txt").unwrap().to_vec(),
                        mode: ProjectedFileMode::Executable,
                    })
                } else {
                    entry.clone()
                };
                (path.clone(), entry)
            }),
        )
        .unwrap();
        let plan = plan_profile_application(&package, &mode_changed).unwrap();
        assert_eq!(
            plan.targets["docs/profile.txt"].action,
            PlannedTargetAction::Update
        );
        assert_eq!(
            plan.targets["docs/profile.txt"].mode,
            ProjectedFileMode::Regular
        );
    }

    #[test]
    fn test_plan_rejects_reserved_interpolation_before_validation() {
        let temp = tempfile::tempdir().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        storage.init().unwrap();
        let package = EmbeddedProfilePackage::from_dir(&INTERPOLATION_PACKAGE).unwrap();
        assert!(matches!(
            plan_profile_application(&package, &snapshot_from_tree(temp.path())),
            Err(ProfilePlanError::InvalidInterpolation { location })
                if location == "assets/profile.txt"
        ));
    }
}
