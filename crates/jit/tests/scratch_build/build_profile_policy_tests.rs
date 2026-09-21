//! Regression coverage for jit:57d0eb79's build-profile policy.
//!
//! REQ-01/02: the workspace manifest bounds `[profile.dev]`/`[profile.test]`
//! debug info to `line-tables-only` and states the incremental-compilation
//! intent explicitly, rather than relying on Cargo's undocumented default, so
//! builds keep line-number backtraces without embedding a full debugger payload
//! in every test executable. That manifest setting governs gate runs too: the
//! gate's separate ban on incremental compilation is retired, because a gate
//! run against a warm target directory does have a later rebuild to amortize
//! against — every agent worktree run and every local re-check is one.
//!
//! jit:6d10e5d4 added two more policies this file covers. First, the
//! dependency-package override `[profile.dev.package."*"].opt-level = 1`:
//! third-party dependency code is compiled once and then executed by every
//! test process, so leaving it unoptimized is repeated cost, and level 1 was
//! the measured selection over level 2
//! (dev/benchmarks/dependency-profile-6d10e5d4/). Second, that issue's REQ-04
//! requires the profiler that measures these very policies to pass its own
//! `--self-test` mode; this file re-invokes
//! `scripts/profile-test-suite.sh --self-test` so that evidence is
//! re-established on every suite run rather than attested once.
//!
//! jit:94d85bf1 added the suite-duration policy: the gate prepares the measured
//! suite in its own reported step before the named suite clock starts, then
//! hands the measurement to `scripts/rust-build-budget.sh` as
//! `--test-suite-ms`, so the budget declared there is enforced live over an
//! already-built warm target instead of silently skipped. jit:0708d692
//! completed what that step prepares: compiling the suite outside the clock
//! left the `test` substep paying first-touch I/O on executables linked seconds
//! earlier inside it, so the same step now also reads them into the page cache.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above the jit crate manifest")
        .to_path_buf()
}

fn workspace_manifest() -> toml::Value {
    let text =
        fs::read_to_string(workspace_root().join("Cargo.toml")).expect("read workspace Cargo.toml");
    toml::from_str(&text).expect("workspace Cargo.toml must be valid TOML")
}

fn profile_table(manifest: &toml::Value, profile: &str) -> toml::Value {
    manifest
        .get("profile")
        .and_then(|p| p.get(profile))
        .unwrap_or_else(|| panic!("workspace Cargo.toml has no [profile.{profile}] table"))
        .clone()
}

#[test]
fn test_dev_and_test_profiles_bound_debug_info_to_line_tables_only() {
    let manifest = workspace_manifest();

    for profile in ["dev", "test"] {
        let table = profile_table(&manifest, profile);
        assert_eq!(
            table.get("debug").and_then(|v| v.as_str()),
            Some("line-tables-only"),
            "[profile.{profile}].debug must stay \"line-tables-only\": full \
             debug info bloats every test executable with a debugger payload \
             most gate runs never use"
        );
    }
}

#[test]
fn test_dev_and_test_profiles_state_incremental_intent_explicitly() {
    let manifest = workspace_manifest();

    for profile in ["dev", "test"] {
        let table = profile_table(&manifest, profile);
        assert_eq!(
            table.get("incremental").and_then(|v| v.as_bool()),
            Some(true),
            "[profile.{profile}].incremental must explicitly state the \
             interactive default (true) rather than leaving Cargo's default \
             undocumented"
        );
    }
}

/// The body of the shell function `name` defines in `script`, from its opening
/// header line to the first line closing it at column zero. Lets a test read
/// what a `run_step` line delegates to, rather than only the invocation.
fn shell_function_body<'a>(script: &'a str, name: &str) -> &'a str {
    let header = format!("\n{name}() {{\n");
    let start = script
        .find(&header)
        .unwrap_or_else(|| panic!("scripts/cargo-ci.sh must define a `{name}` function"))
        + header.len();
    let end = script[start..]
        .find("\n}\n")
        .unwrap_or_else(|| panic!("`{name}` must be closed at column zero"));
    &script[start..start + end]
}

