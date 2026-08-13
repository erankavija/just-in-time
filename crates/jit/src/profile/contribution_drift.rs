//! Packaged contributions bound to the repository registry entries they
//! restate.
//!
//! A contribution and the registry entry it restates are two copies of one
//! declaration. The repository's copy is what this checkout's own validation
//! reads; the packaged copy is what an adopter receives, and an application of
//! the package composes the two — so once they disagree, one of them is
//! carrying a ruling the other never recorded. Nothing in the manifest binds
//! them, which is what this module supplies.
//!
//! Which repository entry a contribution restates is already answered by data:
//! every contribution names its `target` and its `identity`, and the target
//! names the registry file, the table or array inside it, and the field
//! carrying an entry's identity. The pairs are derived from those declarations
//! alone, so a contribution added to a manifest joins the comparison without an
//! edit here.
//!
//! Everything is pure: the registry texts arrive as arguments and nothing is
//! read from the filesystem. The module compiles only for the crate's own tests
//! and the dev-dependency-active builds those need, so an adopter build carries
//! none of it.
//!
//! # What the comparison normalizes through
//!
//! A contribution's value is a serialized fragment and a registry entry is TOML
//! this repository authors, so the two texts differ in spelling long before
//! they differ in meaning. Each side is therefore normalized through the type
//! the registry's own loader parses that entry into, which collapses field
//! order, TOML shape, and a stated default against an omitted one, and leaves
//! every field the loader reads. A target whose parsed
//! model is a validated projection rather than a serde mirror of the authored
//! table — `rules`, whose [`Rule`](crate::declarations::rules::Rule) derives
//! its scope from the assertion — has no such type; its authored values are
//! compared directly, which can report a stated default as a difference but
//! cannot pass over one.
//!
//! # What is not drift
//!
//! A contribution the repository declares no entry for is one copy, not two:
//! nothing can disagree with it, and this repository carries many such
//! contributions (it has never applied its own package, so its registries hold
//! only what it authored). Such a contribution is reported by nothing here.
//!
//! A declaration this repository deliberately holds differently from the
//! package is a [`DeclaredOverride`]: it states which contributions it covers
//! and why, and the caller supplies the set. An override is a declaration, not
//! a silence — a difference it does not cover is still reported.

use std::collections::{BTreeMap, BTreeSet};

use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

use crate::config::{ItemKindConfig, NamespaceConfig, ProjectionConfig};
use crate::declarations::invariants::Invariant;
use crate::declarations::GateDefinition;
use crate::profile::drift_report::{DriftCarrier, DriftReport, DriftSubject};
use crate::repository_state::{
    Contribution, KeyedArrayTarget, MapEntryTarget, ScalarTarget, SetStringTarget,
};
use crate::templates::GraphTemplate;

/// A packaged contribution that could not be compared against its repository
/// counterpart.
///
/// Every variant names the registry and the entry, since a comparison that
/// cannot run is a hole in the binding rather than agreement.
#[derive(Debug, thiserror::Error)]
pub enum ContributionDriftError {
    /// A registry a contribution targets was not supplied by the caller.
    #[error(
        "no text was supplied for registry '{registry}', which contribution '{entry}' targets"
    )]
    RegistryMissing {
        /// Repository-relative path of the registry.
        registry: String,
        /// The contribution's entry identity.
        entry: String,
    },
    /// A registry's text is not the TOML document its entries are read from.
    #[error("registry '{registry}' does not parse as TOML: {source}")]
    RegistryParse {
        /// Repository-relative path of the registry.
        registry: String,
        /// The underlying parse error.
        source: toml_edit::de::Error,
    },
    /// A registry's keyed array holds an entry without its identity field.
    #[error("registry '{registry}' holds a '{array}' entry without a '{field}' identity")]
    EntryIdentityMissing {
        /// Repository-relative path of the registry.
        registry: String,
        /// The array whose entry lacks an identity.
        array: String,
        /// The field carrying an entry's identity.
        field: String,
    },
    /// One side's value does not parse as the type the comparison normalizes
    /// through.
    #[error("the {side} '{entry}' declaration is not a {expected}: {source}")]
    NotComparable {
        /// Which carrier held the value.
        side: DriftSide,
        /// The entry identity.
        entry: String,
        /// The type the comparison normalizes through.
        expected: &'static str,
        /// The underlying deserialization error.
        source: serde_json::Error,
    },
}

