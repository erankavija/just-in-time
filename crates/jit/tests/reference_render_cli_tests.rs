//! Integration tests for `jit reference render`.
//!
//! Exercises the rules-and-gates projection end-to-end through the real CLI
//! binary: the effective `.jit/rules.toml` rule set and the `.jit/gates.toml` gate
//! registry are rendered into one reference document at the target configured by
//! `[rules_gates_projection]`. Covers the shipped separate-file default and region
//! mode (byte-preserving content outside the delimiters).

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

#[test]
fn test_reference_render_writes_separate_file_default() {
    // With no [rules_gates_projection] table, the shipped default targets a
    // separate jit-owned file (.jit/rules-and-gates.md). `render` must actually
    // write it end-to-end through the real binary. `jit init` scaffolds the
    // default rule set and an empty gate registry.
    let temp = setup_test_repo();

    let output = Command::new(jit_binary())
        .args(["reference", "render", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "reference render failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["target"].as_str().unwrap(), ".jit/rules-and-gates.md");
    assert_eq!(json["mode"].as_str().unwrap(), "separate-file");
    assert!(
        json["rules"].as_u64().unwrap() >= 1,
        "scaffolded rules: {json}"
    );

    // The default jit-owned target was actually written with the rendered
    // registries, using canonical kind-segmented addresses.
    let written = std::fs::read_to_string(temp.path().join(".jit/rules-and-gates.md")).unwrap();
    assert!(written.contains("## Rules"), "rendered file: {written}");
    assert!(written.contains("@/rule/label-format"));
    // A freshly-scaffolded gate registry is empty -> explicit line.
    assert!(written.contains("## Gates"));
    assert!(written.contains("_No gates declared._"));
}

#[test]
fn test_reference_render_region_mode_byte_preserves_surroundings() {
    // Region mode rewrites ONLY the delimited region; everything outside the
    // delimiters is byte-preserved.
    let temp = setup_test_repo();

    let begin = "<!-- jit:rules-and-gates:begin -->";
    let end = "<!-- jit:rules-and-gates:end -->";
    let prefix = "# Rules and Gates\n\nHand-written intro the user owns.\n\n";
    let suffix = "\n\n## Other sections\n\nMore hand-written prose.\n";
    let original = format!("{prefix}{begin}\nstale placeholder\n{end}{suffix}");
    std::fs::write(temp.path().join("REFERENCE.md"), &original).unwrap();

    // Configure region mode targeting the hand-written doc.
    let config = format!(
        "[rules_gates_projection]\nmode = \"region\"\ntarget = \"REFERENCE.md\"\nregion-begin = \"{begin}\"\nregion-end = \"{end}\"\n"
    );
    std::fs::write(temp.path().join(".jit/config.toml"), &config).unwrap();

    let output = Command::new(jit_binary())
        .args(["reference", "render", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "reference render (region) failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["mode"].as_str().unwrap(), "region");

    let updated = std::fs::read_to_string(temp.path().join("REFERENCE.md")).unwrap();
    // Content OUTSIDE the delimiters is byte-preserved.
    assert!(
        updated.starts_with(&format!("{prefix}{begin}")),
        "prefix not preserved: {updated}"
    );
    assert!(
        updated.ends_with(&format!("{end}{suffix}")),
        "suffix not preserved: {updated}"
    );
    // The region was replaced with the rendered rule set.
    assert!(updated.contains("@/rule/label-format"));
    assert!(!updated.contains("stale placeholder"));
}
