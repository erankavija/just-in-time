//! `jit validate --scope <id>` — scoped bracket-subtree validation (T2, D14, R2).
//!
//! `--scope` evaluates the declarative rules over a container's bracket subtree:
//! the container's transitive dependency closure INCLUDING the `type:breakdown`
//! node `B` (bounded so the walk stops at `B` and never pulls in `P`/upstream).
//! For each in-slice issue the rules whose `when` selector matches it fire — so a
//! rule keyed on `type:breakdown` runs because `B` is in scope. Whole-repo rule
//! kinds (`label-uniqueness`, repo-wide `label-reference`, `type-hierarchy`) are
//! EXCLUDED exactly as at transition time (R2 / CC-2a). The command fails with
//! findings shown when an error-severity finding is produced.
//!
//! These are in-process tests over validation capture and scope operations;
//! command failure presentation is covered by subprocess tests.

use jit::commands::test_helpers::{memory_executor_declaring, MemoryDeclarations};
use jit::commands::CommandExecutor;
use jit::domain::identity::{IssueId, IssueIdPrefix};
use jit::domain::{DocumentReference, State};
use jit::storage::{InMemoryStorage, IssueStore, RepositoryStateStore};
use jit::test_utils::{document_reference, issue_id};
use jit::validation::report::Selection;

trait ScopeFixtureStore: IssueStore + RepositoryStateStore {
    fn publish_scope_issue(executor: &CommandExecutor<Self>, issue: jit::domain::Issue) -> IssueId;
}

impl ScopeFixtureStore for InMemoryStorage {
    fn publish_scope_issue(executor: &CommandExecutor<Self>, issue: jit::domain::Issue) -> IssueId {
        let id = issue.id.clone();
        jit::test_harness::seed_memory_issue(executor.storage(), &issue);
        id
    }
}

impl ScopeFixtureStore for jit::storage::JsonFileStorage {
    fn publish_scope_issue(executor: &CommandExecutor<Self>, issue: jit::domain::Issue) -> IssueId {
        let dependencies = issue.dependencies.clone();
        let documents = issue.documents.clone();
        let target_state = issue.state;
        let id = executor
            .issues()
            .create_issue(
                issue.title,
                issue.description,
                issue.priority,
                issue.gates_required,
                issue.labels,
                issue.content_format,
                None,
                false,
            )
            .unwrap()
            .issue
            .id;
        for dependency in dependencies {
            executor.graph().add_dependency(&id, &dependency).unwrap();
        }
        if executor.storage().load_issue(&id).unwrap().state != target_state {
            executor
                .issues()
                .update_issue(
                    &id,
                    None,
                    None,
                    None,
                    Some(target_state),
                    vec![],
                    vec![],
                    None,
                    None,
                    true,
                )
                .unwrap();
        }
        for document in documents {
            executor
                .documents()
                .add_document_reference(
                    &id,
                    &document.path,
                    document.commit.as_deref(),
                    document.label.as_deref(),
                    document.doc_type.as_deref(),
                    false,
                )
                .unwrap();
        }
        id
    }
}