/// Which carrier a value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftSide {
    /// The registry this repository authors.
    Repository,
    /// The manifest the package ships.
    Packaged,
}

impl std::fmt::Display for DriftSide {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Repository => "repository",
            Self::Packaged => "packaged",
        })
    }
}

/// The contributions one [`DeclaredOverride`] can cover, named by the target
/// each contribution declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverrideScope {
    /// Entries of one keyed-array registry.
    KeyedArray(KeyedArrayTarget),
    /// Entries of one `.jit/config.toml` table.
    MapEntry(MapEntryTarget),
    /// One `.jit/config.toml` scalar.
    Scalar(ScalarTarget),
    /// Members of one `.jit/config.toml` array.
    SetString(SetStringTarget),
    /// Declared projections.
    Projection,
}

/// One declaration this repository deliberately holds differently from the
/// packaged contribution that names it.
///
/// An override is narrow by construction: it names the package whose
/// contribution it covers, which contributions of that package it covers and,
/// unless it covers the whole entry, the single top-level field the two
/// carriers state differently. Every other field of the same entry stays bound,
/// so an override suppresses the difference it was declared for and nothing
/// else.
///
/// The package is named rather than defaulted because a reason is a reason
/// about one package. This repository publishes several, they contribute to the
/// same registries, and a reason that holds for one — a placeholder its adopter
/// is meant to replace, an example naming this repository's own issues — need
/// not hold for the next. An override that widened across packages by omission
/// would be suppressing a comparison on the strength of a reason that was never
/// asserted about it (`@/invariant/convention-convergence`). Two packages
/// needing the same field suppressed are therefore two entries, each carrying
/// the reason that holds for its own package.
#[derive(Debug, Clone, Copy)]
pub struct DeclaredOverride {
    /// The package whose contribution this override covers, by manifest id.
    pub package: &'static str,
    /// Which contributions the override covers.
    pub scope: OverrideScope,
    /// The entry it covers, or every entry of the scope when absent.
    pub identity: Option<&'static str>,
    /// The entry's top-level field it covers, or the whole entry when absent.
    pub field: Option<&'static str>,
    /// Why this repository holds its own value.
    pub reason: &'static str,
}

impl DeclaredOverride {
    /// Whether this override covers `contribution` as declared by `package`.
    fn covers(&self, package: &str, contribution: &Contribution) -> bool {
        self.package == package
            && self.scope == scope_of(contribution)
            && self
                .identity
                .is_none_or(|identity| identity == entry_identity(contribution))
    }

    /// Whether this override still has something to excuse in `contribution`
    /// as declared by `package`: it covers it, and the field it names — if it
    /// names one — is a field that contribution declares.
    pub fn applies_to(&self, package: &str, contribution: &Contribution) -> bool {
        self.covers(package, contribution)
            && self.field.is_none_or(|field| {
                contributed_value(contribution)
                    .get(field)
                    .is_some_and(|value| !value.is_null())
            })
    }
}

/// The scope a contribution falls in.
fn scope_of(contribution: &Contribution) -> OverrideScope {
    match contribution {
        Contribution::Scalar { target, .. } => OverrideScope::Scalar(*target),
        Contribution::MapEntry { target, .. } => OverrideScope::MapEntry(*target),
        Contribution::SetString { target, .. } => OverrideScope::SetString(*target),
        Contribution::KeyedArray { target, .. } => OverrideScope::KeyedArray(*target),
        Contribution::Projection { .. } => OverrideScope::Projection,
    }
}

