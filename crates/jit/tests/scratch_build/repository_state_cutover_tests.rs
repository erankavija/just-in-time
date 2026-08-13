//! Known current-tree regressions forbidden by the repository-state cutover.
//!
//! Publisher access itself is compiler-enforced rather than asserted here: the
//! in-repository writers (`storage::atomic_write`) and the transaction kernel
//! (`storage::file_transaction`) are `pub(in crate::storage)`, so `crate::commands`
//! cannot name them and a regression fails the build. This file covers the
//! properties visibility cannot express — deleted modules and wrappers,
//! canonical renderer ownership, and the modules that must reach the filesystem
//! only through a named discipline or not at all — because `std::fs` is public
//! to every crate and no `pub(in ...)` restriction reaches it. Each such test
//! carries a guard asserting the mechanism it is about is still present, so an
//! absent call shape can only mean the discipline held rather than that the
//! code it was about moved away.

use std::path::{Path, PathBuf};

fn production_source(path: &Path) -> String {
    let source = std::fs::read_to_string(path).unwrap();
    source
        .split_once("\n#[cfg(test)]\nmod ")
        .map_or(source.as_str(), |(production, _)| production)
        .to_string()
}

fn definition_owners(root: &Path, definition: &str) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut owners = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && production_source(&path).contains(definition)
            {
                owners.push(path.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    owners
}

/// Require one capability shape to remain in the canonical module.
///
/// This deliberately identifies authorities by the data they decode, model, or
/// enumerate rather than their function/type spelling: a renamed parallel
/// implementation is still a parallel authority. Package-local helpers that
/// consume the canonical reader for a known path do not have the complete
/// inventory shape and therefore remain permitted.
fn assert_canonical_authority(
    authority: &str,
    mut owners: Vec<PathBuf>,
    expected: PathBuf,
) -> Result<(), String> {
    owners.sort();
    (owners == [expected.clone()]).then_some(()).ok_or_else(|| {
        format!(
            "forbidden duplicate {authority} authority: found {owners:?}, expected only \
             {expected:?}; the clean-cut lifecycle contract has one canonical package decoder, model, and \
             applied-record inventory reader rather than parallel formats or readers"
        )
    })
}

fn authority_owners_from_sources<'a>(
    sources: impl IntoIterator<Item = (&'a Path, &'a str)>,
    is_authority: fn(&str) -> bool,
) -> Vec<PathBuf> {
    let mut owners = sources
        .into_iter()
        .filter(|(_, source)| is_authority(source))
        .map(|(path, _)| path.to_path_buf())
        .collect::<Vec<_>>();
    owners.sort();
    owners
}

fn authority_owners(root: &Path, is_authority: fn(&str) -> bool) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                let source = production_source(&path);
                sources.push((relative, source));
            }
        }
    }
    authority_owners_from_sources(
        sources
            .iter()
            .map(|(path, source)| (path.as_path(), source.as_str())),
        is_authority,
    )
}

/// A manifest wire authority both parses TOML and decides the versioned
/// `manifest-version` representation that becomes `ProfilePackageModel`.
fn is_manifest_decoder(source: &str) -> bool {
    source.contains("toml::from_str")
        && source.contains("manifest-version")
        && source.contains("ProfilePackageModel")
}

/// A package model authority carries the manifest's complete semantic
/// contribution vocabulary, not merely a view or a resolved copy of it.
fn is_package_model(source: &str) -> bool {
    [
        "compatible_jit:",
        "variables: Vec<ProfileVariableDeclaration>",
        "contributions: Vec<Contribution>",
        "assets: Vec<AssetDeclaration>",
        "regions: Vec<RegionDeclaration>",
    ]
    .into_iter()
    .all(|field| source.contains(field))
}

/// An applied-record inventory authority lists the profile directory and
/// returns the records that listing names. Known-path readers do neither: they
/// receive a path from a caller and only decode that one record.
fn is_applied_record_inventory_reader(source: &str) -> bool {
    source.contains("Result<Vec<(VirtualPath, AppliedProfileRecord)>")
        && source.contains("listing_fingerprints()")
}

/// Reject a predecessor declaration rather than its harmless mentions in tests,
/// documentation, or a historical event parser.
fn assert_retired_lifecycle_entry_point_absent(
    production: &str,
    definition: &str,
) -> Result<(), String> {
    (!production.contains(definition))
        .then_some(())
        .ok_or_else(|| {
            format!(
                "forbidden retired lifecycle entry point returned: {definition}; it violates the \
             clean-cut lifecycle contract by restoring a single-profile selector, per-package \
             publication bypass, or pre-aggregate audit constructor"
            )
        })
}

