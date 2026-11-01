//! The `[item_kinds]` table a repository receives, and what it makes index
//! (issue `fbe0a401`, REQ-04).
//!
//! The engine bakes in NO item kinds: kinds are authored entirely in the
//! `[item_kinds]` config table. This test pins, through the SHIPPED binary,
//! that:
//!
//! 1. Applying the `jit-default` package writes an editable `[item_kinds]`
//!    table carrying exactly the kinds that package declares, each with the
//!    declaration the package gave it.
//! 2. A repo whose ONLY config is that written one indexes every kind (the
//!    table, not a baked default, is what makes them index).
//! 3. A repo with NO `[item_kinds]` table indexes NOTHING — proving there are no
//!    baked built-ins.

use jit::repository_state::{Contribution, MapEntryTarget};
use jit::test_cli::JitCli;
use serde_json::Value;
use std::collections::BTreeMap;

/// The package carrying the generic vocabulary, including the item kinds.
const DEFAULT_PACKAGE: &str = "jit-default";

/// Run `jit item list [--kind <kind>] --json` and return the parsed JSON.
fn item_list(cli: &JitCli, kind: Option<&str>) -> Value {
    let kind_args = kind.into_iter().flat_map(|kind| ["--kind", kind]);
    cli.run_json(["item", "list"].into_iter().chain(kind_args))
        .expect("jit item list failed")
}

