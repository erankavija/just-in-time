use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn jit(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run jit")
}

/// The id the checked-in fixture package declares.
const FIXTURE_PROFILE: &str = "planner-asset-only";

/// Copy the fixture package tree into `repo` at the repository-relative
/// `location`, and return that location.
///
/// A package is applied from inside the repository it is applied to, because
/// the applied-profile record names its worktree-relative location.
fn package_at<'a>(repo: &Path, location: &'a str) -> &'a str {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/profile-packages")
        .join(FIXTURE_PROFILE);
    jit::test_utils::copy_package_tree(&source, &repo.join(location));
    location
}

/// The record this repository stores for the fixture package.
fn stored_record(repo: &Path) -> Value {
    let path = repo.join(format!(".jit/profiles/{FIXTURE_PROFILE}.json"));
    serde_json::from_slice(&fs::read(&path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", path.display());
    }))
    .expect("a stored record is JSON")
}

/// Republish the package tree at `location` under `version`.
///
/// Two copies of one fixture are byte-identical and therefore indistinguishable
/// in everything a resolution reports, so a test that asks which copy was read
/// edits one of them first. The published asset is left alone: identical target
/// bytes are what keeps the second application an ordinary re-application
/// rather than a target conflict.
fn set_package_version(repo: &Path, location: &str, version: &str) -> PathBuf {
    let manifest = repo.join(location).join("manifest.toml");
    let declared = fs::read_to_string(&manifest)
        .unwrap()
        .replace("version = \"1.0.0\"", &format!("version = \"{version}\""));
    fs::write(&manifest, declared).unwrap();
    manifest
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON: {error}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn test_profile_list_and_show_work_without_repository() {
    let repo = TempDir::new().unwrap();

    // A directory that is not a repository has recorded no profile, so it names
    // none; inspection still resolves the package this binary carries.
    let list = jit(repo.path(), &["profile", "list", "--json"]);
    assert!(list.status.success(), "{list:?}");
    let list = json(&list);
    assert_eq!(list["count"], 0);
    assert_eq!(list["profiles"].as_array().unwrap(), &Vec::<Value>::new());

    let show = jit(repo.path(), &["profile", "show", "jit-dogfood", "--json"]);
    assert!(show.status.success(), "{show:?}");
    let show = json(&show);
    assert_eq!(show["manifest"]["profile"]["id"], "jit-dogfood");
    assert_eq!(show["origin"], serde_json::json!({ "source": "embedded" }));
    assert!(show["package_hash"].as_str().unwrap().len() >= 64);
    assert!(!repo.path().join(".jit").exists());
}

#[test]
fn test_profile_show_reads_the_package_a_supplied_location_holds() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let location = package_at(repo.path(), "vendor/planner");

    let show = jit(
        repo.path(),
        &[
            "profile",
            "show",
            FIXTURE_PROFILE,
            "--from",
            location,
            "--json",
        ],
    );

    assert!(show.status.success(), "{show:?}");
    let show = json(&show);
    // Nothing compiled into the binary declares this profile, so reporting it
    // at all is the location having been read.
    assert_eq!(show["manifest"]["profile"]["id"], FIXTURE_PROFILE);
    assert_eq!(
        show["origin"],
        serde_json::json!({ "source": "directory", "location": location })
    );
}

#[test]
fn test_profile_apply_from_a_supplied_location_is_re_read_from_the_record() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let location = package_at(repo.path(), "vendor/planner");

    let applied = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            FIXTURE_PROFILE,
            "--from",
            location,
            "--json",
        ],
    );
    assert!(applied.status.success(), "{applied:?}");
    assert_eq!(json(&applied)["status"], "applied");
    assert_eq!(
        stored_record(repo.path())["origin"],
        serde_json::json!({ "source": "directory", "location": location })
    );

    // No location is named this time: the record is the repository's own
    // statement of where the bytes are, and reading them back from there is
    // what makes the second application an exact no-op.
    let reapplied = jit(
        repo.path(),
        &["profile", "apply", FIXTURE_PROFILE, "--json"],
    );
    assert!(reapplied.status.success(), "{reapplied:?}");
    assert_eq!(json(&reapplied)["status"], "unchanged");

    let listed = jit(repo.path(), &["profile", "list", "--json"]);
    assert!(listed.status.success(), "{listed:?}");
    let listed = json(&listed);
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["profiles"][0]["id"], FIXTURE_PROFILE);
    assert_eq!(listed["profiles"][0]["applied"], true);
    assert_eq!(
        listed["profiles"][0]["origin"],
        serde_json::json!({ "source": "directory", "location": location })
    );
}