/// Reject every production and generated-contract spelling of the retired
/// per-package profile audit event. Unknown event records remain generic parser
/// input; this guard concerns only the current typed vocabulary and the schema
/// projected from it.
fn assert_retired_profile_applied_contract_absent(
    production_sources: impl IntoIterator<Item = (PathBuf, String)>,
    generated_event_tags: impl IntoIterator<Item = String>,
) -> Result<(), String> {
    let mut violations = production_sources
        .into_iter()
        .flat_map(|(path, source)| {
            [
                ("ProfileApplied", "typed Event::ProfileApplied variant"),
                (
                    "profile_applied",
                    "retired profile_applied tag or constructor",
                ),
            ]
            .into_iter()
            .filter(move |(needle, _)| source.contains(needle))
            .map(move |(_, surface)| format!("{surface} in {}", path.display()))
        })
        .collect::<Vec<_>>();
    violations.extend(
        generated_event_tags
            .into_iter()
            .filter(|tag| tag == "profile_applied")
            .map(|tag| format!("generated schema event tag `{tag}`")),
    );

    violations.is_empty().then_some(()).ok_or_else(|| {
        format!(
            "forbidden retired profile-applied event contract returned: {}; use only \
             Event::ProfileLifecycle and the `profile_lifecycle` schema tag; generic unknown-event \
             retention must not restore a profile-specific decoder or compatibility path",
            violations.join(", ")
        )
    })
}

fn production_sources(root: &Path) -> Vec<(PathBuf, String)> {
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push((
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    production_source(&path),
                ));
            }
        }
    }
    sources
}

fn generated_event_tags() -> Vec<String> {
    jit::CommandSchema::generate()
        .events
        .into_iter()
        .map(|event| event.tag.as_str().to_string())
        .collect()
}

/// Find a positive secret/sensitive classification while accepting the one
/// deliberate vocabulary for this contract: an input may be described as
/// *non-secret* or *non-sensitive*. Splitting into words makes this independent
/// of JSON, Rust comments, and prose punctuation without maintaining a field
/// inventory for every public surface.
fn sensitive_profile_input_description(source: &str) -> Option<&'static str> {
    let words = source
        .split(|character: char| !character.is_ascii_alphabetic())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    words.iter().enumerate().find_map(|(index, word)| {
        matches!(word.as_str(), "secret" | "sensitive")
            .then(|| (index == 0 || words[index - 1] != "non").then_some(word.as_str()))
            .flatten()
            .map(|word| match word {
                "secret" => "secret",
                _ => "sensitive",
            })
    })
}

fn assert_profile_input_surface_is_non_sensitive(
    surface: &str,
    description: &str,
) -> Result<(), String> {
    sensitive_profile_input_description(description)
        .map(|term| {
            format!(
                "forbidden {term} profile input description in {surface}; it violates the \
                 clean-cut lifecycle contract because profile inputs must remain non-secret and \
                 their values must not gain a sensitive-data channel"
            )
        })
        .map_or(Ok(()), Err)
}

/// The bridge maps every generated tool into exactly one curation section.
/// Lifecycle input descriptions can therefore move between `include` and
/// `exclude` without falling outside this check; membership is the generated
/// init command plus the profile command family, not its current curation.
fn lifecycle_bridge_inventory(inventory: &serde_json::Value) -> Vec<&serde_json::Value> {
    ["include", "exclude"]
        .into_iter()
        .flat_map(|section| {
            inventory[section]
                .as_object()
                .expect("bridge inventory keeps every curation section as a generated-tool map")
                .iter()
        })
        .filter(|(name, _)| *name == "jit_init" || name.starts_with("jit_profile_"))
        .map(|(_, description)| description)
        .collect()
}

fn profile_generated_schema_text() -> String {
    let schema = jit::CommandSchema::generate();
    let profile = schema
        .commands
        .get("profile")
        .expect("generated schema retains the profile command family");
    let init = schema
        .commands
        .get("init")
        .expect("generated schema retains initialization's profile inputs");
    let manifest = schema
        .types
        .get("ProfilePackageModel")
        .expect("generated schema retains the canonical manifest model");
    let lifecycle_events = schema
        .events
        .iter()
        .filter(|event| event.tag.as_str() == "profile_lifecycle")
        .collect::<Vec<_>>();
    assert_eq!(
        lifecycle_events.len(),
        1,
        "the generated schema must carry one canonical profile lifecycle audit event"
    );
    serde_json::json!({
        "init": init,
        "profile": profile,
        "manifest": manifest,
        "profile_lifecycle_events": lifecycle_events,
    })
    .to_string()
}

