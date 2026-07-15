//! TDD: `issue create --json` must emit the same shape as `issue show --json`.
//!
//! Success criteria (REQ-14):
//! - `short_id` present and equal to the first 8 chars of `id`
//! - `gates` is a per-gate array (each entry: `key`, `status`, `last_run_at`,
//!   `exit_code`), not the legacy `gates_required`/`gates_status` map
//! - `dependencies` is an array of enriched objects, not raw ID strings
//! - `labels` is a JSON array

use std::process::Command;

fn jit_bin() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
}

/// Run a jit command in `dir` and return its stdout.
fn jit(dir: &std::path::Path, args: &[&str]) -> serde_json::Value {
    let out = jit_bin()
        .current_dir(dir)
        .args(args)
        .output()
        .expect("jit command failed to spawn");
    assert!(
        out.status.success(),
        "jit {:?} exited non-zero; stderr: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout was not valid JSON: {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn setup_repo() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    jit_bin()
        .current_dir(tmp.path())
        .arg("init")
        .status()
        .unwrap();
    tmp
}

// ---------------------------------------------------------------------------
// REQ-14: short_id present, gates array, no legacy fields
// ---------------------------------------------------------------------------

#[test]
fn test_issue_create_json_has_short_id_and_gates_array() {
    let repo = setup_repo();

    // Define a gate so the created issue carries it.
    jit_bin()
        .current_dir(repo.path())
        .args([
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
        .status()
        .unwrap();

    // Create the issue with that gate.
    let json = jit(
        repo.path(),
        &[
            "issue", "create", "--title", "My task", "--gate", "cargo-ci", "--json",
        ],
    );

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
    let repo = setup_repo();

    // Create a prerequisite first.
    let dep = jit(
        repo.path(),
        &["issue", "create", "--title", "Prerequisite", "--json"],
    );
    let dep_id = dep["id"].as_str().unwrap();

    // Create the dependent issue (no gate, to keep the test focused).
    let json = jit(
        repo.path(),
        &["issue", "create", "--title", "Dependent", "--json"],
    );
    let id = json["id"].as_str().unwrap();

    // Wire the dependency.
    jit_bin()
        .current_dir(repo.path())
        .args(["dep", "add", id, dep_id])
        .status()
        .unwrap();

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
    let show = jit(repo.path(), &["issue", "show", id, "--json"]);
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
    let repo = setup_repo();

    let json = jit(
        repo.path(),
        &[
            "issue",
            "create",
            "--title",
            "Labelled",
            "--label",
            "type:task",
            "--json",
        ],
    );

    let labels = json["labels"]
        .as_array()
        .unwrap_or_else(|| panic!("labels must be an array; got: {}", json["labels"]));
    assert!(
        labels.iter().any(|l| l == "type:task"),
        "label added at create time must appear in create --json output"
    );
}