/// The shell variable `scripts/cargo-ci.sh` measures the suite clock into. The
/// two tests below tie "the value the gate measures" to "the value the gate
/// enforces" through this one name rather than restating either site.
const SUITE_CLOCK_VARIABLE: &str = "suite_clock_ms";

#[test]
fn test_cargo_ci_compiles_and_warms_the_measured_suite_before_starting_the_suite_clock() {
    let script = fs::read_to_string(workspace_root().join("scripts/cargo-ci.sh"))
        .expect("read scripts/cargo-ci.sh");

    let clock_start = script
        .find("suite_clock_started_ms=")
        .expect("scripts/cargo-ci.sh must start the named suite clock");
    let nextest_step = script
        .find("run_step test ")
        .expect("scripts/cargo-ci.sh must run the test step");
    assert!(
        clock_start < nextest_step,
        "the suite clock must start before the first substep it measures"
    );

    let before_clock = &script[..clock_start];
    let prepare_step = before_clock
        .rfind("run_step ")
        .and_then(|start| before_clock[start..].lines().next())
        .expect("scripts/cargo-ci.sh must run its gate steps through run_step");
    let prepare_impl = prepare_step
        .split_whitespace()
        .nth(2)
        .map(|name| shell_function_body(&script, name))
        .unwrap_or_else(|| {
            panic!(
                "the reported step immediately preceding the suite clock must \
                 delegate to a named function whose body can be read: \
                 {prepare_step}"
            )
        });

    assert!(
        prepare_impl.contains("--no-run"),
        "the reported step immediately preceding the suite clock must compile \
         the measured suite without running it. The enforced budget is defined \
         over an already-built warm target, so compilation paid inside the \
         clock would fail the budget on every cold target — every fresh clone \
         and every CI runner — with nothing wrong in the tree. Found: \
         {prepare_step}"
    );
    assert!(
        prepare_impl.contains(".executable") && prepare_impl.contains(">/dev/null"),
        "the same step must also read the executables it just linked into the \
         page cache. Compiling outside the clock is not enough on its own: the \
         `test` substep pays first-touch I/O on those executables inside the \
         clock, which made one unchanged commit fail the suite budget cold and \
         pass it warm. Found: {prepare_impl}"
    );
}

#[test]
fn test_cargo_ci_enforces_the_measured_suite_clock_through_the_budget_checker() {
    let script = fs::read_to_string(workspace_root().join("scripts/cargo-ci.sh"))
        .expect("read scripts/cargo-ci.sh");

    let measurement = script
        .find(&format!("{SUITE_CLOCK_VARIABLE}=$(("))
        .expect("scripts/cargo-ci.sh must compute the suite-clock duration");
    let budget_step = script
        .find("run_step budget")
        .expect("scripts/cargo-ci.sh must run the budget step");
    assert!(
        measurement < budget_step,
        "the suite clock must be measured before the budget step that consumes it"
    );

    // Read the checker's arguments as one logical command: a shell continuation
    // splits the invocation across physical lines without changing it.
    let joined = script.replace("\\\n", " ");
    let checker_invocation = joined
        .lines()
        .find(|line| line.contains("rust-build-budget.sh") && !line.trim_start().starts_with('#'))
        .expect("scripts/cargo-ci.sh must invoke the build-budget checker");
    let enforced_value = checker_invocation
        .split_whitespace()
        .skip_while(|token| *token != "--test-suite-ms")
        .nth(1)
        .unwrap_or_else(|| {
            panic!(
                "the budget step must pass the measured duration as \
                 --test-suite-ms; without that argument the checker skips its \
                 suite-duration check entirely and the budget is never \
                 enforced: {checker_invocation}"
            )
        });

    assert!(
        enforced_value.contains('$') && enforced_value.contains(SUITE_CLOCK_VARIABLE),
        "--test-suite-ms must carry the live value the gate just measured into \
         `{SUITE_CLOCK_VARIABLE}`, not a literal or an unrelated value: \
         {checker_invocation}"
    );
}

