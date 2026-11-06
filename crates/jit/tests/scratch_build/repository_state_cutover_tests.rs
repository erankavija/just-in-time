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
             {expected:?}; the clean-cut lifecycle contract has one canonical package decoder and \
             model rather than parallel formats"
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

/// A manifest wire authority parses TOML, checks its `manifest-version`
/// discriminator, and maps the wire to `ProfilePackageModel`.
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

fn assert_directory_only_profile_sources(commands: &str) -> Result<(), String> {
    let resolution_errors = enum_body(commands, "pub enum ProfileResolutionError")
        .ok_or_else(|| "profile resolution error declaration disappeared".to_string())?;
    let retired_error_terms = ["embedded", "compiled", "provenance"]
        .into_iter()
        .filter(|term| resolution_errors.to_ascii_lowercase().contains(term))
        .collect::<Vec<_>>();
    if !retired_error_terms.is_empty() {
        return Err(format!(
            "forbidden source-only profile resolver error returned ({retired_error_terms:?}); a package resolves only from a repository directory"
        ));
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
    let lines = text.lines().collect::<Vec<_>>();
    let children = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let name = declared_module(line)?;
            // A `#[path]` module resolves beside the declaring file.
            let explicit = index
                .checked_sub(1)
                .and_then(|previous| lines[previous].trim().strip_prefix("#[path = \""))
                .and_then(|rest| rest.strip_suffix("\"]"))
                .map(|path| file.parent().unwrap().join(path));
            Some((name, explicit))
        })
        .flat_map(|(name, explicit)| {
            let child = explicit
                .map(|path| vec![path])
                .unwrap_or_else(|| {
                    vec![
                        directory.join(format!("{name}.rs")),
                        directory.join(name).join("mod.rs"),
                    ]
                })
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

/// The one function allowed to create a directory that can become the data
/// root: initialization's publisher stages the new root beside it, and
/// `publish_staged_data_root` renames that stage into place.
const DATA_ROOT_CREATOR: &str = "stage_new_data_root";

/// The publishers allowed to write repository state, each found by the
/// definition it owns, with the reason it may.
const REPOSITORY_STATE_PUBLISHERS: [(&str, &str); 3] = [
    (
        "pub fn commit_redo_record(",
        "the record is a mutation's commit point, written and synced in the data root's \
         scratch directory before any live file changes",
    ),
    (
        "pub struct RedoApplier {",
        "applying the record is how a committed mutation, or one a crash left, reaches the \
         roots",
    ),
    (
        "fn publish_staged_data_root(",
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
const NON_REPOSITORY_WRITES: [WriteClass; 8] = [
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
        "git directory",
        &["git_dir"],
        "hooks and the identity of a checkout over an external store live in the git \
         directory, which no repository root contains",
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
const OUTSIDE_ROOT_WRITES: [&str; 4] = [
    "claims control plane",
    "git directory",
    "export",
    "build script",
];

/// Directory creations inside a repository root whose target is never the
/// data root itself.
const IN_ROOT_DIRECTORIES: [WriteClass; 2] = [
    (
        "beneath a held root",
        &["CapabilityRoots", "DirectoryHandles"],
        "capability roots, and the directory handles walked beneath a held root, hold only \
         directories that exist, so a directory created through one lies beneath that root, \
         never at it",
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

/// Where a publisher's writes are sanctioned: every function of a module, or
/// one named function in it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Publisher {
    module: PathBuf,
    function: Option<String>,
}

impl Publisher {
    fn module(module: impl Into<PathBuf>) -> Self {
        Self {
            module: module.into(),
            function: None,
        }
    }

    fn function(module: impl Into<PathBuf>, function: &str) -> Self {
        Self {
            module: module.into(),
            function: Some(function.to_string()),
        }
    }

    fn covers(&self, function: &ProductionFunction) -> bool {
        self.module == function.module
            && self
                .function
                .as_ref()
                .is_none_or(|name| *name == function.name)
    }
}

/// What the writer scan found: the functions writing an unclassified target,
/// and the publishers and classes that matched a write.
#[derive(Debug, Default)]
struct WriterScan {
    unclassified: BTreeSet<String>,
    publishers_seen: BTreeSet<Publisher>,
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
    publishers: &BTreeSet<Publisher>,
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
            let publisher = publishers
                .iter()
                .find(|publisher| publisher.covers(function));
            match (publisher, class) {
                (Some(publisher), _) => {
                    scan.publishers_seen.insert(publisher.clone());
                }
                (None, Some((class, _, _))) => {
                    scan.classes_seen.insert(class);
                }
                (None, None) if !function.is_exported => delegated.push(function.name.clone()),
                (None, None) => {
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

/// The module owning each of `definitions`, as a whole-module publisher.
fn publisher_sources(
    sources: &[(PathBuf, String)],
    definitions: &[&str],
) -> Result<BTreeSet<Publisher>, String> {
    definitions
        .iter()
        .map(|definition| {
            let owners = sources
                .iter()
                .filter(|(_, source)| source.contains(definition))
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            match owners.as_slice() {
                [owner] => Ok(Publisher::module(owner.clone())),
                _ => Err(format!(
                    "`{definition}` is defined in {owners:?} rather than in exactly one source"
                )),
            }
        })
        .collect()
}

/// The one production function named `name`, as a single-function publisher.
fn publisher_function(functions: &[ProductionFunction], name: &str) -> Result<Publisher, String> {
    let owners = functions
        .iter()
        .filter(|function| function.name == name)
        .map(|function| function.module.clone())
        .collect::<Vec<_>>();
    match owners.as_slice() {
        [owner] => Ok(Publisher::function(owner.clone(), name)),
        _ => Err(format!(
            "`fn {name}` is defined in {owners:?} rather than exactly once"
        )),
    }
}

/// Every publisher and class was seen writing, and no writer is unclassified;
/// `rule` names what an unclassified writer breaks.
fn assert_writers_classified(
    scan: &WriterScan,
    publishers: &BTreeSet<Publisher>,
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
/// creation outside [`DATA_ROOT_CREATOR`] — the function, not its module — to
/// the classes whose target cannot be the data root.
#[test]
fn test_data_root_is_created_only_by_the_initialization_publisher() {
    let functions = production_functions(&workspace_compiled_sources());
    let publishers =
        BTreeSet::from([publisher_function(&functions, DATA_ROOT_CREATOR)
            .unwrap_or_else(|error| panic!("{error}"))]);
    let classes = non_data_root_directories();
    let scan = scan_filesystem_writers(&functions, &DIRECTORY_CREATIONS, &classes, &publishers);
    assert_writers_classified(&scan, &publishers, &classes, DATA_ROOT_CREATION_RULE)
        .unwrap_or_else(|error| panic!("{error}"));
}

/// Red-first proof for the data-root creation scan: a lock or identity writer
/// creating its file's directory is flagged although the repository-state scan
/// admits it, and so is a creator in the publisher's module outside the
/// publisher function, while a creation beneath a held root, in the publisher,
/// or in a private helper only the publisher calls is not.
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
            "pub fn stage(parent: &Dir, leaf: &str) {\n    \
             parent.create_dir(leaf).unwrap();\n    make_child(parent);\n}\n\n\
             fn make_child(parent: &Dir) {\n    parent.create_dir(\"child\").unwrap();\n}\n\n\
             pub fn scaffold(parent: &Dir) {\n    parent.create_dir(\".jit\").unwrap();\n}\n",
        ),
    ]);
    let init_publisher = BTreeSet::from([Publisher::function("storage/init_publish.rs", "stage")]);

    let admitted = scan_filesystem_writers(
        &writers,
        &FILESYSTEM_MUTATIONS,
        &NON_REPOSITORY_WRITES,
        &BTreeSet::from([Publisher::module("storage/init_publish.rs")]),
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
            "storage/init_publish.rs::scaffold".to_string(),
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

    let publishers = BTreeSet::from([Publisher::module("storage/record.rs")]);
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

#[test]
fn test_domain_layer_names_nothing_in_repository_state() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/domain");
    let offenders: Vec<_> = production_sources(&source_root)
        .into_iter()
        .filter(|(_, source)| {
            source.lines().any(|line| {
                let code = line.trim_start();
                !code.starts_with("//")
                    && (code.contains("crate::repository_state")
                        || code.contains("repository_state::"))
            })
        })
        .map(|(path, _)| path)
        .collect();
    assert!(
        offenders.is_empty(),
        "domain sources name repository_state: {offenders:?}"
    );
}

/// The package lifecycle has one decoder and one runtime model. These
/// ownership checks guard only declarations that can become a competing format
/// authority.
#[test]
fn test_profile_lifecycle_has_one_canonical_package_authority() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for (authority, expected, classify) in [
        (
            "manifest decoder",
            "profile/wire.rs",
            is_manifest_decoder as fn(&str) -> bool,
        ),
        ("package model", "profile/manifest.rs", is_package_model),
    ] {
        assert_canonical_authority(
            authority,
            authority_owners(&source_root, classify),
            PathBuf::from(expected),
        )
        .unwrap_or_else(|error| panic!("{error}"));
    }
}

/// Profile packages enter through one worktree-relative directory contract.
/// The source enums prove the runtime and reported-origin vocabularies, and the
/// live command schema proves every CLI output projection carries the same
/// single wire option. The archive plan's separately scoped `Embedded` variant
/// is intentionally outside these semantic profile surfaces.
#[test]
fn test_profile_lifecycle_has_only_the_repository_directory_package_source() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = crate_root.join("src");

    assert_directory_only_profile_sources(&production_source(
        &source_root.join("commands/profile.rs"),
    ))
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
    let manifest = production_source(&crate_root.join("src/profile/manifest.rs"));
    let cli = production_source(&crate_root.join("src/cli.rs"));
    let profile_cli = cli
        .split_once("/// Repository profile commands.")
        .expect("profile CLI declaration remains a separately documented command family")
        .1;
    let audit_types = production_source(&crate_root.join("src/domain/types.rs"));
    let profile_audit_inputs = audit_types
        .split_once("/// One profile's lifecycle result within an aggregate mutation.")
        .expect("profile lifecycle audit retains its per-profile result type")
        .1
        .split_once("/// The tagged body of an [`Event`]: its variant and payload fields.")
        .expect("profile lifecycle audit inputs remain separate from unrelated event types")
        .0;

    for (surface, description) in [
        ("profile manifest model", manifest),
        ("profile command-line definitions", profile_cli.to_string()),
        ("generated profile schema", profile_generated_schema_text()),
        (
            "profile lifecycle audit record",
            profile_audit_inputs.to_string(),
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

    let canonical_commands = r#"
pub enum ProfileResolutionError { UnreadableLocation { location: String, source: ProfilePackageError }, }
"#;
    assert!(assert_directory_only_profile_sources(canonical_commands).is_ok());

    let error = assert_directory_only_profile_sources(
        r#"
pub enum ProfileResolutionError { EmbeddedSourceCannotResolve { location: String }, }
"#,
    )
    .expect_err("the adversarial mutation must be rejected");
    assert!(
        error.contains("forbidden source-only profile resolver error returned"),
        "{error}"
    );

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
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let owners = definition_owners(&source.join("profile"), "_cap_fs_ext_follow");
    assert!(
        owners.is_empty(),
        "profile builds its own open options: {owners:?}"
    );
    assert_eq!(
        definition_owners(&source.join("storage"), "_cap_fs_ext_nonblock"),
        vec![PathBuf::from("capability_fs.rs")],
        "storage capability reads own the non-blocking no-follow options"
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

/// A package tree reaches the worktree through one publication: one plan
/// against one captured image, published through one mutation session whose
/// redo record is the commit point. A second route is where the no-replace
/// guarantee, the whole-tree delta, or the retry on a concurrent change goes
/// missing (`@/inv/convention-convergence`).
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
    assert!(
        profile.contains("fn publish_package_tree("),
        "fn publish_package_tree( is absent, so the counts above are not about the publication"
    );
}