/// How a contribution's entry is identified, in the spelling its registry uses.
///
/// A keyed-array, map-entry, or projection contribution carries its own
/// identity; a scalar or set-string contribution addresses a configuration key,
/// so the key path is what names it.
fn entry_identity(contribution: &Contribution) -> String {
    match contribution {
        Contribution::KeyedArray { identity, .. } | Contribution::MapEntry { identity, .. } => {
            identity.clone()
        }
        Contribution::Projection { name, .. } => name.clone(),
        Contribution::Scalar { target, .. } => {
            let (table, key) = target.config_path();
            format!("{table}.{key}")
        }
        Contribution::SetString { target, .. } => {
            let (table, key) = target.config_path();
            format!("{table}.{key}")
        }
    }
}

/// Every registry a contribution in `contributions` restates an entry of.
///
/// The caller reads these paths and hands their text to
/// [`contribution_drift_reports`], so which files the comparison consults comes
/// from the manifest rather than from a list beside it.
pub fn contributed_registry_paths(contributions: &[Contribution]) -> BTreeSet<&'static str> {
    contributions
        .iter()
        .map(Contribution::registry_path)
        .collect()
}

/// Repository-relative path of the manifest that declares package `id`.
///
/// Derived from the location this repository publishes its packages at, so a
/// carrier names the file a reader opens without any caller spelling the path
/// for a particular package.
pub fn packaged_manifest_path(id: &str) -> String {
    format!(
        "{}/{id}/{}",
        crate::test_utils::PROFILE_PACKAGE_SOURCES,
        crate::profile::MANIFEST_FILE_NAME
    )
}

/// Every contribution of `package` whose repository counterpart states
/// something else, one report per contribution.
///
/// `package` is the manifest id of the package declaring `contributions`; it
/// names the packaged carrier and selects which overrides apply. `registries`
/// maps each repository-relative registry path to its authored text —
/// [`contributed_registry_paths`] names the set that must be present.
/// `overrides` declares which pairs this repository deliberately holds apart,
/// and an entry naming a different package is not consulted.
///
/// # Errors
///
/// [`ContributionDriftError`] when a targeted registry is absent from
/// `registries`, when a registry does not parse, or when either side's value is
/// not the declaration its target's comparison type describes.
pub fn contribution_drift_reports(
    package: &str,
    contributions: &[Contribution],
    registries: &BTreeMap<String, String>,
    overrides: &[DeclaredOverride],
) -> Result<Vec<DriftReport>, ContributionDriftError> {
    let documents = parsed_registries(contributions, registries)?;
    let manifest = packaged_manifest_path(package);
    contributions
        .iter()
        .filter(|contribution| {
            !overrides
                .iter()
                .any(|declared| declared.field.is_none() && declared.covers(package, contribution))
        })
        .filter_map(|contribution| {
            contribution_report(package, &manifest, contribution, &documents, overrides).transpose()
        })
        .collect()
}

/// Each targeted registry's text parsed into the values its entries are read
/// from.
fn parsed_registries(
    contributions: &[Contribution],
    registries: &BTreeMap<String, String>,
) -> Result<BTreeMap<&'static str, Value>, ContributionDriftError> {
    contributions
        .iter()
        .map(Contribution::registry_path)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|registry| {
            let text = registries.get(registry).ok_or_else(|| {
                ContributionDriftError::RegistryMissing {
                    registry: registry.to_string(),
                    entry: contributions
                        .iter()
                        .find(|contribution| contribution.registry_path() == registry)
                        .map(entry_identity)
                        .unwrap_or_default(),
                }
            })?;
            let document = toml_edit::de::from_str::<Value>(text).map_err(|source| {
                ContributionDriftError::RegistryParse {
                    registry: registry.to_string(),
                    source,
                }
            })?;
            Ok((registry, document))
        })
        .collect()
}

