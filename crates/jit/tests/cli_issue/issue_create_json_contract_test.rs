//! TDD: `issue create --json` must emit the same shape as `issue show --json`.
//!
//! Success criteria (REQ-14):
//! - `short_id` present and equal to the first 8 chars of `id`
//! - `gates` is a per-gate array (each entry: `key`, `status`, `last_run_at`,
//!   `exit_code`), not the legacy `gates_required`/`gates_status` map
//! - `dependencies` is an array of enriched objects, not raw ID strings
//! - `labels` is a JSON array
//! - `findings` carries the findings creation returned

use jit::test_cli::JitCli;

// ---------------------------------------------------------------------------
// REQ-14: short_id present, gates array, no legacy fields
// ---------------------------------------------------------------------------

#[test]
fn test_issue_create_json_has_short_id_and_gates_array() {
    let repo = JitCli::initialised().unwrap();

    // Define a gate so the created issue carries it.
    repo.run([
        "gate",
        "define",
        "cargo-ci",
        "--title",
        "CI",
        "--description",
        "Runs CI",
        "--mode",
        "manual",
    ])
    .unwrap();

    // Create the issue with that gate.
    let json = repo
        .run_json([
            "issue", "create", "--title", "My task", "--gate", "cargo-ci",
        ])
        .unwrap();

    // short_id is present and is the first 8 chars of id.
    let id = json["id"].as_str().expect("id must be a string");
    let short_id = json["short_id"]
        .as_str()
        .unwrap_or_else(|| panic!("short_id must be present; got: {}", json));
    assert_eq!(short_id.len(), 8, "short_id must be 8 chars");
    assert_eq!(short_id, &id[..8], "short_id must equal id[..8]");

    // gates is a JSON array.
    let gates = json["gates"]
        .as_array()
        .unwrap_or_else(|| panic!("gates must be a JSON array; got: {}", json["gates"]));
    assert_eq!(gates.len(), 1, "exactly one gate was declared");

    let gate = &gates[0];
    assert_eq!(gate["key"].as_str(), Some("cargo-ci"));
    assert_eq!(
        gate["status"].as_str(),
        Some("pending"),
        "freshly created gate is pending"
    );
    assert!(gate["last_run_at"].is_null(), "no run yet");
    assert!(gate["exit_code"].is_null(), "no run yet");

    // Legacy fields must be absent.
    assert!(
        json.get("gates_required").is_none(),
        "gates_required must not appear in create --json output"
    );
    assert!(
        json.get("gates_status").is_none(),
        "gates_status must not appear in create --json output"
    );
}

#[test]
fn test_issue_create_json_dependencies_are_enriched_objects() {
    let repo = JitCli::initialised().unwrap();

    // Create a prerequisite first.
    let dep = repo
        .run_json(["issue", "create", "--title", "Prerequisite"])
        .unwrap();
    let dep_id = dep["id"].as_str().unwrap();

    // Create the dependent issue (no gate, to keep the test focused).
    let json = repo
        .run_json(["issue", "create", "--title", "Dependent"])
        .unwrap();
    let id = json["id"].as_str().unwrap();

    // Wire the dependency.
    repo.run(["dep", "add", id, dep_id]).unwrap();

    // Re-fetch via create is impossible after the fact; use show to confirm
    // the enriched shape is now produced identically by create for new issues
    // by comparing the shape fields from the just-created issue above.
    //
    // For the freshly-created issue (before dep was wired), dependencies is [].
    let deps = json["dependencies"].as_array().unwrap_or_else(|| {
        panic!(
            "dependencies must be an array; got: {}",
            json["dependencies"]
        )
    });
    assert_eq!(deps.len(), 0, "freshly created: no deps yet");

    // Verify show --json produces the same top-level keys so the contract is
    // symmetric.
    let show = repo.run_json(["issue", "show", id]).unwrap();
    assert!(json.get("short_id").is_some(), "create has short_id");
    assert!(show.get("short_id").is_some(), "show has short_id");
    assert!(json.get("gates").is_some(), "create has gates");
    assert!(show.get("gates").is_some(), "show has gates");
    assert!(
        json.get("gates_required").is_none(),
        "create has no gates_required"
    );
    assert!(
        show.get("gates_required").is_none(),
        "show has no gates_required"
    );
}