/// Wire a `plan` bracket onto container `container_id`: a planning node `P`
/// (`type:planning`, in `planning_state`) carrying a `plan`-labeled doc reference
/// at `plan_path`, and a breakdown node `B` (`type:breakdown`,
/// `brackets:<container-short-id>`) that depends on `P`. Returns the planning
/// node id.
///
/// This is the bracket shape plan-doc resolution walks: the plan-doc location is
/// read from `P`'s `plan` reference (the validation-time source of truth), NOT
/// the template path. The container itself is not modified.
fn wire_plan_bracket<S: ScopeFixtureStore>(
    executor: &CommandExecutor<S>,
    container_id: &IssueId,
    plan_path: &str,
    planning_state: State,
) -> IssueId {
    let c_short = executor
        .storage()
        .load_issue(container_id)
        .unwrap()
        .short_id();

    let mut p = crate::fixture_issue("planning".to_string(), String::new());
    p.id = derived_id(&["plan", container_id.as_str()]);
    p.labels = vec!["type:planning".to_string()];
    p.state = planning_state;
    p.documents.push(DocumentReference {
        label: Some("plan".to_string()),
        ..document_reference(plan_path)
    });
    let p_id = S::publish_scope_issue(executor, p);

    let mut b = crate::fixture_issue("breakdown".to_string(), String::new());
    b.id = derived_id(&["break", container_id.as_str()]);
    b.labels = vec!["type:breakdown".to_string(), format!("brackets:{c_short}")];
    b.dependencies = vec![p_id.clone()];
    b.state = State::Backlog;
    S::publish_scope_issue(executor, b);

    p_id
}

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
labels = ["brackets:{container.short_id}"]
depends_on = ["planning"]
"#;

/// Seed an exact issue preimage into the in-memory aggregate, returning its id.
/// The deterministic id [`seed`] assigns, so a child can name its container's
/// short id before the container exists.
fn seed_id(title: &str, description: &str) -> IssueId {
    derived_id(&[title, description])
}

/// A canonical issue id determined by `parts`, so equal inputs name one issue.
fn derived_id(parts: &[&str]) -> IssueId {
    let hash = parts
        .iter()
        .flat_map(|part| part.bytes())
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    issue_id(u128::from(hash))
}

/// The `satisfies` label crediting criterion `req` of the issue `seed` creates
/// from `title` and `description`.
fn credit(title: &str, description: &str, req: &str) -> String {
    format!("satisfies:{}/{req}", seed_id(title, description).short_id())
}

fn seed(
    executor: &CommandExecutor<InMemoryStorage>,
    title: &str,
    labels: &[&str],
    description: &str,
    deps: &[IssueId],
) -> IssueId {
    let mut issue = crate::fixture_issue(title.to_string(), description.to_string());
    issue.id = seed_id(title, description);
    issue.labels = labels.iter().map(|s| s.to_string()).collect();
    issue.dependencies = deps.to_vec();
    issue.state = State::Backlog;
    InMemoryStorage::publish_scope_issue(executor, issue)
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
/// breakdown) ids. `impl_satisfies` decides whether the impl child credits B's
/// REQ-01.
fn spine_with_breakdown_criteria(
    executor: &CommandExecutor<InMemoryStorage>,
    impl_satisfies: bool,
) -> (IssueId, IssueId) {
    // B declares the criterion; impl depends on B; C depends on impl.
    let b = seed(
        executor,
        "breakdown",
        &["type:breakdown"],
        "## Success Criteria\n\n- [hard] REQ-01: do the thing\n",
        &[],
    );
    let credit = format!("satisfies:{}/REQ-01", b.short_id());
    let impl_labels: Vec<&str> = if impl_satisfies {
        vec!["type:task", &credit]
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
fn test_rule_selection_reports_rules_file_finding_for_unloadable_rules() {
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some("[[rules]\nname = \"unterminated\"\n"),
            after_initialization: true,
            ..Default::default()
        },
    );
    let issue = seed(&executor, "selected issue", &["type:task"], "", &[]);

    let report = executor
        .validation()
        .selection_report()
        .expect("the captured validation report is available");
    let selected = report
        .select(&Selection::Rule {
            issue: IssueIdPrefix::from(&issue),
            rule: "selected-coverage".to_string(),
        })
        .expect("an unloadable rule set is reported as a finding");

    assert!(selected
        .findings
        .iter()
        .any(|finding| finding.rule == "rules-file"));
}

#[test]
fn test_scope_fires_breakdown_rule_and_fails_on_uncovered_criterion() {
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some(COVERAGE_ON_BREAKDOWN),
            templates: Some(PLAN_TEMPLATE_INLINE),
            ..Default::default()
        },
    );
    let (c, _b) = spine_with_breakdown_criteria(&executor, false);

    let report = executor
        .validation()
        .validate_scope(&c)
        .expect("scope validation runs");

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
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some(COVERAGE_ON_BREAKDOWN),
            templates: Some(PLAN_TEMPLATE_INLINE),
            ..Default::default()
        },
    );
    let (c, _b) = spine_with_breakdown_criteria(&executor, true);

    let report = executor
        .validation()
        .validate_scope(&c)
        .expect("scope validation runs");

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
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some(rules),
            templates: Some(PLAN_TEMPLATE_INLINE),
            ..Default::default()
        },
    );

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

    let report = executor
        .validation()
        .validate_scope(&c)
        .expect("scope validation runs");

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
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some(rules),
            templates: Some(PLAN_TEMPLATE_INLINE),
            ..Default::default()
        },
    );

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

    let report = executor
        .validation()
        .validate_scope(&c)
        .expect("scope validation runs");

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
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some(rules),
            templates: Some(templates),
            ..Default::default()
        },
    );

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

    let report = executor
        .validation()
        .validate_scope(&c)
        .expect("scope validation runs");

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

