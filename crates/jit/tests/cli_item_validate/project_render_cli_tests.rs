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
    assert_eq!(json["count"], 1);
    assert_eq!(json["message"], "Rendered 1 projection(s)");
    assert_eq!(json["projections"].as_array().unwrap().len(), 1);
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
    assert_eq!(json["projections"].as_array().unwrap().len(), 1);
    assert_eq!(
        json["projections"][0]["name"].as_str().unwrap(),
        "invariants"
    );

    let written = std::fs::read_to_string(cli.root().join(".jit/invariants.md")).unwrap();
    assert!(written.contains("- **sample-invariant** — "));

    let human = cli
        .command(["project", "render", "--name", "invariants"])
        .output()
        .unwrap();
    assert!(human.status.success());
    assert_eq!(
        String::from_utf8_lossy(&human.stdout),
        "Rendered projection 'invariants' to .jit/invariants.md (separate-file mode, id-anchor style)\n"
    );
}

#[test]
fn test_project_render_projection_error_reports_classified_code_on_both_paths() {
    // A region-mode target missing its begin marker fails with a typed
    // ManagedDocumentError, classified as VALIDATION_FAILED in both modes.
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
    assert!(
        !output.status.success(),
        "typed projection failure fails under --json; stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).expect("JSON error object on stdout");
    assert_eq!(json["error"]["code"].as_str().unwrap(), "VALIDATION_FAILED");

    let human = cli.command(["project", "render"]).output().unwrap();
    assert_eq!(human.status.code(), Some(4));
    assert!(!human.status.success());
    assert!(String::from_utf8_lossy(&human.stderr).contains("required managed region"));
}

#[test]
fn test_project_render_unknown_name_uses_invalid_argument_on_both_paths() {
    let cli = JitCli::with_repository(
        jit::test_utils::profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    let json = cli
        .command(["project", "render", "--name", "does-not-exist", "--json"])
        .output()
        .unwrap();
    assert!(
        !json.status.success(),
        "unknown projection name must fail under --json"
    );
    let envelope: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert!(
        envelope["error"]["code"] == "INVALID_ARGUMENT",
        "unexpected error envelope: {envelope}"
    );

    let human = cli
        .command(["project", "render", "--name", "does-not-exist"])
        .output()
        .unwrap();
    assert_eq!(human.status.code(), Some(2));
    assert!(!human.status.success());
    let stderr = String::from_utf8_lossy(&human.stderr);
    assert!(stderr.contains("does-not-exist"), "stderr: {stderr}");
}

/// Every file and directory under the repository but the event log and
/// machine-local control state, keyed by repository-relative path.
fn repository_snapshot(cli: &JitCli) -> std::collections::BTreeMap<String, Option<Vec<u8>>> {
    fn walk(
        root: &std::path::Path,
        dir: &std::path::Path,
        out: &mut std::collections::BTreeMap<String, Option<Vec<u8>>>,
    ) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let name = path.file_name().unwrap().to_string_lossy();
            if relative == ".git"
                || relative == ".jit/events.jsonl"
                || relative == ".jit/tmp"
                || name.ends_with(".lock")
            {
                continue;
            }
            if path.is_dir() {
                out.insert(relative, None);
                walk(root, &path, out);
            } else {
                out.insert(relative, Some(std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(cli.root(), cli.root(), &mut out);
    out
}

fn event_log(cli: &JitCli) -> Vec<Value> {
    std::fs::read_to_string(cli.root().join(".jit/events.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Run `args`, returning the events it appended and the paths whose content
/// it changed, sorted.
fn appended_and_changed(cli: &JitCli, args: &[&str]) -> (Vec<Value>, Vec<String>) {
    let (events, before) = (event_log(cli), repository_snapshot(cli));
    let output = cli.command(args).output().unwrap();
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after = repository_snapshot(cli);
    let changed = before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    (event_log(cli)[events.len()..].to_vec(), changed)
}

#[test]
fn test_config_set_and_project_render_each_append_one_files_published_event() {
    let cli = JitCli::with_repository(
        jit::test_utils::profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    let render = ["project", "render"];
    let config_set = ["config", "set", "project.name", "renamed-project"];
    let published_by = |args: &[&str]| {
        let (appended, changed) = appended_and_changed(&cli, args);
        assert!(!changed.is_empty(), "{args:?} must write a changed file");
        assert_eq!(
            appended.len(),
            1,
            "{args:?} appends one event: {appended:?}"
        );
        assert_eq!(appended[0]["type"], "files_published", "{args:?}");
        assert_eq!(
            appended[0]["paths"],
            serde_json::json!(changed),
            "{args:?} names the paths it wrote"
        );
        appended[0].clone()
    };

    let configured = published_by(&config_set);
    // Advisory only: an enforced invariant naming no rule fails the validation
    // the no-op `config set` below runs.
    std::fs::write(
        cli.root().join(".jit/invariants.toml"),
        "[[invariants]]\nid = \"advisory\"\nstatement = \"Prefer clarity.\"\nkind = \"advisory\"\n",
    )
    .unwrap();
    std::fs::write(
        cli.root().join("ARCHITECTURE.md"),
        "<!-- jit:invariants:begin -->\nstale\n<!-- jit:invariants:end -->\n",
    )
    .unwrap();
    append_projection(
        &cli,
        "[projection.invariants]\nkind = \"invariant\"\nmode = \"region\"\n\
         target = \"ARCHITECTURE.md\"\nstyle = \"id-anchor\"\n",
    );
    let rendered = published_by(&render);

    assert_ne!(configured["id"], rendered["id"]);
    assert_ne!(configured["timestamp"], rendered["timestamp"]);
    for event in [&configured, &rendered] {
        let timestamp =
            chrono::DateTime::parse_from_rfc3339(event["timestamp"].as_str().unwrap()).unwrap();
        assert!(
            timestamp.timestamp() > 0,
            "{event} carries a real timestamp"
        );
    }
    for args in [&render[..], &config_set[..]] {
        assert_eq!(
            appended_and_changed(&cli, args),
            (Vec::new(), Vec::new()),
            "a no-op {args:?} appends nothing"
        );
    }
}
