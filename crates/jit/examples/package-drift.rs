//! Check this repository's own registries against what its packages declare.
//!
//! A repository-local check run by a configured gate: it reads `.jit/*.toml`,
//! whose contents the Rust test suite must not depend on. Every run also seeds
//! drift and requires a report, so a comparison reaching nothing cannot pass.
//!
//! Usage:
//!   package-drift [ROOT]    (ROOT holds the `.jit/` to compare; default: the
//!                            checkout these sources were compiled from)
//!
//! Exit codes:
//!   0 — every registry agrees with the packages and every gate input holds
//!   1 — findings, one per line on stderr
//!   2 — a usage error

use jit::declarations::{parse_gate_registry, GateDefinition};
use jit::domain::repository_inputs::RepositoryInputs;
use jit::profile::contribution_drift::{DeclaredOverride, OverrideScope};
use jit::profile::drift_report::DriftReport;
use jit::profile::template_region::{
    inside_template_region, outside_template_region, packaged_templates, render_template_block,
    render_template_registry, splice_template_region, template_drift_report,
    TEMPLATE_REGION_GENERATOR, TEMPLATE_REGION_PACKAGE_ID, TEMPLATE_REGISTRY_PATH,
};
use jit::profile::ProfilePackage;
use jit::repository_state::{Contribution, KeyedArrayTarget, MapEntryTarget, VirtualPath};
use jit::templates::{GraphTemplate, TemplateRegistry};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tempfile::TempDir;

/// The package the seeded repository edit is reported against.
const PACKAGE_ID: &str = "jit-dogfood";