// ---------------------------------------------------------------------------
// Container-pointer resolution under scope (ef0065ad): a `container-from-label`
// coverage rule (the repo's `coverage-preview` shape) fires on a breakdown node
// and redirects to the bracketed container. Under `--scope`, the pointer must
// resolve against the COMPLETE repository issue set — not the partial slice — so an
// unrelated breakdown that entered the closure without its container is
// recognized and skipped rather than mis-reported as a dangling pointer, while a
// genuinely dangling pointer still fails precisely.
// ---------------------------------------------------------------------------

/// A `label-coverage` rule keyed on the breakdown node that redirects to the
/// bracketed container via `container-from-label = "brackets"` — the shape the
/// repo's `coverage-preview` rule uses. Error severity, enforcing.
const COVERAGE_VIA_BRACKETS: &str = r#"
[[rules]]
name = "brackets-coverage-preview"
when = { type = "breakdown" }
severity = "error"
enforce = true
assert = { label-coverage = { container-from-label = "brackets", child-link = "dependencies", child-type-exclude = ["breakdown", "planning"] } }
"#;

/// Save a breakdown node carrying an explicit `brackets:<pointer>` label in the
/// given state, returning its id. Used to plant a breakdown whose container is
/// out of scope (or absent) without going through the plan-bracket helper.
fn seed_breakdown_bracketing(
    executor: &CommandExecutor<InMemoryStorage>,
    pointer: &str,
    state: State,
    deps: &[IssueId],
) -> IssueId {
    let mut b = crate::fixture_issue("breakdown".to_string(), String::new());
    b.id = derived_id(&["break", pointer]);
    b.labels = vec!["type:breakdown".to_string(), format!("brackets:{pointer}")];
    b.dependencies = deps.to_vec();
    b.state = state;
    InMemoryStorage::publish_scope_issue(executor, b)
}

