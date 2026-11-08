use jit::test_cli::JitCli;
use jit::test_utils::{profiled_repository_fixture, PROFILE_PACKAGE_SOURCES};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON: {error}\nstatus={}\nstdout={}\nstderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[derive(PartialEq)]
struct FileState {
    bytes: Vec<u8>,
    executable: bool,
}

fn file_snapshot(root: &Path) -> BTreeMap<PathBuf, FileState> {
    const MACHINE_LOCAL_LOCKS: [&str; 4] = [
        ".jit-bootstrap.lock",
        ".jit/.events.lock",
        ".jit/.index.lock",
        ".jit/.repo-write.lock",
    ];

    fn visit(root: &Path, current: &Path, files: &mut BTreeMap<PathBuf, FileState>) {
        for entry in fs::read_dir(current).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap();
            if MACHINE_LOCAL_LOCKS
                .iter()
                .any(|lock| relative == Path::new(lock))
            {
                continue;
            }
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                #[cfg(unix)]
                let executable = {
                    use std::os::unix::fs::PermissionsExt;
                    fs::metadata(&path).unwrap().permissions().mode() & 0o111 != 0
                };
                #[cfg(not(unix))]
                let executable = false;
                files.insert(
                    relative.to_path_buf(),
                    FileState {
                        bytes: fs::read(path).unwrap(),
                        executable,
                    },
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn assert_snapshot_unchanged(root: &Path, before: &BTreeMap<PathBuf, FileState>) {
    let after = file_snapshot(root);
    let changed = before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(changed.is_empty(), "unexpected writes: {changed:?}");
}

#[test]
fn test_cli_validate_fix_preserves_malformed_rules_validation_classification() {
    for json_mode in [false, true] {
        let cli = JitCli::with_repository(
            profiled_repository_fixture("jit-dogfood", PROFILE_PACKAGE_SOURCES).unwrap(),
        );
        fs::write(cli.root().join(".jit/rules.toml"), "[[rules]\n").unwrap();
        let before = file_snapshot(cli.root());
        let args = if json_mode {
            ["validate", "--fix", "--json"].as_slice()
        } else {
            ["validate", "--fix"].as_slice()
        };
        let output = cli.command(args).output().unwrap();

        assert!(!output.status.success());
        if json_mode {
            let error = json(&output);
            assert_eq!(error["error"]["code"], "VALIDATION_FAILED");
            assert!(error["error"]["details"]["integrity_error"]
                .as_str()
                .unwrap()
                .contains("TOML parse error"));
        } else {
            assert!(String::from_utf8_lossy(&output.stdout).contains("TOML parse error"));
        }
        assert_snapshot_unchanged(cli.root(), &before);
    }
}

#[cfg(unix)]
#[test]
fn test_cli_validate_fix_preserves_permission_error_classification() {
    use std::os::unix::fs::PermissionsExt;

    for json_mode in [false, true] {
        let cli = JitCli::with_repository(
            profiled_repository_fixture("jit-dogfood", PROFILE_PACKAGE_SOURCES).unwrap(),
        );
        let schema = cli
            .root()
            .join(".jit/schemas/default-type-hierarchy-known.json");
        let mut stale = fs::read(&schema).unwrap();
        stale.extend_from_slice(b" stale");
        fs::write(&schema, stale).unwrap();

        let parent = schema.parent().unwrap();
        let original_mode = fs::metadata(parent).unwrap().permissions().mode();
        fs::set_permissions(parent, fs::Permissions::from_mode(0o500)).unwrap();
        let args = if json_mode {
            ["validate", "--fix", "--json"].as_slice()
        } else {
            ["validate", "--fix"].as_slice()
        };
        let output = cli.command(args).output().unwrap();
        fs::set_permissions(parent, fs::Permissions::from_mode(original_mode)).unwrap();

        assert!(!output.status.success());
        if json_mode {
            let error = json(&output);
            assert_eq!(error.as_object().map(|object| object.len()), Some(1));
            assert!(output.stderr.is_empty(), "JSON failure stays stream-pure");
            let code = error["error"]["code"]
                .as_str()
                .unwrap()
                .parse::<jit::output::ErrorCode>()
                .expect("validate --fix reports a registered error code");
            assert_eq!(code, jit::output::ErrorCode::PermissionDenied);
            assert!(error["error"]["message"]
                .as_str()
                .unwrap()
                .to_ascii_lowercase()
                .contains("permission denied"));
        } else {
            assert!(String::from_utf8_lossy(&output.stderr)
                .to_ascii_lowercase()
                .contains("permission denied"));
        }
    }
}

#[test]
fn test_validate_derived_state_drift_reports_repair_command_and_repository_paths_on_stdout() {
    let cli = JitCli::initialised().unwrap();

    let config_path = cli.root().join(".jit/config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(
        config_path,
        format!("{config}\n[type_hierarchy]\ntypes = {{ custom = 4 }}\n"),
    )
    .unwrap();

    let validation = cli.command(["validate"]).output().unwrap();
    assert!(!validation.status.success(), "{validation:?}");
    let diagnostic = String::from_utf8_lossy(&validation.stdout);
    assert!(
        diagnostic.contains("jit validate --fix"),
        "diagnostic should name the repair command: {diagnostic}"
    );
    assert!(
        diagnostic.contains(".jit/rules.toml"),
        "diagnostic should use repository-relative paths: {diagnostic}"
    );
    assert!(
        !diagnostic.contains("VirtualPath {"),
        "diagnostic should not expose virtual-path debug formatting: {diagnostic}"
    );
}

#[test]
fn test_recover_repairs_derived_state_drift_and_reports_repair_in_json() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git()
        .unwrap()
        .commit("Initialize recovery fixture")
        .unwrap();

    let config_path = cli.root().join(".jit/config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(
        config_path,
        format!("{config}\n[type_hierarchy]\ntypes = {{ custom = 4 }}\n"),
    )
    .unwrap();

    let recovered = cli.command(["recover", "--json"]).output().unwrap();
    assert!(recovered.status.success(), "{recovered:?}");
    let report = json(&recovered);
    assert!(
        report["derived_state_repaired"].as_u64().unwrap_or(0) > 0,
        "recovery must report a structural repair count: {report}"
    );

    let validation = cli.command(["validate", "--json"]).output().unwrap();
    assert!(validation.status.success(), "{validation:?}");
    assert_eq!(json(&validation)["valid"], true);
}

/// A git repository with no commits resolves no branch, so the git-dependent
/// recovery phase fails there. Derived-state repair must still have run: the
/// repair is a core capability and cannot depend on git history
/// (`@/charter/D-4`).
#[test]
fn test_recover_repairs_derived_state_drift_in_git_repository_without_commits() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().run(["init"]).unwrap();

    let config_path = cli.root().join(".jit/config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(
        config_path,
        format!("{config}\n[type_hierarchy]\ntypes = {{ custom = 4 }}\n"),
    )
    .unwrap();

    // The git-dependent phase still fails on the unresolved HEAD; that
    // behaviour is deliberate and preserved.
    let recovered = cli.command(["recover", "--json"]).output().unwrap();
    assert!(
        !recovered.status.success(),
        "recovery must still surface the unresolved-HEAD failure: {recovered:?}"
    );

    // The repair nevertheless happened, which is what the requirement asks for.
    let validation = cli.command(["validate", "--json"]).output().unwrap();
    assert!(
        validation.status.success(),
        "recovery must repair derived state before the git phase fails: {validation:?}"
    );
    assert_eq!(json(&validation)["valid"], true);
}

#[test]
fn test_recover_repairs_derived_state_drift_without_git() {
    let cli = JitCli::initialised().unwrap();

    let config_path = cli.root().join(".jit/config.toml");
    let config = fs::read_to_string(&config_path).unwrap();
    fs::write(
        config_path,
        format!("{config}\n[type_hierarchy]\ntypes = {{ custom = 4 }}\n"),
    )
    .unwrap();

    let recovered = cli.command(["recover", "--json"]).output().unwrap();
    assert!(recovered.status.success(), "{recovered:?}");
    let report = json(&recovered);
    assert!(
        report["derived_state_repaired"].as_u64().unwrap_or(0) > 0,
        "recovery must report a structural repair count: {report}"
    );
    assert_eq!(
        report["claims_lease_recovery"]["status"], "skipped",
        "recovery must report that git-dependent recovery was skipped: {report}"
    );
    assert_eq!(
        report["claims_lease_recovery"]["reason"], "requires_git",
        "recovery must report why git-dependent recovery was skipped: {report}"
    );
    assert!(
        report["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| {
                *warning == jit::storage::StorageWarning::ClaimsLeaseRecoverySkipped.to_string()
            }),
        "recovery must expose the skip through its warning mechanism: {report}"
    );

    let validation = cli.command(["validate", "--json"]).output().unwrap();
    assert!(validation.status.success(), "{validation:?}");
    assert_eq!(json(&validation)["valid"], true);
}
