//! `jit validate --scope <id>` — scoped bracket-subtree validation (T2, D14, R2).
//!
//! `--scope` evaluates the declarative rules over a container's bracket subtree:
//! the container's transitive dependency closure INCLUDING the `type:breakdown`
//! node `B` (bounded so the walk stops at `B` and never pulls in `P`/upstream).
//! For each in-slice issue the rules whose `when` selector matches it fire — so a
//! rule keyed on `type:breakdown` runs because `B` is in scope. Whole-repo rule
//! kinds (`label-uniqueness`, repo-wide `label-reference`, `type-hierarchy`) are
//! EXCLUDED exactly as at transition time (R2 / CC-2a). The command exits 4 with
//! findings shown when an error-severity finding is produced, 0 when clean.
//!
//! These are in-process tests over `CommandExecutor::validate_scope` (fast and
//! deterministic); the exit-4 CLI contract is covered by a subprocess test.

use jit::commands::CommandExecutor;
use jit::domain::{Issue, State};
use jit::storage::{InMemoryStorage, IssueStore};

/// Build an executor whose `.jit/rules.toml` holds exactly `rules_toml`.
fn executor_with_rules(rules_toml: &str) -> CommandExecutor<InMemoryStorage> {
    std::env::set_var("JIT_TEST_MODE", "1");
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    std::fs::create_dir_all(storage.root()).unwrap();
    std::fs::write(storage.root().join("config.toml"), "").unwrap();
    std::fs::write(storage.root().join("rules.toml"), rules_toml).unwrap();
    CommandExecutor::new(storage)
}

/// Save an issue directly into storage, returning its id.
fn seed(
    executor: &CommandExecutor<InMemoryStorage>,
    title: &str,
    labels: &[&str],
    description: &str,
    deps: &[String],
) -> String {
    let mut issue = Issue::new(title.to_string(), description.to_string());
    issue.labels = labels.iter().map(|s| s.to_string()).collect();
    issue.dependencies = deps.to_vec();
    issue.state = State::Backlog;
    let id = issue.id.clone();
    executor.storage().save_issue(issue).unwrap();
    id
}

/// A `label-coverage` rule keyed on the breakdown node `B`. It checks that the
/// criteria declared on the issue it matches are covered by dependent children
/// (any state — `child-state` omitted). Error severity, enforcing.
const COVERAGE_ON_BREAKDOWN: &str = r#"
[[rules]]
name = "breakdown-coverage-preview"
when = { type = "breakdown" }
severity = "error"
enforce = true
assert = { label-coverage = { } }
"#;

/// Wire a bracket spine `C -> impl -> B`, where `B` carries the success criteria
/// and a dependent impl child may or may not satisfy them. Returns (container,
/// breakdown) ids. `impl_satisfies` decides whether the impl child carries the
/// `satisfies:REQ-01` label.
fn spine_with_breakdown_criteria(
    executor: &CommandExecutor<InMemoryStorage>,
    impl_satisfies: bool,
) -> (String, String) {
    // B declares the criterion; impl depends on B; C depends on impl.
    let b = seed(
        executor,
        "breakdown",
        &["type:breakdown"],
        "## Success Criteria\n\n- [hard] REQ-01: do the thing\n",
        &[],
    );
    let impl_labels: Vec<&str> = if impl_satisfies {
        vec!["type:task", "satisfies:REQ-01"]
    } else {
        vec!["type:task"]
    };
    let impl_id = seed(executor, "impl", &impl_labels, "", std::slice::from_ref(&b));
    let c = seed(
        executor,
        "container",
        &["type:epic"],
        "",
        std::slice::from_ref(&impl_id),
    );
    (c, b)
}

#[test]
fn test_scope_fires_breakdown_rule_and_fails_on_uncovered_criterion() {
    let executor = executor_with_rules(COVERAGE_ON_BREAKDOWN);
    let (c, _b) = spine_with_breakdown_criteria(&executor, false);

    let report = executor.validate_scope(&c).expect("scope validation runs");

    assert!(
        report.has_errors(),
        "an uncovered [hard] criterion on B must produce an error finding: {:?}",
        report.findings
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.rule == "breakdown-coverage-preview"),
        "the breakdown-keyed rule fired because B is in scope: {:?}",
        report.findings
    );
    assert!(
        report.findings.iter().any(|f| f.message.contains("REQ-01")),
        "the finding names the uncovered criterion: {:?}",
        report.findings
    );
}

#[test]
fn test_scope_passes_when_criterion_is_covered() {
    let executor = executor_with_rules(COVERAGE_ON_BREAKDOWN);
    let (c, _b) = spine_with_breakdown_criteria(&executor, true);

    let report = executor.validate_scope(&c).expect("scope validation runs");

    assert!(
        !report.has_errors(),
        "a covered criterion must leave the scope clean: {:?}",
        report.findings
    );
}

#[test]
fn test_scope_excludes_repo_wide_label_uniqueness() {
    // A repo-wide `label-uniqueness` rule (CC-2a / R2): two issues IN the slice
    // share a unique label, which would fail a whole-repo check, but `--scope`
    // must NOT evaluate repo-wide rules, so the scope stays clean.
    let rules = r#"
[[rules]]
name = "unique-key"
when = { type = "task" }
severity = "error"
assert = { label-uniqueness = { namespace = "key", scope = "all" } }
"#;
    let executor = executor_with_rules(rules);

    let b = seed(&executor, "breakdown", &["type:breakdown"], "", &[]);
    let t1 = seed(
        &executor,
        "task1",
        &["type:task", "key:dup"],
        "",
        std::slice::from_ref(&b),
    );
    let t2 = seed(
        &executor,
        "task2",
        &["type:task", "key:dup"],
        "",
        std::slice::from_ref(&t1),
    );
    let c = seed(
        &executor,
        "container",
        &["type:epic"],
        "",
        std::slice::from_ref(&t2),
    );

    let report = executor.validate_scope(&c).expect("scope validation runs");

    assert!(
        !report.has_errors(),
        "repo-wide label-uniqueness must be excluded from --scope: {:?}",
        report.findings
    );
}

