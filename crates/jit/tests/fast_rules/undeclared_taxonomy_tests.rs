//! A repository receives exactly the taxonomy its `config.toml` declares
//! (jit:a30d704d).
//!
//! Each test scaffolds a repository whose `config.toml` is written before
//! initialization — which preserves it — and then reads the artefacts the
//! scaffold published: `.jit/rules.toml` and the `.jit/schemas/*.json`
//! projections its JSON Schema rules reference. Every schema is located through
//! the reference in the rule that owns it, and the expected namespace and type
//! sets come from the declaration the test itself wrote, so a test fails exactly
//! when a repository receives something it did not declare.

use jit::commands::CommandExecutor;
use jit::storage::JsonFileStorage;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Initialize a repository whose `config.toml` is exactly `config_toml`.
fn scaffold_declaring(config_toml: &str) -> (TempDir, std::path::PathBuf) {
    std::env::set_var("JIT_TEST_MODE", "1");
    let temp = TempDir::new().unwrap();
    let jit_dir = temp.path().join(".jit");
    fs::create_dir(&jit_dir).unwrap();
    fs::write(jit_dir.join("config.toml"), config_toml).unwrap();
    let storage = JsonFileStorage::new(&jit_dir);
    let taxonomy = jit::test_taxonomy::test_taxonomy();
    let layout = jit::storage::discover_repository_layout(temp.path(), &jit_dir).unwrap();
    CommandExecutor::new(storage)
        .with_layout(layout)
        .initialize_fresh_repository(temp.path(), &taxonomy.hierarchy_template(), None)
        .unwrap();
    (temp, jit_dir)
}

/// The rule names the scaffold wrote into `.jit/rules.toml`, in file order.
fn scaffolded_rule_names(jit_dir: &Path) -> Vec<String> {
    scaffolded_rules(jit_dir)
        .iter()
        .map(|rule| rule["name"].as_str().unwrap().to_string())
        .collect()
}

fn scaffolded_rules(jit_dir: &Path) -> Vec<toml::Value> {
    let rules: toml::Value =
        toml::from_str(&fs::read_to_string(jit_dir.join("rules.toml")).unwrap()).unwrap();
    rules["rules"].as_array().unwrap().clone()
}

/// Rule name to the `schemas/…` path that rule's assertion references, for
/// every JSON Schema rule the scaffold wrote.
fn schema_references(jit_dir: &Path) -> BTreeMap<String, String> {
    scaffolded_rules(jit_dir)
        .iter()
        .filter_map(|rule| {
            let reference = rule["assert"].get("json-schema")?.as_str()?;
            Some((
                rule["name"].as_str().unwrap().to_string(),
                reference.to_string(),
            ))
        })
        .collect()
}

/// The `.jit/schemas/*.json` file names the scaffold published.
fn published_schema_files(jit_dir: &Path) -> BTreeSet<String> {
    let schemas = jit_dir.join("schemas");
    if !schemas.is_dir() {
        return BTreeSet::new();
    }
    fs::read_dir(schemas)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

/// The schema a named rule references, parsed.
fn schema_of(jit_dir: &Path, rule_name: &str) -> serde_json::Value {
    let reference = schema_references(jit_dir)
        .remove(rule_name)
        .unwrap_or_else(|| panic!("no JSON Schema rule named `{rule_name}` was written"));
    serde_json::from_str(&fs::read_to_string(jit_dir.join(reference)).unwrap()).unwrap()
}

/// The concatenated bytes of every artefact the scaffold derived from the
/// taxonomy: the rule file and every schema projection it published.
fn derived_artefacts(jit_dir: &Path) -> String {
    let mut all = fs::read_to_string(jit_dir.join("rules.toml")).unwrap();
    for name in published_schema_files(jit_dir) {
        all.push_str(&fs::read_to_string(jit_dir.join("schemas").join(name)).unwrap());
    }
    all
}

/// The namespace alternation the `namespace-registry` projection enumerates.
fn projected_namespaces(jit_dir: &Path) -> BTreeSet<String> {
    schema_of(jit_dir, "namespace-registry")["properties"]["raw_labels"]["items"]["pattern"]
        .as_str()
        .unwrap()
        .trim_start_matches("^(")
        .trim_end_matches("):")
        .split('|')
        .map(str::to_string)
        .collect()
}

/// The type names the `type-hierarchy-known` projection enumerates.
fn projected_types(jit_dir: &Path) -> BTreeSet<String> {
    schema_of(jit_dir, "type-hierarchy-known")["properties"]["labels"]["properties"]["type"]
        ["items"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect()
}

fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| value.to_string()).collect()
}

/// A configuration carrying only the structural minimum: no `[namespaces]`,
/// no `[type_hierarchy]`.
const DECLARES_NOTHING: &str = "[version]\nschema = 2\n";

/// REQ-01 + REQ-02 + REQ-05: a repository declaring neither table receives the
/// label grammar alone, and exactly the one projection that rule references.
#[test]
fn test_scaffold_declaring_no_taxonomy_writes_the_label_grammar_alone() {
    let (_temp, jit_dir) = scaffold_declaring(DECLARES_NOTHING);
    let references = schema_references(&jit_dir);

    assert_eq!(scaffolded_rule_names(&jit_dir), vec!["label-format"]);
    // Every published projection is one an emitted rule references, so the one
    // surviving rule leaves exactly one schema file behind.
    assert_eq!(
        published_schema_files(&jit_dir),
        references
            .values()
            .map(|reference| reference.rsplit('/').next().unwrap().to_string())
            .collect::<BTreeSet<_>>(),
    );
    // The surviving rule asserts the label grammar over whole labels, which
    // names no namespace and no type.
    assert!(
        schema_of(&jit_dir, "label-format")["properties"]["raw_labels"]["items"]["pattern"]
            .is_string()
    );
}

