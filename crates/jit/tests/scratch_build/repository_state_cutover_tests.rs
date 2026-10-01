//! Known current-tree regressions forbidden by the repository-state cutover.
//!
//! These are the properties visibility cannot express, because `std::fs` is
//! public to every crate: deleted modules and wrappers, canonical renderer
//! ownership, the one mechanism that writes repository state, and the modules
//! that must reach the filesystem only through a named discipline or not at
//! all. Each test also asserts the mechanism it is about is still present, so
//! an absent call shape can only mean the discipline held rather than that the
//! code it was about moved away.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// A `cfg` attribute that compiles its item only into test builds.
fn is_test_only_cfg(line: &str) -> bool {
    let line = line.trim_start();
    line.strip_prefix("#[cfg(")
        .or_else(|| line.strip_prefix("#![cfg("))
        .is_some_and(|predicate| !predicate.starts_with("not(") && predicate.contains("test"))
}

/// The text a non-test build compiles from `source`, or `None` for a file only
/// tests compile.
///
/// An item behind a test-only `cfg` is dropped through the line closing it at
/// its own indentation, which rustfmt guarantees.
fn production_text(source: &str) -> Option<String> {
    if source
        .lines()
        .any(|line| line.trim_start().starts_with("#![") && is_test_only_cfg(line))
    {
        return None;
    }
    let indentation = |line: &str| line.len() - line.trim_start().len();
    let mut lines = source.lines();
    let mut compiled = String::new();
    while let Some(line) = lines.next() {
        if is_test_only_cfg(line) {
            let item = lines
                .by_ref()
                .find(|item| !item.trim_start().starts_with("#["));
            if item.is_some_and(|item| !item.trim_end().ends_with([';', '}'])) {
                // Advances past the line closing the item.
                lines.by_ref().any(|closing| {
                    indentation(closing) == indentation(line)
                        && closing.trim_start().starts_with(['}', ')', ']'])
                });
            }
        } else {
            compiled.push_str(line);
            compiled.push('\n');
        }
    }
    Some(compiled)
}

fn production_source(path: &Path) -> String {
    production_text(&std::fs::read_to_string(path).unwrap()).unwrap_or_default()
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
/// receive a path from a caller and only decode that one record. The image's
/// `directory_listings()` accessor marks the listing read, since it is the one
/// way a producer learns which record names exist.
fn is_applied_record_inventory_reader(source: &str) -> bool {
    source.contains("Result<Vec<(VirtualPath, AppliedProfileRecord)>")
        && source.contains("directory_listings()")
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

fn enum_body<'a>(source: &'a str, declaration: &str) -> Option<&'a str> {
    let start = source.find(declaration)? + declaration.len();
    let open = source[start..].find('{')? + start;
    let mut depth = 0usize;
    source[open..]
        .char_indices()
        .find_map(|(offset, character)| {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&source[open + 1..open + offset]);
                    }
                }
                _ => {}
            }
            None
        })
}

