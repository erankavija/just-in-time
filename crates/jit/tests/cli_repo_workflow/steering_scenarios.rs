//! Deterministic steering-scenario harness (issue 7aacfd89, CC-9).
//!
//! Exercises validation steering end to end by driving the real `jit` binary in
//! isolated temp repos against the seed scenarios from the June 2026 evaluation.
//! Each scenario is defined purely as data in
//! `crates/jit/tests/fixtures/steering/<name>/scenario.toml`.
//!
//! ## Adding a scenario
//!
//! Create a new subdirectory under `crates/jit/tests/fixtures/steering/` and
//! place a `scenario.toml` there. The test runner enumerates the directory and
//! runs every scenario automatically. No Rust changes are needed.
//!
//! See `crates/jit/tests/fixtures/steering/README.md` for the full schema.
//!
//! ## Isolation guarantee
//!
//! Every scenario runs in a fresh `TempDir`. Two mechanisms together guarantee
//! that the production `.jit/` is never touched:
//!
//! 1. **cwd isolation** — every `jit` invocation uses `.current_dir(temp.path())`
//!    so the default `.jit/` resolution anchors inside the temp directory.
//! 2. **env sanitization** — all `jit` invocations are constructed through
//!    `jit_cmd()`, which calls `.env_remove` on every JIT_* variable that can
//!    redirect or alter storage (`JIT_DATA_DIR`, `JIT_ALLOW_DELETION`,
//!    `JIT_WORKTREE_MODE`, `JIT_ENFORCE_LEASES`, `JIT_LOCK_TIMEOUT`) and pins
//!    `JIT_AGENT_ID` to `"agent:steering-harness"` for determinism.
//!    An absolute `JIT_DATA_DIR` value would otherwise discard the temp cwd and
//!    write directly to an arbitrary path on the developer's machine.
//!
//! ## Fresh-evidence note
//!
//! A `gate-recency` scenario requires back-dating `GateState.updated_at`, which
//! cannot be expressed via CLI commands alone. It is therefore excluded from this
//! harness; recency is covered by `example_rulesets_tests.rs` via clock injection.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Scenario schema types
// ---------------------------------------------------------------------------

/// Top-level scenario descriptor.
#[derive(Debug, Deserialize)]
struct Scenario {
    /// Ruleset name under `docs/examples/<ruleset>/`.
    ruleset: String,
    /// Ordered sequence of CLI steps.
    steps: Vec<Step>,
}

/// One CLI step.
#[derive(Debug, Deserialize)]
struct Step {
    /// `jit` subcommand + arguments (the `jit` binary itself is omitted).
    argv: Vec<String>,
    /// What to capture from this step's output.
    #[serde(default = "default_capture")]
    capture: CaptureMode,
    /// Slot name for the captured UUID (used when `capture = "id"`).
    id_slot: Option<String>,
    /// Optional per-step assertion.
    expect: Option<Expect>,
}