fn contains_cargo_manifest(root: &Path) -> bool {
    std::fs::read_dir(root).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name().is_some_and(|name| name == "Cargo.toml")
            || path.is_dir() && contains_cargo_manifest(&path)
    })
}

/// Raw `std::fs` publication is the one residual channel into a repository root
/// that visibility cannot close: `std::fs` is public to every crate, so no
/// `pub(in ...)` restriction reaches it. This is therefore a deliberate,
/// recorded source-text exception to the compiler-enforced mechanism, and it is
/// kept as narrow as the exception requires — the four cutover command modules,
/// the four call shapes that publish. A crate-wide `clippy.toml`
/// `disallowed_methods` entry is the wrong instrument: `commands/snapshot.rs`
/// legitimately stages exports with raw `std::fs` into temp directories outside
/// the repository roots, which such a rule would forbid.
#[test]
fn test_cutover_command_modules_publish_no_raw_filesystem_writes() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for consumer in ["init.rs", "profile.rs", "project.rs", "validate.rs"] {
        let source = production_source(&source_root.join("commands").join(consumer));
        for publisher in [
            "fs::write(",
            "fs::rename(",
            "File::create(",
            "OpenOptions::new(",
        ] {
            assert!(
                !source.contains(publisher),
                "raw filesystem publication returned in commands/{consumer}: {publisher}"
            );
        }
    }
}

#[test]
fn test_cutover_has_no_known_predecessor_or_compatibility_module() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = crate_root.join("src");
    for relative in [
        "profile/render.rs",
        "profile/snapshot.rs",
        "profile/drift.rs",
        "profile/preset.rs",
        "profile/planner.rs",
        "storage/config_store.rs",
        "validation/rules.rs",
        "validation/defaults.rs",
        "validation/serialize.rs",
        "validation/project_render.rs",
        "validation/projection.rs",
    ] {
        assert!(!source_root.join(relative).exists(), "returned: {relative}");
    }

    let library = production_source(&source_root.join("lib.rs"));
    assert!(library.contains("pub mod repository_state;"));
    for alias in [
        "pub type Storage =",
        "pub type Gate =",
        "pub use crate::repository_state",
    ] {
        assert!(!library.contains(alias), "alias returned: {alias}");
    }
    for facade in ["profile/mod.rs", "storage/mod.rs", "validation/mod.rs"] {
        let source = production_source(&source_root.join(facade));
        for seam in [
            "pub use crate::repository_state",
            "pub use crate::declarations",
            "pub type Storage =",
            "pub type Gate =",
        ] {
            assert!(!source.contains(seam), "facade seam in {facade}: {seam}");
        }
    }

    let workspace = std::fs::read_to_string(crate_root.join("../../Cargo.toml")).unwrap();
    for package in ["crates/repository-state", "crates/declarations"] {
        assert!(
            !workspace.contains(package),
            "intermediate package: {package}"
        );
    }
    for module in ["repository_state", "declarations"] {
        assert!(!contains_cargo_manifest(&source_root.join(module)));
    }

    for (scope, wrapper) in [
        ("storage", "fn save_issue("),
        ("storage", "fn restore_issue_verbatim("),
        ("storage", "fn delete_issue("),
        ("storage", "fn append_event("),
        ("storage", "fn save_gate_run_result("),
        ("storage", "fn save_gate_registry("),
        ("storage", "fn save_gate_preset("),
        ("storage", "fn write_repo_file("),
        ("profile", "fn project_package("),
        ("profile", "fn capture_profile_snapshot("),
        ("profile", "fn plan_profile_application_against("),
    ] {
        assert!(
            definition_owners(&source_root.join(scope), wrapper).is_empty(),
            "plan-deleted wrapper returned in {scope}: {wrapper}"
        );
    }
}

#[test]
fn test_known_projection_renderer_definitions_keep_canonical_owners() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for (definition, file) in [
        ("fn render_managed_document(", "managed_document.rs"),
        ("fn render_projection_body(", "projection_render.rs"),
        ("fn render_invariants_markdown(", "projection.rs"),
        (
            "fn render_rules_and_gates_markdown(",
            "rules_gates_projection.rs",
        ),
        ("fn serialize_ruleset(", "rule_serialize.rs"),
        ("fn render_repo_config(", "initialize.rs"),
    ] {
        assert_eq!(
            definition_owners(&source_root, definition),
            [PathBuf::from("repository_state").join(file)],
            "known renderer ownership changed: {definition}"
        );
    }
}

