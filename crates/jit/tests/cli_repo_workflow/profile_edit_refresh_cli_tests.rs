//! CLI regression coverage for the profile-owned edit-and-refresh journey.

use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

const PROFILE: &str = "captured";
const PACKAGE: &str = "packages/captured";
const TARGET: &str = "docs/guide.md";

fn jit(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run jit")
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

fn requested(application: &Value) -> &Value {
    application["profiles"]
        .as_array()
        .expect("an application reports profiles")
        .last()
        .expect("an application reports its requested profile")
}

fn only_profile(collection: &Value) -> &Value {
    assert_eq!(collection["count"], 1);
    collection["profiles"]
        .as_array()
        .expect("a profile collection reports profiles")
        .first()
        .expect("a one-entry collection reports its profile")
}

fn profile_record(repo: &Path) -> Value {
    serde_json::from_slice(
        &fs::read(repo.join(format!(".jit/profiles/{PROFILE}.json")))
            .expect("read applied profile record"),
    )
    .expect("parse applied profile record")
}

fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, result: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(directory).expect("read package directory") {
            let path = entry.expect("read package entry").path();
            if path.is_dir() {
                visit(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root)
                        .expect("package-relative file")
                        .to_string_lossy()
                        .into_owned(),
                    fs::read(path).expect("read package file"),
                );
            }
        }
    }

    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn write_live_source_package(repo: &Path) {
    let root = repo.join(PACKAGE);
    fs::create_dir_all(root.join("assets/install")).expect("create package directories");
    fs::write(
        root.join("manifest.toml"),
        "[profile]\nmanifest-version = 1\nid = \"captured\"\nversion = \"1.0.0\"\njit = \">=0.2.0, <2.0.0\"\n\n[[live-source]]\nroot = \"docs\"\nexclude = []\n\n[[asset]]\nsource = \"assets/live/docs/guide.md\"\ntarget = \"docs/guide.md\"\n\n[[asset]]\nsource = \"assets/install/settings.toml\"\ntarget = \"settings.toml\"\n",
    )
    .expect("write package manifest");
    fs::write(
        root.join("assets/install/settings.toml"),
        "installed = true\n",
    )
    .expect("write static package asset");
    fs::create_dir_all(repo.join("docs")).expect("create owned target directory");
    fs::write(repo.join(TARGET), "# Captured guide\n").expect("write owned target");
}

fn has_changed_target(report: &Value) -> bool {
    report["error"]["details"]["profiles"]
        .as_array()
        .expect("a divergent report carries profiles")
        .iter()
        .find(|profile| profile["id"] == PROFILE)
        .expect("the report includes the profile")["divergences"]
        .as_array()
        .expect("a profile report carries divergences")
        .iter()
        .any(|entry| entry["kind"] == "changed_target" && entry["target"] == TARGET)
}

/// Capturing an in-place edit makes the same package identity/version
/// re-applicable and returns the repository to recorded agreement
/// (jit:acf49914).
#[test]
fn test_profile_capture_refreshes_an_edited_owned_target_back_into_agreement() {
    let repo = TempDir::new().expect("create repository");
    assert!(jit(repo.path(), &["init"]).status.success());
    write_live_source_package(repo.path());

    let capture = jit(
        repo.path(),
        &[
            "profile",
            "capture",
            "--source",
            PACKAGE,
            "--destination",
            PACKAGE,
            "--json",
        ],
    );
    assert!(capture.status.success(), "{capture:?}");
    let capture = json(&capture);
    let capture = only_profile(&capture);
    assert_eq!(capture["id"], PROFILE);
    assert_eq!(capture["version"], "1.0.0");
    assert_eq!(capture["status"], "applied");

    let initial_apply = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            "path:packages/captured",
            "--json",
        ],
    );
    assert!(initial_apply.status.success(), "{initial_apply:?}");
    let initial_apply = json(&initial_apply);
    assert_eq!(requested(&initial_apply)["id"], capture["id"]);
    assert_eq!(requested(&initial_apply)["version"], capture["version"]);
    assert_eq!(requested(&initial_apply)["status"], "applied");
    let initial_record = profile_record(repo.path());

    let edited = b"# Edited in place\n";
    fs::write(repo.path().join(TARGET), edited).expect("edit the owned target");
    let divergent = jit(repo.path(), &["profile", "validate", "--json"]);
    assert_eq!(divergent.status.code(), Some(4), "{divergent:?}");
    assert!(has_changed_target(&json(&divergent)));

    let repository_before_refusal = files(repo.path());
    let refused = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            "path:packages/captured",
            "--json",
        ],
    );
    assert_eq!(refused.status.code(), Some(4), "{refused:?}");
    assert_eq!(json(&refused)["error"]["code"], "PROFILE_CONFLICT");
    assert_eq!(fs::read(repo.path().join(TARGET)).unwrap(), edited);
    assert_eq!(
        files(repo.path()),
        repository_before_refusal,
        "the refused application wrote somewhere in the repository"
    );

    let refreshed = jit(
        repo.path(),
        &[
            "profile",
            "capture",
            "--source",
            PACKAGE,
            "--destination",
            PACKAGE,
            "--json",
        ],
    );
    assert!(refreshed.status.success(), "{refreshed:?}");
    let refreshed = json(&refreshed);
    let refreshed = only_profile(&refreshed);
    assert_eq!(refreshed["id"], capture["id"]);
    assert_eq!(refreshed["version"], capture["version"]);
    assert_eq!(refreshed["status"], "applied");
    assert_ne!(refreshed["package_hash"], capture["package_hash"]);
    assert!(refreshed["targets"]
        .as_array()
        .expect("a capture reports file decisions")
        .iter()
        .any(
            |file| file["path"] == "packages/captured/assets/live/docs/guide.md"
                && file["action"] == "update"
        ));
    assert_eq!(
        fs::read(
            repo.path()
                .join("packages/captured/assets/live/docs/guide.md")
        )
        .unwrap(),
        edited
    );

    let reapplied = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            "path:packages/captured",
            "--json",
        ],
    );
    assert!(reapplied.status.success(), "{reapplied:?}");
    let reapplied = json(&reapplied);
    assert_eq!(requested(&reapplied)["id"], capture["id"]);
    assert_eq!(requested(&reapplied)["version"], capture["version"]);
    assert_eq!(requested(&reapplied)["status"], "applied");
    assert_eq!(
        profile_record(repo.path())["package_hash"],
        refreshed["package_hash"]
    );
    assert_ne!(
        profile_record(repo.path())["package_hash"],
        initial_record["package_hash"]
    );
    assert_eq!(fs::read(repo.path().join(TARGET)).unwrap(), edited);

    let agreement = jit(repo.path(), &["profile", "validate", "--json"]);
    assert!(agreement.status.success(), "{agreement:?}");
    assert!(json(&agreement)["profiles"]
        .as_array()
        .expect("an agreement report carries profiles")
        .iter()
        .find(|profile| profile["id"] == PROFILE)
        .expect("the agreement report includes the profile")["divergences"]
        .as_array()
        .expect("a profile report carries divergences")
        .is_empty());
}
