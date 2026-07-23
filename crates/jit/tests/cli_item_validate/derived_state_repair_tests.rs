use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

const RULE_ASSERTION: &str = "assert = { json-schema = \"schemas/default-label-format.json\" }";
const STALE_RULE_ASSERTION: &str =
    "assert = { require-label = { label = \"authored:*\", min = 99 } }";
const PROFILE_ASSET: &str = ".agents/skills/jit-manage/SKILL.md";
const EXECUTABLE_ASSET: &str = "scripts/ai-review.sh";
const PROFILE_RECORD: &str = ".jit/profiles/jit-dogfood.json";

fn run(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(args)
        .current_dir(repo)
        .env("JIT_TEST_MODE", "1")
        .output()
        .unwrap()
}

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

fn profiled_repo() -> TempDir {
    let repo = TempDir::new().unwrap();
    let output = run(repo.path(), &["init", "--profile", "jit-dogfood", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    repo
}

fn read(repo: &Path, path: &str) -> Vec<u8> {
    fs::read(repo.join(path)).unwrap()
}

#[derive(PartialEq)]
struct FileState {
    bytes: Vec<u8>,
    executable: bool,
}

fn file_snapshot(root: &Path) -> BTreeMap<PathBuf, FileState> {
    const MACHINE_LOCAL_LOCKS: [&str; 5] = [
        ".jit-bootstrap.lock",
        ".jit/.events.lock",
        ".jit/.gates.lock",
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
fn test_cli_validate_fix_repairs_each_owned_class_and_preserves_authored_bytes() {
    let repo = profiled_repo();
    let root = repo.path();
    let baseline = [
        ".jit/rules.toml",
        ".jit/schemas/default-type-hierarchy-known.json",
        ".jit/reference/rules-and-gates.md",
        "AGENTS.md",
        PROFILE_ASSET,
    ]
    .into_iter()
    .map(|path| (path, read(root, path)))
    .collect::<BTreeMap<_, _>>();

    let baseline_rules = String::from_utf8(baseline[".jit/rules.toml"].clone()).unwrap();
    let authored_rules = format!(
        "# authored header remains byte-exact\n{}",
        baseline_rules
            .replacen(
                "severity = \"error\"",
                "severity = \"error\" # authored policy",
                1
            )
            .replacen(RULE_ASSERTION, STALE_RULE_ASSERTION, 1)
    );
    let expected_rules = authored_rules.replacen(STALE_RULE_ASSERTION, RULE_ASSERTION, 1);
    fs::write(root.join(".jit/rules.toml"), authored_rules).unwrap();

    let mut schema = baseline[".jit/schemas/default-type-hierarchy-known.json"].clone();
    schema.extend_from_slice(b" stale");
    fs::write(
        root.join(".jit/schemas/default-type-hierarchy-known.json"),
        schema,
    )
    .unwrap();
    fs::write(
        root.join(".jit/reference/rules-and-gates.md"),
        b"STALE FULL PROJECTION\n",
    )
    .unwrap();

    let baseline_agents = String::from_utf8(baseline["AGENTS.md"].clone()).unwrap();
    let stale_agents = format!(
        "AUTHORED PREFIX\n{}\nAUTHORED SUFFIX\n",
        baseline_agents
            .replace("_No items declared._", "STALE CONFIGURED REGION")
            .replace("## JIT workflow", "## STALE PROFILE REGION")
    );
    fs::write(root.join("AGENTS.md"), stale_agents).unwrap();
    fs::write(root.join(PROFILE_ASSET), b"STALE PROFILE ASSET\n").unwrap();

    let unowned = root.join(".jit/schemas/default-convention-only.json");
    fs::write(&unowned, b"authored and unowned\n").unwrap();

    let first = run(root, &["validate", "--json"]);
    let second = run(root, &["validate", "--json"]);
    let human = run(root, &["validate"]);
    assert!(!first.status.success());
    assert!(!human.status.success());
    assert_eq!(
        json(&first),
        json(&second),
        "drift diagnosis must be deterministic"
    );
    let diagnosis = String::from_utf8_lossy(&first.stdout);
    let human_diagnosis = format!(
        "{}{}",
        String::from_utf8_lossy(&human.stdout),
        String::from_utf8_lossy(&human.stderr)
    );
    for path in [
        "rules.toml",
        "default-type-hierarchy-known.json",
        "rules-and-gates.md",
        "AGENTS.md",
        PROFILE_ASSET,
    ] {
        assert!(
            diagnosis.contains(path) && human_diagnosis.contains(path),
            "missing drift path {path}: json={diagnosis}; human={human_diagnosis}"
        );
    }
    assert!(
        !diagnosis.contains("authored:*"),
        "persisted default assertion became effective authority: {diagnosis}"
    );

    let fixed = run(root, &["validate", "--fix", "--json"]);
    assert!(
        fixed.status.success(),
        "{}",
        String::from_utf8_lossy(&fixed.stderr)
    );
    assert_eq!(read(root, ".jit/rules.toml"), expected_rules.as_bytes());
    assert_eq!(
        read(root, ".jit/schemas/default-type-hierarchy-known.json"),
        baseline[".jit/schemas/default-type-hierarchy-known.json"]
    );
    assert_eq!(
        read(root, ".jit/reference/rules-and-gates.md"),
        baseline[".jit/reference/rules-and-gates.md"]
    );
    assert_eq!(
        String::from_utf8(read(root, "AGENTS.md")).unwrap(),
        format!("AUTHORED PREFIX\n{}\nAUTHORED SUFFIX\n", baseline_agents)
    );
    assert_eq!(read(root, PROFILE_ASSET), baseline[PROFILE_ASSET]);
    assert_eq!(fs::read(&unowned).unwrap(), b"authored and unowned\n");

    let no_op = run(root, &["validate", "--fix", "--json"]);
    assert!(no_op.status.success());
    assert_eq!(json(&no_op)["fixes_applied"], 0);
}

#[cfg(unix)]
#[test]
fn test_cli_validate_fix_repairs_mode_only_profile_drift() {
    use std::os::unix::fs::PermissionsExt;

    let repo = profiled_repo();
    let path = repo.path().join(EXECUTABLE_ASSET);
    let bytes = fs::read(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

    let drift = run(repo.path(), &["validate", "--json"]);
    assert!(!drift.status.success());
    assert!(String::from_utf8_lossy(&drift.stdout).contains(EXECUTABLE_ASSET));

    let fixed = run(repo.path(), &["validate", "--fix", "--json"]);
    assert!(fixed.status.success());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_ne!(fs::metadata(&path).unwrap().permissions().mode() & 0o111, 0);
    assert_eq!(
        json(&run(repo.path(), &["validate", "--fix", "--json"]))["fixes_applied"],
        0
    );
}

#[test]
fn test_cli_validate_fix_preserves_malformed_rules_validation_classification() {
    for json_mode in [false, true] {
        let repo = profiled_repo();
        fs::write(repo.path().join(".jit/rules.toml"), "[[rules]\n").unwrap();
        let before = file_snapshot(repo.path());
        let args = if json_mode {
            ["validate", "--fix", "--json"].as_slice()
        } else {
            ["validate", "--fix"].as_slice()
        };
        let output = run(repo.path(), args);

        assert_eq!(output.status.code(), Some(4));
        if json_mode {
            let error = json(&output);
            assert_eq!(error["error"]["code"], "VALIDATION_FAILED");
            assert!(error["error"]["message"]
                .as_str()
                .unwrap()
                .contains("TOML parse error"));
        } else {
            assert!(String::from_utf8_lossy(&output.stderr).contains("TOML parse error"));
        }
        assert_snapshot_unchanged(repo.path(), &before);
    }
}

#[cfg(unix)]
#[test]
fn test_cli_validate_fix_preserves_permission_error_classification() {
    use std::os::unix::fs::PermissionsExt;

    for json_mode in [false, true] {
        let repo = profiled_repo();
        let schema = repo
            .path()
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
        let output = run(repo.path(), args);
        fs::set_permissions(parent, fs::Permissions::from_mode(original_mode)).unwrap();

        assert_eq!(output.status.code(), Some(5));
        if json_mode {
            let error = json(&output);
            assert_eq!(error["error"]["code"], "IO_ERROR");
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
fn test_cli_validate_fix_rejects_ambiguous_ownership_transactionally() {
    let repo = profiled_repo();
    let rules_path = repo.path().join(".jit/rules.toml");
    let agents_path = repo.path().join("AGENTS.md");
    fs::write(
        &rules_path,
        fs::read_to_string(&rules_path)
            .unwrap()
            .replacen(RULE_ASSERTION, STALE_RULE_ASSERTION, 1),
    )
    .unwrap();
    fs::write(
        &agents_path,
        fs::read_to_string(&agents_path).unwrap().replacen(
            "<!-- jit:invariants:begin -->",
            "<!-- jit:invariants:begin -->\n<!-- jit:invariants:begin -->",
            1,
        ),
    )
    .unwrap();
    fs::write(repo.path().join("authored.lock"), b"authored lock file\n").unwrap();
    let before = file_snapshot(repo.path());
    assert!(before.contains_key(Path::new("authored.lock")));

    let human = run(repo.path(), &["validate", "--fix"]);
    assert!(!human.status.success());
    assert_eq!(human.status.code(), Some(4));
    let human_error = String::from_utf8_lossy(&human.stderr);
    assert!(
        human_error.contains("derived-materialization validation pass"),
        "{human_error}"
    );
    assert!(
        human_error.contains("duplicate delimiters"),
        "{human_error}"
    );
    assert!(human_error.contains("invariants"), "{human_error}");

    let output = run(repo.path(), &["validate", "--fix", "--json"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));
    let error = json(&output);
    assert_eq!(error["error"]["code"], "VALIDATION_FAILED");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("derived-materialization validation pass"));
    assert!(message.contains("duplicate delimiters"));
    assert!(message.contains("invariants"));
    assert_snapshot_unchanged(repo.path(), &before);
}

#[test]
fn test_cli_validate_fix_profile_provenance_failures_are_zero_write() {
    for mismatch in [true, false] {
        let repo = profiled_repo();
        fs::write(repo.path().join(PROFILE_ASSET), b"STALE PROFILE ASSET\n").unwrap();
        if mismatch {
            let mut record: Value =
                serde_json::from_slice(&read(repo.path(), PROFILE_RECORD)).unwrap();
            record["package_hash"] = Value::String("0".repeat(64));
            fs::write(
                repo.path().join(PROFILE_RECORD),
                serde_json::to_vec_pretty(&record).unwrap(),
            )
            .unwrap();
        } else {
            fs::remove_file(repo.path().join(PROFILE_RECORD)).unwrap();
        }
        let before = file_snapshot(repo.path());

        let human = run(repo.path(), &["validate", "--fix"]);
        let output = run(repo.path(), &["validate", "--fix", "--json"]);
        if mismatch {
            assert!(!human.status.success());
            assert_eq!(human.status.code(), Some(4));
            let human_error = String::from_utf8_lossy(&human.stderr);
            assert!(
                human_error.contains("applied profile provenance"),
                "{human_error}"
            );
            assert!(human_error.contains("does not match"), "{human_error}");
            assert!(!output.status.success());
            assert_eq!(output.status.code(), Some(4));
            let error = json(&output);
            assert_eq!(error["error"]["code"], "VALIDATION_FAILED");
            let message = error["error"]["message"].as_str().unwrap();
            assert!(message.contains("applied profile provenance"));
            assert!(message.contains("does not match"));
        } else {
            assert!(human.status.success());
            assert!(output.status.success());
            assert_eq!(json(&output)["fixes_applied"], 0);
        }
        assert_snapshot_unchanged(repo.path(), &before);
    }
}