#[test]
fn test_scope_skips_unrelated_breakdown_whose_container_is_out_of_scope() {
    // REQ-02 (ef0065ad): a completed, UNRELATED breakdown B2 enters the scoped
    // slice through the dependency closure (as a breakdown boundary node), but
    // the container it brackets (C2) is not in the slice. Its `brackets:` pointer
    // resolves against the FULL repository issue set, so the out-of-scope container is
    // recognized and B2's coverage is simply not the requested bracket — no false
    // missing-pointer finding. Before the fix, resolving `brackets:C2` against the
    // partial slice found nothing and raised a spurious `config error`.
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some(COVERAGE_VIA_BRACKETS),
            templates: Some(PLAN_TEMPLATE_INLINE),
            ..Default::default()
        },
    );

    // C2 lives OUTSIDE the scoped container's closure (unreachable from C1).
    let c2 = seed(
        &executor,
        "other-container",
        &["type:epic"],
        "## Success Criteria\n\n- [hard] REQ-99: unrelated\n",
        &[],
    );
    // B2 is completed and brackets C2; it is reachable from C1 via task1, so it
    // enters C1's slice as a breakdown boundary node while C2 does not.
    let b2 = seed_breakdown_bracketing(&executor, c2.short_id(), State::Done, &[]);
    let task1 = seed(
        &executor,
        "task1",
        &["type:task"],
        "",
        std::slice::from_ref(&b2),
    );
    let c1 = seed(
        &executor,
        "scoped-container",
        &["type:epic"],
        "",
        std::slice::from_ref(&task1),
    );

    let report = executor
        .validation()
        .validate_scope(&c1)
        .expect("scope validation runs");

    assert!(
        !report.has_errors(),
        "an out-of-scope bracketed container must not raise a config error: {:?}",
        report.findings
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.message.contains("no known issue")),
        "no false missing-pointer finding for the unrelated breakdown: {:?}",
        report.findings
    );
}

#[test]
fn test_scope_dangling_brackets_pointer_still_fails_precisely() {
    // REQ-03 (ef0065ad): a breakdown IN the requested scope whose `brackets:`
    // pointer names no issue ANYWHERE (not merely outside the slice) is a genuine
    // configuration error and must still fail with the precise finding. Full-index
    // resolution must not mask a truly dangling pointer.
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some(COVERAGE_VIA_BRACKETS),
            templates: Some(PLAN_TEMPLATE_INLINE),
            ..Default::default()
        },
    );
    let b1 = seed_breakdown_bracketing(&executor, "deadbeef", State::Done, &[]);
    let c1 = seed(
        &executor,
        "scoped-container",
        &["type:epic"],
        "",
        std::slice::from_ref(&b1),
    );

    let report = executor
        .validation()
        .validate_scope(&c1)
        .expect("scope validation runs");

    assert!(
        report.has_errors(),
        "a genuinely dangling brackets pointer must fail: {:?}",
        report.findings
    );
    assert!(
        report.findings.iter().any(
            |f| f.message.contains("brackets:deadbeef") && f.message.contains("no known issue")
        ),
        "the finding must precisely name the dangling pointer: {:?}",
        report.findings
    );
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
labels = ["brackets:{container.short_id}"]
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
/// whether the dependency child credits the container's REQ-77.
fn container_with_external_plan(
    executor: &CommandExecutor<InMemoryStorage>,
    impl_satisfies: bool,
) -> IssueId {
    let body = "no criteria in the body — the plan lives in the external file";
    let credit = credit("container", body, "REQ-77");
    let impl_labels: Vec<&str> = if impl_satisfies {
        vec!["type:task", &credit]
    } else {
        vec!["type:task"]
    };
    let impl_id = seed(executor, "impl", &impl_labels, "", &[]);
    // The container's OWN body has no criteria; coverage must come from the file.
    let c = seed(
        executor,
        "container",
        &["type:epic"],
        body,
        std::slice::from_ref(&impl_id),
    );

    // Seed the repo-root-relative plan into the aggregate image captured by the
    // in-memory mutation session. Writing this fixture to the real `/tmp`
    // filesystem would recreate the retired ambient-read path.
    let plan_path = format!("dev/active/{c}-plan.md");
    executor.storage().add_worktree_file(
        &plan_path,
        "## Success Criteria\n\n- [hard] REQ-77: declared only in the external plan\n",
    );
    // Doc-ref-canonical resolution: the plan location comes from the planning
    // node's `plan` reference, so wire a (done) bracket pointing at the file.
    wire_plan_bracket(executor, &c, &plan_path, State::Done);
    c
}