/// The report one contribution produces, or `None` when it agrees with its
/// repository counterpart or the repository declares no counterpart.
fn contribution_report(
    package: &str,
    manifest: &str,
    contribution: &Contribution,
    documents: &BTreeMap<&'static str, Value>,
    overrides: &[DeclaredOverride],
) -> Result<Option<DriftReport>, ContributionDriftError> {
    let registry = contribution.registry_path();
    let identity = entry_identity(contribution);
    let document =
        documents
            .get(registry)
            .ok_or_else(|| ContributionDriftError::RegistryMissing {
                registry: registry.to_string(),
                entry: identity.clone(),
            })?;
    let subject = |entry: String| DriftSubject {
        repository: DriftCarrier::new(registry, entry),
        packaged: DriftCarrier::new(manifest, format!("contribution '{identity}'")),
        field_root: format!("{registry}[{identity}]"),
        remedy: format!(
            "the two carriers restate one declaration: edit {registry} and {manifest} \
             together, or declare the difference as an override with its reason"
        ),
    };

    // A set-string contribution contributes a member to an array rather than an
    // entry with fields, so what it can disagree about is membership.
    if let Contribution::SetString { target, value } = contribution {
        let (table, key) = target.config_path();
        let Some(members) = document.get(table).and_then(|table| table.get(key)) else {
            return Ok(None);
        };
        let carried = members
            .as_array()
            .is_some_and(|members| members.iter().any(|member| member == value));
        let missing = Vec::from_iter((!carried).then(|| {
            format!(
                "{table}.{key}: the repository's members do not carry {}, which the \
                 package contributes",
                Value::String(value.clone())
            )
        }));
        return Ok(subject(format!("{table}.{key}")).reporting(missing));
    }

    let Some((entry, authored)) = repository_entry(registry, document, contribution)? else {
        return Ok(None);
    };
    let packaged = contributed_value(contribution);
    let suppressed: BTreeSet<&str> = overrides
        .iter()
        .filter(|declared| declared.covers(package, contribution))
        .filter_map(|declared| declared.field)
        .collect();

    Ok(subject(entry).compare(
        &without(
            normalized(&authored, contribution, DriftSide::Repository)?,
            &suppressed,
        ),
        &without(
            normalized(&packaged, contribution, DriftSide::Packaged)?,
            &suppressed,
        ),
    ))
}

/// `value` without the top-level fields an override covers.
///
/// Dropping a covered field from both normalized values is what keeps an
/// override to the one field it declares: every other field of the entry is
/// compared as if the override were absent. The drop follows normalization
/// because a required field removed before it would leave the entry
/// unparseable rather than partly compared.
fn without(value: Value, fields: &BTreeSet<&str>) -> Value {
    match value {
        Value::Object(mut object) if !fields.is_empty() => {
            object.retain(|key, _| !fields.contains(key.as_str()));
            Value::Object(object)
        }
        value => value,
    }
}

/// The value the package contributes, as the manifest declares it.
pub fn contributed_value(contribution: &Contribution) -> Value {
    crate::repository_state::contributed_json_value(contribution)
        .expect("a contribution's declared value is the value its own model carries")
}

/// How the repository's registry names the entry a contribution restates, with
/// the value authored there, or `None` when the repository declares none.
fn repository_entry(
    registry: &str,
    document: &Value,
    contribution: &Contribution,
) -> Result<Option<(String, Value)>, ContributionDriftError> {
    Ok(match contribution {
        Contribution::KeyedArray {
            target, identity, ..
        } => {
            let field = target.identity_field();
            let array = target.array_name();
            document
                .get(array)
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .try_fold(None, |found, entry| {
                    // Every entry's identity is read, not only the sought one:
                    // a registry entry without one is a registry the lookup
                    // cannot be trusted over.
                    let actual = entry.get(field).and_then(Value::as_str).ok_or_else(|| {
                        ContributionDriftError::EntryIdentityMissing {
                            registry: registry.to_string(),
                            array: array.to_string(),
                            field: field.to_string(),
                        }
                    })?;
                    Ok((actual == identity)
                        .then(|| (format!("{array} entry '{identity}'"), entry.clone()))
                        .or(found))
                })?
        }
        Contribution::MapEntry {
            target, identity, ..
        } => target
            .table_path()
            .iter()
            .try_fold(document, |value, key| value.get(*key))
            .and_then(|table| table.get(identity))
            .map(|value| {
                let table = target.table_path().join(".");
                (format!("{table} entry '{identity}'"), value.clone())
            }),
        Contribution::Projection { name, .. } => document
            .get("projection")
            .and_then(|projections| projections.get(name))
            .map(|value| (format!("projection '{name}'"), value.clone())),
        Contribution::Scalar { target, .. } => {
            let (table, key) = target.config_path();
            document
                .get(table)
                .and_then(|table| table.get(key))
                .map(|value| (format!("{table}.{key}"), value.clone()))
        }
        Contribution::SetString { .. } => None,
    })
}