/// REQ-01 + REQ-02: no name the configuration never declared appears in any
/// artefact the scaffold derived from the taxonomy.
#[test]
fn test_scaffold_declaring_no_taxonomy_names_nothing_the_repository_did_not_declare() {
    let (_temp, jit_dir) = scaffold_declaring(DECLARES_NOTHING);
    let derived = derived_artefacts(&jit_dir);
    let taxonomy = jit::test_taxonomy::test_taxonomy();

    // A declaration-free configuration declares no vocabulary at all, so every
    // namespace and type name is one the repository did not declare. These are
    // the ones a supplied taxonomy would have introduced.
    for undeclared in taxonomy.hierarchy.keys().chain(
        taxonomy
            .namespaces
            .keys()
            .filter(|namespace| namespace.as_str() != "type"),
    ) {
        assert!(
            !derived.contains(undeclared),
            "artefacts derived from an empty taxonomy name `{undeclared}`, \
             which the repository never declared:\n{derived}",
        );
    }
}

/// REQ-05: a repository declaring neither table is still a valid repository.
#[test]
fn test_scaffold_declaring_no_taxonomy_still_validates() {
    let (temp, jit_dir) = scaffold_declaring(DECLARES_NOTHING);
    let layout = jit::storage::discover_repository_layout(temp.path(), &jit_dir).unwrap();
    let executor = CommandExecutor::new(JsonFileStorage::new(&jit_dir)).with_layout(layout);

    let report = executor
        .validate_repository_report()
        .unwrap()
        .expect("a repository declaring no taxonomy must validate");

    assert!(
        !report.rule_report.has_errors(),
        "unexpected findings: {:?}",
        report.rule_report.findings,
    );
}

/// REQ-03: a repository declaring both tables receives exactly the vocabulary
/// it declared — every projected name comes from its own declaration.
#[test]
fn test_scaffold_declaring_a_taxonomy_receives_exactly_what_it_declared() {
    let (_temp, jit_dir) = scaffold_declaring(
        r#"
[version]
schema = 2

[type_hierarchy]
types = { objective = 1, initiative = 2, action = 3 }
label_associations = { objective = "objective" }

[namespaces.type]
description = "Issue type"
unique = true

[namespaces.squad]
description = "Owning squad"
unique = false
"#,
    );

    assert_eq!(
        projected_types(&jit_dir),
        names(&["objective", "initiative", "action"]),
    );
    // The declared namespaces, plus the membership namespace the declared label
    // association introduces — both derived from this configuration alone.
    assert_eq!(
        projected_namespaces(&jit_dir),
        names(&["objective", "squad", "type"]),
    );
    // A uniqueness rule exists for the one namespace declared unique, and for
    // no other.
    let unique_rules: Vec<String> = scaffolded_rule_names(&jit_dir)
        .into_iter()
        .filter(|name| name.starts_with("namespace-unique-"))
        .collect();
    assert_eq!(unique_rules, vec!["namespace-unique-type"]);
}

/// REQ-01 + REQ-03: declaring namespaces alone yields the namespace rules and
/// no rule or projection enumerating type names.
#[test]
fn test_scaffold_declaring_namespaces_alone_receives_no_type_hierarchy_rule() {
    let (_temp, jit_dir) = scaffold_declaring(
        r#"
[version]
schema = 2

[namespaces.squad]
description = "Owning squad"
unique = true
"#,
    );
    let rule_names = scaffolded_rule_names(&jit_dir);

    assert_eq!(projected_namespaces(&jit_dir), names(&["squad"]));
    assert!(rule_names.contains(&"namespace-unique-squad".to_string()));
    assert!(
        rule_names.iter().all(|name| {
            name != "type-hierarchy-known"
                && name != "orphan-leaf"
                && name != "strategic-consistency"
        }),
        "a repository declaring no type hierarchy received the rule: {rule_names:?}",
    );
}

/// REQ-02 + REQ-03: declaring a type hierarchy alone yields the type rules and
/// no rule or projection enumerating namespaces.
#[test]
fn test_scaffold_declaring_a_type_hierarchy_alone_receives_no_namespace_rule() {
    let (_temp, jit_dir) = scaffold_declaring(
        r#"
[version]
schema = 2

[type_hierarchy]
types = { objective = 1, action = 2 }
"#,
    );
    let rule_names = scaffolded_rule_names(&jit_dir);

    assert_eq!(projected_types(&jit_dir), names(&["objective", "action"]));
    assert!(rule_names.contains(&"type-hierarchy-known".to_string()));
    assert!(
        rule_names
            .iter()
            .all(|name| { name != "orphan-leaf" && name != "strategic-consistency" }),
        "a repository that applied no package received a workflow rule: {rule_names:?}",
    );
    assert!(
        !rule_names
            .iter()
            .any(|name| name == "namespace-registry" || name.starts_with("namespace-unique-")),
        "a repository declaring no namespaces received a namespace rule: {rule_names:?}",
    );
}