#[test]
fn test_scope_reads_criteria_from_external_plan_uncovered_fails() {
    // The criterion exists only in the external plan file; the child does NOT
    // satisfy it -> the engine must read the FILE and report it uncovered. This
    // is the failing-before-the-wiring case: with the resolver unwired the engine
    // reads the (criteria-free) body and the scope is spuriously clean.
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some(COVERAGE_ON_EPIC),
            templates: Some(PLAN_TEMPLATE_EXTERNAL),
            ..Default::default()
        },
    );
    let c = container_with_external_plan(&executor, false);

    let report = executor
        .validation()
        .validate_scope(&c)
        .expect("scope validation runs");

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
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some(COVERAGE_ON_EPIC),
            templates: Some(PLAN_TEMPLATE_EXTERNAL),
            ..Default::default()
        },
    );
    let c = container_with_external_plan(&executor, true);

    let report = executor
        .validation()
        .validate_scope(&c)
        .expect("scope validation runs");

    assert!(
        !report.has_errors(),
        "a covered external-plan criterion must leave the scope clean: {:?}",
        report.findings
    );
}

#[test]
fn test_scope_missing_external_plan_before_planning_done_is_not_an_error() {
    // The container is breakable + the location is external, but the plan file the
    // planning node's `plan` reference points at is absent AND the bracket's
    // planning node has NOT completed -> the plan is legitimately not authored
    // yet, so the boundary skips it rather than erroring. This is the
    // freshly-applied-bracket case: validate must stay clean before the plan exists.
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some(COVERAGE_ON_EPIC),
            templates: Some(PLAN_TEMPLATE_EXTERNAL),
            ..Default::default()
        },
    );
    let impl_id = seed(&executor, "impl", &["type:task"], "", &[]);
    let c = seed(
        &executor,
        "container",
        &["type:epic"],
        "body without criteria",
        std::slice::from_ref(&impl_id),
    );
    // A backlog planning node references a plan that has not been written yet.
    wire_plan_bracket(
        &executor,
        &c,
        &format!("dev/active/{c}-plan.md"),
        State::Backlog,
    );
    let report = executor
        .validation()
        .validate_scope(&c)
        .expect("a not-yet-authored plan must not surface as a hard error");
    assert!(
        !report.has_errors(),
        "a not-yet-authored plan must validate clean: {:?}",
        report.findings
    );
}

#[test]
fn test_scope_missing_external_plan_after_planning_done_surfaces_error() {
    // Once the bracket's planning node is `done`, the plan it references MUST
    // exist (downstream coverage reads it). A still-missing plan file then
    // surfaces as a contextual error rather than silently passing.
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some(COVERAGE_ON_EPIC),
            templates: Some(PLAN_TEMPLATE_EXTERNAL),
            ..Default::default()
        },
    );

    let c = seed(
        &executor,
        "container",
        &["type:epic"],
        "body without criteria",
        &[],
    );
    // The (done) planning node references a plan file that was never written.
    wire_plan_bracket(
        &executor,
        &c,
        &format!("dev/active/{c}-plan.md"),
        State::Done,
    );

    let err = executor.validation().validate_scope(&c).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("plan document") && message.contains(c.short_id()),
        "a plan missing after planning completed must surface as a contextual error: {message}"
    );
}

#[test]
fn test_scope_resolves_plan_from_relinked_nondefault_path() {
    // Doc-ref-canonical resolution: the planning node's `plan` reference points at
    // a NON-default (archive-style) path, and NO file exists at the template's
    // `dev/active/{id}-plan.md` default. Validate must read the criteria from the
    // re-linked path (proving the template path is not consulted at validation
    // time). The criterion there is uncovered, so the scope must fail naming it.
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some(COVERAGE_ON_EPIC),
            templates: Some(PLAN_TEMPLATE_EXTERNAL),
            ..Default::default()
        },
    );
    let impl_id = seed(&executor, "impl", &["type:task"], "", &[]);
    let c = seed(
        &executor,
        "container",
        &["type:epic"],
        "body without criteria",
        std::slice::from_ref(&impl_id),
    );

    // Write the plan ONLY at the archived path; leave the template default absent.
    let archived = format!("dev/archive/features/{c}/plan.md");
    executor.storage().add_worktree_file(
        &archived,
        "## Success Criteria\n\n- [hard] REQ-77: lives only at the archived path\n",
    );

    // The (done) planning node's `plan` reference points at the archived path.
    wire_plan_bracket(&executor, &c, &archived, State::Done);

    let report = executor
        .validation()
        .validate_scope(&c)
        .expect("scope validation runs");
    assert!(
        report.has_errors() && report.findings.iter().any(|f| f.message.contains("REQ-77")),
        "the criterion read from the re-linked archived plan must drive coverage: {:?}",
        report.findings
    );
}

