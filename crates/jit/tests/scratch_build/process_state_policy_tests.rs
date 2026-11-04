//! No code changes the process environment or working directory, so every
//! test may read them and pass in parallel (`@/inv/orthogonal-tests`); a value
//! a test varies reaches the code under test explicitly.

use std::path::{Path, PathBuf};

const PROCESS_STATE_MUTATORS: [&str; 3] = [
    concat!("set_", "var("),
    concat!("remove_", "var("),
    concat!("set_current_", "dir("),
];

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path);
            }
        }
    }
    sources.sort();
    sources
}

/// Code lines of `source` that call a process-state mutator.
fn mutator_lines(source: &str) -> Vec<(usize, &str)> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
        .filter(|(_, line)| {
            PROCESS_STATE_MUTATORS
                .iter()
                .any(|call| line.contains(call))
        })
        .map(|(index, line)| (index + 1, line.trim()))
        .collect()
}

#[test]
fn test_mutator_lines_flags_an_environment_mutation_and_skips_comments() {
    let source = format!(
        "// std::env::{set}(\"A\", \"1\");\nfn f() {{\n    std::env::{remove}(\"JIT_AGENT_ID\");\n}}\n",
        set = concat!("set_", "var"),
        remove = concat!("remove_", "var"),
    );
    let found = mutator_lines(&source);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, 3);
}

#[test]
fn test_workspace_never_mutates_process_state() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let violations = rust_sources(&workspace.join("crates"))
        .into_iter()
        .flat_map(|path| {
            let source = std::fs::read_to_string(&path).unwrap();
            mutator_lines(&source)
                .into_iter()
                .map(|(line, text)| format!("{}:{line}: {text}", path.display()))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert!(
        violations.is_empty(),
        "pass the value explicitly (`Command::env`, `Command::current_dir`, a parameter) instead \
         of changing the process environment or working directory:\n{}",
        violations.join("\n")
    );
}