#[test]
fn test_scope_does_not_pull_in_plan_or_upstream() {
    // C -> impl -> B -> P, where P carries a violating criterion under a
    // breakdown-keyed-like rule. Because the walk halts at B, P is out of scope
    // and its rules never fire. We model this with a coverage rule keyed on
    // type:planning: it would fire if P were in scope, but it must not.
    let rules = r#"
[[rules]]
name = "planning-coverage"
when = { type = "planning" }
severity = "error"
enforce = true
assert = { label-coverage = { } }
"#;
    let executor = executor_with_rules(rules);

    let p = seed(
        &executor,
        "plan",
        &["type:planning"],
        "## Success Criteria\n\n- [hard] PLAN-01: uncovered\n",
        &[],
    );
    let b = seed(
        &executor,
        "breakdown",
        &["type:breakdown"],
        "",
        std::slice::from_ref(&p),
    );
    let impl_id = seed(
        &executor,
        "impl",
        &["type:task"],
        "",
        std::slice::from_ref(&b),
    );
    let c = seed(
        &executor,
        "container",
        &["type:epic"],
        "",
        std::slice::from_ref(&impl_id),
    );

    let report = executor.validate_scope(&c).expect("scope validation runs");

    assert!(
        !report.has_errors(),
        "P is beyond the breakdown boundary, so its rules must not fire under --scope: {:?}",
        report.findings
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.rule == "planning-coverage"),
        "the planning-keyed rule must not have been evaluated: {:?}",
        report.findings
    );
}

#[test]
fn test_scope_resolves_partial_container_id() {
    let executor = executor_with_rules(COVERAGE_ON_BREAKDOWN);
    let (c, _b) = spine_with_breakdown_criteria(&executor, true);

    // A short prefix of the container id resolves like every other jit command.
    let short = &c[..8];
    let report = executor
        .validate_scope(short)
        .expect("partial container id resolves");
    assert!(!report.has_errors(), "{:?}", report.findings);
}

// ---------------------------------------------------------------------------
// CLI exit-code contract (subprocess): the gate-checker behaviour the
// coverage-preview gate relies on — exit 4 with findings on failure, 0 clean.
// ---------------------------------------------------------------------------

mod cli {
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn jit() -> &'static str {
        env!("CARGO_BIN_EXE_jit")
    }

    fn run(dir: &TempDir, args: &[&str]) -> std::process::Output {
        Command::new(jit())
            .current_dir(dir)
            .args(args)
            .output()
            .expect("jit runs")
    }

    fn create(dir: &TempDir, title: &str, description: &str, labels: &[&str]) -> String {
        let mut args = vec![
            "issue",
            "create",
            "--title",
            title,
            "--description",
            description,
            "--json",
        ];
        for label in labels {
            args.push("--label");
            args.push(label);
        }
        let out = run(dir, &args);
        assert!(
            out.status.success(),
            "create failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        json["id"].as_str().unwrap().to_string()
    }

    /// Coverage rule keyed on the breakdown node; error-severity, enforcing.
    const RULES: &str = r#"
[[rules]]
name = "breakdown-coverage-preview"
when = { type = "breakdown" }
severity = "error"
enforce = true
assert = { label-coverage = { } }
"#;

    /// init + write rules.toml + build a `C -> impl -> B` spine where B carries
    /// the criterion. Returns (TempDir, container-id). The impl child carries
    /// `satisfies:REQ-01` only when `covered`.
    fn setup_spine(covered: bool) -> (TempDir, String) {
        let dir = TempDir::new().unwrap();
        assert!(run(&dir, &["init"]).status.success());
        fs::write(dir.path().join(".jit/rules.toml"), RULES).unwrap();

        let b = create(
            &dir,
            "breakdown",
            "## Success Criteria\n\n- [hard] REQ-01: do the thing\n",
            &["type:breakdown"],
        );
        let impl_labels: &[&str] = if covered {
            &["type:task", "satisfies:REQ-01"]
        } else {
            &["type:task"]
        };
        let impl_id = create(&dir, "impl", "", impl_labels);
        assert!(run(&dir, &["dep", "add", &impl_id, &b]).status.success());

        let c = create(&dir, "container", "", &["type:epic"]);
        assert!(run(&dir, &["dep", "add", &c, &impl_id]).status.success());

        (dir, c)
    }

    #[test]
    fn test_cli_scope_exits_4_with_findings_on_failure() {
        let (dir, c) = setup_spine(false);
        let out = run(&dir, &["validate", "--scope", &c]);
        assert_eq!(
            out.status.code(),
            Some(4),
            "an enforcing-rule failure must exit 4 (ValidationFailed); stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            combined.contains("REQ-01") && combined.contains("breakdown-coverage-preview"),
            "findings must be shown: {combined}"
        );
    }

    #[test]
    fn test_cli_scope_exits_0_when_clean() {
        let (dir, c) = setup_spine(true);
        let out = run(&dir, &["validate", "--scope", &c]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "a clean scope must exit 0; stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn test_cli_scope_rejects_combination_with_id() {
        let dir = TempDir::new().unwrap();
        assert!(run(&dir, &["init"]).status.success());
        let out = run(&dir, &["validate", "someid", "--scope", "other"]);
        assert!(
            !out.status.success(),
            "`--scope` combined with a positional id must be rejected"
        );
    }
}
