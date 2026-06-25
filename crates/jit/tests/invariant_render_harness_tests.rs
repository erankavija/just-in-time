//! Command-harness tests for `CommandExecutor::render_invariants`.
//!
//! Drives the command method directly over `InMemoryStorage` (no subprocess, no
//! real filesystem for the projection target) — the `TestHarness`/`CommandExecutor`
//! layer the project's testing strategy prefers for command behavior. The
//! invariant registry is seeded on the storage root (where `cached_config` reads
//! it), and the rendered target is asserted via the in-memory repo-file map.
//! Complements the binary integration tests in `invariant_render_cli_tests.rs`.

use jit::commands::CommandExecutor;
use jit::storage::{InMemoryStorage, IssueStore};

const INVARIANTS_TOML: &str = r#"
[[invariants]]
id = "INV-01"
statement = "Every dependency edge stays acyclic."
kind = "enforced"
enforced-by = "dag-no-cycles"

[[invariants]]
id = "INV-02"
statement = "Issues prefer functional style."
kind = "advisory"
"#;

/// Build an `InMemoryStorage` whose config root has `invariants.toml` (and an
/// optional `config.toml`) seeded on disk, so `CommandExecutor::cached_config`
/// loads the registry + projection config.
fn storage_with_registry(config_toml: Option<&str>) -> InMemoryStorage {
    std::env::set_var("JIT_TEST_MODE", "1");
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    // The config root is the storage `root()`; seed the registry there.
    let root = storage.root().to_path_buf();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("invariants.toml"), INVARIANTS_TOML).unwrap();
    if let Some(cfg) = config_toml {
        std::fs::write(root.join("config.toml"), cfg).unwrap();
    }
    storage
}

#[test]
fn test_render_invariants_separate_file_default_via_command() {
    let storage = storage_with_registry(None);
    let executor = CommandExecutor::new(storage.clone());

    let result = executor.render_invariants().unwrap();
    assert_eq!(result.target, ".jit/invariants.md");
    assert_eq!(result.mode, "separate-file");
    assert_eq!(result.count, 2);

    // The rendered target is materialized through the storage boundary.
    let written = storage
        .read_repo_file(".jit/invariants.md")
        .unwrap()
        .expect("separate-file target should be written");
    assert!(written.contains("INV-01"), "rendered: {written}");
    assert!(written.contains("INV-02"));
    assert!(written.contains("Every dependency edge stays acyclic."));
    assert!(written.contains("dag-no-cycles"));

    // Clean up the on-disk config root seeded for this test.
    let _ = std::fs::remove_dir_all(storage.root());
}

#[test]
fn test_render_invariants_region_mode_via_command() {
    let begin = "<!-- jit:invariants:begin -->";
    let end = "<!-- jit:invariants:end -->";
    let config = format!(
        "[invariant_projection]\nmode = \"region\"\ntarget = \"GUIDE.md\"\nregion-begin = \"{begin}\"\nregion-end = \"{end}\"\n"
    );
    let storage = storage_with_registry(Some(&config));

    // Seed the existing region-mode target in the in-memory repo-file map.
    let prefix = "# Guide\n\nHand-written intro.\n\n";
    let suffix = "\n\n## Footer\n\nHand-written outro.\n";
    let original = format!("{prefix}{begin}\nstale\n{end}{suffix}");
    storage.add_repo_file("GUIDE.md", &original);

    let executor = CommandExecutor::new(storage.clone());
    let result = executor.render_invariants().unwrap();
    assert_eq!(result.target, "GUIDE.md");
    assert_eq!(result.mode, "region");

    let updated = storage.read_repo_file("GUIDE.md").unwrap().unwrap();
    // Content outside the delimiters is byte-preserved.
    assert!(updated.starts_with(&format!("{prefix}{begin}")));
    assert!(updated.ends_with(&format!("{end}{suffix}")));
    // The region was replaced.
    assert!(updated.contains("INV-01"));
    assert!(!updated.contains("stale"));

    let _ = std::fs::remove_dir_all(storage.root());
}
