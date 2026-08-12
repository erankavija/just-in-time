//! Integration tests for a project's own gate presets.
//!
//! Every preset is declared by the project under `.jit/config/gate-presets/`:
//! these cases exercise capturing one, listing it, showing it, and applying it,
//! and the audit events an application that writes the gate registry appends.

use jit::declarations::GateStage;
use jit::domain::Priority;
use jit::storage::{IssueStore, JsonFileStorage};
use jit::CommandExecutor;
use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

/// A file-backed executor over a repository whose gate-preset directory is the
/// one a project declares its presets in.
///
/// A project-defined preset lives under `.jit/config/gate-presets/`, which the
/// in-memory harness does not model, so these cases run over a real store.
fn file_backed_executor(temp: &TempDir) -> CommandExecutor<JsonFileStorage> {
    std::env::set_var("JIT_TEST_MODE", "1");
    let worktree = temp.path().join("worktree");
    std::fs::create_dir(&worktree).unwrap();
    let data_root = worktree.join(".jit");
    std::fs::create_dir(&data_root).unwrap();
    let storage = JsonFileStorage::new(data_root);
    std::fs::write(storage.root().join("config.toml"), "").unwrap();
    let layout = jit::storage::discover_repository_layout(&worktree, storage.root()).unwrap();
    CommandExecutor::new(storage).with_layout(layout)
}

/// Define `key` as a manual postcheck gate in the repository's own registry.
fn define_gate(executor: &CommandExecutor<JsonFileStorage>, key: &str) {
    executor
        .add_gate_definition(
            key.to_string(),
            format!("{key} gate"),
            format!("Repository-declared {key} gate"),
            false,
            None,
            GateStage::Postcheck,
        )
        .unwrap_or_else(|error| panic!("define the {key} gate: {error}"));
}

/// Create an issue carrying `gates`, and capture those gates into the
/// project-defined preset `preset`.
fn preset_captured_from_gates(
    executor: &CommandExecutor<JsonFileStorage>,
    preset: &str,
    gates: &[&str],
) -> std::path::PathBuf {
    let (reference, _) = executor
        .create_issue(
            format!("Reference issue for {preset}"),
            String::new(),
            Priority::Normal,
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .expect("create reference issue");
    for gate in gates {
        define_gate(executor, gate);
        executor.add_gate(&reference, (*gate).to_string()).unwrap();
    }
    executor
        .create_gate_preset(preset, &reference)
        .expect("save project-defined preset")
}

enum ConcurrentFixtureMessage {
    Ready,
    Complete(
        TempDir,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
    ),
}

/// REQ-01 + REQ-03: each fixture owns a worktree containing its `.jit` data
/// root, so real concurrent preset writes never converge on one bootstrap lock.
#[test]
fn test_project_defined_preset_fixtures_isolate_concurrent_writes() {
    let workers = 2;
    let ready = Arc::new(Barrier::new(workers + 1));
    let (sender, receiver) = mpsc::channel();
    let handles = (0..workers)
        .map(|worker| {
            let ready = Arc::clone(&ready);
            let sender = sender.clone();
            thread::spawn(move || {
                let temp = TempDir::new().expect("create fixture temp directory");
                let executor = file_backed_executor(&temp);
                let data_root = executor.storage().root().to_path_buf();
                let worktree_root = data_root
                    .parent()
                    .expect("fixture data root has a worktree parent")
                    .to_path_buf();

                sender
                    .send(ConcurrentFixtureMessage::Ready)
                    .expect("report ready fixture");
                ready.wait();
                let saved_path = preset_captured_from_gates(
                    &executor,
                    &format!("concurrent-{worker}"),
                    &["tests"],
                );
                sender
                    .send(ConcurrentFixtureMessage::Complete(
                        temp,
                        worktree_root,
                        data_root,
                        saved_path,
                    ))
                    .expect("report fixture result");
            })
        })
        .collect::<Vec<_>>();
    drop(sender);

    (0..workers).for_each(|_| match receiver.recv_timeout(Duration::from_secs(5)) {
        Ok(ConcurrentFixtureMessage::Ready) => {}
        Ok(ConcurrentFixtureMessage::Complete(..)) => {
            panic!("fixture completed before all workers were ready")
        }
        Err(error) => panic!("fixture setup did not complete: {error}"),
    });
    ready.wait();
    let fixtures = (0..workers)
        .map(|_| match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(ConcurrentFixtureMessage::Complete(temp, worktree, data, saved)) => {
                (temp, worktree, data, saved)
            }
            Ok(ConcurrentFixtureMessage::Ready) => {
                panic!("fixture reported ready after concurrent writes started")
            }
            Err(error) => panic!("concurrent preset write did not complete: {error}"),
        })
        .collect::<Vec<_>>();
    handles
        .into_iter()
        .for_each(|handle| handle.join().expect("concurrent fixture worker"));

    assert_eq!(fixtures.len(), workers);
    assert!(fixtures.iter().all(|(temp, worktree, data, saved)| {
        worktree == &temp.path().join("worktree")
            && data == &worktree.join(".jit")
            && saved.exists()
            && worktree.join(".jit-bootstrap.lock").exists()
    }));
    assert_ne!(
        fixtures[0].1, fixtures[1].1,
        "fixtures own distinct worktrees"
    );
    assert_ne!(
        fixtures[0].1.join(".jit-bootstrap.lock"),
        fixtures[1].1.join(".jit-bootstrap.lock"),
        "concurrent writes use distinct bootstrap namespaces"
    );
}

