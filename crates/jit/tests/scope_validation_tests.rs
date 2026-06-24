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

/// A `templates.toml` declaring the `plan` bracket with an INLINE plan: the
/// breakdown node carries `type:breakdown` (the scope-walk boundary) and the
/// planning node declares NO `doc`, so plan-content resolution skips epics (the
/// engine reads each issue's body). The scope walk's boundary type and the
/// breakable types are now TEMPLATE-driven (read off the registry, NOT baked into
/// the engine and NOT read from any flat planning-config block), so a test that
/// exercises the breakdown boundary must declare the template here. The omitted
/// `[type_hierarchy]` means template node/applies_to types are not checked at
/// config load.
const PLAN_TEMPLATE_INLINE: &str = r#"
[[template]]
name = "plan"
applies_to = ["epic"]
[[template.nodes]]
role = "planning"
type = "planning"
[[template.nodes]]
role = "breakdown"
type = "breakdown"
depends_on = ["planning"]
"#;

/// Build an executor whose `.jit/rules.toml` holds exactly `rules_toml` and whose
/// `.jit/templates.toml` declares an inline `plan` bracket, so the `--scope` walk
/// halts at `type:breakdown` nodes. NO flat planning-config block is written: the
/// boundary and breakable types come purely from the template registry.
fn executor_with_rules(rules_toml: &str) -> CommandExecutor<InMemoryStorage> {
    executor_with_rules_and_templates(rules_toml, PLAN_TEMPLATE_INLINE)
}