#[test]
fn test_profile_show_prefers_a_supplied_location_over_the_recorded_one() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let recorded = package_at(repo.path(), "vendor/recorded");
    assert!(jit(
        repo.path(),
        &[
            "profile",
            "apply",
            FIXTURE_PROFILE,
            "--from",
            recorded,
            "--json"
        ]
    )
    .status
    .success());

    // A second copy of the same profile, distinguishable from the recorded one
    // by the version it declares.
    let supplied = package_at(repo.path(), "vendor/supplied");
    set_package_version(repo.path(), supplied, "2.0.0");

    let shown = jit(
        repo.path(),
        &[
            "profile",
            "show",
            FIXTURE_PROFILE,
            "--from",
            supplied,
            "--json",
        ],
    );
    assert!(shown.status.success(), "{shown:?}");
    let shown = json(&shown);
    assert_eq!(shown["manifest"]["profile"]["version"], "2.0.0");
    assert_eq!(
        shown["origin"],
        serde_json::json!({ "source": "directory", "location": supplied })
    );

    // Supplying nothing falls back to the record, which still names the copy it
    // was applied from.
    let recorded_show = jit(repo.path(), &["profile", "show", FIXTURE_PROFILE, "--json"]);
    assert!(recorded_show.status.success(), "{recorded_show:?}");
    let recorded_show = json(&recorded_show);
    assert_eq!(recorded_show["manifest"]["profile"]["version"], "1.0.0");
    assert_eq!(
        recorded_show["origin"],
        serde_json::json!({ "source": "directory", "location": recorded })
    );
}

#[test]
fn test_profile_list_reports_a_recorded_location_that_no_longer_resolves() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let location = package_at(repo.path(), "vendor/planner");
    assert!(jit(
        repo.path(),
        &[
            "profile",
            "apply",
            FIXTURE_PROFILE,
            "--from",
            location,
            "--json"
        ]
    )
    .status
    .success());
    fs::remove_dir_all(repo.path().join(location)).unwrap();

    let listed = jit(repo.path(), &["profile", "list", "--json"]);

    assert!(!listed.status.success(), "{listed:?}");
    let listed = json(&listed);
    let message = listed["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains(&format!(".jit/profiles/{FIXTURE_PROFILE}.json"))
            && message.contains(location),
        "the failure must name the record and the location: {listed}"
    );
    assert_ne!(
        listed["error"]["code"], "PROFILE_NOT_FOUND",
        "a record whose package moved is not a profile this repository never applied"
    );
}

#[test]
fn test_init_profile_applies_the_package_a_supplied_location_holds() {
    let repo = TempDir::new().unwrap();
    let location = package_at(repo.path(), "vendor/planner");

    // A repository being created has no record to read, so the location is the
    // only statement of where its package is.
    let init = jit(
        repo.path(),
        &[
            "init",
            "--profile",
            FIXTURE_PROFILE,
            "--from",
            location,
            "--json",
        ],
    );

    assert!(init.status.success(), "{init:?}");
    assert_eq!(json(&init)["profile"]["status"], "applied");
    assert!(repo.path().join("docs/profile.txt").is_file());
    assert_eq!(
        stored_record(repo.path())["origin"],
        serde_json::json!({ "source": "directory", "location": location })
    );

    // The record initialization wrote is readable as the location it names: a
    // later application that supplies nothing resolves the same package.
    let reapplied = jit(
        repo.path(),
        &["profile", "apply", FIXTURE_PROFILE, "--json"],
    );
    assert!(reapplied.status.success(), "{reapplied:?}");
    assert_eq!(json(&reapplied)["status"], "unchanged");
}

