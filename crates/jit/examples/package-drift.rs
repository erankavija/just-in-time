//! Check this repository's own registries against what its package declares.
//!
//! A repository-local check run by a configured gate: it reads `.jit/*.toml`,
//! whose contents the Rust test suite must not depend on.
//!
//! Usage:
//!   package-drift [ROOT]    (ROOT holds the `.jit/` to compare; default: the
//!                            checkout these sources were compiled from)
//!
//! Exit codes:
//!   0 — every generated region holds the package and every gate input holds
//!   1 — findings, one per line on stderr
//!   2 — a usage error

use jit::commands::{open_repository, RepositoryInvocation};
use jit::declarations::{GateDefinition, GateRegistry};
use jit::domain::repository_inputs::RepositoryInputs;
use jit::profile::registry_region::{
    RegistryRegion, INVARIANT_REGION, REGION_PACKAGE_ID, TEMPLATE_REGION,
};
use jit::profile::ProfilePackage;
use jit::repository_state::VirtualPath;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The gates whose footprint is the whole tree.
const TREE_WIDE_GATES: [&str; 3] = ["cargo-ci", "cargo-ci-features", "docs-mechanical"];

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
    let (_workspace, package) = jit::test_utils::temporary_repository_package(REGION_PACKAGE_ID);
    let findings = [TEMPLATE_REGION, INVARIANT_REGION]
        .iter()
        .filter_map(|region| region_finding(region, &package, &root))
        .chain(gate_input_findings(&root))
        .collect::<Vec<_>>();
    findings.iter().for_each(|finding| eprintln!("{finding}"));
    if findings.is_empty() {
        println!("OK: the generated regions hold the package, and gate inputs hold");
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn region_finding(
    region: &RegistryRegion,
    package: &ProfilePackage,
    root: &Path,
) -> Option<String> {
    let path = region.registry.repository_relative();
    let existing = match std::fs::read(root.join(&path)) {
        Ok(existing) => existing,
        Err(error) => return Some(format!("failed to read {path}: {error}")),
    };
    match region.render(&existing, package) {
        Ok(rendered) if rendered == existing => None,
        Ok(_) => Some(format!(
            "{path} no longer holds the packaged declarations between its region delimiters: \
             edit profiles/{REGION_PACKAGE_ID}/manifest.toml, then regenerate with {}",
            region.generator
        )),
        Err(error) => Some(format!("{path}: {error}")),
    }
}

/// The gate registry of the repository at `root`, read through a captured image.
fn captured_gate_registry(root: &Path) -> anyhow::Result<GateRegistry> {
    let repository = open_repository(RepositoryInvocation {
        current_dir: root.canonicalize()?,
        data_dir_override: None,
        worktree_root_override: None,
        is_init: false,
        opens_mutation_session: false,
        coordinates_claims_first: false,
        is_validation_exempt: false,
        is_recover: false,
        write_override: None,
    })?;
    Ok(repository
        .executor
        .config()
        .captured_declarations()?
        .gates()
        .clone())
}

/// A wrong input footprint makes a reused gate verdict silently stale, and a
/// wrong one still parses, so this repository's footprints are held here.
fn gate_input_findings(root: &Path) -> Vec<String> {
    let path = VirtualPath::GATES.repository_relative();
    let registry = match captured_gate_registry(root) {
        Ok(registry) => registry,
        Err(error) => return vec![format!("{path}: {error:#}")],
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
    ]
    .into_iter()
    .chain(TREE_WIDE_GATES.map(|key| (key, ".agents/skills/jit-manage/SKILL.md")))
    .filter(|(key, path)| !holds(key, &|declared| declared.covers(path)))
    .map(|(key, path)| format!("gate {key} must declare {path} an input"));
    let uncovered = [
        ("npm-ci", "web/node_modules/react/index.js"),
        ("npm-ci", "web/dist/index.html"),
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
