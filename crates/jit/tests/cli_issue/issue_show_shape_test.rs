//! Verify the `jit issue show --json` root shape:
//! - a `short_id` field (first 8 chars of the full UUID) is always present
//! - `labels` and `dependencies` are always JSON arrays (empty `[]` when none),
//!   never absent or null
//!
//! Covered across lifecycle states (backlog and an issue carrying labels +
//! dependencies) so agents can parse the output without defensive guards.

use jit::test_cli::JitCli;

#[test]
fn test_show_root_has_short_id_matching_id_prefix() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Backlog issue", "--description", "Body"])
        .unwrap();

    let json = temp.run_json(["issue", "show", &id]).unwrap();
    let short = json["short_id"]
        .as_str()
        .unwrap_or_else(|| panic!("short_id must be a string at root; got: {}", json));
    assert_eq!(short.len(), 8, "short_id must be 8 chars; got {:?}", short);
    assert_eq!(
        short,
        &id[0..8],
        "short_id must equal id[0..8]; got {:?} vs {:?}",
        short,
        &id[0..8]
    );
}

#[test]
fn test_show_labels_and_dependencies_are_empty_arrays_when_none() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Bare issue", "--description", "Body"])
        .unwrap();

    // New issues may carry a default type label; strip it so we can assert the
    // genuinely-empty case still serializes as `[]` rather than being omitted.
    let labels: Vec<String> = temp.run_json(["issue", "show", &id]).unwrap()["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect();
    for label in &labels {
        temp.run(["issue", "update", &id, "--remove-label", label])
            .unwrap();
    }

    let json = temp.run_json(["issue", "show", &id]).unwrap();

    assert!(
        json["labels"].is_array(),
        "labels must be an array; got: {}",
        json["labels"]
    );
    assert_eq!(
        json["labels"].as_array().unwrap().len(),
        0,
        "labels must be an empty array when none set; got: {}",
        json["labels"]
    );

    assert!(
        json["dependencies"].is_array(),
        "dependencies must be an array; got: {}",
        json["dependencies"]
    );
    assert_eq!(
        json["dependencies"].as_array().unwrap().len(),
        0,
        "dependencies must be an empty array when none set; got: {}",
        json["dependencies"]
    );
}

#[test]
fn test_show_labels_and_dependencies_populated() {
    let temp = JitCli::initialised().unwrap();
    let dep_id = temp
        .create_issue(["--title", "Prerequisite", "--description", "Body"])
        .unwrap();
    let id = temp
        .create_issue(["--title", "Dependent issue", "--description", "Body"])
        .unwrap();

    temp.run(["issue", "update", &id, "--label", "type:task"])
        .unwrap();
    temp.run(["dep", "add", &id, &dep_id]).unwrap();

    let json = temp.run_json(["issue", "show", &id]).unwrap();

    // Still arrays, now non-empty, and short_id still present.
    assert!(json["short_id"].is_string());
    let labels = json["labels"].as_array().expect("labels array");
    assert!(
        labels.iter().any(|l| l == "type:task"),
        "labels must contain added label; got: {}",
        json["labels"]
    );
    let deps = json["dependencies"].as_array().expect("dependencies array");
    assert_eq!(
        deps.len(),
        1,
        "dependencies must list the added dependency; got: {}",
        json["dependencies"]
    );
}

#[test]
fn test_show_short_id_present_across_states() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Lifecycle issue", "--description", "Body"])
        .unwrap();

    // Backlog (just created).
    assert!(temp.run_json(["issue", "show", &id]).unwrap()["short_id"].is_string());

    // Move to in_progress and re-check the shape holds.
    temp.run(["issue", "update", &id, "--state", "in_progress"])
        .unwrap();
    let json = temp.run_json(["issue", "show", &id]).unwrap();
    assert_eq!(json["short_id"].as_str(), Some(&id[0..8]));
    assert!(json["labels"].is_array());
    assert!(json["dependencies"].is_array());
}