/// The package lifecycle has one decoder, one runtime model, and one reader
/// for the repository's applied-record inventory. A helper that reads a known
/// record path is not another inventory: its caller already chose that path.
/// These ownership checks therefore guard only declarations that can become a
/// competing format authority.
#[test]
fn test_profile_lifecycle_has_one_canonical_package_and_record_authority() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for (authority, expected, classify) in [
        (
            "manifest decoder",
            "profile/wire.rs",
            is_manifest_decoder as fn(&str) -> bool,
        ),
        ("package model", "profile/manifest.rs", is_package_model),
        (
            "applied-record inventory reader",
            "repository_state/profile_apply.rs",
            is_applied_record_inventory_reader,
        ),
    ] {
        assert_canonical_authority(
            authority,
            authority_owners(&source_root, classify),
            PathBuf::from(expected),
        )
        .unwrap_or_else(|error| panic!("{error}"));
    }
}

/// The selection stream and aggregate lifecycle event replaced two public
/// predecessor shapes. Keeping their names absent is more precise than
/// banning the legitimate `apply_profile_package*` closure wrappers: those
/// wrappers resolve one package's dependency closure and enter the one
/// aggregate seam rather than publishing per package.
#[test]
fn test_profile_lifecycle_has_no_retired_selector_bypass() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for (relative, definition) in [
        ("commands/init.rs", "struct ProfileSelection"),
        ("commands/profile.rs", "fn apply_one_profile_package("),
    ] {
        assert_retired_lifecycle_entry_point_absent(
            &production_source(&source_root.join(relative)),
            definition,
        )
        .unwrap_or_else(|error| panic!("{error}"));
    }
}

/// The typed Event vocabulary, its fieldless tag mirror, and `jit --schema`
/// are one current contract. The retired per-package audit representation must
/// not return through any of those surfaces after the aggregate cutover.
#[test]
fn test_profile_lifecycle_has_no_retired_profile_applied_event_contract() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert_retired_profile_applied_contract_absent(
        production_sources(&source_root),
        generated_event_tags(),
    )
    .unwrap_or_else(|error| panic!("{error}"));
}

/// Inputs reach users through the manifest and CLI, machine clients through
/// the generated schema and bridge inventory, and later readers through the
/// lifecycle audit projection. Every one must say the same thing: values are
/// ordinary non-secret inputs, not credentials carried by profile lifecycle.
#[test]
fn test_profile_lifecycle_input_surfaces_do_not_offer_secret_or_sensitive_values() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = crate_root.join("../..");
    let manifest = production_source(&crate_root.join("src/profile/manifest.rs"));
    let cli = production_source(&crate_root.join("src/cli.rs"));
    let profile_cli = cli
        .split_once("/// Repository profile commands.")
        .expect("profile CLI declaration remains a separately documented command family")
        .1;
    let audit_types = production_source(&crate_root.join("src/domain/types.rs"));
    let profile_audit_inputs = audit_types
        .split_once("/// One variable's non-sensitive provenance in a lifecycle event.")
        .expect("profile lifecycle audit retains its variable provenance type")
        .1
        .split_once("/// System event types for audit log")
        .expect("profile lifecycle audit inputs remain separate from unrelated event types")
        .0;
    let bridge_inventory =
        std::fs::read_to_string(workspace_root.join("mcp-server/curated-tools.json")).unwrap();
    let bridge_inventory = serde_json::from_str::<serde_json::Value>(&bridge_inventory).unwrap();
    let bridge_lifecycle_inventory = lifecycle_bridge_inventory(&bridge_inventory);

    for (surface, description) in [
        ("profile manifest model", manifest),
        ("profile command-line definitions", profile_cli.to_string()),
        ("generated profile schema", profile_generated_schema_text()),
        (
            "profile lifecycle audit record",
            profile_audit_inputs.to_string(),
        ),
        (
            "MCP init and profile lifecycle inventory",
            serde_json::to_string(&bridge_lifecycle_inventory).unwrap(),
        ),
    ] {
        assert_profile_input_surface_is_non_sensitive(surface, &description)
            .unwrap_or_else(|error| panic!("{error}"));
    }
}