#[test]
fn test_scope_relinked_nondefault_plan_covered_is_clean() {
    // The same re-linked archived plan, now with its criterion satisfied by the
    // child, leaves the scope clean — a moved/archived + re-linked plan keeps
    // `jit validate` green with no leniency.
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some(COVERAGE_ON_EPIC),
            templates: Some(PLAN_TEMPLATE_EXTERNAL),
            ..Default::default()
        },
    );
    let impl_id = seed(
        &executor,
        "impl",
        &[
            "type:task",
            &credit("container", "body without criteria", "REQ-77"),
        ],
        "",
        &[],
    );
    let c = seed(
        &executor,
        "container",
        &["type:epic"],
        "body without criteria",
        std::slice::from_ref(&impl_id),
    );

    let archived = format!("dev/archive/features/{c}/plan.md");
    executor.storage().add_worktree_file(
        &archived,
        "## Success Criteria\n\n- [hard] REQ-77: lives only at the archived path\n",
    );
    wire_plan_bracket(&executor, &c, &archived, State::Done);

    let report = executor
        .validation()
        .validate_scope(&c)
        .expect("scope validation runs");
    assert!(
        !report.has_errors(),
        "a covered criterion in the re-linked archived plan must leave the scope clean: {:?}",
        report.findings
    );
}

#[test]
fn test_inline_template_still_validates_from_body() {
    // With a `plan` template whose planning node declares no `doc` (an inline
    // plan), the engine must keep reading the issue body: the empty plan-content
    // map reproduces the legacy behavior.
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            rules: Some(COVERAGE_ON_BREAKDOWN),
            templates: Some(PLAN_TEMPLATE_INLINE),
            ..Default::default()
        },
    );
    let (c, _b) = spine_with_breakdown_criteria(&executor, false);

    let report = executor
        .validation()
        .validate_scope(&c)
        .expect("scope validation runs");

    assert!(
        report.has_errors() && report.findings.iter().any(|f| f.message.contains("REQ-01")),
        "inline criteria in the body must still validate: {:?}",
        report.findings
    );
}

// ---------------------------------------------------------------------------
// File-backed regression (jit:1536006d): the external plan base dir is the REPO
// ROOT (parent of `.jit`), NOT `storage.root()` (the `.jit` dir). The
// InMemoryStorage tests above seed the canonical `Worktree(...)` entry directly,
// so these tests retain the physical-path regression: a real `JsonFileStorage`
// rooted at `<tmp>/.jit` with the plan at the repo-root-relative
// `<tmp>/dev/active/{id}-plan.md` (NOT under `.jit/`).
// ---------------------------------------------------------------------------

mod file_backed_external_plan {
    use super::{COVERAGE_ON_EPIC, PLAN_TEMPLATE_EXTERNAL};
    use jit::commands::CommandExecutor;
    use jit::domain::identity::IssueId;
    use jit::domain::State;
    use jit::storage::JsonFileStorage;
    use jit::test_utils::setup_test_repo_with_config;
    use tempfile::TempDir;

    const CONFIG: &str =
        "[type_hierarchy]\ntypes = { epic = 2, planning = 3, breakdown = 3, task = 4 }\n";

