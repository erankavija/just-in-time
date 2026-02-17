//! Tests for automatic state transitions in `jit validate --fix`
//!
//! After worktree merges, issues in backlog may have all dependencies complete
//! but never auto-transition to ready. The validate --fix command should detect
//! and fix these pending transitions.

mod harness;
use harness::TestHarness;
use jit::domain::State;
use jit::storage::IssueStore;

#[test]
fn test_validate_fix_transitions_backlog_to_ready() {
    let mut h = TestHarness::new();

    // Create a story in backlog with one task dependency
    let task = h.create_issue("Task");
    let story = h.create_issue("Story");
    h.executor.add_dependency(&story, &task).unwrap();

    // Verify story is in backlog (has dependencies)
    let story_issue = h.storage.load_issue(&story).unwrap();
    assert_eq!(story_issue.state, State::Backlog);

    // Complete the task (simulating worktree merge - state changes without transition)
    let mut task_issue = h.storage.load_issue(&task).unwrap();
    task_issue.state = State::Done;
    h.storage.save_issue(task_issue).unwrap();

    // Story is still in backlog (auto-transition didn't run)
    let story_issue = h.storage.load_issue(&story).unwrap();
    assert_eq!(story_issue.state, State::Backlog);

    // Run validate --fix
    let (fixes, _messages) = h.executor.validate_with_fix(true, false).unwrap();
    assert_eq!(fixes, 1, "Should fix one pending transition");

    // Story should now be ready
    let story_issue = h.storage.load_issue(&story).unwrap();
    assert_eq!(
        story_issue.state,
        State::Ready,
        "Story should transition to ready"
    );
}

#[test]
fn test_validate_fix_ignores_incomplete_dependencies() {
    let mut h = TestHarness::new();

    // Create a story with two task dependencies
    let task1 = h.create_issue("Task 1");
    let task2 = h.create_issue("Task 2");
    let story = h.create_issue("Story");
    h.executor.add_dependency(&story, &task1).unwrap();
    h.executor.add_dependency(&story, &task2).unwrap();

    // Complete only one task
    let mut task1_issue = h.storage.load_issue(&task1).unwrap();
    task1_issue.state = State::Done;
    h.storage.save_issue(task1_issue.clone()).unwrap();

    // Run validate --fix
    let (fixes, _messages) = h.executor.validate_with_fix(true, false).unwrap();
    assert_eq!(fixes, 0, "Should not fix - dependencies incomplete");

    // Story should still be in backlog
    let story_issue = h.storage.load_issue(&story).unwrap();
    assert_eq!(story_issue.state, State::Backlog);
}

#[test]
fn test_validate_fix_transitions_multiple_issues() {
    let mut h = TestHarness::new();

    // Create multiple stories with dependencies
    let task1 = h.create_issue("Task 1");
    let task2 = h.create_issue("Task 2");
    let story1 = h.create_issue("Story 1");
    let story2 = h.create_issue("Story 2");

    h.executor.add_dependency(&story1, &task1).unwrap();
    h.executor.add_dependency(&story2, &task2).unwrap();

    // Complete both tasks
    let mut task1_issue = h.storage.load_issue(&task1).unwrap();
    task1_issue.state = State::Done;
    h.storage.save_issue(task1_issue.clone()).unwrap();

    let mut task2_issue = h.storage.load_issue(&task2).unwrap();
    task2_issue.state = State::Done;
    h.storage.save_issue(task2_issue.clone()).unwrap();

    // Run validate --fix
    let (fixes, _messages) = h.executor.validate_with_fix(true, false).unwrap();
    assert_eq!(fixes, 2, "Should fix two pending transitions");

    // Both stories should be ready
    let story1_issue = h.storage.load_issue(&story1).unwrap();
    assert_eq!(story1_issue.state, State::Ready);

    let story2_issue = h.storage.load_issue(&story2).unwrap();
    assert_eq!(story2_issue.state, State::Ready);
}

#[test]
fn test_validate_fix_dry_run_no_changes() {
    let mut h = TestHarness::new();

    // Create a story with completed dependency
    let task = h.create_issue("Task");
    let story = h.create_issue("Story");
    h.executor.add_dependency(&story, &task).unwrap();

    let mut task_issue = h.storage.load_issue(&task).unwrap();
    task_issue.state = State::Done;
    h.storage.save_issue(task_issue).unwrap();

    // Run validate --fix with dry-run
    let (fixes, _messages) = h.executor.validate_with_fix(true, true).unwrap();
    assert_eq!(fixes, 1, "Should detect one pending transition");

    // Story should still be in backlog (dry run doesn't apply changes)
    let story_issue = h.storage.load_issue(&story).unwrap();
    assert_eq!(
        story_issue.state,
        State::Backlog,
        "Dry run should not change state"
    );
}