/// The gates whose footprint is the whole tree.
const TREE_WIDE_GATES: [&str; 4] = ["cargo-ci", "cargo-ci-features", "tests", "docs-mechanical"];

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let root = match arguments.as_slice() {
        [] => Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        [root] if Path::new(root).is_dir() => PathBuf::from(root),
        _ => {
            eprintln!("usage: package-drift [ROOT]");
            return ExitCode::from(2);
        }
    };
    let (_workspaces, packages) = published_packages();
    let findings = [
        contribution_findings(&packages, &root),
        template_findings(&packages, &root),
        gate_input_findings(&root),
    ]
    .concat();
    findings.iter().for_each(|finding| eprintln!("{finding}\n"));
    if findings.is_empty() {
        println!("OK: the registries agree with the packages, and gate inputs hold");
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// `message` unless `holds`.
fn unless(holds: bool, message: impl FnOnce() -> String) -> Option<String> {
    (!holds).then(message)
}

/// The declarations this repository deliberately holds differently from
/// the packaged contribution that names them.
///
/// Every other contribution is bound: a packaged value and the registry
/// entry it restates are two copies of one declaration, and an edit to
/// either alone is reported. An entry here says the two are not copies of
/// one declaration at all, and why. Each is as narrow as its reason —
/// unless the whole entry is this repository's own, only the single field
/// the two carriers state differently is named, and every remaining field
/// of that entry stays bound.
const DECLARED_OVERRIDES: &[DeclaredOverride] = &[
    DeclaredOverride {
        package: PACKAGE_ID,
        scope: OverrideScope::KeyedArray(KeyedArrayTarget::Gates),
        identity: None,
        field: Some("checker"),
        reason: "The package ships every review gate as a portable placeholder an \
                 adopter replaces without changing the workflow graph, which the \
                 manifest declares above its gate contributions. This checkout is \
                 such an adopter: each of its gates runs a real command.",
    },
    DeclaredOverride {
        package: PACKAGE_ID,
        scope: OverrideScope::KeyedArray(KeyedArrayTarget::Gates),
        identity: None,
        field: Some("title"),
        reason: "A replaced checker is titled for the command it runs rather than \
                 for the placeholder it replaced.",
    },
    DeclaredOverride {
        package: PACKAGE_ID,
        scope: OverrideScope::KeyedArray(KeyedArrayTarget::Gates),
        identity: None,
        field: Some("description"),
        reason: "A replaced checker is described by what it runs; the packaged \
                 description is bound instead to the reference the package \
                 installs, which is rendered from the same contributions.",
    },
    DeclaredOverride {
        package: PACKAGE_ID,
        scope: OverrideScope::KeyedArray(KeyedArrayTarget::Invariants),
        identity: None,
        field: Some("enforced-by"),
        reason: "The checkout-only cargo-ci gate is not package content, so the \
                 packaged enforcement claims bind to the portable repo-validate \
                 gate instead, which the manifest declares above its invariant \
                 contributions. The statement each entry carries stays bound.",
    },
    DeclaredOverride {
        package: PACKAGE_ID,
        scope: OverrideScope::KeyedArray(KeyedArrayTarget::Rules),
        identity: None,
        field: Some("origin"),
        reason: "A rule's origin records which declaration authored it, so a rule \
                 this repository authored before the package carries its own \
                 provenance marker rather than the package's.",
    },
    DeclaredOverride {
        package: PACKAGE_ID,
        scope: OverrideScope::KeyedArray(KeyedArrayTarget::Rules),
        identity: None,
        field: Some("description"),
        reason: "Each carrier describes the rule for its own reader; the packaged \
                 description is bound instead to the reference the package \
                 installs, which is rendered from the same contributions. What the \
                 rule asserts stays bound.",
    },
    DeclaredOverride {
        package: PACKAGE_ID,
        scope: OverrideScope::MapEntry(MapEntryTarget::Namespaces),
        identity: None,
        field: Some("examples"),
        reason: "Namespace examples are documentation and never enforced, so each \
                 carrier names its own vocabulary: this repository's examples cite \
                 its own issues, which an adopter has no counterpart for.",
    },
    DeclaredOverride {
        package: PACKAGE_ID,
        scope: OverrideScope::Projection,
        identity: Some("rules-and-gates"),
        field: None,
        reason: "The two are different projections of one registry rather than one \
                 projection declared twice: this repository renders the reference \
                 into a region of its published documentation, and the package \
                 installs it as a separate file under .jit/reference/.",
    },
];

/// Every package this repository publishes, assembled from the checkout, with
/// the directories holding the assembled trees.
fn published_packages() -> (Vec<TempDir>, Vec<(String, ProfilePackage)>) {
    jit::test_utils::published_package_ids()
        .into_iter()
        .map(|id| {
            let (workspace, package) = jit::test_utils::temporary_repository_package(&id);
            (workspace, (id, package))
        })
        .unzip()
}

/// The registries under `root`, keyed by the repository-relative path the
/// contributions target.
///
/// Panics when a targeted registry cannot be read: a registry the comparison
/// cannot consult is a contribution left unbound.
fn contributed_registries(contributions: &[Contribution], root: &Path) -> BTreeMap<String, String> {
    jit::profile::contribution_drift::contributed_registry_paths(contributions)
        .into_iter()
        .map(|registry| {
            let text = std::fs::read_to_string(root.join(&registry))
                .unwrap_or_else(|error| panic!("failed to read {registry}: {error}"));
            (registry.to_string(), text)
        })
        .collect()
}

fn contribution_drift(
    id: &str,
    package: &ProfilePackage,
    registries: &BTreeMap<String, String>,
) -> Vec<DriftReport> {
    jit::profile::contribution_drift::contribution_drift_reports(
        id,
        &package.model().contributions,
        registries,
        DECLARED_OVERRIDES,
    )
    .expect("every contributed registry entry is comparable")
}

fn contribution_findings(packages: &[(String, ProfilePackage)], root: &Path) -> Vec<String> {
    let drift = packages.iter().flat_map(|(id, package)| {
        let registries = contributed_registries(&package.model().contributions, root);
        contribution_drift(id, package, &registries)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    });
    let uncompared = packages
        .iter()
        .filter(|(_, package)| {
            contributed_registries(&package.model().contributions, root).is_empty()
        })
        .map(|(id, _)| format!("package {id} contributes to no registry this walk compares"));
    let stale_overrides = DECLARED_OVERRIDES
        .iter()
        .filter(|declared| {
            !packages.iter().any(|(id, package)| {
                package
                    .model()
                    .contributions
                    .iter()
                    .any(|contribution| declared.applies_to(id, contribution))
            })
        })
        .map(|declared| {
            format!(
                "package {} no longer declares what this override excuses: {}",
                declared.package, declared.reason
            )
        });
    drift
        .chain(uncompared)
        .chain(stale_overrides)
        .chain(unbound_targets(packages, root))
        .chain(seeded_repository_edit_finding(packages, root))
        .collect()
}

/// Every target a manifest contributes to is bound: an edit to the packaged
/// declaration alone is reported for each. The seed is a field of the
/// contribution itself, so no target is bound by an expectation stated here.
fn unbound_targets(packages: &[(String, ProfilePackage)], root: &Path) -> Vec<String> {
    let declared: BTreeSet<(String, String)> = packages
        .iter()
        .flat_map(|(id, package)| {
            package
                .model()
                .contributions
                .iter()
                .map(|contribution| (id.clone(), declared_target(contribution)))
        })
        .collect();
    let bound: BTreeSet<(String, String)> = packages
        .iter()
        .flat_map(|(id, package)| {
            let contributions = &package.model().contributions;
            let registries = contributed_registries(contributions, root);
            contributions
                .iter()
                .filter(|contribution| {
                    seeded_packaged_edits(contribution)
                        .iter()
                        .any(|seeded| seeded_drift(id, seeded, &registries).len() == 1)
                })
                .map(|contribution| (id.clone(), declared_target(contribution)))
                .collect::<Vec<_>>()
        })
        .collect();
    declared
        .difference(&bound)
        .map(|(id, target)| {
            format!("a seeded edit to package {id}'s {target} contribution went unreported")
        })
        .collect()
}

/// An edit to the repository's registry alone is reported, naming both carriers
/// and the differing text.
fn seeded_repository_edit_finding(
    packages: &[(String, ProfilePackage)],
    root: &Path,
) -> Option<String> {
    let (_, package) = packages.iter().find(|(id, _)| id == PACKAGE_ID)?;
    let registry = VirtualPath::INVARIANTS.repository_relative();
    let mut registries = contributed_registries(&package.model().contributions, root);
    let committed = registries.get(&registry)?.clone();
    registries.insert(
        registry.clone(),
        seeded_invariant_statement(&committed, "domain-agnostic"),
    );
    let reported = contribution_drift(PACKAGE_ID, package, &registries);
    let names_both_carriers = reported.iter().any(|report| {
        let rendered = report.to_string();
        report.repository().path == registry
            && report.packaged().path
                == jit::profile::contribution_drift::packaged_manifest_path(PACKAGE_ID)
            && rendered.contains(SEEDED_EDIT)
            && [report.repository(), report.packaged()]
                .iter()
                .all(|carrier| {
                    rendered.contains(&carrier.path) && rendered.contains(&carrier.entry)
                })
    });
    unless(names_both_carriers, || {
        format!("a seeded edit to {registry} went unreported: {reported:?}")
    })
}

/// The text a seeded edit writes into a declaration.
const SEEDED_EDIT: &str = "a value the package does not carry";

/// The target a contribution declares, spelled the way the manifest spells
/// it, so the walk above states its coverage in the manifest's vocabulary.
fn declared_target(contribution: &Contribution) -> String {
    match serde_json::to_value(contribution) {
        Ok(serde_json::Value::Object(fields)) => fields
            .get("target")
            .or_else(|| fields.get("kind"))
            .and_then(|target| target.as_str().map(str::to_string)),
        _ => None,
    }
    .expect("a contribution declares its target")
}

/// The reports one edited contribution produces against `registries`.
///
/// A seed that leaves the declaration uninterpretable — a token that is not
/// one of a closed vocabulary, say — reports nothing here and the walk
/// above tries the next field.
fn seeded_drift(
    package: &str,
    contribution: &Contribution,
    registries: &BTreeMap<String, String>,
) -> Vec<DriftReport> {
    jit::profile::contribution_drift::contribution_drift_reports(
        package,
        std::slice::from_ref(contribution),
        registries,
        DECLARED_OVERRIDES,
    )
    .unwrap_or_default()
}

/// Every way this walk knows to edit one packaged declaration away from the
/// repository's, one edited contribution per field it can change.
///
/// The field values are the contribution's own, changed in place and in
/// kind, so a declaration of any shape yields candidates without this
/// helper knowing what its fields mean.
fn seeded_packaged_edits(contribution: &Contribution) -> Vec<Contribution> {
    let value = jit::profile::contribution_drift::contributed_value(contribution);
    let candidates: Vec<serde_json::Value> = match &value {
        serde_json::Value::Object(fields) => fields
            .iter()
            .filter_map(|(key, field)| {
                let mut edited = fields.clone();
                edited.insert(key.clone(), seeded_value(field)?);
                Some(serde_json::Value::Object(edited))
            })
            .collect(),
        scalar => seeded_value(scalar).into_iter().collect(),
    };
    candidates
        .into_iter()
        .filter_map(|value| reseated(contribution, value))
        .collect()
}

/// `value` changed in place and in kind, or `None` when this walk has no
/// change for its shape.
fn seeded_value(value: &serde_json::Value) -> Option<serde_json::Value> {
    use serde_json::Value;
    match value {
        Value::String(_) => Some(Value::String(SEEDED_EDIT.to_string())),
        Value::Bool(carried) => Some(Value::Bool(!carried)),
        Value::Number(carried) => carried
            .as_i64()
            .map(|carried| Value::Number((carried + 1).into())),
        Value::Array(carried) => Some(Value::Array(
            [
                carried.clone(),
                vec![Value::String(SEEDED_EDIT.to_string())],
            ]
            .concat(),
        )),
        Value::Object(_) | Value::Null => None,
    }
}

/// `contribution` carrying `value` instead of its own.
///
/// A projection's value is a complete configuration rather than a free
/// value, so a seed that is not one is no candidate.
fn reseated(contribution: &Contribution, value: serde_json::Value) -> Option<Contribution> {
    Some(match contribution {
        Contribution::KeyedArray {
            target, identity, ..
        } => Contribution::KeyedArray {
            target: *target,
            identity: identity.clone(),
            value,
        },
        Contribution::MapEntry {
            target, identity, ..
        } => Contribution::MapEntry {
            target: *target,
            identity: identity.clone(),
            value,
        },
        Contribution::Projection { name, .. } => Contribution::Projection {
            name: name.clone(),
            value: serde_json::from_value(value).ok()?,
        },
        Contribution::Scalar { target, .. } => Contribution::Scalar {
            target: *target,
            value: value.as_str()?.to_string(),
        },
        Contribution::SetString { target, .. } => Contribution::SetString {
            target: *target,
            value: value.as_str()?.to_string(),
        },
    })
}

/// `registry` with the statement of invariant `identity` rewritten.
fn seeded_invariant_statement(registry: &str, identity: &str) -> String {
    let mut document = registry
        .parse::<toml_edit::DocumentMut>()
        .expect("the invariant registry parses");
    let entry = document["invariants"]
        .as_array_of_tables_mut()
        .expect("the registry declares an invariant array")
        .iter_mut()
        .find(|entry| entry.get("id").and_then(toml_edit::Item::as_str) == Some(identity))
        .unwrap_or_else(|| panic!("the repository declares invariant '{identity}'"));
    entry["statement"] = toml_edit::value(SEEDED_EDIT);
    document.to_string()
}

/// The declarations a template registry's text carries, as the registry
/// loader parses them. The type check is left to the repository's own
/// config load: what is compared here is the declaration, not the hierarchy
/// it names.
fn template_declarations_of(registry: &[u8]) -> Vec<GraphTemplate> {
    let unchecked_hierarchy: [&str; 0] = [];
    TemplateRegistry::from_toml_str(std::str::from_utf8(registry).unwrap(), &unchecked_hierarchy)
        .unwrap()
        .templates
}

/// The declarations a registry's generated region carries: the extent the
/// package is the authority for (`@/issue/e204e63d/decision/D-1`), which is
/// what the drift guard compares.
fn region_declarations_of(registry: &[u8]) -> Vec<GraphTemplate> {
    template_declarations_of(inside_template_region(registry).unwrap().as_bytes())
}

fn template_findings(packages: &[(String, ProfilePackage)], root: &Path) -> Vec<String> {
    let (_, package) = packages
        .iter()
        .find(|(id, _)| id == TEMPLATE_REGION_PACKAGE_ID)
        .expect("the template region's package is published");
    let registry_path = TEMPLATE_REGISTRY_PATH.repository_relative();
    let existing = std::fs::read(root.join(&registry_path))
        .unwrap_or_else(|error| panic!("failed to read {registry_path}: {error}"));
    let packaged = packaged_templates(package).expect("the packaged templates parse");
    let repository = region_declarations_of(&existing);
    let block = render_template_block(&packaged).expect("the packaged templates render");
    let rendered = render_template_registry(&existing, package).expect("the registry renders");
    let stale = splice_template_region(&existing, "# a declaration the package does not make\n")
        .expect("the registry holds its region");

    let carried = unless(rendered == existing, || {
        format!(
            "{registry_path} no longer carries the packaged template declarations. The block              between the region delimiters is generated from              profiles/{TEMPLATE_REGION_PACKAGE_ID}/manifest.toml: edit the packaged declaration,              then regenerate with {TEMPLATE_REGION_GENERATOR}"
        )
    });
    let drift = template_drift_report(&repository, &packaged)
        .expect("the declarations compare")
        .map(|report| report.to_string());
    let owns_region_only = unless(
        outside_template_region(&rendered).ok() == outside_template_region(&existing).ok()
            && outside_template_region(&stale).ok() == outside_template_region(&existing).ok(),
        || "rendering changed registry bytes outside the region delimiters".to_string(),
    );
    let restores = unless(
        splice_template_region(&stale, &block).ok().as_ref() == Some(&rendered)
            && splice_template_region(&rendered, &block).ok().as_ref() == Some(&rendered),
        || "rendering over a diverged region did not restore the packaged block".to_string(),
    );
    [
        carried,
        drift,
        owns_region_only,
        restores,
        still_brackets(&existing, &rendered),
        seeded_repository_template_finding(&existing, &packaged),
        seeded_packaged_template_finding(&repository, &packaged),
        outside_region_template_finding(&existing, &packaged),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Every container type a declaration brackets is still bracketed by it.
fn still_brackets(existing: &[u8], rendered: &[u8]) -> Option<String> {
    let load = |registry: &[u8]| {
        let unchecked_hierarchy: [&str; 0] = [];
        TemplateRegistry::from_toml_str(
            std::str::from_utf8(registry).expect("the registry is UTF-8"),
            &unchecked_hierarchy,
        )
        .expect("the registry loads")
    };
    let (before, after) = (load(existing), load(rendered));
    before.templates.iter().find_map(|template| {
        template.applies_to.iter().find_map(|container_type| {
            let applied = after
                .template_for_container(container_type)
                .map(|applied| applied.name.as_str());
            unless(applied == Some(template.name.as_str()), || {
                format!("the rendered registry stops bracketing container type '{container_type}'")
            })
        })
    })
}

/// A hand edit to the generated region is reported, naming the changed field.
fn seeded_repository_template_finding(
    existing: &[u8],
    packaged: &[GraphTemplate],
) -> Option<String> {
    let mut edited = region_declarations_of(existing);
    let node = edited
        .first_mut()
        .and_then(|template| template.nodes.first_mut())?;
    node.description = Some(format!(
        "{} A sentence the package does not carry.",
        node.description.as_deref().unwrap_or_default()
    ));
    let hand_edited =
        splice_template_region(existing, &render_template_block(&edited).ok()?).ok()?;
    let report = template_drift_report(&region_declarations_of(&hand_edited), packaged)
        .ok()?
        .map(|report| report.to_string())
        .unwrap_or_default();
    unless(
        [
            "template[0].nodes[0].description",
            TEMPLATE_REGION_GENERATOR,
        ]
        .iter()
        .all(|expected| report.contains(expected)),
        || format!("a seeded edit to the template region went unreported: {report}"),
    )
}

/// An edit to the packaged declaration alone is reported, naming the changed
/// field.
fn seeded_packaged_template_finding(
    repository: &[GraphTemplate],
    packaged: &[GraphTemplate],
) -> Option<String> {
    let mut edited = packaged.to_vec();
    let anchor = edited
        .first_mut()
        .and_then(|template| template.anchors.first_mut())?;
    anchor
        .gates
        .push("a-gate-the-repository-omits".parse().ok()?);
    let appended = anchor.gates.len() - 1;
    let report = template_drift_report(repository, &edited)
        .ok()?
        .map(|report| report.to_string())
        .unwrap_or_default();
    unless(
        report.contains(&format!("template[0].anchors[0].gates[{appended}]"))
            && report.contains(TEMPLATE_REGION_GENERATOR),
        || format!("a seeded edit to the packaged template went unreported: {report}"),
    )
}

/// A template the repository authors outside the delimiters is not drift: the
/// package is the authority for the generated region, not the whole registry.
fn outside_region_template_finding(existing: &[u8], packaged: &[GraphTemplate]) -> Option<String> {
    let mut authored = packaged.to_vec();
    authored.first_mut()?.name = "authored-outside-the-region".to_string();
    let registry = [
        existing,
        b"\n",
        render_template_block(&authored).ok()?.as_bytes(),
    ]
    .concat();
    template_drift_report(&region_declarations_of(&registry), packaged)
        .ok()?
        .filter(|_| region_declarations_of(existing) == region_declarations_of(&registry))
        .map(|report| format!("a template authored outside the region was drift:\n{report}"))
}

/// A wrong input footprint makes a reused gate verdict silently stale, and a
/// wrong one still parses, so this repository's footprints are held here.
fn gate_input_findings(root: &Path) -> Vec<String> {
    let path = VirtualPath::GATES.repository_relative();
    let bytes = std::fs::read(root.join(&path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"));
    let registry = match parse_gate_registry(&bytes) {
        Ok(registry) => registry,
        Err(error) => return vec![format!("{path} does not parse: {error}")],
    };
    let gate = |key: &str| -> Option<&GateDefinition> { registry.gates.get(&key.parse().ok()?) };
    let inputs = |key: &str| -> Option<&RepositoryInputs> { gate(key)?.inputs.as_ref() };
    let holds =
        |key: &str, check: &dyn Fn(&RepositoryInputs) -> bool| inputs(key).is_some_and(check);

    let covered = [
        ("cargo-ci", "crates/jit/src/lib.rs"),
        ("cargo-ci", ".config/nextest.toml"),
        ("npm-ci", "web/package-lock.json"),
        ("npm-ci", "web/src/App.tsx"),
        ("mcp-ci", "mcp-server/package-lock.json"),
    ]
    .into_iter()
    .chain(TREE_WIDE_GATES.map(|key| (key, ".agents/skills/jit-manage/SKILL.md")))
    .filter(|(key, path)| !holds(key, &|declared| declared.covers(path)))
    .map(|(key, path)| format!("gate {key} must declare {path} an input"));
    let uncovered = [
        ("npm-ci", "web/node_modules/react/index.js"),
        ("npm-ci", "web/dist/index.html"),
        ("mcp-ci", "mcp-server/node_modules/express/index.js"),
    ]
    .into_iter()
    .filter(|(key, path)| !holds(key, &|declared| !declared.covers(path)))
    .map(|(key, path)| format!("gate {key} must leave generated {path} out of its inputs"));
    let excluded = TREE_WIDE_GATES
        .into_iter()
        .flat_map(|key| {
            [".agents/worktrees", ".agents/cache-pool", "target"].map(|directory| (key, directory))
        })
        .chain([("npm-ci", "web/node_modules")])
        .filter(|(key, directory)| !holds(key, &|declared| declared.excludes_directory(directory)))
        .map(|(key, directory)| format!("gate {key} must skip {directory} rather than walk it"));
    let no_inputs = ["code-review", "secret-detection"]
        .into_iter()
        .filter(|key| gate(key).is_none_or(|gate| gate.inputs.is_some()))
        .map(|key| {
            format!("gate {key}'s verdict is not a function of the tree: it declares no inputs")
        });
    covered
        .chain(uncovered)
        .chain(excluded)
        .chain(no_inputs)
        .collect()
}