/// `value` normalized through the type the registry's own loader parses this
/// contribution's entry into, or unchanged when the target has no such type.
fn normalized(
    value: &Value,
    contribution: &Contribution,
    side: DriftSide,
) -> Result<Value, ContributionDriftError> {
    let entry = entry_identity(contribution);
    match comparison_type(contribution) {
        ComparisonType::Authored => Ok(value.clone()),
        ComparisonType::Gate => through::<GateDefinition>(value, side, &entry, "gate definition"),
        ComparisonType::Invariant => through::<Invariant>(value, side, &entry, "invariant entry"),
        ComparisonType::Template => through::<GraphTemplate>(value, side, &entry, "graph template"),
        ComparisonType::Namespace => {
            through::<NamespaceConfig>(value, side, &entry, "namespace declaration")
        }
        ComparisonType::ItemKind => {
            through::<ItemKindConfig>(value, side, &entry, "item-kind declaration")
        }
        ComparisonType::Projection => {
            through::<ProjectionConfig>(value, side, &entry, "projection configuration")
        }
    }
}

/// The type a contribution's two carriers are compared through.
///
/// Every target resolves here, so a target added to the contribution model has
/// to state which type its entries normalize through rather than falling
/// silently into one.
enum ComparisonType {
    /// No serde mirror of the authored entry exists: compare what is authored.
    Authored,
    Gate,
    Invariant,
    Template,
    Namespace,
    ItemKind,
    Projection,
}

/// Which comparison type a contribution's target resolves to.
fn comparison_type(contribution: &Contribution) -> ComparisonType {
    match contribution {
        Contribution::KeyedArray { target, .. } => match target {
            KeyedArrayTarget::Gates => ComparisonType::Gate,
            KeyedArrayTarget::Invariants => ComparisonType::Invariant,
            KeyedArrayTarget::Templates => ComparisonType::Template,
            // `Rule` is a validated projection of the authored table rather
            // than a serde mirror of it, so there is no type to normalize
            // through.
            KeyedArrayTarget::Rules => ComparisonType::Authored,
        },
        Contribution::MapEntry { target, .. } => match target {
            MapEntryTarget::Namespaces => ComparisonType::Namespace,
            MapEntryTarget::ItemKinds => ComparisonType::ItemKind,
            // A hierarchy level is a number and a label association is a
            // string; neither has fields to normalize.
            MapEntryTarget::TypeHierarchyTypes | MapEntryTarget::LabelAssociations => {
                ComparisonType::Authored
            }
        },
        Contribution::Projection { .. } => ComparisonType::Projection,
        Contribution::Scalar { .. } | Contribution::SetString { .. } => ComparisonType::Authored,
    }
}