/// The item kinds the default package declares, keyed by kind name.
///
/// Read from the manifest rather than restated here, so this file compares a
/// repository against the package's own declaration.
fn declared_item_kinds() -> BTreeMap<String, Value> {
    let (_workspace, package) = jit::test_utils::temporary_repository_package(DEFAULT_PACKAGE);
    package
        .model()
        .contributions
        .iter()
        .filter_map(|contribution| match contribution {
            Contribution::MapEntry {
                target: MapEntryTarget::ItemKinds,
                identity,
                value,
            } => Some((identity.clone(), value.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn test_init_emits_item_kinds_table_the_package_declares() {
    // REQ-04, clause 1: the written `[item_kinds]` table carries exactly the
    // kinds the applied package declares, each with the declaration it gave.
    let cli = JitCli::with_repository(
        jit::test_utils::profiled_repository_fixture(DEFAULT_PACKAGE, "packages")
            .expect("clone a coherent default-package repository fixture"),
    );
    let config: Value = toml::from_str::<toml::Value>(
        &std::fs::read_to_string(cli.root().join(".jit").join("config.toml")).unwrap(),
    )
    .unwrap()
    .try_into()
    .unwrap();

    let declared = declared_item_kinds();
    assert!(!declared.is_empty(), "the package declares item kinds");
    let written = config["item_kinds"]
        .as_object()
        .expect("the applied configuration carries an [item_kinds] table");

    assert_eq!(
        written.keys().cloned().collect::<Vec<_>>(),
        declared.keys().cloned().collect::<Vec<_>>(),
        "neither less nor more than the package declares"
    );
    for (name, declaration) in &declared {
        assert_eq!(&written[name], declaration, "[item_kinds.{name}]");
    }
}

#[test]
fn test_init_emits_the_namespaces_the_registry_first_kinds_link_through() {
    // A registry-first kind's entries are addressed straight from a repository
    // registry, so a label linking to one is authored against a namespace that
    // must itself be declared — otherwise it would fail the namespace-registry
    // rule even though the item it names resolves (REQ-02, jit:d30695e4).
    let cli = JitCli::with_repository(
        jit::test_utils::profiled_repository_fixture(DEFAULT_PACKAGE, "packages")
            .expect("clone a coherent default-package repository fixture"),
    );
    let config = toml::from_str::<toml::Value>(
        &std::fs::read_to_string(cli.root().join(".jit").join("config.toml")).unwrap(),
    )
    .unwrap();
    let namespaces = config["namespaces"]
        .as_table()
        .expect("the applied configuration carries a namespace registry");

    let linked = config["item_kinds"]
        .as_table()
        .expect("the applied configuration carries an [item_kinds] table")
        .values()
        .filter(|kind| {
            kind.get("source-of-truth").and_then(toml::Value::as_str) == Some("registry-first")
        })
        .filter_map(|kind| kind.get("link-namespaces"))
        .filter_map(toml::Value::as_array)
        .flatten()
        .filter_map(toml::Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        !linked.is_empty(),
        "a declared registry-first kind names a link-namespace"
    );
    for namespace in linked {
        assert!(
            namespaces.contains_key(namespace),
            "the kind link-namespace `{namespace}` must be a declared namespace"
        );
    }
}

#[test]
fn test_init_authored_table_indexes_all_kinds() {
    // REQ-04, clause 2: a repo whose ONLY config is the written one indexes
    // every declared kind — the table, not a baked default, makes them index.
    // `rule` items come for free here: initialization also scaffolds
    // `.jit/rules.toml` with the rules the registry generates, unlike
    // `invariants.toml` below, which this test writes by hand. `gate`'s registry
    // (`.jit/gates.toml`) starts EMPTY, so this test defines one gate through
    // the real `jit gate define` CLI before asserting.
    let cli = JitCli::with_repository(
        jit::test_utils::profiled_repository_fixture(DEFAULT_PACKAGE, "packages")
            .expect("clone a coherent default-package repository fixture"),
    );

    let issue_body = "\
## Success Criteria\n\n- [hard] REQ-01: writes are atomic\n\n\
## Decisions\n\n- D-01: store issues as JSON\n\n\
## Risks\n\n- RISK-01: concurrent writers\n";
    let id = cli
        .create_issue(["-t", "fixture", "-d", issue_body])
        .unwrap();
    let short: String = id.chars().take(8).collect();

    std::fs::write(
        cli.root().join(".jit").join("invariants.toml"),
        "[[invariants]]\nid = \"acyclic\"\nstatement = \"acyclic\"\nkind = \"enforced\"\n",
    )
    .unwrap();

    cli.run([
        "gate",
        "define",
        "cargo-ci",
        "--title",
        "Cargo CI",
        "--description",
        "Full Rust CI pipeline must pass.",
    ])
    .expect("gate define failed");

    let all = item_list(&cli, None);
    let kinds: Vec<&str> = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    for expected in declared_item_kinds().keys() {
        assert!(
            kinds.contains(&expected.as_str()),
            "the declared table must index a {expected} item: {kinds:?}"
        );
    }
    // Spot-check addressing across both substrates.
    let qids: Vec<&str> = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["qualified_id"].as_str().unwrap())
        .collect();
    assert!(qids.contains(&format!("@/issue/{short}/requirement/REQ-01").as_str()));
    assert!(qids.contains(&"@/invariant/acyclic"));
    assert!(qids.contains(&"@/gate/cargo-ci"));
}

#[test]
fn test_no_item_kinds_table_indexes_nothing() {
    // REQ-04, clause 3 (no built-ins): with the `[item_kinds]` table removed, a
    // repo carrying issue criteria AND an invariants registry indexes NOTHING —
    // there are no baked default kinds.
    let cli = JitCli::with_repository(
        jit::test_utils::profiled_repository_fixture(DEFAULT_PACKAGE, "packages")
            .expect("clone a coherent default-package repository fixture"),
    );

    let issue_body = "\
## Success Criteria\n\n- [hard] REQ-01: writes are atomic\n\n\
## Decisions\n\n- D-01: store issues as JSON\n\n\
## Risks\n\n- RISK-01: concurrent writers\n";
    cli.create_issue(["-t", "fixture", "-d", issue_body])
        .unwrap();

    std::fs::write(
        cli.root().join(".jit").join("invariants.toml"),
        "[[invariants]]\nid = \"acyclic\"\nstatement = \"acyclic\"\nkind = \"enforced\"\n",
    )
    .unwrap();

    // Strip the [item_kinds] table: write a config with none.
    std::fs::write(
        cli.root().join(".jit").join("config.toml"),
        "[version]\nschema = 2\n",
    )
    .unwrap();

    let all = item_list(&cli, None);
    assert_eq!(
        all["items"].as_array().unwrap().len(),
        0,
        "with no [item_kinds] table there are no kinds (no baked built-ins): {all}"
    );
}
