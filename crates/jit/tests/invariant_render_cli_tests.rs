//! Integration tests for `jit invariant render`.
//!
//! Exercises the invariant projection end-to-end through the real CLI binary:
//! the `.jit/invariants.toml` registry is rendered into the documentation target
//! configured by `[invariant_projection]`. Covers the shipped separate-file
//! default and region mode (byte-preserving content outside the delimiters).

use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let output = Command::new(jit_binary())
        .arg("init")
        .current_dir(temp.path())
        .output()
        .expect("Failed to run jit init");
    assert!(output.status.success(), "jit init failed");
    temp
}

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

#[test]
fn test_invariant_render_writes_separate_file_default() {
    // With no [invariant_projection] table, the shipped default targets a
    // separate jit-owned file (.jit/invariants.md). `render` must actually write
    // it end-to-end through the real binary.
    let temp = setup_test_repo();
    std::fs::write(temp.path().join(".jit/invariants.toml"), INVARIANTS_TOML).unwrap();

    let output = Command::new(jit_binary())
        .args(["invariant", "render", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "invariant render failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["target"].as_str().unwrap(), ".jit/invariants.md");
    assert_eq!(json["mode"].as_str().unwrap(), "separate-file");
    assert_eq!(json["count"].as_u64().unwrap(), 2);

    // The default jit-owned target was actually written with the rendered registry.
    let written = std::fs::read_to_string(temp.path().join(".jit/invariants.md")).unwrap();
    assert!(written.contains("INV-01"), "rendered file: {written}");
    assert!(written.contains("INV-02"));
    assert!(written.contains("Every dependency edge stays acyclic."));
    assert!(written.contains("dag-no-cycles"));
}

#[test]
fn test_invariant_render_region_mode_byte_preserves_surroundings() {
    // Region mode rewrites ONLY the delimited region; everything outside the
    // delimiters is byte-preserved.
    let temp = setup_test_repo();
    std::fs::write(temp.path().join(".jit/invariants.toml"), INVARIANTS_TOML).unwrap();

    let begin = "<!-- jit:invariants:begin -->";
    let end = "<!-- jit:invariants:end -->";
    let prefix = "# Architecture\n\nHand-written intro the user owns.\n\n";
    let suffix = "\n\n## Other sections\n\nMore hand-written prose.\n";
    let original = format!("{prefix}{begin}\nstale placeholder\n{end}{suffix}");
    std::fs::write(temp.path().join("ARCHITECTURE.md"), &original).unwrap();

    // Configure region mode targeting the hand-written doc.
    let config = format!(
        "[invariant_projection]\nmode = \"region\"\ntarget = \"ARCHITECTURE.md\"\nregion-begin = \"{begin}\"\nregion-end = \"{end}\"\n"
    );
    std::fs::write(temp.path().join(".jit/config.toml"), &config).unwrap();

    let output = Command::new(jit_binary())
        .args(["invariant", "render", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "invariant render (region) failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["mode"].as_str().unwrap(), "region");

    let updated = std::fs::read_to_string(temp.path().join("ARCHITECTURE.md")).unwrap();
    // Content OUTSIDE the delimiters is byte-preserved.
    assert!(
        updated.starts_with(&format!("{prefix}{begin}")),
        "prefix not preserved: {updated}"
    );
    assert!(
        updated.ends_with(&format!("{end}{suffix}")),
        "suffix not preserved: {updated}"
    );
    // The region was replaced.
    assert!(updated.contains("INV-01"));
    assert!(!updated.contains("stale placeholder"));
}