/// Red-first proof for the structural predicates above. These are deliberately
/// small source fragments rather than mutations of production files: each
/// proves the corresponding guard has a failing path without coupling the
/// suite to a temporary working-tree rewrite.
#[test]
fn test_profile_lifecycle_structural_guards_reject_their_forbidden_shapes() {
    for (authority, expected, classify, canonical, renamed_duplicate) in [
        (
            "manifest decoder",
            "profile/wire.rs",
            is_manifest_decoder as fn(&str) -> bool,
            "fn decode_manifest() { toml::from_str(text); manifest-version; ProfilePackageModel; }",
            "fn reconstruct_package_wire() { toml::from_str(text); manifest-version; ProfilePackageModel; }",
        ),
        (
            "package model",
            "profile/manifest.rs",
            is_package_model,
            "struct ProfilePackageModel { compatible_jit: String, variables: Vec<ProfileVariableDeclaration>, contributions: Vec<Contribution>, assets: Vec<AssetDeclaration>, regions: Vec<RegionDeclaration> }",
            "struct AlternatePackageShape { compatible_jit: String, variables: Vec<ProfileVariableDeclaration>, contributions: Vec<Contribution>, assets: Vec<AssetDeclaration>, regions: Vec<RegionDeclaration> }",
        ),
        (
            "applied-record inventory reader",
            "repository_state/profile_apply.rs",
            is_applied_record_inventory_reader,
            "fn applied_profile_records() -> Result<Vec<(VirtualPath, AppliedProfileRecord)>> { listing_fingerprints(); }",
            "fn collect_installed_provenance() -> Result<Vec<(VirtualPath, AppliedProfileRecord)>> { listing_fingerprints(); }",
        ),
    ] {
        let duplicate = assert_canonical_authority(
            authority,
            authority_owners_from_sources(
                [
                    (Path::new(expected), canonical),
                    (Path::new("profile/renamed_parallel.rs"), renamed_duplicate),
                ],
                classify,
            ),
            PathBuf::from(expected),
        )
        .unwrap_err();
        assert!(
            duplicate.contains(&format!("forbidden duplicate {authority} authority"))
                && duplicate.contains("clean-cut lifecycle contract")
        );
    }

    let retired_bypass = assert_retired_lifecycle_entry_point_absent(
        "pub(super) fn apply_one_profile_package() {}",
        "fn apply_one_profile_package(",
    )
    .unwrap_err();
    assert!(
        retired_bypass.contains("forbidden retired lifecycle entry point")
            && retired_bypass.contains("clean-cut lifecycle contract")
    );

    let retired_event_contract = assert_retired_profile_applied_contract_absent(
        [
            (
                PathBuf::from("domain/types.rs"),
                "enum Event { ProfileApplied { id: String } }".to_string(),
            ),
            (
                PathBuf::from("domain/event_catalog.rs"),
                "EventTag::ProfileApplied => \"profile_applied\"; \
                 Event::draft_profile_applied()"
                    .to_string(),
            ),
        ],
        ["profile_applied".to_string()],
    )
    .unwrap_err();
    for expected in [
        "typed Event::ProfileApplied variant",
        "retired profile_applied tag or constructor",
        "generated schema event tag `profile_applied`",
        "use only Event::ProfileLifecycle",
    ] {
        assert!(
            retired_event_contract.contains(expected),
            "{retired_event_contract}"
        );
    }

    for term in ["secret", "sensitive"] {
        let error = assert_profile_input_surface_is_non_sensitive(
            "injected profile input surface",
            &format!("a {term} profile input"),
        )
        .unwrap_err();
        assert!(
            error.contains(&format!("forbidden {term} profile input description"))
                && error.contains("clean-cut lifecycle contract")
        );
    }
    assert!(assert_profile_input_surface_is_non_sensitive(
        "affirmative contract",
        "a non-secret and non-sensitive profile input"
    )
    .is_ok());

    let omitted_init = serde_json::json!({
        "include": { "jit_profile_apply": "ordinary profile inputs" },
        "exclude": { "jit_init": "accepts a secret initialization input" },
    });
    let bridge_error = assert_profile_input_surface_is_non_sensitive(
        "injected MCP lifecycle inventory",
        &serde_json::to_string(&lifecycle_bridge_inventory(&omitted_init)).unwrap(),
    )
    .unwrap_err();
    assert!(
        bridge_error.contains("forbidden secret profile input description")
            && bridge_error.contains("clean-cut lifecycle contract")
    );
}

