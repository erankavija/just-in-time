//! Regression coverage for jit:3398bc19's dependency-feature narrowing.
//!
//! REQ-01/REQ-04: `jsonschema` compiles repository schemas that only use
//! same-document fragment refs (`#/types/State`, `#/types/Priority` in
//! `crate::schema`) so it never needs the crate's remote-resolution
//! infrastructure. Default features pull that infrastructure in anyway
//! (`resolve-http`/`resolve-file`/`tls-aws-lc-rs`, and transitively reqwest,
//! hyper, and aws-lc-rs), so the manifest disables default features.
//! REQ-02: `ureq` (the remote-document-access client) must compile exactly one
//! TLS implementation. The manifest disables default features and enables
//! exactly Rustls plus the `gzip` content-decoding feature explicitly; a
//! second backend (e.g. native-tls alongside the default Rustls stack) is the
//! regression these tests exist to catch.
//! REQ-03/REQ-06: the manifest intent above is necessary but not sufficient —
//! a stray feature edge elsewhere in the workspace could reintroduce the
//! banned packages without changing either dependency line. These tests
//! additionally interrogate the *resolved* dependency graph (`cargo tree`),
//! scoped to the `jsonschema` and `ureq` subtrees specifically: `jit-server`
//! (a separate binary in this workspace) legitimately depends on `hyper` via
//! `axum` for its own web server, so a whole-workspace "no hyper anywhere"
//! check would false-positive on that unrelated, pre-existing dependency.
//! Scoping to each crate's own subtree instead asserts the property that
//! actually matters: neither package pulls the banned crates in *for its own
//! purpose*.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above the jit crate manifest")
        .to_path_buf()
}

fn workspace_toml(path: &str) -> toml::Value {
    let text = std::fs::read_to_string(workspace_root().join(path))
        .unwrap_or_else(|error| panic!("read {path}: {error}"));
    toml::from_str(&text).unwrap_or_else(|error| panic!("parse {path}: {error}"))
}

fn jit_crate_manifest() -> toml::Value {
    workspace_toml("crates/jit/Cargo.toml")
}

fn dependency_table<'a>(manifest: &'a toml::Value, name: &str) -> &'a toml::Value {
    manifest
        .get("dependencies")
        .and_then(|deps| deps.get(name))
        .unwrap_or_else(|| panic!("crates/jit/Cargo.toml has no [dependencies].{name} entry"))
}

#[test]
fn test_workspace_dependencies_exclude_libgit2_bindings() {
    let lock = workspace_toml("Cargo.lock");
    let packages = lock["package"].as_array().expect("lockfile package array");
    for banned in ["git2", "libgit2-sys", "libssh2-sys", "openssl-sys"] {
        assert!(
            packages
                .iter()
                .all(|package| package["name"].as_str() != Some(banned)),
            "Cargo.lock must not contain `{banned}`"
        );
    }

    for path in ["crates/jit/Cargo.toml", "crates/server/Cargo.toml"] {
        let manifest = workspace_toml(path);
        let targets = manifest
            .get("target")
            .and_then(toml::Value::as_table)
            .into_iter()
            .flat_map(|targets| targets.values());
        for section in std::iter::once(&manifest).chain(targets) {
            for kind in ["dependencies", "build-dependencies", "dev-dependencies"] {
                if let Some(dependencies) = section.get(kind).and_then(toml::Value::as_table) {
                    assert!(
                        dependencies.iter().all(|(name, dependency)| {
                            name != "git2"
                                && dependency.get("package").and_then(toml::Value::as_str)
                                    != Some("git2")
                        }),
                        "{path} [{kind}] must not declare git2, including renamed dependencies"
                    );
                }
            }
        }
    }
}

#[test]
fn test_jsonschema_manifest_disables_default_features() {
    let manifest = jit_crate_manifest();
    let dep = dependency_table(&manifest, "jsonschema");
    assert_eq!(
        dep.get("default-features").and_then(|v| v.as_bool()),
        Some(false),
        "jsonschema must set default-features = false: its defaults \
         (resolve-http, resolve-file, tls-aws-lc-rs) pull reqwest, hyper, and \
         aws-lc-rs for remote $ref resolution this crate never performs \
         (repository schemas only use local fragment refs)"
    );

    let explicit_features: Vec<&str> = dep
        .get("features")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|v| {
                    v.as_str()
                        .expect("jsonschema feature entries must be strings")
                })
                .collect()
        })
        .unwrap_or_default();
    for feature in &explicit_features {
        assert!(
            !feature.starts_with("resolve-") && !feature.starts_with("tls-"),
            "jsonschema must not explicitly re-enable a remote-resolution or \
             TLS feature (disabling defaults alone would not stop this): got \
             `{feature}` in {explicit_features:?}"
        );
    }
}