/// jit:0708d692: nothing the measured suite runs may compile, because the suite
/// clock the `budget` step judges is defined over an already-built target.
/// cargo-nextest runs its setup scripts inside that clock, and Cargo unifies
/// features over the packages one invocation selects — so a setup script
/// selecting a single package resolves a second variant of the whole dependency
/// graph and builds it mid-suite. Measured on a target freshly built by
/// `cargo test --workspace --no-run`, the one setup script that invokes Cargo
/// compiled 60 crates in 24,677 ms under `-p jit --test cli_issue` and was
/// fresh in 131 ms under `--workspace --test cli_issue`.
#[test]
fn test_nextest_setup_scripts_reuse_the_workspace_build_resolution() {
    const BUILDING_SUBCOMMANDS: [&str; 5] = ["test", "build", "check", "run", "nextest"];

    let root = workspace_root();
    let config: toml::Value = fs::read_to_string(root.join(".config/nextest.toml"))
        .expect("read .config/nextest.toml")
        .parse()
        .expect("nextest configuration must be valid TOML");
    let declared = config
        .get("scripts")
        .and_then(|scripts| scripts.get("setup"))
        .and_then(toml::Value::as_table)
        .expect(".config/nextest.toml must declare its setup scripts under [scripts.setup]");

    let repository_scripts = declared.iter().filter_map(|(name, declaration)| {
        let command = declaration.get("command")?;
        // Only a script stored in the repository can be read here; a
        // target-relative command names a built binary, not a source file.
        if command.get("relative-to").and_then(toml::Value::as_str) != Some("workspace-root") {
            return None;
        }
        Some((name, command.get("command-line")?.as_str()?.to_string()))
    });

    let mut checked = 0usize;
    for (name, command_line) in repository_scripts {
        let path = root.join(command_line.split_whitespace().next().expect("script path"));
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read setup script {}: {error}", path.display()));

        for line in text.lines() {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            let Some(cargo) = tokens.iter().position(|token| token.contains("cargo")) else {
                continue;
            };
            if !tokens[cargo + 1..]
                .iter()
                .any(|token| BUILDING_SUBCOMMANDS.contains(token))
            {
                continue;
            }
            checked += 1;
            assert!(
                line.contains("--workspace"),
                "setup script `{name}` invokes Cargo with a package selection \
                 that is not the workspace, so its unit graph is resolved \
                 independently of the build the gate already paid for and it \
                 compiles inside the measured suite clock: {line}"
            );
        }
    }

    assert!(
        checked > 0,
        "no Cargo invocation was found in the repository setup scripts \
         `.config/nextest.toml` declares, so this policy checked nothing"
    );
}

#[test]
fn test_cargo_build_scripts_share_the_host_lock_and_enable_available_sccache() {
    let root = workspace_root();
    let gate =
        fs::read_to_string(root.join("scripts/cargo-ci.sh")).expect("read scripts/cargo-ci.sh");

    assert!(
        gate.contains("${XDG_RUNTIME_DIR:-/tmp}/cargo-ci.lock"),
        "the gate must default to the cross-repository host build lock"
    );
    assert!(
        gate.contains("CARGO_CI_NO_SCCACHE")
            && gate.contains("[ -z \"${RUSTC_WRAPPER:-}\" ]")
            && gate.contains("command -v sccache")
            && gate.contains("export RUSTC_WRAPPER=sccache"),
        "the gate must use host sccache when available while preserving an explicit wrapper and an opt-out"
    );
    assert!(
        gate.contains("[ \"${1:-}\" = \"--cargo\" ]")
            && gate.contains("exec cargo \"$@\""),
        "focused Cargo checks must be able to reuse the gate's host lock and cache setup without running the full gate"
    );

    for script in [
        "scripts/benchmark-rust-build.sh",
        "scripts/benchmark-session-cost.sh",
    ] {
        let contents = fs::read_to_string(root.join(script))
            .unwrap_or_else(|error| panic!("read {script}: {error}"));
        assert!(
            contents.contains("${XDG_RUNTIME_DIR:-/tmp}/cargo-ci.lock"),
            "{script} must serialize against the same cross-repository host build lock as cargo-ci.sh"
        );
    }
}