/// @/inv/event-log (jit:bb7d57a2): a preset application that WRITES the gate
/// registry appends a registry-scoped audit event per definition write —
/// `gate_definition_created` for a new key, `gate_definition_updated` for a
/// timeout-override overwrite — and a no-write re-application appends none.
#[test]
fn test_apply_gate_preset_appends_definition_events() {
    let temp = TempDir::new().unwrap();
    let executor = file_backed_executor(&temp);
    // Captured from a definition the project authors, then removed from the
    // registry, so the first application is the write that defines its key.
    preset_captured_from_gates(&executor, "review", &["team-review"]);
    executor.remove_gate_definition("team-review").unwrap();

    let issue_with = |title: &str| {
        executor
            .create_issue(
                title.to_string(),
                String::new(),
                Priority::Normal,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .expect("create issue")
            .0
    };
    // Authoring the definition to capture it appended events of its own, so
    // every count below is relative to the log the applications start from.
    let logged = |kind: &str| {
        executor
            .storage()
            .read_events()
            .unwrap()
            .iter()
            .filter(|event| event.get_type() == kind)
            .count()
    };
    let baseline = (
        logged("gate_definition_created"),
        logged("gate_definition_updated"),
    );
    let counted = |kind: &str| {
        logged(kind)
            - match kind {
                "gate_definition_created" => baseline.0,
                _ => baseline.1,
            }
    };

    let first = issue_with("First gated issue");
    executor
        .apply_gate_preset(&first, "review", None, false, false, &[])
        .expect("apply the project-defined preset");
    assert_eq!(
        counted("gate_definition_created"),
        1,
        "first application defines the preset gate -> one created event"
    );
    assert_eq!(counted("gate_definition_updated"), 0);

    // Re-apply without an override: the key exists, nothing is written, no event.
    let second = issue_with("Second gated issue");
    executor
        .apply_gate_preset(&second, "review", None, false, false, &[])
        .expect("re-apply preset");
    assert_eq!(
        counted("gate_definition_created"),
        1,
        "no-write re-application appends nothing"
    );
    assert_eq!(counted("gate_definition_updated"), 0);

    // Re-apply WITH a timeout override: the existing definition is overwritten.
    let third = issue_with("Overridden gated issue");
    executor
        .apply_gate_preset(&third, "review", Some(120), false, false, &[])
        .expect("re-apply preset with timeout override");
    assert_eq!(counted("gate_definition_created"), 1);
    assert_eq!(
        counted("gate_definition_updated"),
        1,
        "timeout-override overwrite of an existing definition -> one updated event"
    );
}

/// REQ-02: the full lifecycle of a project's own preset — save, list, show,
/// apply — exercised in-process end to end, over a real `JsonFileStorage`
/// because the save/list/show path reads `.jit/config/gate-presets/`.
#[test]
fn test_project_defined_preset_save_list_show_apply_in_process() {
    let temp = TempDir::new().unwrap();
    let executor = file_backed_executor(&temp);

    // SAVE: capture a reference issue's gates into a project-defined preset,
    // written under the project's gate-presets directory.
    let saved_path = preset_captured_from_gates(&executor, "ci", &["tests", "code-review"]);
    assert!(
        saved_path.exists(),
        "preset should be written to disk at {saved_path:?}"
    );
    assert!(
        temp.path()
            .join("worktree/.jit/config/gate-presets/ci.json")
            .exists(),
        "project-defined preset should live under .jit/config/gate-presets/"
    );

    // LIST: the answer is exactly what the project declares. The
    // planning-bracket names are checked beside it because they are the ones an
    // adopter is most likely to expect the engine to answer for, and it answers
    // for none of them until the repository declares them itself.
    let presets = executor.list_gate_presets().expect("list presets");
    assert_eq!(
        presets
            .iter()
            .map(|preset| preset.name.as_str())
            .collect::<Vec<_>>(),
        vec!["ci"],
        "the listing reports the project's own presets and nothing else"
    );
    assert_eq!(presets[0].gate_count, 2);
    for absent in ["plan-review", "coverage-preview", "breakdown-review"] {
        assert!(
            executor.show_gate_preset(absent).is_err(),
            "{absent} must not resolve as a preset the project never declared"
        );
    }

    // SHOW: the saved preset reports the bundled gate keys.
    let shown = executor.show_gate_preset("ci").expect("show preset");
    let keys: Vec<&str> = shown.gates.iter().map(|g| g.key.as_str()).collect();
    assert!(keys.contains(&"tests"), "ci bundles the tests gate");
    assert!(
        keys.contains(&"code-review"),
        "ci bundles the code-review gate"
    );

    // APPLY: attaching the preset to a fresh issue adds both bundled gates.
    let (target, _) = executor
        .create_issue(
            "Feature issue".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .expect("create target issue");
    let (result, _warnings) = executor
        .apply_gate_preset(&target, "ci", None, false, false, &[])
        .expect("apply project-defined preset");
    assert!(result.added.contains(&"tests".to_string()));
    assert!(result.added.contains(&"code-review".to_string()));

    // The target issue now requires both gates.
    let issue = executor.storage().load_issue(&target).expect("load target");
    assert!(issue.gates_required.contains(&"tests".to_string()));
    assert!(issue.gates_required.contains(&"code-review".to_string()));
}