#[test]
fn test_validate_fix_ignores_ready_issues() {
    let mut h = TestHarness::new();

    // Create issues already in ready state
    let issue1 = h.create_issue("Ready issue 1");
    let issue2 = h.create_issue("Ready issue 2");

    // Manually set to ready
    let mut issue1_loaded = h.storage.load_issue(&issue1).unwrap();
    issue1_loaded.state = State::Ready;
    h.storage.save_issue(issue1_loaded.clone()).unwrap();

    let mut issue2_loaded = h.storage.load_issue(&issue2).unwrap();
    issue2_loaded.state = State::Ready;
    h.storage.save_issue(issue2_loaded.clone()).unwrap();

    // Run validate --fix
    let (fixes, _messages) = h.executor.validate_with_fix(true, false).unwrap();
    assert_eq!(fixes, 0, "Should not touch issues already in ready state");
}

#[test]
fn test_validate_fix_ignores_done_issues() {
    let mut h = TestHarness::new();

    // Create a done issue that had dependencies
    let task = h.create_issue("Task");
    let story = h.create_issue("Story");
    h.executor.add_dependency(&story, &task).unwrap();

    // Both done
    let mut task_issue = h.storage.load_issue(&task).unwrap();
    task_issue.state = State::Done;
    h.storage.save_issue(task_issue).unwrap();

    let mut story_issue = h.storage.load_issue(&story).unwrap();
    story_issue.state = State::Done;
    h.storage.save_issue(story_issue).unwrap();

    // Run validate --fix
    let (fixes, _messages) = h.executor.validate_with_fix(true, false).unwrap();
    assert_eq!(fixes, 0, "Should not touch done issues");
}

#[test]
fn test_validate_fix_complex_dependency_chain() {
    let mut h = TestHarness::new();

    // Create a chain: epic -> story1, story2 -> task1, task2, task3
    let task1 = h.create_issue("Task 1");
    let task2 = h.create_issue("Task 2");
    let task3 = h.create_issue("Task 3");
    let story1 = h.create_issue("Story 1");
    let story2 = h.create_issue("Story 2");
    let epic = h.create_issue("Epic");

    h.executor.add_dependency(&story1, &task1).unwrap();
    h.executor.add_dependency(&story1, &task2).unwrap();
    h.executor.add_dependency(&story2, &task3).unwrap();
    h.executor.add_dependency(&epic, &story1).unwrap();
    h.executor.add_dependency(&epic, &story2).unwrap();

    // Complete all tasks
    for task_id in [&task1, &task2, &task3] {
        let mut task = h.storage.load_issue(task_id).unwrap();
        task.state = State::Done;
        h.storage.save_issue(task).unwrap();
    }

    // Run validate --fix
    let (fixes, _messages) = h.executor.validate_with_fix(true, false).unwrap();
    assert_eq!(fixes, 2, "Should transition story1 and story2 only");

    // Stories should be ready
    let story1_issue = h.storage.load_issue(&story1).unwrap();
    assert_eq!(story1_issue.state, State::Ready);

    let story2_issue = h.storage.load_issue(&story2).unwrap();
    assert_eq!(story2_issue.state, State::Ready);

    // Epic should still be in backlog (dependencies need to be Done, not just Ready)
    let epic_issue = h.storage.load_issue(&epic).unwrap();
    assert_eq!(epic_issue.state, State::Backlog);

    // Now mark stories as done
    let mut story1_loaded = h.storage.load_issue(&story1).unwrap();
    story1_loaded.state = State::Done;
    h.storage.save_issue(story1_loaded.clone()).unwrap();

    let mut story2_loaded = h.storage.load_issue(&story2).unwrap();
    story2_loaded.state = State::Done;
    h.storage.save_issue(story2_loaded.clone()).unwrap();

    // Run validate --fix again
    let (more_fixes, _messages) = h.executor.validate_with_fix(true, false).unwrap();
    assert_eq!(more_fixes, 1, "Should now transition epic");

    // Epic should now be ready
    let epic_issue = h.storage.load_issue(&epic).unwrap();
    assert_eq!(epic_issue.state, State::Ready);
}
