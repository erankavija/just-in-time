//! Demonstration of test harness usage
//!
//! This shows the recommended patterns for using the TestHarness
//! for fast, reliable in-process testing.

mod harness;

use harness::TestHarness;
use jit::domain::{Priority, State};

// ========== Query Tests ==========

#[test]
fn test_harness_query_ready() {
    let h = TestHarness::new();

    // Create ready and non-ready issues
    let ready_id = h.create_ready_issue("Ready task");
    let assigned_id = h.create_ready_issue("Assigned task");
    h.executor
        .claim_issue(&assigned_id, "agent:worker-1".to_string())
        .unwrap();

    // Query
    let ready = h.executor.query_ready().unwrap();

    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id, ready_id);
}

#[test]
fn test_harness_query_by_priority() {
    let h = TestHarness::new();

    let high_id = h.create_issue_with_priority("High", Priority::High);
    let _normal_id = h.create_issue("Normal");
    let _critical_id = h.create_issue_with_priority("Critical", Priority::Critical);

    let high_issues = h.executor.query_by_priority(Priority::High).unwrap();

    assert_eq!(high_issues.len(), 1);
    assert_eq!(high_issues[0].id, high_id);
}

// ========== Issue Lifecycle Tests ==========

#[test]
fn test_harness_issue_lifecycle() {
    let h = TestHarness::new();

    // Create
    let id = h.create_issue("Task");
    assert_eq!(h.all_issues().len(), 1);

    // Update state
    let _ = h
        .executor
        .update_issue(
            &id,
            None,
            None,
            None,
            Some(State::Ready),
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .unwrap();
    let issue = h.get_issue(&id);
    assert_eq!(issue.state, State::Ready);

    // Claim
    h.executor
        .claim_issue(&id, "agent:worker-1".to_string())
        .unwrap();
    let issue = h.get_issue(&id);
    assert_eq!(issue.assignee, Some("agent:worker-1".parse().unwrap()));

    // Release
    h.executor.release_issue(&id, "timeout").unwrap();
    let issue = h.get_issue(&id);
    assert!(issue.assignee.is_none());

    // Delete
    h.executor.delete_issue(&id).unwrap();
    assert_eq!(h.all_issues().len(), 0);
}

// ========== Dependency Tests ==========

#[test]
fn test_harness_dependencies_block() {
    let h = TestHarness::new();

    let parent = h.create_issue("Parent");
    let child = h.create_issue("Child");

    // Add dependency
    h.executor.add_dependency(&child, &parent).unwrap();

    // Child should be blocked
    let child_issue = h.get_issue(&child);
    let all = h.all_issues();
    let resolved: std::collections::HashMap<String, &jit::domain::Issue> =
        all.iter().map(|i| (i.id.clone(), i)).collect();
    assert!(child_issue.is_blocked(&resolved));

    // Complete parent
    let _ = h
        .executor
        .update_issue(
            &parent,
            None,
            None,
            None,
            Some(State::Done),
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .unwrap();

    // Child should be unblocked
    let all = h.all_issues();
    let resolved: std::collections::HashMap<String, &jit::domain::Issue> =
        all.iter().map(|i| (i.id.clone(), i)).collect();
    assert!(!child_issue.is_blocked(&resolved));
}

#[test]
fn test_harness_cycle_detection() {
    let h = TestHarness::new();

    let issue1 = h.create_issue("Task 1");
    let issue2 = h.create_issue("Task 2");

    // Create dependency: 2 depends on 1
    h.executor.add_dependency(&issue2, &issue1).unwrap();

    // Try to create cycle: 1 depends on 2
    let result = h.executor.add_dependency(&issue1, &issue2);
    assert!(result.is_err(), "Cycle should be rejected");
}

// ========== Gate Tests ==========

#[test]
fn test_harness_gates() {
    let h = TestHarness::new();

    // Add gate definition
    h.add_gate("review", "Code Review", "Manual review", false);

    // Create issue with gate
    let id = h.create_issue_with_gates("Task", vec!["review".to_string()]);

    // Issue should NOT be blocked by pending gate (gates don't block starting work)
    let issue = h.get_issue(&id);
    let all = h.all_issues();
    let resolved: std::collections::HashMap<String, &jit::domain::Issue> =
        all.iter().map(|i| (i.id.clone(), i)).collect();
    assert!(!issue.is_blocked(&resolved));

    // But gates do prevent completion
    assert!(issue.has_unpassed_gates());

    // Pass gate
    h.executor
        .pass_gate(&id, "review".to_string(), None, false)
        .unwrap();

    // Issue gates should be passed now
    let issue = h.get_issue(&id);
    assert!(!issue.has_unpassed_gates());
}

// ========== Complex Scenarios ==========

#[test]
fn test_harness_complex_workflow() {
    let h = TestHarness::new();

    // Setup gates (use manual gates for this workflow test)
    h.add_gate("tests", "Tests", "Unit tests", false);
    h.add_gate("review", "Review", "Code review", false);

    // Create epic with dependencies
    let dep1 = h.create_issue_with_gates("Dependency 1", vec!["tests".to_string()]);
    let dep2 = h.create_issue_with_gates("Dependency 2", vec!["tests".to_string()]);
    let epic = h.create_issue_with_gates("Epic", vec!["review".to_string()]);

    h.executor.add_dependency(&epic, &dep1).unwrap();
    h.executor.add_dependency(&epic, &dep2).unwrap();

    // Pass gates for dependencies
    h.executor
        .pass_gate(&dep1, "tests".to_string(), None, false)
        .unwrap();
    h.executor
        .pass_gate(&dep2, "tests".to_string(), None, false)
        .unwrap();

    // Complete dependencies
    let _ = h
        .executor
        .update_issue(
            &dep1,
            None,
            None,
            None,
            Some(State::Done),
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .unwrap();
    let _ = h
        .executor
        .update_issue(
            &dep2,
            None,
            None,
            None,
            Some(State::Done),
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .unwrap();

    // Pass epic's gate
    h.executor
        .pass_gate(&epic, "review".to_string(), None, false)
        .unwrap();

    // Epic should now be unblocked
    let epic_issue = h.get_issue(&epic);
    let all = h.all_issues();
    let resolved: std::collections::HashMap<String, &jit::domain::Issue> =
        all.iter().map(|i| (i.id.clone(), i)).collect();
    assert!(!epic_issue.is_blocked(&resolved));
}

// ========== Performance Test Example ==========

#[test]
fn test_harness_scales_with_many_issues() {
    let h = TestHarness::new();

    // Create many issues quickly (all auto-transition to Ready since no blockers)
    for i in 0..100 {
        h.create_issue(&format!("Task {}", i));
    }

    assert_eq!(h.all_issues().len(), 100);

    // Query should be fast - all are ready since no blockers
    let ready = h.executor.query_ready().unwrap();
    assert_eq!(ready.len(), 100); // All auto-transitioned to Ready
}

// ========== Addressable Item Model (jit:56ab0224) ==========

#[test]
fn test_harness_item_list_and_resolve() {
    let h = TestHarness::new().with_item_kinds();
    let id = h.create_issue_with_desc(
        "Foundational",
        "## Success Criteria\n\n- [hard] REQ-01: first\n- [hard] REQ-02: second\n",
    );
    let short: String = id.chars().take(8).collect();

    // Items are indexed across the repo and addressed by qualified id.
    let listed = h.executor.list_items(None).unwrap();
    assert_eq!(listed.count, 2);

    // The `<short-id>/<self-id>` sugar resolves through the same issue-id resolver,
    // and the resolved item reports its canonical uniform qualified id.
    let shown = h.executor.show_item(&format!("{short}/REQ-01")).unwrap();
    assert_eq!(shown.item.self_id, "REQ-01");
    assert_eq!(
        shown.item.qualified_id,
        format!("@/issue/{short}/requirement/REQ-01")
    );
}

#[test]
fn test_harness_item_kind_compatible_with_label_coverage() {
    use jit::domain::item::ItemKind;

    // REQ-05: the canonical requirement kind (as `jit init` authors it) expands to
    // exactly the (section, marker, id-pattern) triple the label-coverage rule
    // consumes by default, so the coverage machinery is compatible with the item
    // model without rewriting any rule.
    let kind = ItemKind::from_config(
        "requirement",
        &jit::config::ItemKindConfig {
            section: Some("success_criteria".to_string()),
            id_pattern: Some("[A-Z][A-Z0-9]*-[0-9]+".to_string()),
            markers: Some(vec!["[hard]".to_string()]),
            link_namespaces: Some(vec!["satisfies".to_string()]),
            ..Default::default()
        },
    )
    .unwrap();
    let (section, marker, pattern) = kind.as_triple();
    assert_eq!(section, "success_criteria");
    assert_eq!(marker, Some("[hard]"));
    assert_eq!(pattern, "[A-Z][A-Z0-9]*-[0-9]+");

    // REQ-05: a generic link label `<ns>:<issue>/<self-id>` (here the
    // requirement kind's `satisfies` namespace) resolves to the addressed item
    // via its qualified id — not merely the legacy unqualified `satisfies:REQ-01`.
    let h = TestHarness::new().with_item_kinds();
    let container = h.create_issue_with_desc(
        "Container",
        "## Success Criteria\n\n- [hard] REQ-01: covered\n",
    );
    let short: String = container.chars().take(8).collect();
    let qualified_label = format!("satisfies:{short}/REQ-01");
    let resolved = h
        .executor
        .resolve_link_label(&qualified_label)
        .unwrap()
        .expect("qualified satisfies: reference resolves to the addressed item");
    assert_eq!(resolved.item.self_id, "REQ-01");
    assert_eq!(
        resolved.item.qualified_id,
        format!("@/issue/{short}/requirement/REQ-01")
    );

    // An unresolvable qualified reference is reported, not silently dropped.
    let bad = format!("satisfies:{short}/REQ-99");
    assert!(h.executor.resolve_link_label(&bad).is_err());

    // The default link namespace of the requirement kind is `satisfies`.
    assert_eq!(kind.link_namespaces(), &["satisfies".to_string()]);
}

// ========== Container Rollup Tests (jit:7fe5c743) ==========

/// Build a container whose direct children span several states, wired as
/// dependencies, then aggregate those children with `StateRollup::from_issues`
/// (the composition `issue progress` performs). Verifies the terminal-state
/// semantics: done/rejected counted distinctly, open = non-terminal.
#[test]
fn test_harness_container_state_rollup() {
    use jit::output::StateRollup;

    let h = TestHarness::new();
    let epic = h.create_issue("Epic");
    let done = h.create_issue("Done child");
    let wip = h.create_issue("WIP child");
    let rejected = h.create_issue("Rejected child");
    let ready = h.create_issue("Ready child");

    for child in [&done, &wip, &rejected, &ready] {
        h.executor.add_dependency(&epic, child).unwrap();
    }
    let set_state = |id: &str, state: State| {
        h.executor
            .update_issue(
                id,
                None,
                None,
                None,
                Some(state),
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap();
    };
    set_state(&done, State::Done);
    set_state(&wip, State::InProgress);
    set_state(&rejected, State::Rejected);

    let container = h.get_issue(&epic);
    let children: Vec<_> = container
        .dependencies
        .iter()
        .map(|id| h.get_issue(id))
        .collect();

    let rollup = StateRollup::from_issues(&children);
    assert_eq!(rollup.total, 4);
    assert_eq!(rollup.done, 1);
    assert_eq!(rollup.rejected, 1);
    assert_eq!(rollup.open, 2); // in_progress + ready
    assert_eq!(rollup.percent, 25);
    // Every state present, zero-count states included.
    assert_eq!(rollup.by_state.len(), State::all().len());
    let done_bucket = rollup
        .by_state
        .iter()
        .find(|c| c.state == State::Done)
        .unwrap();
    assert_eq!(done_bucket.count, 1);
    let gated_bucket = rollup
        .by_state
        .iter()
        .find(|c| c.state == State::Gated)
        .unwrap();
    assert_eq!(gated_bucket.count, 0);
}

/// `issue_status_response` (the per-child projection reused by `issue children`)
/// carries the child's state, per-gate status, and unmet dependencies.
#[test]
fn test_harness_issue_status_response_projection() {
    let h = TestHarness::new();
    let dep = h.create_issue("Upstream");
    let child = h.create_issue_with_gates("Gated child", vec!["tests".into()]);
    h.executor.add_dependency(&child, &dep).unwrap();

    let issue = h.get_issue(&child);
    let status = h.executor.issue_status_response(issue).unwrap();

    assert_eq!(status.title, "Gated child");
    assert_eq!(status.gates.len(), 1);
    assert_eq!(status.gates[0].key, "tests");
    // The upstream dependency is not terminal, so it is unmet.
    assert_eq!(status.unmet_dependencies.len(), 1);
    assert_eq!(status.unmet_dependencies[0], dep[0..8]);
}

/// An empty container aggregates to total 0 with percent 0 (no divide-by-zero)
/// and still enumerates every state.
#[test]
fn test_harness_empty_container_rollup() {
    use jit::output::StateRollup;

    let rollup = StateRollup::from_issues(&[]);
    assert_eq!(rollup.total, 0);
    assert_eq!(rollup.percent, 0);
    assert_eq!(rollup.by_state.len(), State::all().len());
    assert!(rollup.by_state.iter().all(|c| c.count == 0));
}