    /// Declares the external-`doc` `plan` template and the coverage rule in
    /// the repository's `.jit/` (the doc location comes from the template registry).
    fn declare_external_plan(repo_root: &TempDir) {
        let jit_dir = repo_root.path().join(".jit");
        std::fs::write(jit_dir.join("templates.toml"), PLAN_TEMPLATE_EXTERNAL).unwrap();
        std::fs::write(jit_dir.join("rules.toml"), COVERAGE_ON_EPIC).unwrap();
    }

    fn seed(
        executor: &CommandExecutor<JsonFileStorage>,
        title: &str,
        labels: &[&str],
        description: &str,
        deps: &[IssueId],
    ) -> IssueId {
        let mut issue = crate::fixture_issue(title.to_string(), description.to_string());
        issue.id = super::seed_id(title, description);
        issue.labels = labels.iter().map(|s| s.to_string()).collect();
        issue.dependencies = deps.to_vec();
        issue.state = State::Backlog;
        <JsonFileStorage as super::ScopeFixtureStore>::publish_scope_issue(executor, issue)
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
    ) -> IssueId {
        let c = seed(
            executor,
            "container",
            &["type:epic"],
            "no criteria in the body — the plan lives in the external file",
            &[],
        );
        let credit = format!("satisfies:{}/REQ-77", c.short_id());
        let impl_labels: Vec<&str> = if impl_satisfies {
            vec!["type:task", &credit]
        } else {
            vec!["type:task"]
        };
        let impl_id = seed(executor, "impl", &impl_labels, "", &[]);
        executor.graph().add_dependency(&c, &impl_id).unwrap();

        // Write the plan under the REPO ROOT, NOT under `.jit/`.
        let plan_dir = repo_root.path().join("dev/active");
        std::fs::create_dir_all(&plan_dir).unwrap();
        let plan_path = format!("dev/active/{c}-plan.md");
        std::fs::write(
            repo_root.path().join(&plan_path),
            "## Success Criteria\n\n- [hard] REQ-77: declared only in the external plan\n",
        )
        .unwrap();
        // Doc-ref-canonical: wire a (done) bracket whose `plan` reference points at
        // the repo-root-relative plan file.
        super::wire_plan_bracket(executor, &c, &plan_path, State::Done);
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
        let (repo_root, executor) = setup_test_repo_with_config(CONFIG).unwrap();
        declare_external_plan(&repo_root);
        let c = container_with_repo_root_plan(&repo_root, &executor, false);

        let report = executor
            .validation()
            .validate_scope(&c)
            .expect("scope validation runs");

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
        let (repo_root, executor) = setup_test_repo_with_config(CONFIG).unwrap();
        declare_external_plan(&repo_root);
        let c = container_with_repo_root_plan(&repo_root, &executor, true);

        let report = executor
            .validation()
            .validate_scope(&c)
            .expect("scope validation runs");

        assert!(
            !report.has_errors(),
            "a covered repo-root-plan criterion must leave the scope clean: {:?}",
            report.findings
        );
    }
}

// ---------------------------------------------------------------------------
// CLI failure presentation (subprocess): the gate-checker behaviour the
// coverage-preview gate relies on — findings shown on failure.
// ---------------------------------------------------------------------------

mod cli {
    use jit::domain::identity::IssueId;
    use jit::test_cli::JitCli;
    use jit::test_taxonomy::{
        setup_test_repo_with_taxonomy, type_label, TaxonomyRepo, TestTaxonomy,
    };
    use std::fs;

    /// Coverage rule keyed on the breakdown node; error-severity, enforcing.
    fn coverage_rules(breakdown_type: &str) -> String {
        format!(
            r#"
[[rules]]
name = "breakdown-coverage-preview"
when = {{ type = "{breakdown_type}" }}
severity = "error"
enforce = true
assert = {{ label-coverage = {{ }} }}
"#
        )
    }