#[test]
fn test_dev_profile_package_override_sets_dependency_opt_level_to_one() {
    let manifest = workspace_manifest();
    let dev = profile_table(&manifest, "dev");
    let opt_level = dev
        .get("package")
        .and_then(|package| package.get("*"))
        .and_then(|wildcard| wildcard.get("opt-level"))
        .and_then(toml::Value::as_integer);

    assert_eq!(
        opt_level,
        Some(1),
        "[profile.dev.package.\"*\"].opt-level must stay 1: dependencies are \
         compiled once and then executed by every test process, so their \
         unoptimized code is repeated cost, and level 1 was the measured \
         selection over level 2 (dev/benchmarks/dependency-profile-6d10e5d4/)"
    );
}

#[test]
fn test_profile_test_suite_script_passes_its_own_self_test() {
    let workspace = workspace_root();
    let script = workspace.join("scripts/profile-test-suite.sh");
    let self_test = Command::new(&script)
        .arg("--self-test")
        .current_dir(&workspace)
        .output()
        .expect("run scripts/profile-test-suite.sh --self-test");

    assert!(
        self_test.status.success()
            && String::from_utf8_lossy(&self_test.stdout)
                .contains("profile-test-suite: self-test passed"),
        "profiler self-test must pass: stdout={} stderr={}",
        String::from_utf8_lossy(&self_test.stdout),
        String::from_utf8_lossy(&self_test.stderr)
    );
}

// MSRV drift guard. The supported Rust version is declared once, as
// `rust-version` under `[workspace.package]` in the workspace `Cargo.toml`, and
// every member inherits it via `rust-version.workspace = true`. CI's `msrv` job
// compiles the workspace on exactly that toolchain, but that only catches a
// member that still inherits; this test catches a member that silently *stops*
// inheriting (dropping out of the enforced policy). It asserts structure, not a
// specific number, so it never becomes a second hand-maintained copy of the
// version literal.
#[test]
fn test_workspace_declares_and_inherits_rust_version() {
    let workspace = workspace_root();

    let root_manifest = fs::read_to_string(workspace.join("Cargo.toml")).unwrap();
    let declared = root_manifest
        .lines()
        .find_map(|line| {
            let line = line.trim();
            line.strip_prefix("rust-version")?
                .trim_start()
                .strip_prefix('=')
                .map(|rest| rest.trim().trim_matches('"').to_string())
        })
        .expect("workspace Cargo.toml must declare rust-version under [workspace.package]");

    // A concrete "major.minor" MSRV, not a moving channel like "stable".
    let mut parts = declared.split('.');
    let major: u32 = parts.next().unwrap().parse().unwrap_or_else(|_| {
        panic!("rust-version must be a numeric version, got {declared:?}");
    });
    let minor: u32 = parts
        .next()
        .expect("rust-version must include a minor component")
        .parse()
        .unwrap_or_else(|_| panic!("rust-version must be numeric, got {declared:?}"));
    assert!(
        major >= 1 && (major > 1 || minor > 0),
        "rust-version {declared:?} is implausibly low"
    );

    // Every workspace member must inherit the single declared value rather than
    // pin its own, so the MSRV job enforces one policy for the whole workspace.
    for member in ["crates/jit", "crates/server"] {
        let manifest = fs::read_to_string(workspace.join(member).join("Cargo.toml")).unwrap();
        assert!(
            manifest.contains("rust-version.workspace = true")
                || manifest.contains("rust-version = { workspace = true }"),
            "{member}/Cargo.toml must inherit rust-version from the workspace"
        );
    }
}