fn default_capture() -> CaptureMode {
    CaptureMode::Id
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CaptureMode {
    /// Extract the first UUID from stdout and store it under `id_slot`.
    Id,
    /// Do not capture anything.
    None,
}

/// Per-step assertion.
#[derive(Debug, Deserialize)]
struct Expect {
    /// Expected process exit code.
    exit: Option<i32>,
    /// Substrings that must appear in the combined stdout + stderr.
    #[serde(default)]
    contains: Vec<String>,
    /// Substrings that must NOT appear in the combined stdout + stderr.
    #[serde(default)]
    not_contains: Vec<String>,
    /// Informational label; validated against the step type derived from argv.
    enforcement_point: Option<EnforcementPoint>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum EnforcementPoint {
    /// A `jit issue create` or `jit issue update` step (write-path local rules).
    Write,
    /// A `jit validate` step (graph rules in validate mode).
    Validate,
    /// A `jit issue update --state <target>` step (transition-time graph rules).
    Transition,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

/// Root of the workspace; used to locate `docs/examples/`.
fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points to `crates/jit`; go two levels up.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root must exist")
}

/// Build a sanitized `jit` [`Command`] anchored in `dir`.
///
/// Removes every JIT_* environment variable that can redirect or alter storage,
/// and pins `JIT_AGENT_ID` to a fixed harness identity. This is the single place
/// all scenario spawns go through; never call `Command::new(jit_binary())` directly
/// in this module.
///
/// Variables removed:
/// - `JIT_DATA_DIR` — would redirect `.jit/` to an arbitrary (possibly absolute) path.
/// - `JIT_ALLOW_DELETION` — would allow destructive operations not expected by tests.
/// - `JIT_WORKTREE_MODE` — could alter worktree resolution, masking the temp dir.
/// - `JIT_ENFORCE_LEASES` — could change lease-enforcement behaviour.
/// - `JIT_LOCK_TIMEOUT` — could slow tests or interact with external lock state.
fn jit_cmd(dir: &Path) -> Command {
    let mut cmd = Command::new(jit_binary());
    cmd.current_dir(dir)
        .env_remove("JIT_DATA_DIR")
        .env_remove("JIT_ALLOW_DELETION")
        .env_remove("JIT_WORKTREE_MODE")
        .env_remove("JIT_ENFORCE_LEASES")
        .env_remove("JIT_LOCK_TIMEOUT")
        .env("JIT_AGENT_ID", "agent:steering-harness");
    cmd
}

/// Initialize a fresh isolated repo and install the named ruleset.
fn setup_scenario_repo(ruleset: &str) -> TempDir {
    let temp = TempDir::new().expect("tempdir");

    // `jit init` in the temp dir.
    let out = jit_cmd(temp.path())
        .arg("init")
        .output()
        .expect("jit init failed to spawn");
    assert!(
        out.status.success(),
        "jit init failed in scenario repo: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Overwrite rules.toml and install schemas/ from docs/examples/<ruleset>/.
    let example_dir = workspace_root().join("docs/examples").join(ruleset);
    assert!(
        example_dir.exists(),
        "ruleset directory does not exist: {}",
        example_dir.display()
    );

    let rules_src = example_dir.join("rules.toml");
    let rules_dst = temp.path().join(".jit/rules.toml");
    fs::copy(&rules_src, &rules_dst)
        .unwrap_or_else(|e| panic!("failed to copy {}: {e}", rules_src.display()));

    let schemas_src = example_dir.join("schemas");
    if schemas_src.exists() {
        let schemas_dst = temp.path().join(".jit/schemas");
        // Remove the scaffolded schemas dir first (jit init creates it).
        if schemas_dst.exists() {
            fs::remove_dir_all(&schemas_dst).unwrap();
        }
        copy_dir_all(&schemas_src, &schemas_dst)
            .unwrap_or_else(|e| panic!("failed to copy schemas: {e}"));
    }

    temp
}

/// Recursively copy a directory.
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}

/// Extract the first UUID (8-4-4-4-12 hex) from a string.
fn extract_uuid(s: &str) -> Option<String> {
    // Simple pattern scan: look for the standard UUID shape.
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 36 <= bytes.len() {
        let candidate = &s[i..i + 36];
        if is_uuid(candidate) {
            return Some(candidate.to_string());
        }
        i += 1;
    }
    None
}

fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    // Pattern: 8-4-4-4-12 hex digits separated by hyphens at positions 8,13,18,23.
    for (idx, &byte) in b.iter().enumerate() {
        if idx == 8 || idx == 13 || idx == 18 || idx == 23 {
            if byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// Substitute `$<slot>` and `$<slot>_short` in argv with captured ids.
///
/// Slots are applied longest-name-first so that a slot named `epic2` is never
/// partially consumed by a shorter slot named `epic` (e.g. `$epic2` would
/// otherwise be rewritten to `<uuid>2` when the `epic` slot is applied first).
fn substitute_argv(argv: &[String], ids: &HashMap<String, String>) -> Vec<String> {
    // Sort slot names by descending length so longer names are replaced first.
    let mut slots: Vec<(&String, &String)> = ids.iter().collect();
    slots.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));

    argv.iter()
        .map(|arg| {
            let mut result = arg.clone();
            for (slot, uuid) in &slots {
                let short = &uuid[..8];
                // Replace _short first to avoid the plain-slot pattern consuming
                // the leading characters of `$<slot>_short`.
                result = result.replace(&format!("${slot}_short"), short);
                result = result.replace(&format!("${slot}"), uuid.as_str());
            }
            result
        })
        .collect()
}

/// Derive the enforcement point from the step's argv.
fn step_enforcement_point(argv: &[String]) -> Option<EnforcementPoint> {
    // ["validate"] or ["validate", ...]
    if argv.first().map(|s| s.as_str()) == Some("validate") {
        return Some(EnforcementPoint::Validate);
    }
    // ["issue", "update", ..., "--state", ...]
    if argv.first().map(|s| s.as_str()) == Some("issue")
        && argv.get(1).map(|s| s.as_str()) == Some("update")
        && argv.iter().any(|a| a == "--state" || a == "-s")
    {
        return Some(EnforcementPoint::Transition);
    }
    // ["issue", "create"] or ["issue", "update"] (no --state)
    if argv.first().map(|s| s.as_str()) == Some("issue")
        && (argv.get(1).map(|s| s.as_str()) == Some("create")
            || argv.get(1).map(|s| s.as_str()) == Some("update"))
    {
        return Some(EnforcementPoint::Write);
    }
    None
}

// ---------------------------------------------------------------------------
// Core runner
// ---------------------------------------------------------------------------

fn run_scenario(name: &str, fixture_dir: &Path) {
    let toml_path = fixture_dir.join("scenario.toml");
    let toml_src = fs::read_to_string(&toml_path)
        .unwrap_or_else(|e| panic!("scenario {name}: cannot read scenario.toml: {e}"));
    let scenario: Scenario = toml::from_str(&toml_src)
        .unwrap_or_else(|e| panic!("scenario {name}: cannot parse scenario.toml: {e}"));

    let repo = setup_scenario_repo(&scenario.ruleset);

    // Captured id slots: slot_name -> full UUID.
    let mut ids: HashMap<String, String> = HashMap::new();

    for (step_idx, step) in scenario.steps.iter().enumerate() {
        let argv = substitute_argv(&step.argv, &ids);

        let output = jit_cmd(repo.path())
            .args(&argv)
            .output()
            .unwrap_or_else(|e| panic!("scenario {name} step {step_idx}: failed to spawn: {e}"));

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}{stderr}");
        let exit_code = output.status.code().unwrap_or(-1);

        // Capture UUID if requested.
        if step.capture == CaptureMode::Id {
            if let Some(slot) = &step.id_slot {
                let uuid = extract_uuid(&combined).unwrap_or_else(|| {
                    panic!(
                        "scenario {name} step {step_idx}: capture=id but no UUID found in output.\n\
                         argv: {argv:?}\n\
                         stdout: {stdout}\n\
                         stderr: {stderr}"
                    )
                });
                ids.insert(slot.clone(), uuid);
            }
        }

        // Apply per-step assertions.
        //
        // If no `expect` block is present the step must still succeed (exit 0).
        // Unexpected non-zero exits are swallowed otherwise, silently hiding
        // intermediate failures.
        let default_exit_zero = step.expect.is_none();
        if default_exit_zero {
            assert_eq!(
                exit_code, 0,
                "scenario {name} step {step_idx}: expected exit 0 (implicit default) got {exit_code}\n\
                 argv: {argv:?}\n\
                 stdout: {stdout}\n\
                 stderr: {stderr}"
            );
        }

        if let Some(expect) = &step.expect {
            // Exit code.
            if let Some(want_exit) = expect.exit {
                assert_eq!(
                    exit_code, want_exit,
                    "scenario {name} step {step_idx}: expected exit {want_exit} got {exit_code}\n\
                     argv: {argv:?}\n\
                     stdout: {stdout}\n\
                     stderr: {stderr}"
                );
            }

            // Must-contain substrings.
            for needle in &expect.contains {
                assert!(
                    combined.contains(needle.as_str()),
                    "scenario {name} step {step_idx}: output does not contain {needle:?}\n\
                     argv: {argv:?}\n\
                     combined output:\n{combined}"
                );
            }

            // Must-not-contain substrings.
            for needle in &expect.not_contains {
                assert!(
                    !combined.contains(needle.as_str()),
                    "scenario {name} step {step_idx}: output must NOT contain {needle:?}\n\
                     argv: {argv:?}\n\
                     combined output:\n{combined}"
                );
            }

            // Enforcement point: validate that it matches the step type.
            if let Some(want_ep) = expect.enforcement_point {
                let actual_ep = step_enforcement_point(&argv).unwrap_or_else(|| {
                    panic!(
                        "scenario {name} step {step_idx}: enforcement_point={want_ep:?} specified \
                         but step type cannot be derived from argv: {argv:?}"
                    )
                });
                assert_eq!(
                    actual_ep, want_ep,
                    "scenario {name} step {step_idx}: enforcement_point mismatch.\n\
                     expected {want_ep:?}, derived {actual_ep:?} from argv: {argv:?}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Test entry points — one per scenario (enumerated from fixtures dir)
// ---------------------------------------------------------------------------

/// Load all scenario fixture directories and run each one.
///
/// Rather than hard-coding test functions per scenario (which would require
/// Rust changes when adding scenarios), we drive them all from a single test
/// function. Each scenario's name appears in the panic message on failure.
#[test]
fn test_all_steering_scenarios() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/steering");

    assert!(
        fixtures_dir.exists(),
        "steering fixtures directory missing: {}",
        fixtures_dir.display()
    );

    let mut scenarios: Vec<(String, PathBuf)> = Vec::new();
    for entry in fs::read_dir(&fixtures_dir).expect("read fixtures/steering") {
        let entry = entry.expect("read dir entry");
        let path = entry.path();
        // Non-directory entries (README.md, etc.) are allowed without a scenario.toml.
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .expect("dir entry has file name")
            .to_string_lossy()
            .into_owned();
        let toml_path = path.join("scenario.toml");
        assert!(
            toml_path.exists(),
            "steering fixture directory '{name}' has no scenario.toml — \
             either add scenario.toml or remove the directory \
             ({})",
            toml_path.display()
        );
        scenarios.push((name, path));
    }

    // Sort for deterministic ordering.
    scenarios.sort_by(|(a, _), (b, _)| a.cmp(b));

    assert!(
        !scenarios.is_empty(),
        "no scenario.toml files found under {}",
        fixtures_dir.display()
    );

    for (name, path) in &scenarios {
        run_scenario(name, path);
    }
}

// ---------------------------------------------------------------------------
// Unit tests for the harness helpers
// ---------------------------------------------------------------------------

#[test]
fn test_extract_uuid_finds_uuid() {
    let s = "Created issue: 4793351c-0148-4994-832f-96052ffcf8cc\n";
    assert_eq!(
        extract_uuid(s),
        Some("4793351c-0148-4994-832f-96052ffcf8cc".to_string())
    );
}

#[test]
fn test_extract_uuid_none_for_plain_text() {
    assert_eq!(extract_uuid("no uuid here"), None);
}

#[test]
fn test_substitute_argv_replaces_slot_and_short() {
    let mut ids = HashMap::new();
    ids.insert(
        "epic".to_string(),
        "abcdef01-0000-0000-0000-000000000000".to_string(),
    );

    let argv = vec![
        "issue".to_string(),
        "update".to_string(),
        "$epic".to_string(),
        "--state".to_string(),
        "done".to_string(),
    ];
    let result = substitute_argv(&argv, &ids);
    assert_eq!(result[2], "abcdef01-0000-0000-0000-000000000000");

    let argv2 = vec![
        "dep".to_string(),
        "add".to_string(),
        "$epic_short".to_string(),
    ];
    let result2 = substitute_argv(&argv2, &ids);
    assert_eq!(result2[2], "abcdef01");
}

#[test]
fn test_step_enforcement_point_validate() {
    let argv: Vec<String> = vec!["validate".to_string()];
    assert_eq!(
        step_enforcement_point(&argv),
        Some(EnforcementPoint::Validate)
    );
}

#[test]
fn test_step_enforcement_point_transition() {
    let argv: Vec<String> = ["issue", "update", "abc", "--state", "done"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        step_enforcement_point(&argv),
        Some(EnforcementPoint::Transition)
    );
}

#[test]
fn test_step_enforcement_point_write_create() {
    let argv: Vec<String> = ["issue", "create", "--title", "T"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(step_enforcement_point(&argv), Some(EnforcementPoint::Write));
}

#[test]
fn test_step_enforcement_point_write_update_no_state() {
    let argv: Vec<String> = ["issue", "update", "abc", "--title", "New"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(step_enforcement_point(&argv), Some(EnforcementPoint::Write));
}

#[test]
fn test_step_enforcement_point_dep_is_none() {
    let argv: Vec<String> = ["dep", "add", "abc", "def"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(step_enforcement_point(&argv), None);
}

#[test]
fn test_substitute_argv_prefix_collision() {
    // Slot "epic2" must not be partially consumed by the shorter slot "epic".
    // Without longest-first ordering, "$epic2" would become "<epic-uuid>2".
    let mut ids = HashMap::new();
    ids.insert(
        "epic".to_string(),
        "aaaaaaaa-0000-0000-0000-000000000000".to_string(),
    );
    ids.insert(
        "epic2".to_string(),
        "bbbbbbbb-0000-0000-0000-000000000000".to_string(),
    );

    let argv = vec!["$epic".to_string(), "$epic2".to_string()];
    let result = substitute_argv(&argv, &ids);
    assert_eq!(result[0], "aaaaaaaa-0000-0000-0000-000000000000");
    assert_eq!(result[1], "bbbbbbbb-0000-0000-0000-000000000000");
}

#[test]
fn test_jit_cmd_strips_jit_data_dir() {
    // Verify jit_cmd explicitly removes the isolation-breaking variables and
    // pins JIT_AGENT_ID, by inspecting the Command's configured env overrides
    // directly (Command::get_envs reports removals as (key, None)). No
    // process-global env mutation is needed, so this is safe under parallel
    // test execution.
    let tmp = TempDir::new().expect("tempdir for env test");
    let cmd = jit_cmd(tmp.path());

    let envs: std::collections::HashMap<_, _> = cmd
        .get_envs()
        .map(|(k, v)| (k.to_os_string(), v.map(|v| v.to_os_string())))
        .collect();

    for removed in ["JIT_DATA_DIR", "JIT_ALLOW_DELETION", "JIT_WORKTREE_MODE"] {
        assert_eq!(
            envs.get(std::ffi::OsStr::new(removed)),
            Some(&None),
            "jit_cmd must remove {removed} from the child environment"
        );
    }
    assert_eq!(
        envs.get(std::ffi::OsStr::new("JIT_AGENT_ID")),
        Some(&Some("agent:steering-harness".into())),
        "jit_cmd must pin JIT_AGENT_ID for determinism"
    );
}