    /// Seed a `C -> impl -> B` spine into `cli` where B carries the criterion from
    /// the shared taxonomy fixture. Returns the container id. The impl child credits
    /// B's REQ-01 only when `covered`.
    fn seed_spine(cli: &JitCli, taxonomy: &TestTaxonomy, covered: bool) -> String {
        let config_path = cli.root().join(".jit/config.toml");
        let config = fs::read_to_string(&config_path).unwrap();
        fs::write(
            config_path,
            format!("{config}\n[namespaces.satisfies]\ndescription = \"Criterion coverage\"\nunique = false\n"),
        )
        .unwrap();
        fs::write(
            cli.root().join(".jit/rules.toml"),
            coverage_rules(taxonomy.type_at_level(3)),
        )
        .unwrap();

        let b = cli
            .create_issue([
                "--title",
                "breakdown",
                "--description",
                "## Success Criteria\n\n- [hard] REQ-01: do the thing\n",
                "--label",
                &type_label(taxonomy, 3),
            ])
            .unwrap();
        let b_id: IssueId = b.parse().expect("the binary prints a canonical issue id");
        let credit = format!("satisfies:{}/REQ-01", b_id.short_id());
        let implementation_label = type_label(taxonomy, 4);
        let impl_labels: &[&str] = if covered {
            &[&implementation_label, &credit]
        } else {
            &[&implementation_label]
        };
        let impl_id = cli
            .create_issue(
                ["--title", "impl", "--description", ""]
                    .into_iter()
                    .chain(impl_labels.iter().flat_map(|label| ["--label", *label])),
            )
            .unwrap();
        cli.run(["dep", "add", &impl_id, &b]).unwrap();

        let c = cli
            .create_issue([
                "--title",
                "container",
                "--description",
                "",
                "--label",
                &type_label(taxonomy, 2),
            ])
            .unwrap();
        cli.run(["dep", "add", &c, &impl_id]).unwrap();
        c
    }

    #[test]
    fn test_cli_scope_failure_shows_findings() {
        let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
        let cli = JitCli::with_repository(temp);
        let c = seed_spine(&cli, &taxonomy, false);

        let out = cli.command(["validate", "--scope", &c]).output().unwrap();

        assert!(
            !out.status.success(),
            "an enforcing-rule failure must fail; stderr: {}",
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
    fn test_cli_scope_json_failure_emits_one_validation_envelope() {
        let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
        let cli = JitCli::with_repository(temp);
        let c = seed_spine(&cli, &taxonomy, false);

        let out = cli
            .command(["validate", "--scope", &c, "--json"])
            .output()
            .unwrap();

        assert!(!out.status.success());
        assert!(out.stderr.is_empty(), "JSON failure stays stream-pure");
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(json.as_object().map(|object| object.len()), Some(1));
        assert_eq!(json["error"]["code"], "VALIDATION_FAILED");
        assert!(json["error"]["details"]["findings"]
            .as_array()
            .is_some_and(|findings| findings.iter().any(|finding| {
                finding["rule"] == "breakdown-coverage-preview"
                    && finding["message"]
                        .as_str()
                        .is_some_and(|message| message.contains("REQ-01"))
            })));
    }

    #[test]
    fn test_cli_scope_json_success_keeps_the_scope_report() {
        let TaxonomyRepo { temp, taxonomy } = setup_test_repo_with_taxonomy();
        let cli = JitCli::with_repository(temp);
        let c = seed_spine(&cli, &taxonomy, true);

        let json = cli.run_json(["validate", "--scope", &c]).unwrap();

        assert!(
            json["findings"].is_array(),
            "successful report stays direct: {json}"
        );
        assert!(json.get("error").is_none());
    }

    #[test]
    fn test_cli_scope_rejects_combination_with_id() {
        let cli = JitCli::with_repository(setup_test_repo_with_taxonomy().temp);

        let out = cli
            .command(["validate", "someid", "--scope", "other"])
            .output()
            .unwrap();

        assert!(
            !out.status.success(),
            "`--scope` combined with a positional id must be rejected"
        );
    }
}
