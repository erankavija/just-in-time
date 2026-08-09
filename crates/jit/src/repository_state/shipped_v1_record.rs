//! One deliberately narrow migration boundary for the released v1 dogfood
//! applied-record image (`@/inv/canonical-cutover`). Remove this module and its
//! caller once the final supported JIT binary capable of encountering that
//! record is outside the upgrade path; ordinary record readers remain v2-only.

use super::*;
use serde::Deserialize;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

const EVIDENCE_BYTES: &[u8] = include_bytes!("shipped_v1_dogfood_evidence.json");
const EVIDENCE_SHA256: &str = "26976c0d4bc3ac8e5df5b05b98068b26da198385eae2eec36e3eb1470dc8e042";
const SHIPPED_ID: &str = "jit-dogfood";
const SHIPPED_VERSION: &str = "1.0.0";
const SHIPPED_COMPATIBLE_JIT: &str = ">=1.0.0, <2.0.0";
const SHIPPED_PACKAGE_HASH: &str =
    "43829e7e032e5e9ec40776103b1996f15e7291664c8b11e403c20b7f54af905c";

#[cfg(test)]
static EXACT_CONVERSION_COUNT: AtomicUsize = AtomicUsize::new(0);

/// A typed, actionable failure at the sole shipped-v1 conversion boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ShippedV1MigrationError {
    #[error("cannot decode the shipped five-field applied-profile v1 record: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("shipped-v1 migration only accepts embedded jit-dogfood 1.0.0 provenance")]
    UnexpectedRecord,
    #[error("the pinned shipped-v1 migration evidence is invalid: {0}")]
    InvalidEvidence(String),
    #[error("shipped-v1 record target hashes do not exactly match the pinned dogfood image")]
    TargetHashesDiffer,
    #[error("shipped-v1 migration cannot read current repository unit '{path}'")]
    MissingUnit { path: String },
    #[error("shipped-v1 migration detected drift in '{path}'")]
    Drift { path: String },
    #[error("shipped-v1 record at '{path}' names '{id}', expected '{expected}'")]
    RecordPathMismatch {
        path: String,
        id: String,
        expected: String,
    },
    #[error(
        "shipped-v1 migration found malformed or ambiguous semantic unit '{identity}' in '{path}'"
    )]
    AmbiguousSemantic { path: String, identity: String },
}