fn top_level_enum_entries(body: &str) -> Vec<String> {
    let body = body
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            !line.starts_with("//") && !line.starts_with('#')
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut entries = Vec::new();
    for (index, character) in body.char_indices() {
        match character {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let entry = body[start..index].trim();
                if !entry.is_empty() {
                    entries.push(entry.to_string());
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    entries
}

fn top_level_enum_variants(body: &str) -> Vec<String> {
    top_level_enum_entries(body)
        .into_iter()
        .map(|entry| {
            entry
                .split(|character: char| {
                    character == '(' || character == '{' || character.is_whitespace()
                })
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

fn functions_with_shape<'a>(source: &'a str, required: &[&str]) -> Vec<&'a str> {
    function_bodies(source)
        .into_iter()
        .filter(|function| required.iter().all(|needle| function.contains(needle)))
        .collect()
}

fn profile_origin_schema_definitions(value: &serde_json::Value) -> Vec<&serde_json::Value> {
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .flat_map(profile_origin_schema_definitions)
            .collect(),
        serde_json::Value::Object(values) => values
            .iter()
            .flat_map(|(key, value)| {
                let mut definitions = (key == "ProfileOrigin")
                    .then_some(value)
                    .into_iter()
                    .collect::<Vec<_>>();
                definitions.extend(profile_origin_schema_definitions(value));
                definitions
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn tagged_source_values(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(values) => values.iter().flat_map(tagged_source_values).collect(),
        serde_json::Value::Object(values) => {
            let local = values
                .get("properties")
                .and_then(serde_json::Value::as_object)
                .and_then(|properties| properties.get("source"))
                .and_then(|source| source.get("enum"))
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string);
            local
                .chain(values.values().flat_map(tagged_source_values))
                .collect()
        }
        _ => Vec::new(),
    }
}

fn assert_directory_only_profile_sources(
    domain: &str,
    package: &str,
    commands: &str,
    schema: &serde_json::Value,
) -> Result<(), String> {
    for (surface, declaration, source) in [
        ("applied profile origin", "pub enum ProfileOrigin", domain),
        (
            "runtime profile package source",
            "pub enum ProfilePackageSource",
            package,
        ),
    ] {
        let body = enum_body(source, declaration)
            .ok_or_else(|| format!("{surface} declaration disappeared: {declaration}"))?;
        let variants = top_level_enum_variants(body);
        if variants != ["Directory"] {
            return Err(format!(
                "forbidden non-directory {surface}: found variants {variants:?}; profile packages and their applied records have one repository-directory source"
            ));
        }
    }

    let resolution_errors = enum_body(commands, "pub enum ProfileResolutionError")
        .ok_or_else(|| "profile resolution error declaration disappeared".to_string())?;
    let record_only_errors = top_level_enum_entries(resolution_errors)
        .into_iter()
        .filter(|entry| {
            entry.contains("record: String")
                && !entry.contains("location: String")
                && !entry.contains("source: ProfilePackageError")
        })
        .collect::<Vec<_>>();
    if !record_only_errors.is_empty() {
        return Err(format!(
            "forbidden record-only profile resolver error returned ({record_only_errors:?}); an unsupported package source is not a resolver case and must fail at strict applied-record decoding"
        ));
    }
    let retired_error_terms = ["embedded", "compiled", "provenance"]
        .into_iter()
        .filter(|term| resolution_errors.to_ascii_lowercase().contains(term))
        .collect::<Vec<_>>();
    if !retired_error_terms.is_empty() {
        return Err(format!(
            "forbidden source-only profile resolver error returned ({retired_error_terms:?}); unsupported origins must fail at strict applied-record decoding"
        ));
    }

    let exclusion_scans =
        functions_with_shape(commands, &["capture_applied_records(", "record.origin"]);
    if !exclusion_scans.is_empty() {
        return Err(
            "forbidden applied-record source exclusion scan returned; every applied record must resolve through recorded_package".to_string(),
        );
    }
    let graph_routes = functions_with_shape(
        commands,
        &[
            "Result<ResolvedProfileGraph>",
            "self.resolve_profile_graph_with_applied(",
        ],
    );
    if graph_routes.len() != 1 {
        return Err(format!(
            "forbidden profile graph resolver bypass: found {} graph entry points feeding the applied-package graph, expected the one canonical route",
            graph_routes.len()
        ));
    }

    let origins = profile_origin_schema_definitions(schema);
    if origins.is_empty() {
        return Err("generated CommandSchema exposes no ProfileOrigin definitions, so its source contract cannot be verified".to_string());
    }
    for origin in origins {
        let mut sources = tagged_source_values(origin);
        sources.sort();
        sources.dedup();
        if sources != ["directory"] {
            return Err(format!(
                "forbidden generated ProfileOrigin source options {sources:?}; every live CommandSchema occurrence must expose only `directory`"
            ));
        }
    }
    Ok(())
}

fn assert_no_compiled_profile_package_source(
    sources: impl IntoIterator<Item = (PathBuf, String)>,
) -> Result<(), String> {
    let violations = sources
        .into_iter()
        .filter(|(_, source)| {
            source.contains("ProfilePackage")
                && ["include_bytes!", "include_str!", "include_dir!"]
                    .into_iter()
                    .any(|capability| source.contains(capability))
        })
        .map(|(path, _)| path.display().to_string())
        .collect::<Vec<_>>();
    violations.is_empty().then_some(()).ok_or_else(|| {
        format!(
            "forbidden compiled profile package source returned in {violations:?}; production package bytes enter only through repository directories"
        )
    })
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

/// `item` with its leading visibility removed.
fn without_visibility(item: &str) -> &str {
    let item = item.trim_start();
    match item.strip_prefix("pub") {
        Some(rest) if rest.starts_with('(') => rest.split_once(") ").map_or(item, |(_, rest)| rest),
        Some(rest) if rest.starts_with(' ') => rest.trim_start(),
        _ => item,
    }
}

/// The module an out-of-line `mod name;` declaration on `line` names.
fn declared_module(line: &str) -> Option<&str> {
    without_visibility(line)
        .strip_prefix("mod ")?
        .trim_end()
        .strip_suffix(';')
}

/// `file` and every module file it declares, each with its [`production_text`];
/// a module a non-test build leaves out contributes nothing. A crate root, like
/// a `mod.rs`, declares its modules beside itself.
fn compiled_modules(file: &Path, is_crate_root: bool) -> Vec<(PathBuf, String)> {
    let Some(text) = production_text(&std::fs::read_to_string(file).unwrap()) else {
        return Vec::new();
    };
    let directory = match is_crate_root || file.file_name().is_some_and(|name| name == "mod.rs") {
        true => file.parent().unwrap().to_path_buf(),
        false => file.with_extension(""),
    };
    let children = text
        .lines()
        .filter_map(declared_module)
        .flat_map(|name| {
            let child = [
                directory.join(format!("{name}.rs")),
                directory.join(name).join("mod.rs"),
            ]
            .into_iter()
            .find(|candidate| candidate.is_file())
            .unwrap_or_else(|| {
                panic!(
                    "module `{name}` in {} resolves to no file, so this walk would miss what \
                     it compiles",
                    file.display()
                )
            });
            compiled_modules(&child, false)
        })
        .collect::<Vec<_>>();
    std::iter::once((file.to_path_buf(), text))
        .chain(children)
        .collect()
}

/// Every source file a non-test build of a workspace member compiles, once
/// each, keyed by its workspace-relative path.
///
/// The roots are the targets Cargo itself resolves for each member — its
/// library, every binary whether declared or discovered under `src/bin`, and
/// its build script — so a target added by convention cannot fall outside the
/// walk. Test, bench, and example targets ship nothing.
fn workspace_compiled_sources() -> Vec<(PathBuf, String)> {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--offline",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata runs");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata = serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
    let workspace = PathBuf::from(metadata["workspace_root"].as_str().unwrap());
    metadata["packages"]
        .as_array()
        .expect("cargo metadata lists the workspace members")
        .iter()
        .flat_map(|package| package["targets"].as_array().unwrap())
        .filter(|target| {
            target["kind"]
                .as_array()
                .unwrap()
                .iter()
                .all(|kind| !matches!(kind.as_str(), Some("test" | "bench" | "example")))
        })
        .flat_map(|target| compiled_modules(Path::new(target["src_path"].as_str().unwrap()), true))
        .map(|(path, text)| (path.strip_prefix(&workspace).unwrap().to_path_buf(), text))
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .collect()
}

/// Calls that create, replace, rename, truncate, or remove a filesystem entry,
/// by path or through an open directory handle.
const FILESYSTEM_MUTATIONS: [&str; 22] = [
    "fs::write(",
    "fs::rename(",
    "fs::create_dir",
    "fs::remove_",
    "fs::copy(",
    "fs::hard_link(",
    "fs::set_permissions(",
    "File::create(",
    ".write(true)",
    ".append(true)",
    ".create(true)",
    ".create_new(true)",
    ".create_dir(",
    ".create_dir_all(",
    ".remove_file(",
    ".remove_dir(",
    ".remove_dir_all(",
    ".rename(",
    ".set_len(",
    ".set_permissions(",
    ".persist(",
    "renameat",
];

/// Calls that create a directory, by path or through an open directory handle.
const DIRECTORY_CREATIONS: [&str; 3] = ["fs::create_dir", ".create_dir(", ".create_dir_all("];

/// The definition owning initialization's publisher, the one writer allowed to
/// create an absent data root.
const DATA_ROOT_PUBLISHER: &str = "fn publish_staged_data_root(";

/// The publishers allowed to write repository state, each found by the
/// definition it owns, with the reason it may.
const REPOSITORY_STATE_PUBLISHERS: [(&str, &str); 3] = [
    (
        "pub struct RedoRecord {",
        "the record is a mutation's commit point, written and synced in the data root's \
         scratch directory before any live file changes",
    ),
    (
        "pub struct RedoApplier {",
        "applying the record is how a committed mutation, or one a crash left, reaches the \
         roots",
    ),
    (
        DATA_ROOT_PUBLISHER,
        "an absent data root cannot be created through the record, whose home is a scratch \
         directory inside that root; it is staged beside it and published by a no-replace \
         rename instead",
    ),
];

/// A class of writes: its name, the markers recognising it in the writing
/// function's code, and the reason its target is safe.
type WriteClass = (&'static str, &'static [&'static str], &'static str);

/// Writes whose target lies outside repository state, each class recognised by
/// the shape that fixes its target: any one of its markers in the writing
/// function's code.
const NON_REPOSITORY_WRITES: [WriteClass; 9] = [
    (
        "advisory lock",
        &[
            "lock_exclusive",
            "lock_shared",
            "Fs4FileExt::unlock",
            "\"lock.meta\"",
            "== \"lock\"",
        ],
        "a lock file and its metadata coordinate writers and carry no repository data",
    ),
    (
        "claims control plane",
        &["shared_jit"],
        "leases live under the git common directory, shared by every worktree and outside \
         each one's roots",
    ),
    (
        "temporary debris",
        &["find_temp_files("],
        "an orphaned `.tmp` file is the leftover of a write that never published",
    ),
    (
        "server process state",
        &["pid_file_path(", "log_file"],
        "the pid file and server log are machine-local files the storage format names",
    ),
    (
        "worktree identity",
        &["WorktreeIdentity"],
        "the identity file is machine-local and names one checkout, not repository content",
    ),
    (
        "user configuration",
        &["UserConfigRoot"],
        "it lives in the user's configuration directory, outside every repository",
    ),
    (
        "git hooks",
        &["git_dir"],
        "hooks live in the git directory, which no repository root contains",
    ),
    (
        "export",
        &["ExternalExportPath", "tempfile::"],
        "an export destination is proven outside both roots before it is typed, and a \
         snapshot is staged in a private temporary directory",
    ),
    (
        "build script",
        &["\"cargo:"],
        "a build script runs at compile time against the source checkout, before any \
         repository exists",
    ),
];

/// The [`NON_REPOSITORY_WRITES`] classes whose targets lie outside every
/// repository root, so a directory they create is never the data root. Lock,
/// identity, debris, and server-state targets live inside the data root, where
/// creating a parent directory can create the root itself.
const OUTSIDE_ROOT_WRITES: [&str; 5] = [
    "claims control plane",
    "git hooks",
    "export",
    "user configuration",
    "build script",
];

/// Directory creations inside a repository root whose target is never the
/// data root itself.
const IN_ROOT_DIRECTORIES: [WriteClass; 2] = [
    (
        "beneath a held root",
        &["CapabilityRoots"],
        "capability roots hold only roots that exist, so a directory created through one \
         lies beneath that root, never at it",
    ),
    (
        "redo record scratch",
        &["RECORD_SCRATCH_DIR"],
        "the record's scratch directory is created non-recursively inside a data root that \
         already exists",
    ),
];

/// Directory creations whose target cannot be the data root.
fn non_data_root_directories() -> Vec<WriteClass> {
    NON_REPOSITORY_WRITES
        .into_iter()
        .filter(|(class, _, _)| OUTSIDE_ROOT_WRITES.contains(class))
        .chain(IN_ROOT_DIRECTORIES)
        .collect()
}

/// Each function's text, from its signature line to the next signature.
fn function_bodies(source: &str) -> Vec<&str> {
    let starts = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(index, _)| index + 1))
        .filter(|start| {
            let line = source[*start..].lines().next().unwrap_or_default();
            ["const ", "async ", "unsafe "]
                .into_iter()
                .fold(without_visibility(line), |item, qualifier| {
                    item.strip_prefix(qualifier).unwrap_or(item)
                })
                .starts_with("fn ")
        })
        .collect::<Vec<_>>();
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            &source[*start..starts.get(index + 1).copied().unwrap_or(source.len())]
        })
        .collect()
}

/// One production function: where it lives, its name, whether it is visible
/// beyond its own module tree, and its code with comment lines set aside.
struct ProductionFunction {
    module: PathBuf,
    name: String,
    is_exported: bool,
    code: String,
}

fn production_functions(sources: &[(PathBuf, String)]) -> Vec<ProductionFunction> {
    sources
        .iter()
        .flat_map(|(module, source)| {
            function_bodies(source).into_iter().map(|function| {
                let signature = function.lines().next().unwrap_or_default().trim_start();
                let name = signature
                    .split_once("fn ")
                    .map_or("", |(_, rest)| rest)
                    .split(|character: char| !(character.is_alphanumeric() || character == '_'))
                    .next()
                    .unwrap_or_default();
                ProductionFunction {
                    module: module.clone(),
                    name: name.to_string(),
                    is_exported: signature.starts_with("pub fn ")
                        || signature.starts_with("pub(crate) "),
                    code: function
                        .lines()
                        .filter(|line| !line.trim_start().starts_with("//"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                }
            })
        })
        .collect()
}

/// Whether `code` names `function` other than by defining it.
fn names_function(code: &str, function: &str) -> bool {
    let is_identifier = |character: char| character.is_alphanumeric() || character == '_';
    code.match_indices(function).any(|(start, _)| {
        let before = &code[..start];
        let after = &code[start + function.len()..];
        !before.ends_with(is_identifier)
            && !after.starts_with(is_identifier)
            && !before.ends_with("fn ")
    })
}

/// What the writer scan found: the functions writing an unclassified target,
/// and the publishers and classes that matched a write.
#[derive(Debug, Default)]
struct WriterScan {
    unclassified: BTreeSet<String>,
    publishers_seen: BTreeSet<PathBuf>,
    classes_seen: BTreeSet<&'static str>,
}

/// Classify every function that performs one of `mutations`.
///
/// A write inside a publisher is the sanctioned route, and calling a publisher
/// is going through it. Any other writer must match one of `classes`. A writer
/// no class matches and that its own
/// module tree confines — a private helper or a storage-private primitive —
/// hands the question to its callers: each function naming it is a writer in
/// turn, wherever its path came from. An exported writer no class matches is
/// unclassified.
fn scan_filesystem_writers(
    functions: &[ProductionFunction],
    mutations: &[&str],
    classes: &[WriteClass],
    publishers: &BTreeSet<PathBuf>,
) -> WriterScan {
    let mut scan = WriterScan::default();
    let mut classified = BTreeSet::new();
    let mut delegated = Vec::<String>::new();
    loop {
        let writers = functions
            .iter()
            .enumerate()
            .filter(|(index, function)| {
                !classified.contains(index)
                    && (mutations
                        .iter()
                        .any(|mutation| function.code.contains(mutation))
                        || delegated
                            .iter()
                            .any(|helper| names_function(&function.code, helper)))
            })
            .collect::<Vec<_>>();
        if writers.is_empty() {
            return scan;
        }
        for (index, function) in writers {
            classified.insert(index);
            let class = classes.iter().find(|(_, markers, _)| {
                markers.iter().any(|marker| function.code.contains(marker))
            });
            match (publishers.contains(&function.module), class) {
                (true, _) => {
                    scan.publishers_seen.insert(function.module.clone());
                }
                (false, Some((class, _, _))) => {
                    scan.classes_seen.insert(class);
                }
                (false, None) if !function.is_exported => delegated.push(function.name.clone()),
                (false, None) => {
                    scan.unclassified.insert(format!(
                        "{}::{}",
                        function.module.display(),
                        function.name
                    ));
                }
            }
        }
    }
}

/// The source owning each of `definitions`.
fn publisher_sources(
    sources: &[(PathBuf, String)],
    definitions: &[&str],
) -> Result<BTreeSet<PathBuf>, String> {
    definitions
        .iter()
        .map(|definition| {
            let owners = sources
                .iter()
                .filter(|(_, source)| source.contains(definition))
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            match owners.as_slice() {
                [owner] => Ok(owner.clone()),
                _ => Err(format!(
                    "`{definition}` is defined in {owners:?} rather than in exactly one source"
                )),
            }
        })
        .collect()
}

/// Every publisher and class was seen writing, and no writer is unclassified;
/// `rule` names what an unclassified writer breaks.
fn assert_writers_classified(
    scan: &WriterScan,
    publishers: &BTreeSet<PathBuf>,
    classes: &[WriteClass],
    rule: &str,
) -> Result<(), String> {
    let unseen_publishers = publishers
        .difference(&scan.publishers_seen)
        .collect::<Vec<_>>();
    let unseen_classes = classes
        .iter()
        .map(|(class, _, _)| *class)
        .filter(|class| !scan.classes_seen.contains(class))
        .collect::<Vec<_>>();
    if !unseen_publishers.is_empty() || !unseen_classes.is_empty() {
        return Err(format!(
            "the scan no longer sees publishers {unseen_publishers:?} or classes \
             {unseen_classes:?} write, so what it classifies establishes nothing until it \
             does"
        ));
    }
    scan.unclassified.is_empty().then_some(()).ok_or_else(|| {
        format!(
            "filesystem writes to an unclassified target in {:?}; {rule}",
            scan.unclassified
        )
    })
}

/// Repository state reaches the filesystem through the redo record, and through
/// the absent-root publisher only where the record cannot exist yet.
///
/// Repository state is what `docs/reference/storage-format.md` versions: the
/// data root's contents and the worktree files a mutation publishes. Every
/// function a non-test build of the workspace compiles that mutates the
/// filesystem is therefore either a publisher's, one whose target a
/// [`NON_REPOSITORY_WRITES`] class places outside repository state, or a
/// confined helper whose callers answer for it. Where a write's path came from
/// does not matter: an unclassified writer fails. `std::fs` is public to every
/// crate, so no visibility rule can close this.
#[test]
fn test_repository_state_is_written_only_by_the_redo_record_and_the_absent_root_publisher() {
    let sources = workspace_compiled_sources();
    let definitions = REPOSITORY_STATE_PUBLISHERS.map(|(definition, _)| definition);
    let publishers =
        publisher_sources(&sources, &definitions).unwrap_or_else(|error| panic!("{error}"));
    let scan = scan_filesystem_writers(
        &production_functions(&sources),
        &FILESYSTEM_MUTATIONS,
        &NON_REPOSITORY_WRITES,
        &publishers,
    );
    assert_writers_classified(
        &scan,
        &publishers,
        &NON_REPOSITORY_WRITES,
        REPOSITORY_STATE_RULE,
    )
    .unwrap_or_else(|error| panic!("{error}"));
}

/// What an unclassified repository-state writer breaks.
const REPOSITORY_STATE_RULE: &str = "a mutation reaches the repository's roots only through its \
     session's redo record (`@/charter/D-17`), and any other write names the non-repository \
     class it belongs to";

/// What an unclassified directory creation breaks.
const DATA_ROOT_CREATION_RULE: &str = "an absent data root is created only by initialization's \
     publisher, and any other directory creation names the class that keeps its target from \
     being the data root";

/// An absent data root is created only by initialization's publisher.
///
/// The repository-state scan admits lock and identity writes by their target
/// file, which lives inside the data root, so it cannot see such a writer
/// creating the root as that file's parent. This scan holds every directory
/// creation to the classes whose target cannot be the data root.
#[test]
fn test_data_root_is_created_only_by_the_initialization_publisher() {
    let sources = workspace_compiled_sources();
    let publishers = publisher_sources(&sources, &[DATA_ROOT_PUBLISHER])
        .unwrap_or_else(|error| panic!("{error}"));
    let classes = non_data_root_directories();
    let scan = scan_filesystem_writers(
        &production_functions(&sources),
        &DIRECTORY_CREATIONS,
        &classes,
        &publishers,
    );
    assert_writers_classified(&scan, &publishers, &classes, DATA_ROOT_CREATION_RULE)
        .unwrap_or_else(|error| panic!("{error}"));
}

/// Red-first proof for the data-root creation scan: a lock or identity writer
/// creating its file's directory is flagged although the repository-state scan
/// admits it, while a creation beneath a held root or in the initialization
/// publisher is not.
#[test]
fn test_data_root_creation_scan_flags_lock_and_identity_writers_creating_their_directory() {
    let functions = |fragments: &[(&str, &str)]| {
        production_functions(
            &fragments
                .iter()
                .map(|(module, fragment)| {
                    (PathBuf::from(module), production_text(fragment).unwrap())
                })
                .collect::<Vec<_>>(),
        )
    };
    let writers = functions(&[
        (
            "storage/repo_lock.rs",
            "pub fn acquire(path: &Path) {\n    \
             std::fs::create_dir_all(path.parent().unwrap()).unwrap();\n    \
             locker.lock_exclusive(path);\n}\n",
        ),
        (
            "storage/worktree_identity.rs",
            "pub fn record(paths: &WorktreePaths, identity: &WorktreeIdentity) {\n    \
             fs::create_dir_all(&paths.local_jit).unwrap();\n}\n",
        ),
        (
            "storage/redo_apply.rs",
            "fn make(roots: &CapabilityRoots, leaf: &str) {\n    \
             roots.data().create_dir(leaf).unwrap();\n}\n",
        ),
        (
            "storage/init_publish.rs",
            "pub fn stage(parent: &Dir, leaf: &str) {\n    parent.create_dir(leaf).unwrap();\n}\n",
        ),
    ]);
    let init_publisher = BTreeSet::from([PathBuf::from("storage/init_publish.rs")]);

    let admitted = scan_filesystem_writers(
        &writers,
        &FILESYSTEM_MUTATIONS,
        &NON_REPOSITORY_WRITES,
        &init_publisher,
    );
    assert!(admitted.unclassified.is_empty(), "{admitted:?}");

    let flagged = scan_filesystem_writers(
        &writers,
        &DIRECTORY_CREATIONS,
        &non_data_root_directories(),
        &init_publisher,
    );
    assert_eq!(
        flagged.unclassified,
        BTreeSet::from([
            "storage/repo_lock.rs::acquire".to_string(),
            "storage/worktree_identity.rs::record".to_string(),
        ])
    );
}

/// Red-first proof for the writer scan: a helper writing a path it was handed
/// is flagged at the first caller no class covers, whichever module calls it,
/// and neither a classified write, a test-only item, nor a comment is.
#[test]
fn test_repository_state_writer_scan_flags_unclassified_writers_wherever_their_path_comes_from() {
    let scan_fragments = |fragments: &[(&str, &str)]| {
        let sources = fragments
            .iter()
            .map(|(module, fragment)| (PathBuf::from(module), production_text(fragment).unwrap()))
            .collect::<Vec<_>>();
        scan_filesystem_writers(
            &production_functions(&sources),
            &FILESYSTEM_MUTATIONS,
            &NON_REPOSITORY_WRITES,
            &BTreeSet::new(),
        )
        .unclassified
    };
    let helper =
        "fn put(path: &Path, bytes: &[u8]) {\n    std::fs::write(path, bytes).unwrap();\n}\n";
    let caller = "pub fn save(layout: &RepositoryLayout) {\n    \
                  put(&layout.data_root().join(\"index.json\"), b\"{}\");\n}\n";
    assert_eq!(
        scan_fragments(&[("commands/save.rs", &format!("{helper}{caller}"))]),
        BTreeSet::from(["commands/save.rs::save".to_string()])
    );
    assert_eq!(
        scan_fragments(&[
            ("storage/put.rs", &format!("pub(crate) {helper}")),
            ("commands/save.rs", caller),
        ]),
        BTreeSet::from(["storage/put.rs::put".to_string()])
    );
    assert_eq!(
        scan_fragments(&[(
            "storage/lock.rs",
            "pub fn lock(path: &Path) {\n    std::fs::write(path.with_extension(\"lock.meta\"), b\"\").unwrap();\n    \
             locker.lock_exclusive(path);\n}\n\n#[cfg(all(test, unix))]\nmod tests {\n    \
             pub fn seed(path: &Path) {\n        std::fs::write(path, b\"\").unwrap();\n    }\n}\n\n\
             pub fn read(path: &Path) {\n    // std::fs::write(path, b\"\")\n}\n",
        )]),
        BTreeSet::new()
    );
    assert_eq!(
        production_text("#![cfg(any(test, feature = \"test-support\"))]\nfn seed() {}\n"),
        None
    );

    let record = PathBuf::from("storage/record.rs");
    let publishers = BTreeSet::from([record.clone()]);
    let complete = WriterScan {
        unclassified: BTreeSet::from(["commands/save.rs::save".to_string()]),
        publishers_seen: publishers.clone(),
        classes_seen: NON_REPOSITORY_WRITES
            .iter()
            .map(|(class, _, _)| *class)
            .collect(),
    };
    let direct = assert_writers_classified(
        &complete,
        &publishers,
        &NON_REPOSITORY_WRITES,
        REPOSITORY_STATE_RULE,
    )
    .unwrap_err();
    assert!(
        direct.contains("unclassified target in {\"commands/save.rs::save\"}"),
        "{direct}"
    );
    let blind = assert_writers_classified(
        &WriterScan {
            classes_seen: BTreeSet::new(),
            ..complete
        },
        &publishers,
        &NON_REPOSITORY_WRITES,
        REPOSITORY_STATE_RULE,
    )
    .unwrap_err();
    assert!(blind.contains("no longer sees"), "{blind}");
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

/// Profile packages enter through one worktree-relative directory contract.
/// The source enums prove the runtime and record vocabularies, the graph shape
/// proves mutations cannot silently omit records based on provenance, and the
/// live command schema proves every CLI/MCP output projection carries the same
/// single wire option. The archive plan's separately scoped `Embedded` variant
/// is intentionally outside these semantic profile surfaces.
#[test]
fn test_profile_lifecycle_has_only_the_repository_directory_package_source() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = crate_root.join("src");
    let schema = serde_json::to_value(jit::CommandSchema::generate())
        .expect("the generated command schema serializes");

    assert_directory_only_profile_sources(
        &production_source(&source_root.join("domain/types.rs")),
        &production_source(&source_root.join("profile/package.rs")),
        &production_source(&source_root.join("commands/profile.rs")),
        &schema,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    assert_no_compiled_profile_package_source(production_sources(&source_root.join("profile")))
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
            "fn applied_profile_records() -> Result<Vec<(VirtualPath, AppliedProfileRecord)>> { directory_listings(); }",
            "fn collect_installed_provenance() -> Result<Vec<(VirtualPath, AppliedProfileRecord)>> { directory_listings(); }",
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

    let directory_domain = "pub enum ProfileOrigin { Directory(RootRelativePath), }";
    let directory_package = "pub enum ProfilePackageSource { Directory(PathBuf), }";
    let canonical_commands = r#"
pub enum ProfileResolutionError { UnresolvableRecordedLocation { record: String, location: String, source: ProfilePackageError }, }
    fn resolve_profile_graph(&self) -> Result<ResolvedProfileGraph> {
        self.resolve_profile_graph_with_applied(selected, self.resolve_applied_profile_packages()?)
    }
"#;
    let directory_schema = serde_json::json!({
        "definitions": {
            "ProfileOrigin": {
                "properties": {
                    "source": { "type": "string", "enum": ["directory"] },
                    "location": { "type": "string" }
                }
            }
        }
    });
    assert!(assert_directory_only_profile_sources(
        directory_domain,
        directory_package,
        canonical_commands,
        &directory_schema,
    )
    .is_ok());

    for (domain, package, commands, schema, expected) in [
        (
            "pub enum ProfileOrigin { Directory(RootRelativePath), BinaryResident, }",
            directory_package,
            canonical_commands,
            directory_schema.clone(),
            "forbidden non-directory applied profile origin",
        ),
        (
            directory_domain,
            "pub enum ProfilePackageSource { Directory(PathBuf), StaticBundle, }",
            canonical_commands,
            directory_schema.clone(),
            "forbidden non-directory runtime profile package source",
        ),
        (
            directory_domain,
            directory_package,
            r#"
pub enum ProfileResolutionError { StaticRecordCannotResolve { record: String }, }
    fn resolve_profile_graph(&self) -> Result<ResolvedProfileGraph> {
        self.resolve_profile_graph_with_applied(selected, self.resolve_applied_profile_packages()?)
    }
"#,
            directory_schema.clone(),
            "forbidden record-only profile resolver error returned",
        ),
        (
            directory_domain,
            directory_package,
            r#"
pub enum ProfileResolutionError { UnresolvableRecordedLocation { source: ProfilePackageError }, }
    fn resolve_profile_graph(&self) -> Result<ResolvedProfileGraph> {
        self.resolve_profile_graph_with_applied(selected, self.resolve_applied_profile_packages()?)
    }
    fn collect_ownership_only_ids(&self) -> Result<BTreeSet<String>> {
        capture_applied_records(session, path)?;
        read(record.origin)
    }
"#,
            directory_schema.clone(),
            "forbidden applied-record source exclusion scan returned",
        ),
        (
            directory_domain,
            directory_package,
            r#"
pub enum ProfileResolutionError { UnresolvableRecordedLocation { source: ProfilePackageError }, }
    fn resolve_profile_graph(&self) -> Result<ResolvedProfileGraph> {
        self.resolve_profile_graph_with_applied(selected, self.resolve_applied_profile_packages()?)
    }
    fn resolve_mutating_graph(&self) -> Result<ResolvedProfileGraph> {
        self.resolve_profile_graph_with_applied(selected, filtered_applied()?)
    }
"#,
            directory_schema.clone(),
            "forbidden profile graph resolver bypass",
        ),
        (
            directory_domain,
            directory_package,
            canonical_commands,
            serde_json::json!({
                "definitions": {
                    "ProfileOrigin": {
                        "oneOf": [
                            { "properties": { "source": { "enum": ["directory"] } } },
                            { "properties": { "source": { "enum": ["binary"] } } }
                        ]
                    }
                }
            }),
            "forbidden generated ProfileOrigin source options",
        ),
    ] {
        let error = assert_directory_only_profile_sources(domain, package, commands, &schema)
            .err()
            .unwrap_or_else(|| {
                panic!("the adversarial mutation for `{expected}` must be rejected")
            });
        assert!(error.contains(expected), "{error}");
    }

    let compiled_source = assert_no_compiled_profile_package_source([(
        PathBuf::from("profile/static_bundle.rs"),
        "static WORKFLOW: &[u8] = include_bytes!(\"workflow.tar\"); fn package() -> ProfilePackage { decode(WORKFLOW) }".to_string(),
    )])
    .unwrap_err();
    assert!(
        compiled_source.contains("forbidden compiled profile package source returned")
            && compiled_source.contains("profile/static_bundle.rs")
            && compiled_source
                .contains("production package bytes enter only through repository directories"),
        "{compiled_source}"
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
/// produces a value, and the finished tree reaches the filesystem only through
/// the redo record a mutation session commits once the whole tree exists. A
/// module that wrote the tree itself would have to unwind it on
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
/// through one mutation session, whose redo record is the commit point either
/// route reaches. A second publication route is
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