#[test]
fn test_ureq_manifest_selects_exactly_one_tls_backend_explicitly() {
    let manifest = jit_crate_manifest();
    let dep = dependency_table(&manifest, "ureq");
    assert_eq!(
        dep.get("default-features").and_then(|v| v.as_bool()),
        Some(false),
        "ureq must set default-features = false so its TLS backend is chosen \
         deliberately rather than combined with an explicitly-added second one"
    );

    let features: Vec<&str> = dep
        .get("features")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("ureq dependency must declare an explicit features list"))
        .iter()
        .map(|v| v.as_str().expect("ureq feature entries must be strings"))
        .collect();

    assert!(
        features.contains(&"rustls"),
        "ureq must explicitly enable the rustls TLS backend, got {features:?}"
    );
    assert!(
        !features.contains(&"native-tls"),
        "ureq must not enable native-tls alongside rustls: exactly one TLS \
         backend, got {features:?}"
    );
    assert!(
        features.contains(&"gzip"),
        "ureq must explicitly enable gzip: it is the content-decoding feature \
         the remote-document path relies on, and must not depend on an \
         implicit default, got {features:?}"
    );
}

/// Query one package's normal/build dependency subtree with the suite's Cargo.
/// Dev-dependencies do not describe the crate's shipped dependency graph.
fn cargo_tree_subtree(package: &str) -> String {
    let cargo =
        std::env::var_os("CARGO").expect("CARGO must name the Cargo executable running this suite");
    let output = Command::new(&cargo)
        .current_dir(workspace_root())
        .args(["tree", "-e", "normal,build", "-p", package])
        .output()
        .unwrap_or_else(|error| {
            panic!("failed to spawn Cargo {cargo:?} for `tree -p {package}`: {error}")
        });

    assert!(
        output.status.success(),
        "cargo tree -p {package} failed (exit {:?}):\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A `cargo tree` line renders a dependency as `<name> v<version>`, so
/// `"<name> v"` uniquely identifies that package regardless of tree-drawing
/// prefix or indentation, and (crucially) without false-matching a
/// differently-named package that merely shares a prefix — `"rustls v"` does
/// not occur inside `"rustls-webpki v"` or `"rustls-pki-types v"` because a
/// hyphen, not a space, follows `rustls` in those names.
fn tree_contains_package(tree: &str, name: &str) -> bool {
    tree.contains(&format!("{name} v"))
}

#[test]
fn test_jsonschema_resolved_subtree_excludes_remote_resolution_infrastructure() {
    let tree = cargo_tree_subtree("jsonschema");

    for banned in ["reqwest", "hyper", "tokio-rustls", "aws-lc-rs", "rustls"] {
        assert!(
            !tree_contains_package(&tree, banned),
            "jsonschema's resolved dependency subtree must not contain \
             `{banned}` — remote $ref resolution must stay disabled \
             (default-features = false). Full subtree:\n{tree}"
        );
    }
}

#[test]
fn test_ureq_resolved_subtree_selects_exactly_one_tls_backend_with_explicit_gzip() {
    let tree = cargo_tree_subtree("ureq");

    assert!(
        tree_contains_package(&tree, "rustls"),
        "ureq's resolved subtree must contain rustls (the selected TLS \
         backend). Full subtree:\n{tree}"
    );
    assert!(
        !tree_contains_package(&tree, "native-tls"),
        "ureq's resolved subtree must not also contain native-tls — exactly \
         one TLS backend may compile. Full subtree:\n{tree}"
    );
    assert!(
        tree_contains_package(&tree, "flate2"),
        "ureq's resolved subtree must contain flate2 (the crate backing the \
         explicit gzip content-decoding feature). Full subtree:\n{tree}"
    );
}

#[test]
fn test_workspace_manifest_has_no_stray_reqwest_or_native_tls_dependency() {
    // Belt-and-braces: confirm neither banned crate is declared as a direct
    // dependency anywhere a future edit could reintroduce it, independent of
    // resolved-graph checks above.
    for manifest_path in [
        "Cargo.toml",
        "crates/jit/Cargo.toml",
        "crates/server/Cargo.toml",
    ] {
        let manifest = workspace_toml(manifest_path);
        for table_name in ["dependencies", "workspace.dependencies"] {
            let Some(table) = (match table_name {
                "workspace.dependencies" => manifest
                    .get("workspace")
                    .and_then(|w| w.get("dependencies")),
                _ => manifest.get("dependencies"),
            }) else {
                continue;
            };
            for banned in ["reqwest", "native-tls"] {
                assert!(
                    table.get(banned).is_none(),
                    "{manifest_path} [{table_name}] must not declare a direct \
                     dependency on `{banned}`"
                );
            }
        }
    }
}