/// Build an executor with explicit `rules.toml` and `templates.toml` contents and
/// NO `config.toml` (so the boundary/breakable-types/doc-location are derived
/// entirely from the template registry).
fn executor_with_rules_and_templates(
    rules_toml: &str,
    templates_toml: &str,
) -> CommandExecutor<InMemoryStorage> {
    std::env::set_var("JIT_TEST_MODE", "1");
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    std::fs::create_dir_all(storage.root()).unwrap();
    std::fs::write(storage.root().join("templates.toml"), templates_toml).unwrap();
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
fn test_scope_boundary_is_template_driven_custom_breakdown_type() {
    // The breakdown boundary TYPE is read from the container template's breakdown
    // node, not baked into the engine. Declare a template whose breakdown node has
    // the CUSTOM type `synthesis` (applies_to `goal`) and build
    // C -> impl -> S(synthesis) -> P(planning, violating). The walk must halt at
    // the `type:synthesis` node: a rule keyed on `type:synthesis` fires (S is in
    // scope) while the planning-keyed rule on P never fires (P, upstream of the
    // boundary, is out of scope). This fails against any engine that hardcodes
    // `type:breakdown` as the boundary or reads it from a flat planning-config block.
    let templates = r#"
[[template]]
name = "plan"
applies_to = ["goal"]
[[template.nodes]]
role = "planning"
type = "planning"
[[template.nodes]]
role = "breakdown"
type = "synthesis"
depends_on = ["planning"]
"#;
    let rules = r#"
[[rules]]
name = "synthesis-coverage"
when = { type = "synthesis" }
severity = "error"
enforce = true
assert = { label-coverage = { } }

[[rules]]
name = "planning-coverage"
when = { type = "planning" }
severity = "error"
enforce = true
assert = { label-coverage = { } }
"#;
    let executor = executor_with_rules_and_templates(rules, templates);

    // P (planning) declares an uncovered criterion: it is BEYOND the boundary, so
    // its rule must not fire.
    let p = seed(
        &executor,
        "plan",
        &["type:planning"],
        "## Success Criteria\n\n- [hard] PLAN-01: uncovered\n",
        &[],
    );
    // S (synthesis) is the boundary; it declares an uncovered criterion that MUST
    // surface, proving S is in scope.
    let s = seed(
        &executor,
        "synthesis",
        &["type:synthesis"],
        "## Success Criteria\n\n- [hard] SYN-01: uncovered\n",
        std::slice::from_ref(&p),
    );
    let impl_id = seed(
        &executor,
        "impl",
        &["type:task"],
        "",
        std::slice::from_ref(&s),
    );
    let c = seed(
        &executor,
        "container",
        &["type:goal"],
        "",
        std::slice::from_ref(&impl_id),
    );

    let report = executor.validate_scope(&c).expect("scope validation runs");

    assert!(
        report
            .findings
            .iter()
            .any(|f| f.rule == "synthesis-coverage" && f.message.contains("SYN-01")),
        "the custom-named boundary node is in scope, so its rule fires: {:?}",
        report.findings
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.rule == "planning-coverage"),
        "P is beyond the synthesis boundary; its rule must not fire: {:?}",
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
// External plan-doc resolution: a breakable container whose `## Success
// Criteria` lives ONLY in an external plan file (its `description` has no
// criteria) must validate against the criteria FROM THE FILE. The boundary
// resolves the file and injects it; the pure engine reads the injected content.
// (jit:1536006d)
// ---------------------------------------------------------------------------

/// A `plan` template whose planning node declares an EXTERNAL `doc`: the
/// breakable container's plan/criteria live in `dev/active/{container.id}-plan.md`
/// (the `{container.id}` token mirrors the production template), NOT inline. The
/// breakdown node keeps `type:breakdown` as the scope-walk boundary.
const PLAN_TEMPLATE_EXTERNAL: &str = r#"
[[template]]
name = "plan"
applies_to = ["epic"]
[[template.nodes]]
role = "planning"
type = "planning"
doc = "dev/active/{container.id}-plan.md"
[[template.nodes]]
role = "breakdown"
type = "breakdown"
depends_on = ["planning"]
"#;

/// A `label-coverage` rule keyed on the breakable container itself (`type:epic`).
/// The container declares the criteria; a dependency child satisfies them. This
/// mirrors the SDD ruleset's `sdd-hard-criteria-covered`.
const COVERAGE_ON_EPIC: &str = r#"
[[rules]]
name = "epic-criteria-covered"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { label-coverage = { satisfies-namespace = "satisfies", child-link = "dependencies" } }
"#;

/// Wire `C -> impl`, where C is a breakable container whose `description` carries
/// NO criteria — its `## Success Criteria` section lives ONLY in the external
/// plan file written at `dev/active/{C.id}-plan.md`. `impl_satisfies` decides
/// whether the dependency child claims `satisfies:REQ-77`.
fn container_with_external_plan(
    executor: &CommandExecutor<InMemoryStorage>,
    impl_satisfies: bool,
) -> String {
    let impl_labels: Vec<&str> = if impl_satisfies {
        vec!["type:task", "satisfies:REQ-77"]
    } else {
        vec!["type:task"]
    };
    let impl_id = seed(executor, "impl", &impl_labels, "", &[]);
    // The container's OWN body has no criteria; coverage must come from the file.
    let c = seed(
        executor,
        "container",
        &["type:epic"],
        "no criteria in the body — the plan lives in the external file",
        std::slice::from_ref(&impl_id),
    );

    // Write the external plan doc carrying the criteria, at the {id}-substituted
    // path under the REPO ROOT (the parent of `.jit`, i.e. `storage.root()`'s
    // parent), since `plan_doc_location` templates are repo-root-relative. For
    // `InMemoryStorage` this parent is `/tmp`; the file-backed module below
    // exercises a real `.jit`-rooted layout.
    let repo_root = executor
        .storage()
        .root()
        .parent()
        .expect("storage root has a parent")
        .to_path_buf();
    let plan_dir = repo_root.join("dev/active");
    std::fs::create_dir_all(&plan_dir).unwrap();
    std::fs::write(
        plan_dir.join(format!("{c}-plan.md")),
        "## Success Criteria\n\n- [hard] REQ-77: declared only in the external plan\n",
    )
    .unwrap();
    c
}

#[test]
fn test_scope_reads_criteria_from_external_plan_uncovered_fails() {
    // The criterion exists only in the external plan file; the child does NOT
    // satisfy it -> the engine must read the FILE and report it uncovered. This
    // is the failing-before-the-wiring case: with the resolver unwired the engine
    // reads the (criteria-free) body and the scope is spuriously clean.
    let executor = executor_with_rules_and_templates(COVERAGE_ON_EPIC, PLAN_TEMPLATE_EXTERNAL);
    let c = container_with_external_plan(&executor, false);

    let report = executor.validate_scope(&c).expect("scope validation runs");

    assert!(
        report.has_errors(),
        "an uncovered criterion declared ONLY in the external plan must fail: {:?}",
        report.findings
    );
    assert!(
        report.findings.iter().any(|f| f.message.contains("REQ-77")),
        "the finding names the criterion read from the external file: {:?}",
        report.findings
    );
}

#[test]
fn test_scope_reads_criteria_from_external_plan_covered_clean() {
    // Same external-plan criterion, now satisfied by the child -> clean. Proves
    // the file content (not the empty body) drove coverage.
    let executor = executor_with_rules_and_templates(COVERAGE_ON_EPIC, PLAN_TEMPLATE_EXTERNAL);
    let c = container_with_external_plan(&executor, true);

    let report = executor.validate_scope(&c).expect("scope validation runs");

    assert!(
        !report.has_errors(),
        "a covered external-plan criterion must leave the scope clean: {:?}",
        report.findings
    );
}

#[test]
fn test_scope_missing_external_plan_before_planning_done_is_not_an_error() {
    // The container is breakable + the location is external, but the plan file is
    // absent AND no bracket planning node has completed -> the plan is legitimately
    // not authored yet, so the boundary skips it rather than erroring. This is the
    // freshly-applied-bracket case: validate must stay clean before the plan exists.
    let executor = executor_with_rules_and_templates(COVERAGE_ON_EPIC, PLAN_TEMPLATE_EXTERNAL);
    let impl_id = seed(&executor, "impl", &["type:task"], "", &[]);
    let c = seed(
        &executor,
        "container",
        &["type:epic"],
        "body without criteria",
        std::slice::from_ref(&impl_id),
    );
    // No plan file written, no bracket planning node.
    let report = executor
        .validate_scope(&c)
        .expect("a not-yet-authored plan must not surface as a hard error");
    assert!(
        !report.has_errors(),
        "an un-bracketed container with no plan must validate clean: {:?}",
        report.findings
    );
}

#[test]
fn test_scope_missing_external_plan_after_planning_done_surfaces_error() {
    // Once the bracket's planning node is `done`, the plan MUST exist (downstream
    // coverage reads it). A still-missing external plan then surfaces as a
    // contextual error rather than silently passing.
    let executor = executor_with_rules_and_templates(COVERAGE_ON_EPIC, PLAN_TEMPLATE_EXTERNAL);

    // Planning node P, completed.
    let p = seed(&executor, "planning", &["type:planning"], "", &[]);
    let mut p_issue = executor.storage().load_issue(&p).unwrap();
    p_issue.state = State::Done;
    executor.storage().save_issue(p_issue).unwrap();

    // Container C (external plan location, no file).
    let c = seed(
        &executor,
        "container",
        &["type:epic"],
        "body without criteria",
        &[],
    );
    let c_short = executor.storage().load_issue(&c).unwrap().short_id();

    // Breakdown node B brackets C and depends on the (done) planning node.
    let bracket_label = format!("brackets:{c_short}");
    let b = seed(
        &executor,
        "breakdown",
        &["type:breakdown", &bracket_label],
        "",
        std::slice::from_ref(&p),
    );
    // Wire C -> B so the bracket is in C's validation scope.
    let mut c_issue = executor.storage().load_issue(&c).unwrap();
    c_issue.dependencies = vec![b];
    executor.storage().save_issue(c_issue).unwrap();

    let err = executor.validate_scope(&c).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("plan document") && message.contains(&c[..8]),
        "a plan missing after planning completed must surface as a contextual error: {message}"
    );
}

#[test]
fn test_inline_template_still_validates_from_body() {
    // With a `plan` template whose planning node declares no `doc` (an inline
    // plan), the engine must keep reading the issue body: the empty plan-content
    // map reproduces the legacy behavior.
    let executor = executor_with_rules(COVERAGE_ON_BREAKDOWN);
    let (c, _b) = spine_with_breakdown_criteria(&executor, false);

    let report = executor.validate_scope(&c).expect("scope validation runs");

    assert!(
        report.has_errors() && report.findings.iter().any(|f| f.message.contains("REQ-01")),
        "inline criteria in the body must still validate: {:?}",
        report.findings
    );
}

// ---------------------------------------------------------------------------
// File-backed regression (jit:1536006d): the external plan base dir is the REPO
// ROOT (parent of `.jit`), NOT `storage.root()` (the `.jit` dir). The
// InMemoryStorage tests above write the plan under `storage.root()/dev/active`
// (= `.jit/dev/active`), which MASKS the `.jit`-vs-repo-root distinction. These
// tests use a real `JsonFileStorage` rooted at `<tmp>/.jit` with the plan file
// at the REPO-ROOT-relative `<tmp>/dev/active/{id}-plan.md` (NOT under `.jit/`),
// so they FAIL while the resolver passes `storage.root()` and PASS after the
// base dir is corrected to the repo root.
// ---------------------------------------------------------------------------

mod file_backed_external_plan {
    use super::{COVERAGE_ON_EPIC, PLAN_TEMPLATE_EXTERNAL};
    use jit::commands::CommandExecutor;
    use jit::domain::{Issue, State};
    use jit::storage::{IssueStore, JsonFileStorage};
    use tempfile::TempDir;

    /// Build a `JsonFileStorage`-backed executor rooted at `<tmp>/.jit`, with the
    /// rule set and the external-`doc` `plan` template written into `.jit/` (NO
    /// flat planning-config block: the doc location comes from the template registry).
    /// Returns the temp dir (kept alive = the repo root) and the executor.
    fn executor() -> (TempDir, CommandExecutor<JsonFileStorage>) {
        std::env::set_var("JIT_TEST_MODE", "1");
        let repo_root = TempDir::new().unwrap();
        let jit_dir = repo_root.path().join(".jit");
        let storage = JsonFileStorage::new(&jit_dir);
        storage.init().unwrap();
        std::fs::write(jit_dir.join("templates.toml"), PLAN_TEMPLATE_EXTERNAL).unwrap();
        std::fs::write(jit_dir.join("rules.toml"), COVERAGE_ON_EPIC).unwrap();
        (repo_root, CommandExecutor::new(storage))
    }

    fn seed(
        executor: &CommandExecutor<JsonFileStorage>,
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

    /// Wire `C -> impl`, with C a breakable container whose criteria live ONLY in
    /// the external plan file written at the REPO-ROOT-relative path
    /// `<repo_root>/dev/active/{C.id}-plan.md` (NOT under `.jit/`). The old buggy
    /// base dir (`storage.root()` = `.jit`) would look under `.jit/dev/active/`
    /// and never find this file.
    fn container_with_repo_root_plan(
        repo_root: &TempDir,
        executor: &CommandExecutor<JsonFileStorage>,
        impl_satisfies: bool,
    ) -> String {
        let impl_labels: Vec<&str> = if impl_satisfies {
            vec!["type:task", "satisfies:REQ-77"]
        } else {
            vec!["type:task"]
        };
        let impl_id = seed(executor, "impl", &impl_labels, "", &[]);
        let c = seed(
            executor,
            "container",
            &["type:epic"],
            "no criteria in the body — the plan lives in the external file",
            std::slice::from_ref(&impl_id),
        );

        // Write the plan under the REPO ROOT, NOT under `.jit/`.
        let plan_dir = repo_root.path().join("dev/active");
        std::fs::create_dir_all(&plan_dir).unwrap();
        std::fs::write(
            plan_dir.join(format!("{c}-plan.md")),
            "## Success Criteria\n\n- [hard] REQ-77: declared only in the external plan\n",
        )
        .unwrap();
        c
    }

    #[test]
    fn test_file_backed_external_plan_at_repo_root_uncovered_fails() {
        // The criterion lives only in `<repo_root>/dev/active/{id}-plan.md`; the
        // child does NOT satisfy it. With the corrected repo-root base dir the
        // engine reads the file and reports it uncovered. With the old
        // `storage.root()` base dir the file under `.jit/dev/active` is absent, so
        // resolution errored (or the criterion was never seen) — this test FAILS
        // before the fix.
        let (repo_root, executor) = executor();
        let c = container_with_repo_root_plan(&repo_root, &executor, false);

        let report = executor.validate_scope(&c).expect("scope validation runs");

        assert!(
            report.has_errors(),
            "an uncovered criterion in the repo-root plan file must fail: {:?}",
            report.findings
        );
        assert!(
            report.findings.iter().any(|f| f.message.contains("REQ-77")),
            "the finding names the criterion read from the repo-root file: {:?}",
            report.findings
        );
    }

    #[test]
    fn test_file_backed_external_plan_at_repo_root_covered_clean() {
        // Same repo-root plan criterion, now satisfied by the child -> clean,
        // proving the file content (not the empty body) drove coverage AND that
        // the file was found at the repo root rather than under `.jit/`.
        let (repo_root, executor) = executor();
        let c = container_with_repo_root_plan(&repo_root, &executor, true);

        let report = executor.validate_scope(&c).expect("scope validation runs");

        assert!(
            !report.has_errors(),
            "a covered repo-root-plan criterion must leave the scope clean: {:?}",
            report.findings
        );
    }
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