/// Exact historical record wire. This is intentionally not an enum variant of
/// `AppliedProfileRecord`: it is used only by the named one-way boundary.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShippedV1AppliedProfileRecord {
    id: String,
    version: String,
    origin: crate::domain::ProfileOrigin,
    package_hash: String,
    target_hashes: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    evidence_commit: String,
    manifest_path: String,
    manifest_sha256: String,
    package: EvidencePackage,
    target_hashes: BTreeMap<String, String>,
    semantic_contributions: Vec<EvidenceSemantic>,
    assets: Vec<EvidenceAsset>,
    regions: Vec<EvidenceRegion>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidencePackage {
    files: usize,
    bytes: usize,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceSemantic {
    target: String,
    kind: String,
    identity: String,
    declaration_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceAsset {
    target: String,
    source: String,
    executable: bool,
    historical_mode: String,
    source_sha256: String,
    source_bytes: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceRegion {
    target: String,
    source: String,
    region_id: String,
    placement: String,
    historical_mode: String,
    source_sha256: String,
    source_bytes: usize,
    declaration_sha256: String,
}

/// Decode and convert exactly the released v1 dogfood record against the
/// current captured units. Every pin is checked before a v2 record is returned,
/// so callers can place its write in the same recoverable materialization plan.
///
/// This function is the migration's named removal point: delete it, its pinned
/// evidence, and its mutating caller once the final binary capable of reading
/// the shipped v1 image is outside the supported upgrade path.
pub(crate) fn migrate_shipped_v1_record(
    bytes: &[u8],
    base: &RepositoryImage,
) -> Result<AppliedProfileRecord, ShippedV1MigrationError> {
    #[cfg(test)]
    EXACT_CONVERSION_COUNT.fetch_add(1, Ordering::Relaxed);
    let record = serde_json::from_slice::<ShippedV1AppliedProfileRecord>(bytes)?;
    let evidence = production_evidence()?;
    migrate_with_evidence(&record, base, &evidence)
}

#[cfg(test)]
pub(super) fn reset_exact_conversion_count() {
    EXACT_CONVERSION_COUNT.store(0, Ordering::Relaxed);
}

#[cfg(test)]
pub(super) fn exact_conversion_count() -> usize {
    EXACT_CONVERSION_COUNT.load(Ordering::Relaxed)
}

/// Identify only the exact historical record shape and pinned package metadata
/// before a mutating command resolves its selected closure. This does *not*
/// accept a record for ordinary reads or conversion: current repository units
/// are still authenticated by [`migrate_shipped_v1_record`] under the held
/// application session before any publication is possible.
pub(crate) fn is_shipped_v1_candidate(bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<JsonValue>(bytes) else {
        return false;
    };
    let Some(record) = value.as_object() else {
        return false;
    };
    ["id", "version", "origin", "package_hash", "target_hashes"]
        .iter()
        .all(|key| record.contains_key(*key))
        && record.get("id").and_then(JsonValue::as_str) == Some(SHIPPED_ID)
        && record.get("version").and_then(JsonValue::as_str) == Some(SHIPPED_VERSION)
        && record.get("package_hash").and_then(JsonValue::as_str) == Some(SHIPPED_PACKAGE_HASH)
        && record
            .get("origin")
            .and_then(JsonValue::as_object)
            .and_then(|origin| origin.get("source"))
            .and_then(JsonValue::as_str)
            == Some("embedded")
}

/// Exact current repository units required before the migration matcher may
/// authenticate a shipped-v1 record. The command captures these before its
/// normal semantic preflight; this list never supplies package behavior.
pub(crate) fn shipped_v1_migration_paths() -> Result<Vec<String>, ShippedV1MigrationError> {
    Ok(production_evidence()?.target_hashes.into_keys().collect())
}

fn production_evidence() -> Result<Evidence, ShippedV1MigrationError> {
    if sha256(EVIDENCE_BYTES) != EVIDENCE_SHA256 {
        return Err(ShippedV1MigrationError::InvalidEvidence(
            "embedded evidence checksum differs from the pinned historical artifact".into(),
        ));
    }
    let evidence = serde_json::from_slice::<Evidence>(EVIDENCE_BYTES)?;
    if evidence.evidence_commit != "e6aabd515beda27ec97d1b6c2daf7ecd01073837"
        || evidence.manifest_path != "profiles/jit-dogfood/manifest.toml"
        || evidence.manifest_sha256
            != "cffc4cbccc5c87389daa732574545d9accb647cbdde9af84d4dfc783056f295d"
        || evidence.package.files != 64
        || evidence.package.bytes != 432_282
        || evidence.package.sha256 != SHIPPED_PACKAGE_HASH
        || evidence.target_hashes.len() != 68
        || evidence.semantic_contributions.len() != 40
        || evidence.assets.len() != 62
        || evidence.regions.len() != 1
    {
        return Err(ShippedV1MigrationError::InvalidEvidence(
            "embedded evidence no longer names the released dogfood v1 image".into(),
        ));
    }
    Ok(evidence)
}

fn migrate_with_evidence(
    record: &ShippedV1AppliedProfileRecord,
    base: &RepositoryImage,
    evidence: &Evidence,
) -> Result<AppliedProfileRecord, ShippedV1MigrationError> {
    if record.id != SHIPPED_ID
        || record.version != SHIPPED_VERSION
        || record.origin != crate::domain::ProfileOrigin::Embedded
        || record.package_hash != SHIPPED_PACKAGE_HASH
    {
        return Err(ShippedV1MigrationError::UnexpectedRecord);
    }
    if record.target_hashes != evidence.target_hashes {
        return Err(ShippedV1MigrationError::TargetHashesDiffer);
    }
    let semantic = evidence
        .semantic_contributions
        .iter()
        .map(|pin| semantic_claim(base, pin))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let assets = evidence
        .assets
        .iter()
        .map(|pin| asset_claim(base, pin))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let regions = evidence
        .regions
        .iter()
        .map(|pin| region_claim(base, pin))
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(AppliedProfileRecord::new(
        SHIPPED_ID,
        SHIPPED_VERSION,
        SHIPPED_COMPATIBLE_JIT,
        crate::domain::ProfileOrigin::Embedded,
        SHIPPED_PACKAGE_HASH,
        ResolvedVariables::default(),
        semantic.into_iter().chain(assets).chain(regions).collect(),
    ))
}

fn semantic_claim(
    base: &RepositoryImage,
    pin: &EvidenceSemantic,
) -> Result<AppliedProfileClaim, ShippedV1MigrationError> {
    let contribution = semantic_contribution(base, pin)?;
    if sha256(
        &canonical_json_bytes(&contribution)
            .map_err(|error| ShippedV1MigrationError::InvalidEvidence(error.to_string()))?,
    ) != pin.declaration_sha256
    {
        return Err(ShippedV1MigrationError::Drift {
            path: pin.target.clone(),
        });
    }
    AppliedProfileClaim::semantic(&contribution, true)
        .map_err(|error| ShippedV1MigrationError::InvalidEvidence(error.to_string()))
}

fn semantic_contribution(
    base: &RepositoryImage,
    pin: &EvidenceSemantic,
) -> Result<Contribution, ShippedV1MigrationError> {
    let semantic = semantic_registry(base, &pin.target)?;
    match (pin.target.as_str(), pin.kind.as_str()) {
        (".jit/config.toml", "map-entry") => {
            let candidates = [
                MapEntryTarget::TypeHierarchyTypes,
                MapEntryTarget::LabelAssociations,
                MapEntryTarget::Namespaces,
                MapEntryTarget::ItemKinds,
            ]
            .into_iter()
            .filter_map(|target| {
                semantic_map_entry(&semantic, target.table_path(), &pin.identity).map(|value| {
                    Contribution::MapEntry {
                        target,
                        identity: pin.identity.clone(),
                        value: value.clone(),
                    }
                })
            })
            .collect::<Vec<_>>();
            exactly_one(candidates, pin)
        }
        (".jit/config.toml", "projection") => semantic
            .get("projection")
            .and_then(|entries| entries.get(&pin.identity))
            .cloned()
            .map(|value| {
                serde_json::from_value(value).map(|value| Contribution::Projection {
                    name: pin.identity.clone(),
                    value,
                })
            })
            .transpose()
            .map_err(|_| ShippedV1MigrationError::AmbiguousSemantic {
                path: pin.target.clone(),
                identity: pin.identity.clone(),
            })?
            .ok_or_else(|| ShippedV1MigrationError::AmbiguousSemantic {
                path: pin.target.clone(),
                identity: pin.identity.clone(),
            }),
        (target, "keyed-array") => {
            let target = match target {
                ".jit/gates.toml" => KeyedArrayTarget::Gates,
                ".jit/invariants.toml" => KeyedArrayTarget::Invariants,
                ".jit/rules.toml" => KeyedArrayTarget::Rules,
                ".jit/templates.toml" => KeyedArrayTarget::Templates,
                _ => {
                    return Err(ShippedV1MigrationError::AmbiguousSemantic {
                        path: pin.target.clone(),
                        identity: pin.identity.clone(),
                    })
                }
            };
            let entries = semantic
                .get(target.array_name())
                .and_then(JsonValue::as_array)
                .ok_or_else(|| ShippedV1MigrationError::AmbiguousSemantic {
                    path: pin.target.clone(),
                    identity: pin.identity.clone(),
                })?;
            exactly_one(
                entries
                    .iter()
                    .filter(|value| {
                        value
                            .get(target.identity_field())
                            .and_then(JsonValue::as_str)
                            == Some(pin.identity.as_str())
                    })
                    .cloned()
                    .map(|value| Contribution::KeyedArray {
                        target,
                        identity: pin.identity.clone(),
                        value,
                    })
                    .collect(),
                pin,
            )
        }
        _ => Err(ShippedV1MigrationError::AmbiguousSemantic {
            path: pin.target.clone(),
            identity: pin.identity.clone(),
        }),
    }
}

fn exactly_one(
    candidates: Vec<Contribution>,
    pin: &EvidenceSemantic,
) -> Result<Contribution, ShippedV1MigrationError> {
    match candidates.as_slice() {
        [contribution] => Ok(contribution.clone()),
        _ => Err(ShippedV1MigrationError::AmbiguousSemantic {
            path: pin.target.clone(),
            identity: pin.identity.clone(),
        }),
    }
}

fn semantic_registry(
    base: &RepositoryImage,
    target: &str,
) -> Result<JsonValue, ShippedV1MigrationError> {
    let path = base
        .layout()
        .classify_repository_relative(target)
        .map_err(|_| ShippedV1MigrationError::MissingUnit {
            path: target.to_string(),
        })?;
    let RepositoryEntry::File { bytes, mode, .. } =
        base.entry(&path)
            .map_err(|_| ShippedV1MigrationError::MissingUnit {
                path: target.to_string(),
            })?
    else {
        return Err(ShippedV1MigrationError::MissingUnit {
            path: target.to_string(),
        });
    };
    let document = MergeDocument::load(target, Some((bytes.clone(), *mode))).map_err(|_| {
        ShippedV1MigrationError::AmbiguousSemantic {
            path: target.to_string(),
            identity: "registry".into(),
        }
    })?;
    semantic_document(target, &document.document).map_err(|_| {
        ShippedV1MigrationError::AmbiguousSemantic {
            path: target.to_string(),
            identity: "registry".into(),
        }
    })
}

fn asset_claim(
    base: &RepositoryImage,
    pin: &EvidenceAsset,
) -> Result<AppliedProfileClaim, ShippedV1MigrationError> {
    let path = base
        .layout()
        .classify_repository_relative(&pin.target)
        .map_err(|_| ShippedV1MigrationError::MissingUnit {
            path: pin.target.clone(),
        })?;
    let expected_mode = if pin.executable && pin.historical_mode == "100755" {
        FileMode::Executable
    } else if !pin.executable && pin.historical_mode == "100644" {
        FileMode::Regular
    } else {
        return Err(ShippedV1MigrationError::InvalidEvidence(pin.source.clone()));
    };
    let RepositoryEntry::File { bytes, mode, .. } =
        base.entry(&path)
            .map_err(|_| ShippedV1MigrationError::MissingUnit {
                path: pin.target.clone(),
            })?
    else {
        return Err(ShippedV1MigrationError::MissingUnit {
            path: pin.target.clone(),
        });
    };
    if *mode != expected_mode
        || bytes.len() != pin.source_bytes
        || sha256(bytes) != pin.source_sha256
    {
        return Err(ShippedV1MigrationError::Drift {
            path: pin.target.clone(),
        });
    }
    Ok(AppliedProfileClaim::asset(&path, bytes, *mode, true))
}

fn region_claim(
    base: &RepositoryImage,
    pin: &EvidenceRegion,
) -> Result<AppliedProfileClaim, ShippedV1MigrationError> {
    let region_id = pinned_region_id(pin)?;
    let path = base
        .layout()
        .classify_repository_relative(&pin.target)
        .map_err(|_| ShippedV1MigrationError::MissingUnit {
            path: pin.target.clone(),
        })?;
    let RepositoryEntry::File { bytes, mode, .. } =
        base.entry(&path)
            .map_err(|_| ShippedV1MigrationError::MissingUnit {
                path: pin.target.clone(),
            })?
    else {
        return Err(ShippedV1MigrationError::MissingUnit {
            path: pin.target.clone(),
        });
    };
    let source =
        normalized_dogfood_region(bytes).ok_or_else(|| ShippedV1MigrationError::Drift {
            path: pin.target.clone(),
        })?;
    if *mode != FileMode::Regular
        || source.len() != pin.source_bytes
        || sha256(&source) != pin.source_sha256
    {
        return Err(ShippedV1MigrationError::Drift {
            path: pin.target.clone(),
        });
    }
    Ok(AppliedProfileClaim::managed_region(
        &path, region_id, &source, *mode, true,
    ))
}

fn normalized_dogfood_region(bytes: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let begin = "<!-- jit:dogfood-guidance:begin -->\n";
    let end = "<!-- jit:dogfood-guidance:end -->";
    let nested_begin = "<!-- jit:invariants:begin -->";
    let nested_end = "<!-- jit:invariants:end -->";
    let region = uniquely_marked_region(text, begin, end)?;
    let (before, nested) = region.split_once(nested_begin)?;
    let (_, after) = nested.split_once(nested_end)?;
    (region.matches(nested_begin).count() == 1
        && region.matches(nested_end).count() == 1
        && text.matches(nested_begin).count() == 1
        && text.matches(nested_end).count() == 1)
        .then_some(())?;
    Some(
        format!("{before}{nested_begin}\n_No invariants declared._\n{nested_end}{after}")
            .into_bytes(),
    )
}

fn pinned_region_id(
    pin: &EvidenceRegion,
) -> Result<crate::profile::RegionId, ShippedV1MigrationError> {
    let region_id = crate::profile::RegionId::try_from(pin.region_id.clone())
        .map_err(ShippedV1MigrationError::InvalidEvidence)?;
    if pin.target != "AGENTS.md"
        || pin.source != "assets/regions/agents-jit-guidance.md"
        || pin.placement != "append"
        || pin.historical_mode != "100644"
    {
        return Err(ShippedV1MigrationError::InvalidEvidence(pin.source.clone()));
    }
    let declaration = crate::profile::RegionDeclaration {
        source: pin.source.clone(),
        target: pin.target.clone(),
        region_id: region_id.clone(),
        placement: crate::profile::RegionPlacement::Append,
        template: false,
    };
    let actual = canonical_json_bytes(&declaration)
        .map_err(|error| ShippedV1MigrationError::InvalidEvidence(error.to_string()))?;
    if sha256(&actual) != pin.declaration_sha256 {
        return Err(ShippedV1MigrationError::InvalidEvidence(
            "region declaration metadata does not match the pinned historical declaration".into(),
        ));
    }
    Ok(region_id)
}

fn uniquely_marked_region<'a>(text: &'a str, begin: &str, end: &str) -> Option<&'a str> {
    (text.matches(begin).count() == 1 && text.matches(end).count() == 1).then_some(())?;
    let begin_at = text.find(begin)? + begin.len();
    let end_at = text.find(end)?;
    (begin_at < end_at).then(|| &text[begin_at..end_at])
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::{
        CaptureBudget, CaptureSpec, EntryIdentity, RepositoryLayout, RepositoryRootEvidence,
    };

    fn synthetic_image(path: &str, bytes: Option<&[u8]>) -> RepositoryImage {
        let layout = RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "worktree", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap();
        let path = layout.classify_repository_relative(path).unwrap();
        let mut spec = CaptureSpec::phase_one(
            [],
            CaptureBudget {
                max_paths: 1,
                max_listings: 0,
                max_bytes: 1024,
                max_depth: 8,
            },
        )
        .unwrap();
        spec.discover_paths([path.clone()]).unwrap();
        let entry = match bytes {
            Some(bytes) => RepositoryEntry::File {
                identity: EntryIdentity::for_bytes("synthetic", bytes).unwrap(),
                bytes: bytes.to_vec(),
                mode: FileMode::Regular,
            },
            None => RepositoryEntry::Absent,
        };
        RepositoryImage::close(
            layout,
            spec,
            BTreeMap::from([(path, entry)]),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap()
    }

    fn synthetic_evidence(bytes: &[u8]) -> Evidence {
        Evidence {
            evidence_commit: "synthetic".into(),
            manifest_path: "synthetic".into(),
            manifest_sha256: "a".repeat(64),
            package: EvidencePackage {
                files: 1,
                bytes: 1,
                sha256: SHIPPED_PACKAGE_HASH.into(),
            },
            target_hashes: BTreeMap::new(),
            semantic_contributions: Vec::new(),
            assets: vec![EvidenceAsset {
                target: "README.md".into(),
                source: "assets/readme.md".into(),
                executable: false,
                historical_mode: "100644".into(),
                source_sha256: sha256(bytes),
                source_bytes: bytes.len(),
            }],
            regions: Vec::new(),
        }
    }

    fn shipped_record(target_hashes: BTreeMap<String, String>) -> ShippedV1AppliedProfileRecord {
        ShippedV1AppliedProfileRecord {
            id: SHIPPED_ID.into(),
            version: SHIPPED_VERSION.into(),
            origin: crate::domain::ProfileOrigin::Embedded,
            package_hash: SHIPPED_PACKAGE_HASH.into(),
            target_hashes,
        }
    }

    #[test]
    fn test_production_evidence_pins_the_exact_shipped_dogfood_image() {
        let evidence = production_evidence().expect("the embedded historical evidence is pinned");
        assert_eq!(evidence.target_hashes.len(), 68);
        assert_eq!(evidence.semantic_contributions.len(), 40);
        assert_eq!(evidence.assets.len(), 62);
        assert_eq!(evidence.regions.len(), 1);
    }

    #[test]
    fn test_shipped_v1_decoder_rejects_missing_or_unknown_fields() {
        let missing = serde_json::json!({
            "id": SHIPPED_ID,
            "version": SHIPPED_VERSION,
            "origin": { "source": "embedded" },
            "package_hash": SHIPPED_PACKAGE_HASH,
        });
        let unknown = serde_json::json!({
            "id": SHIPPED_ID,
            "version": SHIPPED_VERSION,
            "origin": { "source": "embedded" },
            "package_hash": SHIPPED_PACKAGE_HASH,
            "target_hashes": {},
            "record_version": 1,
        });
        assert!(serde_json::from_value::<ShippedV1AppliedProfileRecord>(missing).is_err());
        assert!(serde_json::from_value::<ShippedV1AppliedProfileRecord>(unknown).is_err());
    }

    #[test]
    fn test_migrate_with_evidence_rejects_a_non_embedded_or_non_dogfood_record_before_units() {
        let evidence = production_evidence().expect("the embedded historical evidence is pinned");
        let record = ShippedV1AppliedProfileRecord {
            id: "other".into(),
            version: SHIPPED_VERSION.into(),
            origin: crate::domain::ProfileOrigin::Embedded,
            package_hash: SHIPPED_PACKAGE_HASH.into(),
            target_hashes: evidence.target_hashes.clone(),
        };
        let layout = RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "worktree", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap();
        let image = RepositoryImage::close(
            layout,
            CaptureSpec::phase_one(
                [],
                CaptureBudget {
                    max_paths: 1,
                    max_listings: 0,
                    max_bytes: 0,
                    max_depth: 1,
                },
            )
            .unwrap(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();
        assert!(matches!(
            migrate_with_evidence(&record, &image, &evidence),
            Err(ShippedV1MigrationError::UnexpectedRecord)
        ));
    }

    #[test]
    fn test_migrate_with_evidence_converts_a_pinned_synthetic_asset_to_a_retained_v2_claim() {
        let bytes = b"pinned asset\n";
        let record = shipped_record(BTreeMap::new());
        let migrated = migrate_with_evidence(
            &record,
            &synthetic_image("README.md", Some(bytes)),
            &synthetic_evidence(bytes),
        )
        .expect("the parameterized matcher converts a complete synthetic image");

        assert_eq!(migrated.record_version, APPLIED_PROFILE_RECORD_VERSION);
        assert_eq!(migrated.origin, crate::domain::ProfileOrigin::Embedded);
        assert!(migrated.claims.iter().all(|claim| claim.retain_if_unowned));
        assert!(matches!(
            migrated.claims.first().map(|claim| &claim.identity),
            Some(AppliedProfileClaimIdentity::Asset { .. })
        ));
    }

    #[test]
    fn test_migrate_with_evidence_fails_closed_for_metadata_target_missing_and_drift() {
        let bytes = b"pinned asset\n";
        let evidence = synthetic_evidence(bytes);
        let image = synthetic_image("README.md", Some(bytes));
        let mut wrong_hash = shipped_record(BTreeMap::new());
        wrong_hash.package_hash = "b".repeat(64);
        let wrong_targets = shipped_record(BTreeMap::from([("README.md".into(), "a".repeat(64))]));

        assert!(matches!(
            migrate_with_evidence(&wrong_hash, &image, &evidence),
            Err(ShippedV1MigrationError::UnexpectedRecord)
        ));
        assert!(matches!(
            migrate_with_evidence(&wrong_targets, &image, &evidence),
            Err(ShippedV1MigrationError::TargetHashesDiffer)
        ));
        assert!(matches!(
            migrate_with_evidence(
                &shipped_record(BTreeMap::new()),
                &synthetic_image("README.md", None),
                &evidence
            ),
            Err(ShippedV1MigrationError::MissingUnit { .. })
        ));
        assert!(matches!(
            migrate_with_evidence(
                &shipped_record(BTreeMap::new()),
                &synthetic_image("README.md", Some(b"drift\n")),
                &evidence
            ),
            Err(ShippedV1MigrationError::Drift { .. })
        ));
    }

    #[test]
    fn test_normalized_dogfood_region_rejects_duplicate_or_misnested_markers() {
        let valid = concat!(
            "before\n",
            "<!-- jit:dogfood-guidance:begin -->\n",
            "owned\n",
            "<!-- jit:invariants:begin -->\n",
            "old\n",
            "<!-- jit:invariants:end -->\n",
            "<!-- jit:dogfood-guidance:end -->\n",
        );
        let duplicate_outer = format!(
            "{valid}<!-- jit:dogfood-guidance:begin -->\ntrailing\n<!-- jit:dogfood-guidance:end -->"
        );
        let duplicate_nested = valid.replacen(
            "<!-- jit:invariants:end -->",
            "<!-- jit:invariants:end -->\n<!-- jit:invariants:end -->",
            1,
        );

        assert!(normalized_dogfood_region(valid.as_bytes()).is_some());
        assert!(normalized_dogfood_region(duplicate_outer.as_bytes()).is_none());
        assert!(normalized_dogfood_region(duplicate_nested.as_bytes()).is_none());
    }

    #[test]
    fn test_pinned_region_id_rejects_wrong_declaration_metadata() {
        let mut pin = production_evidence()
            .expect("production evidence is available")
            .regions
            .into_iter()
            .next()
            .expect("one region pin");
        pin.target = "README.md".into();

        assert!(matches!(
            pinned_region_id(&pin),
            Err(ShippedV1MigrationError::InvalidEvidence(_))
        ));
    }

    #[test]
    fn test_semantic_contribution_rejects_malformed_or_ambiguous_current_units() {
        let ambiguous = b"[type_hierarchy]\ntypes = { shared = 1 }\n\n[namespaces.shared]\ndescription = \"duplicate identity\"\n";
        let pin = EvidenceSemantic {
            target: ".jit/config.toml".into(),
            kind: "map-entry".into(),
            identity: "shared".into(),
            declaration_sha256: "a".repeat(64),
        };
        assert!(matches!(
            semantic_contribution(&synthetic_image(".jit/config.toml", Some(ambiguous)), &pin),
            Err(ShippedV1MigrationError::AmbiguousSemantic { .. })
        ));
        assert!(matches!(
            semantic_contribution(
                &synthetic_image(".jit/config.toml", Some(b"not = [valid")),
                &pin
            ),
            Err(ShippedV1MigrationError::AmbiguousSemantic { .. })
        ));
    }
}