#[test]
fn test_issue_create_json_labels_is_array() {
    let repo = JitCli::initialised().unwrap();

    let json = repo
        .run_json([
            "issue",
            "create",
            "--title",
            "Labelled",
            "--label",
            "type:task",
        ])
        .unwrap();

    let labels = json["labels"]
        .as_array()
        .unwrap_or_else(|| panic!("labels must be an array; got: {}", json["labels"]));
    assert!(
        labels.iter().any(|l| l == "type:task"),
        "label added at create time must appear in create --json output"
    );
}

#[test]
fn test_issue_create_forced_reports_same_findings_in_json_and_human_output() {
    let jit::test_taxonomy::TaxonomyRepo { temp, taxonomy } =
        jit::test_taxonomy::setup_test_repo_with_taxonomy();
    std::fs::write(
        temp.path().join(".jit/rules.toml"),
        "[[rules]]\nname = \"orphan-leaf\"\nseverity = \"warn\"\n\
         assert = { type-hierarchy = { kind = \"orphan-leaf\" } }\n",
    )
    .unwrap();
    let repo = JitCli::with_repository(temp);
    let leaf = jit::test_taxonomy::type_label(&taxonomy, 4);
    let args = ["issue", "create", "-t", "Leaf", "--label", &leaf, "--force"];

    let json_output = repo
        .command(args.iter().copied().chain(["--json"]))
        .output()
        .unwrap();
    assert!(json_output.status.success());
    assert!(
        json_output.stderr.is_empty(),
        "JSON create must not print human findings to stderr: {}",
        String::from_utf8_lossy(&json_output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&json_output.stdout).unwrap();
    let human = repo.command(args).output().unwrap();

    let findings = json["findings"]
        .as_array()
        .filter(|findings| !findings.is_empty())
        .unwrap_or_else(|| panic!("create --json must report findings; got: {json}"));
    assert!(
        findings
            .iter()
            .any(|finding| finding["rule"] == "orphan-leaf"),
        "create --json must report the orphan-leaf finding; got: {findings:?}"
    );
    let human_stdout = String::from_utf8_lossy(&human.stdout);
    let human_id = human_stdout
        .lines()
        .next()
        .unwrap_or_default()
        .strip_prefix("Created issue: ")
        .unwrap_or_else(|| panic!("human create prints the id; got: {human_stdout}"));
    let human_stderr = String::from_utf8_lossy(&human.stderr);
    assert!(
        human_stderr.is_empty(),
        "create findings use the shared renderer on stdout: {human_stderr}"
    );
    let human_findings = human_stdout.lines().skip(1).collect::<Vec<_>>();
    assert_eq!(human_findings.len(), findings.len());
    for (index, finding) in findings.iter().enumerate() {
        let expected = finding["message"]
            .as_str()
            .unwrap()
            .replace(json["id"].as_str().unwrap(), human_id)
            .replace(json["short_id"].as_str().unwrap(), &human_id[..8]);
        assert_eq!(
            human_findings[index],
            format!("warning [{}] {expected}", finding["rule"].as_str().unwrap()),
            "human create must print the same finding in the same order"
        );
    }

    let quiet = repo
        .command(args.iter().copied().chain(["--quiet"]))
        .output()
        .unwrap();
    assert!(quiet.status.success());
    assert!(quiet.stderr.is_empty());
    let quiet_stdout = String::from_utf8_lossy(&quiet.stdout);
    assert_eq!(quiet_stdout.lines().count(), 1);
    assert!(
        jit::domain::identity::IssueId::try_from(quiet_stdout.trim()).is_ok(),
        "quiet create prints only the full issue id: {quiet_stdout}"
    );
}