/// A package read must resolve each source once, at the open, and take
/// everything it decides from that handle.
///
/// Pathname-addressed reads are the one shape visibility cannot forbid:
/// `std::fs` is public to every crate. A capture that stats a declared target
/// and then reads it by name answers two questions about two objects whenever
/// something replaces the name in between — the read follows a link the check
/// never saw, and the mode judged is not the mode of the bytes admitted. The
/// property is structural rather than behavioural because reproducing that
/// interval in a test means racing it, and a test that must win a race to fail
/// is a test that reports nothing on the runs it loses.
#[test]
fn test_package_capture_addresses_no_declared_source_by_pathname() {
    let capture = production_source(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/profile/package_capture.rs"),
    );
    for reader in [
        "fs::read(",
        "fs::read_to_string(",
        "fs::metadata(",
        "fs::symlink_metadata(",
        "fs::canonicalize(",
        "File::open(",
    ] {
        assert!(
            !capture.contains(reader),
            "package capture addressed a source by pathname: {reader}"
        );
    }
    assert!(
        capture.contains("open_path_nofollow("),
        "package capture no longer reads through the shared no-follow open, so \
         the absence of pathname readers above establishes nothing"
    );
}

/// One no-follow opener serves every route into a package.
///
/// Walking a package directory and capturing a tree from declared repository
/// files both need the same open — no link followed into the name, no
/// resolution out of the anchoring directory, non-blocking so an entry of the
/// wrong kind cannot hold the reader open forever. A second copy is where one
/// of those subtleties goes missing (`@/inv/convention-convergence`), so the
/// options are built in exactly one place.
#[test]
fn test_profile_package_reads_build_no_follow_options_in_one_place() {
    let profile = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/profile");
    let owners = definition_owners(&profile, "_cap_fs_ext_follow");
    assert_eq!(
        owners,
        vec![PathBuf::from("nofollow.rs")],
        "each entry names a profile module building its own no-follow open"
    );
}

/// Reading a package archive touches nothing outside the caller's memory.
///
/// This is what makes "a refused or interrupted add leaves no partially
/// extracted package" true by construction rather than by cleanup: extraction
/// produces a value, and the only thing that reaches the filesystem is the
/// recoverable transaction the command layer publishes the finished tree
/// through. A module that opened a staging directory would have to unwind it on
/// every refusal path, and the one path that forgot would be the one that
/// leaked. It is also what makes the read local: no network client, no
/// subprocess, and no environment lookup can be reached from a module that
/// names none of them.
///
/// The property is structural because the alternative is to interrupt a real
/// extraction at each of its refusal points and inspect the filesystem after
/// each one, which tests the points someone remembered to enumerate rather than
/// the absence of the capability.
#[test]
fn test_package_archive_read_reaches_no_filesystem_network_or_process() {
    let archive = production_source(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/profile/package_archive.rs"),
    );
    for capability in [
        "fs::",
        "File::",
        "std::net",
        "ureq",
        "reqwest",
        "Command::",
        "env::",
        "tempfile",
    ] {
        assert!(
            !archive.contains(capability),
            "the package archive module reached for {capability}"
        );
    }
    for entry_point in [
        "pub fn pack_package_archive(",
        "pub fn read_package_archive(",
    ] {
        assert!(
            archive.contains(entry_point),
            "{entry_point} no longer lives here, so the absence of the capabilities above \
             establishes nothing"
        );
    }
}

/// A package tree reaches the worktree through one publication.
///
/// Capturing a tree from the repository files a manifest declares and adding
/// one from a portable archive differ in where the bytes came from, not in how
/// they land: both derive one plan against one captured image and publish it
/// through the shared recoverable transaction. A second publication route is
/// where the no-replace guarantee, the whole-tree delta, or the retry on a
/// concurrent change goes missing for one of them
/// (`@/inv/convention-convergence`).
#[test]
fn test_package_tree_publication_has_one_owner_in_the_command_layer() {
    let profile =
        production_source(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/profile.rs"));
    assert_eq!(
        profile.matches("finalize_package_tree_capture(").count(),
        1,
        "a package tree is finalized in more than one place in the profile commands"
    );
    assert_eq!(
        profile.matches("PackageTreeCapture::new(").count(),
        1,
        "a package tree publication is declared in more than one place in the profile commands"
    );
    for caller in [
        "TreePlacement::Republish",
        "TreePlacement::RequireAbsent",
        "fn publish_package_tree(",
    ] {
        assert!(
            profile.contains(caller),
            "{caller} is absent, so the counts above are not about the two publications"
        );
    }
}