/// `value` round-tripped through `T`.
fn through<T: DeserializeOwned + Serialize>(
    value: &Value,
    side: DriftSide,
    entry: &str,
    expected: &'static str,
) -> Result<Value, ContributionDriftError> {
    let typed: T = serde_json::from_value(value.clone()).map_err(|source| {
        ContributionDriftError::NotComparable {
            side,
            entry: entry.to_string(),
            expected,
            source,
        }
    })?;
    serde_json::to_value(typed).map_err(|source| ContributionDriftError::NotComparable {
        side,
        entry: entry.to_string(),
        expected,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The registries a set of contributions targets, each holding `text`.
    fn registries(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(path, text)| ((*path).to_string(), (*text).to_string()))
            .collect()
    }

    fn invariant_contribution(statement: &str) -> Contribution {
        Contribution::KeyedArray {
            target: KeyedArrayTarget::Invariants,
            identity: "a-property".to_string(),
            value: json!({
                "id": "a-property",
                "statement": statement,
                "kind": "advisory",
            }),
        }
    }

    const REPOSITORY_INVARIANTS: &str = "\
[[invariants]]
id = \"a-property\"
statement = \"the ruling this repository recorded\"
kind = \"advisory\"
";

    /// The package id every case below declares its contributions under.
    const PACKAGE: &str = "a-package";

    fn reports(
        contributions: &[Contribution],
        registries: &BTreeMap<String, String>,
        overrides: &[DeclaredOverride],
    ) -> Vec<DriftReport> {
        contribution_drift_reports(PACKAGE, contributions, registries, overrides)
            .expect("the comparison runs")
    }

    /// A contribution restating the repository's entry verbatim is not drift.
    #[test]
    fn test_contribution_drift_reports_is_silent_when_the_two_carriers_restate_each_other() {
        let reported = reports(
            &[invariant_contribution(
                "the ruling this repository recorded",
            )],
            &registries(&[(".jit/invariants.toml", REPOSITORY_INVARIANTS)]),
            &[],
        );

        assert!(reported.is_empty(), "{reported:?}");
    }

    /// A contribution whose text left the repository's behind is reported,
    /// naming both carriers and both texts.
    #[test]
    fn test_contribution_drift_reports_names_both_carriers_and_the_differing_text() {
        let reported = reports(
            &[invariant_contribution(
                "a statement the package amended alone",
            )],
            &registries(&[(".jit/invariants.toml", REPOSITORY_INVARIANTS)]),
            &[],
        );

        assert_eq!(reported.len(), 1);
        let rendered = reported[0].to_string();
        assert_eq!(reported[0].repository().path, ".jit/invariants.toml");
        assert_eq!(reported[0].packaged().path, packaged_manifest_path(PACKAGE));
        for named in [
            "a-property",
            "the ruling this repository recorded",
            "a statement the package amended alone",
        ] {
            assert!(rendered.contains(named), "{rendered}");
        }
    }

    /// A contribution the repository declares no entry for is one copy, not
    /// two, so nothing can disagree with it.
    #[test]
    fn test_contribution_drift_reports_passes_over_a_contribution_the_registry_does_not_declare() {
        let reported = reports(
            &[Contribution::KeyedArray {
                target: KeyedArrayTarget::Invariants,
                identity: "a-property-the-repository-omits".to_string(),
                value: json!({
                    "id": "a-property-the-repository-omits",
                    "statement": "s",
                    "kind": "advisory",
                }),
            }],
            &registries(&[(".jit/invariants.toml", REPOSITORY_INVARIANTS)]),
            &[],
        );

        assert!(reported.is_empty(), "{reported:?}");
    }

    /// Normalization collapses a difference in spelling: the repository states
    /// a field the package leaves to its default, and the entries still agree.
    #[test]
    fn test_contribution_drift_reports_passes_over_a_stated_default_the_package_omits() {
        let contribution = Contribution::KeyedArray {
            target: KeyedArrayTarget::Templates,
            identity: "plan".to_string(),
            value: json!({"name": "plan", "applies_to": ["epic"]}),
        };
        let stated_defaults = "\
[[template]]
name = \"plan\"
applies_to = [\"epic\"]
anchors = []
nodes = []
anchor_edges = []
transforms = []
";

        assert!(reports(
            &[contribution],
            &registries(&[(".jit/templates.toml", stated_defaults)]),
            &[],
        )
        .is_empty());
    }

    /// A difference in meaning survives normalization.
    #[test]
    fn test_contribution_drift_reports_reports_a_difference_normalization_cannot_collapse() {
        let contribution = Contribution::KeyedArray {
            target: KeyedArrayTarget::Templates,
            identity: "plan".to_string(),
            value: json!({"name": "plan", "applies_to": ["epic"]}),
        };
        let widened = "\
[[template]]
name = \"plan\"
applies_to = [\"epic\", \"milestone\"]
";

        let reported = reports(
            &[contribution],
            &registries(&[(".jit/templates.toml", widened)]),
            &[],
        );
        assert_eq!(reported.len(), 1);
        assert!(reported[0].to_string().contains("milestone"));
    }

    /// A map-entry contribution is bound to the configuration table its target
    /// names.
    #[test]
    fn test_contribution_drift_reports_binds_a_map_entry_to_its_configuration_table() {
        let contribution = Contribution::MapEntry {
            target: MapEntryTarget::Namespaces,
            identity: "satisfies".to_string(),
            value: json!({"description": "the packaged wording", "unique": false}),
        };
        let config = "\
[namespaces.satisfies]
description = \"the repository's wording\"
unique = false
";

        let reported = reports(
            &[contribution],
            &registries(&[(".jit/config.toml", config)]),
            &[],
        );
        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0].repository().path, ".jit/config.toml");
        assert!(reported[0].repository().entry.contains("satisfies"));
    }

    /// A projection contribution is bound to the projection the repository
    /// declares under the same name.
    #[test]
    fn test_contribution_drift_reports_binds_a_projection_to_the_declared_projection() {
        let contribution = Contribution::Projection {
            name: "invariants".to_string(),
            value: serde_json::from_value(json!({
                "kind": "invariant",
                "mode": "region",
                "target": "AGENTS.md",
                "style": "id-anchor",
            }))
            .unwrap(),
        };
        let config = "\
[projection.invariants]
kind = \"invariant\"
mode = \"region\"
target = \"CONTRIBUTING.md\"
style = \"id-anchor\"
";

        let reported = reports(
            &[contribution],
            &registries(&[(".jit/config.toml", config)]),
            &[],
        );
        assert_eq!(reported.len(), 1);
        assert!(reported[0].to_string().contains("CONTRIBUTING.md"));
    }

    /// A scalar contribution is bound to the configuration key its target
    /// names.
    #[test]
    fn test_contribution_drift_reports_binds_a_scalar_to_its_configuration_key() {
        let contribution = Contribution::Scalar {
            target: ScalarTarget::ValidationDefaultType,
            value: "task".to_string(),
        };
        let config = "[validation]\ndefault_type = \"story\"\n";

        let reported = reports(
            &[contribution],
            &registries(&[(".jit/config.toml", config)]),
            &[],
        );
        assert_eq!(reported.len(), 1);
        assert!(reported[0].to_string().contains("story"));
    }

    /// A set-string contribution disagrees about membership: the repository's
    /// array not carrying the contributed member is what is reported.
    #[test]
    fn test_contribution_drift_reports_reports_a_set_member_the_repository_array_omits() {
        let contribution = Contribution::SetString {
            target: SetStringTarget::StrategicTypes,
            value: "epic".to_string(),
        };
        let without_member = "[type_hierarchy]\nstrategic_types = [\"milestone\"]\n";
        let with_member = "[type_hierarchy]\nstrategic_types = [\"milestone\", \"epic\"]\n";

        let reported = reports(
            std::slice::from_ref(&contribution),
            &registries(&[(".jit/config.toml", without_member)]),
            &[],
        );
        assert_eq!(reported.len(), 1);
        assert!(reported[0].to_string().contains("epic"));

        assert!(reports(
            &[contribution],
            &registries(&[(".jit/config.toml", with_member)]),
            &[],
        )
        .is_empty());
    }

    /// A field-scoped override suppresses the field it names and leaves every
    /// other field of the same entry bound.
    #[test]
    fn test_contribution_drift_reports_suppresses_only_the_field_an_override_names() {
        let contribution = Contribution::KeyedArray {
            target: KeyedArrayTarget::Invariants,
            identity: "a-property".to_string(),
            value: json!({
                "id": "a-property",
                "statement": "the ruling this repository recorded",
                "kind": "advisory",
                "enforced-by": "@/gate/portable",
            }),
        };
        let repository = "\
[[invariants]]
id = \"a-property\"
statement = \"the ruling this repository recorded\"
kind = \"advisory\"
enforced-by = \"@/gate/checkout-only\"
";
        let binding = DeclaredOverride {
            package: PACKAGE,
            scope: OverrideScope::KeyedArray(KeyedArrayTarget::Invariants),
            identity: None,
            field: Some("enforced-by"),
            reason: "the checkout's gate is not package content",
        };

        assert!(reports(
            std::slice::from_ref(&contribution),
            &registries(&[(".jit/invariants.toml", repository)]),
            &[binding],
        )
        .is_empty());

        // The same override leaves the statement bound.
        let amended = "\
[[invariants]]
id = \"a-property\"
statement = \"a ruling the package never recorded\"
kind = \"advisory\"
enforced-by = \"@/gate/checkout-only\"
";
        let reported = reports(
            &[contribution],
            &registries(&[(".jit/invariants.toml", amended)]),
            &[binding],
        );
        assert_eq!(reported.len(), 1);
        assert!(reported[0]
            .to_string()
            .contains("a ruling the package never recorded"));
    }

    /// An entry-scoped override covers the entry it names and no other.
    #[test]
    fn test_contribution_drift_reports_suppresses_only_the_entry_an_override_names() {
        let contributions = [
            invariant_contribution("a statement the package amended alone"),
            Contribution::KeyedArray {
                target: KeyedArrayTarget::Invariants,
                identity: "another-property".to_string(),
                value: json!({
                    "id": "another-property",
                    "statement": "a second amended statement",
                    "kind": "advisory",
                }),
            },
        ];
        let repository = "\
[[invariants]]
id = \"a-property\"
statement = \"the ruling this repository recorded\"
kind = \"advisory\"

[[invariants]]
id = \"another-property\"
statement = \"a second repository ruling\"
kind = \"advisory\"
";
        let reported = reports(
            &contributions,
            &registries(&[(".jit/invariants.toml", repository)]),
            &[DeclaredOverride {
                package: PACKAGE,
                scope: OverrideScope::KeyedArray(KeyedArrayTarget::Invariants),
                identity: Some("a-property"),
                field: None,
                reason: "the repository authored this entry independently",
            }],
        );

        assert_eq!(reported.len(), 1);
        assert!(reported[0].repository().entry.contains("another-property"));
    }

    /// A registry the contributions target and the caller did not supply is an
    /// error, not silent agreement.
    #[test]
    fn test_contribution_drift_reports_errors_when_a_targeted_registry_is_unavailable() {
        let error = contribution_drift_reports(
            PACKAGE,
            &[invariant_contribution("s")],
            &BTreeMap::new(),
            &[],
        )
        .expect_err("an unavailable registry cannot be compared");

        assert!(matches!(
            error,
            ContributionDriftError::RegistryMissing { ref registry, .. }
                if registry == ".jit/invariants.toml"
        ));
    }

    /// A value that is not the declaration its target describes is an error,
    /// not silent agreement.
    #[test]
    fn test_contribution_drift_reports_errors_when_a_carrier_value_is_not_comparable() {
        let error = contribution_drift_reports(
            PACKAGE,
            &[Contribution::KeyedArray {
                target: KeyedArrayTarget::Invariants,
                identity: "a-property".to_string(),
                value: json!({"id": "a-property", "statement": "s", "kind": "not-a-kind"}),
            }],
            &registries(&[(".jit/invariants.toml", REPOSITORY_INVARIANTS)]),
            &[],
        )
        .expect_err("an uninterpretable value cannot be compared");

        assert!(matches!(
            error,
            ContributionDriftError::NotComparable {
                side: DriftSide::Packaged,
                ..
            }
        ));
    }

    /// The registries the comparison consults come from the contributions.
    #[test]
    fn test_contributed_registry_paths_names_every_registry_the_contributions_target() {
        let paths = contributed_registry_paths(&[
            invariant_contribution("s"),
            Contribution::MapEntry {
                target: MapEntryTarget::Namespaces,
                identity: "satisfies".to_string(),
                value: json!({"description": "d", "unique": false}),
            },
        ]);

        assert_eq!(
            paths,
            BTreeSet::from([".jit/invariants.toml", ".jit/config.toml"])
        );
    }
}
