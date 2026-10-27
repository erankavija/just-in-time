//! Integration tests for `jit project render`.
//!
//! Exercises the generic documentation-projection command end-to-end through the
//! real CLI binary: a `[projection.<name>]` table renders an addressable item kind
//! into its configured target. Covers region-mode byte-preservation, `--name`
//! selection, and the unknown-name error path.

use jit::test_cli::JitCli;
use serde_json::Value;

const INVARIANTS_TOML: &str = r#"
[[invariants]]
id = "sample-invariant"
statement = "Every dependency edge stays acyclic."
kind = "enforced"
enforced-by = "dag-no-cycles"

[[invariants]]
id = "second-invariant"
statement = "Issues prefer functional style."
kind = "advisory"
"#;

/// Append a `[projection.<name>]` table to the package-provided config (which
/// already declares `[item_kinds.invariant]`), preserving the package kinds.
fn append_projection(cli: &JitCli, table: &str) {
    let path = cli.root().join(".jit/config.toml");
    let mut config = std::fs::read_to_string(&path).unwrap();
    config.push('\n');
    config.push_str(table);
    std::fs::write(&path, config).unwrap();
}

#[test]
fn test_project_render_region_mode_byte_preserves_surroundings() {
    let cli = JitCli::with_repository(
        jit::test_utils::profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    std::fs::write(cli.root().join(".jit/invariants.toml"), INVARIANTS_TOML).unwrap();

    let begin = "<!-- jit:invariants:begin -->";
    let end = "<!-- jit:invariants:end -->";
    let prefix = "# Architecture\n\nHand-written intro the user owns.\n\n";
    let suffix = "\n\n## Other sections\n\nMore hand-written prose.\n";
    let original = format!("{prefix}{begin}\nstale placeholder\n{end}{suffix}");
    std::fs::write(cli.root().join("ARCHITECTURE.md"), &original).unwrap();

    append_projection(
        &cli,
        "[projection.invariants]\nkind = \"invariant\"\nmode = \"region\"\n\
         target = \"ARCHITECTURE.md\"\nstyle = \"id-anchor\"\n",
    );

    let output = cli
        .command(["project", "render", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "project render failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 1);
    assert_eq!(
        json["projections"][0]["target"].as_str().unwrap(),
        "ARCHITECTURE.md"
    );
    assert_eq!(json["projections"][0]["mode"].as_str().unwrap(), "region");

    let updated = std::fs::read_to_string(cli.root().join("ARCHITECTURE.md")).unwrap();
    // Content OUTSIDE the delimiters is byte-preserved.
    assert!(
        updated.starts_with(&format!("{prefix}{begin}")),
        "prefix not preserved: {updated}"
    );
    assert!(
        updated.ends_with(&format!("{end}{suffix}")),
        "suffix not preserved: {updated}"
    );
    // The id-anchor rows replaced the stale placeholder.
    assert!(updated.contains("- **sample-invariant** — Every dependency edge stays acyclic."));
    assert!(!updated.contains("stale placeholder"));
}

#[test]
fn test_project_render_named_selects_single_projection() {
    let cli = JitCli::with_repository(
        jit::test_utils::profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    std::fs::write(cli.root().join(".jit/invariants.toml"), INVARIANTS_TOML).unwrap();
    append_projection(
        &cli,
        "[projection.invariants]\nkind = \"invariant\"\nmode = \"separate-file\"\n\
         target = \".jit/invariants.md\"\nstyle = \"id-anchor\"\n",
    );

    let output = cli
        .command(["project", "render", "--name", "invariants", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["count"].as_u64().unwrap(), 1);
    assert_eq!(
        json["projections"][0]["name"].as_str().unwrap(),
        "invariants"
    );

    let written = std::fs::read_to_string(cli.root().join(".jit/invariants.md")).unwrap();
    assert!(written.contains("- **sample-invariant** — "));
}

#[test]
fn test_project_render_json_projection_error_exits_validation_failed() {
    // A typed projection failure under --json carries VALIDATION_FAILED and
    // exit 4, matching the non-JSON classification (jit:450db193 review F1,
    // round 5). A region-mode target missing its begin marker fails the splice
    // with a typed ProjectionError.
    let cli = JitCli::with_repository(
        jit::test_utils::profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    std::fs::write(cli.root().join(".jit/invariants.toml"), INVARIANTS_TOML).unwrap();
    std::fs::write(
        cli.root().join("ARCHITECTURE.md"),
        "# Architecture\n\nNo region markers here.\n",
    )
    .unwrap();
    append_projection(
        &cli,
        "[projection.invariants]\nkind = \"invariant\"\nmode = \"region\"\n\
         target = \"ARCHITECTURE.md\"\nstyle = \"id-anchor\"\n",
    );

    let output = cli
        .command(["project", "render", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(4),
        "typed projection failure exits 4 under --json; stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).expect("JSON error object on stdout");
    assert_eq!(json["error"]["code"].as_str().unwrap(), "VALIDATION_FAILED");
}

#[test]
fn test_project_render_unknown_name_errors() {
    let cli = JitCli::with_repository(
        jit::test_utils::profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    let output = cli
        .command(["project", "render", "--name", "does-not-exist"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "unknown projection name must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does-not-exist") || stderr.contains("unknown"),
        "stderr: {stderr}"
    );
}