#[test]
fn test_init_from_without_a_profile_is_a_usage_error() {
    let repo = TempDir::new().unwrap();
    let location = package_at(repo.path(), "vendor/planner");

    let init = jit(repo.path(), &["init", "--from", location, "--json"]);

    assert_eq!(init.status.code(), Some(2), "{init:?}");
    assert!(!repo.path().join(".jit").exists());
}

/// The inspection output carries the roots the package's live assets are drawn
/// from, and carries them usably: the declaration and the assets it bounds are
/// read out of the same document, so a consumer needs nothing else to tell
/// which root any packaged live asset came from.
#[test]
fn test_profile_show_json_reports_the_roots_its_live_assets_are_drawn_from() {
    let repo = TempDir::new().unwrap();

    let show = jit(repo.path(), &["profile", "show", "jit-dogfood", "--json"]);
    assert!(show.status.success(), "{show:?}");
    let show = json(&show);

    let roots: Vec<&str> = show["manifest"]["live-source"]
        .as_array()
        .expect("the reported manifest declares its live-source roots")
        .iter()
        .map(|declaration| {
            assert!(
                declaration["exclude"].is_array(),
                "each declared root carries its own exclusion list: {declaration}"
            );
            declaration["root"]
                .as_str()
                .unwrap_or_else(|| panic!("a declared root is a path string: {declaration}"))
        })
        .collect();
    assert!(!roots.is_empty(), "the package declares at least one root");

    let live_targets: Vec<&str> = show["manifest"]["asset"]
        .as_array()
        .expect("the reported manifest declares assets")
        .iter()
        .filter(|asset| {
            asset["source"]
                .as_str()
                .is_some_and(|source| source.starts_with("assets/live/"))
        })
        .map(|asset| {
            asset["target"]
                .as_str()
                .expect("an asset target is a string")
        })
        .collect();
    assert!(
        !live_targets.is_empty(),
        "the package declares live assets for the roots to bound"
    );

    let unclaimed: Vec<(&str, usize)> = live_targets
        .iter()
        .map(|target| {
            (
                *target,
                roots
                    .iter()
                    .filter(|root| {
                        target
                            .strip_prefix(*root)
                            .is_some_and(|remainder| remainder.starts_with('/'))
                    })
                    .count(),
            )
        })
        .filter(|(_, claiming)| *claiming != 1)
        .collect();
    assert_eq!(
        unclaimed,
        Vec::new(),
        "each entry names a reported live asset target and the number of reported roots claiming it"
    );
}

#[test]
fn test_profile_unknown_id_has_typed_json_error_without_mutation() {
    let repo = TempDir::new().unwrap();
    let output = jit(
        repo.path(),
        &["init", "--profile", "missing-profile", "--json"],
    );

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(json(&output)["error"]["code"], "PROFILE_NOT_FOUND");
    assert!(!repo.path().join(".jit").exists());
}

#[test]
fn test_profiled_init_publishes_valid_repo_and_applied_inventory() {
    let repo = TempDir::new().unwrap();
    let init = jit(repo.path(), &["init", "--profile", "jit-dogfood", "--json"]);
    assert!(
        init.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&init.stdout),
        String::from_utf8_lossy(&init.stderr)
    );
    let init = json(&init);
    assert_eq!(init["profile"]["status"], "applied");
    assert!(repo.path().join(".jit/profiles/jit-dogfood.json").is_file());
    assert!(repo
        .path()
        .join(".agents/skills/jit-manage/SKILL.md")
        .is_file());
    assert_eq!(
        fs::read_to_string(repo.path().join(".jit/events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );

    let validate = jit(repo.path(), &["validate", "--json"]);
    assert!(validate.status.success(), "{validate:?}");
    assert_eq!(json(&validate)["valid"], true);

    let list = jit(repo.path(), &["profile", "list", "--json"]);
    assert!(list.status.success(), "{list:?}");
    assert_eq!(json(&list)["profiles"][0]["applied"], true);
}

#[test]
fn test_validate_plain_and_json_report_installed_profile_drift() {
    let repo = TempDir::new().unwrap();
    assert!(
        jit(repo.path(), &["init", "--profile", "jit-dogfood", "--json"])
            .status
            .success()
    );
    fs::write(
        repo.path().join(".agents/skills/jit-manage/SKILL.md"),
        "STALE\n",
    )
    .unwrap();

    let plain = jit(repo.path(), &["validate"]);
    assert!(!plain.status.success(), "{plain:?}");
    assert!(
        String::from_utf8_lossy(&plain.stdout).contains("jit-manage/SKILL.md"),
        "stdout={}",
        String::from_utf8_lossy(&plain.stdout)
    );

    let structured = jit(repo.path(), &["validate", "--json"]);
    assert!(!structured.status.success(), "{structured:?}");
    let structured = json(&structured);
    let details = &structured["error"]["details"];
    assert_eq!(details["valid"], false);
    assert!(details["integrity_error"]
        .as_str()
        .is_some_and(|message| message.contains("jit-manage/SKILL.md")));
}

#[test]
fn test_profile_apply_dry_run_is_read_only_then_apply_is_exact_no_op() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let events_before = fs::read(repo.path().join(".jit/events.jsonl")).unwrap();

    let preview = jit(
        repo.path(),
        &["profile", "apply", "jit-dogfood", "--dry-run", "--json"],
    );
    assert!(preview.status.success(), "{preview:?}");
    assert_eq!(json(&preview)["status"], "would_apply");
    assert_eq!(
        fs::read(repo.path().join(".jit/events.jsonl")).unwrap(),
        events_before
    );
    assert!(!repo.path().join(".jit/profiles").exists());

    let applied = jit(repo.path(), &["profile", "apply", "jit-dogfood", "--json"]);
    assert!(applied.status.success(), "{applied:?}");
    assert_eq!(json(&applied)["status"], "applied");
    let events_after = fs::read(repo.path().join(".jit/events.jsonl")).unwrap();

    let unchanged = jit(repo.path(), &["profile", "apply", "jit-dogfood", "--json"]);
    assert!(unchanged.status.success(), "{unchanged:?}");
    assert_eq!(json(&unchanged)["status"], "unchanged");
    assert_eq!(
        fs::read(repo.path().join(".jit/events.jsonl")).unwrap(),
        events_after
    );
}

#[test]
fn test_profile_reapply_repairs_missing_and_stale_default_schemas_before_no_op() {
    let repo = TempDir::new().unwrap();
    let initialized = jit(repo.path(), &["init", "--profile", "jit-dogfood", "--json"]);
    assert!(initialized.status.success(), "{initialized:?}");

    let namespace_schema = repo
        .path()
        .join(".jit/schemas/default-namespace-registry.json");
    let type_schema = repo
        .path()
        .join(".jit/schemas/default-type-hierarchy-known.json");
    let expected_namespace = fs::read(&namespace_schema).unwrap();
    let expected_types = fs::read(&type_schema).unwrap();
    let events_path = repo.path().join(".jit/events.jsonl");
    let event_count_before = fs::read_to_string(&events_path).unwrap().lines().count();

    fs::remove_file(&namespace_schema).unwrap();
    let preview = jit(
        repo.path(),
        &["profile", "apply", "jit-dogfood", "--dry-run", "--json"],
    );
    assert!(preview.status.success(), "{preview:?}");
    assert_eq!(json(&preview)["status"], "would_apply");
    assert!(!namespace_schema.exists(), "dry-run must remain read-only");

    let repaired_missing = jit(repo.path(), &["profile", "apply", "jit-dogfood", "--json"]);
    assert!(repaired_missing.status.success(), "{repaired_missing:?}");
    assert_eq!(json(&repaired_missing)["status"], "applied");
    assert_eq!(fs::read(&namespace_schema).unwrap(), expected_namespace);

    fs::write(&type_schema, b"stale\n").unwrap();
    let repaired_stale = jit(repo.path(), &["profile", "apply", "jit-dogfood", "--json"]);
    assert!(repaired_stale.status.success(), "{repaired_stale:?}");
    assert_eq!(json(&repaired_stale)["status"], "applied");
    assert_eq!(fs::read(&type_schema).unwrap(), expected_types);

    let events_after_repairs = fs::read(&events_path).unwrap();
    assert_eq!(
        std::str::from_utf8(&events_after_repairs)
            .unwrap()
            .lines()
            .count(),
        event_count_before + 2
    );
    let unchanged = jit(repo.path(), &["profile", "apply", "jit-dogfood", "--json"]);
    assert!(unchanged.status.success(), "{unchanged:?}");
    assert_eq!(json(&unchanged)["status"], "unchanged");
    assert_eq!(fs::read(events_path).unwrap(), events_after_repairs);
}

#[test]
fn test_profiled_init_conflict_leaves_no_jit_and_preserves_occupant() {
    let repo = TempDir::new().unwrap();
    fs::create_dir_all(repo.path().join("contrib/gates")).unwrap();
    let occupant = repo.path().join("contrib/gates/ai-review.sh");
    fs::write(&occupant, b"local script\n").unwrap();

    let output = jit(repo.path(), &["init", "--profile", "jit-dogfood", "--json"]);

    assert_eq!(output.status.code(), Some(4));
    assert_eq!(json(&output)["error"]["code"], "PROFILE_CONFLICT");
    assert_eq!(fs::read(&occupant).unwrap(), b"local script\n");
    assert!(!repo.path().join(".jit").exists());
}

#[test]
fn test_existing_partial_profiled_init_conflict_does_not_plain_init_first() {
    let repo = TempDir::new().unwrap();
    fs::create_dir_all(repo.path().join(".jit")).unwrap();
    let index = b"{\n  \"schema_version\": 2,\n  \"all_ids\": [],\n  \"deleted_ids\": []\n}";
    fs::write(repo.path().join(".jit/index.json"), index).unwrap();
    fs::create_dir_all(repo.path().join("contrib/gates")).unwrap();
    fs::write(
        repo.path().join("contrib/gates/ai-review.sh"),
        b"local script\n",
    )
    .unwrap();

    let output = jit(repo.path(), &["init", "--profile", "jit-dogfood", "--json"]);

    assert_eq!(output.status.code(), Some(4));
    assert_eq!(json(&output)["error"]["code"], "PROFILE_CONFLICT");
    assert_eq!(
        fs::read(repo.path().join(".jit/index.json")).unwrap(),
        index
    );
    for path in [
        "gates.toml",
        "events.jsonl",
        "config.toml",
        "rules.toml",
        "issues",
    ] {
        assert!(
            !repo.path().join(".jit").join(path).exists(),
            "profile preflight failure must not scaffold {path}"
        );
    }
}

#[test]
fn test_existing_partial_profiled_init_atomically_completes_neutral_scaffold() {
    let repo = TempDir::new().unwrap();
    fs::create_dir_all(repo.path().join(".jit")).unwrap();
    let index = b"{\n  \"schema_version\": 2,\n  \"all_ids\": [],\n  \"deleted_ids\": []\n}";
    fs::write(repo.path().join(".jit/index.json"), index).unwrap();

    let output = jit(repo.path(), &["init", "--profile", "jit-dogfood", "--json"]);

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for path in [
        "gates.toml",
        "events.jsonl",
        "config.toml",
        "rules.toml",
        "issues",
        "profiles/jit-dogfood.json",
    ] {
        assert!(
            repo.path().join(".jit").join(path).exists(),
            "missing {path}"
        );
    }
    assert_eq!(
        fs::read(repo.path().join(".jit/index.json")).unwrap(),
        index,
        "existing neutral bytes must be preserved"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".jit/events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(jit(repo.path(), &["validate", "--json"]).status.success());
}
